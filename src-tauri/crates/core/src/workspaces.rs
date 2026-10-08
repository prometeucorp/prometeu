//! Workspace catalog. Persistence and directory preparation belong to injected adapters.
use crate::board::{Board, Choice, Project, ProviderId, Repo, Status, Tab, Workspace};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const PRIMARY: &str = "primary";
pub const LIMIT: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub v: u32,
    pub active: String,
    pub board: Board,
}
pub trait CatalogStore: Send + Sync {
    fn load(&self) -> Result<Option<Catalog>, String>;
    fn save(&self, catalog: &Catalog) -> Result<(), String>;
}
pub trait WorkspaceFolders: Send + Sync {
    fn inspect(&self, path: &str) -> Result<Project, String>;
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRequest {
    pub title: String,
    pub path: String,
    pub branch: String,
    pub base: String,
}
pub struct PreparedWorktree {
    pub project: Project,
    pub path: String,
    pub branch: String,
    pub base: String,
}
pub trait WorkspaceWorktrees: Send + Sync {
    /// Create a new branch and checkout. Preserve uncertain outcomes for explicit recovery.
    fn prepare(&self, id: &str, request: &WorktreeRequest) -> Result<PreparedWorktree, String>;
}

/// Composition chooses the initial board; existing catalogs always take precedence.
pub trait CatalogSeed {
    fn create(
        &self,
        folders: &dyn WorkspaceFolders,
        provider: ProviderId,
    ) -> Result<Catalog, String>;
}
pub struct EmptyCatalog;
impl CatalogSeed for EmptyCatalog {
    fn create(&self, _: &dyn WorkspaceFolders, _: ProviderId) -> Result<Catalog, String> {
        Ok(Catalog {
            v: 1,
            active: String::new(),
            board: Board::default(),
        })
    }
}
pub struct PrimaryCatalog<'a>(pub &'a str);
impl CatalogSeed for PrimaryCatalog<'_> {
    fn create(
        &self,
        folders: &dyn WorkspaceFolders,
        provider: ProviderId,
    ) -> Result<Catalog, String> {
        let project = folders.inspect(self.0)?;
        let mut board = Board::default();
        board.workspaces.push(in_place(
            PRIMARY.into(),
            project.name.clone(),
            &project,
            &board.stages[0],
            provider,
        ));
        board.projects.push(project);
        Ok(Catalog {
            v: 1,
            active: PRIMARY.into(),
            board,
        })
    }
}

/// An admitted draft owns only immutable inputs and injected directory inspection.
/// Preparation can run away from the catalog owner without cloning persisted state.
pub struct ApplicationPreparation {
    id: String,
    draft: crate::workspace_draft::Draft,
    folders: Arc<dyn WorkspaceFolders>,
    provider: ProviderId,
}
pub struct PreparedApplication {
    project: Project,
    workspace: Workspace,
}
impl PreparedApplication {
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }
}
impl ApplicationPreparation {
    pub fn prepare(
        self,
        worktrees: &dyn WorkspaceWorktrees,
        references: &dyn crate::repository::RepositoryReferences,
    ) -> Result<PreparedApplication, String> {
        let Self {
            id,
            draft,
            folders,
            provider,
        } = self;
        let title = draft.title.trim();
        let prepared = match (
            draft.worktree,
            draft.branch.trim().is_empty(),
            draft.new_branch,
        ) {
            (true, false, None | Some(true)) => worktrees.prepare(
                &id,
                &WorktreeRequest {
                    title: title.into(),
                    path: draft.project.clone(),
                    branch: draft.branch.clone(),
                    base: draft.base.clone(),
                },
            )?,
            (false, true, _) => {
                let project = folders.inspect(&draft.project)?;
                PreparedWorktree {
                    path: project.path.clone(),
                    branch: references.head(&project.path)?.unwrap_or_default(),
                    project,
                    base: draft.base.clone(),
                }
            }
            _ => return Err(crate::error::code("err.windows.workspaceOptions")),
        };
        let mut workspace = in_place(id, title.into(), &prepared.project, &draft.stage, provider);
        workspace.worktree = prepared.path.clone();
        workspace.branch = prepared.branch;
        workspace.repos[0].worktree = prepared.path.clone();
        workspace.repos[0].base = prepared.base;
        workspace.mcp = draft
            .launch
            .mcp
            .clone()
            .map(crate::selection::Selection::only);
        workspace.plugins = draft
            .launch
            .plugins
            .clone()
            .map(crate::selection::Selection::only);
        workspace.skills = draft
            .launch
            .skills
            .clone()
            .map(crate::selection::Selection::only);
        workspace.model = draft.launch.model.clone();
        workspace.effort = draft.launch.effort.clone();
        workspace.issue = draft.issue.clone();
        workspace.tabs[0].permission = Some(
            draft
                .launch
                .permission
                .unwrap_or(crate::actions::Permission::Auto),
        );
        workspace.tabs[0].pending_prompt =
            crate::workspace_draft::first_message(None, &draft.prompt, &draft.inject);
        Ok(PreparedApplication {
            project: prepared.project,
            workspace,
        })
    }
}

