//! The resident host. It owns terminals, retains hooks for desktops that are away and streams
//! transcript files, so execution never depends on a desktop's connection. Several desktops may be
//! attached at once, one per device (ADR 0104).
use base64::{engine::general_purpose::STANDARD, Engine};
use portable_pty::CommandBuilder;
use prometeu_core::lock::lock;
use prometeu_core::process::ProcessHandle;
use prometeu_core::terminal::{TerminalControl, TerminalFactory, TerminalSize};
use prometeu_host::{socket_name, valid_id, workspace_slice, LINE_LIMIT, PROTOCOL};
use prometeu_process::terminal::UnixTerminalFactory;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Output retained per terminal for reattachment. A gap beyond it replays the whole buffer.
const RETAINED: usize = 2 * 1_048_576;
/// Hooks retained per terminal for desktops that are away; the oldest are dropped first.
const QUEUED_HOOKS: usize = 512;
/// Bytes waiting to be written to one attachment before the host drops it. A desktop that stopped
/// reading (asleep behind a half-open connection) reattaches and replays what it missed instead
/// of growing the host's memory.
const OUTBOX_LIMIT: usize = 16 * 1_048_576;

/// `OUTBOX_LIMIT`, or `PROMETEU_HOST_OUTBOX_LIMIT` bytes, for tests on slow PTYs.
fn outbox_limit() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("PROMETEU_HOST_OUTBOX_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(OUTBOX_LIMIT)
    })
}
/// Desktops attached at once.
const CLIENT_LIMIT: usize = 8;
const LABEL_LIMIT: usize = 64;
const DECISION_WAIT: Duration = Duration::from_secs(300);
/// How long `exit` waits for the PTY to drain after the process ends.
const DRAIN_WAIT: Duration = Duration::from_secs(2);
const EXEC_OUTPUT: usize = 4 * 1_048_576;
const WATCH_CHUNK: usize = 1_048_576;
const WATCH_FILE_LIMIT: u64 = 256 * 1_048_576;

struct Chunk {
    seq: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct Ring {
    chunks: VecDeque<Chunk>,
    size: usize,
    seq: u64,
    code: Option<u32>,
    running: bool,
}
impl Ring {
    fn push(&mut self, bytes: &[u8]) -> u64 {
        self.seq += 1;
        self.size += bytes.len();
        self.chunks.push_back(Chunk {
            seq: self.seq,
            bytes: bytes.to_vec(),
        });
        while self.size > RETAINED && self.chunks.len() > 1 {
            if let Some(old) = self.chunks.pop_front() {
                self.size -= old.bytes.len();
            }
        }
        self.seq
    }
    /// Bytes after `seq`, or everything retained with `reset` when part of the gap was dropped.
    fn since(&self, seq: u64) -> (Vec<u8>, bool) {
        let oldest = self.chunks.front().map(|c| c.seq).unwrap_or(self.seq + 1);
        let reset = seq + 1 < oldest || seq > self.seq;
        let mut bytes = Vec::new();
        for chunk in &self.chunks {
            if reset || chunk.seq > seq {
                bytes.extend_from_slice(&chunk.bytes);
            }
        }
        (bytes, reset)
    }
}

pub(crate) struct Term {
    pub(crate) id: String,
    instance: String,
    workspace: Option<String>,
    pid: u32,
    started: u64,
    unit: Option<String>,
    pub(crate) input: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn TerminalControl>,
    pub(crate) process: ProcessHandle,
    ring: Mutex<Ring>,
    hooks: Mutex<HookLog>,
    size: Mutex<Sizing>,
    /// A desktop collected the exited terminal while another still follows it: it is dropped once
    /// no attachment follows it.
    forgotten: AtomicBool,
}

struct HookEntry {
    number: u64,
    frame: Value,
    /// A desktop confirmed it, so a device this terminal never served does not receive it.
    delivered: bool,
}

/// Non-permission hooks of one terminal and, per device, which of them it confirmed through its
/// `hook_reply`. A device is owed every retained hook it has not confirmed: a frame queued to an
/// attachment that was already dead (a desktop asleep behind a half-open connection) is sent again
/// when that desktop comes back, whether another desktop received it meanwhile or not. Desktops
/// handle hooks on separate threads, so confirmations arrive in any order and each counts only for
/// its own hook.
#[derive(Default)]
struct HookLog {
    entries: VecDeque<HookEntry>,
    /// Confirmed hook numbers per device, among the retained entries.
    confirmed: HashMap<String, HashSet<u64>>,
}

impl HookLog {
    fn push(&mut self, number: u64, frame: Value) {
        if self.entries.len() >= QUEUED_HOOKS {
            if let Some(dropped) = self.entries.pop_front() {
                for numbers in self.confirmed.values_mut() {
                    numbers.remove(&dropped.number);
                }
            }
        }
        self.entries.push_back(HookEntry {
            number,
            frame,
            delivered: false,
        });
    }

    /// The hooks a device is owed. The first time this terminal serves a device, the hooks another
    /// desktop already confirmed count as its own (the single-desktop queue's behavior); every
    /// later hook is owed to it until it confirms that hook, even when the log is empty now.
    fn missed(&mut self, device: &str) -> Vec<Value> {
        let entries = &self.entries;
        let confirmed = self.confirmed.entry(device.into()).or_insert_with(|| {
            entries
                .iter()
                .filter(|e| e.delivered)
                .map(|e| e.number)
                .collect()
        });
        entries
            .iter()
            .filter(|e| !confirmed.contains(&e.number))
            .map(|e| e.frame.clone())
            .collect()
    }

    /// A device's desktop handled this hook. Returns false when the terminal holds no such hook.
    fn confirm(&mut self, device: &str, number: u64) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|e| e.number == number) else {
            return false;
        };
        entry.delivered = true;
        self.confirmed
            .entry(device.into())
            .or_default()
            .insert(number);
        true
    }
}

/// The PTY has one size. The desktop that last typed into a terminal drives it; the others'
/// requests are remembered and applied when they type, so two windows never fight over it.
#[derive(Default)]
struct Sizing {
    driver: Option<u64>,
    applied: Option<TerminalSize>,
}

/// One attachment's outgoing frames, written by its own thread.
#[derive(Clone)]
struct Outbox {
    sender: mpsc::Sender<String>,
    queued: Arc<AtomicUsize>,
    alive: Arc<AtomicBool>,
    stream: Arc<UnixStream>,
}

impl Outbox {
    fn send(&self, frame: &Value) -> bool {
        if !self.alive.load(Ordering::Acquire) {
            return false;
        }
        let line = frame.to_string();
        let size = line.len();
        if self.queued.fetch_add(size, Ordering::AcqRel) + size > outbox_limit() {
            self.close();
            return false;
        }
        self.sender.send(line).is_ok()
    }

