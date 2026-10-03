//! Private account registry persistence for an explicit execution-side root.
use crate::{io, private as paths};
use prometeu_core::accounts::{AccountStore, Registry};
use prometeu_core::error::code;
use std::path::PathBuf;

pub struct FileAccountStore {
    path: PathBuf,
}
impl FileAccountStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            path: root.join("accounts.json"),
        }
    }
}
impl AccountStore for FileAccountStore {
    fn load(&self) -> Result<Option<Registry>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(body) => serde_json::from_str(&body)
                .map(Some)
                .map_err(|_| code("err.account.store")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io(error)),
        }
    }
    fn save(&self, registry: &Registry) -> Result<(), String> {
        paths::write_private(&self.path, &serde_json::to_string(registry).map_err(io)?).map_err(io)
    }
}
