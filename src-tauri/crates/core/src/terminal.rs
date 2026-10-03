//! Terminal byte retention and delivery. The execution host owns native PTYs and reader tasks.
use crate::lock::lock;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub const SCROLLBACK: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

pub trait TerminalControl: Send + Sync {
    fn resize(&self, size: TerminalSize) -> Result<(), String>;
    fn running(&self) -> bool;
    fn system_id(&self) -> u32;
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