pub struct Workspaces {
    catalog: Catalog,
    store: Arc<dyn CatalogStore>,
    folders: Arc<dyn WorkspaceFolders>,
    provider: ProviderId,
}
impl Workspaces {
    pub fn open(
        store: Arc<dyn CatalogStore>,
        folders: Arc<dyn WorkspaceFolders>,
        primary: &str,
        provider: ProviderId,
    ) -> Result<Self, String> {
        Self::open_with(store, folders, primary, provider, &PrimaryCatalog(primary))
    }
    pub fn open_with(
        store: Arc<dyn CatalogStore>,
        folders: Arc<dyn WorkspaceFolders>,
        primary: &str,
        provider: ProviderId,
        seed: &dyn CatalogSeed,
    ) -> Result<Self, String> {
        let catalog = match store.load()? {
            Some(catalog) => catalog,
            None => {
                let catalog = seed.create(folders.as_ref(), provider)?;
                store.save(&catalog)?;
                catalog
            }
        };
        let mut ids = std::collections::HashSet::new();
        let mut sessions = std::collections::HashSet::new();
        if catalog.v != 1
            || catalog.board.workspaces.len() > LIMIT
            || catalog.board.stages.is_empty()
            || catalog.board.workspaces.iter().any(|w| {
                w.tabs.len() > 32
                    || w.tabs.iter().any(|t| {
                        !valid_id(&t.id)
                            || !sessions.insert(t.id.as_str())
                            || t.choice.as_ref().is_some_and(|c| c.agent != provider)
                    })
                    || w.active
                        .as_ref()
                        .is_some_and(|active| !w.tabs.iter().any(|t| &t.id == active))
                    || !valid_id(&w.id)
                    || !ids.insert(w.id.as_str())
                    || w.agent != provider
                    || (w.id == PRIMARY && w.worktree != primary)
            })
            || (!catalog.active.is_empty() && !ids.contains(catalog.active.as_str()))
        {
            return Err("workspace_catalog_invalid".into());
        }
        Ok(Self {
            catalog,
            store,
            folders,
            provider,
        })
    }
    pub fn add_project(&mut self, path: &str) -> Result<Project, String> {
        let project = self.folders.inspect(path)?;
        let mut next = self.catalog.clone();
        let project = crate::projects::register(&mut next.board, project);
        self.commit(next)?;
        Ok(project)
    }
    pub fn reorder_projects(&mut self, ids: &[String]) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        crate::projects::reorder(&mut next.board.projects, ids);
        self.commit(next)
    }
    pub fn remove_project(&mut self, id: &str) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        crate::projects::remove(&mut next.board, id);
        self.commit(next)
    }
    pub fn snapshot(&self) -> Catalog {
        self.catalog.clone()
    }
    pub fn change(
        &mut self,
        id: &str,
        change: crate::workspace_lifecycle::Change,
    ) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        crate::workspace_lifecycle::apply(&mut next.board, id, change)?;
        if !next.board.workspaces.iter().any(|w| w.id == next.active) {
            next.active.clear();
        }
        self.commit(next)
    }
    pub fn workspace_tools(
        &mut self,
        id: &str,
        axis: crate::workspace_tools::Axis,
        body: &serde_json::Value,
        name: &str,
    ) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        if let Some(value) = crate::workspace_tools::patch(body, name, axis)? {
            crate::workspace_tools::apply(&mut next.board, id, axis, value)?;
        }
        self.commit(next)
    }
    pub fn global_tools(&mut self, body: &serde_json::Value) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        crate::workspace_tools::global(&mut next.board, body)?;
        self.commit(next)
    }
    pub fn tool_trust(
        &mut self,
        declaration: crate::tool_resolution::ProjectDeclaration,
        hash: &str,
        approved: bool,
        at: u64,
    ) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        crate::tool_resolution::record_trust(
            &mut next.board.tool_trust,
            declaration,
            hash,
            approved,
            at,
        )?;
        self.commit(next)
    }
    pub fn get(&self, id: &str) -> Result<&Workspace, String> {
        self.catalog
            .board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or("workspace_not_found".into())
    }
    pub fn create(&mut self, title: &str, path: &str) -> Result<Catalog, String> {
        let title = self.validate_creation(title)?;
        let project = self.folders.inspect(path)?;
        let workspace = in_place(
            uuid::Uuid::new_v4().to_string(),
            title.into(),
            &project,
            &self.catalog.board.stages[0],
            self.provider,
        );
        self.register(project, workspace)
    }
    pub fn create_worktree(
        &mut self,
        request: &WorktreeRequest,
        worktrees: &dyn WorkspaceWorktrees,
    ) -> Result<Catalog, String> {
        let title = self.validate_creation(&request.title)?;
        let id = uuid::Uuid::new_v4().to_string();
        let prepared = worktrees.prepare(&id, request)?;
        let mut workspace = in_place(
            id,
            title.into(),
            &prepared.project,
            &self.catalog.board.stages[0],
            self.provider,
        );
        workspace.worktree = prepared.path.clone();
        workspace.branch = prepared.branch;
        workspace.repos[0].worktree = prepared.path.clone();
        workspace.repos[0].base = prepared.base;
        // Git and catalog persistence are separate commits. Never delete files after an
        // uncertain save: the atomic rename may already have published the new catalog.
        self.register(prepared.project, workspace)
            .map_err(|error| format!("workspace_worktree_unsaved: {}\n{error}", prepared.path))
    }
    pub fn create_application(
        &mut self,
        draft: &crate::workspace_draft::Draft,
        worktrees: &dyn WorkspaceWorktrees,
        references: &dyn crate::repository::RepositoryReferences,
    ) -> Result<Workspace, String> {
        let prepared = self
            .plan_application(draft)?
            .prepare(worktrees, references)?;
        self.finish_application(prepared)
    }
    /// Admit the existing application draft before any checkout or persistence effects.
    pub fn plan_application(
        &self,
        draft: &crate::workspace_draft::Draft,
    ) -> Result<ApplicationPreparation, String> {
        self.validate_creation(&draft.title)?;
        if draft.launch.agent != self.provider {
            return Err("workspace_provider_unsupported".into());
        }
        if !self.catalog.board.stages.contains(&draft.stage) {
            return Err("workspace_stage_invalid".into());
        }
        if !draft.extras.is_empty()
            || !draft.kickoff.is_empty()
            || draft.launch.plan
            || !draft.launch.instructions.is_empty()
            || draft.launch.config_scope.is_some()
        {
            return Err(crate::error::code("err.windows.workspaceOptions"));
        }
        match (
            draft.worktree,
            draft.branch.trim().is_empty(),
            draft.new_branch,
        ) {
            (true, false, None | Some(true)) | (false, true, _) => {}
            _ => return Err(crate::error::code("err.windows.workspaceOptions")),
        }
        Ok(ApplicationPreparation {
            id: uuid::Uuid::new_v4().to_string(),
            draft: draft.clone(),
            folders: self.folders.clone(),
            provider: self.provider,
        })
    }
    pub fn finish_application(
        &mut self,
        prepared: PreparedApplication,
    ) -> Result<Workspace, String> {
        let PreparedApplication { project, workspace } = prepared;
        let result = (|| {
            self.validate_creation(&workspace.title)?;
            if !self.catalog.board.stages.contains(&workspace.stage) {
                return Err("workspace_stage_invalid".into());
            }
            self.register(project, workspace.clone())?;
            Ok(workspace.clone())
        })();
        result.map_err(|error: String| {
            format!(
                "workspace_creation_unsaved: {}\n{error}",
                workspace.worktree
            )
        })
    }
    pub fn launch_result(&mut self, id: &str, error: Option<String>) -> Result<Workspace, String> {
        self.get(id)?;
        let mut next = self.catalog.clone();
        let workspace = next.board.workspace_mut(id).unwrap();
        if error.is_none() {
            workspace.tabs[0].pending_prompt = None;
        }
        workspace.failed = error;
        let workspace = workspace.clone();
        self.commit(next)?;
        Ok(workspace)
    }
    pub fn port(&mut self, id: &str, port: u16) -> Result<Workspace, String> {
        self.get(id)?;
        let mut next = self.catalog.clone();
        let workspace = next.board.workspace_mut(id).unwrap();
        workspace.port = Some(port);
        let workspace = workspace.clone();
        self.commit(next)?;
        Ok(workspace)
    }
    fn validate_creation<'a>(&self, title: &'a str) -> Result<&'a str, String> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("workspace_title_invalid".into());
        }
        if self.catalog.board.workspaces.len() >= LIMIT {
            return Err("workspace_limit".into());
        }
        Ok(title)
    }
    fn register(&mut self, project: Project, workspace: Workspace) -> Result<Catalog, String> {
        let mut next = self.catalog.clone();
        next.board.workspaces.push(workspace);
        crate::projects::register(&mut next.board, project);
        self.commit(next)
    }
    pub fn select(&mut self, id: &str) -> Result<Catalog, String> {
        self.get(id)?;
        let mut next = self.catalog.clone();
        next.active = id.into();
        self.commit(next)
    }
    pub fn stage(&mut self, id: &str, stage: &str) -> Result<Catalog, String> {
        self.get(id)?;
        if !self.catalog.board.stages.iter().any(|s| s == stage) {
            return Err("workspace_stage_invalid".into());
        }
        let mut next = self.catalog.clone();
        next.board
            .workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .unwrap()
            .stage = stage.into();
        self.commit(next)
    }
    pub fn add_tab(&mut self, workspace: &str, choice: Option<Choice>) -> Result<Tab, String> {
        let current = self.get(workspace)?;
        if current.tabs.len() >= 32 {
            return Err("workspace_tab_limit".into());
        }
        if choice.as_ref().is_some_and(|c| c.agent != self.provider) {
            return Err("workspace_provider_unsupported".into());
        }
        let mut next = self.catalog.clone();
        let tab = Tab {
            id: uuid::Uuid::new_v4().to_string(),
            title: String::new(),
            status: Status::Desligada,
            note: None,
            pending_prompt: None,
            plan: false,
            permission: current.tabs.first().and_then(|tab| tab.permission),
            task: None,
            agent_session: None,
            tokens: None,
            context_tokens: None,
            context_window: None,
            choice,
            kickoff: None,
        };
        let workspace = next.board.workspace_mut(workspace).unwrap();
        workspace.active = Some(tab.id.clone());
        workspace.tabs.push(tab.clone());
        self.commit(next)?;
        Ok(tab)
    }
    pub fn close_tab(&mut self, workspace: &str, tab: &str) -> Result<Catalog, String> {
        self.get(workspace)?;
        let mut next = self.catalog.clone();
        let workspace = next.board.workspace_mut(workspace).unwrap();
        workspace.tabs.retain(|t| t.id != tab);
        if workspace.active.as_deref() == Some(tab) {
            workspace.active = workspace.tabs.first().map(|t| t.id.clone());
        }
        self.commit(next)
    }
    pub fn mark_read(&mut self, id: &str) -> Result<Catalog, String> {
        self.get(id)?;
        let mut next = self.catalog.clone();
        next.board.workspace_mut(id).unwrap().unread = false;
        self.commit(next)
    }
    pub fn focus_tab(&mut self, id: &str, tab: &str) -> Result<Catalog, String> {
        if !self.get(id)?.tabs.iter().any(|t| t.id == tab) {
            return Err("workspace_not_found".into());
        }
        let mut next = self.catalog.clone();
        next.board.workspace_mut(id).unwrap().active = Some(tab.into());
        self.commit(next)
    }
    fn commit(&mut self, next: Catalog) -> Result<Catalog, String> {
        self.store.save(&next)?;
        self.catalog = next;
        Ok(self.snapshot())
    }
}
pub fn valid_id(id: &str) -> bool {
    id == PRIMARY || uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
}

