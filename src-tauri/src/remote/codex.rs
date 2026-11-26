//! Codex conversations on a remote machine (ADR 0105). The conversation runs as a local Codex
//! terminal conversation does: a private `codex app-server` owns the thread and the native TUI
//! attaches to it with `--remote`. Both run on the machine as host terminals, under the machine's
//! own Codex sign-in. This app reads the engine's RPC stream through a relay terminal
//! (`prometeu-host codex-relay`), so the stream survives a dropped SSH connection like any
//! terminal output.
use prometeu_core::lock::lock;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

use super::connection::{Connection, TerminalSink};
use super::terminal::Remote;

/// The first release whose private engine protocol Prometeu follows, as for local Codex.
pub const MINIMUM: &str = "0.169.0";

/// An engine started before its TUI opened belongs to a launch still bootstrapping, maybe on
/// another computer (ADR 0104). Only older ones without a TUI are left over.
const ORPHAN_GRACE_SECS: u64 = 60;

/// A link this short was a failed adoption, not a working conversation: do not follow it again
/// on its own.
const READOPT_AFTER: Duration = Duration::from_secs(10);

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub installed: bool,
    pub version: Option<String>,
    /// The installed release is at least [`MINIMUM`].
    pub supported: bool,
    pub logged_in: bool,
    /// [`MINIMUM`], for the interface's explanation.
    pub minimum: String,
}

/// `codex --version` and `codex login status` on the machine. Only exit codes and the version
/// line are read: the status text may describe the credential.
pub fn status(connection: &Connection) -> Result<CodexStatus, String> {
    let ran = connection.exec(&["codex", "--version"], None, Duration::from_secs(20))?;
    if ran.code != Some(0) {
        return Ok(CodexStatus {
            installed: false,
            version: None,
            supported: false,
            logged_in: false,
            minimum: MINIMUM.into(),
        });
    }
    let version = ran.stdout.trim().to_string();
    let supported = prometeu_protocols::cli_version::at_least(&version, MINIMUM);
    let login = connection.exec(&["codex", "login", "status"], None, Duration::from_secs(20))?;
    Ok(CodexStatus {
        installed: true,
        version: Some(version),
        supported,
        logged_in: login.code == Some(0),
        minimum: MINIMUM.into(),
    })
}

/// What keeps a conversation from starting, as an error the person can act on.
pub fn problem(status: &CodexStatus, machine: &str) -> Option<String> {
    if !status.installed {
        return Some(crate::i18n::ta(
            "err.remote.codexMissing",
            &[("name", machine.to_string())],
        ));
    }
    if !status.supported {
        return Some(crate::i18n::ta(
            "err.remote.codexVersion",
            &[
                ("name", machine.to_string()),
                ("version", status.version.clone().unwrap_or_default()),
                ("minimum", MINIMUM.to_string()),
            ],
        ));
    }
    if !status.logged_in {
        return Some(crate::i18n::ta(
            "err.remote.codexSignIn",
            &[("name", machine.to_string())],
        ));
    }
    None
}

/// The running terminal named `id` in a `terminal_list` reply.
fn listed<'a>(list: &'a Value, id: &str) -> Option<&'a Value> {
    list.as_array()?.iter().find(|t| t["id"] == json!(id))
}

/// A conversation's engine on the machine, and this app's relay to it.
pub struct RemoteEngine {
    pub app: AppHandle,
    pub connection: Arc<Connection>,
    pub session: String,
    pub workspace: String,
    pub root: String,
    pub home: String,
    /// This build's host executable on the machine, which provides the relay.
    pub host_binary: String,
    pub limits: super::Limits,
    /// The machine already runs this conversation: connect to its engine instead of starting one.
    pub adopt: bool,
    /// The conversation's TUI terminal.
    pub tui: Arc<Remote>,
    relay: Option<(String, Arc<RelayStream>)>,
    linked: Option<Instant>,
    /// This instance opened the engine. Another computer resuming the same conversation at the
    /// same moment gets `terminal_exists` and must never end the engine it did not start.
    owns_engine: bool,
}

