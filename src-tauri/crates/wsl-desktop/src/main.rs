#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod clipboard;
mod consent;
mod explorer;
use prometeu_bridge::{
    ApplicationWslLauncher, ResidentWslLauncher, RuntimeClient, RuntimeConnector, RuntimeEvents,
    StdioConnector, Target,
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
    /// Explicit-target attachment used only by the native acceptance fixture (`wsl_connect`).
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
// `wsl_connect`, `wsl_disconnect` and `wsl_shutdown` are not used by the interface. The native
// acceptance fixture (`scripts/test-windows-application.mjs`) uses them to replace the bootstrap
// attachment with an explicit target running a synthetic provider.
#[tauri::command]
async fn wsl_connect(
    app: tauri::AppHandle,
    state: State<'_, Arc<Desktop>>,
    target: Target,
    connection: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut session = state.session.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = session.as_ref() {
            if existing.target == target {
                return Ok(());
            }
            return Err("disconnect the current runtime first".into());
        }
        let events = Arc::new(Events {
            app,
            connection,
            active: AtomicBool::new(true),
        });
        let client = state.connector.connect(&target, events.clone())?;
        *session = Some(Connected {
            target,
            client,
            events,
            connector: state.connector.clone(),
        });
        Ok(())
    })
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
            application_request,
            application_paths,
            wsl_connect,
            wsl_disconnect,
            wsl_shutdown
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
