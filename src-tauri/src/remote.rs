//! Remote machines (ADR 0101): computers reached over SSH that run workspaces for this app. Each
//! machine gets the `prometeu-host` daemon, which owns terminal agents so they keep working while
//! this computer sleeps or the app is closed. The board, conversations and interface stay here.
//!
//! Machines live in `<root>/machines.json`: a name, an SSH destination (an alias from the
//! person's `~/.ssh/config` works) and default resource limits for each workspace. Credentials
//! never pass through the app: `ssh` authenticates with the person's own keys and agent.
pub mod agent;
pub mod agents;
pub mod codex;
pub mod codex_login;
pub mod connection;
pub mod install;
pub mod login;
pub mod registry;
pub mod supervision;
pub mod terminal;
pub mod workspace;

use connection::{Connection, Phase, Wake};
use prometeu_core::lock::lock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::AppState;

/// Resource limits applied to each workspace's systemd slice on the machine. Values use systemd
/// syntax (`4G`, `200%`); empty means unlimited.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    #[serde(default)]
    pub memory_max: String,
    #[serde(default)]
    pub cpu_quota: String,
}

impl Limits {
    fn valid(value: &str) -> bool {
        value.is_empty()
            || (value.len() <= 16
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '%' || c == '.'))
    }
    pub fn to_wire(&self) -> Value {
        json!({"memory_max": self.memory_max, "cpu_quota": self.cpu_quota})
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Machine {
    pub id: String,
    pub name: String,
    /// The SSH destination, passed as one argument after `--`.
    pub ssh: String,
    #[serde(default)]
    pub limits: Limits,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    #[serde(default)]
    machines: Vec<Machine>,
}

fn store_path() -> std::path::PathBuf {
    crate::paths::root().join("machines.json")
}

fn load() -> Vec<Machine> {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Store>(&text).ok())
        .map(|store| store.machines)
        .unwrap_or_default()
}

fn save(machines: &[Machine]) -> Result<(), String> {
    let body = serde_json::to_string_pretty(&json!({"v": 1, "machines": machines}))
        .map_err(crate::i18n::io)?;
    crate::paths::write_private(&store_path(), &body).map_err(crate::i18n::io)
}

/// One argument for `ssh`: no options, spaces or control characters.
pub fn valid_destination(ssh: &str) -> bool {
    !ssh.is_empty()
        && ssh.len() <= 255
        && !ssh.starts_with('-')
        && ssh.chars().all(|c| c.is_ascii_graphic())
}

#[derive(Default)]
pub struct Service {
    connections: Mutex<HashMap<String, Arc<Connection>>>,
    app: Mutex<Option<AppHandle>>,
    /// The last `claude auth status` seen on each machine.
    claude: Mutex<HashMap<String, login::ClaudeStatus>>,
    /// Whether Codex is installed and signed in on each machine, once checked (ADR 0105).
    codex: Mutex<HashMap<String, codex::CodexStatus>>,
    /// Whether agents are enabled on each machine, once checked (ADR 0103).
    agents: Mutex<HashMap<String, agents::AgentsStatus>>,
    /// Conversations whose process ended while no desktop was attached: their next launch adopts
    /// the finished terminal to collect its output, hooks and transcript, instead of starting
    /// Claude again.
    pub(crate) recovering: Mutex<std::collections::HashSet<String>>,
}

