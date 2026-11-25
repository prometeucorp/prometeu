//! Terminal byte retention and delivery. The execution host owns native PTYs and reader tasks.
use crate::lock::lock;
use crate::process::ProcessHandle;
use crate::tasks::TaskExecutor;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SCROLLBACK: usize = 512 * 1024;

/// Dock keys are `<owner>:<kind>`. A project owner is its folder path, which may itself contain
/// ':', while kinds never do: `/repo` must not claim the shells of `/repo:v2`.
pub fn dock_kind<'a>(key: &'a str, owner: &str) -> Option<&'a str> {
    key.strip_prefix(owner)?
        .strip_prefix(':')
        .filter(|kind| !kind.contains(':'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

pub trait TerminalControl: Send + Sync {
    fn resize(&self, size: TerminalSize) -> Result<(), String>;
    fn running(&self) -> bool;
    /// Whether the terminal runs a command rather than its shell waiting for input: a job holding
    /// the foreground, or a program the shell replaced itself with.
    fn running_command(&self) -> bool {
        false
    }
    fn system_id(&self) -> u32;
    /// Release terminal IO without starting a shutdown policy. Conversation hosts own escalation;
    /// implementations must cancel held reader/writer handles without waiting for their release.
    fn disconnect(&self) {}
    /// Request asynchronous hangup and escalation. Implementations serialize signals with reaping.
    fn close(&self);
}

pub trait TerminalWait: Send {
    /// Retain the native terminal exit code, including its signal convention. Drop cleans up an
    /// unreaped child even when opening the host or starting its reader fails.
    fn wait(&mut self) -> Result<u32, String>;
}

pub struct StartedTerminal {
    pub input: Box<dyn Write + Send>,
    pub output: Box<dyn Read + Send>,
    pub control: Arc<dyn TerminalControl>,
    /// The same child, exposed to conversation lifecycle and resource accounting. The waiter
    /// remains its single owner; this handle does not start another reaper or shutdown task.
    pub process: ProcessHandle,
    pub waiter: Box<dyn TerminalWait>,
}

#[derive(Debug)]
pub enum TerminalError {
    Open(String),
    Spawn(String),
    Reader(String),
    Writer(String),
}

pub trait TerminalFactory<Request>: Send + Sync {
    fn open(&self, request: Request, size: TerminalSize) -> Result<StartedTerminal, TerminalError>;
}

/// Ports run inside the scrollback ordering lock and must not reenter terminal operations.
pub trait TerminalEvents: Send + Sync {
    fn output(&self, bytes: &[u8], seq: u64);
    fn closed(&self, code: Option<u32>);
}

#[derive(Default)]
pub struct Scroll {
    pub bytes: Vec<u8>,
    pub seq: u64,
    pub exit_code: Option<u32>,
}
impl Scroll {
    pub fn absorb(&mut self, bytes: &[u8]) -> u64 {
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > SCROLLBACK {
            self.bytes.drain(..self.bytes.len() - SCROLLBACK);
        }
        self.seq += 1;
        self.seq
    }
}

pub struct TerminalOutput {
    pub buffer: Arc<Mutex<Scroll>>,
    events: Arc<dyn TerminalEvents>,
    gone: AtomicBool,
}
impl TerminalOutput {
    pub fn new(events: Arc<dyn TerminalEvents>) -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Scroll::default())),
            events,
            gone: AtomicBool::new(false),
        }
    }
    /// Headers, raw output and final status share one byte sequence with snapshots.
    pub fn feed(&self, bytes: &[u8]) {
        let mut buffer = lock(&self.buffer);
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        let seq = buffer.absorb(bytes);
        self.events.output(bytes, seq);
    }
    pub fn finish(&self, code: Option<u32>, notice: &[u8]) {
        let mut buffer = lock(&self.buffer);
        buffer.exit_code = code;
        if self.gone.load(Ordering::Relaxed) {
            return;
        }
        let seq = buffer.absorb(notice);
        self.events.output(notice, seq);
    }
    /// Hosts run completion callbacks outside the buffer lock, then publish closure.
    pub fn closed(&self, code: Option<u32>) {
        let _buffer = lock(&self.buffer);
        if !self.gone.load(Ordering::Relaxed) {
            self.events.closed(code);
        }
    }
    pub fn retire(&self) {
        let _buffer = lock(&self.buffer);
        self.gone.store(true, Ordering::Relaxed);
    }
}

/// How long a terminal's trailing output may keep arriving after its process exited. The host
/// daemon waits as long before announcing a remote exit.
pub const EXIT_DRAIN: Duration = Duration::from_secs(2);

/// Reaps a terminal's process on its own task, so a reader can end at process exit and not only
/// at EOF: a descendant that inherited the terminal keeps it open after the program ends, and
/// whether a PTY reports EOF then differs between systems. Without this, a conversation whose
/// agent already exited would never publish its closure and would look busy forever.
pub struct TerminalExit {
    receiver: mpsc::Receiver<Result<u32, String>>,
    exited: Option<(Result<u32, String>, Instant)>,
    drain: Duration,
}
impl TerminalExit {
    /// The waiter stays the single reaper. If scheduling fails, the task drops it, and its Drop
    /// cleans up the unreaped child.
    pub fn watch(
        mut waiter: Box<dyn TerminalWait>,
        tasks: &dyn TaskExecutor,
        drain: Duration,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        tasks.spawn(Box::new(move || {
            let _ = sender.send(waiter.wait());
        }))?;
        Ok(Self {
            receiver,
            exited: None,
            drain,
        })
    }

    fn observe(&mut self) {
        if self.exited.is_some() {
            return;
        }
        let result = match self.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("terminal waiter ended".into()),
        };
        self.exited = Some((result, Instant::now()));
    }

    /// Whether the process exited and its trailing output had the drain period to arrive. A
    /// reader still open then is held by something other than the program and may stop.
    pub fn drained(&mut self) -> bool {
        self.observe();
        self.exited
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= self.drain)
    }

    /// The exit result, waiting for it when EOF came first.
    pub fn wait(mut self) -> Result<u32, String> {
        self.observe();
        match self.exited {
            Some((result, _)) => result,
            None => self
                .receiver
                .recv()
                .unwrap_or_else(|_| Err("terminal waiter ended".into())),
        }
    }
}

pub struct Terminal {
    input: Box<dyn Write + Send>,
    control: Arc<dyn TerminalControl>,
    pub output: Arc<TerminalOutput>,
    pub buffer: Arc<Mutex<Scroll>>,
    pub script_name: Option<String>,
}
impl Terminal {
    pub fn new(
        input: Box<dyn Write + Send>,
        control: Arc<dyn TerminalControl>,
        output: Arc<TerminalOutput>,
        script_name: Option<String>,
    ) -> Self {
        Self {
            input,
            control,
            buffer: output.buffer.clone(),
            output,
            script_name,
        }
    }
    pub fn write(&mut self, data: &str) -> Result<(), String> {
        self.write_bytes(data.as_bytes())
    }
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<(), String> {
        self.input
            .write_all(data)
            .and_then(|()| self.input.flush())
            .map_err(|e| e.to_string())
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.control.resize(TerminalSize { cols, rows })
    }
    pub fn alive(&self) -> bool {
        self.control.running()
    }
    /// A shell running a command, rather than waiting at its prompt.
    pub fn running_command(&self) -> bool {
        self.control.running_command()
    }
    pub fn pid(&self) -> u32 {
        self.control.system_id()
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        self.output.retire();
        self.control.close();
    }
}

#[cfg(test)]
mod tests;