    /// Hold a bulk sender (a transcript catching up) until the writer drained half the limit, so
    /// only a desktop that stopped reading reaches it.
    fn wait_for_room(&self) {
        while self.alive.load(Ordering::Acquire)
            && self.queued.load(Ordering::Acquire) > outbox_limit() / 2
        {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Stop serving the attachment at once; its reader sees the end of the stream.
    fn close(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// Tell the attachment it was replaced, then stop serving it after that frame. Its reader
    /// stops at once, even when the replaced connection is half-open and would never send again.
    fn replace(&self) {
        self.send(&json!({"event": "replaced"}));
        self.alive.store(false, Ordering::Release);
        let _ = self.stream.shutdown(std::net::Shutdown::Read);
    }
}

struct Client {
    generation: u64,
    /// Identifies the desktop across its attachments; empty for desktops that predate it.
    device: String,
    label: String,
    connected: u64,
    out: Outbox,
    subscribed: HashSet<String>,
    /// The size this desktop last asked for, per terminal.
    sizes: HashMap<String, TerminalSize>,
}

/// A permission request waiting for the desktops it was delivered to. The first decision wins;
/// a desktop that answers with nothing, or goes away, abstains. When all abstain, Claude's own
/// dialog answers. The hook's own thread removes the entry and tells the others, since a reply can
/// arrive before it knows who received the request.
struct Decision {
    sender: mpsc::Sender<Vec<u8>>,
    waiting: Option<HashSet<u64>>,
    abstained: HashSet<u64>,
    decided: Option<u64>,
}

impl Decision {
    fn settled(&self) -> bool {
        self.decided.is_some()
            || self
                .waiting
                .as_ref()
                .is_some_and(|w| w.iter().all(|g| self.abstained.contains(g)))
    }

    /// Wake the hook's thread once nobody is left to answer.
    fn abstain(&mut self, generation: u64) {
        if self.decided.is_none() && self.abstained.insert(generation) && self.settled() {
            let _ = self.sender.send(Vec::new());
        }
    }
}

/// What a conversation's hooks last said, for clients that cannot read its screen.
#[derive(Clone, Default)]
pub(crate) struct Activity {
    pub(crate) state: &'static str,
    pub(crate) at: u64,
    pub(crate) notification: Option<String>,
    background: HashSet<String>,
    main_done: bool,
}

pub(crate) struct Host {
    pub(crate) root: PathBuf,
    ready: Value,
    scopes: bool,
    terminals: Mutex<HashMap<String, Arc<Term>>>,
    /// Terminal IDs whose process is being started; another open of the same ID is refused.
    opening: Mutex<HashSet<String>>,
    clients: Mutex<Vec<Client>>,
    generations: AtomicU64,
    hooks: AtomicU64,
    decisions: Mutex<HashMap<u64, Decision>>,
    units: AtomicU64,
    pub(crate) registry: Arc<crate::registry::Store>,
    pub(crate) supervision: Arc<crate::supervision::Supervision>,
    pub(crate) todos: crate::todos::Store,
    activity: Mutex<HashMap<String, Activity>>,
    pub(crate) codex_watch: Mutex<HashMap<String, u32>>,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Host {
    pub(crate) fn set_supervisor(&self, id: &str, supervisor: Option<&str>) -> Result<(), String> {
        // The same lock orders hook observations and their activity snapshots, so attaching
        // midway through work cannot race a final hook into an unfinished synthetic execution.
        let activity = lock(&self.activity);
        let registry = self.registry.read()?;
        let workspace = registry
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or("unknown_workspace")?;
        let active: Vec<(String, Vec<String>, bool)> = workspace
            .conversations
            .iter()
            .filter_map(|c| {
                let state = activity.get(&c.id)?;
                (matches!(state.state, "working" | "permission") || !state.background.is_empty())
                    .then(|| {
                        (
                            c.id.clone(),
                            state.background.iter().cloned().collect(),
                            state.main_done,
                        )
                    })
            })
            .collect();
        self.supervision.set_binding(id, supervisor, &active)
    }

    /// Tell the attached desktops that an agent changed the registry, so they adopt new
    /// workspaces and conversations without waiting for their next attachment (ADR 0103).
    pub(crate) fn announce_registry(&self) {
        for client in lock(&self.clients).iter() {
            client.out.send(&json!({"event": "registry"}));
        }
    }

    /// Tell the attached desktops that the machine's to-do list changed.
    pub(crate) fn announce_todos(&self) {
        for client in lock(&self.clients).iter() {
            client.out.send(&json!({"event": "todos"}));
        }
    }

    /// Deliver a terminal frame to every attachment that follows the terminal; returns the
    /// generations reached.
    fn emit_terminal(&self, id: &str, frame: &Value) -> HashSet<u64> {
        lock(&self.clients)
            .iter()
            .filter(|c| c.subscribed.contains(id) && c.out.send(frame))
            .map(|c| c.generation)
            .collect()
    }

    /// Send a frame to the listed attachments that are still here.
    fn emit_to(&self, generations: &HashSet<u64>, frame: &Value) {
        for client in lock(&self.clients).iter() {
            if generations.contains(&client.generation) {
                client.out.send(frame);
            }
        }
    }

    /// Record a desktop's answer to a permission request; only the first decision counts. Returns
    /// false when no permission request has this number.
    fn decide(&self, hook: u64, generation: u64, stdout: Vec<u8>) -> bool {
        let mut decisions = lock(&self.decisions);
        let Some(decision) = decisions.get_mut(&hook) else {
            return false;
        };
        if stdout.is_empty() {
            decision.abstain(generation);
        } else if decision.decided.is_none() {
            decision.decided = Some(generation);
            let _ = decision.sender.send(stdout);
        }
        true
    }

    /// An attachment ended: it abstains from pending decisions, and exited terminals that only it
    /// still followed are dropped.
    fn detached(&self, generation: u64) {
        {
            for decision in lock(&self.decisions).values_mut() {
                decision.abstain(generation);
            }
        }
        let followed: HashSet<String> = lock(&self.clients)
            .iter()
            .flat_map(|c| c.subscribed.iter().cloned())
            .collect();
        let mut terminals = lock(&self.terminals);
        let dropped: Vec<String> = terminals
            .values()
            .filter(|t| {
                t.forgotten.load(Ordering::Acquire)
                    && !followed.contains(&t.id)
                    && !lock(&t.ring).running
            })
            .map(|t| t.id.clone())
            .collect();
        for id in dropped {
            terminals.remove(&id);
            let _ = std::fs::remove_dir_all(self.root.join("sessions").join(&id));
        }
    }

    /// Apply a desktop's size when it drives the terminal, or when nobody attached does.
    fn resize(&self, term: &Term, generation: u64, size: TerminalSize) -> Result<(), String> {
        let mut sizing = lock(&term.size);
        let driver = sizing.driver.filter(|d| {
            lock(&self.clients)
                .iter()
                .any(|c| c.generation == *d && c.subscribed.contains(&term.id))
        });
        {
            let mut clients = lock(&self.clients);
            if let Some(client) = clients.iter_mut().find(|c| c.generation == generation) {
                client.sizes.insert(term.id.clone(), size);
            }
        }
        if driver.is_some_and(|d| d != generation) || sizing.applied == Some(size) {
            return Ok(());
        }
        term.control.resize(size)?;
        sizing.applied = Some(size);
        Ok(())
    }

    /// The desktop that types into a terminal drives its size from now on.
    fn drive(&self, term: &Term, generation: u64) {
        let mut sizing = lock(&term.size);
        if sizing.driver == Some(generation) {
            return;
        }
        sizing.driver = Some(generation);
        let wanted = lock(&self.clients)
            .iter()
            .find(|c| c.generation == generation)
            .and_then(|c| c.sizes.get(&term.id).copied());
        if let Some(size) = wanted.filter(|s| sizing.applied != Some(*s)) {
            if term.control.resize(size).is_ok() {
                sizing.applied = Some(size);
            }
        }
    }

    /// Whether a terminal with this ID is running now.
    pub(crate) fn is_running(&self, id: &str) -> bool {
        lock(&self.terminals)
            .get(id)
            .is_some_and(|t| lock(&t.ring).running)
    }

    /// A conversation's observed state: its hooks' account while its terminal runs, `ended`
    /// otherwise.
    pub(crate) fn activity(&self, id: &str) -> Activity {
        let mut activity = lock(&self.activity).get(id).cloned().unwrap_or_default();
        if !self.is_running(id) {
            activity.state = "ended";
        } else if activity.state.is_empty() {
            activity.state = "starting";
        }
        activity
    }

    pub(crate) fn terminal_pid(&self, id: &str) -> Option<u32> {
        lock(&self.terminals)
            .get(id)
            .filter(|t| lock(&t.ring).running)
            .map(|t| t.pid)
    }

    /// A late observer cannot change the state or stop a replacement engine.
    pub(crate) fn codex_activity(
        &self,
        id: &str,
        pid: u32,
        state: &'static str,
        notification: Option<String>,
    ) {
        let watches = lock(&self.codex_watch);
        if watches.get(id) != Some(&pid) {
            return;
        }
        let mut activity = lock(&self.activity);
        let entry = activity.entry(id.into()).or_default();
        entry.state = if state == "idle" && !entry.background.is_empty() {
            "working"
        } else {
            state
        };
        entry.at = now();
        entry.notification = notification;
    }

    /// Keep native lifecycle facts and their activity snapshot in the same order as hooks.
    pub(crate) fn codex_observe(
        &self,
        id: &str,
        pid: u32,
        observation: prometeu_core::supervision::Observation,
    ) -> Result<(), String> {
        use prometeu_core::supervision::Observation;
        let watches = lock(&self.codex_watch);
        if watches.get(id) != Some(&pid) {
            return Ok(());
        }
        let mut activity = lock(&self.activity);
        self.supervision.observe(id, observation.clone())?;
        let entry = activity.entry(id.into()).or_default();
        entry.at = now();
        entry.notification = None;
        match observation {
            Observation::Started => {
                entry.main_done = false;
                entry.state = "working";
            }
            Observation::Working => {
                entry.state = if entry.main_done && entry.background.is_empty() {
                    "idle"
                } else {
                    "working"
                };
            }
            Observation::Blocked => entry.state = "permission",
            Observation::Completed => {
                entry.main_done = true;
                entry.state = if entry.background.is_empty() {
                    "idle"
                } else {
                    "working"
                };
            }
            Observation::Failed | Observation::Interrupted => {
                entry.main_done = false;
                entry.background.clear();
                entry.state = "idle";
            }
            Observation::BackgroundStarted(child) => {
                entry.background.insert(child);
                entry.state = "working";
            }
            Observation::BackgroundFinished(child) => {
                entry.background.remove(&child);
                if entry.main_done && entry.background.is_empty() {
                    entry.state = "idle";
                }
            }
            Observation::Exited { .. } => {
                entry.background.clear();
                entry.main_done = false;
                entry.state = "ended";
            }
        }
        Ok(())
    }

    pub(crate) fn end_pid(&self, id: &str, pid: u32) {
        let term = lock(&self.terminals).get(id).cloned();
        if let Some(term) = term.filter(|t| t.pid == pid) {
            term.process.control.terminate();
            let deadline = Instant::now() + Duration::from_secs(5);
            while lock(&term.ring).running && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if lock(&term.ring).running {
                term.process.control.kill();
            }
        }
    }

    /// Write raw input to a running terminal.
    pub(crate) fn write(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let term = find(self, id)?;
        let mut input = lock(&term.input);
        input.write_all(data).map_err(|e| e.to_string())?;
        input.flush().map_err(|e| e.to_string())
    }

    /// Type a paste and the Enter that submits it while holding the terminal's input, so another
    /// client's message cannot land between them.
    pub(crate) fn submit(&self, id: &str, paste: &[u8]) -> Result<(), String> {
        let term = find(self, id)?;
        let mut input = lock(&term.input);
        input.write_all(paste).map_err(|e| e.to_string())?;
        input.flush().map_err(|e| e.to_string())?;
        // Claude handles a paste before the key that submits it.
        std::thread::sleep(Duration::from_millis(150));
        input.write_all(b"\r").map_err(|e| e.to_string())?;
        input.flush().map_err(|e| e.to_string())
    }

    /// Terminate a terminal and wait briefly for it to exit, then kill it.
    pub(crate) fn end(&self, id: &str) {
        let Ok(term) = find(self, id) else { return };
        term.process.control.terminate();
        let deadline = Instant::now() + Duration::from_secs(5);
        while lock(&term.ring).running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if lock(&term.ring).running {
            term.process.control.kill();
        }
    }

    /// Drop an exited terminal and its hook settings.
    pub(crate) fn forget(&self, id: &str) {
        let mut terminals = lock(&self.terminals);
        if terminals.get(id).is_some_and(|t| lock(&t.ring).running) {
            return;
        }
        terminals.remove(id);
        lock(&self.activity).remove(id);
        let _ = std::fs::remove_dir_all(self.root.join("sessions").join(id));
    }

    fn running(&self) -> usize {
        lock(&self.terminals)
            .values()
            .filter(|t| lock(&t.ring).running)
            .count()
    }
}

/// Adopt the login shell's PATH: the host may start from a non-interactive SSH command or a
/// systemd unit, where `~/.local/bin` and similar entries are missing and agent CLIs live.
fn adopt_login_path() {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let output = Command::new(shell)
            .args(["-ilc", r#"printf %s "$PATH""#])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = sender.send(output);
    });
    if let Ok(Ok(output)) = receiver.recv_timeout(Duration::from_secs(5)) {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if output.status.success() && !path.is_empty() {
            std::env::set_var("PATH", path);
        }
    }
}

fn command_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn linger() -> bool {
    let user = std::env::var("USER").unwrap_or_default();
    Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger", "--value"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes")
}

fn describe(root: &Path, scopes: bool, linger: bool, release: Option<&str>) -> Value {
    let uname = |flag: &str| {
        Command::new("uname")
            .arg(flag)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    json!({
        "event": "ready",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        // The desktop app release that started this host, when it said.
        "release": release,
        "executable": std::env::current_exe().ok().map(|p| p.display().to_string()),
        "pid": std::process::id(),
        "home": std::env::var("HOME").unwrap_or_default(),
        "user": std::env::var("USER").unwrap_or_default(),
        "root": root.display().to_string(),
        "os": uname("-s"),
        "arch": uname("-m"),
        "linger": linger,
        "scopes": scopes,
        // Several desktops may stay attached at once (ADR 0104).
        "clients": true,
        "codex_agents": true,
    })
}

pub fn serve(root: &Path, release: Option<&str>) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)
        .map_err(|e| e.to_string())?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(root.join(format!("host-v{PROTOCOL}.lock")))
        .map_err(|e| e.to_string())?;
    // SAFETY: flock on an owned descriptor; the file stays open for the process lifetime.
    if unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        // Another host already serves this root.
        return Ok(());
    }
    adopt_login_path();
    // A new host owns no terminal yet, so no hook settings are live. Dropping them lets updates
    // remove the builds they named.
    // ponytail: assumes one protocol per root; a future protocol bump running two hosts side by
    // side would need per-protocol session folders.
    let _ = std::fs::remove_dir_all(root.join("sessions"));
    let path = root.join(socket_name());
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let linger = linger();
    let scopes = linger && command_ok("systemd-run", &["--version"]);
    let registry = Arc::new(crate::registry::Store::new(root));
    let supervision = crate::supervision::Supervision::new(root, registry.clone())?;
    let host = Arc::new(Host {
        root: root.into(),
        ready: describe(root, scopes, linger, release),
        scopes,
        terminals: Mutex::new(HashMap::new()),
        opening: Mutex::new(HashSet::new()),
        clients: Mutex::new(Vec::new()),
        generations: AtomicU64::new(0),
        hooks: AtomicU64::new(0),
        decisions: Mutex::new(HashMap::new()),
        units: AtomicU64::new(0),
        registry,
        supervision,
        todos: crate::todos::store(root),
        activity: Mutex::new(HashMap::new()),
        codex_watch: Mutex::new(HashMap::new()),
    });
    host.supervision.start();
    crate::agents::listen(host.clone());
    eprintln!(
        "prometeu-host {} serving {} (scopes: {scopes})",
        env!("CARGO_PKG_VERSION"),
        path.display()
    );
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let host = host.clone();
        std::thread::spawn(move || {
            let _ = connection(&host, stream);
        });
    }
    drop(lock_file);
    Ok(())
}

