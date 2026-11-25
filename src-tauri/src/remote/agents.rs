//! Agents on a machine (ADR 0103), from Settings: enable, rotate and disable the host's agent
//! socket over the machine's attachment. The setup's token goes straight to the screen that
//! shows the MCP configuration; this app neither stores nor logs it, and it never reaches the
//! relay.
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::connection::Connection;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AgentsStatus {
    pub enabled: bool,
    pub socket: String,
    /// Whether something answers at the socket.
    pub serving: bool,
    /// Why the host cannot serve the socket, such as a file taking its path.
    pub error: Option<String>,
}

/// Enabling either needs the person's consent to open folders under the machine's home to other
/// users, or is done.
#[derive(Serialize)]
#[serde(untagged)]
pub enum EnableOutcome {
    NeedsOpen { needs_open: Vec<String> },
    Ready(AgentsSetup),
}

/// What Settings shows once, right after enabling or rotating.
#[derive(Serialize)]
pub struct AgentsSetup {
    /// The container's bind mount, as a `docker run` option.
    pub mount: String,
    /// `{"mcpServers": {"prometeu": …}}`, pretty-printed, with the token: what Copy copies.
    pub config: String,
    /// The same configuration with the token masked: what the screen shows, so a screenshot
    /// never carries it.
    pub preview: String,
    /// The socket clients used before this setup moved it.
    pub moved_from: Option<String>,
    /// The host's user ID the configuration expects the client to see, and the real one.
    pub client_uid: u32,
    pub local_uid: u32,
    /// Folders on the machine made traversable by other users.
    pub opened: Vec<String>,
}

/// The installed build this app attaches with, which always knows the current agent commands,
/// even while an older host still serves the socket.
fn host_command(connection: &Connection, args: &[&str]) -> Result<Value, String> {
    let binary =
        super::install::local_binary().ok_or_else(|| crate::i18n::t("err.remote.noBuild"))?;
    let root = connection
        .ready()
        .and_then(|r| r.get("root").and_then(Value::as_str).map(str::to_owned))
        .ok_or_else(|| crate::i18n::t(super::connection::OFFLINE))?;
    let program = format!("{root}/host/{}/prometeu-host", binary.hash);
    let mut argv = vec![program.as_str(), "agents"];
    argv.extend_from_slice(args);
    argv.extend_from_slice(&["--json", "--root", &root]);
    let stdout = connection
        .exec(&argv, None, Duration::from_secs(30))?
        .ok()?;
    serde_json::from_str(stdout.trim()).map_err(|_| {
        crate::i18n::ta(
            "err.remote.command",
            &[("detail", "invalid agents output".to_string())],
        )
    })
}

pub fn status(connection: &Connection) -> Result<AgentsStatus, String> {
    let status = host_command(connection, &["status"])?;
    Ok(AgentsStatus {
        enabled: status["enabled"] == Value::Bool(true),
        socket: status["socket"].as_str().unwrap_or_default().to_string(),
        serving: status["serving"] == Value::Bool(true),
        error: status["error"].as_str().map(str::to_owned),
    })
}

/// Read the status again after attaching or changing it, and publish it with the machines.
pub fn refresh(app: &AppHandle, connection: &Arc<Connection>) {
    if let Ok(status) = status(connection) {
        super::publish_agents(app, &connection.id, status);
    }
}

fn masked(config: &Value) -> Value {
    let mut masked = config.clone();
    if let Some(token) = masked.pointer_mut("/mcpServers/prometeu/env/PROMETEU_AGENT_TOKEN") {
        *token = Value::String("•".repeat(16));
    }
    masked
}