impl RemoteEngine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        app: AppHandle,
        connection: Arc<Connection>,
        session: String,
        workspace: String,
        root: String,
        home: String,
        host_binary: String,
        limits: super::Limits,
        adopt: bool,
        tui: Arc<Remote>,
    ) -> Self {
        Self {
            app,
            connection,
            session,
            workspace,
            root,
            home,
            host_binary,
            limits,
            adopt,
            tui,
            relay: None,
            linked: None,
            owns_engine: false,
        }
    }

    fn engine_id(&self) -> String {
        prometeu_host::codex::engine_terminal(&self.session)
    }

    /// Start the engine terminal and return its process ID.
    fn open_engine(&self, command: &Command, socket: &str) -> Result<u64, String> {
        // The engine listens in the conversation's private session directory, created 0700.
        self.connection.write_file(
            &format!("{}/sessions/{}/codex", self.root, self.session),
            b"",
            0o600,
        )?;
        let mut argv = vec![command.get_program().to_string_lossy().to_string()];
        argv.extend(command.get_args().map(|a| a.to_string_lossy().to_string()));
        argv.push("--listen".into());
        argv.push(format!("unix://{socket}"));
        let mut env = Map::new();
        let mut unset = Vec::new();
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    env.insert(
                        key.to_string_lossy().to_string(),
                        json!(value.to_string_lossy()),
                    );
                }
                None => unset.push(key.to_string_lossy().to_string()),
            }
        }
        let cwd = command
            .get_current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| self.home.clone());
        let params = json!({
            "id": self.engine_id(), "workspace": self.workspace, "argv": argv, "cwd": cwd,
            "env": env, "unset": unset, "cols": 80, "rows": 24, "limits": self.limits.to_wire(),
        });
        let opened = match self.connection.request("terminal_open", params.clone()) {
            Err(error) if error == "terminal_exists" && self.clear_stale_engine()? => {
                self.connection.request("terminal_open", params)
            }
            other => other,
        };
        match opened {
            Ok(reply) => reply["pid"]
                .as_u64()
                .filter(|pid| *pid > 0)
                .ok_or_else(|| crate::i18n::t("err.terminalAgent.engine")),
            Err(error) if error == "terminal_exists" => {
                Err(crate::i18n::t("err.remote.startedElsewhere"))
            }
            Err(error) => Err(error),
        }
    }

    /// End an engine left from an earlier launch whose TUI is gone. Returns whether the name is
    /// free again.
    fn clear_stale_engine(&self) -> Result<bool, String> {
        let list = self.connection.request("terminal_list", json!({}))?;
        let Some(engine) = listed(&list, &self.engine_id()) else {
            return Ok(true);
        };
        if !orphan(engine, listed(&list, &self.session), super::now_secs()) {
            return Ok(false);
        }
        self.connection.notify(
            "terminal_signal",
            json!({"id": self.engine_id(), "signal": "kill"}),
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            let list = self.connection.request("terminal_list", json!({}))?;
            if listed(&list, &self.engine_id()).is_none_or(|t| t["running"] != json!(true)) {
                let _ = self
                    .connection
                    .request("terminal_forget", json!({"id": self.engine_id()}));
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(false)
    }

    /// The running engine's process ID, when the machine still runs it. A TUI whose engine is
    /// gone has nothing to show; it is ended so the next launch resumes the thread in a new one.
    fn running_engine(&self) -> Result<u64, String> {
        let list = self.connection.request("terminal_list", json!({}))?;
        let pid = listed(&list, &self.engine_id())
            .filter(|t| t["running"] == json!(true))
            .and_then(|t| t["pid"].as_u64())
            .filter(|pid| *pid > 0);
        pid.ok_or_else(|| {
            self.connection.notify(
                "terminal_signal",
                json!({"id": self.session, "signal": "terminate"}),
            );
            crate::i18n::t("err.remote.codexEngineLost")
        })
    }

    fn open_relay(&self, socket: &str, pid: u64) -> Result<(String, Arc<RelayStream>), String> {
        let id = prometeu_host::codex::relay_terminal(
            &self.session,
            &uuid::Uuid::new_v4().simple().to_string(),
        );
        self.connection.request(
            "terminal_open",
            json!({
                "id": id, "cwd": self.home, "cols": 80, "rows": 24,
                "argv": [self.host_binary, "codex-relay", "--socket", socket, "--pid", pid.to_string()],
            }),
        )?;
        let stream = Arc::new(RelayStream::default());
        self.connection.subscribe(&id, stream.clone());
        Ok((id, stream))
    }
}

impl RemoteEngine {
    /// Drop a terminal this app just ended once it had time to exit. A terminal still running
    /// then is kept by the host and forgotten at the next attachment instead.
    fn forget_later(&self, id: String) {
        let connection = self.connection.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            connection.notify("terminal_forget", json!({"id": id}));
        });
    }
}

