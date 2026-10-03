//! Project file access; each host supplies the root representation and native implementation.
use serde::{Deserialize, Serialize};
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
pub const FILE_BLOCK_BYTES: usize = 256 * 1024;

/// Internal bounded transport response; the application still receives a single byte buffer.
#[derive(Debug, Deserialize, Serialize)]
pub struct FileBlock {
    pub data: Vec<u8>,
    pub size: u64,
    pub stamp: String,
}
/// A validated execution-side entry, resolved before a native host reveals it.
#[derive(Deserialize, Serialize)]
pub struct FileLocation {
    pub path: String,
    pub dir: bool,
}
#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}
pub trait ProjectFiles<Root: ?Sized>: Send + Sync {
    fn bytes(&self, root: &Root, rel: &str) -> Result<Vec<u8>, String>;
    fn block(
        &self,
        root: &Root,
        rel: &str,
        offset: u64,
        stamp: Option<&str>,
    ) -> Result<FileBlock, String>;
    fn location(&self, root: &Root, rel: &str) -> Result<FileLocation, String>;
    fn list(&self, root: &Root, rel: &str) -> Vec<Entry>;
    fn read(&self, root: &Root, rel: &str) -> Result<String, String>;
    fn stamp(&self, root: &Root, rel: &str) -> Result<String, String>;
    fn write(&self, root: &Root, rel: &str, text: &str, was: &str) -> Result<(), String>;
}

/// Tree mutations remain separate from text I/O and never delete permanently.
pub trait ProjectEntries<Root: ?Sized>: Send + Sync {
    fn create(&self, root: &Root, rel: &str, dir: bool) -> Result<(), String>;
    fn rename(&self, root: &Root, from: &str, to: &str) -> Result<(), String>;
    fn trash(&self, root: &Root, rel: &str) -> Result<(), String>;
}

pub trait ProjectSearch: Send + Sync {
    fn find(
        &self,
        root: &std::path::Path,
        repositories: &[std::path::PathBuf],
        query: &str,
        recent: &[String],
        files: bool,
    ) -> Vec<Entry>;
}
