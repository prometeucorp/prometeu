//! Signing Codex in on a remote machine from Settings (ADR 0105). The machine runs
//! `codex login --device-auth` in one of its host terminals, as the SSH user of the machine. This
//! app reads only the verification link and the one-time code that the CLI prints. The person
//! enters the code in their own browser, and Codex on the machine polls OpenAI, writes the
//! credential under its own `~/.codex` and exits. Nothing from this computer's Codex account goes
//! to the machine, and no output other than the challenge leaves this module.
use prometeu_core::lock::lock;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use super::codex::{self, CodexStatus};
use super::connection::{Connection, TerminalSink};
use super::login::{plain, Sink};

/// Codex asks OpenAI for the code before it prints anything.
const CHALLENGE_WAIT: Duration = Duration::from_secs(if cfg!(test) { 1 } else { 30 });
/// Codex stops polling 15 minutes after it requests the code (`device_code_auth.rs`, 0.172.0).
/// The extra minute covers a slow exit.
const LOGIN_WAIT: Duration = Duration::from_secs(16 * 60);
const END_WAIT: Duration = Duration::from_millis(if cfg!(test) { 300 } else { 5000 });
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

/// What a sign-in needs from the machine's connection. Tests drive the lifecycle through a fake.
pub(super) trait LoginHost: Send + Sync {
    fn connected(&self) -> bool;
    /// The SSH destination that the connection reaches now.
    fn destination(&self) -> String;
    fn home(&self) -> Option<String>;
    fn request(&self, method: &str, params: Value) -> Result<Value, String>;
    fn notify(&self, method: &str, params: Value);
    fn subscribe(&self, id: &str, sink: Arc<dyn TerminalSink>);
    fn release(&self, id: &str, sink: *const ());
}

impl LoginHost for Connection {
    fn connected(&self) -> bool {
        Connection::connected(self)
    }
    fn destination(&self) -> String {
        self.machine().ssh
    }
    fn home(&self) -> Option<String> {
        Connection::home(self)
    }
    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        Connection::request(self, method, params)
    }
    fn notify(&self, method: &str, params: Value) {
        Connection::notify(self, method, params)
    }
    fn subscribe(&self, id: &str, sink: Arc<dyn TerminalSink>) {
        Connection::subscribe(self, id, sink)
    }
    fn release(&self, id: &str, sink: *const ()) {
        Connection::release(self, id, sink)
    }
}

/// One sign-in attempt, bound to the connection and the destination that it started on. It is
/// registered before its terminal exists, so that a cancel always sees it.
#[derive(Clone)]
struct Pending {
    terminal: String,
    sink: Arc<Sink>,
    cancelled: Arc<AtomicBool>,
    /// `terminal_open` was sent: from then on, the terminal can exist on the machine.
    opened: Arc<AtomicBool>,
    started: Instant,
    host: Arc<dyn LoginHost>,
    ssh: String,
}

