//! Local transcript adapters. Provider-owned history is read without appending V1 mirrors.

use crate::paths;
use prometeu_core::conversation::stream::TranscriptStore;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct FileTranscriptStore {
    seed: PathBuf,
    log: PathBuf,
}

impl FileTranscriptStore {
    pub fn new(seed: PathBuf, log: PathBuf) -> Self {
        Self { seed, log }
    }
}

impl TranscriptStore for FileTranscriptStore {
    fn load(&self) -> Result<String, String> {
        read(&self.seed)
    }

    fn append(&self, line: &str) -> Result<(), String> {
        if let Some(dir) = self.log.parent() {
            paths::ensure_private_dir(dir)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.log).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        writeln!(file, "{line}").map_err(|error| error.to_string())
    }
}

/// The provider is the writer. App events remain in the live buffer, never in its native file.
pub struct ProviderTranscriptStore {
    path: PathBuf,
}

impl ProviderTranscriptStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TranscriptStore for ProviderTranscriptStore {
    fn load(&self) -> Result<String, String> {
        read(&self.path)
    }

    fn append(&self, _line: &str) -> Result<(), String> {
        Ok(())
    }
}

fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_history_is_read_without_appending_canonical_mirrors() {
        let root =
            std::env::temp_dir().join(format!("prometeu-transcript-{}", uuid::Uuid::new_v4()));
        let path = root.join("native.jsonl");
        let store = ProviderTranscriptStore::new(path.clone());
        assert_eq!(store.load().unwrap(), "");
        store.append("ignored").unwrap();
        assert!(!path.exists());
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, "native history\n").unwrap();
        store.append("also ignored").unwrap();
        assert_eq!(store.load().unwrap(), "native history\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_history_appends_privately_and_preserves_the_seed() {
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("prometeu-transcript-{}", uuid::Uuid::new_v4()));
        let seed = root.join("seed.jsonl");
        let log = root.join("nested/log.jsonl");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&seed, "legacy history\n").unwrap();
        let store = FileTranscriptStore::new(seed, log.clone());
        assert_eq!(store.load().unwrap(), "legacy history\n");
        store.append("{\"v\":1,\"type\":\"user.message\"}").unwrap();
        std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
        store
            .append("{\"v\":1,\"type\":\"turn.completed\"}")
            .unwrap();
        assert_eq!(
            std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(log.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read_to_string(log).unwrap(),
            "{\"v\":1,\"type\":\"user.message\"}\n{\"v\":1,\"type\":\"turn.completed\"}\n"
        );
        assert_eq!(store.load().unwrap(), "legacy history\n");
        std::fs::remove_dir_all(root).unwrap();
    }
}
