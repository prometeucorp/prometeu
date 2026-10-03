//! Existing desktop command/event contracts over injected runtime contexts.
use crate::{host::Host, RuntimeEvents};
use prometeu_core::{
    board::{Board, Choice, Status},
    conversation::work::Work,
    lock::lock,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

struct Observation {
    status: Status,
    ready: bool,
    generation: String,
    work: Work,
    requests: HashSet<String>,
}
impl Default for Observation {
    fn default() -> Self {
        Self {
            status: Status::Desligada,
            ready: false,
            generation: String::new(),
            work: Work::default(),
            requests: HashSet::new(),
        }
    }
}
pub struct ApplicationEvents {
    enabled: AtomicBool,
    language: Mutex<String>,
    looking: Mutex<Option<String>>,
    observations: Mutex<HashMap<String, Observation>>,
    board: Mutex<Board>,
    output: Arc<dyn RuntimeEvents>,
}
impl ApplicationEvents {
    pub(crate) fn pick(&self, pt: &str, en: &str) -> String {
        match lock(&self.language).starts_with("en") {
            true => en.to_owned(),
            false => pt.to_owned(),
        }
    }
    pub fn new(output: Arc<dyn RuntimeEvents>) -> Self {
        Self {
            enabled: AtomicBool::new(false),
            language: Mutex::new(String::new()),
            looking: Mutex::new(None),
            observations: Mutex::new(HashMap::new()),
            board: Mutex::new(Board::default()),
            output,
        }
    }
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }
    pub fn board(&self, board: Board) -> Board {
        let observations = lock(&self.observations);
        self.overlay(board, &observations)
    }
    fn overlay(&self, board: Board, observations: &HashMap<String, Observation>) -> Board {
        let mut saved = lock(&self.board);
        *saved = board;
        for workspace in &mut saved.workspaces {
            for tab in &mut workspace.tabs {
                tab.status = observations
                    .get(&tab.id)
                    .map_or(Status::Desligada, |o| o.status);
            }
        }
        saved.clone()
    }
    fn emit(&self, name: &str, payload: Value) -> Result<(), String> {
        self.output
            .publish(json!({"v":1,"application":{"name":name,"payload":payload}}))
    }
    pub fn publish_board(&self, board: Board) -> Result<(), String> {
        let observations = lock(&self.observations);
        let board = self.overlay(board, &observations);
        self.emit(
            "board",
            serde_json::to_value(board).map_err(|e| e.to_string())?,
        )
    }
    pub fn observe(&self, session: &str, frame: &Value) -> Result<(), String> {
        let Some(event) = frame.get("event") else {
            if frame["lifecycle"] == "exited" {
                let mut observations = lock(&self.observations);
                let observed = observations.entry(session.into()).or_default();
                if frame["generation"].as_str() == Some(observed.generation.as_str()) {
                    observed.status = Status::Desligada;
                    if self.enabled.load(Ordering::Acquire) {
                        self.update_status(session, observed.status)?;
                        self.emit("chat-closed", json!(session))?;
                    }
                }
            }
            return Ok(());
        };
        let mut observations = lock(&self.observations);
        let observed = observations.entry(session.into()).or_default();
        let generation = frame["generation"].as_str().unwrap_or_default();
        if observed.generation != generation {
            *observed = Observation {
                generation: generation.into(),
                ..Default::default()
            };
        }
        let settled = observed.work.observe(event);
        match event["type"].as_str() {
            Some("session.identity") if !observed.ready => {
                observed.ready = true;
                observed.status = Status::Pronta;
            }
            Some("user.message" | "assistant.started" | "assistant.block") => {
                observed.status = Status::Rodando
            }
            Some("request.opened") => {
                observed
                    .requests
                    .insert(event["requestId"].as_str().unwrap_or_default().into());
                observed.status = Status::Querendo;
            }
            Some("request.closed") => {
                observed
                    .requests
                    .remove(event["requestId"].as_str().unwrap_or_default());
                observed.status = Status::Rodando;
            }
            Some("turn.completed" | "background.changed") if settled => {
                observed.status = Status::Pronta
            }
            _ => {}
        }
        if !observed.requests.is_empty() {
            observed.status = Status::Querendo;
        }
        if self.enabled.load(Ordering::Acquire) {
            self.emit("chat", json!([session, format!("{event}\n"), frame["seq"]]))?;
            self.update_status(session, observed.status)?;
        }
        Ok(())
    }
    fn update_status(&self, session: &str, status: Status) -> Result<(), String> {
        let mut board = lock(&self.board);
        let Some(tab) = board
            .workspaces
            .iter_mut()
            .flat_map(|w| &mut w.tabs)
            .find(|t| t.id == session)
        else {
            return Ok(());
        };
        if tab.status == status {
            return Ok(());
        }
        tab.status = status;
        self.emit(
            "board",
            serde_json::to_value(&*board).map_err(|e| e.to_string())?,
        )
    }
}

