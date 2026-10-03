//! Application script orchestration over injected preparation, context and terminal ports.
use crate::{
    application::{dock_key, DockEvents},
    host::Host,
};
use prometeu_core::{
    board::Workspace,
    error::{code, with_args},
};
use prometeu_files::{scripts, settings::FILES};
use serde_json::json;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct DockLaunch {
    pub command: Option<String>,
    pub environment: Vec<(String, String)>,
    pub header: Option<String>,
    pub name: Option<String>,
}
/// File copying has no catalog or process effects. The owner starts Setup after completion.
pub(crate) struct PreparedSetup {
    command: String,
    header: Option<String>,
}
impl PreparedSetup {
    pub fn prepare(
        workspace: &Workspace,
        declarations: &prometeu_files::settings::Scripts,
        preparation: &dyn scripts::WorkspacePreparation,
        pick: impl Fn(&str, &str) -> String,
    ) -> Option<Self> {
        let notes = preparation.hydrate(
            Path::new(&workspace.worktree),
            Path::new(&workspace.repo),
            &declarations.copy,
        );
        let header = scripts::report(&notes, pick);
        (declarations.setup.is_some() || header.is_some()).then(|| Self {
            command: declarations.setup.clone().unwrap_or_else(|| "true".into()),
            header,
        })
    }
}
pub(crate) struct PendingLaunch {
    workspace: String,
    deadline: Instant,
}
impl Host {
    fn ensure_port(&mut self, id: &str) -> Result<Workspace, String> {
        let workspace = self.catalog.get(id)?.clone();
        if workspace.port.is_some_and(scripts::usable) {
            return Ok(workspace);
        }
        let taken = self
            .catalog
            .snapshot()
            .board
            .workspaces
            .iter()
            .filter_map(|w| w.port)
            .collect::<Vec<_>>();
        let port = self
            .services
            .preparation
            .allocate_port(Path::new(&workspace.worktree), &taken)
            .ok_or_else(|| code("err.session.noPort"))?;
        let workspace = self.catalog.port(id, port)?;
        self.application_events
            .publish_board(self.catalog.snapshot().board)?;
        Ok(workspace)
    }
    pub(crate) fn open_application_dock(
        &mut self,
        id: &str,
        kind: &str,
        name: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<String, String> {
        let key = dock_key(id, kind)?;
        if self.catalog.get(id).is_err() {
            let root = self.file_root(id)?;
            if matches!(kind, "setup" | "run") {
                return Err(code("err.session.noWorkspace"));
            }
            let launch = DockLaunch {
                command: None,
                environment: vec![],
                header: None,
                name: None,
            };
            return self.start_dock(&root, key, launch, cols, rows);
        }
        if self.catalog.get(id)?.cleaned {
            return Err(code("err.session.cleaned"));
        }
        if matches!(kind, "setup" | "run") {
            self.check_operation_paths(std::slice::from_ref(&self.catalog.get(id)?.worktree))?;
        }
        let workspace = self.ensure_port(id)?;
        if !matches!(kind, "setup" | "run") {
            let launch = DockLaunch {
                command: None,
                header: None,
                name: None,
                environment: scripts::workspace_env(&workspace),
            };
            return self.start_dock(Path::new(&workspace.worktree), key, launch, cols, rows);
        }
        let declarations = self
            .services
            .settings
            .read(Path::new(&workspace.worktree), Path::new(&workspace.repo));
        let run = match kind {
            "run" => Some(declarations.run(name).ok_or_else(|| {
                with_args(
                    "err.dock.noScript",
                    &[("kind", kind.into()), ("file", FILES[0].into())],
                )
            })?),
            _ => None,
        };
        if let Some(terminal) = self.docks.get(&key) {
            if terminal.current()?["running"] == true {
                if kind == "run" && terminal.script_name.as_deref() != run.map(|r| r.name.as_str())
                {
                    return Err(code("err.dock.running"));
                }
                return Ok(key);
            }
        }
        let (command, header) = match kind {
            "setup" => {
                let prepared = PreparedSetup::prepare(
                    &workspace,
                    &declarations,
                    self.services.preparation.as_ref(),
                    |pt, en| self.application_events.pick(pt, en),
                )
                .ok_or_else(|| {
                    with_args(
                        "err.dock.noScript",
                        &[("kind", kind.into()), ("file", FILES[0].into())],
                    )
                })?;
                (Some(prepared.command), prepared.header)
            }
            "run" => (run.map(|r| r.command.clone()), None),
            _ => (None, None),
        };
        let launch = DockLaunch {
            command,
            header,
            name: run.map(|r| r.name.clone()),
            environment: scripts::workspace_env(&workspace),
        };
        self.start_dock(Path::new(&workspace.worktree), key, launch, cols, rows)
    }
    pub(crate) fn start_prepared_setup(
        &mut self,
        id: &str,
        prepared: PreparedSetup,
        cols: u16,
        rows: u16,
    ) -> Result<String, String> {
        if self.catalog.get(id)?.cleaned {
            return Err(code("err.session.cleaned"));
        }
        let workspace = self.ensure_port(id)?;
        let key = dock_key(id, "setup")?;
        let launch = DockLaunch {
            command: Some(prepared.command),
            header: prepared.header,
            name: None,
            environment: scripts::workspace_env(&workspace),
        };
        self.start_dock(Path::new(&workspace.worktree), key, launch, cols, rows)
    }
    fn start_dock(
        &mut self,
        root: &Path,
        key: String,
        launch: DockLaunch,
        cols: u16,
        rows: u16,
    ) -> Result<String, String> {
        if self
            .docks
            .get(&key)
            .is_some_and(|dock| dock.current().is_ok_and(|v| v["running"] == true))
        {
            return Ok(key);
        }
        if let Some(previous) = self.docks.get_mut(&key) {
            previous.shutdown()?;
        }
        let mut terminal = self.factory.shell(
            root,
            launch,
            Arc::new(DockEvents {
                key: key.clone(),
                application: self.application_events.clone(),
            }),
        )?;
        terminal.open(cols, rows)?;
        terminal.attached(false);
        self.docks.insert(key.clone(), terminal);
        Ok(key)
    }
    pub(crate) fn setup_running(&self, workspace: &str) -> Result<bool, String> {
        self.docks
            .get(&format!("{workspace}:setup"))
            .map(|dock| dock.current().map(|v| v["running"] == true))
            .unwrap_or(Ok(false))
    }
    pub(crate) fn admit_application_input(&self, workspace: &str) -> Result<(), String> {
        self.admit_operation_input(self.catalog.get(workspace)?)?;
        if self.catalog.get(workspace)?.cleaned {
            return Err(code("err.session.cleaned"));
        }
        match self.setup_running(workspace)?
            || self
                .pending_launches
                .values()
                .any(|p| p.workspace == workspace)
        {
            true => Err(code("err.windows.preparing")),
            false => Ok(()),
        }
    }
    pub(crate) fn launch_application(
        &mut self,
        workspace: &Workspace,
        setup: Option<PreparedSetup>,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        if let Some(setup) = setup {
            self.start_prepared_setup(&workspace.id, setup, cols, rows)?;
        }
        let session = &workspace.tabs[0].id;
        self.session_context(session)?.runtime.start()?;
        self.pending_launches.insert(
            session.clone(),
            PendingLaunch {
                workspace: workspace.id.clone(),
                deadline: Instant::now() + Duration::from_secs(15),
            },
        );
        Ok(())
    }
    /// Called by the host event loop even without an attached window. No waiting on a script or
    /// provider readiness is performed while handling an application request.
    pub fn poll_launches(&mut self) -> Result<(), String> {
        let sessions = self.pending_launches.keys().cloned().collect::<Vec<_>>();
        for session in sessions {
            let pending = &self.pending_launches[&session];
            let workspace = pending.workspace.clone();
            if self
                .admit_operation_input(self.catalog.get(&workspace)?)
                .is_err()
            {
                continue;
            }
            let state = self
                .contexts
                .get(&session)
                .ok_or_else(|| "workspace_not_found".to_string())
                .and_then(|c| c.runtime.ready());
            let ready = match state {
                Ok(true) if self.setup_running(&workspace)? => continue,
                Ok(false) if Instant::now() < pending.deadline => continue,
                Ok(false) => Err("agent readiness timed out; message was not sent".into()),
                result => result.map(|_| ()),
            };
            // Remove admission before the attempt. A failed publication cannot replay input.
            self.pending_launches.remove(&session);
            let result = ready.and_then(|()| {
                let ws = self.catalog.get(&workspace)?;
                self.services.discovery.accounts.active(ws.agent)?;
                if let Some(prompt) = &ws.tabs[0].pending_prompt {
                    let warning = match self.docks.get(&format!("{workspace}:setup")) {
                        Some(dock) => scripts::setup_warning(dock.current()?["code"].as_u64().map(|n| n as u32), |pt, en| self.application_events.pick(pt, en)),
                        None => None,
                    };
                    self.contexts.get_mut(&session).ok_or("workspace_not_found")?.runtime.command(&json!({"v":1,"type":"message.send","text":format!("{}{}", warning.unwrap_or_default(), prompt)}))?;
                }
                Ok(())
            });
            self.catalog.launch_result(&workspace, result.err())?;
            self.application_events
                .publish_board(self.catalog.snapshot().board)?;
        }
        Ok(())
    }
}