pub(crate) fn read_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut line = Vec::new();
    let read = reader
        .take(LINE_LIMIT as u64 + 1)
        .read_until(b'\n', &mut line)
        .map_err(|e| e.to_string())?;
    if read == 0 {
        return Ok(None);
    }
    if line.len() > LINE_LIMIT {
        return Err("line too long".into());
    }
    String::from_utf8(line)
        .map(|s| Some(s.trim_end().to_string()))
        .map_err(|e| e.to_string())
}

fn connection(host: &Arc<Host>, stream: UnixStream) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let Some(first) = read_line(&mut reader)? else {
        return Ok(());
    };
    let first: Value = serde_json::from_str(&first).map_err(|e| e.to_string())?;
    if let Some(request) = first.get("supervision") {
        let mut stream = stream;
        let response = match host.supervision.configure(request) {
            Ok(result) => json!({"result":result}),
            Err(error) => json!({"error":error}),
        };
        return writeln!(stream, "{response}").map_err(|e| e.to_string());
    }
    if let Some(hook) = first.get("hook") {
        return serve_hook(host, stream, hook);
    }
    if first.pointer("/attach/protocol").and_then(Value::as_u64) != Some(PROTOCOL as u64) {
        let mut stream = stream;
        let _ = writeln!(stream, "{}", json!({"event": "error", "error": "protocol"}));
        return Ok(());
    }
    serve_client(host, stream, reader, &first["attach"])
}

