//! Signing Claude in on a remote machine without an SSH session (ADR 0101). The machine runs
//! `claude auth login --claudeai` in one of its terminals; this app shows the authorization link,
//! the person signs in in their own browser, and the code Claude shows them goes back to that
//! terminal. The credential is created and kept on the machine; nothing from this computer's
//! Claude account is sent there.
use prometeu_core::lock::lock;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use super::connection::{Connection, TerminalSink};

const LINK_WAIT: Duration = Duration::from_secs(30);
const CODE_WAIT: Duration = Duration::from_secs(90);
/// Wide enough that Claude never wraps the authorization link.
const COLUMNS: u16 = 1000;

/// Remove CSI, OSC and two-byte escape sequences, keeping printable text and line breaks.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            if !ch.is_control() || ch == '\n' || ch == '\r' || ch == '\t' {
                out.push(ch);
            }
            continue;
        }
        match chars.next() {
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// The first complete Claude authorization link in the output. A link counts as complete only
/// once whitespace follows it, so a chunk boundary never yields a truncated one.
pub fn authorization_link(output: &str) -> Option<String> {
    let text = plain(output);
    let mut rest = text.as_str();
    while let Some(start) = rest.find("https://") {
        let candidate = &rest[start..];
        let end = candidate.find(char::is_whitespace)?;
        let link = &candidate[..end];
        let host = link["https://".len()..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        let path_ok = link["https://".len() + host.len()..].starts_with("/oauth/authorize");
        if path_ok
            && (host == "claude.ai"
                || host == "claude.com"
                || host.ends_with(".claude.ai")
                || host.ends_with(".claude.com"))
        {
            return Some(link.to_string());
        }
        rest = &candidate[end..];
    }
    None
}

/// What a login terminal printed, and how it ended once it did (`Some(None)`: it was lost).
#[derive(Default)]
pub(super) struct Output {
    pub(super) text: String,
    pub(super) exit: Option<Option<u32>>,
}

/// Collects the output of a login terminal on the machine. The Codex login uses it too.
pub(super) struct Sink {
    output: Mutex<Output>,
    changed: Condvar,
    seq: AtomicU64,
}

impl Sink {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Sink {
            output: Mutex::new(Output::default()),
            changed: Condvar::new(),
            seq: AtomicU64::new(0),
        })
    }

    /// Wake the waiters without output, so that they check a flag of their own.
    pub(super) fn wake(&self) {
        let _output = lock(&self.output);
        self.changed.notify_all();
    }

    pub(super) fn wait_for<T>(
        &self,
        timeout: Duration,
        mut done: impl FnMut(&Output) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut output = lock(&self.output);
        loop {
            if let Some(value) = done(&output) {
                return Some(value);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            output = self
                .changed
                .wait_timeout(output, left)
                .unwrap_or_else(|poison| poison.into_inner())
                .0;
        }
    }
}

impl TerminalSink for Sink {
    fn replay(&self, data: Vec<u8>, _reset: bool, seq: u64) {
        self.seq.store(seq, Ordering::Release);
        lock(&self.output).text = String::from_utf8_lossy(&data).into_owned();
        self.changed.notify_all();
    }
    fn output(&self, seq: u64, data: Vec<u8>) {
        if seq <= self.seq.swap(seq, Ordering::AcqRel) {
            return;
        }
        lock(&self.output)
            .text
            .push_str(&String::from_utf8_lossy(&data));
        self.changed.notify_all();
    }
    fn exit(&self, code: Option<u32>) {
        lock(&self.output).exit = Some(code);
        self.changed.notify_all();
    }
    fn lost(&self) {
        self.exit(None);
    }
    fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::Acquire)
    }
}

/// One sign-in in progress per machine.
#[derive(Default)]
pub struct Logins(Mutex<std::collections::HashMap<String, (String, Arc<Sink>)>>);

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStatus {
    pub installed: bool,
    pub logged_in: bool,
    pub email: Option<String>,
    pub method: Option<String>,
}

/// `claude auth status` on the machine. A missing CLI is reported, not an error.
pub fn status(connection: &Connection) -> Result<ClaudeStatus, String> {
    let ran = connection.exec(&["claude", "auth", "status"], None, Duration::from_secs(20))?;
    let parsed: Option<Value> = serde_json::from_str(ran.stdout.trim()).ok();
    let Some(value) = parsed else {
        return Ok(ClaudeStatus {
            installed: ran.code.is_some_and(|code| code != 127)
                && !ran.stderr.contains("not found"),
            logged_in: false,
            email: None,
            method: None,
        });
    };
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(ClaudeStatus {
        installed: true,
        logged_in: value.get("loggedIn").and_then(Value::as_bool) == Some(true),
        email: text("email"),
        method: text("authMethod").filter(|m| m != "none"),
    })
}

fn stop(connection: &Connection, id: &str) {
    connection.notify("terminal_signal", json!({"id": id, "signal": "kill"}));
    connection.notify("terminal_forget", json!({"id": id}));
}

/// Start `claude auth login` on the machine and return its authorization link.
#[tauri::command(async)]
pub fn machine_claude_login(app: AppHandle, id: String) -> Result<String, String> {
    let connection = app.state::<super::Service>().connected(&id)?;
    let logins = app.state::<Logins>();
    if let Some((terminal, _)) = lock(&logins.0).remove(&id) {
        stop(&connection, &terminal);
    }
    let (terminal, sink, link) = begin(&connection)?;
    lock(&logins.0).insert(id, (terminal, sink));
    Ok(link)
}

