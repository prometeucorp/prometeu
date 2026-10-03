#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod clipboard;
mod consent;
mod explorer;
use prometeu_bridge::terminal::{OpenedTerminal, TerminalSnapshot};
use prometeu_bridge::workspaces::Catalog;
use prometeu_bridge::{
    ApplicationWslLauncher, ResidentWslLauncher, Response, RuntimeClient, RuntimeConnector,
    RuntimeEvents, Snapshot, Started, StdioConnector, Target,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{Emitter, Manager, State};

include!(concat!(env!("OUT_DIR"), "/runtime_package.rs"));

struct Connected {
    target: Target,
    client: Box<dyn RuntimeClient>,
    events: Arc<Events>,
    connector: Arc<dyn RuntimeConnector>,
}
struct Desktop {
    bootstrap: prometeu_bridge::bootstrap::Bootstrap,
    connector: Arc<dyn RuntimeConnector>,
    application_connector: Arc<dyn RuntimeConnector>,
    application: prometeu_bridge::application::NativeApplication,
    session: Mutex<Option<Connected>>,
}
struct Events {
    app: tauri::AppHandle,
    connection: String,
    active: AtomicBool,
}
impl RuntimeEvents for Events {
    fn publish(&self, frame: Value) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        if let Some(event) = frame.get("application") {
            if let Some(
                name @ ("board" | "chat" | "chat-closed" | "pty" | "pty-closed" | "accounts"),
            ) = event["name"].as_str()
            {
                let _ = self.app.emit(name, &event["payload"]);
            }
            return;
        }
        let _ = self.app.emit(
            "wsl-runtime",
            serde_json::json!({"connection":self.connection,"frame":frame}),
        );
    }
}
impl Desktop {
    fn reconnect(&self, current: &mut Connected) -> Result<bool, String> {
        if current.client.connected() {
            return Ok(false);
        }
        current.events.active.store(false, Ordering::Release);
        current.client.disconnect()?;
        let events = Arc::new(Events {
            app: current.events.app.clone(),
            connection: current.events.connection.clone(),
            active: AtomicBool::new(true),
        });
        let client = current.connector.connect(&current.target, events.clone())?;
        if !client.connected() {
            return Err("runtime disconnected during attachment".into());
        }
        current.client = client;
        current.events = events;
        Ok(true)
    }

