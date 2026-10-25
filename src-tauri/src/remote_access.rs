//! Opt-in personal Cloud bridge. The backend owns the connection; no provider credential leaves it.
use crate::{cloud, embedded_mcp, lock::lock, mcp_access::Client, paths, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tungstenite::{client::IntoClientRequest, stream::MaybeTlsStream, Message, WebSocket};

static SETTINGS: Mutex<()> = Mutex::new(());
static OUTBOX: Mutex<()> = Mutex::new(());
static ONLINE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
struct Settings {
    enabled: bool,
    account: String,
    /// Pin source paths as well as IDs; replacing a project must not expand an old grant.
    projects: BTreeMap<String, String>,
}

#[derive(Default, Deserialize, Serialize)]
struct Outbox {
    observed: BTreeMap<String, Value>,
    events: Vec<Value>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Owner {
    account: String,
    projects: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct Status {
    enabled: bool,
    online: bool,
    projects: Vec<String>,
    manage_url: Option<String>,
}

fn settings() -> Result<Settings, String> {
    read("remote-access.json")
}

fn read<T: serde::de::DeserializeOwned + Default>(name: &str) -> Result<T, String> {
    match std::fs::read(paths::root().join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "remote_storage".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(_) => Err("remote_storage".into()),
    }
}

fn save(name: &str, value: &impl Serialize) -> Result<(), String> {
    paths::write_private(
        &paths::root().join(name),
        &serde_json::to_string(value).map_err(|_| "remote_storage")?,
    )
}

fn active(saved: &Settings) -> bool {
    saved.enabled && cloud::remote_identity().is_ok_and(|identity| identity.0 == saved.account)
}

#[tauri::command]
pub fn remote_access_status() -> Result<Status, String> {
    let saved = settings()?;
    Ok(Status {
        enabled: active(&saved),
        online: ONLINE.load(Ordering::Relaxed) && active(&saved),
        projects: saved.projects.keys().cloned().collect(),
        manage_url: cloud::remote_identity()
            .ok()
            .map(|identity| format!("{}/remote", identity.1)),
    })
}

#[tauri::command]
pub fn remote_access_save(
    app: AppHandle,
    enabled: bool,
    projects: Vec<String>,
) -> Result<Status, String> {
    let _guard = lock(&SETTINGS);
    let mut saved = settings()?;
    if enabled {
        saved.account = cloud::remote_identity()?.0;
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        if projects.is_empty() || projects.len() > 100 {
            return Err("remote_projects_required".into());
        }
        saved.projects = projects
            .into_iter()
            .map(|id| {
                board
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| (id, p.path.clone()))
                    .ok_or("project_not_found".into())
            })
            .collect::<Result<_, String>>()?;
    }
    saved.enabled = enabled;
    save("remote-access.json", &saved)?;
    ONLINE.store(false, Ordering::Relaxed);
    remote_access_status()
}

fn client(
    saved: &Settings,
    board: &crate::state::Board,
    request: &Value,
) -> Result<Client, String> {
    let id = request["client"].as_str().ok_or("unauthorized")?;
    uuid::Uuid::parse_str(id).map_err(|_| "unauthorized")?;
    let ids = request["projects"]
        .as_array()
        .filter(|p| !p.is_empty() && p.len() <= 100)
        .ok_or("unauthorized")?;
    let projects = ids
        .iter()
        .map(|id| {
            let id = id.as_str().ok_or("unauthorized")?;
            let path = saved.projects.get(id).ok_or("unauthorized")?;
            if !board.projects.iter().any(|p| p.id == id && p.path == *path) {
                return Err("unauthorized");
            }
            Ok(path.clone())
        })
        .collect::<Result<Vec<_>, &str>>()?;
    Ok(Client {
        id: format!("remote:{id}"),
        conversation: None,
        projects,
    })
}

fn execute(app: &AppHandle, account: &str, request: &Value) -> Result<Value, String> {
    let _guard = lock(&SETTINGS);
    let saved = settings()?;
    if !active(&saved) || saved.account != account {
        return Err("remote_disabled".into());
    }
    let name = request["name"].as_str().ok_or("invalid_tool")?;
    let definition = embedded_mcp::tools()
        .into_iter()
        .find(|t| t["name"] == name)
        .ok_or("invalid_tool")?;
    let scope = if definition["annotations"]["readOnlyHint"] == true {
        "prometeu:read"
    } else {
        "prometeu:write"
    };
    if !request["scope"]
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .any(|s| s == scope)
    {
        return Err("unauthorized".into());
    }
    let state = app.state::<AppState>();
    let owner = client(&saved, &lock(&state.board), request)?;
    let mut owners: BTreeMap<String, Owner> = read("remote-owners.json")?;
    if owners
        .get(&owner.id)
        .is_some_and(|o| o.account != saved.account)
    {
        return Err("unauthorized".into());
    }
    owners.insert(
        owner.id.clone(),
        Owner {
            account: saved.account.clone(),
            projects: saved.projects.clone(),
        },
    );
    save("remote-owners.json", &owners)?;
    embedded_mcp::dispatch(app, &owner, name, &request["arguments"])
}

fn event_value(d: &crate::delegation::Delegation, workspace_state: &str) -> Option<Value> {
    let execution = d.executions.last()?;
    Some(
        json!({"agent_id": d.id, "execution_id": execution.id, "state": execution.state, "workspace_state": workspace_state,
        "outcome": execution.outcome, "input_required": !d.requests.is_empty(),
        "request_ids": d.requests.iter().filter_map(|r| r["requestId"].as_str()).filter(|id| id.len() <= 128).take(16).collect::<Vec<_>>()}),
    )
}

/// Called after ordered publication, outside conversation locks. A durable outbox survives disconnects.
pub fn observe(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let (delegation, workspace_state) = {
        let board = lock(&state.board);
        let delegation = board.delegations.iter().find(|d| d.id == id);
        let workspace =
            delegation.and_then(|d| board.workspaces.iter().find(|w| w.id == d.workspace));
        let status = match workspace {
            Some(w) if w.failed.is_some() => "failed",
            Some(w) if w.preparing => "preparing",
            Some(w) if !w.archived && !w.cleaned => "ready",
            _ => "unavailable",
        };
        (delegation.cloned(), status)
    };
    let Some(d) = delegation else {
        return;
    };
    let Some(owner) = d.owner.strip_prefix("remote:") else {
        return;
    };
    let Ok(owners) = read::<BTreeMap<String, Owner>>("remote-owners.json") else {
        return;
    };
    let Some(saved) = owners.get(&d.owner) else {
        return;
    };
    let Some(value) = event_value(&d, workspace_state) else {
        return;
    };
    let projects: Vec<_> = saved
        .projects
        .iter()
        .filter(|(_, path)| d.repository_heads.contains_key(*path))
        .map(|(id, _)| id.clone())
        .collect();
    if projects.len() != d.repository_heads.len() {
        return;
    }
    let _guard = lock(&OUTBOX);
    let Ok(mut outbox) = read::<Outbox>("remote-events.json") else {
        return;
    };
    if outbox.observed.get(id) == Some(&value) {
        return;
    }
    // Bound disk use. Leave the observation unchanged so reconnect reconciliation retries it.
    if outbox.events.len() >= 10_000 {
        return;
    }
    outbox.observed.insert(id.to_string(), value.clone());
    outbox.events.push(
        json!({"type":"event", "key":uuid::Uuid::new_v4().to_string(),
        "account":saved.account, "client":owner, "projects":projects, "value":value}),
    );
    let _ = save("remote-events.json", &outbox);
}

fn ticket(app: &AppHandle, saved: &Settings) -> Result<Value, String> {
    let identity = cloud::remote_identity()?;
    if identity.0 != saved.account {
        return Err("account_changed".into());
    }
    let projects: Vec<_> = {
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        board
            .projects
            .iter()
            .filter(|p| saved.projects.get(&p.id) == Some(&p.path))
            .map(|p| json!({"id":p.id,"name":p.name}))
            .collect()
    };
    let response = cloud::remote_api(
        &saved.account,
        "/api/remote/ticket",
        json!({"device":identity.2,"label":identity.3,"projects":projects}),
        Duration::from_secs(12),
    )?;
    if response.0 != 200 {
        return Err("remote_unavailable".into());
    }
    Ok(response.1)
}

fn socket_url(origin: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(origin).map_err(|_| "invalid_gateway")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || local && url.scheme() == "http")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("invalid_gateway".into());
    }
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).map_err(|_| "invalid_gateway")?;
    url.set_path("/remote/connect");
    Ok(url)
}

