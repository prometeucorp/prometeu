//! Admit effects on the host, run native work separately, then commit to the current catalog.
use crate::{
    application::{dock_key, Request},
    dock::PreparedSetup,
    host::Host,
};
use prometeu_core::{board::Workspace, workspaces::PreparedApplication};
use serde_json::{json, Value};
use std::path::Path;

pub(crate) enum Outcome {
    Value(Value),
    Workspace(Box<PreparedApplication>, Option<PreparedSetup>, u16, u16),
    Setup(String, PreparedSetup, u16, u16),
}
pub(crate) struct Operation {
    effect: Box<dyn FnOnce() -> Result<Outcome, String> + Send>,
    writes: Vec<String>,
}
impl Operation {
    pub fn run(self) -> Result<Outcome, String> {
        (self.effect)()
    }
}
impl Host {
    pub(crate) fn prepare_operation(&self, request: Request) -> Result<Operation, String> {
        let board = self.application_events.board(self.catalog.snapshot().board);
        let mut writes = Vec::new();
        match &request {
            Request::WorkspaceGitAction {
                id,
                repo,
                operation,
                ..
            } => {
                prometeu_core::git::admit_mutation(&board, id, operation)?;
                writes.push(
                    prometeu_core::git::repository(&board, id, *repo)?
                        .worktree
                        .clone(),
                );
            }
            Request::WorkspaceGitResolve { id, repo, .. } => {
                writes.push(
                    prometeu_core::git::repository(&board, id, *repo)?
                        .worktree
                        .clone(),
                );
            }
            Request::TreeRestore { id, .. } => {
                writes.push(self.file_root(id)?.to_string_lossy().into())
            }
            Request::OpenDock {
                id,
                kind,
                cols,
                rows,
                ..
            } if kind == "setup" => {
                if *cols == 0 || *rows == 0 {
                    return Err("terminal_size_invalid".into());
                }
                let workspace = self.catalog.get(id)?;
                if workspace.cleaned {
                    return Err(prometeu_core::error::code("err.session.cleaned"));
                }
                writes.push(workspace.worktree.clone());
            }
            Request::AgentModels { .. } | Request::Accounts => {}
            Request::WorkspaceGitStatus { .. }
            | Request::WorkspaceGitDiff { .. }
            | Request::WorkspaceGitHistory { .. }
            | Request::WorkspaceGitBranches { .. }
            | Request::WorkspaceGitConflict { .. }
            | Request::FileBase { .. }
            | Request::TreeGitStatus { .. }
            | Request::WorkspaceBranch { .. }
            | Request::ListBranches { .. }
            | Request::CreateWorkspace { .. } => {}
            _ => return Err("application_operation_unsupported".into()),
        }
        self.check_operation_paths(&writes)?;
        if let Request::CreateWorkspace { draft, cols, rows } = request {
            if cols == 0 || rows == 0 {
                return Err("terminal_size_invalid".into());
            }
            self.services
                .discovery
                .accounts
                .active(draft.launch.agent)?;
            let preparation = self.catalog.plan_application(&draft)?;
            let worktrees = self.worktrees.clone();
            let references = self.services.references.clone();
            let discovery = self.services.discovery.clone();
            let settings = self.services.settings.clone();
            let files = self.services.preparation.clone();
            let events = self.application_events.clone();
            return Ok(Operation {
                writes,
                effect: Box::new(move || {
                    discovery.validate_model(&draft.launch)?;
                    let prepared = preparation.prepare(worktrees.as_ref(), references.as_ref())?;
                    let workspace = prepared.workspace();
                    let declarations =
                        settings.read(Path::new(&workspace.worktree), Path::new(&workspace.repo));
                    let setup = PreparedSetup::prepare(
                        workspace,
                        &declarations,
                        files.as_ref(),
                        |pt, en| events.pick(pt, en),
                    );
                    Ok(Outcome::Workspace(Box::new(prepared), setup, cols, rows))
                }),
            });
        }
        if let Request::OpenDock { id, cols, rows, .. } = request {
            let key = dock_key(&id, "setup")?;
            if self.setup_running(&id)? {
                return Ok(Operation {
                    writes: vec![],
                    effect: Box::new(move || Ok(Outcome::Value(Value::String(key)))),
                });
            }
            let workspace = self.catalog.get(&id)?.clone();
            let settings = self.services.settings.clone();
            let files = self.services.preparation.clone();
            let events = self.application_events.clone();
            return Ok(Operation {
                writes,
                effect: Box::new(move || {
                    let declarations =
                        settings.read(Path::new(&workspace.worktree), Path::new(&workspace.repo));
                    let prepared = PreparedSetup::prepare(
                        &workspace,
                        &declarations,
                        files.as_ref(),
                        |pt, en| events.pick(pt, en),
                    )
                    .ok_or_else(|| {
                        prometeu_core::error::with_args(
                            "err.dock.noScript",
                            &[
                                ("kind", "setup".into()),
                                ("file", prometeu_files::settings::FILES[0].into()),
                            ],
                        )
                    })?;
                    Ok(Outcome::Setup(id, prepared, cols, rows))
                }),
            });
        }
        let tree = match &request {
            Request::FileBase { id, .. }
            | Request::TreeGitStatus { id }
            | Request::TreeRestore { id, .. } => self.git_tree(id).ok(),
            _ => None,
        };
        let root = match &request {
            Request::WorkspaceBranch { id } => Some(self.file_root(id)?),
            _ => None,
        };
        let discovery = self.services.discovery.clone();
        let git = self.services.git.clone();
        let references = self.services.references.clone();
        Ok(Operation {
            writes,
            effect: Box::new(move || {
                let result = match request {
                    Request::AgentModels { agent } => {
                        let catalog = discovery.models(agent).map_err(|error| {
                            format!(
                                "application-error:{}",
                                serde_json::to_string(&error).unwrap()
                            )
                        })?;
                        serde_json::to_value(catalog).map_err(|e| e.to_string())
                    }
                    Request::Accounts => discovery.refresh(),
                    Request::WorkspaceGitStatus { id } => {
                        let repos = prometeu_core::git::workspace_repos(&board, &id)?;
                        serde_json::to_value(git.status(repos)).map_err(|e| e.to_string())
                    }
                    Request::WorkspaceGitDiff {
                        id,
                        repo,
                        scope,
                        path,
                        reference,
                    } => {
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        serde_json::to_value(git.diff(
                            repo,
                            scope,
                            path.as_deref(),
                            reference.as_deref(),
                        )?)
                        .map_err(|e| e.to_string())
                    }
                    Request::WorkspaceGitAction {
                        id,
                        repo,
                        operation,
                        paths,
                        message,
                        expected,
                        remote,
                    } => {
                        prometeu_core::git::admit_mutation(&board, &id, &operation)?;
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        git.action(
                            repo,
                            prometeu_core::git::Mutation {
                                operation,
                                paths: &paths,
                                message: message.as_deref(),
                                expected: expected.as_deref(),
                                remote: remote.as_deref(),
                            },
                        )?;
                        Ok(Value::Null)
                    }
                    Request::WorkspaceGitHistory { id, repo } => {
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        serde_json::to_value(git.history(repo)?).map_err(|e| e.to_string())
                    }
                    Request::WorkspaceGitBranches { id, repo } => {
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        serde_json::to_value(git.branches(repo, &board)?).map_err(|e| e.to_string())
                    }
                    Request::WorkspaceGitConflict { id, repo, path } => {
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        serde_json::to_value(git.conflict(repo, &path)?).map_err(|e| e.to_string())
                    }
                    Request::WorkspaceGitResolve {
                        id,
                        repo,
                        path,
                        was,
                        text,
                    } => {
                        let repo = prometeu_core::git::repository(&board, &id, repo)?;
                        git.resolve(repo, &path, &was, &text)?;
                        Ok(Value::Null)
                    }
                    Request::FileBase { rel, .. } => serde_json::to_value(
                        tree.as_ref()
                            .and_then(|(root, repos)| git.file_base(root, repos, &rel)),
                    )
                    .map_err(|e| e.to_string()),
                    Request::TreeGitStatus { .. } => {
                        let marks = tree
                            .map(|(root, repos)| git.tree(&root, &repos))
                            .unwrap_or_default();
                        serde_json::to_value(marks).map_err(|e| e.to_string())
                    }
                    Request::TreeRestore { rel, .. } => {
                        let (root, repos) = tree.ok_or("workspace_not_found")?;
                        git.restore(&root, &repos, &rel)?;
                        Ok(Value::Null)
                    }
                    Request::WorkspaceBranch { .. } => {
                        let path = root.ok_or("workspace_not_found")?;
                        Ok(json!(
                            references.head(path.to_str().ok_or("workspace_folder_invalid")?)?
                        ))
                    }
                    Request::ListBranches { project } => {
                        serde_json::to_value(references.branches(&project)?)
                            .map_err(|e| e.to_string())
                    }
                    _ => Err("application_operation_unsupported".into()),
                };
                result.map(Outcome::Value)
            }),
        })
    }
    pub(crate) fn start_operation(
        &mut self,
        command: String,
        args: Value,
    ) -> Result<Value, String> {
        self.poll_operations();
        let request = serde_json::from_value(json!({"command": command, "args": args}))
            .map_err(|_| "application_operation_unsupported".to_string())?;
        let operation = self.prepare_operation(request)?;
        let writes = operation.writes.clone();
        let started = self.operations.start(move || operation.run())?;
        self.operation_writes
            .insert(started["job"].as_str().unwrap().into(), writes);
        Ok(started)
    }
    pub fn poll_operations(&mut self) {
        self.services.resources.mcp_jobs.settle();
        for (id, result) in self.operations.take_ready() {
            let result = result.and_then(|outcome| self.finish_operation(outcome));
            self.operation_writes.remove(&id);
            self.operations.complete(&id, result);
        }
    }
    pub(crate) fn finish_operation(&mut self, outcome: Outcome) -> Result<Value, String> {
        match outcome {
            Outcome::Value(value) => Ok(value),
            Outcome::Setup(id, prepared, cols, rows) => self
                .start_prepared_setup(&id, prepared, cols, rows)
                .map(Value::String),
            Outcome::Workspace(prepared, setup, cols, rows) => {
                let workspace = self.catalog.finish_application(*prepared)?;
                self.application_events
                    .publish_board(self.catalog.snapshot().board)?;
                if let Err(error) = self.launch_application(&workspace, setup, cols, rows) {
                    self.catalog.launch_result(&workspace.id, Some(error))?;
                    self.application_events
                        .publish_board(self.catalog.snapshot().board)?;
                }
                serde_json::to_value(self.catalog.get(&workspace.id)?).map_err(|e| e.to_string())
            }
        }
    }
    pub(crate) fn check_operation_paths(&self, paths: &[String]) -> Result<(), String> {
        match self
            .operation_writes
            .values()
            .flatten()
            .any(|held| paths.iter().any(|path| path == held))
        {
            true => Err(prometeu_core::error::code("err.windows.operationBusy")),
            false => Ok(()),
        }
    }
    pub(crate) fn admit_operation_input(&self, workspace: &Workspace) -> Result<(), String> {
        self.check_operation_paths(
            &workspace
                .repos
                .iter()
                .map(|r| r.worktree.clone())
                .collect::<Vec<_>>(),
        )
    }
    pub(crate) fn admit_destructive_operation(&self) -> Result<(), String> {
        match self.operations.running() {
            true => Err(prometeu_core::error::code("err.windows.operationBusy")),
            false => Ok(()),
        }
    }
}
