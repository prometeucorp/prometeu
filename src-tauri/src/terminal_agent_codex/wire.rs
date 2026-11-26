use super::{QUEUE_BYTES, QUEUE_LIMIT};
use serde_json::Value;
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};
use tungstenite::{protocol::WebSocketConfig, Message, WebSocket};

const MAX_MESSAGE: usize = 1024 * 1024;
// Native user items include inline image data, including during paginated replay.
// Match the history byte ceiling rather than the small outbound control limit.
const MAX_INCOMING_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) enum Error {
    /// A remote engine could not start, with the localized reason to show.
    Remote(String),
    Io,
    Protocol,
    Identity,
    SocketIdentity,
    Conflict,
    Closed,
    Limit,
    Timeout,
}
impl Error {
    pub fn message(&self) -> String {
        match self {
            Self::Remote(message) => message.clone(),
            _ => super::failure(),
        }
    }
}
impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Self::Io
    }
}

/// The engine's message stream: its own WebSocket on this computer, or the newline-delimited
/// relay of one that runs on a remote machine (ADR 0105).
enum Link {
    Socket(Box<WebSocket<UnixStream>>),
    Lines {
        frames: mpsc::Receiver<Value>,
        writer: Box<dyn Write + Send>,
    },
}

pub(super) struct Client {
    socket: Link,
    serial: u64,
    queued: VecDeque<Value>,
    queued_bytes: usize,
}
impl Client {
    pub fn connect(
        path: &Path,
        pid: u32,
        deadline: Instant,
        mut exited: impl FnMut() -> Result<bool, Error>,
    ) -> Result<Self, Error> {
        let stream = loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            if exited()? {
                return Err(Error::Closed);
            }
            match endpoint(path) {
                Ok(endpoint) => match UnixStream::connect(endpoint) {
                    Ok(stream) => break stream,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                        ) => {}
                    Err(error) => return Err(error.into()),
                },
                Err(Error::Io) if !path.exists() => {}
                Err(error) => return Err(error),
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        verify_peer(&stream, pid)?;
        stream.set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_INCOMING_MESSAGE))
            .max_frame_size(Some(MAX_INCOMING_MESSAGE))
            .write_buffer_size(0)
            .max_write_buffer_size(MAX_MESSAGE + 1024);
        let (socket, _) =
            tungstenite::client::client_with_config("ws://localhost/", stream, Some(config))
                .map_err(|_| Error::Protocol)?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(20)))?;
        Ok(Self {
            socket: Link::Socket(Box::new(socket)),
            serial: 0,
            queued: VecDeque::new(),
            queued_bytes: 0,
        })
    }
    /// A remote engine's relay: one JSON object per line once the relay announced itself. The
    /// relay checked the engine's socket and peer on the machine; the remote host's terminal
    /// carries the lines in order and without gaps, or ends the stream.
    pub fn lines(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        deadline: Instant,
    ) -> Result<Self, Error> {
        // The engine worker drains this hop at once into its own byte-budgeted queue; a short
        // bound keeps large native items from piling up here.
        let (sender, frames) = mpsc::sync_channel(4);
        let (ready, announced) = mpsc::channel();
        std::thread::Builder::new()
            .name("codex-remote-engine".into())
            .spawn(move || read_lines(reader, ready, sender))?;
        match announced.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(Error::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Error::Closed),
        }
        Ok(Self {
            socket: Link::Lines { frames, writer },
            serial: 0,
            queued: VecDeque::new(),
            queued_bytes: 0,
        })
    }
    pub fn send(&mut self, value: Value) -> Result<(), Error> {
        let text = value.to_string();
        if text.len() > MAX_MESSAGE {
            return Err(Error::Limit);
        }
        match &mut self.socket {
            Link::Socket(socket) => socket
                .send(Message::Text(text.into()))
                .map_err(|_| Error::Closed),
            Link::Lines { writer, .. } => {
                let mut line = text.into_bytes();
                line.push(b'\n');
                writer
                    .write_all(&line)
                    .and_then(|_| writer.flush())
                    .map_err(|_| Error::Closed)
            }
        }
    }
    fn receive(&mut self) -> Result<Option<Value>, Error> {
        let socket = match &mut self.socket {
            Link::Socket(socket) => socket,
            Link::Lines { frames, .. } => {
                return match frames.recv_timeout(Duration::from_millis(20)) {
                    Ok(value) => Ok(Some(value)),
                    Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                    Err(mpsc::RecvTimeoutError::Disconnected) => Err(Error::Closed),
                }
            }
        };
        match socket.read() {
            Ok(Message::Text(text)) => {
                let value: Value = serde_json::from_str(&text).map_err(|_| Error::Protocol)?;
                if !value.is_object() {
                    return Err(Error::Protocol);
                }
                Ok(Some(value))
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => {
                socket.flush().map_err(|_| Error::Closed)?;
                Ok(None)
            }
            Ok(_) => Err(Error::Closed),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(tungstenite::Error::Capacity(_)) => Err(Error::Limit),
            Err(_) => Err(Error::Closed),
        }
    }
    pub fn next(&mut self) -> Result<Option<Value>, Error> {
        if let Some(value) = self.queued.pop_front() {
            self.queued_bytes -= value.to_string().len();
            Ok(Some(value))
        } else {
            self.receive()
        }
    }
    fn retain(&mut self, value: Value) -> Result<(), Error> {
        self.queued_bytes += value.to_string().len();
        if self.queued.len() == QUEUE_LIMIT || self.queued_bytes > QUEUE_BYTES {
            return Err(Error::Limit);
        }
        self.queued.push_back(value);
        Ok(())
    }
    pub fn call(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, Error> {
        self.serial += 1;
        let id = format!("prometeu-{}", self.serial);
        self.send(serde_json::json!({"id":id,"method":method,"params":params}))?;
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let Some(value) = self.receive()? else {
                continue;
            };
            if value.get("method").is_none() && value["id"] == id {
                if value.get("error").is_some() || value.get("result").is_none() {
                    return Err(Error::Protocol);
                }
                return Ok(value["result"].clone());
            }
            self.retain(value)?;
        }
    }
    pub fn verify_settings(
        &self,
        thread: &str,
        expected: &Value,
        reply: &Value,
    ) -> Result<(), Error> {
        if reply["thread"]["id"] != thread {
            return Err(Error::Identity);
        }
        let actual = serde_json::json!({"approvalPolicy":reply["approvalPolicy"],
            "sandboxPolicy":reply["sandbox"],"effort":reply["reasoningEffort"],
            "collaborationMode":reply["collaborationMode"]});
        if !same_settings(&actual, expected) {
            return Err(Error::Conflict);
        }
        Ok(())
    }
}

