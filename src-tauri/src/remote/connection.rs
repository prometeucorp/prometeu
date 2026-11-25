//! One SSH attachment to a machine's resident host (ADR 0101). The SSH process is disposable:
//! when it dies (sleep, network change, VPS reboot) the loop reconnects with backoff and
//! reattaches every terminal and transcript mirror from the last position this app applied.
use base64::{engine::general_purpose::STANDARD, Engine};
use prometeu_core::lock::lock;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use super::Machine;

pub const OFFLINE: &str = "err.remote.offline";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const BACKOFF: [u64; 7] = [1, 2, 3, 5, 10, 15, 30];

/// Receives one remote terminal's stream. Implementations order replay and live output by the
/// host's sequence numbers and must not block: they run on the connection's reader thread.
pub trait TerminalSink: Send + Sync {
    fn replay(&self, data: Vec<u8>, reset: bool, seq: u64);
    fn output(&self, seq: u64, data: Vec<u8>);
    fn exit(&self, code: Option<u32>);
    /// The host no longer knows the terminal (it restarted): its process is gone.
    fn lost(&self);
    fn last_seq(&self) -> u64;
}

/// Answers a hook relayed from the machine. Runs on its own thread; may wait for a decision.
pub trait HookSink: Send + Sync {
    fn hook(&self, frame: &Value) -> Vec<u8>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Offline,
    Connecting,
    Connected,
    NotInstalled,
    Replaced,
    Failed,
}

type Callback = Box<dyn FnOnce(Result<Value, String>) + Send>;

/// Why the machine needs this app's attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A new attachment, once terminals and mirrors are reattached.
    Attached,
    /// An agent changed the machine's registry (ADR 0103).
    Registry,
}

/// Runs on its own thread after every attachment and registry change.
pub type Attached = Box<dyn Fn(&Arc<Connection>, Wake) + Send + Sync>;
enum Pending {
    Wait(mpsc::Sender<Result<Value, String>>),
    /// Runs on the reader thread, before any later frame: used where order matters.
    Inline(Callback),
}

/// Mirrors a remote directory of append-only files under the app's private root, so readers that
/// expect local files (the terminal transcript observer) work unchanged.
pub struct Mirror {
    pub id: String,
    pub remote: String,
    pub local: PathBuf,
    pub depth: u32,
}

impl Mirror {
    fn relative(path: &str) -> Option<PathBuf> {
        let path = Path::new(path);
        path.components()
            .all(|c| matches!(c, Component::Normal(_)))
            .then(|| path.to_path_buf())
    }

    fn offsets(&self) -> Map<String, Value> {
        let mut offsets = Map::new();
        let mut stack = vec![(self.local.clone(), 0u32)];
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() && depth < self.depth {
                    stack.push((path, depth + 1));
                } else if kind.is_file() {
                    if let (Ok(meta), Ok(relative)) =
                        (entry.metadata(), path.strip_prefix(&self.local))
                    {
                        if let Some(relative) = relative.to_str() {
                            offsets.insert(relative.into(), json!(meta.len()));
                        }
                    }
                }
            }
        }
        offsets
    }

    fn apply(&self, path: &str, offset: u64, reset: bool, data: &[u8]) -> Result<(), String> {
        use std::io::{Seek, SeekFrom};
        use std::os::unix::fs::OpenOptionsExt;
        let relative = Self::relative(path).ok_or("invalid mirror path")?;
        let target = self.local.join(relative);
        if let Some(parent) = target.parent() {
            crate::paths::ensure_private_dir(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&target)
            .map_err(|e| e.to_string())?;
        if reset {
            file.set_len(0).map_err(|e| e.to_string())?;
        } else if file.metadata().map_err(|e| e.to_string())?.len() > offset {
            file.set_len(offset).map_err(|e| e.to_string())?;
        }
        file.seek(SeekFrom::Start(if reset { 0 } else { offset }))
            .map_err(|e| e.to_string())?;
        file.write_all(data).map_err(|e| e.to_string())
    }
}

