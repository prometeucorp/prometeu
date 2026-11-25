//! Project file access; each host supplies the root representation and native implementation.
use serde::Serialize;
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub dir: bool,
}
pub trait ProjectFiles<Root: ?Sized>: Send + Sync {
    fn bytes(&self, root: &Root, rel: &str) -> Result<Vec<u8>, String>;
    fn list(&self, root: &Root, rel: &str) -> Vec<Entry>;
    fn read(&self, root: &Root, rel: &str) -> Result<String, String>;
    fn stamp(&self, root: &Root, rel: &str) -> Result<String, String>;
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
