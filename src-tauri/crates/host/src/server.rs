//! The resident host. It owns terminals, queues hooks while no desktop is attached and streams
//! transcript files, so execution never depends on the desktop's connection.
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Output retained per terminal for reattachment. A gap beyond it replays the whole buffer.
const RETAINED: usize = 2 * 1_048_576;
/// Hooks queued per terminal while no desktop is attached; the oldest are dropped first.
const QUEUED_HOOKS: usize = 512;
const DECISION_WAIT: Duration = Duration::from_secs(300);
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

struct Term {
    id: String,
    workspace: Option<String>,
    pid: u32,
    started: u64,
    unit: Option<String>,
    input: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn TerminalControl>,
    process: ProcessHandle,
    ring: Mutex<Ring>,
    hooks: Mutex<VecDeque<Value>>,
}

struct Client {
    generation: u64,
    out: mpsc::Sender<String>,
    alive: Arc<AtomicBool>,
    subscribed: HashSet<String>,
}

struct Host {
    root: PathBuf,
    ready: Value,
    scopes: bool,
    terminals: Mutex<HashMap<String, Arc<Term>>>,
    client: Mutex<Option<Client>>,
    generations: AtomicU64,
    hooks: AtomicU64,
    decisions: Mutex<HashMap<u64, mpsc::Sender<Vec<u8>>>>,
    units: AtomicU64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn send(out: &mpsc::Sender<String>, frame: &Value) -> bool {
    out.send(frame.to_string()).is_ok()
}

impl Host {
    /// Deliver a terminal frame when the attached client subscribed to it.
    fn emit_terminal(&self, id: &str, frame: &Value) -> bool {
        let client = lock(&self.client);
        match client.as_ref() {
            Some(client) if client.subscribed.contains(id) => send(&client.out, frame),
            _ => false,
        }
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

fn describe(root: &Path, scopes: bool, linger: bool) -> Value {
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
        "executable": std::env::current_exe().ok().map(|p| p.display().to_string()),
        "pid": std::process::id(),
        "home": std::env::var("HOME").unwrap_or_default(),
        "user": std::env::var("USER").unwrap_or_default(),
        "root": root.display().to_string(),
        "os": uname("-s"),
        "arch": uname("-m"),
        "linger": linger,
        "scopes": scopes,
    })
}

pub fn serve(root: &Path) -> Result<(), String> {
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
    let path = root.join(socket_name());
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|e| e.to_string())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    let linger = linger();
    let scopes = linger && command_ok("systemd-run", &["--version"]);
    let host = Arc::new(Host {
        root: root.into(),
        ready: describe(root, scopes, linger),
        scopes,
        terminals: Mutex::new(HashMap::new()),
        client: Mutex::new(None),
        generations: AtomicU64::new(0),
        hooks: AtomicU64::new(0),
        decisions: Mutex::new(HashMap::new()),
        units: AtomicU64::new(0),
    });
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

fn read_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
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
    if let Some(hook) = first.get("hook") {
        return serve_hook(host, stream, hook);
    }
    if first.pointer("/attach/protocol").and_then(Value::as_u64) != Some(PROTOCOL as u64) {
        let mut stream = stream;
        let _ = writeln!(stream, "{}", json!({"event": "error", "error": "protocol"}));
        return Ok(());
    }
    serve_client(host, stream, reader)
}