fn setup_view(setup: &Value) -> AgentsSetup {
    let mount = &setup["mount"];
    let config = serde_json::json!({"mcpServers": {"prometeu": setup["server"]}});
    let number = |key: &str| setup[key].as_u64().unwrap_or_default() as u32;
    AgentsSetup {
        mount: format!(
            "-v {}:{}:ro",
            mount["source"].as_str().unwrap_or_default(),
            mount["target"].as_str().unwrap_or_default()
        ),
        preview: serde_json::to_string_pretty(&masked(&config)).unwrap_or_default(),
        config: serde_json::to_string_pretty(&config).unwrap_or_default(),
        moved_from: setup["moved_from"].as_str().map(str::to_owned),
        client_uid: number("client_uid"),
        local_uid: number("local_uid"),
        opened: setup["opened"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Enable agents on the machine, or replace their token when `rotate`, and return the
/// configuration a bot needs. Rotating cuts off every connected agent client. Folders under the
/// machine's home are opened to other users only with `open_home`, which Settings passes after
/// asking; without it, the outcome names them and nothing changes.
#[tauri::command(async)]
pub fn machine_agents_enable(
    app: AppHandle,
    id: String,
    rotate: bool,
    open_home: bool,
) -> Result<EnableOutcome, String> {
    let connection = app.state::<super::Service>().connected(&id)?;
    let mut args = vec!["setup"];
    if rotate {
        args.push("--rotate");
    }
    if open_home {
        args.push("--open-home");
    }
    let setup = host_command(&connection, &args)?;
    if let Some(folders) = setup["needs_open"].as_array() {
        let needs_open = folders
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        return Ok(EnableOutcome::NeedsOpen { needs_open });
    }
    // The host binds within a few seconds; wait for it, or for its reason not to, so Settings
    // never reports agents as enabled while nothing serves them.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let current = status(&connection);
        let settled = current
            .as_ref()
            .map_or(true, |s| s.serving || s.error.is_some());
        if let Ok(current) = current {
            super::publish_agents(&app, &connection.id, current);
        }
        if settled || std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(EnableOutcome::Ready(setup_view(&setup)))
}

/// Disable agents on the machine: the token is deleted and the host closes the socket.
#[tauri::command(async)]
pub fn machine_agents_disable(app: AppHandle, id: String) -> Result<(), String> {
    let connection = app.state::<super::Service>().connected(&id)?;
    host_command(&connection, &["disable"])?;
    refresh(&app, &connection);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_output_becomes_a_mount_and_a_ready_to_paste_config() {
        let setup = serde_json::json!({
            "token": "t0k",
            "mount": {"source": "/home/me/.prometeu-remote-agents", "target": "/prometeu", "read_only": true},
            "server": {"command": "/prometeu/prometeu-mcp", "args": ["mcp", "--socket", "/prometeu/agents-v1.sock"],
                "env": {"PROMETEU_AGENT_TOKEN": "t0k", "PROMETEU_AGENT_NAME": "agent", "PROMETEU_AGENT_HOST_UID": "65534"}},
            "client_uid": 65534, "local_uid": 1000, "opened": ["/home/me"],
        });
        let view = setup_view(&setup);
        assert_eq!(
            view.mount,
            "-v /home/me/.prometeu-remote-agents:/prometeu:ro"
        );
        let config: Value = serde_json::from_str(&view.config).unwrap();
        assert_eq!(config["mcpServers"]["prometeu"], setup["server"]);
        assert_eq!((view.client_uid, view.local_uid), (65534, 1000));
        assert_eq!(view.opened, vec!["/home/me".to_string()]);
        assert!(
            !view.preview.contains("t0k"),
            "the screen never shows the token"
        );
        let preview: Value = serde_json::from_str(&view.preview).unwrap();
        assert_eq!(
            preview["mcpServers"]["prometeu"]["args"],
            setup["server"]["args"]
        );
        assert_eq!(view.moved_from, None);
    }

    #[test]
    fn enabling_asks_before_opening_folders_and_settings_tells_the_outcomes_apart() {
        let asks = EnableOutcome::NeedsOpen {
            needs_open: vec!["/home/me".into()],
        };
        assert_eq!(
            serde_json::to_value(asks).unwrap(),
            serde_json::json!({"needs_open": ["/home/me"]})
        );
        let ready =
            serde_json::to_value(EnableOutcome::Ready(setup_view(&serde_json::json!({})))).unwrap();
        assert!(ready.get("needs_open").is_none());
        assert!(ready.get("config").is_some());
    }
}