pub struct Connection {
    pub id: String,
    machine: Mutex<Machine>,
    phase: Mutex<(Phase, Option<String>)>,
    ready: Mutex<Option<Value>>,
    update_pending: AtomicBool,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending: Mutex<HashMap<u64, Pending>>,
    next: AtomicU64,
    terminals: Mutex<HashMap<String, Arc<dyn TerminalSink>>>,
    mirrors: Mutex<HashMap<String, Arc<Mirror>>>,
    hooks: Mutex<HashMap<String, Arc<dyn HookSink>>>,
    wanted: AtomicBool,
    looping: AtomicBool,
    changed: Box<dyn Fn() + Send + Sync>,
    attached: Attached,
}

impl Connection {
    pub fn new(
        machine: Machine,
        changed: Box<dyn Fn() + Send + Sync>,
        attached: Attached,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: machine.id.clone(),
            machine: Mutex::new(machine),
            phase: Mutex::new((Phase::Offline, None)),
            ready: Mutex::new(None),
            update_pending: AtomicBool::new(false),
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            terminals: Mutex::new(HashMap::new()),
            mirrors: Mutex::new(HashMap::new()),
            hooks: Mutex::new(HashMap::new()),
            wanted: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            changed,
            attached,
        })
    }

    pub fn machine(&self) -> Machine {
        lock(&self.machine).clone()
    }
    pub fn set_machine(&self, machine: Machine) {
        *lock(&self.machine) = machine;
    }
    pub fn phase(&self) -> (Phase, Option<String>) {
        lock(&self.phase).clone()
    }
    pub fn ready(&self) -> Option<Value> {
        lock(&self.ready).clone()
    }
    pub fn update_pending(&self) -> bool {
        self.update_pending.load(Ordering::Acquire)
    }
    pub fn home(&self) -> Option<String> {
        self.ready()
            .and_then(|r| r.get("home").and_then(Value::as_str).map(str::to_owned))
    }

    fn set_phase(&self, phase: Phase, detail: Option<String>) {
        *lock(&self.phase) = (phase, detail);
        (self.changed)();
    }

    /// Keep this machine attached, reconnecting until `stop`.
    pub fn start(self: &Arc<Self>) {
        self.wanted.store(true, Ordering::Release);
        if self.looping.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        std::thread::Builder::new()
            .name(format!("remote-{}", self.id))
            .spawn(move || {
                this.run();
                this.looping.store(false, Ordering::Release);
            })
            .ok();
    }

    pub fn stop(&self) {
        self.wanted.store(false, Ordering::Release);
        if let Some(mut child) = lock(&self.child).take() {
            let _ = child.kill();
        }
    }

    /// Drop the current SSH process; the loop reconnects at once.
    pub fn reconnect(self: &Arc<Self>) {
        if let Some(mut child) = lock(&self.child).take() {
            let _ = child.kill();
        }
        self.start();
    }

    fn run(self: &Arc<Self>) {
        let mut attempt = 0usize;
        while self.wanted.load(Ordering::Acquire) {
            self.set_phase(Phase::Connecting, None);
            let started = Instant::now();
            let outcome = self.session();
            self.fail_pending();
            *lock(&self.ready) = None;
            match outcome {
                Outcome::NotInstalled => {
                    self.set_phase(Phase::NotInstalled, None);
                    self.wanted.store(false, Ordering::Release);
                    return;
                }
                Outcome::Replaced => {
                    self.set_phase(Phase::Replaced, None);
                    self.wanted.store(false, Ordering::Release);
                    return;
                }
                Outcome::Failed(detail) => self.set_phase(Phase::Failed, Some(detail)),
                Outcome::Closed => self.set_phase(Phase::Offline, None),
            }
            if !self.wanted.load(Ordering::Acquire) {
                self.set_phase(Phase::Offline, None);
                return;
            }
            if started.elapsed() > Duration::from_secs(60) {
                attempt = 0;
            }
            let wait = Duration::from_secs(BACKOFF[attempt.min(BACKOFF.len() - 1)]);
            attempt += 1;
            let deadline = Instant::now() + wait;
            while Instant::now() < deadline && self.wanted.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    fn fail_pending(&self) {
        let pending: Vec<Pending> = lock(&self.pending).drain().map(|(_, p)| p).collect();
        for pending in pending {
            match pending {
                Pending::Wait(sender) => {
                    let _ = sender.send(Err(crate::i18n::t(OFFLINE)));
                }
                Pending::Inline(callback) => callback(Err(crate::i18n::t(OFFLINE))),
            }
        }
    }

    fn session(self: &Arc<Self>) -> Outcome {
        let machine = self.machine();
        let Some(binary) = super::install::local_binary() else {
            return Outcome::Failed("prometeu-host build missing".into());
        };
        let root = super::install::remote_root();
        let command = format!(
            "exec \"$HOME/{root}/host/{}/prometeu-host\" attach --root \"$HOME/{root}\"",
            binary.hash
        );
        let mut child = match super::install::ssh(&machine.ssh)
            .arg(&command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        *lock(&self.stdin) = child.stdin.take();
        *lock(&self.child) = Some(child);
        let (tail_sender, tail) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stderr) = stderr.as_mut() {
                let _ = stderr.take(64 * 1024).read_to_end(&mut bytes);
            }
            let _ = tail_sender.send(String::from_utf8_lossy(&bytes).to_string());
        });
        let mut ready_seen = false;
        let mut replaced = false;
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let Ok(frame) = serde_json::from_str::<Value>(line.trim_end()) else {
                    continue;
                };
                match frame.get("event").and_then(Value::as_str) {
                    Some("ready") => {
                        ready_seen = true;
                        self.attach_ready(frame);
                    }
                    Some("replaced") => replaced = true,
                    Some(_) => self.event(&frame),
                    None => self.reply(frame),
                }
            }
        }
        *lock(&self.stdin) = None;
        let status = lock(&self.child).take().and_then(|mut child| {
            let _ = child.kill();
            child.wait().ok()
        });
        let stderr = tail
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or_default();
        if replaced {
            return Outcome::Replaced;
        }
        if ready_seen {
            return Outcome::Closed;
        }
        if status.and_then(|s| s.code()) == Some(127) || stderr.contains("No such file") {
            return Outcome::NotInstalled;
        }
        let detail = stderr.trim().lines().last().unwrap_or("").to_string();
        Outcome::Failed(if detail.is_empty() {
            "ssh closed".into()
        } else {
            detail
        })
    }

    fn attach_ready(self: &Arc<Self>, frame: Value) {
        let expected = super::install::local_binary().map(|b| b.hash.clone());
        let current = frame
            .get("executable")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        *lock(&self.ready) = Some(frame);
        let stale = expected.is_some_and(|hash| !current.contains(&hash));
        self.update_pending.store(stale, Ordering::Release);
        self.set_phase(Phase::Connected, None);
        let this = self.clone();
        std::thread::spawn(move || {
            if stale && this.retire_stale() {
                return;
            }
            this.reattach();
            (this.attached)(&this, Wake::Attached);
        });
    }

    /// An older host build still serves the socket. Replace it only while it runs nothing, so
    /// an update never ends live work; otherwise it keeps serving with the same protocol.
    fn retire_stale(self: &Arc<Self>) -> bool {
        let idle = self
            .request("terminal_list", json!({}))
            .ok()
            .and_then(|list| list.as_array().cloned())
            .is_some_and(|list| list.iter().all(|t| t["running"] != json!(true)));
        if idle && self.request("shutdown", json!({})).is_ok() {
            self.reconnect();
            return true;
        }
        false
    }

    fn reattach(self: &Arc<Self>) {
        let terminals: Vec<(String, Arc<dyn TerminalSink>)> = lock(&self.terminals)
            .iter()
            .map(|(id, sink)| (id.clone(), sink.clone()))
            .collect();
        for (id, sink) in terminals {
            self.attach_terminal(&id, sink);
        }
        let mirrors: Vec<Arc<Mirror>> = lock(&self.mirrors).values().cloned().collect();
        for mirror in mirrors {
            let _ = self.send_watch(&mirror);
        }
    }

    fn attach_terminal(&self, id: &str, sink: Arc<dyn TerminalSink>) {
        let seq = sink.last_seq();
        self.request_inline(
            "terminal_attach",
            json!({"id": id, "seq": seq}),
            Box::new(move |result| match result {
                Ok(reply) => {
                    let data = reply["data"]
                        .as_str()
                        .and_then(|d| STANDARD.decode(d).ok())
                        .unwrap_or_default();
                    let reset = reply["reset"].as_bool().unwrap_or(false);
                    sink.replay(data, reset, reply["seq"].as_u64().unwrap_or(0));
                }
                Err(error) if error.contains("unknown_terminal") => sink.lost(),
                Err(_) => {}
            }),
        );
    }

    fn send_watch(&self, mirror: &Mirror) -> Result<Value, String> {
        self.request(
            "watch",
            json!({"id": mirror.id, "dir": mirror.remote, "depth": mirror.depth, "offsets": mirror.offsets()}),
        )
    }

    /// Follow a terminal from the host's current output; it is reattached after reconnecting.
    pub fn subscribe(&self, id: &str, sink: Arc<dyn TerminalSink>) {
        lock(&self.terminals).insert(id.into(), sink.clone());
        self.attach_terminal(id, sink);
    }

    /// Stop following a terminal once it ended, unless a newer launch already replaced `sink`.
    pub fn release(&self, id: &str, sink: *const ()) {
        let mut terminals = lock(&self.terminals);
        if terminals
            .get(id)
            .is_some_and(|current| Arc::as_ptr(current) as *const () == sink)
        {
            terminals.remove(id);
        }
    }

    pub fn mirror(&self, mirror: Mirror) -> Result<(), String> {
        let mirror = Arc::new(mirror);
        let existing = lock(&self.mirrors).insert(mirror.id.clone(), mirror.clone());
        if existing.is_some() {
            return Ok(());
        }
        self.send_watch(&mirror).map(|_| ())
    }

    pub fn on_hook(&self, session: &str, sink: Arc<dyn HookSink>) {
        lock(&self.hooks).insert(session.into(), sink);
    }

    fn event(self: &Arc<Self>, frame: &Value) {
        let id = frame.get("id").and_then(Value::as_str).unwrap_or_default();
        match frame.get("event").and_then(Value::as_str) {
            Some("output") => {
                let sink = lock(&self.terminals).get(id).cloned();
                if let (Some(sink), Some(data)) = (sink, frame["data"].as_str()) {
                    if let Ok(data) = STANDARD.decode(data) {
                        sink.output(frame["seq"].as_u64().unwrap_or(0), data);
                    }
                }
            }
            Some("exit") => {
                let sink = lock(&self.terminals).get(id).cloned();
                if let Some(sink) = sink {
                    sink.exit(frame["code"].as_u64().map(|c| c as u32));
                }
            }
            Some("file") => {
                let watch = frame["watch"].as_str().unwrap_or_default();
                let mirror = lock(&self.mirrors).get(watch).cloned();
                if let (Some(mirror), Some(data)) = (mirror, frame["data"].as_str()) {
                    if let Ok(data) = STANDARD.decode(data) {
                        let _ = mirror.apply(
                            frame["path"].as_str().unwrap_or_default(),
                            frame["offset"].as_u64().unwrap_or(0),
                            frame["reset"].as_bool().unwrap_or(false),
                            &data,
                        );
                    }
                }
            }
            Some("registry") => {
                let this = self.clone();
                std::thread::spawn(move || (this.attached)(&this, Wake::Registry));
            }
            Some("hook") => {
                let session = frame["session"].as_str().unwrap_or_default();
                let sink = lock(&self.hooks).get(session).cloned();
                let number = frame["hook"].clone();
                let frame = frame.clone();
                let this = self.clone();
                std::thread::spawn(move || {
                    let stdout = sink.map(|sink| sink.hook(&frame)).unwrap_or_default();
                    this.notify(
                        "hook_reply",
                        json!({"hook": number, "stdout": STANDARD.encode(stdout)}),
                    );
                });
            }
            _ => {}
        }
    }

    fn reply(&self, frame: Value) {
        let Some(id) = frame.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(pending) = lock(&self.pending).remove(&id) else {
            return;
        };
        let result = match frame.get("error") {
            Some(error) => Err(error.as_str().unwrap_or("remote error").to_string()),
            None => Ok(frame.get("result").cloned().unwrap_or(Value::Null)),
        };
        match pending {
            Pending::Wait(sender) => {
                let _ = sender.send(result);
            }
            Pending::Inline(callback) => callback(result),
        }
    }

    fn write(&self, id: u64, method: &str, params: Value) -> Result<(), String> {
        let mut line = json!({"id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        let mut stdin = lock(&self.stdin);
        let stdin = stdin.as_mut().ok_or_else(|| crate::i18n::t(OFFLINE))?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|_| crate::i18n::t(OFFLINE))
    }

    pub fn connected(&self) -> bool {
        self.phase().0 == Phase::Connected
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_timeout(method, params, REQUEST_TIMEOUT)
    }

    pub fn request_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = mpsc::channel();
        lock(&self.pending).insert(id, Pending::Wait(sender));
        if let Err(error) = self.write(id, method, params) {
            lock(&self.pending).remove(&id);
            return Err(error);
        }
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(crate::i18n::t(OFFLINE))
            }
        }
    }

    fn request_inline(&self, method: &str, params: Value, callback: Callback) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.pending).insert(id, Pending::Inline(callback));
        if let Err(error) = self.write(id, method, params) {
            if let Some(Pending::Inline(callback)) = lock(&self.pending).remove(&id) {
                callback(Err(error));
            }
        }
    }

    /// Send without waiting: terminal input and resizes keep their order on the stream and a
    /// lost reply has an unknown outcome, as for every other remote write.
    pub fn notify(&self, method: &str, params: Value) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.write(id, method, params);
    }

    /// Run a program on the machine and return its exit code and output.
    pub fn exec(
        &self,
        argv: &[&str],
        cwd: Option<&str>,
        timeout: Duration,
    ) -> Result<Exec, String> {
        let reply = self.request_timeout(
            "exec",
            json!({"argv": argv, "cwd": cwd, "timeout_ms": timeout.as_millis() as u64}),
            timeout + Duration::from_secs(10),
        )?;
        let decode = |key: &str| {
            reply[key]
                .as_str()
                .and_then(|d| STANDARD.decode(d).ok())
                .map(|b| String::from_utf8_lossy(&b).to_string())
                .unwrap_or_default()
        };
        Ok(Exec {
            code: reply["code"].as_i64(),
            stdout: decode("stdout"),
            stderr: decode("stderr"),
        })
    }

    pub fn write_file(&self, path: &str, data: &[u8], mode: u32) -> Result<(), String> {
        self.request(
            "file_write",
            json!({"path": path, "data": STANDARD.encode(data), "mode": mode}),
        )
        .map(|_| ())
    }
}