/// Split a relay's output into JSON objects. Lines before the relay's announcement are not part of
/// the stream. A malformed or oversized line ends it: a frame was lost, so the canonical stream
/// can no longer be trusted, and the receiver sees the closure.
fn read_lines(
    reader: Box<dyn Read + Send>,
    ready: mpsc::Sender<()>,
    frames: mpsc::SyncSender<Value>,
) {
    use std::io::BufRead;
    let mut reader = io::BufReader::new(reader);
    let mut line = Vec::new();
    let mut announced = false;
    loop {
        line.clear();
        match (&mut reader)
            .take(MAX_INCOMING_MESSAGE as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => return,
            Ok(_) if line.last() != Some(&b'\n') => return,
            Ok(_) => {}
        }
        let text = line.trim_ascii();
        if !announced {
            if text == prometeu_host::codex::RELAY_READY.as_bytes() {
                announced = true;
                if ready.send(()).is_err() {
                    return;
                }
            }
            continue;
        }
        if text.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(text) else {
            return;
        };
        if !value.is_object() || frames.send(value).is_err() {
            return;
        }
    }
}

/// Codex 0.169 advertises an alias to a SHA-256 named endpoint in its private socket directory.
/// Validate that exact mapping; following an arbitrary advertised symlink is not sufficient.
fn endpoint(path: &Path) -> Result<PathBuf, Error> {
    use sha2::{Digest, Sha256};
    use std::os::unix::ffi::OsStrExt;
    let alias = std::fs::symlink_metadata(path)?;
    if alias.uid() != unsafe { libc::geteuid() } {
        return Err(Error::SocketIdentity);
    }
    let target = if alias.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        let parent = std::fs::canonicalize(path.parent().ok_or(Error::SocketIdentity)?)?;
        let hash = format!(
            "{:x}",
            Sha256::digest(
                parent
                    .join(path.file_name().ok_or(Error::SocketIdentity)?)
                    .as_os_str()
                    .as_bytes()
            )
        );
        if target.file_name().and_then(|name| name.to_str()) != Some(hash.as_str()) {
            return Err(Error::SocketIdentity);
        }
        target
    } else {
        path.to_owned()
    };
    let parent = std::fs::symlink_metadata(target.parent().ok_or(Error::SocketIdentity)?)?;
    let socket = std::fs::symlink_metadata(&target)?;
    let uid = unsafe { libc::geteuid() };
    if !parent.is_dir()
        || parent.uid() != uid
        || parent.mode() & 0o777 != 0o700
        || !socket.file_type().is_socket()
        || socket.uid() != uid
        || socket.mode() & 0o077 != 0
    {
        return Err(Error::SocketIdentity);
    }
    Ok(target)
}