/// Who an attachment says it is. The device identifies a desktop across its attachments; any
/// process that reached this socket already runs as the host's user, so it is a label for
/// replacement and display, not a credential.
fn identity(attach: &Value) -> (String, String) {
    let device = attach
        .get("device")
        .and_then(Value::as_str)
        .filter(|d| valid_id(d))
        .unwrap_or_default()
        .to_string();
    let label: String = attach
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(LABEL_LIMIT)
        .collect();
    (device, label)
}

/// Attachments from different devices are served side by side. A new attachment replaces only
/// one from the same device: after sleep that desktop's old SSH connection can look alive for
/// minutes, and its new one must win. Desktops that send no device replace each other, as before.
fn serve_client(
    host: &Arc<Host>,
    stream: UnixStream,
    mut reader: BufReader<UnixStream>,
    attach: &Value,
) -> Result<(), String> {
    let (device, label) = identity(attach);
    let (sender, lines) = mpsc::channel::<String>();
    let out = Outbox {
        sender,
        queued: Arc::new(AtomicUsize::new(0)),
        alive: Arc::new(AtomicBool::new(true)),
        stream: Arc::new(stream.try_clone().map_err(|e| e.to_string())?),
    };
    let generation = host.generations.fetch_add(1, Ordering::Relaxed) + 1;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    // The writer holds no sender: it ends once every sender of this attachment is gone.
    let (queued, alive, shutdown) = (out.queued.clone(), out.alive.clone(), out.stream.clone());
    std::thread::spawn(move || {
        for mut line in lines {
            queued.fetch_sub(line.len(), Ordering::AcqRel);
            line.push('\n');
            if writer.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
        alive.store(false, Ordering::Release);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
    });
    {
        let mut clients = lock(&host.clients);
        let (same, others): (Vec<Client>, Vec<Client>) = std::mem::take(&mut *clients)
            .into_iter()
            .partition(|c| c.device == device);
        *clients = others;
        for previous in same {
            previous.out.replace();
        }
        if clients.len() >= CLIENT_LIMIT {
            drop(clients);
            out.send(&json!({"event": "error", "error": "busy"}));
            out.alive.store(false, Ordering::Release);
            return Ok(());
        }
        out.send(&host.ready);
        clients.push(Client {
            generation,
            device: device.clone(),
            label,
            connected: now(),
            out: out.clone(),
            subscribed: HashSet::new(),
            sizes: HashMap::new(),
        });
    }
    let conn = Conn {
        out: out.clone(),
        generation,
        device,
        watches: Arc::default(),
    };
    while out.alive.load(Ordering::Acquire) {
        let line = match read_line(&mut reader) {
            Ok(Some(line)) => line,
            _ => break,
        };
        if line.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                out.send(&json!({"id": null, "error": error.to_string()}));
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let host = host.clone();
        let conn = conn.clone();
        // Requests run in order, so an attach never overtakes the open before it. Only work
        // that can take long (commands, cgroup reads, shutdown) runs beside the stream.
        let inline = !matches!(method.as_str(), "exec" | "stats" | "shutdown");
        let run = move || {
            match dispatch(&host, &conn, &id, &method, &params) {
                Ok(Some(result)) => conn.reply(&id, Ok(result)),
                Ok(None) => {}
                Err(error) => conn.reply(&id, Err(error)),
            };
        };
        if inline {
            run();
        } else {
            std::thread::spawn(run);
        }
    }
    out.alive.store(false, Ordering::Release);
    lock(&host.clients).retain(|c| c.generation != generation);
    host.detached(generation);
    Ok(())
}

#[derive(Clone)]
struct Conn {
    out: Outbox,
    generation: u64,
    device: String,
    /// Each watch of this attachment, by ID: a request to scan now, answered once its file
    /// frames are on the stream.
    watches: Arc<Mutex<HashMap<String, mpsc::Sender<mpsc::Sender<()>>>>>,
}
impl Conn {
    fn reply(&self, id: &Value, result: Result<Value, String>) {
        let frame = match result {
            Ok(result) => json!({"id": id, "result": result}),
            Err(error) => json!({"id": id, "error": error}),
        };
        self.out.send(&frame);
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

fn terminal_id(params: &Value) -> Result<String, String> {
    let id = text(params, "id")?;
    valid_id(id)
        .then(|| id.to_string())
        .ok_or_else(|| "invalid id".to_string())
}

fn bytes(params: &Value, key: &str) -> Result<Vec<u8>, String> {
    STANDARD
        .decode(text(params, key)?)
        .map_err(|e| e.to_string())
}

pub(crate) fn find(host: &Host, id: &str) -> Result<Arc<Term>, String> {
    lock(&host.terminals)
        .get(id)
        .cloned()
        .ok_or_else(|| "unknown_terminal".to_string())
}

fn dispatch(
    host: &Arc<Host>,
    conn: &Conn,
    id: &Value,
    method: &str,
    params: &Value,
) -> Result<Option<Value>, String> {
    let result = match method {
        "ping" => json!({}),
        // Requests that change the machine hold off a non-forced shutdown, and fail once one has
        // started, so an update never cuts them off halfway (ADR 0103).
        "exec" => {
            let _change = crate::agents::Mutation::begin()?;
            exec(params)?
        }
        "file_write" => {
            let _change = crate::agents::Mutation::begin()?;
            file_write(params)?
        }
        "file_read" => file_read(params)?,
        "terminal_open" => {
            let _change = crate::agents::Mutation::begin()?;
            let opened = open(host, params)?;
            crate::codex_agent::watch_registered(host);
            // The desktop that opened a terminal drives its size until another one types.
            let term = find(host, &terminal_id(params)?)?;
            *lock(&term.size) = Sizing {
                driver: Some(conn.generation),
                applied: Some(size(params)?),
            };
            if let Some(client) = lock(&host.clients)
                .iter_mut()
                .find(|c| c.generation == conn.generation)
            {
                client.sizes.insert(term.id.clone(), size(params)?);
            }
            opened
        }
        "terminal_write" => {
            let term = find(host, &terminal_id(params)?)?;
            let data = bytes(params, "data")?;
            host.drive(&term, conn.generation);
            let mut input = lock(&term.input);
            input.write_all(&data).map_err(|e| e.to_string())?;
            input.flush().map_err(|e| e.to_string())?;
            json!({})
        }
        "terminal_resize" => {
            let term = find(host, &terminal_id(params)?)?;
            host.resize(&term, conn.generation, size(params)?)?;
            json!({})
        }
        "terminal_stop" => {
            let term = find(host, &terminal_id(params)?)?;
            if text(params, "instance")? != term.instance || !term.process.control.running() {
                return Err("terminal_changed".into());
            }
            // Keep the same process handle through validation and signaling. Replacing the ID
            // cannot retarget this handle, and its control never signals a reaped process.
            term.process.control.kill();
            json!({})
        }
        "terminal_signal" => {
            let term = find(host, &terminal_id(params)?)?;
            match text(params, "signal")? {
                "interrupt" => term.process.control.interrupt(),
                "terminate" => term.process.control.terminate(),
                "kill" => term.process.control.kill(),
                "hangup" => term.control.close(),
                other => return Err(format!("unknown signal {other}")),
            }
            json!({})
        }
        "terminal_forget" => {
            let id = terminal_id(params)?;
            if host.is_running(&id) {
                return Err("terminal_running".into());
            }
            // Another desktop still following the terminal has yet to collect its output and
            // hooks: keep it until that one forgets it or goes away.
            let followed = {
                let mut clients = lock(&host.clients);
                for client in clients.iter_mut() {
                    if client.generation == conn.generation {
                        client.subscribed.remove(&id);
                    }
                }
                clients.iter().any(|c| c.subscribed.contains(&id))
            };
            let mut terminals = lock(&host.terminals);
            if followed {
                if let Some(term) = terminals.get(&id) {
                    term.forgotten.store(true, Ordering::Release);
                }
                return Ok(Some(json!({})));
            }
            if terminals.get(&id).is_some_and(|t| lock(&t.ring).running) {
                return Err("terminal_running".into());
            }
            terminals.remove(&id);
            // The conversation's hook settings name a build; dropping them lets updates remove it.
            let _ = std::fs::remove_dir_all(host.root.join("sessions").join(&id));
            json!({})
        }
        "terminal_list" => {
            let terminals = lock(&host.terminals);
            Value::Array(
                terminals
                    .values()
                    .map(|t| {
                        let ring = lock(&t.ring);
                        json!({
                            "id": t.id, "instance": t.instance, "workspace": t.workspace, "pid": t.pid, "unit": t.unit,
                            "started": t.started, "running": ring.running, "code": ring.code,
                            "seq": ring.seq,
                        })
                    })
                    .collect(),
            )
        }
        "terminal_attach" => {
            attach(host, conn, id, params)?;
            return Ok(None);
        }
        "watch" => {
            watch(conn, params)?;
            json!({})
        }
        "watch_flush" => {
            let id = terminal_id(params)?;
            let flush = lock(&conn.watches)
                .get(&id)
                .cloned()
                .ok_or("unknown_watch")?;
            let (done, flushed) = mpsc::channel();
            flush.send(done).map_err(|_| "unknown_watch")?;
            flushed
                .recv_timeout(Duration::from_secs(10))
                .map_err(|_| "flush_timeout")?;
            json!({})
        }
        "hook_reply" => {
            let hook = params
                .get("hook")
                .and_then(Value::as_u64)
                .ok_or("missing hook")?;
            let stdout = bytes(params, "stdout").unwrap_or_default();
            // Desktops reply to every hook: for a permission request with its decision, for the
            // others as confirmation that they handled it.
            if !host.decide(hook, conn.generation, stdout) {
                let terminals: Vec<Arc<Term>> = lock(&host.terminals).values().cloned().collect();
                for term in terminals {
                    if lock(&term.hooks).confirm(&conn.device, hook) {
                        break;
                    }
                }
            }
            json!({})
        }
        "clients_list" => Value::Array(
            lock(&host.clients)
                .iter()
                .map(|c| {
                    json!({
                        "device": c.device, "label": c.label, "connected": c.connected,
                        "current": c.generation == conn.generation,
                    })
                })
                .collect(),
        ),
        "client_detach" => {
            let device = text(params, "device")?;
            let mut clients = lock(&host.clients);
            let index = clients
                .iter()
                .position(|c| c.device == device && c.generation != conn.generation)
                .ok_or("unknown_client")?;
            // Like a replacement: the desktop stops reconnecting until its person asks.
            clients.remove(index).out.replace();
            json!({})
        }
        "stats" => stats(host, params),
        "registry_list" => crate::agents::registry_view(host, params["codex_agents"] == true)?,
        "registry_put" => crate::agents::registry_put(host, params)?,
        "registry_close" => crate::agents::registry_close(host, params)?,
        "todos_list" | "todos_create" | "todos_set_status" | "todos_enable" => {
            crate::todos::call(host, method, params, true)?
        }
        "shutdown" => {
            let force = params.get("force").and_then(Value::as_bool) == Some(true);
            if !force && host.running() > 0 {
                return Err("host_busy".into());
            }
            crate::agents::begin_shutdown(force)?;
            conn.reply(id, Ok(json!({})));
            let _ = std::fs::remove_file(host.root.join(socket_name()));
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        }
        other => return Err(format!("unknown method {other}")),
    };
    Ok(Some(result))
}

fn size(params: &Value) -> Result<TerminalSize, String> {
    let dimension = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_u64)
            .filter(|v| (1..=1000).contains(v))
            .map(|v| v as u16)
            .ok_or_else(|| format!("invalid {key}"))
    };
    Ok(TerminalSize {
        cols: dimension("cols")?,
        rows: dimension("rows")?,
    })
}

fn string_list(params: &Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Apply workspace limits to its slice. The slice exists only once a scope references it, so
/// this runs after the terminal starts and retries briefly.
fn limit_slice(slice: String, limits: Map<String, Value>) {
    let mut properties = Vec::new();
    for (key, property) in [
        ("memory_max", "MemoryMax"),
        ("memory_high", "MemoryHigh"),
        ("cpu_quota", "CPUQuota"),
        ("tasks_max", "TasksMax"),
    ] {
        let value = match limits.get(key) {
            Some(Value::String(v)) if !v.trim().is_empty() => v.trim().to_string(),
            Some(Value::Number(n)) => n.to_string(),
            _ => continue,
        };
        if value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '%' || c == '.')
        {
            properties.push(format!("{property}={value}"));
        }
    }
    if properties.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..20 {
            let mut command = Command::new("systemctl");
            command
                .args(["--user", "set-property", "--runtime", &slice])
                .args(&properties)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if command.status().is_ok_and(|s| s.success()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
}

/// A terminal ID held while its process starts, released however the open ends.
pub(crate) struct Reservation<'a> {
    host: &'a Host,
    id: String,
}

impl<'a> Reservation<'a> {
    pub(crate) fn take(host: &'a Host, id: &str) -> Result<Self, String> {
        let terminals = lock(&host.terminals);
        let mut opening = lock(&host.opening);
        if opening.contains(id) || terminals.get(id).is_some_and(|t| lock(&t.ring).running) {
            return Err("terminal_exists".into());
        }
        opening.insert(id.into());
        Ok(Self {
            host,
            id: id.into(),
        })
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        lock(&self.host.opening).remove(&self.id);
    }
}

pub(crate) fn open(host: &Arc<Host>, params: &Value) -> Result<Value, String> {
    let id = terminal_id(params)?;
    // Desktops and agents share this reservation, so only its owner can start the process.
    let reserved = Reservation::take(host, &id)?;
    open_reserved(host, params, &reserved)
}

pub(crate) fn open_reserved(
    host: &Arc<Host>,
    params: &Value,
    reserved: &Reservation<'_>,
) -> Result<Value, String> {
    let id = reserved.id.clone();
    let argv = string_list(params, "argv");
    if argv.is_empty() {
        return Err("missing argv".into());
    }
    let cwd = text(params, "cwd")?;
    if !Path::new(cwd).is_dir() {
        return Err(format!("missing directory {cwd}"));
    }
    let workspace = params
        .get("workspace")
        .and_then(Value::as_str)
        .filter(|w| valid_id(w))
        .map(str::to_owned);
    let slice = workspace.as_deref().and_then(workspace_slice);
    let (program, unit) = match (&slice, host.scopes) {
        (Some(slice), true) => {
            let unit = format!(
                "prometeu-t{}-{}",
                id.replace(['-', '_'], ""),
                host.units.fetch_add(1, Ordering::Relaxed)
            );
            let mut wrapped = vec![
                "systemd-run".to_string(),
                "--user".into(),
                "--scope".into(),
                "--quiet".into(),
                "--collect".into(),
                format!("--slice={slice}"),
                format!("--unit={unit}"),
                "--".into(),
            ];
            wrapped.extend(argv);
            (wrapped, Some(unit))
        }
        _ => (argv, None),
    };
    let mut command = CommandBuilder::new(&program[0]);
    command.args(&program[1..]);
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    for key in string_list(params, "unset") {
        command.env_remove(key);
    }
    if let Some(env) = params.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }
    let instance = crate::agents::uuid()?;
    let started = UnixTerminalFactory
        .open(command, size(params)?)
        .map_err(|e| format!("{e:?}"))?;
    if let (Some(slice), Some(limits)) = (
        slice.filter(|_| unit.is_some()),
        params.get("limits").and_then(Value::as_object),
    ) {
        limit_slice(slice, limits.clone());
    }
    let pid = started.process.control.system_id();
    let term = Arc::new(Term {
        id: id.clone(),
        instance,
        workspace,
        pid,
        started: now(),
        unit,
        input: Mutex::new(started.input),
        control: started.control,
        process: started.process,
        ring: Mutex::new(Ring {
            running: true,
            ..Ring::default()
        }),
        hooks: Mutex::new(HookLog::default()),
        size: Mutex::new(Sizing {
            driver: None,
            applied: size(params).ok(),
        }),
        forgotten: AtomicBool::new(false),
    });
    lock(&host.terminals).insert(id.clone(), term.clone());
    let mut output = started.output;
    let reader_host = host.clone();
    let reader_term = term.clone();
    // The reader reports when the PTY is drained, so `exit` follows the terminal's last bytes.
    let (drained, output_done) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _drained = drained;
        let mut buffer = [0u8; 16384];
        loop {
            let count = match output.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            // Push and emit under the ring lock, so an attachment's replay and the live frames
            // that follow it never overlap or leave a gap.
            let mut ring = lock(&reader_term.ring);
            let seq = ring.push(&buffer[..count]);
            reader_host.emit_terminal(
                &reader_term.id,
                &json!({"event": "output", "id": reader_term.id, "seq": seq, "data": STANDARD.encode(&buffer[..count])}),
            );
        }
    });
    let mut waiter = started.waiter;
    let exit_host = host.clone();
    let exit_term = term.clone();
    std::thread::spawn(move || {
        let code = waiter.wait().ok();
        // A background process that inherited the PTY can keep it open; do not wait for it forever.
        let _ = output_done.recv_timeout(DRAIN_WAIT);
        let codex = exit_host.registry.read().ok().is_some_and(|registry| {
            registry
                .conversation(&exit_term.id)
                .is_some_and(|(_, conversation)| {
                    conversation.provider == crate::registry::Provider::Codex
                })
        });
        let mut ring = lock(&exit_term.ring);
        // Retire the old execution before exposing the stopped terminal. A sender that sees
        // `running == false` may immediately resume it under a new execution ID.
        let mut activity = lock(&exit_host.activity);
        let state = activity.entry(exit_term.id.clone()).or_default();
        state.state = "ended";
        // Codex's RPC observer owns retirement after its final native snapshot. The TUI can
        // exit before the last completed item reaches that observer.
        if !codex {
            state.background.clear();
            state.main_done = false;
            if let Err(error) = exit_host.supervision.observe(
                &exit_term.id,
                prometeu_core::supervision::Observation::Exited {
                    success: code == Some(0),
                },
            ) {
                eprintln!("could not persist supervision process exit: {error}");
            }
        }
        ring.running = false;
        ring.code = code;
        exit_host.emit_terminal(
            &exit_term.id,
            &json!({"event": "exit", "id": exit_term.id, "code": code, "seq": ring.seq}),
        );
    });
    Ok(json!({"pid": pid, "unit": term.unit}))
}

