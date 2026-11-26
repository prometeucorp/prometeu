//! The two short-lived modes: the SSH attachment proxy and the native hook command.
use base64::{engine::general_purpose::STANDARD, Engine};
use prometeu_host::{socket_name, HOOK_ENV_PREFIX, HOOK_INPUT_LIMIT, PROTOCOL, ROOT};
use serde_json::{json, Map, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn default_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(ROOT)
}

pub fn socket(root: &Path) -> PathBuf {
    root.join(socket_name())
}

/// Start the resident host in its own session, detached from the SSH session that asked for it:
/// the SSH hangup reaches neither it nor the terminals it owns. `release` is the desktop app's
/// version, which the host reports so computers on older apps leave it running (ADR 0104).
fn spawn_server(root: &Path, release: Option<&str>) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("host.log"))
        .map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = Command::new(exe);
    command
        .args(["serve", "--root"])
        .arg(root)
        .args(release.map(|r| ["--release", r]).into_iter().flatten())
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and touches no Rust state in the child.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn connect_or_start(root: &Path, release: Option<&str>) -> Result<UnixStream, String> {
    let path = socket(root);
    if let Ok(stream) = UnixStream::connect(&path) {
        return Ok(stream);
    }
    spawn_server(root, release)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match UnixStream::connect(&path) {
            Ok(stream) => return Ok(stream),
            Err(error) if Instant::now() >= deadline => {
                return Err(format!("host did not start: {error}"))
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Relay this process's stdio to the host. Either side closing ends the relay; the host only
/// detaches the client and keeps every terminal running. `device` and `label` identify the desktop,
/// so attachments from different devices coexist (ADR 0104).
pub fn attach(
    root: &Path,
    device: Option<&str>,
    label: Option<&str>,
    release: Option<&str>,
) -> Result<(), String> {
    let mut stream = connect_or_start(root, release)?;
    let mut hello =
        json!({"attach": {"protocol": PROTOCOL, "device": device, "label": label}}).to_string();
    hello.push('\n');
    stream
        .write_all(hello.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut upstream = stream.try_clone().map_err(|e| e.to_string())?;
    let writer = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut upstream);
        let _ = upstream.shutdown(std::net::Shutdown::Write);
    });
    let mut stdout = std::io::stdout().lock();
    let mut buffer = [0u8; 65536];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if stdout.write_all(&buffer[..count]).is_err() || stdout.flush().is_err() {
                    break;
                }
            }
        }
    }
    drop(writer);
    Ok(())
}

fn read_stdin(limit: usize, deadline: Instant) -> Result<Vec<u8>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = std::io::stdin()
            .lock()
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| "hook input timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("hook input too large".into());
    }
    Ok(bytes)
}

/// Forward one native hook invocation. Claude's PermissionRequest waits for the desktop's
/// decision; every other hook returns as soon as the host has queued it.
pub fn hook(root: &Path) -> Result<(), String> {
    let payload = read_stdin(HOOK_INPUT_LIMIT, Instant::now() + Duration::from_secs(2))?;
    let permission = serde_json::from_slice::<Value>(&payload)
        .ok()
        .and_then(|value| {
            value
                .get("hook_event_name")
                .and_then(Value::as_str)
                .map(|name| name == "PermissionRequest")
        })
        .unwrap_or(false);
    let env: Map<String, Value> = std::env::vars()
        .filter(|(key, _)| key.starts_with(HOOK_ENV_PREFIX))
        .map(|(key, value)| (key, Value::String(value)))
        .collect();
    let mut stream = UnixStream::connect(socket(root)).map_err(|e| e.to_string())?;
    let wait = if permission {
        Duration::from_secs(305)
    } else {
        Duration::from_secs(3)
    };
    stream
        .set_read_timeout(Some(wait))
        .map_err(|e| e.to_string())?;
    let mut line = json!({"hook": {"env": env, "payload": STANDARD.encode(&payload), "permission": permission}})
        .to_string();
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|e| e.to_string())?;
    let reply: Value = serde_json::from_str(&reply).map_err(|e| e.to_string())?;
    let stdout = reply
        .get("stdout")
        .and_then(Value::as_str)
        .map(|data| STANDARD.decode(data))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if !stdout.is_empty() {
        let mut out = std::io::stdout().lock();
        out.write_all(&stdout).map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}