#[derive(Serialize, Clone)]
pub struct MachineView {
    #[serde(flatten)]
    pub machine: Machine,
    pub phase: Phase,
    pub detail: Option<String>,
    pub host: Option<Value>,
    pub update_pending: bool,
    /// Claude's sign-in on the machine, once checked.
    pub claude: Option<login::ClaudeStatus>,
    /// Codex's installation and sign-in on the machine, once checked.
    pub codex: Option<codex::CodexStatus>,
    pub agents: Option<agents::AgentsStatus>,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    fn connection_for(&self, machine: Machine) -> Arc<Connection> {
        let mut connections = lock(&self.connections);
        if let Some(existing) = connections.get(&machine.id) {
            existing.set_machine(machine);
            return existing.clone();
        }
        let app = lock(&self.app).clone();
        let changed_app = app.clone();
        let connection = Connection::new(
            machine.clone(),
            Box::new(move || {
                if let Some(app) = &changed_app {
                    publish(app);
                }
            }),
            Box::new(move |connection, wake| {
                if let Some(app) = &app {
                    if wake == Wake::Attached {
                        attached(app, connection);
                    }
                    reattach_running(app, connection);
                }
            }),
        );
        connections.insert(machine.id, connection.clone());
        connection
    }

    pub fn get(&self, id: &str) -> Option<Arc<Connection>> {
        lock(&self.connections).get(id).cloned()
    }

    pub fn machine(&self, id: &str) -> Option<Machine> {
        self.get(id).map(|c| c.machine())
    }

    /// The machine's connection when attached, or a localized offline error.
    pub fn connected(&self, id: &str) -> Result<Arc<Connection>, String> {
        let connection = self
            .get(id)
            .ok_or_else(|| crate::i18n::t("err.remote.unknown"))?;
        if connection.connected() {
            Ok(connection)
        } else {
            Err(crate::i18n::t(connection::OFFLINE))
        }
    }

    pub fn views(&self) -> Vec<MachineView> {
        let claude = lock(&self.claude).clone();
        let codex = lock(&self.codex).clone();
        let agents = lock(&self.agents).clone();
        let mut views: Vec<MachineView> = lock(&self.connections)
            .values()
            .map(|c| {
                let (phase, detail) = c.phase();
                MachineView {
                    machine: c.machine(),
                    phase,
                    detail,
                    host: c.ready(),
                    update_pending: c.update_pending(),
                    claude: claude.get(&c.id).cloned(),
                    codex: codex.get(&c.id).cloned(),
                    agents: agents.get(&c.id).cloned(),
                }
            })
            .collect();
        views.sort_by(|a, b| a.machine.name.cmp(&b.machine.name));
        views
    }
}

pub(crate) fn publish_claude(app: &AppHandle, id: &str, status: login::ClaudeStatus) {
    lock(&app.state::<Service>().claude).insert(id.into(), status);
    publish(app);
}

pub(crate) fn publish_codex(app: &AppHandle, id: &str, status: codex::CodexStatus) {
    lock(&app.state::<Service>().codex).insert(id.into(), status);
    publish(app);
}

pub(crate) fn publish_agents(app: &AppHandle, id: &str, status: agents::AgentsStatus) {
    lock(&app.state::<Service>().agents).insert(id.into(), status);
    publish(app);
}

fn publish(app: &AppHandle) {
    let views = app.state::<Service>().views();
    let _ = app.emit("machines", &views);
}

/// Connect every saved machine and keep them attached.
pub fn start(app: &AppHandle) {
    let service = app.state::<Service>();
    *lock(&service.app) = Some(app.clone());
    for machine in load() {
        service.connection_for(machine).start();
    }
    let app = app.clone();
    std::thread::spawn(move || fill_origins(&app, None));
}

/// Record the normalized `origin` of projects that lack one: local clones through Git here, and
/// the clones on `machine` through it. A machine's clone of a local project then appears as that
/// project. Projects without an origin are asked again on the next pass.
pub fn fill_origins(app: &AppHandle, machine: Option<&Arc<Connection>>) {
    let state = app.state::<crate::AppState>();
    let missing: Vec<(String, Option<String>, String)> = lock(&state.board)
        .projects
        .iter()
        .filter(|p| p.origin.is_none() && p.host.as_deref() == machine.map(|c| c.id.as_str()))
        .map(|p| (p.id.clone(), p.host.clone(), p.path.clone()))
        .collect();
    let found: Vec<(String, String)> = missing
        .into_iter()
        .filter_map(|(id, _, path)| {
            let url = match machine {
                Some(connection) => workspace::origin_url(connection, &path),
                None => workspace::local_origin_url(&path),
            }?;
            Some((id, workspace::normalize_origin(&url)?))
        })
        .collect();
    if found.is_empty() {
        return;
    }
    {
        let mut board = lock(&state.board);
        for (id, origin) in found {
            if let Some(project) = board.projects.iter_mut().find(|p| p.id == id) {
                project.origin = Some(origin);
            }
        }
    }
    crate::state::publish(app);
}