pub(crate) struct DockEvents {
    pub key: String,
    pub application: Arc<ApplicationEvents>,
}
impl RuntimeEvents for DockEvents {
    fn publish(&self, frame: Value) -> Result<(), String> {
        let terminal = &frame["terminal"];
        match terminal["kind"].as_str() {
            Some("output") => self
                .application
                .emit("pty", json!([self.key, terminal["data"], terminal["seq"]])),
            Some("closed") => self
                .application
                .emit("pty-closed", json!([self.key, terminal["code"]])),
            _ => Ok(()),
        }
    }
}
pub(crate) fn dock_key(id: &str, kind: &str) -> Result<String, String> {
    let valid = matches!(kind, "terminal" | "setup" | "run")
        || kind
            .strip_prefix("terminal-")
            .is_some_and(|n| n.parse::<u32>().is_ok_and(|n| (2..=32).contains(&n)));
    if !valid {
        return Err("application_dock_unsupported".into());
    }
    Ok(format!("{id}:{kind}"))
}
#[derive(Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
pub(crate) enum Request {
    SetWorkspaceMcp(Value),
    SetWorkspacePlugins(Value),
    SetWorkspaceSkills(Value),
    SetToolsGlobal(Value),
    WorkspaceTools {
        id: String,
        agent: Option<prometeu_core::board::ProviderId>,
    },
    ProjectTools {
        id: String,
    },
    ProjectToolsTrust {
        id: String,
        hash: String,
        approved: bool,
    },
    McpInherited {
        id: String,
        agent: Option<prometeu_core::board::ProviderId>,
    },
    McpFound,
    ApplicationOperationStart {
        command: String,
        args: Value,
    },
    ApplicationOperationPoll {
        job: String,
    },
    McpOperationStart {
        operation: crate::mcp::Operation,
    },
    McpOperationPoll {
        job: String,
    },
    McpAuthCancel {
        state: String,
    },
    McpSave {
        server: prometeu_tools::mcp::Server,
        revision: Option<u64>,
    },
    McpRemove {
        id: String,
    },
    RenameWorkspace {
        id: String,
        title: String,
    },
    RenameTab {
        workspace: String,
        tab: String,
        title: String,
    },
    PinWorkspace {
        id: String,
        pinned: bool,
    },
    SetUnread {
        id: String,
        unread: bool,
    },
    ArchiveWorkspace {
        id: String,
        archived: bool,
    },
    FinishWorkspace {
        id: String,
    },
    RemoveWorkspace {
        id: String,
    },
    SetTabChoice {
        id: String,
        tab: String,
        choice: Choice,
    },
    CleanupList,
    CleanupWorktree {
        id: String,
        force: bool,
    },
    CatalogState,
    SkillHub,
    PluginHub,
    PluginLook {
        source: String,
    },
    PluginSave {
        plugin: prometeu_tools::packages::Plugin,
        revision: Option<u64>,
    },
    PluginInstall {
        source: String,
    },
    PluginUpdate {
        id: String,
    },
    PluginRemove {
        id: String,
    },
    PluginScrap {
        dir: String,
    },
    McpHub,
    SkillSave {
        skill: prometeu_tools::skills::Skill,
        revision: Option<u64>,
    },
    SkillRemove {
        id: String,
    },
    LoadBoard,
    AddProject {
        path: String,
    },
    RemoveProject {
        id: String,
    },
    WorkspaceGitStatus {
        id: String,
    },
    WorkspaceGitDiff {
        id: String,
        repo: usize,
        scope: prometeu_core::git::DiffScope,
        path: Option<String>,
        reference: Option<String>,
    },
    WorkspaceGitAction {
        id: String,
        repo: usize,
        operation: prometeu_core::git::GitAction,
        paths: Vec<String>,
        message: Option<String>,
        expected: Option<String>,
        remote: Option<String>,
    },
    WorkspaceGitHistory {
        id: String,
        repo: usize,
    },
    WorkspaceGitBranches {
        id: String,
        repo: usize,
    },
    WorkspaceGitConflict {
        id: String,
        repo: usize,
        path: String,
    },
    WorkspaceGitResolve {
        id: String,
        repo: usize,
        path: String,
        was: String,
        text: String,
    },
    FileBase {
        id: String,
        rel: String,
    },
    ReorderProjects {
        ids: Vec<String>,
    },
    TreeGitStatus {
        id: String,
    },
    TreeRestore {
        id: String,
        rel: String,
    },
    CreateWorkspace {
        draft: Box<prometeu_core::workspace_draft::Draft>,
        cols: u16,
        rows: u16,
    },
    Agents,
    AgentModels {
        agent: prometeu_core::board::ProviderId,
    },
    Accounts,
    AccountSelect {
        id: String,
    },
    AccountRemove {
        id: String,
    },
    AccountLogin {
        provider: prometeu_core::board::ProviderId,
        id: Option<String>,
        method: Option<String>,
    },
    WorkspaceScripts {
        id: String,
    },
    WorkspaceBranch {
        id: String,
    },
    ListBranches {
        project: String,
    },
    OpenDock {
        id: String,
        kind: String,
        name: Option<String>,
        cols: u16,
        rows: u16,
    },
    CloseDock {
        id: String,
        kind: String,
    },
    DockState {
        id: String,
    },
    PtyBuffer {
        session: String,
        #[serde(default)]
        snapshot: bool,
    },
    PtyWrite {
        session: String,
        data: String,
    },
    PtyResize {
        session: String,
        cols: u16,
        rows: u16,
    },
    ListDir {
        id: String,
        rel: String,
    },
    ReadFile {
        id: String,
        rel: String,
    },
    ReadBytes {
        id: String,
        rel: String,
        #[serde(default)]
        offset: u64,
        stamp: Option<String>,
    },
    RevealPath {
        id: String,
        rel: String,
    },
    FileStamp {
        id: String,
        rel: String,
    },
    WriteFile {
        id: String,
        rel: String,
        text: String,
        was: String,
    },
    CreatePath {
        id: String,
        rel: String,
        dir: bool,
    },
    RenamePath {
        id: String,
        from: String,
        to: String,
    },
    TrashPath {
        id: String,
        rel: String,
    },
    FindPaths {
        id: String,
        query: String,
        recent: Vec<String>,
        files: Option<bool>,
    },
    NewTab {
        workspace: String,
        prompt: String,
        choice: Option<Choice>,
    },
    CloseTab {
        workspace: String,
        tab: String,
    },
    SetLang {
        lang: String,
    },
    LookAt {
        id: Option<String>,
    },
    ChatSnapshot {
        session: String,
    },
    ChatSend {
        session: String,
        text: String,
    },
    ChatControl {
        session: String,
        frame: Value,
    },
    SetStage {
        id: String,
        stage: String,
    },
    FocusTab {
        workspace: String,
        tab: String,
    },
}
impl Host {
    pub(crate) fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
        let request: Request = serde_json::from_value(json!({"command":command,"args":args}))
            .map_err(|_| format!("application_command_unsupported: {command}"))?;
        self.application_events.enable();
        use prometeu_core::workspace_lifecycle::Change;
        use prometeu_core::workspace_tools::Axis;
        match request {
            Request::ApplicationOperationStart { command, args } => {
                self.start_operation(command, args)
            }
            Request::ApplicationOperationPoll { job } => {
                self.poll_operations();
                self.operations.poll(&job)
            }
            Request::SetWorkspaceMcp(body) => self.set_workspace_tools(body, Axis::Mcp, "mcp"),
            Request::SetWorkspacePlugins(body) => {
                self.set_workspace_tools(body, Axis::Plugins, "plugins")
            }
            Request::SetWorkspaceSkills(body) => {
                self.set_workspace_tools(body, Axis::Skills, "skills")
            }
            Request::SetToolsGlobal(body) => {
                let catalog = self.catalog.global_tools(&body)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::WorkspaceTools { id, agent } => {
                let board = self.catalog.snapshot().board;
                let ws = self.catalog.get(&id)?;
                if agent.is_some_and(|a| a != ws.agent) {
                    return Err("workspace_provider_unsupported".into());
                }
                serde_json::to_value(self.services.tool_selection.effective(&board, ws))
                    .map_err(|e| e.to_string())
            }
            Request::ProjectTools { id } => serde_json::to_value(
                self.services
                    .tool_selection
                    .project(&self.catalog.snapshot().board, &id),
            )
            .map_err(|e| e.to_string()),
            Request::ProjectToolsTrust { id, hash, approved } => {
                let declaration = self
                    .services
                    .tool_selection
                    .declaration(&self.catalog.snapshot().board, &id)
                    .ok_or_else(|| prometeu_core::error::code("err.tools.changed"))?;
                let at = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let catalog = self.catalog.tool_trust(declaration, &hash, approved, at)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::McpInherited { id, agent } => {
                let ws = self.catalog.get(&id)?;
                if agent.is_some_and(|a| a != ws.agent) {
                    return Err("workspace_provider_unsupported".into());
                }
                // The registered Codex adapter has no Claude inherited-MCP base (ADR 0047).
                Ok(json!([]))
            }
            Request::McpFound => serde_json::to_value(prometeu_tools::mcp_discovery::found(
                &self.services.resources.mcp_discovery,
                &self.services.resources.mcp.load(),
            ))
            .map_err(|e| e.to_string()),
            Request::McpOperationStart { operation } => self.services.resources.mcp_jobs.start(
                operation,
                self.services.resources.auth.clone(),
                lock(&self.application_events.language).clone(),
            ),
            Request::McpOperationPoll { job } => self.services.resources.mcp_jobs.poll(&job),
            Request::McpAuthCancel { state } => {
                self.services.resources.auth.cancel(&state);
                Ok(Value::Null)
            }
            Request::McpSave { server, revision } => {
                if revision.is_some() {
                    return Err(prometeu_core::error::code("err.catalog.disconnected"));
                }
                serde_json::to_value(self.services.resources.mcp.save(server)?)
                    .map_err(|e| e.to_string())
            }
            Request::McpRemove { id } => {
                serde_json::to_value(self.services.resources.mcp.remove(&id)?)
                    .map_err(|e| e.to_string())
            }
            Request::RenameWorkspace { id, title } => self
                .change_workspace(&id, Change::Rename(title))
                .map(|()| Value::Null),
            Request::RenameTab {
                workspace,
                tab,
                title,
            } => self
                .change_workspace(&workspace, Change::RenameTab { tab, title })
                .map(|()| Value::Null),
            Request::PinWorkspace { id, pinned } => self
                .change_workspace(&id, Change::Pin(pinned))
                .map(|()| Value::Null),
            Request::SetUnread { id, unread } => self
                .change_workspace(&id, Change::Unread(unread))
                .map(|()| Value::Null),
            Request::ArchiveWorkspace { id, archived } => self
                .archive_workspace(&id, archived, false)
                .map(|()| Value::Null),
            Request::FinishWorkspace { id } => self
                .archive_workspace(&id, true, true)
                .map(|()| Value::Null),
            Request::RemoveWorkspace { id } => self.remove_workspace(&id).map(|()| Value::Null),
            Request::SetTabChoice { id, tab, choice } => {
                self.retune_tab(&id, &tab, choice).map(|()| Value::Null)
            }
            Request::CleanupList => {
                serde_json::to_value(self.cleanup_list()).map_err(|e| e.to_string())
            }
            Request::CleanupWorktree { id, force } => {
                self.cleanup_workspace(&id, force).map(|()| Value::Null)
            }
            Request::CatalogState => serde_json::to_value(self.services.resources.catalog.state()?)
                .map_err(|e| e.to_string()),
            Request::PluginLook { source } => {
                serde_json::to_value(self.services.resources.plugins.look(source)?)
                    .map_err(|e| e.to_string())
            }
            Request::PluginSave { plugin, revision } => {
                if revision.is_some() {
                    return Err(prometeu_core::error::code("err.catalog.disconnected"));
                }
                serde_json::to_value(self.services.resources.plugins.save(plugin)?)
                    .map_err(|e| e.to_string())
            }
            Request::PluginInstall { source } => {
                serde_json::to_value(self.services.resources.plugins.install(source)?)
                    .map_err(|e| e.to_string())
            }
            Request::PluginUpdate { id } => {
                serde_json::to_value(self.services.resources.plugins.update(id)?)
                    .map_err(|e| e.to_string())
            }
            Request::PluginRemove { id } => {
                serde_json::to_value(self.services.resources.plugins.remove(&id)?)
                    .map_err(|e| e.to_string())
            }
            Request::PluginScrap { dir } => {
                self.services.resources.plugins.scrap(dir);
                Ok(Value::Null)
            }
            Request::SkillHub => serde_json::to_value(self.services.resources.skills.load())
                .map_err(|e| e.to_string()),
            Request::PluginHub => serde_json::to_value(self.services.resources.packages.load())
                .map_err(|e| e.to_string()),
            Request::McpHub => {
                serde_json::to_value(self.services.resources.mcp.load()).map_err(|e| e.to_string())
            }
            Request::SkillSave { skill, revision } => {
                if revision.is_some() {
                    return Err(prometeu_core::error::code("err.catalog.disconnected"));
                }
                serde_json::to_value(self.services.resources.skills.save(skill)?)
                    .map_err(|e| e.to_string())
            }
            Request::SkillRemove { id } => {
                serde_json::to_value(self.services.resources.skills.remove(&id)?)
                    .map_err(|e| e.to_string())
            }
            Request::AddProject { path } => {
                let project = self.catalog.add_project(&path)?;
                self.application_events
                    .publish_board(self.catalog.snapshot().board)?;
                serde_json::to_value(project).map_err(|e| e.to_string())
            }
            Request::ReorderProjects { ids } => {
                let catalog = self.catalog.reorder_projects(&ids)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::RemoveProject { id } => {
                let catalog = self.catalog.remove_project(&id)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            request @ (Request::WorkspaceGitStatus { .. }
            | Request::WorkspaceGitDiff { .. }
            | Request::WorkspaceGitAction { .. }
            | Request::WorkspaceGitHistory { .. }
            | Request::WorkspaceGitBranches { .. }
            | Request::WorkspaceGitConflict { .. }
            | Request::WorkspaceGitResolve { .. }
            | Request::FileBase { .. }
            | Request::TreeGitStatus { .. }
            | Request::TreeRestore { .. }
            | Request::CreateWorkspace { .. }
            | Request::WorkspaceBranch { .. }
            | Request::ListBranches { .. }
            | Request::AgentModels { .. }
            | Request::Accounts) => {
                let operation = self.prepare_operation(request)?;
                operation
                    .run()
                    .and_then(|outcome| self.finish_operation(outcome))
            }
            Request::Agents => {
                serde_json::to_value(self.services.discovery.agents()).map_err(|e| e.to_string())
            }
            Request::AccountSelect { id } => {
                let value = self.services.discovery.select(&id)?;
                self.application_events.emit("accounts", value.clone())?;
                Ok(value)
            }
            Request::AccountRemove { id } => {
                let value = self.services.discovery.remove(&id)?;
                self.application_events.emit("accounts", value.clone())?;
                Ok(value)
            }
            Request::AccountLogin {
                provider,
                id,
                method,
            } => {
                if id
                    .as_deref()
                    .is_some_and(|id| id != prometeu_core::accounts::key(provider))
                {
                    return Err(prometeu_core::error::code("err.account.external"));
                }
                let value = self
                    .services
                    .discovery
                    .attach(provider, method.as_deref().unwrap_or("external"))?;
                self.application_events.emit("accounts", value.clone())?;
                Ok(value)
            }
            Request::NewTab {
                workspace,
                prompt,
                choice,
            } => {
                self.admit_application_input(&workspace)?;
                self.services
                    .discovery
                    .accounts
                    .active(self.catalog.get(&workspace)?.agent)?;
                if let Some(choice) = &choice {
                    self.services
                        .discovery
                        .validate_model(&choice.clone().into())?;
                }
                let tab = self.catalog.add_tab(&workspace, choice)?;
                self.application_events
                    .publish_board(self.catalog.snapshot().board)?;
                let runtime = &mut self.session_context(&tab.id)?.runtime;
                match prompt.trim().is_empty() {
                    true => {
                        runtime.start()?;
                    }
                    false => runtime.send(&prompt)?,
                }
                serde_json::to_value(tab).map_err(|e| e.to_string())
            }
            Request::CloseTab { workspace, tab } => {
                if !self
                    .catalog
                    .get(&workspace)?
                    .tabs
                    .iter()
                    .any(|t| t.id == tab)
                {
                    return Err("workspace_not_found".into());
                }
                self.pending_launches.remove(&tab);
                if let Some(context) = self.contexts.get_mut(&tab) {
                    context.runtime.stop()?;
                }
                let catalog = self.catalog.close_tab(&workspace, &tab)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::ListDir { id, rel } => {
                serde_json::to_value(self.services.files.list(&self.file_root(&id)?, &rel))
                    .map_err(|e| e.to_string())
            }
            Request::ReadFile { id, rel } => self
                .services
                .files
                .read(&self.file_root(&id)?, &rel)
                .map(Value::String),
            Request::ReadBytes {
                id,
                rel,
                offset,
                stamp,
            } => serde_json::to_value(self.services.files.block(
                &self.file_root(&id)?,
                &rel,
                offset,
                stamp.as_deref(),
            )?)
            .map_err(|error| error.to_string()),
            Request::RevealPath { id, rel } => {
                serde_json::to_value(self.services.files.location(&self.file_root(&id)?, &rel)?)
                    .map_err(|error| error.to_string())
            }
            Request::FileStamp { id, rel } => self
                .services
                .files
                .stamp(&self.file_root(&id)?, &rel)
                .map(Value::String),
            Request::WriteFile { id, rel, text, was } => {
                let root = self.file_root(&id)?;
                let result = self.services.files.write(&root, &rel, &text, &was);
                self.services.git.invalidate(&root);
                result.map(|()| Value::Null)
            }
            Request::CreatePath { id, rel, dir } => {
                let root = self.file_root(&id)?;
                let result = self.services.project_entries.create(&root, &rel, dir);
                self.services.git.invalidate(&root);
                result.map(|()| Value::Null)
            }
            Request::RenamePath { id, from, to } => {
                let root = self.file_root(&id)?;
                let result = self.services.project_entries.rename(&root, &from, &to);
                self.services.git.invalidate(&root);
                result.map(|()| Value::Null)
            }
            Request::TrashPath { id, rel } => {
                let root = self.file_root(&id)?;
                let result = self.services.project_entries.trash(&root, &rel);
                self.services.git.invalidate(&root);
                result.map(|()| Value::Null)
            }
            Request::FindPaths {
                id,
                query,
                recent,
                files,
            } => {
                let entries = self
                    .git_tree(&id)
                    .map(|(root, repositories)| {
                        self.services.project_search.find(
                            &root,
                            &repositories,
                            &query,
                            &recent,
                            files.unwrap_or(false),
                        )
                    })
                    .unwrap_or_default();
                serde_json::to_value(entries).map_err(|e| e.to_string())
            }
            Request::OpenDock {
                id,
                kind,
                name,
                cols,
                rows,
            } => self
                .open_application_dock(&id, &kind, name.as_deref(), cols, rows)
                .map(Value::String),
            Request::CloseDock { id, kind } => {
                let root = self.file_root(&id)?;
                if kind == "setup" {
                    self.check_operation_paths(&[root.to_string_lossy().into()])?;
                }
                let key = dock_key(&id, &kind)?;
                if let Some(terminal) = self.docks.get_mut(&key) {
                    let current = terminal.current()?;
                    if let Some(id) = current["id"].as_str() {
                        terminal.stop(id)?;
                    }
                }
                Ok(Value::Null)
            }
            Request::DockState { id } => {
                self.file_root(&id)?;
                let prefix = format!("{id}:");
                let mut docks = Vec::new();
                for (key, terminal) in &self.docks {
                    if let Some(kind) = key.strip_prefix(&prefix) {
                        let current = terminal.current()?;
                        docks.push(json!({"kind":kind,"alive":current["running"] == true}));
                    }
                }
                Ok(json!(docks))
            }
            Request::PtyBuffer { session, snapshot } => {
                let current = self
                    .docks
                    .get(&session)
                    .map_or(Ok(Value::Null), |dock| dock.current())?;
                match (snapshot, current.is_null()) {
                    (true, _) => Ok(current),
                    (false, false) => Ok(current["data"].clone()),
                    (false, true) => Err("terminal_not_found".into()),
                }
            }
            Request::PtyWrite { session, data } => {
                let terminal = self.docks.get_mut(&session).ok_or("terminal_not_found")?;
                let id = terminal.current()?["id"]
                    .as_str()
                    .ok_or("terminal_not_found")?
                    .to_owned();
                terminal.write(&id, data.as_bytes()).map(|()| Value::Null)
            }
            Request::PtyResize {
                session,
                cols,
                rows,
            } => {
                let terminal = self.docks.get_mut(&session).ok_or("terminal_not_found")?;
                let id = terminal.current()?["id"]
                    .as_str()
                    .ok_or("terminal_not_found")?
                    .to_owned();
                terminal.resize(&id, cols, rows).map(|()| Value::Null)
            }
            Request::WorkspaceScripts { id } => {
                let workspace = match self.catalog.get(&id) {
                    Ok(workspace) => workspace,
                    Err(_) => {
                        self.file_root(&id)?;
                        let mut value =
                            serde_json::to_value(prometeu_files::settings::Scripts::default())
                                .map_err(|e| e.to_string())?;
                        value["port"] = Value::Null;
                        return Ok(value);
                    }
                };
                let repository = workspace.repos.first().ok_or("workspace_catalog_invalid")?;
                let scripts = self.services.settings.read(
                    std::path::Path::new(&workspace.worktree),
                    std::path::Path::new(&repository.path),
                );
                let mut value = serde_json::to_value(scripts).map_err(|e| e.to_string())?;
                value["port"] = json!(workspace.port);
                Ok(value)
            }
            Request::LoadBoard => {
                serde_json::to_value(self.application_events.board(self.catalog.snapshot().board))
                    .map_err(|e| e.to_string())
            }
            Request::ChatSnapshot { session } => {
                Ok(self.session_context(&session)?.runtime.snapshot()?["snapshot"].clone())
            }
            Request::ChatSend { session, text } => {
                let board = self.catalog.snapshot().board;
                let workspace = board
                    .workspaces
                    .iter()
                    .find(|w| w.tabs.iter().any(|t| t.id == session))
                    .ok_or("workspace_not_found")?;
                self.admit_application_input(&workspace.id)?;
                self.services.discovery.accounts.active(workspace.agent)?;
                self.session_context(&session)?.runtime.send(&text)?;
                Ok(Value::Null)
            }
            Request::ChatControl { session, frame } => {
                if frame["type"] == "message.send" {
                    let workspace = self
                        .catalog
                        .snapshot()
                        .board
                        .workspaces
                        .into_iter()
                        .find(|w| w.tabs.iter().any(|t| t.id == session))
                        .ok_or("workspace_not_found")?;
                    self.admit_application_input(&workspace.id)?;
                    self.services.discovery.accounts.active(workspace.agent)?;
                }
                self.session_context(&session)?.runtime.command(&frame)?;
                Ok(Value::Null)
            }
            Request::SetStage { id, stage } => {
                let catalog = self.catalog.stage(&id, &stage)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::FocusTab { workspace, tab } => {
                let catalog = self.catalog.focus_tab(&workspace, &tab)?;
                self.application_events.publish_board(catalog.board)?;
                Ok(Value::Null)
            }
            Request::SetLang { lang } => {
                *lock(&self.application_events.language) = lang;
                Ok(Value::Null)
            }
            Request::LookAt { id } => {
                if let Some(id) = &id {
                    let catalog = self.catalog.mark_read(id)?;
                    self.application_events.publish_board(catalog.board)?;
                }
                *lock(&self.application_events.looking) = id;
                Ok(Value::Null)
            }
        }
    }
    fn set_workspace_tools(
        &mut self,
        body: Value,
        axis: prometeu_core::workspace_tools::Axis,
        name: &str,
    ) -> Result<Value, String> {
        let id = body["id"]
            .as_str()
            .ok_or_else(|| prometeu_core::error::code("err.tools.badPayload"))?;
        let catalog = self.catalog.workspace_tools(id, axis, &body, name)?;
        self.application_events.publish_board(catalog.board)?;
        Ok(Value::Null)
    }
    pub(crate) fn git_tree(
        &self,
        id: &str,
    ) -> Result<(std::path::PathBuf, Vec<std::path::PathBuf>), String> {
        let root = self.file_root(id)?;
        let board = self.catalog.snapshot().board;
        let repos = prometeu_core::git::workspace_repos(&board, id)
            .map(|repos| {
                repos
                    .iter()
                    .map(|r| std::path::PathBuf::from(&r.worktree))
                    .collect()
            })
            .unwrap_or_else(|_| vec![root.clone()]);
        Ok((root, repos))
    }
    pub(crate) fn file_root(&self, id: &str) -> Result<std::path::PathBuf, String> {
        let board = self.catalog.snapshot().board;
        board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.worktree.clone())
            .or_else(|| {
                board
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.path.clone())
            })
            .map(std::path::PathBuf::from)
            .ok_or_else(|| prometeu_core::error::code("err.session.noWorkspace"))
    }
    pub(crate) fn session_context(
        &mut self,
        session: &str,
    ) -> Result<&mut crate::host::Context, String> {
        let workspace = self
            .catalog
            .snapshot()
            .board
            .workspaces
            .into_iter()
            .find(|w| w.tabs.iter().any(|t| t.id == session))
            .ok_or("workspace_not_found")?;
        if !self.contexts.contains_key(session) {
            self.contexts
                .insert(session.into(), self.factory.open(&workspace, session)?);
        }
        self.contexts
            .get_mut(session)
            .ok_or("workspace_not_found".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Events(Mutex<Vec<Value>>);
    impl RuntimeEvents for Events {
        fn publish(&self, frame: Value) -> Result<(), String> {
            lock(&self.0).push(frame);
            Ok(())
        }
    }
    #[test]
    fn requests_background_settlement_and_retired_exits_preserve_observed_status() {
        let output = Arc::new(Events(Mutex::new(Vec::new())));
        let events = ApplicationEvents::new(output.clone());
        events.enable();
        let event = |value| {
            events
                .observe(
                    "session",
                    &json!({"event":value,"generation":"new","seq":1}),
                )
                .unwrap()
        };
        let status = || lock(&events.observations)["session"].status;
        event(json!({"type":"commands.updated"}));
        assert!(status() == Status::Desligada);
        event(json!({"type":"session.identity","providerSession":"thread"}));
        assert!(status() == Status::Pronta);
        event(json!({"type":"assistant.started"}));
        event(json!({"type":"commands.updated"}));
        assert!(status() == Status::Rodando);
        event(json!({"type":"request.opened","requestId":"a"}));
        event(json!({"type":"request.opened","requestId":"b"}));
        event(json!({"type":"request.closed","requestId":"a"}));
        assert!(status() == Status::Querendo);
        event(json!({"type":"request.closed","requestId":"b"}));
        event(json!({"type":"background.changed","tasks":[{"id":"child"}]}));
        event(json!({"type":"turn.completed","outcome":"ok"}));
        assert!(status() == Status::Rodando);
        event(json!({"type":"background.changed","tasks":[]}));
        assert!(status() == Status::Pronta);
        events
            .observe("session", &json!({"lifecycle":"exited","generation":"old"}))
            .unwrap();
        assert!(status() == Status::Pronta);
        assert!(lock(&output.0)
            .iter()
            .all(|frame| frame["application"]["name"] != "chat-closed"));
        events
            .observe("session", &json!({"lifecycle":"exited","generation":"new"}))
            .unwrap();
        assert!(status() == Status::Desligada);
        assert_eq!(
            lock(&output.0).last().unwrap()["application"]["name"],
            "chat-closed"
        );
    }
}
