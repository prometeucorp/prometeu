//! Codex on a remote machine (ADR 0105). The desktop's native Codex engine (a private
//! `codex app-server` that owns the TUI's thread) runs on the machine as a host terminal, and the
//! TUI attaches to it there. The desktop still reads the engine's RPC stream, through a relay that
//! also runs as a host terminal: it owns the WebSocket connection to the engine's Unix socket and
//! speaks newline-delimited JSON on a raw PTY. Riding a terminal gives the stream the host's
//! sequence numbers, so a desktop that loses SSH reads exactly the frames it missed.
use std::path::Path;
use std::time::Duration;

/// The relay prints this line once the engine's socket passed its checks and the WebSocket is
/// open. Anything before it (a shell's noise) is not part of the stream.
pub const RELAY_READY: &str = r#"{"prometeu":"codex-relay","ready":1}"#;

/// The largest message either side relays: native user items carry inline images, as in the
/// desktop's own engine client.
pub const MESSAGE_LIMIT: usize = 64 * 1_048_576;

/// The longest a relay waits for the engine's socket to appear and answer.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// The terminal that runs a conversation's engine; the conversation's own ID runs its TUI.
pub fn engine_terminal(session: &str) -> String {
    format!("{session}-codex")
}

/// One desktop incarnation's relay to a conversation's engine. Each computer that follows the
/// conversation has its own (ADR 0104), so the ID carries the incarnation.
pub fn relay_terminal(session: &str, incarnation: &str) -> String {
    let short: String = incarnation
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect();
    format!("{session}-rpc-{short}")
}

/// The conversation an engine or relay terminal belongs to, or `None` for any other terminal.
pub fn companion_of(id: &str) -> Option<&str> {
    if let Some(session) = id.strip_suffix("-codex") {
        return (!session.is_empty()).then_some(session);
    }
    let (session, incarnation) = id.rsplit_once("-rpc-")?;
    (!session.is_empty()
        && !incarnation.is_empty()
        && incarnation.chars().all(|ch| ch.is_ascii_alphanumeric()))
    .then_some(session)
}

/// The engine's listening address under the conversation's private session directory, which
/// `terminal_forget` removes with the conversation. Codex replaces it with an alias to its own
/// private endpoint.
pub fn socket_path(root: &str, session: &str) -> String {
    format!("{root}/sessions/{session}/rpc")
}

/// Relay stdio to the engine listening at `socket`, whose server process is `pid` or one of its
/// descendants (an npm install runs the native server under a Node launcher). Returns when either
/// side closes.
#[cfg(target_os = "linux")]
pub fn relay(socket: &Path, pid: u32, timeout: Duration) -> Result<(), String> {
    linux::relay(socket, pid, timeout)
}

