use prometeu_core::{
    board::Workspace,
    workspaces::{valid_id, Workspaces, PRIMARY},
};
use prometeu_runtime::{
    host::{self, Context, ContextFactory, Host, WorkspaceEvents},
    provider::CodexPreparation,
    resident::{self, Configuration},
    store::Store,
    terminal::{LoginShell, TerminalService},
    workspaces::{FileCatalog, LinuxFolders},
    Runtime, RuntimeEvents,
};
use serde_json::Value;
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
struct Output(Mutex<std::io::Stdout>);
impl RuntimeEvents for Output {
    fn publish(&self, frame: Value) -> Result<(), String> {
        let mut out = self.0.lock().unwrap_or_else(|e| e.into_inner());
        writeln!(out, "{frame}")
            .and_then(|()| out.flush())
            .map_err(|e| e.to_string())
    }
}
fn compose_terminal(
    workdir: PathBuf,
    config: &Configuration,
    output: Arc<dyn RuntimeEvents>,
) -> TerminalService {
    TerminalService::new(
        workdir,
        Arc::new(prometeu_process::terminal::ResponsiveTerminalFactory),
        Arc::new(LoginShell(config.shell.clone())),
        output.clone(),
        Arc::new(prometeu_process::ThreadExecutor),
    )
}
fn compose_context(
    root: &std::path::Path,
    workdir: &std::path::Path,
    config: &Configuration,
    output: Arc<dyn RuntimeEvents>,
    provider: CodexPreparation,
) -> Result<Context, String> {
    let store = Arc::new(Store::open(root, workdir)?);
    let terminals = compose_terminal(store.workdir(), config, output.clone());
    let runtime = Runtime::new(
        store,
        output,
        Arc::new(provider),
        Arc::new(prometeu_process::UnixProcessLauncher),
        Arc::new(prometeu_process::ThreadExecutor),
    );
    Ok(Context { runtime, terminals })
}
struct NativeContexts {
    tools: Arc<dyn prometeu_tools::StartupTools>,
    selection: Arc<dyn prometeu_core::tool_resolution::ToolSelection>,
    accounts: Arc<prometeu_core::accounts::AccountRegistry>,
    root: PathBuf,
    config: Configuration,
    selected: Arc<Mutex<String>>,
    output: Arc<dyn RuntimeEvents>,
    application: Arc<prometeu_runtime::application::ApplicationEvents>,
}
impl NativeContexts {
    fn preparation(&self, workspace: Option<&Workspace>, session: &str) -> CodexPreparation {
        let tab = workspace.and_then(|w| w.tabs.iter().find(|t| t.id == session));
        let choice = tab.and_then(|t| t.choice.as_ref());
        let model = choice
            .map(|c| c.model.as_str())
            .or_else(|| workspace.map(|w| w.model.as_str()))
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.config.model);
        CodexPreparation {
            workspace: workspace.map_or(PRIMARY, |w| w.id.as_str()).into(),
            session: session.into(),
            selection: self.selection.clone(),
            tools: self.tools.clone(),
            profile: prometeu_runtime::tools::profile(),
            executable: self.config.codex.clone().into(),
            model: model.into(),
            effort: choice
                .map(|c| c.effort.clone())
                .or_else(|| workspace.map(|w| w.effort.clone()))
                .unwrap_or_default(),
            accounts: self.accounts.clone(),
            permission: tab.and_then(|t| t.permission).unwrap_or_default(),
        }
    }
}
impl ContextFactory for NativeContexts {
    fn retune(
        &self,
        runtime: &mut Runtime,
        workspace: &Workspace,
        session: &str,
    ) -> Result<(), String> {
        runtime.reconfigure(Arc::new(self.preparation(Some(workspace), session)))
    }
    fn archive(
        &self,
        workdir: &std::path::Path,
        launch: prometeu_runtime::dock::DockLaunch,
    ) -> Result<(), String> {
        use std::process::{Command, Stdio};
        let Some(command) = launch.command else {
            return Ok(());
        };
        let mut child = Command::new("/bin/sh")
            .args(["-lc", &command])
            .current_dir(workdir)
            .envs(launch.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn shell(
        &self,
        workdir: &std::path::Path,
        launch: prometeu_runtime::dock::DockLaunch,
        output: Arc<dyn RuntimeEvents>,
    ) -> Result<TerminalService, String> {
        use prometeu_runtime::terminal::{EnvironmentShell, ScriptShell, ShellPreparation};
        let shell: Arc<dyn ShellPreparation> = match launch.command {
            Some(command) => Arc::new(ScriptShell {
                command,
                environment: launch.environment,
            }),
            None => Arc::new(EnvironmentShell {
                shell: LoginShell(self.config.shell.clone()),
                environment: launch.environment,
            }),
        };
        let mut terminal = TerminalService::new(
            workdir.to_owned(),
            Arc::new(prometeu_process::terminal::ResponsiveTerminalFactory),
            shell,
            output,
            Arc::new(prometeu_process::ThreadExecutor),
        )
        .with_header(launch.header);
        terminal.script_name = launch.name;
        Ok(terminal)
    }
    fn open(&self, workspace: &Workspace, session: &str) -> Result<Context, String> {
        if !valid_id(&workspace.id) || !valid_id(session) {
            return Err("workspace_catalog_invalid".into());
        }
        let tab = workspace.tabs.iter().find(|tab| tab.id == session);
        if tab.is_none() && session != workspace.id {
            return Err("workspace_not_found".into());
        }
        let workspace_root = match workspace.id.as_str() {
            PRIMARY => self.root.clone(),
            id => self.root.join("workspaces").join(id),
        };
        let root = match session == workspace.id {
            true => workspace_root,
            false => workspace_root.join("tabs").join(session),
        };
        compose_context(
            &root,
            std::path::Path::new(&workspace.worktree),
            &self.config,
            Arc::new(WorkspaceEvents {
                id: session.into(),
                application: self.application.clone(),
                selected: self.selected.clone(),
                output: self.output.clone(),
            }),
            self.preparation(Some(workspace), session),
        )
    }
}
fn compose(
    root: &std::path::Path,
    config: &Configuration,
    output: Arc<dyn RuntimeEvents>,
    catalog_mode: &resident::CatalogMode,
) -> Result<Host, String> {
    let application = Arc::new(prometeu_runtime::application::ApplicationEvents::new(
        output.clone(),
    ));
    let selected = Arc::new(Mutex::new(PRIMARY.to_owned()));
    // Acquiring the original Store first preserves the legacy root lease and transcript location.
    let accounts = Arc::new(prometeu_core::accounts::AccountRegistry::new(Arc::new(
        prometeu_files::accounts::FileAccountStore::new(root.into()),
    )));
    let discovery = Arc::new(prometeu_runtime::discovery::Discovery {
        accounts: accounts.clone(),
        providers: vec![Arc::new(prometeu_runtime::discovery::CodexDiscovery {
            executable: config.codex.clone().into(),
            workdir: config.workdir.clone(),
            queries: Arc::new(prometeu_process::query::UnixQueryLauncher),
        })],
    });
    let resources = prometeu_runtime::resources::native(root);
    let settings = Arc::new(prometeu_files::settings::NativeSettings);
    let selection = Arc::new(prometeu_runtime::tools::Selection {
        catalog: Arc::new(FileCatalog(root.into())),
        settings: settings.clone(),
        mcp: resources.mcp.clone(),
        packages: resources.packages.clone(),
    });
    let factory = NativeContexts {
        tools: prometeu_runtime::tools::native(
            root,
            std::path::Path::new(&config.codex),
            &resources,
        ),
        selection: selection.clone(),
        accounts: accounts.clone(),
        application: application.clone(),
        root: root.into(),
        config: config.clone(),
        selected: selected.clone(),
        output: output.clone(),
    };
    let mut primary = compose_context(
        root,
        &config.workdir,
        config,
        Arc::new(WorkspaceEvents {
            id: PRIMARY.into(),
            application: application.clone(),
            selected: selected.clone(),
            output: output.clone(),
        }),
        factory.preparation(None, PRIMARY),
    )?;
    let primary_path = config.workdir.to_str().ok_or("workspace_folder_invalid")?;
    let seed: &dyn prometeu_core::workspaces::CatalogSeed = match catalog_mode {
        resident::CatalogMode::Workspace => {
            &prometeu_core::workspaces::PrimaryCatalog(primary_path)
        }
        resident::CatalogMode::Application => &prometeu_core::workspaces::EmptyCatalog,
    };
    let catalog = Workspaces::open_with(
        Arc::new(FileCatalog(root.into())),
        Arc::new(LinuxFolders),
        primary_path,
        prometeu_core::board::ProviderId::Codex,
        seed,
    )?;
    if let Ok(workspace) = catalog.get(PRIMARY) {
        // The bootstrap store is leased before loading the catalog; apply its saved tab
        // settings now so a full runtime restart preserves model overrides as well.
        if workspace.tabs.iter().any(|tab| tab.id == PRIMARY) {
            factory.retune(&mut primary.runtime, workspace, PRIMARY)?;
        }
    }
    Host::new(
        catalog,
        host::ApplicationServices {
            cleanup: Arc::new(prometeu_git::cleanup::NativeCleanup),
            resources,
            tool_selection: selection,
            git: Arc::new(prometeu_git::NativeGit),
            discovery,
            references: Arc::new(prometeu_runtime::repository::GitReferences(Arc::new(
                prometeu_process::command::UnixCommandRunner,
            ))),
            project_entries: Arc::new(prometeu_files::entries::NativeEntries {
                trash: Arc::new(prometeu_files::entries::SystemTrash),
            }),
            project_search: Arc::new(prometeu_files::search::NativeSearch::default()),
            files: Arc::new(prometeu_files::NativeFiles),
            settings,
            preparation: Arc::new(prometeu_files::scripts::NativePreparation),
        },
        application,
        Box::new(prometeu_runtime::worktrees::GitWorktrees {
            root: root.into(),
            runner: Arc::new(prometeu_process::command::UnixCommandRunner),
        }),
        primary,
        Box::new(factory),
        selected,
    )
}
fn run() -> Result<(), String> {
    let mut options = std::collections::HashMap::new();
    let mut args = std::env::args().skip(1);
    while let Some(key) = args.next() {
        if key == "--help" {
            println!("prometeu-runtime --root EMPTY_OR_RUNTIME_DIRECTORY --workdir DIRECTORY [--codex EXECUTABLE] [--model MODEL] [--shell EXECUTABLE] [--transport stdio|resident|serve] [--catalog workspace|application]\nExperimental workspace host; addressed conversation sessions. See docs/contracts/wsl-runtime.md.");
            return Ok(());
        }
        if ![
            "--root",
            "--workdir",
            "--codex",
            "--model",
            "--shell",
            "--transport",
            "--catalog",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unknown option: {key}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {key}"))?;
        if options.insert(key.clone(), value).is_some() {
            return Err(format!("duplicate option: {key}"));
        }
    }
    let root = PathBuf::from(options.remove("--root").ok_or("--root is required")?);
    let config = Configuration {
        v: 1,
        workdir: PathBuf::from(options.remove("--workdir").ok_or("--workdir is required")?)
            .canonicalize()
            .map_err(|e| e.to_string())?,
        codex: options.remove("--codex").unwrap_or("codex".into()),
        model: options.remove("--model").unwrap_or_default(),
        shell: options
            .remove("--shell")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("SHELL").map(PathBuf::from))
            .unwrap_or_else(|| "/bin/sh".into()),
    };
    let catalog = match options
        .remove("--catalog")
        .as_deref()
        .unwrap_or("workspace")
    {
        "workspace" => resident::CatalogMode::Workspace,
        "application" => resident::CatalogMode::Application,
        mode => return Err(format!("unknown catalog: {mode}")),
    };
    match options.remove("--transport").as_deref().unwrap_or("stdio") {
        "resident" => resident::proxy_with_catalog(&root, &config, &catalog),
        "serve" => resident::serve(&root, &config, |events| {
            compose(&root, &config, events, &catalog)
        }),
        "stdio" => {
            let output = Arc::new(Output(Mutex::new(std::io::stdout())));
            let mut host = compose(&root, &config, output.clone(), &catalog)?;
            host.attached(true);
            output.publish(host::ready(&[
                "terminal.v1",
                "workspaces.v1",
                "worktrees.v1",
                "application.v1",
                "application.operations.v1",
                "application.initialization.v1",
            ]))?;
            let (send, receive) = std::sync::mpsc::sync_channel(2);
            std::thread::spawn(move || {
                let mut input = std::io::stdin().lock();
                loop {
                    let line = host::read_request(&mut input);
                    let finished = !matches!(&line, Ok(Some(_)));
                    if send.send(line).is_err() || finished {
                        break;
                    }
                }
            });
            loop {
                host.poll_operations();
                host.poll_launches()?;
                match receive.recv_timeout(std::time::Duration::from_millis(10)) {
                    Ok(Ok(Some(line))) => {
                        let (reply, shutdown) = host.request(&line);
                        output.publish(reply)?;
                        if shutdown {
                            break;
                        }
                    }
                    Ok(Ok(None)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Err(error)) => return Err(error),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
            host.shutdown()
        }
        mode => Err(format!("unknown transport: {mode}")),
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
