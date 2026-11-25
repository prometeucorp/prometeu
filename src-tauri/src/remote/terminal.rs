//! A terminal owned by a machine's host, presented through the same `StartedTerminal` ports as a
//! local PTY. Losing the connection is not process exit: the reader simply waits for the
//! reattachment's replay. Only the host's exit frame, or a host that no longer knows the
//! terminal, ends the stream.
use base64::{engine::general_purpose::STANDARD, Engine};
use prometeu_core::lock::lock;
use prometeu_core::process::{ProcessControl, ProcessHandle};
use prometeu_core::terminal::{StartedTerminal, TerminalControl, TerminalSize, TerminalWait};
use serde_json::json;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::connection::{Connection, TerminalSink};

struct Chunk {
    bytes: Vec<u8>,
    replay: bool,
}

#[derive(Default)]
struct Stream {
    chunks: VecDeque<Chunk>,
    /// `Some(code)` after exit; `Some(None)` when the process was lost with the host.
    exit: Option<Option<u32>>,
    disconnected: bool,
}

pub struct Remote {
    id: String,
    connection: Arc<Connection>,
    stream: Mutex<Stream>,
    ready: Condvar,
    seq: AtomicU64,
    running: AtomicBool,
    /// True while the conversation's reader processes bytes the host replays after an
    /// attachment: the native reader must not answer terminal queries that were already
    /// answered, or that the program stopped waiting for.
    pub replaying: Arc<AtomicBool>,
    /// Set when this app opened the terminal: its first attachment carries output nobody has
    /// seen, including startup queries the program still waits on, so it is live, not replay.
    fresh: AtomicBool,
    /// The watch that mirrors this conversation's transcripts.
    mirror: Mutex<Option<String>>,
}

impl Remote {
    pub fn new(connection: Arc<Connection>, id: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            connection,
            stream: Mutex::new(Stream::default()),
            ready: Condvar::new(),
            seq: AtomicU64::new(0),
            running: AtomicBool::new(true),
            replaying: Arc::new(AtomicBool::new(false)),
            fresh: AtomicBool::new(false),
            mirror: Mutex::new(None),
        })
    }

    pub fn mirrored_by(&self, watch: &str) {
        *lock(&self.mirror) = Some(watch.into());
    }

    /// After the process exits: bring the transcript mirror up to date, so the conversation reads
    /// its last turn before it stops, then let the host drop the terminal, its retained output
    /// and its hook settings.
    fn settle(&self) {
        if let Some(watch) = lock(&self.mirror).clone() {
            let _ = self.connection.request_timeout(
                "watch_flush",
                json!({"id": watch}),
                Duration::from_secs(10),
            );
        }
        self.connection
            .notify("terminal_forget", json!({"id": self.id}));
    }

    /// The terminal was just opened by this app rather than adopted from the host.
    pub fn opened(&self) {
        self.fresh.store(true, Ordering::Release);
    }

    fn push(&self, bytes: Vec<u8>, replay: bool) {
        if bytes.is_empty() {
            return;
        }
        lock(&self.stream).chunks.push_back(Chunk { bytes, replay });
        self.ready.notify_all();
    }

    fn finish(&self, code: Option<u32>) {
        self.running.store(false, Ordering::Release);
        lock(&self.stream).exit.get_or_insert(code);
        self.ready.notify_all();
        self.connection
            .release(&self.id, self as *const Self as *const ());
    }

    fn signal(&self, signal: &str) {
        self.connection
            .notify("terminal_signal", json!({"id": self.id, "signal": signal}));
    }

    /// Present the remote terminal through the local terminal ports.
    pub fn started(self: &Arc<Self>) -> StartedTerminal {
        StartedTerminal {
            input: Box::new(Input(self.clone())),
            output: Box::new(Output {
                remote: self.clone(),
                partial: None,
                previous_replay: false,
            }),
            control: Arc::new(Control(self.clone())),
            process: ProcessHandle::new(Arc::new(Process(self.clone()))),
            waiter: Box::new(Waiter(self.clone())),
        }
    }
}

impl TerminalSink for Remote {
    fn replay(&self, data: Vec<u8>, reset: bool, seq: u64) {
        // A reset means part of the gap is gone: clear the screen before the retained output.
        let mut bytes = if reset && self.seq.load(Ordering::Acquire) > 0 {
            b"\x1bc".to_vec()
        } else {
            Vec::new()
        };
        bytes.extend(data);
        self.seq.store(seq, Ordering::Release);
        let live = self.fresh.swap(false, Ordering::AcqRel);
        self.push(bytes, !live);
    }

    fn output(&self, seq: u64, data: Vec<u8>) {
        if seq <= self.seq.load(Ordering::Acquire) {
            return;
        }
        self.seq.store(seq, Ordering::Release);
        self.push(data, false);
    }

    fn exit(&self, code: Option<u32>) {
        self.finish(code);
    }

    fn lost(&self) {
        self.finish(None);
    }

    fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::Acquire)
    }
}

struct Output {
    remote: Arc<Remote>,
    partial: Option<Vec<u8>>,
    previous_replay: bool,
}

