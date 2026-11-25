//! Bounded binary reads.
use crate::{io, open};
use prometeu_core::{error::with_args, files::MAX_FILE_BYTES};
use std::{
    fs::{File, Metadata},
    io::Read,
    path::Path,
};

pub(crate) fn stamp(meta: &Metadata) -> String {
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .unwrap_or_default();
    format!(
        "{}.{}-{}",
        modified.as_secs(),
        modified.subsec_nanos(),
        meta.len()
    )
}
fn source(root: &Path, rel: &str) -> Result<File, String> {
    let path = open(root, rel, MAX_FILE_BYTES)?;
    if !std::fs::metadata(&path).map_err(io)?.is_file() {
        return Err(io("Not a regular file"));
    }
    let file = File::open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(io("Not a regular file"));
    }
    Ok(file)
}
fn bounded(file: File) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(with_args(
            "err.session.tooBig",
            &[("kb", (bytes.len() / 1024).to_string())],
        ));
    }
    Ok(bytes)
}
pub(crate) fn read(root: &Path, rel: &str) -> Result<Vec<u8>, String> {
    bounded(source(root, rel)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::NativeFiles;
    use prometeu_core::files::ProjectFiles;
    #[test]
    fn binary_reads_preserve_bytes_and_bounds() {
        let root = std::env::temp_dir().join(format!("binary-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let body: Vec<_> = (0..1024 * 513).map(|i| (i % 256) as u8).collect();
        std::fs::write(root.join("sample.bin"), &body).unwrap();
        assert_eq!(NativeFiles.bytes(&root, "sample.bin").unwrap(), body);
        std::fs::write(root.join("empty"), []).unwrap();
        assert!(NativeFiles.bytes(&root, "empty").unwrap().is_empty());
        assert!(NativeFiles.bytes(&root, "").is_err());
        let large = File::create(root.join("large")).unwrap();
        large.set_len(MAX_FILE_BYTES + 1).unwrap();
        assert!(NativeFiles
            .bytes(&root, "large")
            .unwrap_err()
            .contains("err.session.tooBig"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn binary_reads_reject_outside_links_and_directories() {
        let base = std::env::temp_dir().join(format!("binary-boundary-{}", uuid::Uuid::new_v4()));
        let root = base.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(base.join("outside"), [0, 255]).unwrap();
        std::os::unix::fs::symlink(base.join("outside"), root.join("link")).unwrap();
        for rel in ["../outside", "link", ""] {
            assert!(NativeFiles.bytes(&root, rel).is_err());
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