/// Reply with everything after the client's last sequence, then subscribe it to live frames and
/// send the hooks its device missed. The ring and hook locks order all three against new output
/// and new hooks.
fn attach(host: &Arc<Host>, conn: &Conn, id: &Value, params: &Value) -> Result<(), String> {
    let term = find(host, &terminal_id(params)?)?;
    let seq = params.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let ring = lock(&term.ring);
    let (data, reset) = ring.since(seq);
    let mut hooks = lock(&term.hooks);
    let mut clients = lock(&host.clients);
    let client = clients
        .iter_mut()
        .find(|c| c.generation == conn.generation)
        .ok_or("detached")?;
    conn.reply(
        id,
        Ok(json!({
            "data": STANDARD.encode(&data), "reset": reset, "seq": ring.seq,
            "running": ring.running, "code": ring.code, "pid": term.pid,
        })),
    );
    client.subscribed.insert(term.id.clone());
    for hook in hooks.missed(&client.device) {
        client.out.send(&hook);
    }
    if !ring.running {
        client
            .out
            .send(&json!({"event": "exit", "id": term.id, "code": ring.code, "seq": ring.seq}));
    }
    Ok(())
}

fn serve_hook(host: &Arc<Host>, mut stream: UnixStream, hook: &Value) -> Result<(), String> {
    let session = hook
        .pointer(&format!("/env/{}SESSION", prometeu_host::HOOK_ENV_PREFIX))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let permission = hook.get("permission").and_then(Value::as_bool) == Some(true);
    observe(host, &session, hook);
    let number = host.hooks.fetch_add(1, Ordering::Relaxed) + 1;
    let frame = json!({
        "event": "hook", "hook": number, "session": session, "permission": permission,
        "env": hook.get("env").cloned().unwrap_or_else(|| json!({})),
        "payload": hook.get("payload").cloned().unwrap_or_else(|| json!("")),
        "at": now(),
    });
    let mut stdout = Vec::new();
    if permission {
        // Every desktop following the conversation shows the request; the first decision wins
        // and the others are told to withdraw theirs. Without any, Claude's dialog answers.
        let (sender, receiver) = mpsc::channel();
        lock(&host.decisions).insert(
            number,
            Decision {
                sender: sender.clone(),
                waiting: None,
                abstained: HashSet::new(),
                decided: None,
            },
        );
        let reached = host.emit_terminal(&session, &frame);
        if let Some(decision) = lock(&host.decisions).get_mut(&number) {
            decision.waiting = Some(reached.clone());
            // Everyone may have abstained before the recipients were known.
            if decision.decided.is_none() && decision.settled() {
                let _ = sender.send(Vec::new());
            }
        }
        if let Ok(decision) = receiver.recv_timeout(DECISION_WAIT) {
            stdout = decision;
        }
        if let Some(decision) = lock(&host.decisions).remove(&number) {
            // Whoever has not answered withdraws the request: another desktop decided it, or
            // nobody did in time.
            let pending: HashSet<u64> = reached
                .iter()
                .filter(|g| decision.decided != Some(**g) && !decision.abstained.contains(g))
                .copied()
                .collect();
            host.emit_to(&pending, &json!({"event": "hook_resolved", "hook": number}));
        }
    } else if let Ok(term) = find(host, &session) {
        let mut log = lock(&term.hooks);
        host.emit_terminal(&session, &frame);
        log.push(number, frame);
    } else {
        host.emit_terminal(&session, &frame);
    }
    let _ = writeln!(stream, "{}", json!({"stdout": STANDARD.encode(&stdout)}));
    Ok(())
}