impl Read for Output {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.partial.is_none() {
            // The previous chunk has been fully processed by now: the reader calls read again
            // only after handling it.
            if self.previous_replay {
                self.remote.replaying.store(false, Ordering::Release);
            }
            let mut stream = lock(&self.remote.stream);
            loop {
                if stream.disconnected {
                    return Ok(0);
                }
                if let Some(chunk) = stream.chunks.pop_front() {
                    self.previous_replay = chunk.replay;
                    if chunk.replay {
                        self.remote.replaying.store(true, Ordering::Release);
                    }
                    self.partial = Some(chunk.bytes);
                    break;
                }
                if stream.exit.is_some() {
                    return Ok(0);
                }
                stream = self
                    .remote
                    .ready
                    .wait(stream)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        }
        let bytes = self.partial.take().unwrap_or_default();
        let count = bytes.len().min(buffer.len());
        buffer[..count].copy_from_slice(&bytes[..count]);
        if count < bytes.len() {
            self.partial = Some(bytes[count..].to_vec());
        }
        Ok(count)
    }
}

struct Input(Arc<Remote>);
impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.0.connection.connected() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "machine offline",
            ));
        }
        self.0.connection.notify(
            "terminal_write",
            json!({"id": self.0.id, "data": STANDARD.encode(bytes)}),
        );
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Control(Arc<Remote>);
impl TerminalControl for Control {
    fn resize(&self, size: TerminalSize) -> Result<(), String> {
        self.0.connection.notify(
            "terminal_resize",
            json!({"id": self.0.id, "cols": size.cols, "rows": size.rows}),
        );
        Ok(())
    }
    fn running(&self) -> bool {
        self.0.running.load(Ordering::Acquire)
    }
    fn system_id(&self) -> u32 {
        0
    }
    /// Release this app's view of the stream; the machine keeps the process.
    fn disconnect(&self) {
        lock(&self.0.stream).disconnected = true;
        self.0.ready.notify_all();
    }
    fn close(&self) {
        self.0.signal("hangup");
    }
}

struct Process(Arc<Remote>);
impl ProcessControl for Process {
    fn system_id(&self) -> u32 {
        0
    }
    fn running(&self) -> bool {
        self.0.running.load(Ordering::Acquire)
    }
    fn interrupt(&self) {
        self.0.signal("interrupt");
    }
    fn terminate(&self) {
        self.0.signal("terminate");
    }
    fn kill(&self) {
        self.0.signal("kill");
    }
    fn wait_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while self.running() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        !self.running()
    }
}

/// Waiting ends at the host's exit frame. Dropping an unreaped waiter leaves the process running
/// on the machine: closing the app must not end remote work.
struct Waiter(Arc<Remote>);
impl TerminalWait for Waiter {
    fn wait(&mut self) -> Result<u32, String> {
        let mut stream = lock(&self.0.stream);
        loop {
            match stream.exit {
                Some(Some(code)) => {
                    drop(stream);
                    self.0.settle();
                    return Ok(code);
                }
                Some(None) => return Err("remote terminal lost".into()),
                None if stream.disconnected => return Err("detached".into()),
                None => {
                    stream = self
                        .0
                        .ready
                        .wait(stream)
                        .unwrap_or_else(|poison| poison.into_inner())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::Machine;

    fn remote() -> Arc<Remote> {
        let connection = Connection::new(
            Machine {
                id: "m".into(),
                name: "m".into(),
                ssh: "m".into(),
                limits: Default::default(),
            },
            Box::new(|| {}),
            Box::new(|_, _| {}),
        );
        Remote::new(connection, "t")
    }

    #[test]
    fn replay_and_live_output_follow_sequences() {
        let remote = remote();
        let mut output = remote.started().output;
        remote.replay(b"old".to_vec(), false, 2);
        remote.output(2, b"duplicate".to_vec());
        remote.output(3, b"new".to_vec());
        let mut buffer = [0u8; 16];
        let count = output.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"old");
        assert!(remote.replaying.load(Ordering::Acquire));
        let count = output.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"new");
        assert!(!remote.replaying.load(Ordering::Acquire));
        remote.exit(Some(0));
        assert_eq!(output.read(&mut buffer).unwrap(), 0);
    }

    #[test]
    fn a_terminal_this_app_opened_answers_its_first_output() {
        let remote = remote();
        remote.opened();
        let mut output = remote.started().output;
        let mut buffer = [0u8; 16];
        // Startup queries produced before the first attachment still wait for their replies.
        remote.replay(b"query".to_vec(), false, 1);
        let count = output.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"query");
        assert!(!remote.replaying.load(Ordering::Acquire));
        // A later reattachment replays output that was already handled.
        remote.replay(b"again".to_vec(), false, 2);
        let count = output.read(&mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"again");
        assert!(remote.replaying.load(Ordering::Acquire));
    }

    #[test]
    fn a_lost_terminal_ends_its_waiter_with_an_error() {
        let remote = remote();
        let mut started = remote.started();
        remote.lost();
        assert!(started.waiter.wait().is_err());
        assert!(!started.process.control.running());
    }
}
