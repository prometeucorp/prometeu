//! Native project file adapter shared by the desktop and WSL runtime.
pub mod accounts;
mod bytes;
pub mod entries;
pub mod private;
pub mod scripts;
pub mod search;
pub mod settings;
use prometeu_core::{
    error::{code, with_args},
    files::{Entry, FileBlock, FileLocation, ProjectFiles},
};
use std::path::{Path, PathBuf};
fn io(cause: impl ToString) -> String {
    with_args("err.io", &[("cause", cause.to_string())])
}
pub fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let root = root.canonicalize().map_err(io)?;
    let real = root.join(rel).canonicalize().map_err(io)?;
    match real.starts_with(&root) {
        true => Ok(real),
        false => Err(code("err.session.outside")),
    }
}
pub fn open(root: &Path, rel: &str, limit: u64) -> Result<PathBuf, String> {
    let file = inside(root, rel)?;
    let meta = std::fs::metadata(&file).map_err(io)?;
    if meta.len() > limit {
        return Err(with_args(
            "err.session.tooBig",
            &[("kb", (meta.len() / 1024).to_string())],
        ));
    }
    Ok(file)
}
/// Reject edits made since the caller read the file, preserving the desktop conflict contract.
pub fn save(file: &Path, text: &str, was: &str) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(io)?;
    let now = String::from_utf8(bytes).map_err(|_| code("err.session.binary"))?;
    if now != was {
        return Err(code("err.session.changed"));
    }
    std::fs::write(file, text).map_err(io)
}
pub struct NativeFiles;
impl ProjectFiles<Path> for NativeFiles {
    fn bytes(&self, root: &Path, rel: &str) -> Result<Vec<u8>, String> {
        bytes::read(root, rel)
    }
    fn block(
        &self,
        root: &Path,
        rel: &str,
        offset: u64,
        stamp: Option<&str>,
    ) -> Result<FileBlock, String> {
        bytes::block(root, rel, offset, stamp)
    }
    fn location(&self, root: &Path, rel: &str) -> Result<FileLocation, String> {
        let path = inside(root, rel)?;
        Ok(FileLocation {
            dir: std::fs::metadata(&path).map_err(io)?.is_dir(),
            path: path
                .to_str()
                .ok_or_else(|| io("Path is not valid UTF-8"))?
                .into(),
        })
    }
    fn list(&self, root: &Path, rel: &str) -> Vec<Entry> {
        let Ok(dir) = inside(root, rel) else {
            return Vec::new();
        };
        let mut out: Vec<Entry> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    return None;
                }
                let dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                let path = match rel.is_empty() {
                    true => name.clone(),
                    false => format!("{rel}/{name}"),
                };
                Some(Entry { name, path, dir })
            })
            .collect();
        out.sort_by_key(|entry| (!entry.dir, entry.name.to_lowercase()));
        out
    }
    fn read(&self, root: &Path, rel: &str) -> Result<String, String> {
        let file = open(root, rel, 2 * 1024 * 1024)?;
        String::from_utf8(std::fs::read(file).map_err(io)?).map_err(|_| code("err.session.binary"))
    }
    fn stamp(&self, root: &Path, rel: &str) -> Result<String, String> {
        let file = open(root, rel, u64::MAX)?;
        let meta = std::fs::metadata(file).map_err(io)?;
        Ok(bytes::stamp(&meta))
    }
    fn write(&self, root: &Path, rel: &str, text: &str, was: &str) -> Result<(), String> {
        save(&inside(root, rel)?, text, was)
    }
}

pub mod tool_declarations;