fn in_place(
    id: String,
    title: String,
    project: &Project,
    stage: &str,
    provider: ProviderId,
) -> Workspace {
    Workspace {
        id: id.clone(),
        title,
        project: project.id.clone(),
        repo: project.path.clone(),
        repo_name: project.name.clone(),
        branch: String::new(),
        worktree: project.path.clone(),
        repos: vec![Repo {
            path: project.path.clone(),
            name: project.name.clone(),
            worktree: project.path.clone(),
            base: String::new(),
            pr: None,
        }],
        stage: stage.into(),
        archived: false,
        pinned: false,
        unread: false,
        model: String::new(),
        effort: String::new(),
        agent: provider,
        port: None,
        issue: None,
        pr: None,
        cleaned: false,
        preserve_branches: vec![],
        shared: false,
        share_team: None,
        audience: None,
        remote_control: false,
        rights: None,
        preparing: false,
        failed: None,
        mcp: None,
        plugins: None,
        skills: None,
        active: Some(id.clone()),
        tabs: vec![Tab {
            id,
            title: String::new(),
            status: Status::Desligada,
            note: None,
            pending_prompt: None,
            plan: false,
            permission: None,
            task: None,
            agent_session: None,
            tokens: None,
            context_tokens: None,
            context_window: None,
            choice: None,
            kickoff: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    };
    #[derive(Default)]
    struct Memory {
        value: Mutex<Option<Catalog>>,
        fail: AtomicBool,
    }
    impl CatalogStore for Memory {
        fn load(&self) -> Result<Option<Catalog>, String> {
            Ok(self.value.lock().unwrap().clone())
        }
        fn save(&self, value: &Catalog) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("disk full".into());
            }
            *self.value.lock().unwrap() = Some(value.clone());
            Ok(())
        }
    }
    struct Folders;
    impl WorkspaceFolders for Folders {
        fn inspect(&self, path: &str) -> Result<Project, String> {
            if path == "/missing" {
                return Err("workspace_folder_invalid".into());
            }
            Ok(Project {
                id: path.into(),
                path: path.into(),
                name: "Project".into(),
            })
        }
    }
    #[test]
    fn lifecycle_changes_persist_without_conflating_stage_and_status_and_roll_back_failed_saves() {
        use crate::workspace_lifecycle::Change;
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        service.catalog.board.workspaces[0].tabs[0].status = Status::Rodando;
        service
            .change(PRIMARY, Change::Rename("  Review  ".into()))
            .unwrap();
        service.change(PRIMARY, Change::Rename(" ".into())).unwrap();
        service.change(PRIMARY, Change::Pin(true)).unwrap();
        assert!(service.get(PRIMARY).unwrap().tabs[0].status == Status::Rodando);
        let stage = service.get(PRIMARY).unwrap().stage.clone();
        store.fail.store(true, Ordering::SeqCst);
        assert!(service.change(PRIMARY, Change::Archive(true)).is_err());
        assert!(!service.get(PRIMARY).unwrap().archived);
        assert!(service.get(PRIMARY).unwrap().tabs[0].status == Status::Rodando);
        assert!(service.change(PRIMARY, Change::Remove).is_err());
        assert_eq!(service.snapshot().active, PRIMARY);
        store.fail.store(false, Ordering::SeqCst);
        service.change(PRIMARY, Change::Archive(true)).unwrap();
        assert_eq!(service.get(PRIMARY).unwrap().stage, stage);
        assert!(service.get(PRIMARY).unwrap().tabs[0].status == Status::Desligada);
        let mut restored =
            Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Codex).unwrap();
        assert_eq!(restored.get(PRIMARY).unwrap().title, "Review");
        assert!(restored.get(PRIMARY).unwrap().pinned);
        restored.change(PRIMARY, Change::Finish).unwrap();
        assert_eq!(
            &restored.get(PRIMARY).unwrap().stage,
            restored.snapshot().board.stages.last().unwrap()
        );
        restored.change(PRIMARY, Change::Remove).unwrap();
        assert!(restored.snapshot().active.is_empty());
        assert!(restored.snapshot().board.workspaces.is_empty());
        assert_eq!(restored.snapshot().board.projects.len(), 1);
    }
    #[test]
    fn empty_application_catalog_reopens_and_preserves_existing_workspace_catalogs() {
        let store = Arc::new(Memory::default());
        let open = || {
            Workspaces::open_with(
                store.clone(),
                Arc::new(Folders),
                "/missing",
                ProviderId::Codex,
                &EmptyCatalog,
            )
            .unwrap()
        };
        let mut catalog = open();
        assert!(catalog.snapshot().board.projects.is_empty());
        assert!(catalog.snapshot().board.workspaces.is_empty());
        assert!(open().snapshot().active.is_empty());
        catalog.add_project("/project").unwrap();
        assert_eq!(open().snapshot().board.projects.len(), 1);
        let created = catalog.create("First conversation", "/project").unwrap();
        assert_eq!(
            open().snapshot().board.workspaces[0].id,
            created.board.workspaces[0].id
        );
        let legacy = Arc::new(Memory::default());
        Workspaces::open(
            legacy.clone(),
            Arc::new(Folders),
            "/original",
            ProviderId::Codex,
        )
        .unwrap();
        let restored = Workspaces::open_with(
            legacy,
            Arc::new(Folders),
            "/original",
            ProviderId::Codex,
            &EmptyCatalog,
        )
        .unwrap();
        assert_eq!(restored.snapshot().active, PRIMARY);
        assert_eq!(
            restored.snapshot().board.workspaces[0].worktree,
            "/original"
        );
    }
    #[test]
    fn project_registration_is_independent_of_workspaces_and_commits_before_publication() {
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        let project = service.add_project("/another").unwrap();
        assert_eq!(service.add_project("/another").unwrap().id, project.id);
        assert_eq!(service.snapshot().board.projects.len(), 2);
        assert_eq!(service.snapshot().board.workspaces.len(), 1);
        assert!(service.add_project("/missing").is_err());
        store.fail.store(true, Ordering::SeqCst);
        assert!(service.add_project("/failed").is_err());
        assert!(service.remove_project("/project").is_err());
        assert_eq!(service.snapshot().board.projects.len(), 2);
        store.fail.store(false, Ordering::SeqCst);
        service.remove_project("/project").unwrap();
        let restored = Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Codex)
            .unwrap()
            .snapshot();
        assert_eq!(restored.board.projects.len(), 1);
        assert_eq!(restored.board.projects[0].id, project.id);
        assert_eq!(restored.board.workspaces.len(), 1);
        assert_eq!(restored.board.workspaces[0].project, "/project");
    }
    #[test]
    fn project_reordering_persists_and_failed_saves_preserve_the_previous_order() {
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        let second = service.add_project("/another").unwrap();
        service
            .reorder_projects(std::slice::from_ref(&second.id))
            .unwrap();
        assert_eq!(service.snapshot().board.projects[0].id, second.id);
        store.fail.store(true, Ordering::SeqCst);
        assert!(service.reorder_projects(&["/project".into()]).is_err());
        assert_eq!(service.snapshot().board.projects[0].id, second.id);
        store.fail.store(false, Ordering::SeqCst);
        let restored =
            Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Codex).unwrap();
        assert_eq!(restored.snapshot().board.projects[0].id, second.id);
        assert_eq!(restored.snapshot().board.projects[1].id, "/project");
    }
    #[test]
    fn tool_selection_and_trust_publish_only_after_catalog_persistence() {
        use crate::{
            selection::Selection, tool_resolution::ProjectDeclaration, workspace_tools::Axis,
        };
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        let body = serde_json::json!({"mcp":{"base":"none","add":["docs"]}});
        let declaration = || ProjectDeclaration {
            repo: "/project".into(),
            hash: "displayed".into(),
            file: None,
            tools: crate::selection::Tools::default(),
        };
        store.fail.store(true, Ordering::SeqCst);
        assert!(service
            .workspace_tools(PRIMARY, Axis::Mcp, &body, "mcp")
            .is_err());
        assert!(service.global_tools(&body).is_err());
        assert!(service
            .tool_trust(declaration(), "displayed", true, 42)
            .is_err());
        assert_eq!(service.get(PRIMARY).unwrap().mcp, None);
        assert_eq!(service.snapshot().board.tools.mcp, None);
        assert!(service.snapshot().board.tool_trust.is_empty());
        store.fail.store(false, Ordering::SeqCst);
        service
            .workspace_tools(PRIMARY, Axis::Mcp, &body, "mcp")
            .unwrap();
        service.global_tools(&body).unwrap();
        service
            .tool_trust(declaration(), "displayed", true, 42)
            .unwrap();
        let restored =
            Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Codex).unwrap();
        assert_eq!(
            restored.get(PRIMARY).unwrap().mcp,
            Some(Selection::only(vec!["docs".into()]))
        );
        assert_eq!(
            restored.snapshot().board.tools.mcp,
            restored.get(PRIMARY).unwrap().mcp
        );
        assert!(restored.snapshot().board.tool_trust[0].approved);
    }
    #[test]
    fn application_drafts_validate_before_effects_and_preserve_selected_configuration() {
        struct NoWorktree;
        impl WorkspaceWorktrees for NoWorktree {
            fn prepare(&self, _: &str, _: &WorktreeRequest) -> Result<PreparedWorktree, String> {
                panic!("unexpected checkout mutation")
            }
        }
        struct References;
        impl crate::repository::RepositoryReferences for References {
            fn head(&self, _: &str) -> Result<Option<String>, String> {
                Ok(Some("main".into()))
            }
            fn branches(&self, _: &str) -> Result<crate::repository::Branches, String> {
                unreachable!()
            }
        }
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        let value = serde_json::json!({"project":"/project","title":"Launcher work","stage":"Preparando","branch":"","base":"main","worktree":false,"prompt":"Saved initial prompt","inject":["/outside/attached file.txt"],"agent":"codex","model":"selected","effort":"high","mcp":["docs"],"plugins":[],"skills":["skill-review"]});
        let mut unsupported = value.clone();
        unsupported["extras"] = serde_json::json!(["/other"]);
        assert!(service
            .create_application(
                &serde_json::from_value(unsupported).unwrap(),
                &NoWorktree,
                &References
            )
            .is_err());
        assert_eq!(service.snapshot().board.workspaces.len(), 1);
        let draft = serde_json::from_value(value).unwrap();
        let preparation = service.plan_application(&draft).unwrap();
        service
            .change(
                PRIMARY,
                crate::workspace_lifecycle::Change::Rename("Edited during preparation".into()),
            )
            .unwrap();
        let prepared = preparation.prepare(&NoWorktree, &References).unwrap();
        let created = service.finish_application(prepared).unwrap();
        assert_eq!(
            service.get(PRIMARY).unwrap().title,
            "Edited during preparation"
        );
        assert_eq!(created.branch, "main");
        assert_eq!(created.model, "selected");
        assert_eq!(created.effort, "high");
        assert_eq!(
            created.mcp,
            Some(crate::selection::Selection::only(vec!["docs".into()]))
        );
        assert_eq!(
            created.plugins,
            Some(crate::selection::Selection::only(vec![]))
        );
        assert_eq!(
            created.skills,
            Some(crate::selection::Selection::only(vec![
                "skill-review".into()
            ]))
        );
        assert_eq!(
            created.tabs[0].pending_prompt.as_deref(),
            Some("@\"/outside/attached file.txt\"\n\nSaved initial prompt")
        );
        service
            .launch_result(&created.id, Some("launch failed".into()))
            .unwrap();
        assert!(service.get(&created.id).unwrap().tabs[0]
            .pending_prompt
            .is_some());
        store.fail.store(true, Ordering::SeqCst);
        assert!(service
            .create_application(&draft, &NoWorktree, &References)
            .is_err());
        assert_eq!(service.snapshot().board.workspaces.len(), 2);
    }
    #[test]
    fn tab_admission_and_persistence_keep_identity_and_selection_consistent() {
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex,
        )
        .unwrap();
        store.fail.store(true, Ordering::SeqCst);
        assert!(service.add_tab(PRIMARY, None).is_err());
        assert_eq!(service.get(PRIMARY).unwrap().tabs.len(), 1);
        assert_eq!(
            service.get(PRIMARY).unwrap().active.as_deref(),
            Some(PRIMARY)
        );
        store.fail.store(false, Ordering::SeqCst);
        let sibling = service.add_tab(PRIMARY, None).unwrap();
        assert_ne!(sibling.id, PRIMARY);
        let mut invalid = store.value.lock().unwrap().clone().unwrap();
        invalid.board.workspaces[0].tabs[1].id = PRIMARY.into();
        *store.value.lock().unwrap() = Some(invalid);
        assert!(Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Codex
        )
        .is_err());
        service.focus_tab(PRIMARY, PRIMARY).unwrap();
        service.close_tab(PRIMARY, PRIMARY).unwrap();
        assert_eq!(
            service.get(PRIMARY).unwrap().active.as_deref(),
            Some(sibling.id.as_str())
        );
        let restored =
            Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Codex).unwrap();
        assert_eq!(restored.get(PRIMARY).unwrap().tabs[0].id, sibling.id);
    }
    #[test]
    fn creation_selection_and_stage_preserve_independent_sessions() {
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Claude,
        )
        .unwrap();
        let created = service.create("Second", "/project").unwrap();
        assert_eq!(created.board.projects.len(), 1);
        let second = created.board.workspaces[1].id.clone();
        assert_ne!(
            created.board.workspaces[0].tabs[0].id,
            created.board.workspaces[1].tabs[0].id
        );
        service.select(&second).unwrap();
        service.stage(&second, "Feito").unwrap();
        assert!(matches!(
            service.get(&second).unwrap().tabs[0].status,
            Status::Desligada
        ));
        let restored = Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Claude)
            .unwrap()
            .snapshot();
        assert_eq!(restored.active, second);
        assert_eq!(restored.board.workspaces[0].stage, "Preparando");
        assert_eq!(restored.board.workspaces[1].stage, "Feito");
    }
    #[test]
    fn invalid_input_and_failed_save_do_not_change_the_catalog() {
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Claude,
        )
        .unwrap();
        assert!(service.create("", "/project").is_err());
        assert!(service.create("Missing", "/missing").is_err());
        assert!(service.select("../../escape").is_err());
        assert!(service.stage(PRIMARY, "unknown").is_err());
        store.fail.store(true, Ordering::SeqCst);
        assert!(service.create("Unsaved", "/project").is_err());
        assert_eq!(service.snapshot().board.workspaces.len(), 1);
        assert_eq!(service.snapshot().active, PRIMARY);
    }
    #[test]
    fn worktree_preparation_is_injected_and_failed_catalog_save_preserves_recovery_path() {
        struct Worktrees(std::sync::atomic::AtomicUsize);
        impl WorkspaceWorktrees for Worktrees {
            fn prepare(
                &self,
                id: &str,
                request: &WorktreeRequest,
            ) -> Result<PreparedWorktree, String> {
                assert!(valid_id(id));
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(PreparedWorktree {
                    project: Folders.inspect(&request.path)?,
                    path: "/isolated/checkout".into(),
                    branch: request.branch.clone(),
                    base: request.base.clone(),
                })
            }
        }
        let store = Arc::new(Memory::default());
        let mut service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Claude,
        )
        .unwrap();
        let port = Worktrees(std::sync::atomic::AtomicUsize::new(0));
        let mut request = WorktreeRequest {
            title: " ".into(),
            path: "/project".into(),
            branch: "feature/test".into(),
            base: "main".into(),
        };
        assert!(service.create_worktree(&request, &port).is_err());
        assert_eq!(port.0.load(Ordering::SeqCst), 0);
        request.title = " Isolated ".into();
        let created = service.create_worktree(&request, &port).unwrap();
        assert_eq!(created.active, PRIMARY);
        assert_eq!(created.board.projects.len(), 1);
        let workspace = &created.board.workspaces[1];
        assert_eq!(workspace.title, "Isolated");
        assert_eq!(workspace.repo, "/project");
        assert_eq!(workspace.worktree, "/isolated/checkout");
        assert_eq!(workspace.branch, "feature/test");
        assert_eq!(workspace.repos[0].base, "main");
        store.fail.store(true, Ordering::SeqCst);
        let error = service.create_worktree(&request, &port).err().unwrap();
        assert!(error.starts_with("workspace_worktree_unsaved: /isolated/checkout"));
        assert_eq!(service.snapshot().board.workspaces.len(), 2);
        assert_eq!(port.0.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn corrupt_catalog_cannot_supply_filesystem_identifiers() {
        let store = Arc::new(Memory::default());
        let service = Workspaces::open(
            store.clone(),
            Arc::new(Folders),
            "/project",
            ProviderId::Claude,
        )
        .unwrap();
        let mut corrupt = service.snapshot();
        corrupt.board.workspaces[0].id = "../outside".into();
        store.save(&corrupt).unwrap();
        assert!(
            Workspaces::open(store, Arc::new(Folders), "/project", ProviderId::Claude).is_err()
        );
    }
}
