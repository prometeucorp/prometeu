//! Request dispatch shared by disposable and resident transports.
use crate::{terminal::TerminalService, Runtime, RuntimeEvents};
use prometeu_core::{
    board::Workspace,
    lock::lock,
    workspaces::{Catalog, WorkspaceWorktrees, Workspaces, WorktreeRequest, PRIMARY},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::BufRead;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(crate) const MAX_RESPONSE: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u32,
    id: u64,
    action: Action,
}
#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Start,
    Command { frame: Value },
    Stop,
    Snapshot,
    Shutdown,
    Retire,
    Application { command: String, args: Value },
    WorkspaceList,
    WorkspaceWorktree { request: WorktreeRequest },
    WorkspaceCreate { title: String, path: String },
    WorkspaceSelect { id: String },
    WorkspaceStage { id: String, stage: String },
    TerminalCurrent,
    TerminalOpen { cols: u16, rows: u16 },
    TerminalWrite { id: String, data: Vec<u8> },
    TerminalResize { id: String, cols: u16, rows: u16 },
    TerminalAcknowledge { id: String, seq: u64 },
    TerminalSnapshot { id: String },
    TerminalClose { id: String },
}
pub struct Context {
    pub runtime: Runtime,
    pub terminals: TerminalService,
}
pub trait ContextFactory {
    fn retune(
        &self,
        runtime: &mut Runtime,
        workspace: &Workspace,
        session: &str,
    ) -> Result<(), String>;
    fn archive(
        &self,
        workdir: &std::path::Path,
        launch: crate::dock::DockLaunch,
    ) -> Result<(), String>;
    fn shell(
        &self,
        workdir: &std::path::Path,
        launch: crate::dock::DockLaunch,
        output: Arc<dyn RuntimeEvents>,
    ) -> Result<TerminalService, String>;
    fn open(&self, workspace: &Workspace, session: &str) -> Result<Context, String>;
}
pub struct WorkspaceEvents {
    pub id: String,
    pub selected: Arc<Mutex<String>>,
    pub output: Arc<dyn RuntimeEvents>,
    pub application: Arc<crate::application::ApplicationEvents>,
}
impl RuntimeEvents for WorkspaceEvents {
    fn publish(&self, frame: Value) -> Result<(), String> {
        self.application.observe(&self.id, &frame)?;
        // Background execution keeps its transcript and PTY buffer; selecting it obtains snapshots.
        let selected = lock(&self.selected);
        if *selected == self.id {
            self.output.publish(frame)?;
        }
        Ok(())
    }
}
pub struct ApplicationServices {
    pub tool_selection: Arc<crate::tools::Selection>,
    pub cleanup: Arc<dyn prometeu_core::workspace_lifecycle::WorktreeCleanup>,
    pub project_entries: Arc<dyn prometeu_core::files::ProjectEntries<std::path::Path>>,
    pub project_search: Arc<dyn prometeu_core::files::ProjectSearch>,
    pub resources: crate::resources::Resources,
    pub git: Arc<dyn prometeu_core::git::RepositoryGit>,
    pub discovery: Arc<crate::discovery::Discovery>,
    pub references: Arc<dyn prometeu_core::repository::RepositoryReferences>,
    pub files: Arc<dyn prometeu_core::files::ProjectFiles<std::path::Path>>,
    pub settings: Arc<dyn prometeu_files::settings::RepositorySettings>,
    pub preparation: Arc<dyn prometeu_files::scripts::WorkspacePreparation>,
}
pub struct Host {
    pub(crate) catalog: Workspaces,
    pub(crate) services: ApplicationServices,
    pub(crate) worktrees: Arc<dyn WorkspaceWorktrees>,
    pub(crate) contexts: HashMap<String, Context>,
    pub(crate) docks: HashMap<String, TerminalService>,
    pub(crate) operations: crate::jobs::Jobs<crate::operations::Outcome>,
    pub(crate) operation_writes: HashMap<String, Vec<String>>,
    pub(crate) pending_launches: HashMap<String, crate::dock::PendingLaunch>,
    pub(crate) factory: Box<dyn ContextFactory>,
    selected: Arc<Mutex<String>>,
    attached: bool,
    pub(crate) application_events: Arc<crate::application::ApplicationEvents>,
}
impl Host {
    pub fn new(
        catalog: Workspaces,
        services: ApplicationServices,
        application_events: Arc<crate::application::ApplicationEvents>,
        worktrees: Box<dyn WorkspaceWorktrees>,
        primary: Context,
        factory: Box<dyn ContextFactory>,
        selected: Arc<Mutex<String>>,
    ) -> Result<Self, String> {
        application_events.board(catalog.snapshot().board);
        let id = catalog.snapshot().active;
        let mut host = Self {
            catalog,
            services,
            application_events,
            worktrees: worktrees.into(),
            operations: crate::jobs::Jobs::default(),
            operation_writes: HashMap::new(),
            docks: HashMap::new(),
            pending_launches: HashMap::new(),
            contexts: HashMap::from([(PRIMARY.into(), primary)]),
            factory,
            selected,
            attached: false,
        };
        if !id.is_empty() {
            host.select(&id)?;
        }
        Ok(host)
    }
    pub fn attached(&mut self, attached: bool) {
        self.attached = attached;
        self.context().terminals.attached(attached);
    }
    fn context(&mut self) -> &mut Context {
        let id = lock(&self.selected).clone();
        self.contexts
            .get_mut(&id)
            .expect("selected workspace has a context")
    }
    fn select(&mut self, id: &str) -> Result<Catalog, String> {
        let workspace = self.catalog.get(id)?;
        let session = workspace
            .active
            .clone()
            .unwrap_or_else(|| workspace.id.clone());
        if !self.contexts.contains_key(&session) {
            self.contexts
                .insert(session.clone(), self.factory.open(workspace, &session)?);
        }
        let catalog = self.catalog.select(id)?;
        self.context().terminals.attached(false);
        *lock(&self.selected) = session;
        let attached = self.attached;
        self.context().terminals.attached(attached);
        Ok(catalog)
    }
    /// The transport decides whether a successful shutdown ends its listener.
    pub fn request(&mut self, line: &str) -> (Value, bool) {
        let request: Request = match serde_json::from_str(line) {
            Ok(request) => request,
            Err(error) => {
                return (
                    json!({"v":1,"id":null,"error":format!("invalid request: {error}")}),
                    false,
                )
            }
        };
        let mut quit = request.v == 1 && matches!(request.action, Action::Shutdown);
        let result = match (request.v, request.action) {
            (1, Action::Application { command, args }) => self.application(command, args),
            (1, Action::WorkspaceWorktree { request }) => self
                .catalog
                .create_worktree(&request, self.worktrees.as_ref())
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
            (1, Action::WorkspaceList) => {
                serde_json::to_value(self.catalog.snapshot()).map_err(|e| e.to_string())
            }
            (1, Action::WorkspaceCreate { title, path }) => self
                .catalog
                .create(&title, &path)
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
            (1, Action::WorkspaceSelect { id }) => self
                .select(&id)
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
            (1, Action::WorkspaceStage { id, stage }) => self
                .catalog
                .stage(&id, &stage)
                .and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string())),
            (1, Action::Start) => self.context().runtime.start(),
            (1, Action::Command { frame }) => self
                .context()
                .runtime
                .command(&frame)
                .map(|()| json!({"accepted":true})),
            (1, Action::Stop) => self
                .context()
                .runtime
                .stop()
                .map(|()| json!({"stopped":true})),
            (1, Action::Snapshot) => self.context().runtime.snapshot(),
            (1, Action::Shutdown) => self.shutdown().map(|()| json!({"stopped":true})),
            (1, Action::Retire) => self.idle().and_then(|idle| {
                if idle {
                    self.shutdown()?;
                    quit = true;
                }
                Ok(json!({"retired":idle}))
            }),
            (1, Action::TerminalCurrent) => self.context().terminals.current(),
            (1, Action::TerminalOpen { cols, rows }) => self.context().terminals.open(cols, rows),
            (1, Action::TerminalWrite { id, data }) => self
                .context()
                .terminals
                .write(&id, &data)
                .map(|()| json!({"accepted":true})),
            (1, Action::TerminalResize { id, cols, rows }) => self
                .context()
                .terminals
                .resize(&id, cols, rows)
                .map(|()| json!({"resized":true})),
            (1, Action::TerminalAcknowledge { id, seq }) => self
                .context()
                .terminals
                .acknowledge(&id, seq)
                .map(|()| json!({"acknowledged":true})),
            (1, Action::TerminalSnapshot { id }) => self.context().terminals.snapshot(&id),
            (1, Action::TerminalClose { id }) => self
                .context()
                .terminals
                .close(&id)
                .map(|()| json!({"closed":true})),
            _ => Err("unsupported protocol version".into()),
        };
        let shutdown = quit && result.is_ok();
        let response = match result {
            Ok(value) => json!({"v":1,"id":request.id,"result":value}),
            Err(error) => json!({"v":1,"id":request.id,"error":error}),
        };
        // JSON escaping can expand valid text beyond the transport budget. Return an
        // explicit error while retaining the attachment, rather than publishing a lost reply.
        let response = match serde_json::to_vec(&response) {
            Ok(bytes) if bytes.len() < MAX_RESPONSE => response,
            _ => json!({"v":1,"id":request.id,"error":"response exceeds 8 MiB"}),
        };
        (response, shutdown)
    }
    /// Admission runs in the serialized request loop, before any cleanup or replacement.
    fn idle(&self) -> Result<bool, String> {
        if !self.operations.idle()
            || !self.pending_launches.is_empty()
            || !self.services.resources.mcp_jobs.idle()
            || self.services.resources.auth.pending()
        {
            return Ok(false);
        }
        for context in self.contexts.values() {
            if context.runtime.retained() || context.terminals.current()?["running"] == true {
                return Ok(false);
            }
        }
        for terminal in self.docks.values() {
            if terminal.current()?["running"] == true {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.poll_operations();
        self.admit_destructive_operation()?;
        if self.services.resources.mcp_jobs.running() {
            return Err("MCP operation is still running".into());
        }
        self.pending_launches.clear();
        let mut result = Ok(());
        for context in self.contexts.values_mut() {
            let conversation = context.runtime.stop();
            let terminal = context.terminals.shutdown();
            result = result.and(conversation).and(terminal);
        }
        for terminal in self.docks.values_mut() {
            result = result.and(terminal.shutdown());
        }
        result
    }
}
pub fn ready(capabilities: &[&str]) -> Value {
    json!({"v":1,"lifecycle":"ready","provider":"codex","capabilities":capabilities})
}
pub fn read_request(input: &mut impl std::io::BufRead) -> Result<Option<String>, String> {
    let mut line = String::new();
    let n = std::io::Read::take(input, 1024 * 1024 + 1)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if n > 1024 * 1024 || !line.ends_with('\n') {
        return Err("invalid or oversized request".into());
    }
    Ok(Some(line))
}