/// Keep the conversation's state from its hook, whether or not a desktop is attached.
fn observe(host: &Host, session: &str, hook: &Value) {
    let payload = hook
        .get("payload")
        .and_then(Value::as_str)
        .and_then(|p| STANDARD.decode(p).ok())
        .and_then(|p| serde_json::from_slice::<Value>(&p).ok())
        .unwrap_or(Value::Null);
    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut activity = lock(&host.activity);
    if let Some(observation) = crate::supervision::observation(&payload) {
        if let Err(error) = host.supervision.observe(session, observation) {
            eprintln!("could not persist supervision observation: {error}");
        }
    }
    let entry = activity.entry(session.to_string()).or_default();
    entry.at = now();
    match event {
        "SubagentStart" => {
            if let Some(id) = payload["agent_id"].as_str() {
                entry.background.insert(id.into());
            }
            return;
        }
        "SubagentStop" => {
            if let Some(id) = payload["agent_id"].as_str() {
                entry.background.remove(id);
            }
            return;
        }
        _ if payload["agent_id"].as_str().is_some() => return,
        "Stop" => entry.main_done = true,
        "UserPromptSubmit" => entry.main_done = false,
        "SessionStart" | "SessionEnd" | "StopFailure" => {
            entry.main_done = false;
            entry.background.clear();
        }
        _ => {}
    }
    entry.state = match event {
        "SessionStart" | "Stop" | "StopFailure" => "idle",
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => "working",
        "PermissionRequest" => "permission",
        "SessionEnd" => "ended",
        "Notification" => {
            entry.notification = payload
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            return;
        }
        _ => return,
    };
    if entry.state == "working" {
        entry.notification = None;
    }
}