impl Pending {
    fn new(host: Arc<dyn LoginHost>) -> Self {
        Pending {
            terminal: format!("codex-login-{}", uuid::Uuid::new_v4().simple()),
            sink: Sink::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
            opened: Arc::new(AtomicBool::new(false)),
            started: Instant::now(),
            ssh: host.destination(),
            host,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// The attempt still belongs to the machine's current connection and destination. `now` is
    /// the connection that the machine has now, as a data pointer.
    fn current(&self, now: Option<*const ()>) -> bool {
        !self.cancelled()
            && now == Some(Arc::as_ptr(&self.host) as *const ())
            && self.host.destination() == self.ssh
    }

    /// Kill the terminal if it still runs, then forget it on the host and stop following it.
    /// False when the end could not be confirmed: the terminal can still run on the machine.
    fn end(&self) -> bool {
        if !self.opened.load(Ordering::Acquire) {
            return true;
        }
        let host = &*self.host;
        let mut ended = self.sink.wait_for(Duration::ZERO, |o| o.exit).is_some();
        if !ended && host.connected() {
            host.notify(
                "terminal_signal",
                json!({"id": self.terminal, "signal": "kill"}),
            );
            ended = self.sink.wait_for(END_WAIT, |o| o.exit).is_some();
        }
        if ended {
            host.notify("terminal_forget", json!({"id": self.terminal}));
        }
        host.release(&self.terminal, Arc::as_ptr(&self.sink) as *const ());
        ended
    }
}

/// The Codex sign-ins of this computer: one in progress per machine, and the login terminals whose
/// end this app could not confirm, per machine, with the destination they ran on.
#[derive(Default)]
pub struct CodexLogins {
    active: Mutex<HashMap<String, Pending>>,
    orphans: Mutex<HashMap<String, HashMap<String, String>>>,
}

impl CodexLogins {
    /// Register a new attempt and return the one that it replaces.
    fn register(&self, machine: &str, pending: Pending) -> Option<Pending> {
        lock(&self.active).insert(machine.into(), pending)
    }

    fn get(&self, machine: &str) -> Option<Pending> {
        lock(&self.active).get(machine).cloned()
    }

    /// Unregister `pending` if it is still the machine's attempt.
    fn forget(&self, machine: &str, pending: &Pending) {
        let mut active = lock(&self.active);
        if active
            .get(machine)
            .is_some_and(|p| p.terminal == pending.terminal)
        {
            active.remove(machine);
        }
    }

    /// End the terminal, or keep it for the sweep of the next attachment.
    fn finish(&self, machine: &str, pending: &Pending) {
        if !pending.end() {
            lock(&self.orphans)
                .entry(machine.into())
                .or_default()
                .insert(pending.terminal.clone(), pending.ssh.clone());
        }
    }

    fn cancel(&self, machine: &str, pending: &Pending) {
        pending.cancelled.store(true, Ordering::Release);
        self.finish(machine, pending);
        // Wake a waiter also when the machine is offline and the kill cannot arrive.
        pending.sink.wake();
    }

    fn abandon(&self, machine: &str) {
        let pending = lock(&self.active).remove(machine);
        if let Some(pending) = pending {
            self.cancel(machine, &pending);
        }
    }

    /// Open the login terminal and wait for its challenge. Each failure ends the terminal or keeps
    /// it as an orphan, also when the outcome of `terminal_open` is unknown.
    fn open(&self, machine: &str, pending: &Pending) -> Result<CodexChallenge, String> {
        let host = &*pending.host;
        let home = host.home().unwrap_or_else(|| "/".into());
        pending.opened.store(true, Ordering::Release);
        let opened = host.request(
            "terminal_open",
            json!({"id": pending.terminal, "argv": ["codex", "login", "--device-auth"],
                   "cwd": home, "cols": COLUMNS, "rows": 40}),
        );
        if let Err(error) = opened {
            self.finish(machine, pending);
            return Err(error);
        }
        host.subscribe(&pending.terminal, pending.sink.clone());
        let found = pending.sink.wait_for(CHALLENGE_WAIT, |output| {
            challenge(&output.text)
                .map(Some)
                .or_else(|| output.exit.map(|_| None))
                .or_else(|| pending.cancelled().then_some(None))
        });
        if pending.cancelled() {
            // A cancel during `terminal_open` could not reach the terminal yet.
            self.finish(machine, pending);
            return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
        }
        match found {
            Some(Some(challenge)) => Ok(challenge),
            _ => {
                self.finish(machine, pending);
                Err(crate::i18n::t("err.remote.codexLoginStart"))
            }
        }
    }

    /// Stop the login terminals left on the machine, after an attachment. An orphan of this app
    /// stops at once. A running login terminal older than [`LOGIN_WAIT`] is left over on any
    /// computer, because each computer stops its own one at that age.
    fn sweep(&self, machine: &str, host: &dyn LoginHost, now: u64) {
        let Ok(list) = host.request("terminal_list", json!({})) else {
            return;
        };
        let destination = host.destination();
        let ours: HashSet<String> = lock(&self.orphans)
            .remove(machine)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, ssh)| *ssh == destination)
            .map(|(terminal, _)| terminal)
            .collect();
        let active: HashSet<String> = self.get(machine).map(|p| p.terminal).into_iter().collect();
        let mut kept = HashMap::new();
        for (terminal, running) in stale(&list, &ours, &active, now) {
            if running {
                host.notify("terminal_signal", json!({"id": terminal, "signal": "kill"}));
            }
            if !running || stopped(host, &terminal) {
                host.notify("terminal_forget", json!({"id": terminal}));
            } else if ours.contains(&terminal) {
                kept.insert(terminal, destination.clone());
            }
        }
        if !kept.is_empty() {
            lock(&self.orphans)
                .entry(machine.into())
                .or_default()
                .extend(kept);
        }
    }
}