    fn with_session<T>(
        &self,
        run: impl FnOnce(&mut dyn RuntimeClient) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut session = self.session.lock().map_err(|e| e.to_string())?;
        run(session
            .as_mut()
            .ok_or("runtime is not connected")?
            .client
            .as_mut())
    }
}
#[tauri::command]
async fn application_open(
    app: tauri::AppHandle,
    state: State<'_, Arc<Desktop>>,
    previous: Option<Target>,
) -> Result<Target, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = session.as_mut() {
            state.reconnect(existing)?;
            return Ok(existing.target.clone());
        }
        let target = state.bootstrap.prepare(previous)?;
        let events = Arc::new(Events {
            app,
            connection: "application".into(),
            active: AtomicBool::new(true),
        });
        let client = state
            .application_connector
            .connect(&target, events.clone())?;
        *session = Some(Connected {
            target: target.clone(),
            client,
            events,
            connector: state.application_connector.clone(),
        });
        Ok(target)
    })
    .await
    .map_err(|e| e.to_string())?
}
/// Replaces only a failed attachment. Never rediscover another distribution or replay a request.
#[tauri::command]
async fn application_reconnect(state: State<'_, Arc<Desktop>>) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        let Some(current) = session.as_mut() else {
            return Ok(false);
        };
        state.reconnect(current)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn wsl_connect(
    app: tauri::AppHandle,
    state: State<'_, Arc<Desktop>>,
    target: Target,
    connection: String,
) -> Result<bool, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = session.as_ref() {
            if existing.target == target {
                return Ok(existing.client.terminal_supported());
            }
            return Err("disconnect the current runtime first".into());
        }
        let events = Arc::new(Events {
            app,
            connection,
            active: AtomicBool::new(true),
        });
        let client = state.connector.connect(&target, events.clone())?;
        let terminal = client.terminal_supported();
        *session = Some(Connected {
            target,
            client,
            events,
            connector: state.connector.clone(),
        });
        Ok(terminal)
    })
    .await
    .map_err(|e| e.to_string())?
}
async fn operation<T: Send + 'static>(
    state: State<'_, Arc<Desktop>>,
    run: impl FnOnce(&mut dyn RuntimeClient) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.with_session(run))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn application_paths(
    state: State<'_, Arc<Desktop>>,
    paths: Vec<String>,
    direction: String,
) -> Result<Vec<String>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = state
            .session
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
            .ok_or("runtime is not connected")?
            .target
            .clone();
        state.application.paths(&target, &paths, &direction)
    })
    .await
    .map_err(|e| e.to_string())?
}
// Each bounded request locks its connection independently. Browser consent and polling waits
// must never retain the session lock or redirect an operation to a replacement connection.
struct ApplicationConnection {
    desktop: Arc<Desktop>,
    target: Target,
}
impl prometeu_bridge::application::ApplicationClient for ApplicationConnection {
    fn initialization_supported(&self) -> bool {
        self.desktop
            .session
            .lock()
            .ok()
            .and_then(|s| {
                s.as_ref()
                    .map(|s| s.target == self.target && s.client.initialization_supported())
            })
            .unwrap_or(false)
    }
    fn operations_supported(&self) -> bool {
        self.desktop
            .session
            .lock()
            .ok()
            .and_then(|s| {
                s.as_ref()
                    .map(|s| s.target == self.target && s.client.operations_supported())
            })
            .unwrap_or(false)
    }
    fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
        let mut session = self.desktop.session.lock().map_err(|e| e.to_string())?;
        let session = session.as_mut().ok_or("runtime is not connected")?;
        if session.target != self.target {
            return Err("runtime connection changed".into());
        }
        session.client.application(command, args)
    }
}
#[tauri::command]
async fn application_request(
    state: State<'_, Arc<Desktop>>,
    command: String,
    args: Value,
) -> Result<tauri::ipc::Response, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let target = state
            .session
            .lock()
            .map_err(|e| e.to_string())?
            .as_ref()
            .ok_or("runtime is not connected")?
            .target
            .clone();
        let mut client = ApplicationConnection {
            desktop: state.clone(),
            target: target.clone(),
        };
        let response = state
            .application
            .request(&target, &mut client, command, args)?;
        match response {
            prometeu_bridge::application::ApplicationResponse::Json(value) => {
                serde_json::to_string(&value)
                    .map(tauri::ipc::Response::new)
                    .map_err(|error| error.to_string())
            }
            prometeu_bridge::application::ApplicationResponse::Bytes(bytes) => {
                Ok(tauri::ipc::Response::new(bytes))
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn wsl_workspace_worktree(
    state: State<'_, Arc<Desktop>>,
    request: prometeu_bridge::workspaces::WorktreeRequest,
) -> Result<Catalog, String> {
    operation(state, move |s| s.workspace_worktree(request)).await
}
#[tauri::command]
async fn wsl_workspace_list(state: State<'_, Arc<Desktop>>) -> Result<Catalog, String> {
    operation(state, |s| s.workspace_list()).await
}
#[tauri::command]
async fn wsl_workspace_create(
    state: State<'_, Arc<Desktop>>,
    title: String,
    path: String,
) -> Result<Catalog, String> {
    operation(state, move |s| s.workspace_create(title, path)).await
}
#[tauri::command]
async fn wsl_workspace_select(
    state: State<'_, Arc<Desktop>>,
    id: String,
) -> Result<Catalog, String> {
    operation(state, move |s| s.workspace_select(id)).await
}
#[tauri::command]
async fn wsl_workspace_stage(
    state: State<'_, Arc<Desktop>>,
    id: String,
    stage: String,
) -> Result<Catalog, String> {
    operation(state, move |s| s.workspace_stage(id, stage)).await
}
#[tauri::command]
async fn wsl_start(state: State<'_, Arc<Desktop>>) -> Result<Started, String> {
    operation(state, |s| s.start()).await
}
#[tauri::command]
async fn wsl_send(state: State<'_, Arc<Desktop>>, text: String) -> Result<(), String> {
    operation(state, move |s| s.send(text)).await
}
#[tauri::command]
async fn wsl_respond(
    state: State<'_, Arc<Desktop>>,
    request_id: String,
    response: Response,
) -> Result<(), String> {
    operation(state, move |s| s.respond(request_id, response)).await
}
#[tauri::command]
async fn wsl_stop(state: State<'_, Arc<Desktop>>) -> Result<(), String> {
    operation(state, |s| s.stop()).await
}
#[tauri::command]
async fn wsl_snapshot(state: State<'_, Arc<Desktop>>) -> Result<Snapshot, String> {
    operation(state, |s| s.snapshot()).await
}
#[tauri::command]
async fn wsl_disconnect(state: State<'_, Arc<Desktop>>) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        match session.take() {
            Some(mut connection) => {
                connection.events.active.store(false, Ordering::Release);
                connection.client.disconnect()
            }
            None => Ok(()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn wsl_shutdown(state: State<'_, Arc<Desktop>>) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        let current = session.as_mut().ok_or("runtime is not connected")?;
        current.events.active.store(false, Ordering::Release);
        current.client.shutdown()
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn wsl_terminal_current(
    state: State<'_, Arc<Desktop>>,
) -> Result<Option<TerminalSnapshot>, String> {
    operation(state, |s| s.terminal_current()).await
}
#[tauri::command]
async fn wsl_terminal_open(
    state: State<'_, Arc<Desktop>>,
    cols: u16,
    rows: u16,
) -> Result<OpenedTerminal, String> {
    operation(state, move |s| s.terminal_open(cols, rows)).await
}
#[tauri::command]
async fn wsl_terminal_write(
    state: State<'_, Arc<Desktop>>,
    id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    operation(state, move |s| s.terminal_write(id, data)).await
}
#[tauri::command]
async fn wsl_terminal_resize(
    state: State<'_, Arc<Desktop>>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    operation(state, move |s| s.terminal_resize(id, cols, rows)).await
}
#[tauri::command]
async fn wsl_terminal_snapshot(
    state: State<'_, Arc<Desktop>>,
    id: String,
) -> Result<TerminalSnapshot, String> {
    operation(state, move |s| s.terminal_snapshot(id)).await
}
#[tauri::command]
async fn wsl_terminal_acknowledge(
    state: State<'_, Arc<Desktop>>,
    id: String,
    seq: u64,
) -> Result<(), String> {
    operation(state, move |s| s.terminal_acknowledge(id, seq)).await
}
#[tauri::command]
async fn wsl_terminal_close(state: State<'_, Arc<Desktop>>, id: String) -> Result<(), String> {
    operation(state, move |s| s.terminal_close(id)).await
}

fn main() {
    use prometeu_bridge::{
        bootstrap::{Bootstrap, NativeEnvironment, NativeInstaller, NativeRoots, RuntimePackage},
        paths::{ApplicationPaths, NativePathQuery, WslPaths},
        wsl_command::NativeWslCommands,
    };
    let commands = Arc::new(NativeWslCommands);
    let paths: Arc<dyn ApplicationPaths> = Arc::new(WslPaths(Arc::new(NativePathQuery)));
    let bootstrap = Bootstrap {
        environment: Arc::new(NativeEnvironment(commands.clone())),
        roots: Arc::new(NativeRoots(commands.clone())),
        installer: Arc::new(NativeInstaller {
            commands,
            package: RUNTIME_PACKAGE.map(|(digest, bytes)| RuntimePackage {
                digest: digest.into(),
                bytes: bytes.into(),
            }),
        }),
        root: std::env::var("PROMETEU_WINDOWS_RUNTIME_ROOT").ok(),
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let clipboard = Arc::new(clipboard::Clipboard(app.path().app_local_data_dir()?));
            app.manage(Arc::new(Desktop {
                bootstrap,
                connector: Arc::new(StdioConnector {
                    launcher: Arc::new(ResidentWslLauncher),
                }),
                application_connector: Arc::new(StdioConnector {
                    launcher: Arc::new(ApplicationWslLauncher),
                }),
                session: Mutex::new(None),
                application: prometeu_bridge::application::NativeApplication {
                    paths,
                    files: Arc::new(explorer::Explorer),
                    clipboard,
                    consent: Arc::new(consent::BrowserConsent),
                },
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            application_open,
            application_reconnect,
            wsl_connect,
            application_request,
            application_paths,
            wsl_workspace_worktree,
            wsl_workspace_list,
            wsl_workspace_create,
            wsl_workspace_select,
            wsl_workspace_stage,
            wsl_start,
            wsl_send,
            wsl_respond,
            wsl_stop,
            wsl_snapshot,
            wsl_disconnect,
            wsl_shutdown,
            wsl_terminal_current,
            wsl_terminal_open,
            wsl_terminal_write,
            wsl_terminal_resize,
            wsl_terminal_snapshot,
            wsl_terminal_close,
            wsl_terminal_acknowledge
        ])
        .build(tauri::generate_context!())
        .expect("failed to build WSL desktop")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Dropping the proxy detaches this window; the resident host keeps execution alive.
                if let Ok(mut session) = app.state::<Arc<Desktop>>().session.lock() {
                    session.take();
                }
            }
        });
}
