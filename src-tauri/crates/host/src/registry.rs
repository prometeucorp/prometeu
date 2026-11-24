//! The machine's own record of its projects and workspaces (ADR 0102). Only the host writes it,
//! so the desktop and agent clients never race on the file; each change is one locked
//! read-modify-write followed by an atomic rename.
use serde::{Deserialize, Serialize};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const FILE: &str = "registry.json";
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Creator {
    /// `desktop` or `agent`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Project {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub added_at: u64,
    pub added_by: Creator,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Conversation {
    /// Tab ID, Claude session ID and host terminal ID at once.
    pub id: String,
    pub title: String,
    pub created_at: u64,
    /// `auto` runs without permission prompts; `ask` keeps Claude's dialogs.
    pub permission: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Workspace {
    pub id: String,
    /// The project's repository path.
    pub project: String,
    pub title: String,
    pub branch: String,
    pub base: String,
    pub worktree: String,
    pub created_at: u64,
    pub created_by: Creator,
    #[serde(default)]
    pub closed: bool,
    /// The branch existed before the workspace, so closing it must not delete the branch.
    #[serde(default)]
    pub keep_branch: bool,
    #[serde(default)]
    pub conversations: Vec<Conversation>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Registry {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
}

impl Registry {
    pub fn project(&self, key: &str) -> Option<&Project> {
        self.projects
            .iter()
            .find(|p| p.path == key)
            .or_else(|| self.projects.iter().find(|p| p.name == key))
    }

    pub fn workspace_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    /// The workspace that holds a conversation.
    pub fn conversation(&self, id: &str) -> Option<(&Workspace, &Conversation)> {
        self.workspaces
            .iter()
            .find_map(|w| w.conversations.iter().find(|c| c.id == id).map(|c| (w, c)))
    }

    /// Insert a project, or keep the existing one with the same path.
    pub fn add_project(&mut self, project: Project) -> Project {
        if let Some(existing) = self.projects.iter().find(|p| p.path == project.path) {
            return existing.clone();
        }
        self.projects.push(project.clone());
        project
    }

    /// Insert or replace a workspace record by ID.
    pub fn put_workspace(&mut self, workspace: Workspace) {
        match self.workspace_mut(&workspace.id) {
            Some(existing) => *existing = workspace,
            None => self.workspaces.push(workspace),
        }
    }
}

pub struct Store {
    path: PathBuf,
    lock: std::sync::Mutex<()>,
}

impl Store {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join(FILE),
            lock: std::sync::Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Registry, String> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("registry unreadable: {e}"))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn read(&self) -> Result<Registry, String> {
        let _guard = prometeu_core::lock::lock(&self.lock);
        self.load()
    }

    /// Apply `change` and persist the result; nothing is written when it fails.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut Registry) -> Result<T, String>,
    ) -> Result<T, String> {
        let _guard = prometeu_core::lock::lock(&self.lock);
        let mut registry = self.load()?;
        let result = change(&mut registry)?;
        registry.version = VERSION;
        let bytes = serde_json::to_vec_pretty(&registry).map_err(|e| e.to_string())?;
        write_private(&self.path, &bytes)?;
        Ok(result)
    }
}

/// Write `bytes` to `path` (0600) through a temporary file and a rename.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creator() -> Creator {
        Creator {
            kind: "agent".into(),
            name: Some("bot".into()),
        }
    }

    #[test]
    fn updates_persist_and_failed_changes_write_nothing() {
        let dir = std::env::temp_dir().join(format!("registry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        store
            .update(|r| {
                r.add_project(Project {
                    path: "/repo".into(),
                    name: "repo".into(),
                    origin: None,
                    added_at: 1,
                    added_by: creator(),
                });
                Ok(())
            })
            .unwrap();
        let failed: Result<(), String> = store.update(|r| {
            r.projects.clear();
            Err("no".into())
        });
        assert!(failed.is_err());
        let registry = store.read().unwrap();
        assert_eq!(registry.version, VERSION);
        assert_eq!(registry.project("repo").unwrap().path, "/repo");
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(dir.join(FILE)).unwrap().permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_fields_and_older_files_still_load() {
        let registry: Registry = serde_json::from_str(
            r#"{"workspaces":[{"id":"w","project":"/r","title":"t","branch":"b","base":"HEAD",
            "worktree":"/r","created_at":0,"created_by":{"kind":"desktop"},"future":1}]}"#,
        )
        .unwrap();
        assert!(!registry.workspaces[0].closed);
        assert!(registry.workspaces[0].conversations.is_empty());
    }
}
