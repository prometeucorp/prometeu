//! Durable, content-free supervision events. The host records facts before notifying Cloud;
//! Cloud owns external webhook delivery, while the supervisor continues to use the host MCP.
use crate::registry::{self, Registry, Store, Workspace};
use crate::server::now;
use prometeu_core::lock::lock;
use prometeu_core::supervision::{Binding, Event, Execution, Kind, Observation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const CONFIG: &str = "supervision.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Supervisor {
    id: String,
    name: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    version: u32,
    url: String,
    executor_id: String,
    supervisor: Supervisor,
    token: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PullRequest {
    repository: String,
    number: u64,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct State {
    #[serde(default)]
    executions: BTreeMap<String, Execution>,
    #[serde(default)]
    pull_requests: BTreeMap<String, PullRequest>,
    #[serde(default)]
    outbox: VecDeque<Event>,
}

pub(crate) struct Supervision {
    root: PathBuf,
    executor_id: String,
    registry: Arc<Store>,
    config: Mutex<Option<Config>>,
    generation: Mutex<u64>,
    changed: Condvar,
}

fn load(root: &Path) -> Result<Option<Config>, String> {
    match std::fs::read(root.join(CONFIG)) {
        Ok(bytes) => {
            let config: Config = serde_json::from_slice(&bytes)
                .map_err(|_| "supervision configuration is unreadable")?;
            if config.version != 1 {
                return Err("unsupported supervision configuration version".into());
            }
            origin(&config.url)?;
            Ok(Some(config))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("could not read supervision configuration".into()),
    }
}

fn http() -> Result<reqwest::blocking::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "could not initialize supervision HTTP client".into())
}

fn origin(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "invalid Cloud origin")?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Cloud origin must use HTTPS without credentials, path, query or fragment".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn handshake(url: &str, token: &str) -> Result<Supervisor, String> {
    let response = http()?
        .get(format!("{url}/api/supervision"))
        .bearer_auth(token)
        .send()
        .map_err(|_| "Cloud supervision handshake failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "Cloud supervision handshake returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read Cloud supervision response")?;
    if bytes.len() > 65_536 {
        return Err("Cloud supervision response is too large".into());
    }
    #[derive(Deserialize)]
    struct Response {
        supervisor: Supervisor,
    }
    let result: Response =
        serde_json::from_slice(&bytes).map_err(|_| "invalid Cloud supervision response")?;
    if result.supervisor.id.is_empty()
        || result.supervisor.id.len() > 128
        || result.supervisor.name.trim().is_empty()
        || result.supervisor.name.len() > 256
    {
        return Err("invalid Cloud supervisor identity".into());
    }
    Ok(result.supervisor)
}

/// This administrative channel uses the existing private host socket without replacing the
/// desktop attachment. Credentials never enter command arguments or an MCP response.
pub(crate) fn command(
    root: &Path,
    action: Option<&str>,
    url: Option<&str>,
    discard_pending: bool,
) -> Result<Value, String> {
    let request = match action {
        Some("setup") => {
            let url = origin(url.ok_or("supervision setup requires --url")?)?;
            let mut token = String::new();
            std::io::stdin().take(8193).read_to_string(&mut token).map_err(|_| "could not read token from stdin")?;
            let token = token.trim();
            if token.is_empty() || token.len() > 8192 || token.chars().any(char::is_whitespace) {
                return Err("stdin must contain one supervision ingest token".into());
            }
            let supervisor = handshake(&url, token)?;
            json!({"action":"setup", "url":url, "token":token, "supervisor":supervisor})
        }
        Some("status") => {
            let mut result = status(load(root)?.as_ref());
            result["pending_events"] = json!(Store::new(root).read()?.supervision.outbox.len());
            return Ok(result);
        }
        Some("identity") => json!({"action":"identity"}),
        Some("test") => return test_connection(root),
        Some("disable") => json!({"action":"disable"}),
        Some("reset") if discard_pending => json!({"action":"reset", "discard_pending":true}),
        Some("reset") => return Err("supervision reset requires --discard-pending; it removes supervision bindings and undelivered events, never workspaces or code".into()),
        _ => return Err("usage: prometeu-host supervision setup --url ORIGIN < token-file | identity | status | test | disable [--root DIR]".into()),
    };
    let mut stream = crate::client::connect_or_start(root, None)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    writeln!(stream, "{}", json!({"supervision":request}))
        .map_err(|_| "host supervision request failed")?;
    let line = crate::server::read_line(&mut BufReader::new(stream))?
        .ok_or("host closed supervision request")?;
    let response: Value =
        serde_json::from_str(&line).map_err(|_| "invalid host supervision reply")?;
    match response.get("error").and_then(Value::as_str) {
        Some(error) => Err(error.to_string()),
        None => Ok(response["result"].clone()),
    }
}

/// Only an explicit setup diagnostic polls delivery receipts; the publisher remains event-driven.
fn test_connection(root: &Path) -> Result<Value, String> {
    let config = load(root)?.ok_or("supervisor_not_available")?;
    let token = config.token.as_deref().ok_or("supervisor_not_available")?;
    let client = http()?;
    let id = crate::agents::uuid()?;
    let decode = |response: reqwest::blocking::Response| -> Result<Value, String> {
        if !response.status().is_success() {
            return Err(format!(
                "Cloud supervision test returned HTTP {}",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        response
            .take(16_385)
            .read_to_end(&mut bytes)
            .map_err(|_| "could not read supervision test response")?;
        if bytes.len() > 16_384 {
            return Err("supervision test response is too large".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid supervision test response")?;
        if value["id"] != id
            || !matches!(
                value["status"].as_str(),
                Some("pending" | "sending" | "delivered" | "cancelled")
            )
        {
            return Err("invalid supervision test response".into());
        }
        // Never forward arbitrary response fields or destination errors to the desktop.
        Ok(json!({"status": value["status"], "failed": value["last_error"].is_string()}))
    };
    let mut result = decode(
        client
            .post(format!("{}/api/supervision/tests", config.url))
            .bearer_auth(token)
            .json(&json!({"id": id, "executor_id": config.executor_id}))
            .send()
            .map_err(|_| "Cloud supervision test failed")?,
    )?;
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while matches!(result["status"].as_str(), Some("pending" | "sending"))
        && result["failed"] != true
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_secs(1));
        result = decode(
            client
                .get(format!("{}/api/supervision/tests/{id}", config.url))
                .bearer_auth(token)
                .send()
                .map_err(|_| "Cloud supervision test failed")?,
        )?;
    }
    Ok(result)
}

fn status(config: Option<&Config>) -> Value {
    match config {
        Some(config) => json!({"enabled":config.token.is_some(), "url":config.url,
            "executor_id":config.executor_id, "supervisor":config.supervisor}),
        None => json!({"enabled":false}),
    }
}

fn repository_name(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 100
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}

fn github_repository(value: &str) -> Option<String> {
    let path = if let Some(path) = value
        .strip_prefix("git@github.com:")
        .or_else(|| value.strip_prefix("github.com/"))
    {
        path.to_string()
    } else {
        let url = reqwest::Url::parse(value).ok()?;
        if url.host_str() != Some("github.com") {
            return None;
        }
        url.path().trim_start_matches('/').to_string()
    };
    let path = path
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'));
    repository_name(path).then(|| path.to_string())
}

impl Supervision {
    pub(crate) fn new(root: &Path, registry: Arc<Store>) -> Result<Arc<Self>, String> {
        let config = load(root)?;
        let identity_path = root.join("supervision-executor");
        let executor_id = if let Some(config) = &config {
            config.executor_id.clone()
        } else {
            match std::fs::read_to_string(&identity_path) {
                Ok(id)
                    if !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b)) =>
                {
                    id
                }
                Ok(_) => return Err("invalid supervision executor identity".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    crate::agents::uuid()?
                }
                Err(_) => return Err("could not read supervision executor identity".into()),
            }
        };
        registry::write_private(&identity_path, executor_id.as_bytes())?;
        let instance = Arc::new(Self {
            root: root.into(),
            executor_id,
            registry,
            config: Mutex::new(config),
            generation: Mutex::new(0),
            changed: Condvar::new(),
        });
        instance.reconcile()?;
        Ok(instance)
    }

    pub(crate) fn start(self: &Arc<Self>) {
        let this = self.clone();
        std::thread::spawn(move || this.deliver());
    }

    fn wake(&self) {
        *lock(&self.generation) += 1;
        self.changed.notify_one();
    }

    pub(crate) fn configure(&self, request: &Value) -> Result<Value, String> {
        let mut current = lock(&self.config);
        let next = match request["action"].as_str() {
            Some("identity") => return Ok(json!({"executor_id": self.executor_id})),
            Some("setup") => {
                let url = origin(request["url"].as_str().ok_or("missing Cloud origin")?)?;
                let token = request["token"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 8192)
                    .ok_or("missing supervision token")?;
                let supervisor: Supervisor = serde_json::from_value(request["supervisor"].clone())
                    .map_err(|_| "missing supervisor identity")?;
                if let Some(old) = current.as_ref() {
                    if old.url != url || old.supervisor.id != supervisor.id {
                        let registry = self.registry.read()?;
                        if !registry.supervision.outbox.is_empty()
                            || registry
                                .workspaces
                                .iter()
                                .any(|w| w.supervision.is_some() && !w.closed)
                        {
                            return Err("detach supervised workspaces and deliver pending events before changing Cloud or supervisor".into());
                        }
                    }
                }
                Some(Config {
                    version: 1,
                    url,
                    token: Some(token.into()),
                    supervisor,
                    executor_id: self.executor_id.clone(),
                })
            }
            Some("disable") => current.clone().map(|mut config| {
                config.token = None;
                config
            }),
            Some("reset") if request["discard_pending"] == true => {
                let disabled = current.clone().map(|mut config| {
                    config.token = None;
                    config
                });
                // Disable delivery first. If clearing state fails, pending events remain safely
                // disabled instead of being published by a half-completed administrative reset.
                if let Some(config) = &disabled {
                    registry::write_private(
                        &self.root.join(CONFIG),
                        &serde_json::to_vec(config).map_err(|e| e.to_string())?,
                    )?;
                }
                *current = disabled.clone();
                self.registry.update(|registry| {
                    for workspace in &mut registry.workspaces {
                        workspace.supervision = None;
                    }
                    registry.supervision = State::default();
                    Ok(())
                })?;
                disabled
            }
            _ => return Err("unknown supervision configuration action".into()),
        };
        if let Some(config) = &next {
            registry::write_private(
                &self.root.join(CONFIG),
                &serde_json::to_vec(config).map_err(|e| e.to_string())?,
            )?;
        }
        *current = next;
        let result = status(current.as_ref());
        drop(current);
        self.wake();
        Ok(result)
    }

    pub(crate) fn list(&self) -> Value {
        match lock(&self.config).as_ref().filter(|c| c.token.is_some()) {
            Some(config) => {
                json!([{"id":config.supervisor.id, "name":config.supervisor.name, "executor_id":config.executor_id}])
            }
            None => json!([]),
        }
    }

    pub(crate) fn binding(&self, id: &str) -> Result<Binding, String> {
        let config = lock(&self.config);
        let config = config
            .as_ref()
            .filter(|c| c.token.is_some() && c.supervisor.id == id)
            .ok_or("supervisor_not_available")?;
        Ok(Binding {
            supervisor_id: config.supervisor.id.clone(),
        })
    }

    fn event(
        config: &Config,
        workspace: &Workspace,
        kind: Kind,
        conversation: Option<&str>,
        execution: Option<&str>,
        pr: Option<&PullRequest>,
    ) -> Result<Event, String> {
        Ok(Event {
            version: 1,
            id: crate::agents::uuid()?,
            supervisor_id: workspace
                .supervision
                .as_ref()
                .ok_or("workspace_not_supervised")?
                .supervisor_id
                .clone(),
            executor_id: config.executor_id.clone(),
            workspace_id: workspace.id.clone(),
            kind,
            occurred_at: now(),
            conversation_id: conversation.map(str::to_string),
            execution_id: execution.map(str::to_string),
            repository: pr.map(|p| p.repository.clone()),
            branch: (!workspace.branch.is_empty()
                && workspace.branch.len() <= 255
                && !workspace
                    .branch
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control()))
            .then(|| workspace.branch.clone()),
            pull_request: pr.map(|p| p.number),
            head_sha: None,
        })
    }

    fn push(
        registry: &mut Registry,
        config: &Config,
        workspace: &Workspace,
        kind: Kind,
        conversation: Option<&str>,
        execution: Option<&str>,
    ) -> Result<(), String> {
        let mut event = Self::event(
            config,
            workspace,
            kind,
            conversation,
            execution,
            registry.supervision.pull_requests.get(&workspace.id),
        )?;
        if event.repository.is_none() {
            event.repository = registry
                .project(&workspace.project)
                .and_then(|p| p.origin.as_deref())
                .and_then(github_repository);
        }
        registry.supervision.outbox.push_back(event);
        Ok(())
    }

    pub(crate) fn record_workspace(&self, workspace: Workspace) -> Result<(), String> {
        let config = lock(&self.config);
        self.registry.update(|registry| {
            if let Some(binding) = &workspace.supervision {
                let config = config
                    .as_ref()
                    .filter(|c| c.token.is_some() && c.supervisor.id == binding.supervisor_id)
                    .ok_or("supervisor_not_available")?;
                Self::push(
                    registry,
                    config,
                    &workspace,
                    Kind::WorkspaceCreated,
                    None,
                    None,
                )?;
                for conversation in &workspace.conversations {
                    registry
                        .supervision
                        .executions
                        .entry(conversation.id.clone())
                        .or_default()
                        .observe(Observation::Started, &crate::agents::uuid()?);
                }
            }
            registry.put_workspace(workspace);
            Ok(())
        })?;
        self.wake();
        Ok(())
    }

    pub(crate) fn set_binding(
        &self,
        id: &str,
        supervisor: Option<&str>,
        active: &[(String, Vec<String>, bool)],
    ) -> Result<(), String> {
        let config = lock(&self.config);
        let config = config.as_ref().ok_or("supervisor_not_available")?;
        if supervisor.is_some_and(|id| id != config.supervisor.id || config.token.is_none()) {
            return Err("supervisor_not_available".into());
        }
        self.registry.update(|registry| {
            let workspace = registry.workspace_mut(id).ok_or("unknown_workspace")?;
            if workspace.closed && supervisor.is_some() {
                return Err("workspace_closed".into());
            }
            if workspace
                .supervision
                .as_ref()
                .map(|b| b.supervisor_id.as_str())
                == supervisor
            {
                return Ok(());
            }
            let previous = workspace.clone();
            if previous
                .supervision
                .as_ref()
                .is_some_and(|b| b.supervisor_id != config.supervisor.id)
            {
                return Err("supervisor_not_available".into());
            }
            workspace.supervision = supervisor.map(|id| Binding {
                supervisor_id: id.into(),
            });
            let workspace = workspace.clone();
            if supervisor.is_none() {
                for conversation in &workspace.conversations {
                    registry.supervision.executions.remove(&conversation.id);
                }
                Self::push(
                    registry,
                    config,
                    &previous,
                    Kind::WorkspaceUnsupervised,
                    None,
                    None,
                )?;
                registry.supervision.pull_requests.remove(id);
            } else {
                for (conversation, background, main_done) in active {
                    let execution = registry
                        .supervision
                        .executions
                        .entry(conversation.clone())
                        .or_default();
                    let id = crate::agents::uuid()?;
                    execution.observe(Observation::Started, &id);
                    for child in background {
                        execution.observe(Observation::BackgroundStarted(child.clone()), &id);
                    }
                    if *main_done {
                        execution.observe(Observation::Completed, &id);
                    }
                }
                Self::push(
                    registry,
                    config,
                    &workspace,
                    Kind::WorkspaceCreated,
                    None,
                    None,
                )?;
            }
            Ok(())
        })?;
        self.wake();
        Ok(())
    }

    pub(crate) fn link_pr(&self, id: &str, repository: &str, number: u64) -> Result<(), String> {
        if !repository_name(repository) || number == 0 || number > 2_147_483_647 {
            return Err("invalid_pull_request".into());
        }
        let config = lock(&self.config);
        let config = config.as_ref().ok_or("supervisor_not_available")?;
        self.registry.update(|registry| {
            let workspace = registry
                .workspace_mut(id)
                .ok_or("unknown_workspace")?
                .clone();
            if workspace.closed {
                return Err("workspace_closed".into());
            }
            if workspace.supervision.is_none() {
                return Err("workspace_not_supervised".into());
            }
            if registry
                .supervision
                .pull_requests
                .get(id)
                .is_some_and(|p| p.repository == repository && p.number == number)
            {
                return Ok(());
            }
            registry.supervision.pull_requests.insert(
                id.into(),
                PullRequest {
                    repository: repository.into(),
                    number,
                },
            );
            Self::push(
                registry,
                config,
                &workspace,
                Kind::WorkspacePrLinked,
                None,
                None,
            )
        })?;
        self.wake();
        Ok(())
    }

    pub(crate) fn close(&self, id: &str) -> Result<(), String> {
        let config = lock(&self.config);
        self.registry.update(|registry| {
            let Some(workspace) = registry.workspace_mut(id) else {
                return Ok(());
            };
            if workspace.closed {
                return Ok(());
            }
            workspace.closed = true;
            let workspace = workspace.clone();
            if workspace.supervision.is_some() {
                let config = config.as_ref().ok_or("supervisor_not_available")?;
                Self::push(
                    registry,
                    config,
                    &workspace,
                    Kind::WorkspaceClosed,
                    None,
                    None,
                )?;
            }
            Ok(())
        })?;
        self.wake();
        Ok(())
    }

    pub(crate) fn observe(
        &self,
        conversation: &str,
        observation: Observation,
    ) -> Result<(), String> {
        let config = lock(&self.config);
        let Some(config) = config.as_ref() else {
            return Ok(());
        };
        if !self
            .registry
            .read()?
            .conversation(conversation)
            .is_some_and(|(w, _)| w.supervision.is_some() && !w.closed)
        {
            return Ok(());
        }
        let next = crate::agents::uuid()?;
        self.registry.update(|registry| {
            let Some((workspace, _)) = registry.conversation(conversation) else {
                return Ok(());
            };
            if workspace.supervision.is_none() || workspace.closed {
                return Ok(());
            }
            let workspace = workspace.clone();
            let execution = registry
                .supervision
                .executions
                .entry(conversation.into())
                .or_default();
            let kind = execution.observe(observation, &next);
            let id = execution.id.clone();
            if let Some(kind) = kind {
                Self::push(
                    registry,
                    config,
                    &workspace,
                    kind,
                    Some(conversation),
                    id.as_deref(),
                )?;
            }
            Ok(())
        })?;
        self.wake();
        Ok(())
    }

    fn reconcile(&self) -> Result<(), String> {
        let config = lock(&self.config);
        let Some(config) = config.as_ref() else {
            return Ok(());
        };
        self.registry.update(|registry| {
            for workspace in registry
                .workspaces
                .clone()
                .into_iter()
                .filter(|w| w.supervision.is_some() && !w.closed)
            {
                for conversation in &workspace.conversations {
                    if let Some(execution) =
                        registry.supervision.executions.get_mut(&conversation.id)
                    {
                        let kind = execution.reconcile_restart();
                        let id = execution.id.clone();
                        if let Some(kind) = kind {
                            Self::push(
                                registry,
                                config,
                                &workspace,
                                kind,
                                Some(&conversation.id),
                                id.as_deref(),
                            )?;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn deliver_one(&self, client: &reqwest::blocking::Client) -> Result<bool, String> {
        let config = lock(&self.config).clone();
        let Some(config) = config.filter(|c| c.token.is_some()) else {
            return Ok(false);
        };
        let Some(event) = self.registry.read()?.supervision.outbox.front().cloned() else {
            return Ok(false);
        };
        if event.supervisor_id != config.supervisor.id {
            return Err("pending event belongs to another supervisor".into());
        }
        let response = client
            .post(format!("{}/api/supervision/events", config.url))
            .bearer_auth(config.token.as_ref().unwrap())
            .json(&event)
            .send()
            .map_err(|_| "Cloud supervision delivery failed")?;
        if !response.status().is_success() {
            return Err(format!(
                "Cloud supervision delivery returned HTTP {}",
                response.status().as_u16()
            ));
        }
        self.registry.update(|registry| {
            if registry
                .supervision
                .outbox
                .front()
                .is_some_and(|e| e.id == event.id)
            {
                registry.supervision.outbox.pop_front();
            }
            Ok(())
        })?;
        Ok(true)
    }

    fn deliver(&self) {
        let client = match http() {
            Ok(client) => client,
            Err(error) => {
                eprintln!("{error}");
                return;
            }
        };
        let mut backoff = 1;
        loop {
            let observed = *lock(&self.generation);
            match self.deliver_one(&client) {
                Ok(true) => {
                    backoff = 1;
                    continue;
                }
                Ok(false) => {
                    let guard = lock(&self.generation);
                    drop(
                        self.changed
                            .wait_while(guard, |generation| *generation == observed)
                            .unwrap_or_else(|e| e.into_inner()),
                    );
                }
                Err(error) => {
                    eprintln!("{error}; pending supervision events retained");
                    let guard = lock(&self.generation);
                    let deadline = std::time::Instant::now() + Duration::from_secs(backoff);
                    drop(
                        self.changed
                            .wait_timeout_while(guard, Duration::from_secs(backoff), |_| {
                                std::time::Instant::now() < deadline
                            })
                            .unwrap_or_else(|e| e.into_inner()),
                    );
                    backoff = (backoff * 2).min(60);
                }
            }
        }
    }
}

/// Only final hooks complete work. SessionStart merely means Claude is ready for input.
pub(crate) fn observation(payload: &Value) -> Option<Observation> {
    let event = payload["hook_event_name"].as_str()?;
    // Child hooks share the parent's configured hook command. Only explicit child lifecycle
    // hooks affect its background set; a child's Stop must never finish the main execution.
    if payload["agent_id"].as_str().is_some() && !matches!(event, "SubagentStart" | "SubagentStop")
    {
        return None;
    }
    Some(match event {
        "UserPromptSubmit" => Observation::Started,
        "PreToolUse" if payload["tool_name"] == "AskUserQuestion" => Observation::Blocked,
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => Observation::Working,
        "PermissionRequest" => Observation::Blocked,
        "Stop" => Observation::Completed,
        "StopFailure" => Observation::Failed,
        "SessionEnd" => Observation::Exited { success: false },
        "SubagentStart" => {
            Observation::BackgroundStarted(payload["agent_id"].as_str()?.to_string())
        }
        "SubagentStop" => {
            Observation::BackgroundFinished(payload["agent_id"].as_str()?.to_string())
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Conversation, Creator, Project};
    use std::io::BufRead;

    struct Fixture {
        root: PathBuf,
        store: Arc<Store>,
        supervision: Arc<Supervision>,
    }
    impl Fixture {
        fn new(url: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("supervision-{}", crate::agents::uuid().unwrap()));
            std::fs::create_dir_all(&root).unwrap();
            let store = Arc::new(Store::new(&root));
            let supervision = Supervision::new(&root, store.clone()).unwrap();
            supervision
                .configure(
                    &json!({"action":"setup", "url":url, "token":"private-ingest-token",
                "supervisor":{"id":"supervisor-one", "name":"Reviewer"}}),
                )
                .unwrap();
            store
                .update(|r| {
                    r.add_project(Project {
                        path: "/repo".into(),
                        name: "repo".into(),
                        origin: Some("git@github.com:owner/repo.git".into()),
                        added_at: 1,
                        added_by: Creator {
                            kind: "agent".into(),
                            name: None,
                        },
                    });
                    Ok(())
                })
                .unwrap();
            Self {
                root,
                store,
                supervision,
            }
        }
        fn workspace(&self, supervised: bool) -> Workspace {
            Workspace {
                id: "workspace".into(),
                project: "/repo".into(),
                title: "Private task title".into(),
                branch: "feature/work".into(),
                base: "HEAD".into(),
                worktree: "/private/repo/work".into(),
                created_at: 1,
                created_by: Creator {
                    kind: "agent".into(),
                    name: None,
                },
                closed: false,
                archived: Some(false),
                keep_branch: false,
                conversations: vec![Conversation {
                    id: "conversation".into(),
                    title: "Private prompt".into(),
                    created_at: 1,
                    permission: "auto".into(),
                    ..Default::default()
                }],
                supervision: supervised.then(|| Binding {
                    supervisor_id: "supervisor-one".into(),
                }),
            }
        }
        fn events(&self) -> Vec<Event> {
            self.store
                .read()
                .unwrap()
                .supervision
                .outbox
                .into_iter()
                .collect()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn executor_identity_survives_setup_disable_restart_and_legacy_configuration() {
        let fixture = Fixture::new("https://cloud.example");
        let identity = fixture
            .supervision
            .configure(&json!({"action":"identity"}))
            .unwrap();
        assert_eq!(
            identity["executor_id"],
            status(load(&fixture.root).unwrap().as_ref())["executor_id"]
        );
        fixture
            .supervision
            .configure(&json!({"action":"disable"}))
            .unwrap();
        let reopened = Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        assert_eq!(
            reopened.configure(&json!({"action":"identity"})).unwrap(),
            identity
        );
        // Configurations created before the identity file retain their executor ID.
        std::fs::remove_file(fixture.root.join("supervision-executor")).unwrap();
        let legacy = Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        assert_eq!(
            legacy.configure(&json!({"action":"identity"})).unwrap(),
            identity
        );
        std::fs::remove_file(fixture.root.join(CONFIG)).unwrap();
        let unconfigured = Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        assert_eq!(
            unconfigured
                .configure(&json!({"action":"identity"}))
                .unwrap(),
            identity
        );
        assert!(unconfigured.list().as_array().unwrap().is_empty());
    }

    #[test]
    fn host_test_requires_a_receiver_receipt_and_never_exposes_credentials() {
        use std::io::BufRead;
        for delivered in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let fixture = Fixture::new(&format!("http://{}", listener.local_addr().unwrap()));
            let executor = fixture.supervision.executor_id.clone();
            let peer = std::thread::spawn(move || {
                let mut test_id = String::new();
                for attempt in 0..=usize::from(delivered) {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut request = String::new();
                    reader.read_line(&mut request).unwrap();
                    let mut length = 0;
                    let mut authorized = false;
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        if header == "\r\n" {
                            break;
                        }
                        if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                            length = value.trim().parse::<usize>().unwrap();
                        }
                        authorized |= header
                            .to_lowercase()
                            .starts_with("authorization: bearer private-ingest-token");
                    }
                    assert!(authorized);
                    if attempt == 0 {
                        assert!(request.starts_with("POST /api/supervision/tests "));
                        let mut body = vec![0; length];
                        reader.read_exact(&mut body).unwrap();
                        let value: Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(value["executor_id"], executor);
                        test_id = value["id"].as_str().unwrap().to_owned();
                    } else {
                        assert!(
                            request.starts_with(&format!("GET /api/supervision/tests/{test_id} "))
                        );
                    }
                    let body = json!({"id":test_id, "status": if attempt == 0 { "pending" } else { "delivered" },
                        "last_error": if delivered { None } else { Some("http_error") }, "secret":"never-forward"}).to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .unwrap();
                }
            });
            let result = test_connection(&fixture.root).unwrap();
            peer.join().unwrap();
            assert_eq!(
                result["status"],
                if delivered { "delivered" } else { "pending" }
            );
            assert_eq!(result["failed"], !delivered);
            assert!(!result.to_string().contains("never-forward"));
            assert!(fixture.events().is_empty());
        }
    }

    fn cloud(statuses: Vec<u16>) -> (String, std::sync::mpsc::Receiver<Value>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for status in statuses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                assert!(first.starts_with("POST /api/supervision/events "));
                let mut length = 0;
                let mut authenticated = false;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    authenticated |= line
                        .trim()
                        .eq_ignore_ascii_case("authorization: Bearer private-ingest-token");
                }
                assert!(authenticated);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                sender.send(serde_json::from_slice(&body).unwrap()).unwrap();
                write!(stream, "HTTP/1.1 {status} Result\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
            }
        });
        (url, received)
    }

    #[test]
    fn failed_delivery_survives_restart_and_retries_same_event_before_later_events() {
        let (url, requests) = cloud(vec![503, 410, 200, 200]);
        let fixture = Fixture::new(&url);
        fixture
            .supervision
            .record_workspace(fixture.workspace(true))
            .unwrap();
        fixture
            .supervision
            .observe("conversation", Observation::Completed)
            .unwrap();
        assert!(fixture.supervision.deliver_one(&http().unwrap()).is_err());
        let failed = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        let restarted = Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        assert_eq!(fixture.events().len(), 2);
        assert!(restarted
            .deliver_one(&http().unwrap())
            .unwrap_err()
            .contains("HTTP 410"));
        assert_eq!(
            requests.recv_timeout(Duration::from_secs(2)).unwrap(),
            failed
        );
        assert_eq!(fixture.events().len(), 2);
        assert!(restarted.deliver_one(&http().unwrap()).unwrap());
        assert_eq!(
            requests.recv_timeout(Duration::from_secs(2)).unwrap(),
            failed
        );
        assert!(restarted.deliver_one(&http().unwrap()).unwrap());
        let completed = requests.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(completed["kind"], "execution.completed");
        assert_eq!(completed["repository"], "owner/repo");
        assert_eq!(failed["repository"], "owner/repo");
        assert!(fixture.events().is_empty());
        restarted
            .observe("conversation", Observation::Completed)
            .unwrap();
        assert!(fixture.events().is_empty());
        let serialized = completed.to_string();
        for secret in [
            "private-ingest-token",
            "Private task title",
            "Private prompt",
            "/private/repo/work",
        ] {
            assert!(!serialized.contains(secret));
        }
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(fixture.root.join(CONFIG))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!fixture
            .supervision
            .list()
            .to_string()
            .contains("private-ingest-token"));
    }

    #[test]
    fn execution_hooks_wait_for_children_and_report_restart_failure_once() {
        let fixture = Fixture::new("https://cloud.example");
        fixture
            .supervision
            .record_workspace(fixture.workspace(true))
            .unwrap();
        for payload in [
            json!({"hook_event_name":"SessionStart"}),
            json!({"hook_event_name":"Stop", "agent_id":"child"}),
        ] {
            assert!(observation(&payload).is_none());
        }
        for payload in [
            json!({"hook_event_name":"SubagentStart", "agent_id":"child"}),
            json!({"hook_event_name":"PreToolUse", "tool_name":"AskUserQuestion"}),
            json!({"hook_event_name":"PermissionRequest"}),
            json!({"hook_event_name":"Stop"}),
        ] {
            fixture
                .supervision
                .observe("conversation", observation(&payload).unwrap())
                .unwrap();
        }
        assert_eq!(
            fixture.events().iter().map(|e| e.kind).collect::<Vec<_>>(),
            vec![Kind::WorkspaceCreated, Kind::ExecutionBlocked]
        );
        fixture
            .supervision
            .observe(
                "conversation",
                Observation::BackgroundFinished("child".into()),
            )
            .unwrap();
        assert_eq!(
            fixture.events().last().unwrap().kind,
            Kind::ExecutionCompleted
        );
        fixture
            .supervision
            .observe("conversation", Observation::Started)
            .unwrap();
        Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        Supervision::new(&fixture.root, fixture.store.clone()).unwrap();
        assert_eq!(
            fixture
                .events()
                .iter()
                .filter(|e| e.kind == Kind::ExecutionFailed)
                .count(),
            1
        );
    }

    #[test]
    fn binding_merge_active_attachment_detachment_and_explicit_reset_preserve_workspaces() {
        let fixture = Fixture::new("https://cloud.example");
        fixture
            .supervision
            .record_workspace(fixture.workspace(false))
            .unwrap();
        fixture
            .supervision
            .set_binding(
                "workspace",
                Some("supervisor-one"),
                &[("conversation".into(), vec!["child".into()], true)],
            )
            .unwrap();
        fixture
            .store
            .update(|r| {
                r.merge_workspace(fixture.workspace(false));
                Ok(())
            })
            .unwrap();
        assert!(fixture.store.read().unwrap().workspaces[0]
            .supervision
            .is_some());
        assert_eq!(
            fixture.events().last().unwrap().kind,
            Kind::WorkspaceCreated
        );
        fixture
            .supervision
            .observe(
                "conversation",
                Observation::BackgroundFinished("child".into()),
            )
            .unwrap();
        assert_eq!(
            fixture.events().last().unwrap().kind,
            Kind::ExecutionCompleted
        );
        fixture
            .supervision
            .link_pr("workspace", "owner/repo", 42)
            .unwrap();
        assert!(fixture
            .supervision
            .link_pr("workspace", "owner/repo", 2_147_483_648)
            .is_err());
        fixture.supervision.close("workspace").unwrap();
        fixture
            .supervision
            .set_binding("workspace", None, &[])
            .unwrap();
        assert_eq!(
            fixture.events().last().unwrap().kind,
            Kind::WorkspaceUnsupervised
        );
        let identity = status(lock(&fixture.supervision.config).as_ref())["executor_id"].clone();
        fixture
            .supervision
            .configure(&json!({"action":"disable"}))
            .unwrap();
        assert!(!fixture.supervision.deliver_one(&http().unwrap()).unwrap());
        assert!(!fixture.events().is_empty());
        assert!(fixture.supervision.configure(&json!({"action":"setup", "url":"https://cloud.example", "token":"new", "supervisor":{"id":"other", "name":"Other"}})).is_err());
        assert!(fixture
            .supervision
            .configure(&json!({"action":"reset"}))
            .is_err());
        fixture
            .supervision
            .configure(&json!({"action":"reset", "discard_pending":true}))
            .unwrap();
        assert!(fixture.events().is_empty());
        assert_eq!(fixture.store.read().unwrap().workspaces.len(), 1);
        assert_eq!(
            status(lock(&fixture.supervision.config).as_ref())["executor_id"],
            identity
        );
        assert!(fixture.supervision.list().as_array().unwrap().is_empty());
    }

    #[test]
    fn metadata_and_cloud_origins_are_bounded_before_delivery() {
        for input in [
            "https://github.com/owner/repo.git",
            "git@github.com:owner/repo.git",
            "ssh://git@github.com/owner/repo",
            "github.com/owner/repo",
        ] {
            assert_eq!(github_repository(input).as_deref(), Some("owner/repo"));
        }
        assert_eq!(github_repository("https://not-github.com/owner/repo"), None);
        for input in [
            "http://cloud.example",
            "https://secret@cloud.example",
            "https://cloud.example/path",
            "https://cloud.example?token=secret",
        ] {
            assert!(origin(input).is_err());
        }
        for branch in [
            "b".repeat(256),
            "feature/\u{2009}task".into(),
            "feature/\u{0085}task".into(),
            "feature/\u{007f}task".into(),
        ] {
            let fixture = Fixture::new("https://cloud.example");
            let mut workspace = fixture.workspace(true);
            workspace.branch = branch.clone();
            fixture.supervision.record_workspace(workspace).unwrap();
            assert!(fixture.events()[0].branch.is_none());
            assert_eq!(fixture.store.read().unwrap().workspaces[0].branch, branch);
        }
        let fixture = Fixture::new("https://cloud.example");
        let mut workspace = fixture.workspace(true);
        workspace.branch = "feature/caf\u{00e9}".into();
        fixture.supervision.record_workspace(workspace).unwrap();
        assert_eq!(
            fixture.events()[0].branch.as_deref(),
            Some("feature/caf\u{00e9}")
        );
    }
}
