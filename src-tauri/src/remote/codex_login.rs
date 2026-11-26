//! Signing Codex in on a remote machine from Settings (ADR 0105). The machine runs
//! `codex login --device-auth` in one of its host terminals, as the SSH user of the machine. This
//! app reads only the verification link and the one-time code that the CLI prints. The person
//! enters the code in their own browser, and Codex on the machine polls OpenAI, writes the
//! credential under its own `~/.codex` and exits. Nothing from this computer's Codex account goes
//! to the machine, and no output other than the challenge leaves this module.
use prometeu_core::lock::lock;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use super::codex::{self, CodexStatus};
use super::connection::{Connection, TerminalSink};
use super::login::{plain, Sink};

/// Codex asks OpenAI for the code before it prints anything.
const CHALLENGE_WAIT: Duration = Duration::from_secs(30);
/// Codex stops polling 15 minutes after it requests the code (`device_code_auth.rs`, 0.172.0).
/// The extra minute covers a slow exit.
const LOGIN_WAIT: Duration = Duration::from_secs(16 * 60);
const END_WAIT: Duration = Duration::from_secs(5);
const COLUMNS: u16 = 200;

/// The only host that this app opens for a Codex device sign-in.
const ISSUER_HOST: &str = "auth.openai.com";
const DEVICE_PATH: &str = "/codex/device";

/// What the person needs to finish the sign-in in a browser.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexChallenge {
    pub url: String,
    pub code: String,
}

/// The verification link and the one-time code that `codex login --device-auth` printed. Only
/// complete lines count, so a chunk boundary never yields a cut link or code.
pub fn challenge(output: &str) -> Option<CodexChallenge> {
    let text = plain(output).replace('\r', "");
    let complete = &text[..text.rfind('\n')?];
    let mut lines = complete.lines().map(str::trim);
    let url = lines.by_ref().find_map(device_link)?;
    let code = lines.find(|line| one_time_code(line))?.to_string();
    Some(CodexChallenge { url, code })
}

fn device_link(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let link = line[start..].split_whitespace().next()?;
    let rest = &link["https://".len()..];
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, path) = rest.split_at(host_end);
    let path = path.split(['?', '#']).next().unwrap_or("");
    (host == ISSUER_HOST && path.trim_end_matches('/') == DEVICE_PATH).then(|| link.to_string())
}

fn one_time_code(line: &str) -> bool {
    (4..=32).contains(&line.len())
        && line.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !line.starts_with('-')
        && !line.ends_with('-')
        && line
            .chars()
            .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
}

/// How a sign-in ended for this app.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    SignedIn,
    Cancelled,
    Expired,
    Failed,
}

/// `exit` is `None` when the terminal still ran at the deadline, and `Some(None)` when the host
/// lost it. The account state that `codex login status` reads after the end wins over the exit.
pub fn outcome(
    exit: Option<Option<u32>>,
    cancelled: bool,
    timed_out: bool,
    logged_in: bool,
) -> Outcome {
    if cancelled {
        Outcome::Cancelled
    } else if logged_in {
        Outcome::SignedIn
    } else if exit.is_none() || timed_out {
        Outcome::Expired
    } else {
        Outcome::Failed
    }
}

#[derive(Clone)]
struct Pending {
    terminal: String,
    sink: Arc<Sink>,
    cancelled: Arc<AtomicBool>,
    started: Instant,
}

/// One Codex sign-in in progress per machine on this computer.
#[derive(Default)]
pub struct CodexLogins(Mutex<HashMap<String, Pending>>);

/// Kill the login terminal if it still runs, then drop it on the host and stop following it.
fn end(connection: &Connection, pending: &Pending) {
    let running = pending.sink.wait_for(Duration::ZERO, |o| o.exit).is_none();
    if running && connection.connected() {
        connection.notify(
            "terminal_signal",
            json!({"id": pending.terminal, "signal": "kill"}),
        );
        pending.sink.wait_for(END_WAIT, |o| o.exit);
    }
    connection.notify("terminal_forget", json!({"id": pending.terminal}));
    connection.release(&pending.terminal, Arc::as_ptr(&pending.sink) as *const ());
}