impl crate::terminal_agent_codex::EngineHost for RemoteEngine {
    fn connect(
        &mut self,
        command: &Command,
        _deadline: Instant,
    ) -> Result<crate::terminal_agent_codex::EngineLink, String> {
        let socket = prometeu_host::codex::socket_path(&self.root, &self.session);
        let pid = if self.adopt {
            self.running_engine()?
        } else {
            let pid = self.open_engine(command, &socket)?;
            self.owns_engine = true;
            pid
        };
        let (id, stream) = self.open_relay(&socket, pid)?;
        self.relay = Some((id.clone(), stream.clone()));
        self.linked = Some(Instant::now());
        Ok(crate::terminal_agent_codex::EngineLink {
            address: format!("unix://{socket}"),
            reader: Box::new(RelayReader(stream)),
            writer: Box::new(RelayWriter {
                input: self.connection.clone(),
                id,
            }),
        })
    }
}

impl Drop for RemoteEngine {
    fn drop(&mut self) {
        if let Some((id, stream)) = self.relay.take() {
            // This app's relay serves only this incarnation; another computer has its own.
            stream.end();
            self.connection
                .release(&id, Arc::as_ptr(&stream) as *const ());
            self.connection
                .notify("terminal_signal", json!({"id": id, "signal": "kill"}));
            self.forget_later(id);
        }
        // A fresh launch that never reached its TUI, or a TUI that exited, leaves the engine
        // nothing to serve. An adopted TUI that is still running keeps it.
        if ends_engine(self.owns_engine, self.tui.followed(), self.tui.running()) {
            self.connection.notify(
                "terminal_signal",
                json!({"id": self.engine_id(), "signal": "terminate"}),
            );
            self.forget_later(self.engine_id());
            return;
        }
        // The link was lost while the machine keeps working (a gap in the relay's output, or the
        // relay itself ended): follow the conversation again once this incarnation is gone.
        let lasted = self.linked.is_some_and(|at| at.elapsed() >= READOPT_AFTER);
        if !lasted || crate::terminal_agent_codex::closing() {
            return;
        }
        readopt(
            self.app.clone(),
            self.connection.clone(),
            self.session.clone(),
        );
    }
}

/// Whether dropping this app's side ends the machine's engine. The engine has nothing to serve
/// once the TUI this side followed exited, or when this side started it and never reached its
/// TUI. An engine another launch started (a resume racing on another computer, or an adoption)
/// is never ended for a launch that did not get as far as following its TUI.
pub(super) fn ends_engine(owns: bool, followed: bool, tui_running: bool) -> bool {
    (owns && !followed) || (followed && !tui_running)
}