fn verify_peer(stream: &UnixStream, expected: u32) -> Result<(), Error> {
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "macos")]
    let actual = {
        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of_val(&pid) as libc::socklen_t;
        // The kernel writes only the initialized pid-sized buffer specified by length.
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                0,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut length,
            )
        };
        if status != 0 || length as usize != std::mem::size_of_val(&pid) {
            return Err(Error::SocketIdentity);
        }
        pid as u32
    };
    #[cfg(target_os = "linux")]
    let actual = {
        let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of_val(&peer) as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut length,
            )
        };
        if status != 0 || peer.uid != unsafe { libc::geteuid() } {
            return Err(Error::SocketIdentity);
        }
        peer.pid as u32
    };
    if actual != expected {
        return Err(Error::SocketIdentity);
    }
    Ok(())
}

pub(super) fn remove_owned_endpoint(alias: &Path) {
    // Called only after the owned child has been reaped. Preserve other entries in Codex's
    // shared private directory, including any unrelated native daemon's socket or lock.
    if let Ok(target) = endpoint(alias) {
        if target != alias {
            let _ = std::fs::remove_file(&target);
            let lock = target.with_extension("lock");
            if std::fs::symlink_metadata(&lock)
                .is_ok_and(|entry| entry.is_file() && entry.uid() == unsafe { libc::geteuid() })
            {
                let _ = std::fs::remove_file(lock);
            }
        }
    }
}
fn same_settings(actual: &Value, expected: &Value) -> bool {
    ["approvalPolicy", "sandboxPolicy", "effort"]
        .iter()
        .all(|key| actual[key] == expected[key])
        && actual["collaborationMode"]["mode"] == expected["collaborationMode"]["mode"]
        && ["model", "reasoning_effort"].iter().all(|key| {
            actual["collaborationMode"]["settings"][key]
                == expected["collaborationMode"]["settings"][key]
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;

    /// A relay as the remote host delivers it: bytes in, request lines out.
    fn relay() -> (
        UnixStream,
        BufReader<UnixStream>,
        Box<dyn Read + Send>,
        Box<dyn Write + Send>,
    ) {
        let (output, reader) = UnixStream::pair().unwrap();
        let (writer, requests) = UnixStream::pair().unwrap();
        (
            output,
            BufReader::new(requests),
            Box::new(reader),
            Box::new(writer),
        )
    }

    fn next(client: &mut Client) -> Result<Value, Error> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "no frame");
            if let Some(value) = client.next()? {
                return Ok(value);
            }
        }
    }

    #[test]
    fn a_remote_relay_is_read_after_its_announcement() {
        let (mut output, mut requests, reader, writer) = relay();
        write!(
            output,
            "shell noise\r\n{}\n{}\n",
            prometeu_host::codex::RELAY_READY,
            serde_json::json!({"method": "hello"})
        )
        .unwrap();
        let mut client =
            Client::lines(reader, writer, Instant::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(next(&mut client).unwrap()["method"], "hello");
        let answer = std::thread::spawn(move || {
            let mut line = String::new();
            requests.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "initialize");
            writeln!(output, "{}", serde_json::json!({"method": "early"})).unwrap();
            writeln!(
                output,
                "{}",
                serde_json::json!({"id": request["id"], "result": {"ok": true}})
            )
            .unwrap();
            output
        });
        let result = client
            .call(
                "initialize",
                serde_json::json!({}),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(result, serde_json::json!({"ok": true}));
        // A notification that arrived before the reply is kept in order.
        assert_eq!(next(&mut client).unwrap()["method"], "early");
        let mut output = answer.join().unwrap();
        // A broken line means a lost frame: the stream ends instead of skipping it.
        writeln!(output, "{{\"method\":").unwrap();
        writeln!(output, "{}", serde_json::json!({"method": "after"})).unwrap();
        assert!(matches!(next(&mut client), Err(Error::Closed)));
    }

    #[test]
    fn a_relay_that_never_announces_itself_times_out() {
        let (mut output, _requests, reader, writer) = relay();
        writeln!(output, "{}", serde_json::json!({"method": "hello"})).unwrap();
        let started = Instant::now();
        assert!(matches!(
            Client::lines(reader, writer, started + Duration::from_millis(200)),
            Err(Error::Timeout)
        ));
        let (output, _requests, reader, writer) = relay();
        drop(output);
        assert!(matches!(
            Client::lines(reader, writer, Instant::now() + Duration::from_secs(5)),
            Err(Error::Closed)
        ));
    }
}