fn cancel(connection: Option<&Connection>, pending: &Pending) {
    pending.cancelled.store(true, Ordering::Release);
    if let Some(connection) = connection {
        end(connection, pending);
    }
    // Wake a waiter also when the machine is offline and the kill cannot arrive.
    TerminalSink::lost(&*pending.sink);
}

/// Stop the sign-in in progress on a machine, if there is one.
pub(crate) fn abandon(app: &AppHandle, id: &str) {
    let pending = lock(&app.state::<CodexLogins>().0).remove(id);
    if let Some(pending) = pending {
        cancel(app.state::<super::Service>().get(id).as_deref(), &pending);
    }
}

/// Start `codex login --device-auth` on the machine and return its challenge. `None` means that
/// Codex is already signed in there: Codex removes the stored credential when a login starts, so a
/// sign-in that another computer finished must not be replaced.
#[tauri::command(async)]
pub fn machine_codex_login(app: AppHandle, id: String) -> Result<Option<CodexChallenge>, String> {
    let service = app.state::<super::Service>();
    let connection = service.connected(&id)?;
    let name = service.machine(&id).map_or_else(|| id.clone(), |m| m.name);
    let status = codex::status(&connection)?;
    super::publish_codex(&app, &id, status.clone());
    if status.logged_in {
        return Ok(None);
    }
    if !status.installed || !status.supported {
        return Err(codex::problem(&status, &name).unwrap_or_default());
    }
    abandon(&app, &id);
    let (pending, challenge) = begin(&connection)?;
    let replaced = lock(&app.state::<CodexLogins>().0).insert(id, pending);
    if let Some(replaced) = replaced {
        cancel(Some(&connection), &replaced);
    }
    Ok(Some(challenge))
}

fn begin(connection: &Connection) -> Result<(Pending, CodexChallenge), String> {
    let home = connection.home().unwrap_or_else(|| "/".into());
    let terminal = format!("codex-login-{}", uuid::Uuid::new_v4().simple());
    connection.request(
        "terminal_open",
        json!({"id": terminal, "argv": ["codex", "login", "--device-auth"], "cwd": home,
               "cols": COLUMNS, "rows": 40}),
    )?;
    let pending = Pending {
        terminal,
        sink: Sink::new(),
        cancelled: Arc::new(AtomicBool::new(false)),
        started: Instant::now(),
    };
    connection.subscribe(&pending.terminal, pending.sink.clone());
    let found = pending.sink.wait_for(CHALLENGE_WAIT, |output| {
        challenge(&output.text)
            .map(Some)
            .or_else(|| output.exit.map(|_| None))
    });
    match found {
        Some(Some(challenge)) => Ok((pending, challenge)),
        _ => {
            end(connection, &pending);
            Err(crate::i18n::t("err.remote.codexLoginStart"))
        }
    }
}

/// Wait until the sign-in in progress ends, then return the new Codex status of the machine.
#[tauri::command]
pub async fn machine_codex_login_wait(app: AppHandle, id: String) -> Result<CodexStatus, String> {
    tauri::async_runtime::spawn_blocking(move || wait(&app, &id))
        .await
        .map_err(crate::i18n::io)?
}

fn wait(app: &AppHandle, id: &str) -> Result<CodexStatus, String> {
    let pending = lock(&app.state::<CodexLogins>().0)
        .get(id)
        .cloned()
        .ok_or_else(|| crate::i18n::t("err.remote.loginExpired"))?;
    // An SSH loss does not end the wait: the subscription attaches again with the connection.
    let exit = pending
        .sink
        .wait_for(LOGIN_WAIT.saturating_sub(pending.started.elapsed()), |o| {
            o.exit
        });
    let timed_out = pending
        .sink
        .wait_for(Duration::ZERO, |o| {
            Some(plain(&o.text).contains("timed out"))
        })
        .unwrap_or(false);
    {
        let state = app.state::<CodexLogins>();
        let mut logins = lock(&state.0);
        if logins
            .get(id)
            .is_some_and(|p| p.terminal == pending.terminal)
        {
            logins.remove(id);
        }
    }
    if pending.cancelled.load(Ordering::Acquire) {
        return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
    }
    let service = app.state::<super::Service>();
    if let Some(connection) = service.get(id) {
        end(&connection, &pending);
    }
    let connection = service.connected(id)?;
    let status = codex::status(&connection)?;
    super::publish_codex(app, id, status.clone());
    match outcome(exit, false, timed_out, status.logged_in) {
        Outcome::SignedIn => Ok(status),
        Outcome::Expired => Err(crate::i18n::t("err.remote.codexLoginExpired")),
        Outcome::Cancelled | Outcome::Failed => Err(crate::i18n::t("err.remote.codexLoginFailed")),
    }
}