/// A new attachment replaces the previous one: after sleep the old SSH connection can look
/// alive for minutes, and the person's current desktop must win.
fn serve_client(
    host: &Arc<Host>,
    stream: UnixStream,
    mut reader: BufReader<UnixStream>,
) -> Result<(), String> {
    let (out, lines) = mpsc::channel::<String>();
    let alive = Arc::new(AtomicBool::new(true));
    let generation = host.generations.fetch_add(1, Ordering::Relaxed) + 1;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let writer_alive = alive.clone();
    let shutdown = stream.try_clone().map_err(|e| e.to_string())?;
    std::thread::spawn(move || {
        for mut line in lines {
            line.push('\n');
            if writer.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
        writer_alive.store(false, Ordering::Release);
        let _ = shutdown.shutdown(std::net::Shutdown::Both);
    });
    send(&out, &host.ready);
    if let Some(previous) = lock(&host.client).replace(Client {
        generation,
        out: out.clone(),
        alive: alive.clone(),
        subscribed: HashSet::new(),
    }) {
        previous.alive.store(false, Ordering::Release);
        let _ = previous.out.send(json!({"event": "replaced"}).to_string());
    }
    let conn = Conn {
        out: out.clone(),
        alive: alive.clone(),
        generation,
    };
    while alive.load(Ordering::Acquire) {
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
                send(&out, &json!({"id": null, "error": error.to_string()}));
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
    alive.store(false, Ordering::Release);
    let mut client = lock(&host.client);
    if client.as_ref().is_some_and(|c| c.generation == generation) {
        *client = None;
    }
    Ok(())
}

#[derive(Clone)]
struct Conn {
    out: mpsc::Sender<String>,
    alive: Arc<AtomicBool>,
    generation: u64,
}
impl Conn {
    fn reply(&self, id: &Value, result: Result<Value, String>) {
        let frame = match result {
            Ok(result) => json!({"id": id, "result": result}),
            Err(error) => json!({"id": id, "error": error}),
        };
        send(&self.out, &frame);
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

fn find(host: &Host, id: &str) -> Result<Arc<Term>, String> {
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
        "exec" => exec(params)?,
        "file_write" => file_write(params)?,
        "file_read" => file_read(params)?,
        "terminal_open" => open(host, params)?,
        "terminal_write" => {
            let term = find(host, &terminal_id(params)?)?;
            let data = bytes(params, "data")?;
            let mut input = lock(&term.input);
            input.write_all(&data).map_err(|e| e.to_string())?;
            input.flush().map_err(|e| e.to_string())?;
            json!({})
        }
        "terminal_resize" => {
            let term = find(host, &terminal_id(params)?)?;
            let size = size(params)?;
            term.control.resize(size)?;
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
            let mut terminals = lock(&host.terminals);
            if terminals.get(&id).is_some_and(|t| lock(&t.ring).running) {
                return Err("terminal_running".into());
            }
            terminals.remove(&id);
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
                            "id": t.id, "workspace": t.workspace, "pid": t.pid, "unit": t.unit,
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
        "hook_reply" => {
            let hook = params
                .get("hook")
                .and_then(Value::as_u64)
                .ok_or("missing hook")?;
            let stdout = bytes(params, "stdout").unwrap_or_default();
            if let Some(sender) = lock(&host.decisions).remove(&hook) {
                let _ = sender.send(stdout);
            }
            json!({})
        }
        "stats" => stats(host, params),
        "shutdown" => {
            let force = params.get("force").and_then(Value::as_bool) == Some(true);
            if !force && host.running() > 0 {
                return Err("host_busy".into());
            }
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

fn open(host: &Arc<Host>, params: &Value) -> Result<Value, String> {
    let id = terminal_id(params)?;
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
    {
        let terminals = lock(&host.terminals);
        if terminals.get(&id).is_some_and(|t| lock(&t.ring).running) {
            return Err("terminal_exists".into());
        }
    }
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
        hooks: Mutex::new(VecDeque::new()),
    });
    lock(&host.terminals).insert(id.clone(), term.clone());
    let mut output = started.output;
    let reader_host = host.clone();
    let reader_term = term.clone();
    std::thread::spawn(move || {
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
        let mut ring = lock(&exit_term.ring);
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
/// flush hooks queued while it was away. The ring lock orders all three against new output.
fn attach(host: &Arc<Host>, conn: &Conn, id: &Value, params: &Value) -> Result<(), String> {
    let term = find(host, &terminal_id(params)?)?;
    let seq = params.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let ring = lock(&term.ring);
    let (data, reset) = ring.since(seq);
    {
        let mut client = lock(&host.client);
        let client = client
            .as_mut()
            .filter(|c| c.generation == conn.generation)
            .ok_or("detached")?;
        conn.reply(
            id,
            Ok(json!({
                "data": STANDARD.encode(&data), "reset": reset, "seq": ring.seq,
                "running": ring.running, "code": ring.code, "pid": term.pid,
            })),
        );
        client.subscribed.insert(term.id.clone());
        let queued: Vec<Value> = lock(&term.hooks).drain(..).collect();
        for hook in queued {
            send(&client.out, &hook);
        }
        if !ring.running {
            send(
                &client.out,
                &json!({"event": "exit", "id": term.id, "code": ring.code, "seq": ring.seq}),
            );
        }
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
    let number = host.hooks.fetch_add(1, Ordering::Relaxed) + 1;
    let frame = json!({
        "event": "hook", "hook": number, "session": session, "permission": permission,
        "env": hook.get("env").cloned().unwrap_or_else(|| json!({})),
        "payload": hook.get("payload").cloned().unwrap_or_else(|| json!("")),
        "at": now(),
    });
    let mut stdout = Vec::new();
    if permission {
        let (sender, receiver) = mpsc::channel();
        lock(&host.decisions).insert(number, sender);
        if host.emit_terminal(&session, &frame) {
            if let Ok(decision) = receiver.recv_timeout(DECISION_WAIT) {
                stdout = decision;
            }
        }
        lock(&host.decisions).remove(&number);
    } else if !host.emit_terminal(&session, &frame) {
        if let Ok(term) = find(host, &session) {
            let mut queue = lock(&term.hooks);
            if queue.len() >= QUEUED_HOOKS {
                queue.pop_front();
            }
            queue.push_back(frame);
        }
    }
    let _ = writeln!(stream, "{}", json!({"stdout": STANDARD.encode(&stdout)}));
    Ok(())
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
    let conn = conn.clone();
    std::thread::spawn(move || {
        while conn.alive.load(Ordering::Acquire) {
            let mut files = Vec::new();
            walk(&dir, &dir, depth, &mut files);
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
                if !send(&conn.out, &frame) {
                    return;
                }
                offsets.insert(relative, start + data.len() as u64);
            }
            std::thread::sleep(Duration::from_millis(250));
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
mod tests {
    use super::*;

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