/// Open the login terminal on the machine and wait for its authorization link.
fn begin(connection: &Connection) -> Result<(String, Arc<Sink>, String), String> {
    let home = connection.home().unwrap_or_else(|| "/".into());
    let terminal = format!("claude-login-{}", uuid::Uuid::new_v4().simple());
    connection.request(
        "terminal_open",
        json!({"id": terminal, "argv": ["claude", "auth", "login", "--claudeai"], "cwd": home,
               "cols": COLUMNS, "rows": 40}),
    )?;
    let sink = Sink::new();
    connection.subscribe(&terminal, sink.clone());
    let link = sink.wait_for(LINK_WAIT, |output| {
        authorization_link(&output.text)
            .map(Ok)
            .or_else(|| output.exit.map(|_| Err(plain(&output.text))))
    });
    match link {
        Some(Ok(link)) => Ok((terminal, sink, link)),
        Some(Err(text)) => {
            stop(connection, &terminal);
            Err(crate::i18n::ta(
                "err.remote.login",
                &[("detail", tail(&text))],
            ))
        }
        None => {
            stop(connection, &terminal);
            Err(crate::i18n::ta(
                "err.remote.login",
                &[("detail", "timeout".to_string())],
            ))
        }
    }
}

fn tail(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(300)
        .collect()
}

/// Hand the code Claude showed after sign-in to the waiting login, then report the new status.
#[tauri::command(async)]
pub fn machine_claude_code(
    app: AppHandle,
    id: String,
    code: String,
) -> Result<ClaudeStatus, String> {
    let code = code.trim().to_string();
    if code.is_empty() || code.len() > 4096 || code.chars().any(|c| c.is_control()) {
        return Err(crate::i18n::t("err.remote.loginCode"));
    }
    let connection = app.state::<super::Service>().connected(&id)?;
    let (terminal, sink) = lock(&app.state::<Logins>().0)
        .remove(&id)
        .ok_or_else(|| crate::i18n::t("err.remote.loginExpired"))?;
    let before = lock(&sink.output).text.len();
    connection.request(
        "terminal_write",
        json!({"id": terminal, "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{code}\r"))}),
    )?;
    let exit = sink.wait_for(CODE_WAIT, |output| output.exit);
    let text = lock(&sink.output)
        .text
        .get(before..)
        .unwrap_or("")
        .to_string();
    stop(&connection, &terminal);
    let status = status(&connection)?;
    super::publish_claude(&app, &id, status.clone());
    match exit {
        Some(Some(0)) if status.logged_in => Ok(status),
        None => Err(crate::i18n::ta(
            "err.remote.login",
            &[("detail", "timeout".to_string())],
        )),
        _ => Err(crate::i18n::ta(
            "err.remote.login",
            &[("detail", tail(&plain(&text)))],
        )),
    }
}

/// Abandon a sign-in in progress.
#[tauri::command(async)]
pub fn machine_claude_login_cancel(app: AppHandle, id: String) {
    if let Some((terminal, _)) = lock(&app.state::<Logins>().0).remove(&id) {
        if let Some(connection) = app.state::<super::Service>().get(&id) {
            stop(&connection, &terminal);
        }
    }
}

/// Opt-in: `PROMETEU_REMOTE_TEST_SSH=vps cargo test --bin Prometeu vps_claude_login -- --ignored`.
/// Starts the real login on the machine, finds its link and cancels it.
#[cfg(test)]
mod journey {
    use super::*;
    use crate::remote::{Limits, Machine};

    #[test]
    #[ignore]
    fn vps_claude_login() {
        let Ok(target) = std::env::var("PROMETEU_REMOTE_TEST_SSH") else {
            return;
        };
        let connection = Connection::new(
            Machine {
                id: "journey".into(),
                name: "journey".into(),
                ssh: target,
                limits: Limits::default(),
            },
            Box::new(|| {}),
            Box::new(|_, _| {}),
        );
        connection.start();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !connection.connected() {
            assert!(Instant::now() < deadline, "not connected");
            std::thread::sleep(Duration::from_millis(100));
        }
        let current = status(&connection).expect("status");
        eprintln!("status: {current:?}");
        assert!(current.installed);
        let (terminal, _sink, link) = begin(&connection).expect("link");
        eprintln!("link: {link}");
        assert!(link.starts_with("https://claude.ai/oauth/authorize"));
        stop(&connection, &terminal);
        std::thread::sleep(Duration::from_millis(300));
        connection.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "Opening browser to sign in…\r\nIf the browser didn't open, visit:\r\n\x1b]8;;https://claude.ai/oauth/authorize?z-SbSK_g\x07\x1b[94mhttps://claude.ai/oauth/authorize?z-SbSK_g\x1b[39m\x1b]8;;\x07\r\nPaste code here if prompted > ";

    #[test]
    fn the_link_claude_prints_is_found_through_its_escapes() {
        assert_eq!(
            authorization_link(OUTPUT).as_deref(),
            Some("https://claude.ai/oauth/authorize?z-SbSK_g")
        );
        assert!(plain(OUTPUT).contains("Paste code here if prompted >"));
    }

    #[test]
    fn a_link_cut_by_a_chunk_boundary_waits_for_the_rest() {
        assert_eq!(
            authorization_link("visit: https://claude.ai/oauth/auth"),
            None
        );
    }

    #[test]
    fn other_hosts_and_paths_are_ignored() {
        assert_eq!(
            authorization_link("https://evil.example/oauth/authorize?x \n"),
            None
        );
        assert_eq!(authorization_link("https://claude.ai/docs \n"), None);
        assert_eq!(
            authorization_link("https://claude.ai.evil.example/oauth/authorize \n"),
            None
        );
    }
}