/// Refresh what only changes between attachments: Claude's and Codex's sign-ins, whether agents
/// are enabled and the clones' origins.
fn attached(app: &AppHandle, connection: &Arc<Connection>) {
    if let Ok(status) = login::status(connection) {
        publish_claude(app, &connection.id, status);
    }
    if let Ok(status) = codex::status(connection) {
        publish_codex(app, &connection.id, status);
    }
    agents::refresh(app, connection);
    fill_origins(app, Some(connection));
    codex_login::sweep(app, connection);
}

/// After (re)attaching or an agent's change, adopt what agents created on the machine, then resume
/// the conversations whose processes the machine still runs but this app does not show live, so
/// their status and notifications update without opening each tab.
fn reattach_running(app: &AppHandle, connection: &Arc<Connection>) {
    let synced = registry::sync(app, connection);
    let Ok(list) = connection.request("terminal_list", json!({})) else {
        return;
    };
    let terminals: Vec<(String, bool)> = list
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|t| Some((t["id"].as_str()?.to_owned(), t["running"] == json!(true))))
                .collect()
        })
        .unwrap_or_default();
    end_orphan_engines(connection, &list);
    // An adopted conversation whose process is gone (the machine restarted) resumes from its
    // transcript when opened, like one this app started.
    if let Some(home) = connection.home() {
        for (session, cwd) in &synced.adopted {
            if !terminals.iter().any(|(id, _)| id == session) {
                agent::bind_transcript(&connection.id, &home, cwd, session);
            }
        }
    }
    let state = app.state::<AppState>();
    let tabs: Vec<(String, crate::state::ProviderId)> = lock(&state.board)
        .workspaces
        .iter()
        .filter(|ws| ws.host.as_deref() == Some(connection.id.as_str()) && !ws.archived)
        .flat_map(|ws| {
            ws.tabs.iter().map(|t| {
                (
                    t.id.clone(),
                    t.choice.as_ref().map_or(ws.agent, |c| c.agent),
                )
            })
        })
        .collect();
    let service = app.state::<Service>();
    for (session, running) in terminals {
        let agent = tabs.iter().find(|(id, _)| *id == session).map(|(_, a)| *a);
        if agent == Some(crate::state::ProviderId::Codex) && !running {
            // A Codex conversation's content comes from its engine, which ended with it: there is
            // nothing to collect, and its next launch resumes the thread. A conversation this app
            // still follows collects the exit itself.
            if !lock(&state.sessions.conversations).contains_key(&session) {
                connection.notify("terminal_forget", json!({"id": session}));
            }
            continue;
        }
        if agent.is_none() {
            // A finished terminal no conversation will read again (an archived workspace, a
            // closed tab, the sign-in) only holds memory on the host.
            if !running {
                connection.notify("terminal_forget", json!({"id": session}));
            }
            continue;
        }
        if lock(&state.sessions.conversations).contains_key(&session) {
            continue;
        }
        if !running {
            lock(&service.recovering).insert(session.clone());
        }
        let _ = crate::session::resume_conversation(app, &session);
        lock(&service.recovering).remove(&session);
    }
}