fn exec(params: &Value) -> Result<Value, String> {
    let argv = string_list(params, "argv");
    let Some(program) = argv.first() else {
        return Err("missing argv".into());
    };
    let mut command = Command::new(program);
    command.args(&argv[1..]);
    if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
        command.current_dir(cwd);
    }
    if let Some(env) = params.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                command.env(key, value);
            }
        }
    }
    let input = params
        .get("stdin")
        .and_then(Value::as_str)
        .map(|data| STANDARD.decode(data))
        .transpose()
        .map_err(|e| e.to_string())?;
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let timeout = Duration::from_millis(
        params
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(30_000)
            .min(600_000),
    );
    let mut child = command.spawn().map_err(|e| format!("{program}: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(EXEC_OUTPUT as u64).read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    Ok(json!({
        "code": status.and_then(|s| s.code()),
        "timeout": status.is_none(),
        "stdout": STANDARD.encode(stdout),
        "stderr": STANDARD.encode(stderr),
    }))
}

fn file_write(params: &Value) -> Result<Value, String> {
    let path = PathBuf::from(text(params, "path")?);
    if !path.is_absolute() {
        return Err("path must be absolute".into());
    }
    let data = bytes(params, "data")?;
    let mode = params.get("mode").and_then(Value::as_u64).unwrap_or(0o600) as u32;
    let parent = path.parent().ok_or("no parent")?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&data).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(mode))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
    Ok(json!({}))
}

fn file_read(params: &Value) -> Result<Value, String> {
    let path = text(params, "path")?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(EXEC_OUTPUT as u64);
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(json!({"data": STANDARD.encode(bytes)}))
}

fn walk(dir: &Path, base: &Path, depth: usize, files: &mut Vec<(String, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() && depth > 0 {
            walk(&path, base, depth - 1, files);
        } else if kind.is_file() {
            if let (Ok(meta), Ok(relative)) = (entry.metadata(), path.strip_prefix(base)) {
                if let Some(relative) = relative.to_str() {
                    files.push((relative.to_string(), meta.len()));
                }
            }
        }
    }
}