#[cfg(not(target_os = "linux"))]
pub fn relay(_socket: &Path, _pid: u32, _timeout: Duration) -> Result<(), String> {
    Err("the Codex relay runs only on Linux".into())
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{MESSAGE_LIMIT, RELAY_READY};
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use tungstenite::{protocol::WebSocketConfig, Message, WebSocket};

    /// Outgoing frames are requests and replies from the desktop, never content.
    const OUTGOING_LIMIT: usize = 1_048_576;

    pub fn relay(socket: &Path, pid: u32, timeout: Duration) -> Result<(), String> {
        raw_stdin();
        let deadline = Instant::now() + timeout;
        let stream = connect(socket, pid, deadline)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
            .map_err(|e| e.to_string())?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MESSAGE_LIMIT))
            .max_frame_size(Some(MESSAGE_LIMIT))
            .write_buffer_size(0)
            .max_write_buffer_size(OUTGOING_LIMIT + 1024);
        let (mut socket, _) =
            tungstenite::client::client_with_config("ws://localhost/", stream, Some(config))
                .map_err(|e| format!("websocket handshake: {e}"))?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(|e| e.to_string())?;
        let mut out = io::stdout().lock();
        writeln!(out, "{RELAY_READY}").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        let lines = stdin_lines();
        serve(&mut socket, &lines, &mut out)
    }

    fn serve(
        socket: &mut WebSocket<UnixStream>,
        lines: &mpsc::Receiver<Vec<u8>>,
        out: &mut impl Write,
    ) -> Result<(), String> {
        loop {
            match socket.read() {
                Ok(Message::Text(text)) => {
                    // Compact JSON never holds a raw newline; anything else is re-serialized so
                    // one message stays one line.
                    if text.contains('\n') {
                        let value: serde_json::Value =
                            serde_json::from_str(&text).map_err(|e| e.to_string())?;
                        writeln!(out, "{value}")
                    } else {
                        writeln!(out, "{text}")
                    }
                    .and_then(|_| out.flush())
                    .map_err(|e| e.to_string())?;
                }
                Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                    socket.flush().map_err(|e| e.to_string())?;
                }
                Ok(Message::Binary(_)) => return Err("unexpected binary message".into()),
                Ok(Message::Close(_)) => return Ok(()),
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            }
            loop {
                match lines.try_recv() {
                    Ok(line) => {
                        // Input interrupted by a lost connection can leave a partial line. It is
                        // dropped instead of reaching the engine; its request simply goes
                        // unanswered, as a lost write does anywhere else.
                        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) else {
                            continue;
                        };
                        if !value.is_object() {
                            continue;
                        }
                        socket
                            .send(Message::Text(value.to_string().into()))
                            .map_err(|e| e.to_string())?;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        let _ = socket.close(None);
                        let _ = socket.flush();
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Lines from stdin, without their terminator. Overlong lines are dropped whole.
    fn stdin_lines() -> mpsc::Receiver<Vec<u8>> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut input = BufReader::new(io::stdin().lock());
            let mut line = Vec::new();
            loop {
                line.clear();
                let read = (&mut input)
                    .take(OUTGOING_LIMIT as u64 + 1)
                    .read_until(b'\n', &mut line);
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.last() != Some(&b'\n') => {
                        if line.len() <= OUTGOING_LIMIT {
                            // End of input in the middle of a line.
                            break;
                        }
                        // Skip the rest of an overlong line.
                        let mut rest = Vec::new();
                        if !matches!(input.read_until(b'\n', &mut rest), Ok(n) if n > 0) {
                            break;
                        }
                    }
                    Ok(_) => {
                        let text = line.trim_ascii();
                        if !text.is_empty() && sender.send(text.to_vec()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        receiver
    }

    /// The desktop writes JSON through a PTY. Raw mode keeps the line discipline from echoing,
    /// translating or interpreting any byte.
    fn raw_stdin() {
        unsafe {
            if libc::isatty(0) != 1 {
                return;
            }
            let mut attributes: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut attributes) != 0 {
                return;
            }
            libc::cfmakeraw(&mut attributes);
            libc::tcsetattr(0, libc::TCSANOW, &attributes);
        }
    }

    fn connect(socket: &Path, pid: u32, deadline: Instant) -> Result<UnixStream, String> {
        loop {
            if Instant::now() >= deadline {
                return Err("the Codex engine did not open its socket in time".into());
            }
            match endpoint(socket) {
                Ok(target) => match UnixStream::connect(&target) {
                    Ok(stream) => {
                        verify_peer(&stream, pid)?;
                        return Ok(stream);
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                        ) => {}
                    Err(error) => return Err(error.to_string()),
                },
                Err(Missing) => {}
                Err(Rejected(reason)) => return Err(reason),
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    enum EndpointError {
        Missing,
        Rejected(String),
    }
    use EndpointError::{Missing, Rejected};

    /// Codex advertises an alias to a SHA-256 named endpoint in its private socket directory.
    /// Validate that exact mapping and the ownership and modes of both, as the desktop's own
    /// engine client does (`terminal_agent_codex/wire.rs`).
    fn endpoint(path: &Path) -> Result<PathBuf, EndpointError> {
        use sha2::{Digest, Sha256};
        use std::os::unix::ffi::OsStrExt;
        let uid = unsafe { libc::geteuid() };
        let alias = std::fs::symlink_metadata(path).map_err(|_| Missing)?;
        if alias.uid() != uid {
            return Err(Rejected("the engine socket belongs to another user".into()));
        }
        let target = if alias.file_type().is_symlink() {
            let target = std::fs::read_link(path).map_err(|_| Missing)?;
            let parent = path
                .parent()
                .and_then(|parent| std::fs::canonicalize(parent).ok())
                .ok_or(Missing)?;
            let name = path.file_name().ok_or(Missing)?;
            let hash = format!(
                "{:x}",
                Sha256::digest(parent.join(name).as_os_str().as_bytes())
            );
            if target.file_name().and_then(|name| name.to_str()) != Some(hash.as_str()) {
                return Err(Rejected("the engine socket alias is not Codex's".into()));
            }
            target
        } else {
            path.to_owned()
        };
        let directory = target.parent().ok_or(Missing)?;
        let directory = std::fs::symlink_metadata(directory).map_err(|_| Missing)?;
        let socket = std::fs::symlink_metadata(&target).map_err(|_| Missing)?;
        if !directory.is_dir()
            || directory.uid() != uid
            || directory.mode() & 0o777 != 0o700
            || !socket.file_type().is_socket()
            || socket.uid() != uid
            || socket.mode() & 0o077 != 0
        {
            return Err(Rejected("the engine socket is not private".into()));
        }
        Ok(target)
    }

    fn verify_peer(stream: &UnixStream, expected: u32) -> Result<(), String> {
        use std::os::fd::AsRawFd;
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
            return Err("the engine socket is served by another user".into());
        }
        if !descends_from(peer.pid as u32, expected) {
            return Err("the engine socket is served by another process".into());
        }
        Ok(())
    }

    /// Whether `pid` is `ancestor` or runs below it.
    pub(super) fn descends_from(mut pid: u32, ancestor: u32) -> bool {
        for _ in 0..16 {
            if pid == ancestor {
                return true;
            }
            match parent(pid) {
                Some(next) if next > 1 && next != pid => pid = next,
                _ => return false,
            }
        }
        false
    }

    fn parent(pid: u32) -> Option<u32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name is parenthesized and may hold spaces and parentheses itself.
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_whitespace().nth(1)?.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn companion_terminals_name_their_conversation() {
        let session = "0f8e3b7c-1a2b-4c3d-9e8f-001122334455";
        assert_eq!(companion_of(&engine_terminal(session)), Some(session));
        let relay = relay_terminal(session, "6c1d0e4b-aaaa-bbbb-cccc-ddddeeeeffff");
        assert_eq!(relay, format!("{session}-rpc-6c1d0e4baaaa"));
        assert!(crate::valid_id(&relay));
        assert_eq!(companion_of(&relay), Some(session));
        assert_eq!(companion_of(session), None);
        assert_eq!(companion_of("-codex"), None);
        assert_eq!(companion_of("x-rpc-"), None);
        assert_eq!(companion_of("x-rpc-a.b"), None);
    }

    #[test]
    fn the_socket_lives_in_the_session_directory() {
        assert_eq!(
            socket_path("/home/me/.prometeu-remote", "s"),
            "/home/me/.prometeu-remote/sessions/s/rpc"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_process_descends_from_its_parents_only() {
        let me = std::process::id();
        let parent = std::os::unix::process::parent_id();
        assert!(linux::descends_from(me, me));
        assert!(linux::descends_from(me, parent));
        assert!(!linux::descends_from(parent, me));
    }
}
