//! The machine's own record of its projects and workspaces (ADR 0103). Only the host writes it,
//! so the desktop and agent clients never race on the file; each change is one locked
//! read-modify-write followed by an atomic rename.
use serde::{Deserialize, Serialize};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub const FILE: &str = "registry.json";
const VERSION: u32 = 4;

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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Conversation {
    /// Tab and host terminal ID. Claude also uses it as its native session ID.
    pub id: String,
    pub title: String,
    pub created_at: u64,
    /// `auto` runs without permission prompts; `ask` keeps Claude's dialogs.
    pub permission: String,
    #[serde(default)]
    pub provider: Provider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
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
    /// None keeps pre-archive registries eligible for migration from a desktop's board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    /// The branch existed before the workspace, so closing it must not delete the branch.
    #[serde(default)]
    pub keep_branch: bool,
    #[serde(default)]
    pub conversations: Vec<Conversation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervision: Option<prometeu_core::supervision::Binding>,
}

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Registry {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<Project>,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub supervision: crate::supervision::State,
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

    /// Record a workspace the desktop created or changed. What the host already holds wins,
    /// except the title and the desktop's own new conversations: an agent may have opened
    /// conversations or closed the workspace since the desktop last read the registry.
    pub fn merge_workspace(&mut self, workspace: Workspace) {
        let Some(existing) = self.workspace_mut(&workspace.id) else {
            self.workspaces.push(workspace);
            return;
        };
        existing.title = workspace.title;
        for conversation in workspace.conversations {
            match existing
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation.id)
            {
                Some(known) => {
                    known.title = conversation.title;
                    if known.provider == conversation.provider {
                        if known.thread_id.is_none() {
                            known.thread_id = conversation.thread_id;
                        }
                        known.model = conversation.model;
                        known.effort = conversation.effort;
                    }
                }
                None => existing.conversations.push(conversation),
            }
        }
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
                let registry: Registry = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("registry unreadable: {e}"))?;
                // Rewriting a newer format would silently drop what this host does not know.
                if registry.version > VERSION {
                    return Err(format!(
                        "registry version {} is newer than this host supports",
                        registry.version
                    ));
                }
                Ok(registry)
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
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp-{}-{n}", std::process::id()));
    let temporary = path.with_file_name(name);
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
    fn a_newer_registry_is_never_rewritten() {
        let dir = std::env::temp_dir().join(format!("registry-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let original =
            serde_json::json!({"version": VERSION + 1, "projects": [], "future": [1]}).to_string();
        std::fs::write(dir.join(FILE), &original).unwrap();
        let store = Store::new(&dir);
        assert!(store.read().is_err());
        assert!(store.update(|_| Ok(())).is_err());
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), original);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_version_one_workspace_upgrades_before_persisting_supervision() {
        let dir = std::env::temp_dir().join(format!("registry-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"{"version":1,"workspaces":[{
            "id":"workspace","project":"/repo","title":"Existing task","branch":"feature/work",
            "base":"HEAD","worktree":"/worktree","created_at":1,"created_by":{"kind":"desktop"}
        }]}"#,
        )
        .unwrap();
        let store = Store::new(&dir);
        let legacy = store.read().unwrap();
        assert_eq!(legacy.version, 1);
        assert!(legacy.workspaces[0].supervision.is_none());
        store
            .update(|registry| {
                registry.workspace_mut("workspace").unwrap().supervision =
                    Some(prometeu_core::supervision::Binding {
                        supervisor_id: "supervisor".into(),
                    });
                Ok(())
            })
            .unwrap();
        let saved = store.read().unwrap();
        assert_eq!(saved.version, VERSION);
        assert_eq!(saved.workspaces[0].worktree, "/worktree");
        assert_eq!(saved.workspaces[0].title, "Existing task");
        assert_eq!(
            saved.workspaces[0]
                .supervision
                .as_ref()
                .unwrap()
                .supervisor_id,
            "supervisor"
        );
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
        assert!(registry.workspaces[0].archived.is_none());
        assert!(registry.workspaces[0].conversations.is_empty());
    }

    #[test]
    fn version_two_upgrades_without_losing_supervision_or_inventing_archive_state() {
        let dir = std::env::temp_dir().join(format!("registry-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"{"version":2,"workspaces":[{
            "id":"w","project":"/r","title":"Task","branch":"main","base":"HEAD",
            "worktree":"/r","created_at":0,"created_by":{"kind":"desktop"},
            "supervision":{"supervisor_id":"supervisor"}
        }]}"#,
        )
        .unwrap();
        let store = Store::new(&dir);
        store
            .update(|registry| {
                let workspace = registry.workspace_mut("w").unwrap();
                assert!(workspace.archived.is_none());
                workspace.archived = Some(true);
                Ok(())
            })
            .unwrap();
        let saved = store.read().unwrap();
        assert_eq!(saved.version, VERSION);
        assert_eq!(saved.workspaces[0].archived, Some(true));
        assert_eq!(
            saved.workspaces[0]
                .supervision
                .as_ref()
                .unwrap()
                .supervisor_id,
            "supervisor"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn version_three_keeps_codex_identity_when_archive_state_is_added() {
        let dir = std::env::temp_dir().join(format!("registry-v3-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(FILE),
            r#"{"version":3,"workspaces":[{
            "id":"w","project":"/r","title":"Task","branch":"main","base":"HEAD",
            "worktree":"/r","created_at":0,"created_by":{"kind":"desktop"},
            "conversations":[{"id":"c","title":"Task","created_at":1,"permission":"auto",
                "provider":"codex","thread_id":"native-thread","model":"model","effort":"high"}]
        }]}"#,
        )
        .unwrap();
        let store = Store::new(&dir);
        store
            .update(|registry| {
                let workspace = registry.workspace_mut("w").unwrap();
                assert!(workspace.archived.is_none());
                workspace.archived = Some(true);
                Ok(())
            })
            .unwrap();
        let saved = store.read().unwrap();
        assert_eq!(saved.version, 4);
        assert_eq!(saved.workspaces[0].archived, Some(true));
        let conversation = &saved.workspaces[0].conversations[0];
        assert_eq!(conversation.provider, Provider::Codex);
        assert_eq!(conversation.thread_id.as_deref(), Some("native-thread"));
        assert_eq!(conversation.model, "model");
        assert_eq!(conversation.effort, "high");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_conversations_keep_claude_and_codex_keeps_its_native_identity() {
        let legacy =
            serde_json::json!({"id":"c","title":"Task","created_at":1,"permission":"auto"});
        let claude: Conversation = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(claude.provider, Provider::Claude);
        assert!(claude.thread_id.is_none());
        let mut codex = legacy;
        codex["provider"] = "codex".into();
        codex["thread_id"] = "native-thread".into();
        codex["model"] = "model-from-machine".into();
        codex["effort"] = "high".into();
        let parsed: Conversation = serde_json::from_value(codex.clone()).unwrap();
        assert_eq!(parsed.provider, Provider::Codex);
        assert_eq!(serde_json::to_value(parsed).unwrap(), codex);
        codex["provider"] = "unsupported".into();
        assert!(serde_json::from_value::<Conversation>(codex).is_err());
    }
}
