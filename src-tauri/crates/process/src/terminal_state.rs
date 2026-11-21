//! Ordered private terminal emulation helper. The host supplies a packaged executable or an
//! explicit development command; this adapter never discovers Node or owns the provider's PTY.
pub mod protocol;

use crate::command::{nonblocking, OwnedChild};
use base64::{engine::general_purpose::STANDARD, Engine};
use protocol::{Ack, Operation, Request, Response, WriteAck};
pub use protocol::{Key, Rejection, Size, Snapshot};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Io,
    Closed,
    Timeout,
    Limit,
    InvalidResponse,
    Sequence,
    Rejected(Rejection),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "terminal state helper: {self:?}")
    }
}

impl std::error::Error for Error {}

/// One owner serializes requests, signals and reaping. Share it behind the host's worker queue,
/// never while holding the PTY output/publication lock or a Tauri/UI thread.
pub struct Client {
    owner: Option<OwnedChild>,
    input: Option<ChildStdin>,
    output: ChildStdout,
    next_id: u64,
    timeout: Duration,
}

impl Client {
    pub fn launch(mut command: Command, timeout: Duration) -> Result<Self, Error> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .process_group(0);
        let child = command.spawn().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::Unavailable
            } else {
                Error::Io
            }
        })?;
        let mut owner = OwnedChild::new(child);
        let input = owner.child.stdin.take().ok_or(Error::Io)?;
        let output = owner.child.stdout.take().ok_or(Error::Io)?;
        nonblocking(&input).map_err(|_| Error::Io)?;
        nonblocking(&output).map_err(|_| Error::Io)?;
        Ok(Self {
            owner: Some(owner),
            input: Some(input),
            output,
            next_id: 0,
            timeout,
        })
    }

    pub fn system_id(&self) -> Option<u32> {
        self.owner.as_ref().map(|owner| owner.child.id())
    }

    fn exchange(&mut self, request: &Request<'_>) -> Result<serde_json::Value, Error> {
        let result = self.exchange_inner(request);
        if matches!(
            result,
            Err(Error::Io | Error::Closed | Error::Timeout | Error::InvalidResponse)
                | Err(Error::Rejected(Rejection::Internal))
        ) {
            self.retire();
        }
        result
    }

    fn exchange_inner(&mut self, request: &Request<'_>) -> Result<serde_json::Value, Error> {
        if self.owner.is_none() {
            return Err(Error::Closed);
        }
        let mut outgoing = serde_json::to_vec(request).map_err(|_| Error::Io)?;
        if outgoing.len() >= protocol::MAX_REQUEST_BYTES {
            return Err(Error::Limit);
        }
        outgoing.push(b'\n');
        let deadline = Instant::now() + self.timeout;
        let mut sent = 0;
        let mut incoming = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            let mut progress = false;
            if sent < outgoing.len() {
                match self
                    .input
                    .as_mut()
                    .ok_or(Error::Closed)?
                    .write(&outgoing[sent..])
                {
                    Ok(0) => return Err(Error::Closed),
                    Ok(size) => {
                        sent += size;
                        progress = true;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => return Err(Error::Io),
                }
            }
            let mut bytes = [0; 8192];
            match self.output.read(&mut bytes) {
                Ok(0) => return Err(Error::Closed),
                Ok(size) => {
                    progress = true;
                    if size > protocol::MAX_RESPONSE_BYTES.saturating_sub(incoming.len()) {
                        return Err(Error::InvalidResponse);
                    }
                    let previous = incoming.len();
                    incoming.extend_from_slice(&bytes[..size]);
                    if let Some(relative) = bytes[..size].iter().position(|byte| *byte == b'\n') {
                        let newline = previous + relative;
                        if sent != outgoing.len() || newline + 1 != incoming.len() {
                            return Err(Error::InvalidResponse);
                        }
                        let response: Response = serde_json::from_slice(&incoming[..newline])
                            .map_err(|_| Error::InvalidResponse)?;
                        if response.v != protocol::VERSION
                            || response.id.as_deref() != Some(&request.id)
                        {
                            return Err(Error::InvalidResponse);
                        }
                        return match (response.ok, response.result, response.error) {
                            (true, Some(result), None) => Ok(result),
                            (false, None, Some(error)) => Err(Error::Rejected(error.code)),
                            _ => Err(Error::InvalidResponse),
                        };
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(Error::Io),
            }
            if !progress {
                if self
                    .owner
                    .as_mut()
                    .ok_or(Error::Closed)?
                    .poll()
                    .map_err(|_| Error::Io)?
                    .is_some()
                {
                    return Err(Error::Closed);
                }
                self.wait(sent < outgoing.len(), deadline);
            }
        }
    }

    /// Sleep until the helper can take more input or has output. Every PTY chunk waits on this
    /// round trip, so a fixed sleep would add its whole length to terminal latency. The cap keeps
    /// checking for a helper that exited without closing its pipes.
    fn wait(&self, writing: bool, deadline: Instant) {
        let input = match (&self.input, writing) {
            (Some(input), true) => input.as_raw_fd(),
            _ => -1,
        };
        let mut fds = [
            libc::pollfd {
                fd: self.output.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: input,
                events: libc::POLLOUT,
                revents: 0,
            },
        ];
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(50))
            .as_millis() as libc::c_int;
        // SAFETY: both descriptors are owned by self for the call; poll ignores a negative fd.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    }

    fn request<T: serde::de::DeserializeOwned>(
        &mut self,
        key: &Key,
        operation: Operation<'_>,
    ) -> Result<T, Error> {
        self.next_id = self.next_id.checked_add(1).ok_or(Error::Limit)?;
        let request = Request {
            v: protocol::VERSION,
            id: format!("request-{}", self.next_id),
            key,
            operation,
        };
        let value = self.exchange(&request)?;
        serde_json::from_value(value).map_err(|_| {
            self.retire();
            Error::InvalidResponse
        })
    }

    pub fn create(&mut self, key: &Key, size: Size, scrollback: u16) -> Result<(), Error> {
        let ack: Ack = self.request(key, Operation::Create { size, scrollback })?;
        self.check(ack.seq == 0)
    }

    pub fn write(&mut self, key: &Key, seq: u64, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        if bytes.len() > protocol::MAX_WRITE_BYTES || seq > protocol::MAX_SAFE_SEQUENCE {
            return Err(Error::Limit);
        }
        let data = STANDARD.encode(bytes);
        let ack: WriteAck = self.request(key, Operation::Write { data: &data, seq })?;
        self.check(ack.seq == seq)?;
        ack.replies
            .iter()
            .map(|reply| {
                STANDARD.decode(reply).map_err(|_| {
                    self.retire();
                    Error::InvalidResponse
                })
            })
            .collect()
    }

    pub fn resize(&mut self, key: &Key, seq: u64, size: Size) -> Result<(), Error> {
        let ack: Ack = self.request(key, Operation::Resize { size, seq })?;
        self.check(ack.seq == seq)
    }

    pub fn snapshot(&mut self, key: &Key) -> Result<Snapshot, Error> {
        self.read_screen(key, false)
    }

    /// The snapshot without buffer ANSI or links: enough for readiness, not for restoring a view.
    pub fn view(&mut self, key: &Key) -> Result<Snapshot, Error> {
        self.read_screen(key, true)
    }

    fn read_screen(&mut self, key: &Key, view: bool) -> Result<Snapshot, Error> {
        let state: Snapshot = self.request(key, Operation::Snapshot { view })?;
        self.check(
            state.v == protocol::VERSION
                && state.session == key.session
                && state.generation == key.generation
                && state.seq <= protocol::MAX_SAFE_SEQUENCE
                && (2..=500).contains(&state.cols)
                && (1..=300).contains(&state.rows)
                && state.viewport_lines.len() == usize::from(state.rows)
                && valid_viewport_characters(
                    &state.viewport_lines,
                    &state.viewport_character_columns,
                    state.cols,
                    state.rows,
                )
                && valid_viewport_styles(&state)
                && [
                    &state.appearance.foreground,
                    &state.appearance.background,
                    &state.appearance.cursor,
                ]
                .into_iter()
                .all(|color| {
                    color.len() == 7
                        && color.starts_with('#')
                        && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
                }),
        )?;
        Ok(state)
    }

    pub fn dispose(&mut self, key: &Key) -> Result<u64, Error> {
        let ack: Ack = self.request(key, Operation::Dispose)?;
        Ok(ack.seq)
    }

    fn check(&mut self, valid: bool) -> Result<(), Error> {
        if valid {
            Ok(())
        } else {
            self.retire();
            Err(Error::InvalidResponse)
        }
    }

    fn retire(&mut self) {
        self.input.take();
        self.owner.take();
    }

    pub fn shutdown(mut self) -> Result<(), Error> {
        self.input.take();
        let deadline = Instant::now() + self.timeout;
        loop {
            let owner = self.owner.as_mut().ok_or(Error::Closed)?;
            if let Some(status) = owner.poll().map_err(|_| Error::Io)? {
                self.owner.take();
                return if status.success() {
                    Ok(())
                } else {
                    Err(Error::Closed)
                };
            }
            if Instant::now() >= deadline {
                self.retire();
                return Err(Error::Timeout);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn valid_viewport_characters(lines: &[String], columns: &[Vec<u16>], cols: u16, rows: u16) -> bool {
    let mut total = 0;
    columns.len() == usize::from(rows)
        && lines.len() == usize::from(rows)
        && lines.iter().zip(columns).all(|(text, columns)| {
            total += columns.len();
            total <= protocol::MAX_VIEWPORT_CHARACTERS
                && columns.len() == text.chars().count()
                && columns.iter().all(|column| *column < cols)
                && columns.windows(2).all(|pair| pair[0] <= pair[1])
        })
}

fn valid_viewport_styles(state: &Snapshot) -> bool {
    let color = |color: &protocol::TerminalColor| match color.mode {
        protocol::ColorMode::Default => color.value == -1,
        protocol::ColorMode::Palette => (0..=255).contains(&color.value),
        protocol::ColorMode::Rgb => (0..=0x00ff_ffff).contains(&color.value),
    };
    state.viewport_styles.len() == usize::from(state.rows)
        && state.viewport_styles.iter().all(|runs| {
            let mut end = 0;
            for run in runs {
                if run.start != end
                    || run.end <= run.start
                    || run.end > state.cols
                    || !color(&run.foreground)
                    || !color(&run.background)
                {
                    return false;
                }
                end = run.end;
            }
            end == state.cols
        })
}

/// Association at the last completely applied PTY chunk. A partial helper failure invalidates
/// this stream; it must not expose a snapshot that names an unapplied or partially applied chunk.
pub struct Stream {
    key: Key,
    bytes: u64,
    chunk: u64,
    valid: bool,
}

pub struct AppliedChunk {
    pub chunk_seq: u64,
    pub byte_seq: u64,
    pub replies: Vec<Vec<u8>>,
}

pub struct AppliedSnapshot {
    pub chunk_seq: u64,
    pub state: Snapshot,
}

impl Stream {
    pub fn create(
        client: &mut Client,
        key: Key,
        size: Size,
        scrollback: u16,
    ) -> Result<Self, Error> {
        client.create(&key, size, scrollback)?;
        Ok(Self {
            key,
            bytes: 0,
            chunk: 0,
            valid: true,
        })
    }

    pub fn key(&self) -> &Key {
        &self.key
    }

    pub fn write_chunk(
        &mut self,
        client: &mut Client,
        chunk_seq: u64,
        bytes: &[u8],
    ) -> Result<AppliedChunk, Error> {
        if !self.valid {
            return Err(Error::Closed);
        }
        if self.chunk.checked_add(1) != Some(chunk_seq) {
            return Err(Error::Sequence);
        }
        let end = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::Limit)?;
        if end > protocol::MAX_SAFE_SEQUENCE {
            return Err(Error::Limit);
        }
        let mut replies = Vec::new();
        if bytes.is_empty() {
            match client.write(&self.key, self.bytes, bytes) {
                Ok(values) => replies = values,
                Err(error) => {
                    self.valid = false;
                    return Err(error);
                }
            }
        }
        for part in bytes.chunks(protocol::MAX_WRITE_BYTES) {
            let next = self.bytes + part.len() as u64;
            match client.write(&self.key, next, part) {
                Ok(values) => {
                    self.bytes = next;
                    replies.extend(values);
                }
                Err(error) => {
                    self.valid = false;
                    return Err(error);
                }
            }
        }
        self.chunk = chunk_seq;
        Ok(AppliedChunk {
            chunk_seq,
            byte_seq: self.bytes,
            replies,
        })
    }

    pub fn resize(&mut self, client: &mut Client, size: Size) -> Result<(), Error> {
        if !self.valid {
            return Err(Error::Closed);
        }
        let result = client.resize(&self.key, self.bytes, size);
        if result.is_err() {
            self.valid = false;
        }
        result
    }

    pub fn snapshot(&mut self, client: &mut Client) -> Result<AppliedSnapshot, Error> {
        self.read_screen(client, false)
    }

    /// See [`Client::view`].
    pub fn view(&mut self, client: &mut Client) -> Result<AppliedSnapshot, Error> {
        self.read_screen(client, true)
    }

    fn read_screen(&mut self, client: &mut Client, view: bool) -> Result<AppliedSnapshot, Error> {
        if !self.valid {
            return Err(Error::Closed);
        }
        let state = client.read_screen(&self.key, view)?;
        if state.seq != self.bytes {
            self.valid = false;
            return Err(Error::Sequence);
        }
        Ok(AppliedSnapshot {
            chunk_seq: self.chunk,
            state,
        })
    }

    pub fn dispose(mut self, client: &mut Client) -> Result<(), Error> {
        self.valid = false;
        let seq = client.dispose(&self.key)?;
        if seq == self.bytes {
            Ok(())
        } else {
            Err(Error::Sequence)
        }
    }
}

#[cfg(test)]
mod tests;