/// Stream appends to every regular file under `dir`, starting at the client's offsets. A file
/// shorter than its offset was replaced and restarts from zero with `reset`.
fn watch(conn: &Conn, params: &Value) -> Result<(), String> {
    let id = terminal_id(params)?;
    let dir = PathBuf::from(text(params, "dir")?);
    if !dir.is_absolute() {
        return Err("dir must be absolute".into());
    }
    let depth = params
        .get("depth")
        .and_then(Value::as_u64)
        .unwrap_or(3)
        .min(6) as usize;
    let mut offsets: HashMap<String, u64> = params
        .get("offsets")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_u64().map(|v| (k.clone(), v)))
                .collect()
        })
        .unwrap_or_default();
    let (flush, flushes) = mpsc::channel::<mpsc::Sender<()>>();
    lock(&conn.watches).insert(id.clone(), flush);
    let conn = conn.clone();
    std::thread::spawn(move || {
        // One pass sends at most one chunk per file; false when nothing was new.
        let scan = |offsets: &mut HashMap<String, u64>| -> Option<bool> {
            let mut files = Vec::new();
            walk(&dir, &dir, depth, &mut files);
            let mut sent = false;
            for (relative, size) in files {
                if size > WATCH_FILE_LIMIT {
                    continue;
                }
                let known = offsets.get(&relative).copied().unwrap_or(0);
                let (start, reset) = if size < known {
                    (0, true)
                } else {
                    (known, false)
                };
                if size == start && !reset {
                    continue;
                }
                let mut data = Vec::new();
                let read = std::fs::File::open(dir.join(&relative)).and_then(|mut file| {
                    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(start))?;
                    file.take(WATCH_CHUNK as u64).read_to_end(&mut data)
                });
                if read.is_err() {
                    continue;
                }
                let frame = json!({
                    "event": "file", "watch": id, "path": relative, "offset": start,
                    "reset": reset, "data": STANDARD.encode(&data),
                });
                conn.out.wait_for_room();
                if !conn.out.send(&frame) {
                    return None;
                }
                sent = true;
                offsets.insert(relative, start + data.len() as u64);
            }
            Some(sent)
        };
        while conn.out.alive.load(Ordering::Acquire) {
            if scan(&mut offsets).is_none() {
                return;
            }
            match flushes.recv_timeout(Duration::from_millis(250)) {
                Ok(done) => {
                    // Send everything written so far before answering.
                    loop {
                        match scan(&mut offsets) {
                            None => return,
                            Some(true) => continue,
                            Some(false) => break,
                        }
                    }
                    let _ = done.send(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
    Ok(())
}

fn cgroup_base() -> Option<PathBuf> {
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    let base = PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/prometeu.slice"
    ));
    base.is_dir().then_some(base)
}

fn read_number(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn cpu_usage(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path.join("cpu.stat"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("usage_usec "))
        .and_then(|v| v.trim().parse().ok())
}

/// Resource use per workspace slice plus the host's totals. CPU is cumulative microseconds; the
/// desktop derives a rate from two samples.
fn stats(host: &Host, params: &Value) -> Value {
    let base = cgroup_base();
    let mut workspaces = Map::new();
    for workspace in string_list(params, "workspaces") {
        let Some(slice) = workspace_slice(&workspace) else {
            continue;
        };
        let Some(path) = base.as_ref().map(|b| b.join(&slice)) else {
            continue;
        };
        if !path.is_dir() {
            continue;
        }
        let max = std::fs::read_to_string(path.join("memory.max"))
            .ok()
            .map(|v| v.trim().to_string());
        workspaces.insert(
            workspace,
            json!({
                "memory": read_number(&path.join("memory.current")),
                "memory_max": max.and_then(|v| v.parse::<u64>().ok()),
                "cpu_usec": cpu_usage(&path),
                "cpu_max": std::fs::read_to_string(path.join("cpu.max")).ok().map(|v| v.trim().to_string()),
            }),
        );
    }
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    json!({
        "workspaces": workspaces,
        "memory_total": field("MemTotal:"),
        "memory_available": field("MemAvailable:"),
        "load": std::fs::read_to_string("/proc/loadavg").ok().and_then(|l| l.split_whitespace().next().and_then(|v| v.parse::<f64>().ok())),
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "terminals": host.running(),
    })
}

#[cfg(test)]
pub(crate) fn test_host(root: &Path) -> Arc<Host> {
    let registry = Arc::new(crate::registry::Store::new(root));
    let supervision = crate::supervision::Supervision::new(root, registry.clone()).unwrap();
    Arc::new(Host {
        root: root.into(),
        ready: Value::Null,
        scopes: false,
        terminals: Mutex::default(),
        opening: Mutex::default(),
        clients: Mutex::default(),
        generations: AtomicU64::new(0),
        hooks: AtomicU64::new(0),
        decisions: Mutex::default(),
        units: AtomicU64::new(0),
        registry,
        supervision,
        todos: crate::todos::store(root),
        activity: Mutex::default(),
        codex_watch: Mutex::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_retirement_finishes_before_a_stopped_conversation_can_resume() {
        use prometeu_core::supervision::Observation;
        let root = std::env::temp_dir().join(format!(
            "host-retirement-{}",
            crate::agents::uuid().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let host = test_host(&root);
        host.supervision
            .configure(
                &json!({"action":"setup", "url":"https://cloud.example", "token":"test-token",
            "supervisor":{"id":"supervisor", "name":"Supervisor"}}),
            )
            .unwrap();
        host.supervision.record_workspace(serde_json::from_value(json!({
            "id":"workspace", "project":"/repo", "title":"Task", "branch":"feature", "base":"HEAD", "worktree":"/repo",
            "created_at":1, "created_by":{"kind":"agent"}, "supervision":{"supervisor_id":"supervisor"},
            "conversations":[{"id":"conversation", "title":"Task", "created_at":1, "permission":"auto"}]
        })).unwrap()).unwrap();
        let activity = lock(&host.activity);
        let clients = lock(&host.clients);
        open(&host, &json!({"id":"conversation", "argv":["/bin/sh", "-c", "exit 0"], "cwd":root, "cols":80, "rows":24})).unwrap();
        let terminal = find(&host, "conversation").unwrap();
        // The process emits no output, so only its exit waiter can hold this ring. Hold both
        // possible next locks until the waiter reaches retirement, then release only `clients`.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if terminal.ring.try_lock().is_err() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "exit waiter did not reach retirement"
            );
            std::thread::yield_now();
        }
        drop(clients);
        let resumed_host = host.clone();
        let (resumed, received) = mpsc::channel();
        let resume = std::thread::spawn(move || {
            assert!(!resumed_host.is_running("conversation"));
            resumed_host
                .supervision
                .observe("conversation", Observation::Started)
                .unwrap();
            resumed.send(()).unwrap();
        });
        assert!(
            received.recv_timeout(Duration::from_millis(200)).is_err(),
            "resume became available before the previous execution retired"
        );
        drop(activity);
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        resume.join().unwrap();
        host.supervision
            .observe("conversation", Observation::Completed)
            .unwrap();
        let saved = serde_json::to_value(host.registry.read().unwrap()).unwrap();
        let events = saved["supervision"]["outbox"].as_array().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1]["kind"], "execution.failed");
        assert_eq!(events[2]["kind"], "execution.completed");
        assert_ne!(events[1]["execution_id"], events[2]["execution_id"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ring_replays_only_the_gap() {
        let mut ring = Ring::default();
        ring.push(b"a");
        ring.push(b"b");
        ring.push(b"c");
        assert_eq!(ring.since(1), (b"bc".to_vec(), false));
        assert_eq!(ring.since(3), (Vec::new(), false));
        assert_eq!(ring.since(0), (b"abc".to_vec(), false));
    }

    #[test]
    fn ring_resets_when_the_gap_was_dropped() {
        let mut ring = Ring::default();
        let big = vec![b'x'; RETAINED];
        ring.push(b"lost");
        ring.push(&big);
        ring.push(b"tail");
        let (bytes, reset) = ring.since(1);
        assert!(reset);
        assert!(bytes.ends_with(b"tail"));
        assert!(!bytes.starts_with(b"lost"));
    }

    #[test]
    fn a_sequence_from_another_host_resets() {
        let mut ring = Ring::default();
        ring.push(b"a");
        assert!(ring.since(9).1);
    }
}