/// Abandon the sign-in in progress and stop its terminal on the machine.
#[tauri::command(async)]
pub fn machine_codex_login_cancel(app: AppHandle, id: String) {
    abandon(&app, &id);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `codex login --device-auth` 0.172.0 (linux-x64 npm build) in a PTY, with the
    /// default issuer in place of the local stub issuer that served the code.
    const OUTPUT: &str = "\r\nWelcome to Codex [v\x1b[90m0.172.0\x1b[0m]\r\n\x1b[90mOpenAI's command-line coding agent\x1b[0m\r\n\r\nFollow these steps to sign in with ChatGPT using device code authorization:\r\n\r\n1. Open this link in your browser and sign in to your account\r\n   \x1b[94mhttps://auth.openai.com/codex/device\x1b[0m\r\n\r\n2. Enter this one-time code \x1b[90m(expires in 15 minutes)\x1b[0m\r\n   \x1b[94mWXYZ-1234\x1b[0m\r\n\r\n\x1b[90mContinue only if you started this login in Codex. If a website or another person gave you this code, cancel.\x1b[0m\r\n\r\n";

    #[test]
    fn the_challenge_codex_prints_is_found_through_its_escapes() {
        assert_eq!(
            challenge(OUTPUT),
            Some(CodexChallenge {
                url: "https://auth.openai.com/codex/device".into(),
                code: "WXYZ-1234".into(),
            })
        );
    }

    #[test]
    fn a_challenge_cut_by_a_chunk_boundary_waits_for_the_rest() {
        let cut = OUTPUT.find("WXYZ-12").unwrap() + "WXYZ-12".len();
        assert_eq!(challenge(&OUTPUT[..cut]), None);
        let link = OUTPUT.find("codex/dev").unwrap();
        assert_eq!(challenge(&OUTPUT[..link]), None);
    }

    #[test]
    fn other_hosts_and_paths_are_ignored() {
        for link in [
            "https://evil.example/codex/device",
            "https://auth.openai.com.evil.example/codex/device",
            "https://auth.openai.com/oauth/authorize",
            "http://auth.openai.com/codex/device",
        ] {
            assert_eq!(
                challenge(&format!("open\n   {link}\n   ABCD-EFGH\n")),
                None,
                "{link}"
            );
        }
    }

    #[test]
    fn only_a_code_shaped_line_after_the_link_is_the_code() {
        let text = "2. Enter this one-time code (expires in 15 minutes)\nABCD-EFGH\nhttps://auth.openai.com/codex/device\nexpires\n  QRST-5678 \n";
        assert_eq!(
            challenge(text).map(|c| c.code).as_deref(),
            Some("QRST-5678")
        );
    }

    #[test]
    fn the_account_state_decides_the_outcome() {
        assert_eq!(
            outcome(Some(Some(0)), false, false, true),
            Outcome::SignedIn
        );
        // The host lost the terminal after Codex wrote the credential.
        assert_eq!(outcome(Some(None), false, false, true), Outcome::SignedIn);
        assert_eq!(
            outcome(Some(Some(0)), true, false, true),
            Outcome::Cancelled
        );
        assert_eq!(outcome(None, false, false, false), Outcome::Expired);
        assert_eq!(outcome(Some(Some(1)), false, true, false), Outcome::Expired);
        assert_eq!(outcome(Some(Some(1)), false, false, false), Outcome::Failed);
        assert_eq!(outcome(Some(Some(0)), false, false, false), Outcome::Failed);
    }
}