/// Wait a short time until the host lists `terminal` as stopped or no longer lists it.
fn stopped(host: &dyn LoginHost, terminal: &str) -> bool {
    let deadline = Instant::now() + END_WAIT;
    loop {
        let Ok(list) = host.request("terminal_list", json!({})) else {
            return false;
        };
        let running = list
            .as_array()
            .and_then(|all| all.iter().find(|t| t["id"] == json!(terminal)))
            .is_some_and(|t| t["running"] == json!(true));
        if !running {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The login terminals in a `terminal_list` reply that the sweep ends, with whether each runs.
/// The attempt in progress on this computer is never one of them.
fn stale(
    list: &Value,
    ours: &HashSet<String>,
    active: &HashSet<String>,
    now: u64,
) -> Vec<(String, bool)> {
    let Some(all) = list.as_array() else {
        return Vec::new();
    };
    all.iter()
        .filter_map(|t| {
            let id = t["id"].as_str()?;
            if !id.starts_with("codex-login-") || active.contains(id) {
                return None;
            }
            let running = t["running"] == json!(true);
            let old = t["started"]
                .as_u64()
                .is_some_and(|s| now.saturating_sub(s) >= LOGIN_WAIT.as_secs());
            (!running || ours.contains(id) || old).then(|| (id.to_string(), running))
        })
        .collect()
}

/// The machine's connection now, as a data pointer for [`Pending::current`].
fn connection_now(app: &AppHandle, id: &str) -> Option<*const ()> {
    app.state::<super::Service>()
        .get(id)
        .map(|c| Arc::as_ptr(&c) as *const ())
}

/// Stop the sign-in in progress on a machine, if there is one.
pub(crate) fn abandon(app: &AppHandle, id: &str) {
    app.state::<CodexLogins>().abandon(id);
}

/// End the login terminals left on the machine after an attachment.
pub(crate) fn sweep(app: &AppHandle, connection: &Connection) {
    app.state::<CodexLogins>()
        .sweep(&connection.id, connection, super::now_secs());
}

/// Start `codex login --device-auth` on the machine and return its challenge. `None` means that
/// Codex is already signed in there: Codex removes the stored credential when a login starts, so a
/// sign-in that another computer finished must not be replaced.
#[tauri::command(async)]
pub fn machine_codex_login(app: AppHandle, id: String) -> Result<Option<CodexChallenge>, String> {
    let service = app.state::<super::Service>();
    let connection = service.connected(&id)?;
    let name = service.machine(&id).map_or_else(|| id.clone(), |m| m.name);
    let logins = app.state::<CodexLogins>();
    let pending = Pending::new(connection.clone());
    if let Some(replaced) = logins.register(&id, pending.clone()) {
        logins.cancel(&id, &replaced);
    }
    let started = (|| {
        let status = codex::status(&connection)?;
        if !pending.current(connection_now(&app, &id)) {
            return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
        }
        super::publish_codex(&app, &id, status.clone());
        if status.logged_in {
            return Ok(None);
        }
        if !status.installed || !status.supported {
            return Err(codex::problem(&status, &name).unwrap_or_default());
        }
        logins.open(&id, &pending).map(Some)
    })();
    if !matches!(started, Ok(Some(_))) {
        logins.forget(&id, &pending);
    }
    started
}

/// Wait until the sign-in in progress ends, then return the new Codex status of the machine.
#[tauri::command]
pub async fn machine_codex_login_wait(app: AppHandle, id: String) -> Result<CodexStatus, String> {
    tauri::async_runtime::spawn_blocking(move || wait(&app, &id))
        .await
        .map_err(crate::i18n::io)?
}

fn wait(app: &AppHandle, id: &str) -> Result<CodexStatus, String> {
    let logins = app.state::<CodexLogins>();
    let pending = logins
        .get(id)
        .ok_or_else(|| crate::i18n::t("err.remote.loginExpired"))?;
    // An SSH loss does not end the wait: the subscription attaches again with the connection.
    let exit = pending
        .sink
        .wait_for(LOGIN_WAIT.saturating_sub(pending.started.elapsed()), |o| {
            o.exit.or_else(|| pending.cancelled().then_some(None))
        });
    let timed_out = pending
        .sink
        .wait_for(Duration::ZERO, |o| {
            Some(plain(&o.text).contains("timed out"))
        })
        .unwrap_or(false);
    logins.forget(id, &pending);
    if pending.cancelled() {
        return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
    }
    logins.finish(id, &pending);
    let connection = app.state::<super::Service>().connected(id)?;
    // The status of a different destination is not the result of this attempt.
    if !pending.current(Some(Arc::as_ptr(&connection) as *const ())) {
        return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
    }
    let status = codex::status(&connection)?;
    if !pending.current(connection_now(app, id)) {
        return Err(crate::i18n::t("err.remote.codexLoginCancelled"));
    }
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

    /// A machine connection that records what the sign-in sends. A kill ends the subscribed
    /// terminal; `on_open` runs while `terminal_open` is in flight.
    #[derive(Default)]
    struct FakeHost {
        connected: AtomicBool,
        destination: Mutex<String>,
        open_fails: AtomicBool,
        sent: Mutex<Vec<String>>,
        sinks: Mutex<HashMap<String, Arc<dyn TerminalSink>>>,
        running: Mutex<HashMap<String, (bool, u64)>>,
        on_open: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl FakeHost {
        fn new(destination: &str) -> Arc<Self> {
            let host = FakeHost::default();
            host.connected.store(true, Ordering::Release);
            *lock(&host.destination) = destination.into();
            Arc::new(host)
        }
        fn sent(&self) -> Vec<String> {
            lock(&self.sent).clone()
        }
    }

    impl LoginHost for FakeHost {
        fn connected(&self) -> bool {
            self.connected.load(Ordering::Acquire)
        }
        fn destination(&self) -> String {
            lock(&self.destination).clone()
        }
        fn home(&self) -> Option<String> {
            Some("/home/me".into())
        }
        fn request(&self, method: &str, params: Value) -> Result<Value, String> {
            if method == "terminal_list" {
                let running = lock(&self.running);
                return Ok(Value::Array(
                    running
                        .iter()
                        .map(|(id, (run, started))| json!({"id": id, "running": run, "started": started}))
                        .collect(),
                ));
            }
            let id = params["id"].as_str().unwrap_or_default().to_string();
            lock(&self.sent).push(format!("{method} {id}"));
            if let Some(hook) = lock(&self.on_open).take() {
                hook();
            }
            if self.open_fails.load(Ordering::Acquire) {
                // The request reached the machine, but the reply was lost with SSH.
                lock(&self.running).insert(id, (true, 0));
                return Err("offline".into());
            }
            lock(&self.running).insert(id, (true, 0));
            Ok(json!({}))
        }
        fn notify(&self, method: &str, params: Value) {
            let id = params["id"].as_str().unwrap_or_default().to_string();
            lock(&self.sent).push(format!("{method} {id}"));
            if !self.connected() {
                return;
            }
            if method == "terminal_signal" {
                if let Some(state) = lock(&self.running).get_mut(&id) {
                    state.0 = false;
                }
                if let Some(sink) = lock(&self.sinks).get(&id) {
                    sink.exit(Some(137));
                }
            }
            if method == "terminal_forget" {
                lock(&self.running).remove(&id);
            }
        }
        fn subscribe(&self, id: &str, sink: Arc<dyn TerminalSink>) {
            lock(&self.sinks).insert(id.into(), sink);
        }
        fn release(&self, id: &str, _sink: *const ()) {
            lock(&self.sinks).remove(id);
        }
    }

    fn orphans(logins: &CodexLogins, machine: &str) -> Vec<String> {
        lock(&logins.orphans)
            .get(machine)
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn an_open_whose_reply_was_lost_is_swept_at_the_next_attachment() {
        let host = FakeHost::new("vps");
        host.open_fails.store(true, Ordering::Release);
        host.connected.store(false, Ordering::Release);
        let logins = CodexLogins::default();
        let pending = Pending::new(host.clone());
        logins.register("vps", pending.clone());
        assert!(logins.open("vps", &pending).is_err());
        assert_eq!(orphans(&logins, "vps"), vec![pending.terminal.clone()]);

        // The terminal is young, so only the record of this app makes the sweep stop it.
        host.connected.store(true, Ordering::Release);
        logins.forget("vps", &pending);
        logins.sweep("vps", &*host, 10);
        assert!(host
            .sent()
            .contains(&format!("terminal_signal {}", pending.terminal)));
        assert!(host
            .sent()
            .contains(&format!("terminal_forget {}", pending.terminal)));
        assert!(!lock(&host.running).contains_key(&pending.terminal));
        assert!(orphans(&logins, "vps").is_empty());
    }

    #[test]
    fn a_cancel_while_the_terminal_opens_stops_it_and_hides_the_challenge() {
        let host = FakeHost::new("vps");
        let logins = Arc::new(CodexLogins::default());
        let pending = Pending::new(host.clone());
        logins.register("vps", pending.clone());
        // The machine is removed while `terminal_open` waits for its reply.
        let remover = logins.clone();
        *lock(&host.on_open) = Some(Box::new(move || remover.abandon("vps")));
        assert_eq!(
            logins.open("vps", &pending),
            Err(crate::i18n::t("err.remote.codexLoginCancelled"))
        );
        assert!(host
            .sent()
            .contains(&format!("terminal_signal {}", pending.terminal)));
        assert!(!lock(&host.running).contains_key(&pending.terminal));
        assert!(logins.get("vps").is_none());
    }

    #[test]
    fn a_challenge_wait_cut_by_ssh_keeps_the_terminal_for_the_sweep() {
        let host = FakeHost::new("vps");
        let logins = CodexLogins::default();
        let pending = Pending::new(host.clone());
        logins.register("vps", pending.clone());
        *lock(&host.on_open) = Some(Box::new({
            let host = host.clone();
            move || host.connected.store(false, Ordering::Release)
        }));
        assert_eq!(
            logins.open("vps", &pending),
            Err(crate::i18n::t("err.remote.codexLoginStart"))
        );
        assert_eq!(orphans(&logins, "vps"), vec![pending.terminal.clone()]);
    }

    #[test]
    fn a_login_cancelled_while_ssh_is_down_is_stopped_after_the_reconnection() {
        let host = FakeHost::new("vps");
        let logins = CodexLogins::default();
        let pending = Pending::new(host.clone());
        logins.register("vps", pending.clone());
        // The login started and showed its challenge.
        pending.opened.store(true, Ordering::Release);
        lock(&host.running).insert(pending.terminal.clone(), (true, 10));
        host.subscribe(&pending.terminal, pending.sink.clone());

        host.connected.store(false, Ordering::Release);
        logins.abandon("vps");
        assert!(pending.cancelled());
        assert_eq!(
            lock(&host.running).get(&pending.terminal),
            Some(&(true, 10))
        );
        assert_eq!(orphans(&logins, "vps"), vec![pending.terminal.clone()]);

        host.connected.store(true, Ordering::Release);
        logins.sweep("vps", &*host, 20);
        assert!(!lock(&host.running).contains_key(&pending.terminal));
        assert!(host.sent().ends_with(&[
            format!("terminal_signal {}", pending.terminal),
            format!("terminal_forget {}", pending.terminal),
        ]));
        assert!(orphans(&logins, "vps").is_empty());
    }

    #[test]
    fn an_attempt_belongs_only_to_its_connection_and_destination() {
        let host = FakeHost::new("vps");
        let pending = Pending::new(host.clone());
        let origin = Some(Arc::as_ptr(&host) as *const ());
        assert!(pending.current(origin));
        assert!(!pending.current(None));
        let other = FakeHost::new("vps");
        assert!(!pending.current(Some(Arc::as_ptr(&other) as *const ())));
        *lock(&host.destination) = "other".into();
        assert!(!pending.current(origin));
        *lock(&host.destination) = "vps".into();
        pending.cancelled.store(true, Ordering::Release);
        assert!(!pending.current(origin));
    }

    #[test]
    fn the_sweep_stops_orphans_and_old_logins_but_not_logins_in_progress() {
        let now = 10_000;
        let list = json!([
            {"id": "codex-login-ours", "running": true, "started": now - 5},
            {"id": "codex-login-young", "running": true, "started": now - 5},
            {"id": "codex-login-old", "running": true, "started": now - LOGIN_WAIT.as_secs()},
            {"id": "codex-login-ended", "running": false, "started": now - 5},
            {"id": "codex-login-active", "running": true, "started": now - LOGIN_WAIT.as_secs()},
            {"id": "tab-1", "running": true, "started": 0},
        ]);
        let ours = HashSet::from(["codex-login-ours".to_string()]);
        let active = HashSet::from(["codex-login-active".to_string()]);
        let mut found = stale(&list, &ours, &active, now);
        found.sort();
        assert_eq!(
            found,
            vec![
                ("codex-login-ended".to_string(), false),
                ("codex-login-old".to_string(), true),
                ("codex-login-ours".to_string(), true),
            ]
        );
    }

    #[test]
    fn an_orphan_on_an_earlier_destination_is_not_stopped_on_the_new_one() {
        let host = FakeHost::new("old");
        host.open_fails.store(true, Ordering::Release);
        host.connected.store(false, Ordering::Release);
        let logins = CodexLogins::default();
        let pending = Pending::new(host.clone());
        let _ = logins.open("vps", &pending);
        *lock(&host.destination) = "new".into();
        host.connected.store(true, Ordering::Release);
        lock(&host.running).clear();
        lock(&host.sent).clear();
        logins.sweep("vps", &*host, 10);
        assert!(host.sent().is_empty());
        assert!(orphans(&logins, "vps").is_empty());
    }
}