pub struct Exec {
    pub code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
}

impl Exec {
    /// Succeed with stdout, or fail with the program's own diagnostics.
    pub fn ok(self) -> Result<String, String> {
        if self.code == Some(0) {
            Ok(self.stdout)
        } else {
            let detail = if self.stderr.trim().is_empty() {
                self.stdout
            } else {
                self.stderr
            };
            Err(crate::i18n::ta(
                "err.remote.command",
                &[("detail", detail.trim().to_string())],
            ))
        }
    }
}

enum Outcome {
    Closed,
    NotInstalled,
    Replaced,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_paths_stay_inside_the_mirror() {
        assert!(Mirror::relative("a/b.jsonl").is_some());
        assert!(Mirror::relative("../escape").is_none());
        assert!(Mirror::relative("/abs").is_none());
    }

    #[test]
    fn mirror_applies_appends_and_resets() {
        let local = std::env::temp_dir().join(format!("mirror-{}", uuid::Uuid::new_v4()));
        let mirror = Mirror {
            id: "m".into(),
            remote: "/r".into(),
            local: local.clone(),
            depth: 2,
        };
        mirror.apply("s/a.jsonl", 0, false, b"one\n").unwrap();
        mirror.apply("s/a.jsonl", 4, false, b"two\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("s/a.jsonl")).unwrap(),
            "one\ntwo\n"
        );
        assert_eq!(mirror.offsets()["s/a.jsonl"], 8);
        mirror.apply("s/a.jsonl", 0, true, b"new\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("s/a.jsonl")).unwrap(),
            "new\n"
        );
        let _ = std::fs::remove_dir_all(local);
    }
}