/// Resume a conversation whose TUI still runs on the machine once its previous incarnation left
/// the live conversations, as reattaching does after a reconnection.
fn readopt(app: AppHandle, connection: Arc<Connection>, session: String) {
    std::thread::spawn(move || {
        let state = app.state::<crate::AppState>();
        let deadline = Instant::now() + Duration::from_secs(30);
        while lock(&state.sessions.conversations).contains_key(&session) {
            if Instant::now() >= deadline || crate::terminal_agent_codex::closing() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if !connection.connected() {
            // The next attachment resumes it (`reattach_running`).
            return;
        }
        let running = connection
            .request("terminal_list", json!({}))
            .ok()
            .is_some_and(|list| {
                listed(&list, &session).is_some_and(|t| t["running"] == json!(true))
            });
        if running {
            let _ = crate::session::resume_conversation(&app, &session);
        }
    });
}

/// Whether `engine` is left over: running without its conversation's TUI, and older than a
/// launch's bootstrap.
pub fn orphan(engine: &Value, tui: Option<&Value>, now: u64) -> bool {
    engine["running"] == json!(true)
        && tui.is_none_or(|t| t["running"] != json!(true))
        && engine["started"]
            .as_u64()
            .is_some_and(|started| now.saturating_sub(started) >= ORPHAN_GRACE_SECS)
}

#[derive(Default)]
struct RelayState {
    chunks: VecDeque<Vec<u8>>,
    ended: bool,
}

/// A relay terminal's output as one ordered byte stream. A gap the host could not replay ends it:
/// the RPC stream would be missing frames.
#[derive(Default)]
pub(super) struct RelayStream {
    state: Mutex<RelayState>,
    ready: Condvar,
    seq: AtomicU64,
}

impl RelayStream {
    fn push(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        let mut state = lock(&self.state);
        if !state.ended {
            state.chunks.push_back(bytes);
        }
        self.ready.notify_all();
    }

    fn end(&self) {
        lock(&self.state).ended = true;
        self.ready.notify_all();
    }
}

impl TerminalSink for RelayStream {
    fn replay(&self, data: Vec<u8>, reset: bool, seq: u64) {
        if reset && self.seq.load(Ordering::Acquire) > 0 {
            self.end();
            return;
        }
        self.seq.store(seq, Ordering::Release);
        self.push(data);
    }

    fn output(&self, seq: u64, data: Vec<u8>) {
        if seq <= self.seq.load(Ordering::Acquire) {
            return;
        }
        self.seq.store(seq, Ordering::Release);
        self.push(data);
    }

    fn exit(&self, _code: Option<u32>) {
        self.end();
    }

    fn lost(&self) {
        self.end();
    }

    fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::Acquire)
    }
}

struct RelayReader(Arc<RelayStream>);
impl Read for RelayReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let mut state = lock(&self.0.state);
        loop {
            if let Some(mut chunk) = state.chunks.pop_front() {
                let count = chunk.len().min(buffer.len());
                buffer[..count].copy_from_slice(&chunk[..count]);
                if count < chunk.len() {
                    state.chunks.push_front(chunk.split_off(count));
                }
                return Ok(count);
            }
            if state.ended {
                return Ok(0);
            }
            state = self
                .0
                .ready
                .wait(state)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }
}

/// The longest a relay write waits for the host's confirmation, below the engine's own
/// five-second control deadline.
const WRITE_WAIT: Duration = Duration::from_secs(4);

/// Where relay input goes: the machine's host, which confirms each write.
pub(super) trait RelayInput: Send + Sync {
    fn write_relay(&self, id: &str, bytes: &[u8]) -> Result<(), String>;
}

impl RelayInput for Connection {
    /// A request rather than a notification: a frame written into an SSH connection that is
    /// already dead fails here instead of passing for sent.
    fn write_relay(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
        self.request_timeout(
            "terminal_write",
            json!({"id": id, "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)}),
            WRITE_WAIT,
        )
        .map(|_| ())
    }
}

/// Requests and replies to the engine. A write the host did not confirm fails, so an approval
/// answer stays open and can be sent again after reconnecting; the relay keeps the link.
struct RelayWriter {
    input: Arc<dyn RelayInput>,
    id: String,
}
impl Write for RelayWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.input
            .write_relay(&self.id, bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::NotConnected, error))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(stream: &Arc<RelayStream>) -> Vec<u8> {
        let mut out = Vec::new();
        RelayReader(stream.clone()).read_to_end(&mut out).unwrap();
        out
    }

    #[test]
    fn a_resume_that_lost_the_race_never_ends_the_winners_engine() {
        // Two computers resume the same conversation: the loser got `terminal_exists`.
        assert!(!ends_engine(false, false, true));
        // The winner failed before its TUI: its own engine has nothing to serve.
        assert!(ends_engine(true, false, true));
        // An adoption that failed before following the TUI leaves the running engine alone.
        assert!(!ends_engine(false, false, true));
        // Following a TUI that is still running (app exit, lost link) keeps the engine.
        assert!(!ends_engine(true, true, true));
        assert!(!ends_engine(false, true, true));
        // A followed TUI that exited ends the engine, whoever started it.
        assert!(ends_engine(true, true, false));
        assert!(ends_engine(false, true, false));
    }

    use std::sync::atomic::AtomicBool;