fn connect(value: &Value) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let url = socket_url(value["gateway"].as_str().ok_or("invalid_gateway")?)?;
    let token = value["ticket"]
        .as_str()
        .filter(|v| {
            v.len() == 43
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
        .ok_or("invalid_ticket")?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| "invalid_gateway")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "invalid_ticket")?,
    );
    let address = (
        url.host_str().ok_or("invalid_gateway")?,
        url.port_or_known_default().ok_or("invalid_gateway")?,
    )
        .to_socket_addrs()
        .map_err(|_| "remote_network")?
        .next()
        .ok_or("remote_network")?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|_| "remote_network")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "remote_network")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "remote_network")?;
    let config = tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let (mut socket, _) = tungstenite::client_tls_with_config(request, stream, Some(config), None)
        .map_err(|_| "remote_network")?;
    let tcp = match socket.get_mut() {
        MaybeTlsStream::Plain(tcp) => tcp,
        MaybeTlsStream::Rustls(tls) => &mut tls.sock,
        _ => return Err("remote_tls".into()),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| "remote_network")?;
    Ok(socket)
}

fn send(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, value: Value) -> Result<(), String> {
    socket
        .send(Message::Text(value.to_string().into()))
        .map_err(|_| "remote_network".into())
}

fn run(app: &AppHandle, saved: &Settings) -> Result<(), String> {
    let first = ticket(app, saved)?;
    let mut socket = connect(&first)?;
    send(
        &mut socket,
        json!({"type":"tools", "tools":embedded_mcp::tools()}),
    )?;
    let mut renewed = Instant::now();
    let mut sent = Instant::now() - Duration::from_secs(5);
    let mut webhook_acks = std::collections::HashSet::<String>::new();
    let mut gateway_acks = std::collections::HashSet::<String>::new();
    let mut worker: Option<std::thread::JoinHandle<Value>> = None;
    ONLINE.store(true, Ordering::Relaxed);
    loop {
        if settings()? != *saved || !active(saved) {
            let _ = socket.close(None);
            return Ok(());
        }
        if renewed.elapsed() >= Duration::from_secs(25) {
            let fresh = ticket(app, saved)?;
            if fresh["device"] != first["device"] || fresh["gateway"] != first["gateway"] {
                return Err("device_changed".into());
            }
            send(
                &mut socket,
                json!({"type":"renew", "ticket":fresh["ticket"]}),
            )?;
            renewed = Instant::now();
        }
        if worker.as_ref().is_some_and(|job| job.is_finished()) {
            send(
                &mut socket,
                worker.take().unwrap().join().map_err(|_| "remote_worker")?,
            )?;
        }
        if sent.elapsed() >= Duration::from_secs(5) {
            // Reconcile host stop/preparation paths that do not emit a provider event.
            let state = app.state::<AppState>();
            let ids: Vec<_> = lock(&state.board)
                .delegations
                .iter()
                .filter(|d| d.owner.starts_with("remote:"))
                .map(|d| d.id.clone())
                .collect();
            for id in ids {
                observe(app, &id);
            }
            let events = {
                let _guard = lock(&OUTBOX);
                read::<Outbox>("remote-events.json")?.events
            };
            for mut event in events
                .into_iter()
                .filter(|e| e["account"] == saved.account)
                .take(5)
            {
                event.as_object_mut().unwrap().remove("account");
                let key = event["key"].as_str().unwrap_or("").to_string();
                if !webhook_acks.contains(&key) {
                    if let Ok((status, _)) = cloud::remote_api(
                        &saved.account,
                        "/api/remote/events",
                        json!({"event":event}),
                        Duration::from_secs(2),
                    ) {
                        // A revoked integration has no remaining delivery authority; drop its receipt.
                        if matches!(status, 200 | 400 | 404) {
                            webhook_acks.insert(key.clone());
                        }
                    }
                }
                if !active(saved) {
                    return Err("account_changed".into());
                }
                send(&mut socket, event)?;
            }
            let confirmed: Vec<_> = webhook_acks.intersection(&gateway_acks).cloned().collect();
            if !confirmed.is_empty() {
                let _guard = lock(&OUTBOX);
                let mut outbox: Outbox = read("remote-events.json")?;
                outbox.events.retain(|e| {
                    e["account"] != saved.account || !confirmed.iter().any(|key| e["key"] == *key)
                });
                save("remote-events.json", &outbox)?;
                for key in confirmed {
                    webhook_acks.remove(&key);
                    gateway_acks.remove(&key);
                }
            }
            sent = Instant::now();
        }
        match socket.read() {
            Ok(Message::Text(raw)) => {
                let value: Value =
                    serde_json::from_str(&raw).map_err(|_| "invalid_remote_frame")?;
                match value["type"].as_str() {
                    Some("call") if worker.is_none() => {
                        let app = app.clone();
                        let account = saved.account.clone();
                        worker = Some(std::thread::spawn(move || {
                            let result = execute(&app, &account, &value);
                            let state = app.state::<AppState>();
                            let ids: Vec<_> = lock(&state.board)
                                .delegations
                                .iter()
                                .filter(|d| d.owner.starts_with("remote:"))
                                .map(|d| d.id.clone())
                                .collect();
                            for id in ids {
                                observe(&app, &id);
                            }
                            match result {
                                Ok(result) => {
                                    json!({"type":"result", "id":value["id"], "result":result})
                                }
                                Err(error) => {
                                    json!({"type":"result", "id":value["id"], "error":error})
                                }
                            }
                        }));
                    }
                    Some("call") => send(
                        &mut socket,
                        json!({"type":"result","id":value["id"],"error":"busy"}),
                    )?,
                    Some("ack") => {
                        if let Some(key) = value["key"].as_str() {
                            gateway_acks.insert(key.to_string());
                        }
                    }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => return Err("remote_disconnected".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return Err("remote_disconnected".into()),
        }
    }
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut delay = 1;
        loop {
            if let Ok(saved) = settings() {
                if active(&saved) {
                    let state = app.state::<AppState>();
                    let ids: Vec<_> = lock(&state.board)
                        .delegations
                        .iter()
                        .map(|d| d.id.clone())
                        .collect();
                    for id in ids {
                        observe(&app, &id);
                    }
                    if run(&app, &saved).is_ok() {
                        delay = 1;
                    } else {
                        delay = (delay * 2).min(30);
                    }
                }
            }
            ONLINE.store(false, Ordering::Relaxed);
            std::thread::sleep(Duration::from_secs(delay));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_event_contract_uses_only_task_metadata() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/cloud-api.json")).unwrap();
        let expected = &fixture["remote_event"]["value"];
        let mut delegation: crate::delegation::Delegation = serde_json::from_value(json!({
            "id":expected["agent_id"], "owner":"remote:client", "workspace":"workspace1", "task":"private prompt",
            "request_key":"key", "request_hash":"hash", "repository_heads":{}, "background":null,"requests":[],
            "executions":[{"id":expected["execution_id"],"state":"completed","source":"coordinator","accepted_at":0,"outcome":"ok","request_hash":null}]
        })).unwrap();
        assert_eq!(event_value(&delegation, "ready").unwrap(), *expected);
        delegation
            .requests
            .push(json!({"requestId":"question1","text":"private question"}));
        let event = event_value(&delegation, "failed").unwrap();
        assert_eq!(event["workspace_state"], "failed");
        assert_eq!(event["input_required"], true);
        assert!(!event.to_string().contains("private"));
    }

    #[test]
    fn outbound_socket_authenticates_with_a_header_and_reads_gateway_frames() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let gateway = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut socket = tungstenite::accept_hdr(
                stream,
                |request: &tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri().path(), "/remote/connect");
                    assert!(request.uri().query().is_none());
                    assert_eq!(
                        request.headers()["Authorization"],
                        format!("Bearer {}", "t".repeat(43))
                    );
                    Ok(response)
                },
            )
            .unwrap();
            socket
                .send(Message::Text("{\"type\":\"connected\"}".into()))
                .unwrap();
            let message: Value =
                serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
            assert_eq!(message, json!({"type":"tools","tools":[]}));
        });
        let mut socket = connect(&json!({"gateway":gateway,"ticket":"t".repeat(43)})).unwrap();
        let frame: Value =
            serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
        assert_eq!(frame["type"], "connected");
        send(&mut socket, json!({"type":"tools","tools":[]})).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn remote_scope_pins_project_paths_and_rejects_untrusted_gateways() {
        let board: crate::state::Board = serde_json::from_value(json!({"stages":[],"workspaces":[],"projects":[{"id":"project1","name":"Project","path":"/repo"}]})).unwrap();
        let mut saved = Settings {
            enabled: true,
            account: "test".into(),
            projects: BTreeMap::from([("project1".into(), "/repo".into())]),
        };
        let request = json!({"client":uuid::Uuid::new_v4().to_string(),"projects":["project1"]});
        let owner = client(&saved, &board, &request).unwrap();
        assert!(owner.allows(&board, "/repo"));
        assert!(!owner.allows(&board, "/other"));
        saved
            .projects
            .insert("project1".into(), "/different".into());
        assert!(client(&saved, &board, &request).is_err());
        for origin in [
            "http://example.com",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com?secret=x",
        ] {
            assert!(socket_url(origin).is_err());
        }
        assert_eq!(
            socket_url("https://relay.example").unwrap().as_str(),
            "wss://relay.example/remote/connect"
        );
    }
}