/// End Codex engines whose conversation's TUI is gone (ADR 0105). An engine ends with its TUI
/// when this app sees the TUI exit; this covers TUIs that exited while no desktop was attached.
fn end_orphan_engines(connection: &Connection, list: &Value) {
    let Some(terminals) = list.as_array() else {
        return;
    };
    let now = now_secs();
    for engine in terminals {
        let Some(id) = engine["id"].as_str() else {
            continue;
        };
        let Some(session) = prometeu_host::codex::companion_of(id) else {
            continue;
        };
        if id != prometeu_host::codex::engine_terminal(session) {
            continue;
        }
        let tui = terminals.iter().find(|t| t["id"] == json!(session));
        if codex::orphan(engine, tui, now) {
            connection.notify("terminal_signal", json!({"id": id, "signal": "terminate"}));
        }
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Deserialize)]
pub struct MachineInput {
    pub id: Option<String>,
    pub name: String,
    pub ssh: String,
    #[serde(default)]
    pub limits: Limits,
}

#[tauri::command]
pub fn machines(service: State<Service>) -> Vec<MachineView> {
    service.views()
}

/// Add or edit a machine. Saving does not install anything; it starts connecting.
#[tauri::command(async)]
pub fn machine_save(app: AppHandle, machine: MachineInput) -> Result<MachineView, String> {
    let ssh = machine.ssh.trim().to_string();
    if !valid_destination(&ssh) {
        return Err(crate::i18n::t("err.remote.destination"));
    }
    if !Limits::valid(machine.limits.memory_max.trim())
        || !Limits::valid(machine.limits.cpu_quota.trim())
    {
        return Err(crate::i18n::t("err.remote.limits"));
    }
    let mut machines = load();
    if let Some(id) = machine.id.as_deref() {
        // A sign-in started for an earlier destination must not continue on the new one.
        if machines.iter().any(|m| m.id == id && m.ssh != ssh) {
            codex_login::abandon(&app, id);
        }
    }
    let id = machine
        .id
        .filter(|id| prometeu_host::valid_id(id))
        .unwrap_or_else(|| {
            let base: String = machine
                .name
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .to_string();
            let base = if base.is_empty() {
                "machine".into()
            } else {
                base
            };
            let mut id = base.clone();
            let mut n = 2;
            while machines.iter().any(|m| m.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            id
        });
    let name = if machine.name.trim().is_empty() {
        ssh.clone()
    } else {
        machine.name.trim().to_string()
    };
    let saved = Machine {
        id: id.clone(),
        name,
        ssh,
        limits: Limits {
            memory_max: machine.limits.memory_max.trim().into(),
            cpu_quota: machine.limits.cpu_quota.trim().into(),
        },
    };
    match machines.iter_mut().find(|m| m.id == id) {
        Some(existing) => *existing = saved.clone(),
        None => machines.push(saved.clone()),
    }
    save(&machines)?;
    let service = app.state::<Service>();
    let connection = service.connection_for(saved);
    connection.reconnect();
    publish(&app);
    service
        .views()
        .into_iter()
        .find(|v| v.machine.id == id)
        .ok_or_else(|| crate::i18n::t("err.remote.unknown"))
}

/// Forget a machine. Workspaces on it must be removed first; nothing on the machine is deleted.
#[tauri::command(async)]
pub fn machine_remove(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    if lock(&state.board)
        .workspaces
        .iter()
        .any(|ws| ws.host.as_deref() == Some(id.as_str()))
    {
        return Err(crate::i18n::t("err.remote.inUse"));
    }
    let mut machines = load();
    machines.retain(|m| m.id != id);
    save(&machines)?;
    {
        let mut board = lock(&state.board);
        board
            .projects
            .retain(|p| p.host.as_deref() != Some(id.as_str()));
    }
    crate::state::publish(&app);
    codex_login::abandon(&app, &id);
    let service = app.state::<Service>();
    if let Some(connection) = lock(&service.connections).remove(&id) {
        connection.stop();
    }
    publish(&app);
    Ok(())
}

/// Install or update the host on the machine, then connect to it.
#[tauri::command(async)]
pub fn machine_install(app: AppHandle, id: String) -> Result<MachineView, String> {
    let service = app.state::<Service>();
    let connection = service
        .get(&id)
        .ok_or_else(|| crate::i18n::t("err.remote.unknown"))?;
    install::install(&connection.machine().ssh)?;
    connection.reconnect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !connection.connected() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    publish(&app);
    service
        .views()
        .into_iter()
        .find(|v| v.machine.id == id)
        .ok_or_else(|| crate::i18n::t("err.remote.unknown"))
}

/// Check Claude's sign-in on the machine again.
#[tauri::command(async)]
pub fn machine_claude_status(app: AppHandle, id: String) -> Result<login::ClaudeStatus, String> {
    let connection = app.state::<Service>().connected(&id)?;
    let status = login::status(&connection)?;
    publish_claude(&app, &id, status.clone());
    // "Check again" also covers Codex, which signs in on the machine itself (ADR 0105).
    if let Ok(codex) = codex::status(&connection) {
        publish_codex(&app, &id, codex);
    }
    Ok(status)
}

/// Retry the connection now instead of waiting for the next backoff step.
#[tauri::command]
pub fn machine_connect(app: AppHandle, id: String) -> Result<(), String> {
    let connection = app
        .state::<Service>()
        .get(&id)
        .ok_or_else(|| crate::i18n::t("err.remote.unknown"))?;
    connection.reconnect();
    Ok(())
}

/// Processes owned by the connected host, including idle and unadopted terminals.
#[tauri::command(async)]
pub fn machine_terminals(app: AppHandle, id: String) -> Result<Value, String> {
    app.state::<Service>()
        .connected(&id)?
        .request("terminal_list", json!({}))
}

/// End one host-owned process without deleting its workspace or transcript.
#[tauri::command(async)]
pub fn machine_terminal_stop(
    app: AppHandle,
    id: String,
    terminal: String,
    instance: String,
) -> Result<(), String> {
    app.state::<Service>()
        .connected(&id)?
        .request(
            "terminal_stop",
            json!({"id": terminal, "instance": instance}),
        )
        .map_err(|error| match error.as_str() {
            "terminal_changed" | "unknown_terminal" => crate::i18n::t("err.remote.processChanged"),
            "unknown method terminal_stop" => crate::i18n::t("machines.processes.unsupported"),
            _ => error,
        })?;
    Ok(())
}

/// The computers attached to the machine's host now, this one included (ADR 0104).
#[tauri::command(async)]
pub fn machine_clients(app: AppHandle, id: String) -> Result<Value, String> {
    app.state::<Service>()
        .connected(&id)?
        .request("clients_list", json!({}))
        .map_err(|error| match error.as_str() {
            "unknown method clients_list" => crate::i18n::t("machines.clients.unsupported"),
            _ => error,
        })
}

/// Disconnect another computer from the machine. It stops reconnecting until its person asks;
/// removing its SSH key is what revokes its access.
#[tauri::command(async)]
pub fn machine_client_detach(app: AppHandle, id: String, device: String) -> Result<(), String> {
    app.state::<Service>()
        .connected(&id)?
        .request("client_detach", json!({"device": device}))
        .map_err(|error| match error.as_str() {
            "unknown_client" => crate::i18n::t("machines.clients.gone"),
            _ => error,
        })?;
    Ok(())
}

/// Resource use of the machine and of each of its workspaces' slices.
#[tauri::command(async)]
pub fn machine_stats(app: AppHandle, state: State<AppState>, id: String) -> Result<Value, String> {
    let workspaces: Vec<String> = lock(&state.board)
        .workspaces
        .iter()
        .filter(|ws| ws.host.as_deref() == Some(id.as_str()))
        .map(|ws| ws.id.clone())
        .collect();
    app.state::<Service>()
        .connected(&id)?
        .request("stats", json!({"workspaces": workspaces}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_are_single_ssh_arguments() {
        assert!(valid_destination("vps"));
        assert!(valid_destination("me@100.1.2.3"));
        assert!(!valid_destination("-oProxyCommand=x"));
        assert!(!valid_destination("a b"));
        assert!(!valid_destination(""));
    }

    #[test]
    fn limits_use_systemd_syntax_only() {
        assert!(Limits::valid("4G"));
        assert!(Limits::valid("200%"));
        assert!(Limits::valid(""));
        assert!(!Limits::valid("1G; rm"));
    }
}

/// Opt-in journey against a real machine: `PROMETEU_REMOTE_TEST_SSH=vps cargo test vps_journey
/// -- --ignored --nocapture`. It installs this checkout's Linux host build there.
#[cfg(test)]
mod journey {
    use super::connection::{Connection, TerminalSink};
    use super::terminal::Remote;
    use super::*;
    use std::io::Read;
    use std::time::{Duration, Instant};

    fn wait(until: impl Fn() -> bool, seconds: u64) {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while !until() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    #[ignore]
    fn vps_journey() {
        let Ok(target) = std::env::var("PROMETEU_REMOTE_TEST_SSH") else {
            return;
        };
        install::install(&target).expect("install");
        let connection = Connection::new(
            Machine {
                id: "journey".into(),
                name: "journey".into(),
                ssh: target,
                limits: Limits {
                    memory_max: "512M".into(),
                    cpu_quota: "50%".into(),
                },
            },
            Box::new(|| {}),
            Box::new(|_, _| {}),
        );
        connection.start();
        wait(|| connection.connected(), 30);
        let home = connection.home().unwrap();
        let id = format!("journey-{}", uuid::Uuid::new_v4().simple());
        let workspace = uuid::Uuid::new_v4().to_string();
        connection
            .request(
                "terminal_open",
                json!({"id": id, "workspace": workspace, "cwd": home, "cols": 80, "rows": 24,
                       "argv": ["sh", "-c", "i=0; while [ $i -lt 60 ]; do i=$((i+1)); echo tick-$i; sleep 0.1; done"],
                       "limits": {"memory_max": "512M", "cpu_quota": "50%"}}),
            )
            .expect("open");
        let remote = Remote::new(connection.clone(), &id);
        connection.subscribe(&id, remote.clone());
        let mut started = remote.started();
        let (sender, received) = std::sync::mpsc::channel::<String>();
        let mut output = started.output;
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(count) = output.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let _ = sender.send(String::from_utf8_lossy(&buffer[..count]).to_string());
            }
        });
        let mut text = String::new();
        while !text.contains("tick-10") {
            text.push_str(&received.recv_timeout(Duration::from_secs(10)).unwrap());
        }
        // Drop the SSH connection mid-output, as sleep or a network change would.
        let before = remote.last_seq();
        connection.reconnect();
        wait(|| !connection.connected(), 10);
        wait(|| connection.connected(), 60);
        let code = started.waiter.wait().expect("exit code");
        while let Ok(chunk) = received.recv_timeout(Duration::from_millis(500)) {
            text.push_str(&chunk);
        }
        assert_eq!(code, 0);
        assert!(remote.last_seq() > before);
        let ticks: Vec<u32> = text
            .split(|c: char| c.is_whitespace())
            .filter_map(|word| word.strip_prefix("tick-").and_then(|n| n.parse().ok()))
            .collect();
        assert_eq!(
            ticks,
            (1..=60).collect::<Vec<_>>(),
            "no gap or duplicate: {text:?}"
        );
        let stats = connection
            .request("stats", json!({"workspaces": [workspace]}))
            .expect("stats");
        eprintln!("stats: {stats}");
        let _ = connection.request("terminal_forget", json!({"id": id}));
        connection.stop();
    }
}
