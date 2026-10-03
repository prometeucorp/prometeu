//! Private Unix transport adapter. Execution and request policies stay in the injected host.
use crate::{
    host::{self, Host},
    RuntimeEvents,
};
use prometeu_core::lock::lock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{self, BufReader, Read, Write},
    net::Shutdown,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogMode {
    #[default]
    Workspace,
    Application,
}
impl CatalogMode {
    pub fn argument(&self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Application => "application",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub v: u32,
    pub workdir: PathBuf,
    pub codex: String,
    pub model: String,
    pub shell: PathBuf,
}
const BUDGET: usize = crate::host::MAX_RESPONSE;
struct Delivery {
    queue: mpsc::SyncSender<Vec<u8>>,
    bytes: Arc<AtomicUsize>,
    socket: UnixStream,
}
#[derive(Default)]
struct DeliveryHub(Mutex<Option<Delivery>>);
impl RuntimeEvents for DeliveryHub {
    fn publish(&self, frame: Value) -> Result<(), String> {
        let mut current = lock(&self.0);
        let Some(delivery) = current.as_ref() else {
            return Ok(());
        };
        let bytes = format!("{frame}\n").into_bytes();
        let length = bytes.len();
        let previous = delivery.bytes.fetch_add(length, Ordering::Relaxed);
        if length <= BUDGET && previous + length <= BUDGET && delivery.queue.try_send(bytes).is_ok()
        {
            return Ok(());
        }
        delivery.bytes.fetch_sub(length, Ordering::Relaxed);
        let _ = delivery.socket.shutdown(Shutdown::Both);
        *current = None;
        // Delivery failure detaches the consumer; transcript persistence remains authoritative.
        Ok(())
    }
}
impl DeliveryHub {
    fn detach(&self) {
        if let Some(delivery) = lock(&self.0).take() {
            let _ = delivery.socket.shutdown(Shutdown::Both);
        }
    }
    fn finish(&self) {
        lock(&self.0).take();
    }
}
enum Input {
    Request(u64, String),
    Closed(u64),
}
struct Connection {
    id: u64,
    socket: UnixStream,
    reader: thread::JoinHandle<()>,
    writer: thread::JoinHandle<()>,
}
impl Connection {
    fn close(self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        let _ = self.reader.join();
        let _ = self.writer.join();
    }
}
struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn path(root: &Path) -> PathBuf {
    root.join("resident.sock")
}

pub fn serve(
    root: &Path,
    config: &Configuration,
    compose: impl FnOnce(Arc<dyn RuntimeEvents>) -> Result<Host, String>,
) -> Result<(), String> {
    let hub = Arc::new(DeliveryHub::default());
    // Composition acquires the root lease before any socket is replaced.
    let mut host = compose(hub.clone())?;
    let address = path(root);
    if address.exists() {
        if !fs::symlink_metadata(&address)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_socket()
        {
            return Err("resident endpoint is not a socket".into());
        }
        fs::remove_file(&address).map_err(|e| e.to_string())?;
    }
    let listener = UnixListener::bind(&address).map_err(|e| {
        format!("resident socket: {e}; use a shorter runtime root if its path is too long")
    })?;
    let _endpoint = SocketPath(address.clone());
    fs::set_permissions(&address, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (send, receive) = mpsc::sync_channel(2);
    let mut active: Option<Connection> = None;
    let mut serial = 0;
    loop {
        host.poll_operations();
        host.poll_launches()?;
        match listener.accept() {
            Ok((mut socket, _)) => {
                // Accepted sockets inherit nonblocking mode on some Unix hosts.
                // Attachment and worker reads use blocking I/O with explicit deadlines.
                socket.set_nonblocking(false).map_err(|e| e.to_string())?;
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .map_err(|e| e.to_string())?;
                if active.is_some() {
                    let _ = writeln!(
                        socket,
                        "{}",
                        json!({"v":1,"lifecycle":"rejected","error":"runtime already has an attached client"})
                    );
                    continue;
                }
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|e| e.to_string())?;
                let mut reader = BufReader::new(socket.try_clone().map_err(|e| e.to_string())?);
                let attachment = host::read_request(&mut reader)
                    .ok()
                    .flatten()
                    .and_then(|line| serde_json::from_str::<Configuration>(&line).ok());
                if attachment.as_ref() != Some(config) {
                    let _ = writeln!(
                        socket,
                        "{}",
                        json!({"v":1,"lifecycle":"rejected","error":"resident launch configuration does not match; shut down the existing runtime before changing it"})
                    );
                    continue;
                }
                socket.set_read_timeout(None).map_err(|e| e.to_string())?;
                serial += 1;
                let id = serial;
                let (queue, frames) = mpsc::sync_channel::<Vec<u8>>(64);
                let bytes = Arc::new(AtomicUsize::new(0));
                let queued = bytes.clone();
                let mut output = socket.try_clone().map_err(|e| e.to_string())?;
                let writer = thread::spawn(move || {
                    while let Ok(frame) = frames.recv() {
                        queued.fetch_sub(frame.len(), Ordering::Relaxed);
                        if output.write_all(&frame).is_err() {
                            break;
                        }
                    }
                    let _ = output.shutdown(Shutdown::Both);
                });
                // Queue the handshake before workers can deliver live events.
                {
                    let mut delivery = lock(&hub.0);
                    let hello = format!(
                        "{}\n",
                        resident_ready(&[
                            "terminal.v1",
                            "resident.v1",
                            "workspaces.v1",
                            "worktrees.v1",
                            "application.v1",
                            "application.operations.v1",
                            "application.initialization.v1",
                            "retire.v1"
                        ])
                    )
                    .into_bytes();
                    bytes.fetch_add(hello.len(), Ordering::Relaxed);
                    queue.send(hello).map_err(|e| e.to_string())?;
                    *delivery = Some(Delivery {
                        queue,
                        bytes,
                        socket: socket.try_clone().map_err(|e| e.to_string())?,
                    });
                }
                host.attached(true);
                let commands = send.clone();
                let reader = thread::spawn(move || {
                    while let Ok(Some(line)) = host::read_request(&mut reader) {
                        if commands.send(Input::Request(id, line)).is_err() {
                            return;
                        }
                    }
                    let _ = commands.send(Input::Closed(id));
                });
                active = Some(Connection {
                    id,
                    socket,
                    reader,
                    writer,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        match receive.recv_timeout(Duration::from_millis(10)) {
            Ok(Input::Request(id, line)) if active.as_ref().is_some_and(|a| a.id == id) => {
                let (reply, shutdown) = host.request(&line);
                hub.publish(reply)?;
                if shutdown {
                    hub.finish();
                    // The writer drains the shutdown reply before closing the stream.
                    if let Some(connection) = active.take() {
                        let _ = connection.writer.join();
                        let _ = connection.socket.shutdown(Shutdown::Both);
                        drop(receive);
                        let _ = connection.reader.join();
                    }
                    return Ok(());
                }
            }
            Ok(Input::Closed(id)) if active.as_ref().is_some_and(|a| a.id == id) => {
                host.attached(false);
                hub.detach();
                active.take().unwrap().close();
            }
            _ => {}
        }
    }
}

fn resident_ready(capabilities: &[&str]) -> Value {
    let mut ready = host::ready(capabilities);
    // Bundled runtimes have immutable, content-addressed executable paths.
    if let Ok(executable) = std::env::current_exe() {
        ready["executable"] = json!(executable);
    }
    ready
}

fn connect(root: &Path) -> Result<UnixStream, String> {
    let stream = UnixStream::connect(path(root)).map_err(|e| e.to_string())?;
    let metadata = fs::metadata(root).map_err(|e| e.to_string())?;
    // SAFETY: geteuid only reads the current process credential.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err("resident root must be private and owned by the current user".into());
    }
    Ok(stream)
}
pub fn proxy(root: &Path, config: &Configuration) -> Result<(), String> {
    proxy_with_catalog(root, config, &CatalogMode::Workspace)
}

pub fn proxy_with_catalog(
    root: &Path,
    config: &Configuration,
    catalog: &CatalogMode,
) -> Result<(), String> {
    let stream = match connect(root) {
        Ok(stream) => stream,
        Err(_) => {
            let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
            command
                .arg("--transport")
                .arg("serve")
                .arg("--root")
                .arg(root)
                .arg("--workdir")
                .arg(&config.workdir)
                .arg("--codex")
                .arg(&config.codex)
                .arg("--model")
                .arg(&config.model)
                .arg("--catalog")
                .arg(catalog.argument())
                .arg("--shell")
                .arg(&config.shell)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            // SAFETY: setsid is async-signal-safe and runs before exec, without allocation or locks.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = command.spawn().map_err(|e| e.to_string())?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut diagnostic = String::new();
            loop {
                if let Ok(stream) = connect(root) {
                    // Reap a daemon that shuts down before its proxy disconnects.
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                    break stream;
                }
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    if let Some(stderr) = child.stderr.take() {
                        let _ = stderr.take(4096).read_to_string(&mut diagnostic);
                    }
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "resident runtime did not become available: {diagnostic}"
                    ));
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    };
    let mut input = stream.try_clone().map_err(|e| e.to_string())?;
    writeln!(
        input,
        "{}",
        serde_json::to_string(config).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    // The proxy owns only the attachment. EOF never becomes a shutdown request.
    thread::spawn(move || {
        let _ = forward(&mut io::stdin().lock(), &mut input);
        let _ = input.shutdown(Shutdown::Write);
    });
    forward(&mut &stream, &mut io::stdout().lock()).map_err(|e| e.to_string())?;
    Ok(())
}

// Explicit writes avoid kernel splice holding a pipe while waiting for more socket data on WSL.
fn forward(input: &mut impl Read, output: &mut impl Write) -> io::Result<()> {
    let mut bytes = [0; 8192];
    loop {
        let n = input.read(&mut bytes)?;
        if n == 0 {
            return Ok(());
        }
        output.write_all(&bytes[..n])?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_saturated_consumer_is_detached_without_failing_execution_delivery() {
        let (socket, mut peer) = UnixStream::pair().unwrap();
        let (queue, _receiver) = mpsc::sync_channel(0);
        let hub = DeliveryHub(Mutex::new(Some(Delivery {
            queue,
            bytes: Arc::new(AtomicUsize::new(0)),
            socket,
        })));
        hub.publish(json!({"v":1,"event":{"type":"user.message"}}))
            .unwrap();
        assert!(lock(&hub.0).is_none());
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
        hub.publish(json!({"v":1,"event":{"type":"turn.completed"}}))
            .unwrap();
    }
}
