//! One serialized private atomic document commits definition, cursor and run changes together.
use prometeu_core::automation::AutomationState;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Cursor {
    pub revision: u64,
    pub identity: String,
    pub initialized: bool,
    pub last_polled_at: u64,
    pub seen: BTreeMap<String, String>,
    /// Monotonic admitted event occurrence within this cursor's identity/scope.
    #[serde(default)]
    pub occurrence: u64,
    pub error: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    pub state: AutomationState,
    #[serde(default)]
    pub cursors: BTreeMap<String, Cursor>,
    #[serde(default)]
    pub waits: BTreeMap<String, u64>,
    /// Intent persisted before an external mutation. Crash recovery never retries ambiguous writes.
    #[serde(default)]
    pub intents: BTreeMap<String, String>,
    #[serde(default)]
    pub worktrees: BTreeMap<String, super::workspace::Reservation>,
    #[serde(default)]
    pub retries: BTreeMap<String, u32>,
    #[serde(default)]
    pub workspace_fingerprints: BTreeMap<String, String>,
    #[serde(default)]
    pub local_heads: BTreeMap<String, String>,
    #[serde(default)]
    pub validations: BTreeMap<String, Evidence>,
    #[serde(default)]
    pub approval_evidence: BTreeMap<String, Evidence>,
    #[serde(default)]
    pub cancel_requested: std::collections::BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Evidence {
    pub head: String,
    pub source: String,
    pub passed: bool,
}

fn gate() -> &'static Mutex<()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(Mutex::default)
}

fn read(path: &Path) -> Result<Document, String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_file() || meta.len() > 64 * 1024 * 1024 => {
            return Err("automation_storage_invalid: Store must be a bounded regular file".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Document::default())
        }
        Err(_) => return Err("automation_storage_unavailable".into()),
    }
    let bytes = std::fs::read(path).map_err(|_| "automation_storage_unavailable")?;
    let document: Document = serde_json::from_slice(&bytes).map_err(|_| "automation_storage_corrupt: Workflow history was preserved; repair or restore the private automation store")?;
    if document.state.schema_version != 1 {
        return Err("automation_storage_version: Unsupported workflow store version".into());
    }
    Ok(document)
}

pub(super) fn transaction_at<T>(
    path: &Path,
    write: bool,
    operation: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    let _guard = gate().lock().map_err(|_| "automation_storage_lock")?;
    let mut document = read(path)?;
    let result = operation(&mut document)?;
    if write {
        let bytes =
            serde_json::to_vec(&document).map_err(|_| "automation_storage_serialization")?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(
                "automation_storage_full: Private automation history reached its 64 MiB limit; no transaction was committed. Back up the store before manual recovery".into(),
            );
        }
        crate::paths::write_private_bytes(path, &bytes)
            .map_err(|_| "automation_storage_write: Changes were not committed")?;
    }
    Ok(result)
}

pub(super) fn transaction<T>(
    write: bool,
    operation: impl FnOnce(&mut Document) -> Result<T, String>,
) -> Result<T, String> {
    transaction_at(
        &crate::paths::root().join("automations/state.json"),
        write,
        operation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automations::catalog;
    #[test]
    fn atomic_revision_cas_and_corruption_preserve_previous_document() {
        let root = std::env::temp_dir().join(format!("automation-store-{}", uuid::Uuid::new_v4()));
        let path = root.join("state.json");
        let workflow = catalog::templates().remove(0);
        let saved = transaction_at(&path, true, |doc| {
            doc.state
                .save_workflow(workflow, None, &catalog::registry())
        })
        .unwrap();
        let previous = std::fs::read(&path).unwrap();
        assert!(transaction_at(&path, true, |doc| doc.state.save_workflow(
            saved.clone(),
            Some(0),
            &catalog::registry()
        ))
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        transaction_at(&path, true, |doc| {
            doc.state
                .save_workflow(saved.clone(), Some(saved.revision), &catalog::registry())
        })
        .unwrap();
        transaction_at(&path, false, |doc| {
            assert_eq!(doc.state.revisions.len(), 1);
            Ok(())
        })
        .unwrap();
        std::fs::write(&path, b"corrupt").unwrap();
        assert!(transaction_at(&path, true, |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"corrupt");
        for incomplete in [b"{}".as_slice(), b"{\"cursors\":{}}".as_slice()] {
            std::fs::write(&path, incomplete).unwrap();
            assert!(transaction_at(&path, true, |_| Ok(())).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), incomplete);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn store_is_private_and_rejects_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = std::env::temp_dir().join(format!("automation-store-{}", uuid::Uuid::new_v4()));
        let path = root.join("state.json");
        transaction_at(&path, true, |_| Ok(())).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let link = root.join("link.json");
        symlink(&path, &link).unwrap();
        assert!(transaction_at(&link, true, |_| Ok(())).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