    /// A host connection whose writes fail until it reconnects.
    #[derive(Default)]
    struct Flaky {
        offline: AtomicBool,
        written: Mutex<Vec<(String, Vec<u8>)>>,
    }
    impl RelayInput for Flaky {
        fn write_relay(&self, id: &str, bytes: &[u8]) -> Result<(), String> {
            if self.offline.load(Ordering::Acquire) {
                return Err("offline".into());
            }
            lock(&self.written).push((id.into(), bytes.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn a_relay_write_the_host_did_not_confirm_fails_and_can_be_sent_again() {
        let input = Arc::new(Flaky::default());
        let mut writer = RelayWriter {
            input: input.clone(),
            id: "s-rpc-1".into(),
        };
        input.offline.store(true, Ordering::Release);
        let error = writer.write_all(b"{\"id\":7,\"result\":{}}\n").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);
        assert!(lock(&input.written).is_empty());
        input.offline.store(false, Ordering::Release);
        writer.write_all(b"{\"id\":7,\"result\":{}}\n").unwrap();
        assert_eq!(
            *lock(&input.written),
            vec![(
                "s-rpc-1".to_string(),
                b"{\"id\":7,\"result\":{}}\n".to_vec()
            )]
        );
    }

    #[test]
    fn relay_output_follows_sequences_and_ends_with_the_terminal() {
        let stream = Arc::new(RelayStream::default());
        stream.replay(b"ready\n".to_vec(), false, 1);
        stream.output(1, b"duplicate".to_vec());
        stream.output(2, b"{\"a\":1}\n".to_vec());
        // A reattachment replays only the gap.
        stream.replay(b"{\"b\":2}\n".to_vec(), false, 3);
        stream.exit(Some(0));
        assert_eq!(read_all(&stream), b"ready\n{\"a\":1}\n{\"b\":2}\n");
    }

    #[test]
    fn a_gap_the_host_could_not_replay_ends_the_relay_stream() {
        let stream = Arc::new(RelayStream::default());
        stream.replay(b"ready\n".to_vec(), false, 1);
        stream.replay(b"partial-line\n".to_vec(), true, 9);
        stream.output(10, b"later".to_vec());
        assert_eq!(read_all(&stream), b"ready\n");
    }

    #[test]
    fn a_first_attachment_is_never_a_gap() {
        let stream = Arc::new(RelayStream::default());
        stream.replay(b"ready\n".to_vec(), true, 1);
        stream.lost();
        assert_eq!(read_all(&stream), b"ready\n");
    }

    #[test]
    fn only_old_engines_without_a_running_tui_are_orphans() {
        let engine = |started: u64| json!({"id": "s-codex", "running": true, "started": started});
        let tui = |running: bool| json!({"id": "s", "running": running});
        assert!(orphan(&engine(100), None, 200));
        assert!(orphan(&engine(100), Some(&tui(false)), 200));
        assert!(!orphan(&engine(100), Some(&tui(true)), 200));
        // Another launch may still be bootstrapping it.
        assert!(!orphan(&engine(190), None, 200));
        assert!(!orphan(
            &json!({"id": "s-codex", "running": false, "started": 0}),
            None,
            200
        ));
    }

    #[test]
    fn status_problems_name_what_to_do_first() {
        let ok = CodexStatus {
            installed: true,
            version: Some("codex-cli 0.170.0".into()),
            supported: true,
            logged_in: true,
            minimum: MINIMUM.into(),
        };
        assert_eq!(problem(&ok, "vps"), None);
        let missing = CodexStatus {
            installed: false,
            version: None,
            supported: false,
            logged_in: false,
            minimum: MINIMUM.into(),
        };
        assert!(problem(&missing, "vps")
            .unwrap()
            .contains("err.remote.codexMissing"));
        let old = CodexStatus {
            supported: false,
            version: Some("codex-cli 0.100.0".into()),
            ..ok.clone()
        };
        assert!(problem(&old, "vps")
            .unwrap()
            .contains("err.remote.codexVersion"));
        let signed_out = CodexStatus {
            logged_in: false,
            ..ok
        };
        assert!(problem(&signed_out, "vps")
            .unwrap()
            .contains("err.remote.codexSignIn"));
    }
}
