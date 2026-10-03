//! Application-facing terminal operations. Raw byte arrays preserve split UTF-8 and control codes.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct OpenedTerminal {
    pub id: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct TerminalSnapshot {
    pub id: String,
    pub data: Vec<u8>,
    pub seq: u64,
    pub running: bool,
    pub code: Option<u32>,
}
pub trait TerminalClient: Send {
    fn terminal_current(&mut self) -> Result<Option<TerminalSnapshot>, String>;
    fn terminal_supported(&self) -> bool;
    fn terminal_open(&mut self, cols: u16, rows: u16) -> Result<OpenedTerminal, String>;
    fn terminal_write(&mut self, id: String, data: Vec<u8>) -> Result<(), String>;
    fn terminal_resize(&mut self, id: String, cols: u16, rows: u16) -> Result<(), String>;
    fn terminal_snapshot(&mut self, id: String) -> Result<TerminalSnapshot, String>;
    fn terminal_acknowledge(&mut self, id: String, seq: u64) -> Result<(), String>;
    fn terminal_close(&mut self, id: String) -> Result<(), String>;
}
