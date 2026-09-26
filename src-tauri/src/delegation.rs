//! Local delegation use cases. Ownership is a durable client identity; a workspace is only
//! the environment. This module schedules no work and never grants control over unrelated tabs.

use crate::lock::lock;
use crate::mcp_access::Client;
use crate::state::{Board, Status};
use crate::{chat, conversation, dock, pty, session, AppState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub state: String,
    pub source: String,
    pub accepted_at: u64,
    pub outcome: Option<String>,
    pub request_hash: Option<String>,
}

impl Execution {
    fn new(source: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            state: "queued".into(),
            source: source.into(),
            accepted_at: conversation::now(),
            outcome: None,
            request_hash: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Delegation {
    /// The delegated agent and its conversation share one stable identity in this first version.
    pub id: String,
    pub owner: String,
    pub workspace: String,
    pub task: String,
    /// Caller-supplied key prevents retrying creation from creating a second worktree.
    pub request_key: String,
    pub request_hash: String,
    pub repository_heads: BTreeMap<String, String>,
    #[serde(default)]
    pub permission: Option<crate::actions::Permission>,
    pub executions: Vec<Execution>,
    /// None means no authoritative background signal has been observed in this process lifetime.
    pub background: Option<Vec<Value>>,
    pub requests: Vec<Value>,
}

impl Delegation {
    pub fn reconcile_restart(&mut self, pending: bool) {
        self.background = None;
        self.requests.clear();
        for run in &mut self.executions {
            if run.state == "running" || (run.state == "queued" && !pending) {
                run.state = "stopped".into();
            }
        }
    }

    fn observe(&mut self, event: &Value) -> bool {
        // A resumed turn invalidates the outcome recorded while background tasks held completion;
        // otherwise a later drain would report the old result for a turn still running.
        if conversation::agent_activity(event) {
            if let Some(run) = self.executions.last_mut().filter(|r| r.state == "running") {
                run.outcome = None;
            }
        }
        match event["type"].as_str() {
            Some("session.state") if event["state"] == "busy" => {
                // Accepted follow-ups do not identify a separate provider turn. Keep the active
                // execution until its terminal event rather than orphaning its coordinator ID.
                if self
                    .executions
                    .last()
                    .is_none_or(|r| !matches!(r.state.as_str(), "queued" | "running"))
                {
                    self.executions.push(Execution::new("conversation"));
                }
                self.executions.last_mut().unwrap().state = "running".into();
            }
            Some("session.state") if event["state"] == "starting" => {
                self.background = None;
                self.requests.clear();
            }
            Some("assistant.started")
                if self
                    .executions
                    .last()
                    .is_none_or(|r| r.state == "completed" || r.state == "stopped") =>
            {
                let mut run = Execution::new("background");
                run.state = "running".into();
                self.executions.push(run);
            }
            Some("turn.completed") => {
                // Native subagents outlive the main turn. Record the outcome, but hold completion
                // until they drain: a caller must never read a finished execution from a
                // conversation that still rejects its next message as busy.
                // An interruption ends the children too; the last observation of them is stale.
                let interrupted = event["outcome"] == "interrupted";
                if interrupted {
                    self.background = None;
                }
                let running = working(&self.background);
                if let Some(run) = self.executions.last_mut().filter(|r| r.state == "running") {
                    run.outcome = event["outcome"].as_str().map(str::to_string);
                    if !running {
                        run.state = "completed".into();
                    }
                }
                self.requests.clear();
            }
            Some("background.changed") => {
                self.background = event["tasks"].as_array().cloned();
                // Draining alone never completes a turn; only a held terminal settles here.
                if !working(&self.background) {
                    if let Some(run) = self
                        .executions
                        .last_mut()
                        .filter(|r| r.state == "running" && r.outcome.is_some())
                    {
                        run.state = "completed".into();
                    }
                }
            }
            Some("request.opened") => {
                self.requests
                    .retain(|r| r["requestId"] != event["requestId"]);
                self.requests.push(event.clone());
            }
            Some("request.closed") => {
                self.requests
                    .retain(|r| r["requestId"] != event["requestId"]);
            }
            _ => return false,
        }
        true
    }
}

/// Called while the conversation buffer lock establishes event order. Mutate only memory here;
/// publication and I/O happen after the conversation locks are released.
pub fn observe(app: &AppHandle, id: &str, event: &Value) {
    let state = app.state::<AppState>();
    if let Some(d) = lock(&state.board)
        .delegations
        .iter_mut()
        .find(|d| d.id == id)
    {
        d.observe(event);
    };
}

pub fn publish_observation(app: &AppHandle, id: &str, event: &Value) {
    if !matches!(
        event["type"].as_str(),
        Some(
            "session.state"
                | "assistant.started"
                | "turn.completed"
                | "background.changed"
                | "request.opened"
                | "request.closed"
        )
    ) {
        return;
    }
    let state = app.state::<AppState>();
    let delegated = lock(&state.board).delegations.iter().any(|d| d.id == id);
    if delegated {
        crate::state::publish(app);
    }
}

pub fn stopped(state: &AppState, id: &str) {
    if let Some(d) = lock(&state.board)
        .delegations
        .iter_mut()
        .find(|d| d.id == id)
    {
        d.background = None;
        d.requests.clear();
        if let Some(run) = d.executions.last_mut().filter(|r| r.state == "running") {
            run.state = "stopped".into();
        }
    }
}

/// The authenticated caller cannot provide or override its owner identity in tool arguments.
fn owned<'a>(board: &'a Board, client: &Client, id: &str) -> Result<&'a Delegation, String> {
    client.validate(board)?;
    board
        .delegations
        .iter()
        .find(|d| d.owner == client.id && d.id == id && in_scope(board, client, d))
        .ok_or_else(|| "delegation_not_found".into())
}

fn in_scope(board: &Board, client: &Client, delegation: &Delegation) -> bool {
    delegation
        .repository_heads
        .keys()
        .all(|path| client.allows(board, path))
}

/// The context supplies defaults only. Project authority always comes from the authenticated client.
fn source(
    board: &Board,
    client: &Client,
    args: &Value,
) -> Result<(Vec<crate::state::Repo>, session::Launch), String> {
    client.validate(board)?;
    if args.get("project_id").is_some() {
        let id = required(args, "project_id", 200)?;
        let project = board
            .projects
            .iter()
            .find(|project| project.id == id && client.allows(board, &project.path))
            .ok_or("project_not_found")?;
        return Ok((
            vec![crate::state::Repo {
                path: project.path.clone(),
                name: project.name.clone(),
                worktree: project.path.clone(),
                base: String::new(),
                pr: None,
            }],
            session::Launch::default(),
        ));
    }
    let conversation = client
        .conversation
        .as_deref()
        .ok_or("project_id_required")?;
    let parent = board
        .workspace_of(conversation)
        .ok_or("coordinator_unavailable")?;
    if parent.preparing || parent.cleaned {
        return Err("coordinator_workspace_unavailable".into());
    }
    let repos = if parent.repos.is_empty() {
        vec![parent.primary()]
    } else {
        parent.repos.clone()
    };
    Ok((
        repos,
        parent.launch_of(conversation, &session::ResolvedTools::default()),
    ))
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn required<'a>(args: &'a Value, name: &str, max: usize) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty() && v.len() <= max)
        .ok_or_else(|| format!("invalid_argument: {name}"))
}

fn page(args: &Value, key: &str, default: usize, max: usize) -> Result<usize, String> {
    match args.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|n| *n <= max as u64)
            .map(|n| n as usize)
            .ok_or_else(|| format!("invalid_argument: {key}")),
    }
}

fn summary(board: &Board, d: &Delegation) -> Value {
    let ws = board.workspaces.iter().find(|w| w.id == d.workspace);
    let tab = ws.and_then(|w| w.tabs.iter().find(|t| t.id == d.id));
    json!({
        "agent_id": d.id, "conversation_id": d.id, "workspace_id": d.workspace,
        "task": d.task, "execution": d.executions.last(),
        "workspace": ws.map(|w| json!({"title": w.title, "stage": w.stage,
            "preparing": w.preparing, "failed": w.failed, "archived": w.archived,
            "cleaned": w.cleaned, "branch": w.branch})),
        "conversation_status": tab.map(|t| match t.status {
            Status::Rodando => "working", Status::Querendo => "waiting_for_input",
            Status::Pronta => "ready", Status::Desligada => "stopped",
        }),
        "pending_message": tab.is_some_and(|t| t.pending_prompt.is_some()),
        "background": d.background, "pending_requests": d.requests,
        "available": tab.is_some(),
    })
}

fn script_kind(args: &Value) -> Result<&str, String> {
    match args["kind"].as_str() {
        Some(kind @ ("setup" | "run")) => Ok(kind),
        _ => Err("invalid_argument: kind".into()),
    }
}

fn available_workspace(workspace: &crate::state::Workspace, agent: &str) -> Result<(), String> {
    if workspace.preparing || workspace.cleaned || workspace.archived || workspace.failed.is_some()
    {
        return Err("workspace_unavailable".into());
    }
    if !workspace.tabs.iter().any(|tab| tab.id == agent) {
        return Err("conversation_unavailable".into());
    }
    Ok(())
}

fn script_status(process: Option<&pty::Pty>) -> Value {
    match process {
        Some(process) => json!({
            "state": if process.alive() { "running" } else { "exited" },
            "exit_code": lock(&process.buffer).exit_code,
            "name": process.script_name,
        }),
        None => json!({"state":"not_started", "exit_code":null, "name":null}),
    }
}

fn workspace_runtime(state: &AppState, workspace: &crate::state::Workspace) -> Value {
    let scripts = dock::scripts_of(workspace);
    let ptys = lock(&state.ptys);
    json!({
        "port": workspace.port,
        "url": workspace.port.map(|port| format!("http://localhost:{port}")),
        "run_names": scripts.runs.iter().map(|run| &run.name).collect::<Vec<_>>(),
        "setup": script_status(ptys.get(&format!("{}:setup", workspace.id))),
        "run": script_status(ptys.get(&format!("{}:run", workspace.id))),
    })
}

fn script_log(scroll: &pty::Scroll, limit: usize) -> Value {
    let start = scroll.bytes.len().saturating_sub(limit);
    json!({
        "text": String::from_utf8_lossy(&scroll.bytes[start..]),
        "truncated": start > 0,
        "seq": scroll.seq,
        "exit_code": scroll.exit_code,
    })
}

#[derive(Clone, Serialize)]
struct PreviewRequest<'a> {
    workspace_id: &'a str,
    conversation_id: &'a str,
}

fn worker_choice(
    choice: session::Launch,
    args: &Value,
) -> Result<(crate::state::ProviderId, String, String), String> {
    let provider = match args.get("provider").and_then(Value::as_str) {
        None => choice.agent,
        Some("claude") => crate::state::ProviderId::Claude,
        Some("codex") => crate::state::ProviderId::Codex,
        _ => return Err("invalid_argument: provider".into()),
    };
    let inherit = provider == choice.agent;
    let model = args
        .get("model")
        .map(|_| required(args, "model", 200).map(str::to_string))
        .transpose()?
        .unwrap_or_else(|| if inherit { choice.model } else { String::new() });
    let effort = args
        .get("effort")
        .map(|_| required(args, "effort", 64).map(str::to_string))
        .transpose()?
        .unwrap_or_else(|| {
            if inherit {
                choice.effort
            } else {
                String::new()
            }
        });
    Ok((provider, model, effort))
}

fn previous_creation<'a>(
    board: &'a Board,
    owner: &str,
    key: &str,
    request_hash: &str,
) -> Result<Option<&'a Delegation>, String> {
    let previous = board
        .delegations
        .iter()
        .find(|d| d.owner == owner && d.request_key == key);
    if previous.is_some_and(|d| d.request_hash != request_hash) {
        return Err("request_key_conflict".into());
    }
    Ok(previous)
}

/// Observed background tasks still running. `None` is no authoritative observation, not an
/// empty set.
fn working(background: &Option<Vec<Value>>) -> bool {
    background.as_ref().is_some_and(|tasks| !tasks.is_empty())
}

fn check_send(workspace: &crate::state::Workspace, delegation: &Delegation) -> Result<(), String> {
    let tab = workspace
        .tabs
        .iter()
        .find(|t| t.id == delegation.id)
        .ok_or("conversation_unavailable")?;
    if workspace.preparing || workspace.cleaned || workspace.archived || workspace.failed.is_some()
    {
        return Err("workspace_unavailable".into());
    }
    if matches!(tab.status, Status::Rodando | Status::Querendo)
        || tab.pending_prompt.is_some()
        || working(&delegation.background)
    {
        return Err("conversation_busy: wait for the current execution or interrupt it".into());
    }
    Ok(())
}

fn create(app: &AppHandle, client: &Client, args: &Value) -> Result<Value, String> {
    let key = required(args, "request_key", 128)?;
    let task = required(args, "task", 64 * 1024)?;
    let title = required(args, "title", 200)?;
    let state = app.state::<AppState>();
    let (repos, choice, stage) = {
        let board = lock(&state.board);
        client.validate(&board)?;
        if let Some(previous) =
            previous_creation(&board, &client.id, key, &digest(&args.to_string()))?
        {
            return Ok(summary(&board, owned(&board, client, &previous.id)?));
        }
        let (repos, choice) = source(&board, client, args)?;
        (
            repos,
            choice,
            board.stages.first().cloned().unwrap_or_default(),
        )
    };
    let permission = choice.permission;
    let (provider, model, effort) = worker_choice(choice, args)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut repository_heads = BTreeMap::new();
    for repo in &repos {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo.worktree)
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err("repository_has_no_commit".into());
        }
        repository_heads.insert(
            repo.path.clone(),
            String::from_utf8_lossy(&output.stdout).trim().into(),
        );
    }
    let primary = repos.first().ok_or("repository_unavailable")?;
    let base = repository_heads
        .get(&primary.path)
        .cloned()
        .ok_or("repository_unavailable")?;
    let delegation = Delegation {
        id: id.clone(),
        owner: client.id.clone(),
        workspace: String::new(),
        task: task.into(),
        request_key: key.into(),
        request_hash: digest(&args.to_string()),
        repository_heads,
        permission,
        executions: vec![Execution::new("coordinator")],
        background: None,
        requests: vec![],
    };
    // Source selection enforces client scope before any Git or workspace operation. The agent
    // cannot choose arbitrary paths or reuse an existing workspace.
    let draft: session::Draft = serde_json::from_value(json!({
        "project": primary.path,
        "extras": repos.iter().skip(1).map(|r| r.path.clone()).collect::<Vec<_>>(),
        "branch": format!("delegated-{}", &id[..12]), "base": base,
        "worktree": true, "title": title, "stage": stage, "prompt": task, "inject": [],
        "agent": provider, "model": model, "effort": effort, "permission": permission,
        "mcp": [], "plugins": [], "skills": [],
    }))
    .map_err(|e| e.to_string())?;
    session::create_workspace_owned(app.clone(), app.state(), draft, 80, 24, Some(delegation))?;
    let board = lock(&state.board);
    Ok(summary(&board, owned(&board, client, &id)?))
}

/// All calls arrive serialized at the local MCP boundary. Work is asynchronous after creation/send.
pub fn call(app: &AppHandle, client: &Client, name: &str, args: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    client.validate(&lock(&state.board))?;
    if name == "delegate" {
        return create(app, client, args);
    }
    if name == "list_projects" {
        let offset = page(args, "offset", 0, usize::MAX)?;
        let limit = page(args, "limit", 20, 100)?.max(1);
        let board = lock(&state.board);
        let all: Vec<_> = board
            .projects
            .iter()
            .filter(|project| client.allows(&board, &project.path))
            .collect();
        let items: Vec<_> = all
            .iter()
            .skip(offset)
            .take(limit)
            .map(|project| json!({"project_id":project.id,"name":project.name}))
            .collect();
        return Ok(
            json!({"items":items,"next_offset":(offset.saturating_add(limit) < all.len()).then_some(offset.saturating_add(limit))}),
        );
    }
    if name == "list_delegations" {
        let offset = page(args, "offset", 0, usize::MAX)?;
        let limit = page(args, "limit", 20, 100)?.max(1);
        let board = lock(&state.board);
        let all: Vec<_> = board
            .delegations
            .iter()
            .filter(|d| d.owner == client.id && in_scope(&board, client, d))
            .collect();
        let items: Vec<_> = all
            .iter()
            .skip(offset)
            .take(limit)
            .map(|d| summary(&board, d))
            .collect();
        return Ok(
            json!({"items": items, "next_offset": (offset.saturating_add(limit) < all.len()).then_some(offset.saturating_add(limit))}),
        );
    }
    let id = required(args, "agent_id", 128)?;
    let workspace = {
        let board = lock(&state.board);
        let d = owned(&board, client, id)?.clone();
        if name == "get_execution" {
            let execution = required(args, "execution_id", 260)?;
            return d
                .executions
                .iter()
                .find(|r| r.id == execution)
                .map(|r| json!({"execution": r}))
                .ok_or_else(|| "execution_not_found".into());
        }
        if name == "get_delegation" {
            let mut result = summary(&board, &d);
            let workspace = board
                .workspaces
                .iter()
                .find(|w| w.id == d.workspace)
                .cloned();
            drop(board);
            result["runtime"] = workspace
                .map(|workspace| workspace_runtime(&state, &workspace))
                .unwrap_or(Value::Null);
            return Ok(result);
        }
        let ws = board
            .workspaces
            .iter()
            .find(|w| w.id == d.workspace)
            .ok_or("workspace_unavailable")?
            .clone();
        ws
    };
    match name {
        "run_workspace_script" => {
            available_workspace(&workspace, id)?;
            let kind = script_kind(args)?;
            let name = args
                .get("name")
                .map(|_| required(args, "name", 200))
                .transpose()?;
            if kind == "setup" && name.is_some() {
                return Err("invalid_argument: name is only supported for run".into());
            }
            {
                let ptys = lock(&state.ptys);
                if kind == "run"
                    && ptys
                        .get(&format!("{}:setup", workspace.id))
                        .is_some_and(|p| p.alive())
                {
                    return Err("setup_running: wait for setup to finish".into());
                }
            }
            dock::open_dock(
                app.clone(),
                app.state(),
                workspace.id.clone(),
                kind.into(),
                name.map(str::to_string),
                80,
                24,
            )?;
            let current = lock(&state.board)
                .workspaces
                .iter()
                .find(|w| w.id == workspace.id)
                .cloned()
                .ok_or("workspace_unavailable")?;
            Ok(json!({"runtime": workspace_runtime(&state, &current)}))
        }
        "read_workspace_script_log" => {
            let kind = script_kind(args)?;
            let limit = page(args, "limit_bytes", 16 * 1024, 64 * 1024)?.max(1);
            let ptys = lock(&state.ptys);
            let process = ptys
                .get(&format!("{}:{kind}", workspace.id))
                .ok_or("script_not_started")?;
            let mut result = script_log(&lock(&process.buffer), limit);
            result["running"] = json!(process.alive());
            result["name"] = json!(process.script_name);
            Ok(result)
        }
        "open_workspace_preview" => {
            available_workspace(&workspace, id)?;
            let port = dock::ensure_port(app, &app.state(), &workspace.id)
                .ok_or("workspace_has_no_port")?;
            app.emit_to(
                "main",
                "workspace-preview",
                PreviewRequest {
                    workspace_id: &workspace.id,
                    conversation_id: id,
                },
            )
            .map_err(|error| error.to_string())?;
            Ok(json!({"requested":true, "url":format!("http://localhost:{port}")}))
        }
        "send_message" => {
            let gate = chat::input_gate(id);
            let _input = lock(&gate);
            let (delegation, workspace) = {
                let board = lock(&state.board);
                let d = owned(&board, client, id)?.clone();
                let ws = board
                    .workspaces
                    .iter()
                    .find(|w| w.id == d.workspace)
                    .ok_or("workspace_unavailable")?
                    .clone();
                (d, ws)
            };
            let text = required(args, "text", 64 * 1024)?;
            let key = required(args, "request_key", 128)?;
            // Request IDs are scoped to the delegated conversation, separate from its identity.
            let run_id = format!("{}:{key}", delegation.id);
            if let Some(run) = delegation.executions.iter().find(|r| r.id == run_id) {
                if run.request_hash.as_deref() != Some(&digest(text)) {
                    return Err("request_key_conflict".into());
                }
                return Ok(json!({"execution": run}));
            }
            check_send(&workspace, &delegation)?;
            let mut run = Execution::new("coordinator");
            run.id = run_id.clone();
            run.request_hash = Some(digest(text));
            lock(&state.board)
                .delegations
                .iter_mut()
                .find(|d| d.id == id)
                .unwrap()
                .executions
                .push(run);
            crate::state::publish(app);
            if let Err(error) = crate::state::persist_now(app) {
                if let Some(d) = lock(&state.board)
                    .delegations
                    .iter_mut()
                    .find(|d| d.id == id)
                {
                    if let Some(run) = d.executions.iter_mut().find(|r| r.id == run_id) {
                        run.state = "completed".into();
                        run.outcome = Some("error".into());
                    }
                }
                crate::state::publish(app);
                return Err(error);
            }
            let result = chat::send(app.clone(), app.state(), id.into(), text.into(), true);
            let mut board = lock(&state.board);
            let pending = board
                .tab_mut(id)
                .is_some_and(|t| t.pending_prompt.is_some());
            let d = board.delegations.iter_mut().find(|d| d.id == id).unwrap();
            let run = d.executions.iter_mut().find(|r| r.id == run_id).unwrap();
            if let Err(error) = result {
                if !pending {
                    run.state = "completed".into();
                    run.outcome = Some("error".into());
                }
                drop(board);
                crate::state::publish(app);
                // A failed spawn can leave the message in the existing durable pending queue.
                return Err(format!(
                    "send_failed: {error}; inspect the delegation before retrying"
                ));
            }
            Ok(json!({"execution": run}))
        }
        "interrupt" => {
            chat::chat_control(
                app.state(),
                id.into(),
                json!({"v": 1, "type": "turn.interrupt"}),
            )?;
            Ok(json!({"requested": true}))
        }
        "read_conversation" => {
            let limit = page(args, "limit", 50, 200)?.max(1);
            let snapshot = chat::snapshot(&state, id);
            let events = chat::canonical_history(&snapshot.text);
            let start = events.len().saturating_sub(limit);
            // Individual tool outputs can be large. Return only a bounded suffix of whole events.
            let mut bytes = 0;
            let mut result: Vec<_> = events[start..]
                .iter()
                .rev()
                .take_while(|e| {
                    bytes += e.to_string().len();
                    bytes <= 256 * 1024
                })
                .cloned()
                .collect();
            result.reverse();
            Ok(
                json!({"events": result, "truncated": result.len() < events.len(), "seq": snapshot.seq}),
            )
        }
        "list_files" => {
            let path = args.get("path").and_then(Value::as_str).unwrap_or("");
            let offset = page(args, "offset", 0, usize::MAX)?;
            let limit = page(args, "limit", 100, 500)?.max(1);
            let entries = session::files::list_dir(app.state(), workspace.id, path.into());
            let next = offset.saturating_add(limit);
            Ok(
                json!({"entries": entries.iter().skip(offset).take(limit).collect::<Vec<_>>(),
                "next_offset": (next < entries.len()).then_some(next)}),
            )
        }
        "read_file" => {
            let path = required(args, "path", 4096)?;
            let start = page(args, "start_line", 0, usize::MAX)?;
            let limit = page(args, "limit", 100, 500)?.max(1);
            let content = session::files::read_file(app.state(), workspace.id, path.into())?;
            let lines: Vec<_> = content.lines().collect();
            let mut bytes = 0;
            let selected: Vec<_> = lines
                .iter()
                .skip(start)
                .take(limit)
                .take_while(|line| {
                    bytes += line.len() + 1;
                    bytes <= 128 * 1024
                })
                .copied()
                .collect();
            if selected.is_empty() && start < lines.len() {
                return Err("line_too_large".into());
            }
            let next = start.saturating_add(selected.len());
            Ok(json!({"text": selected.join("\n"), "start_line": start,
                "next_line": (next < lines.len()).then_some(next)}))
        }
        _ => Err("unknown_tool".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delegation() -> Delegation {
        Delegation {
            id: "worker".into(),
            owner: "coordinator".into(),
            workspace: "child".into(),
            task: "implement a task".into(),
            request_key: "first".into(),
            request_hash: "hash".into(),
            repository_heads: BTreeMap::new(),
            permission: None,
            executions: vec![Execution::new("coordinator")],
            background: None,
            requests: vec![],
        }
    }

    fn board() -> Board {
        let tab =
            |id| json!({"id":id,"title":"","status":"pronta","note":null,"pending_prompt":null});
        let workspace = |id, tabs| {
            json!({"id":id,"title":"","repo":"/repo","repo_name":"repo",
            "branch":"main","worktree":"/worktree","stage":"manual stage","tabs":tabs})
        };
        serde_json::from_value(json!({"stages":["manual stage"],"workspaces":[
            workspace("parent", vec![tab("coordinator"),tab("other-coordinator")]),
            workspace("child", vec![tab("worker"),tab("person-created-tab")])
        ]}))
        .unwrap()
    }

    #[test]
    fn workspace_controls_require_an_owned_available_conversation() {
        let mut board = board();
        board.delegations.push(delegation());
        assert!(owned(&board, &Client::conversation("other-coordinator"), "worker").is_err());
        let d = owned(&board, &Client::conversation("coordinator"), "worker").unwrap();
        let mut workspace = board
            .workspaces
            .iter()
            .find(|w| w.id == d.workspace)
            .unwrap()
            .clone();
        assert!(available_workspace(&workspace, &d.id).is_ok());
        assert!(available_workspace(&workspace, "missing-tab").is_err());
        for field in ["preparing", "cleaned", "archived", "failed"] {
            let mut value = serde_json::to_value(&workspace).unwrap();
            value[field] = if field == "failed" {
                json!("setup failed")
            } else {
                json!(true)
            };
            let blocked = serde_json::from_value(value).unwrap();
            assert_eq!(
                available_workspace(&blocked, &d.id),
                Err("workspace_unavailable".into())
            );
        }
        workspace.tabs.retain(|tab| tab.id != d.id);
        assert_eq!(
            available_workspace(&workspace, &d.id),
            Err("conversation_unavailable".into())
        );
        assert_eq!(
            serde_json::to_value(PreviewRequest {
                workspace_id: &d.workspace,
                conversation_id: &d.id,
            })
            .unwrap(),
            json!({"workspace_id":"child", "conversation_id":"worker"})
        );
    }

    #[test]
    fn script_logs_are_bounded_and_preserve_exit_status_without_touching_executions() {
        let mut scroll = pty::Scroll::default();
        scroll.absorb(&vec![b'x'; 70 * 1024]);
        scroll.absorb("\nfailed: café\n".as_bytes());
        scroll.exit_code = Some(7);
        let page = script_log(&scroll, 64 * 1024);
        assert_eq!(page["text"].as_str().unwrap().len(), 64 * 1024);
        assert!(page["text"].as_str().unwrap().ends_with("failed: café\n"));
        assert_eq!(page["truncated"], true);
        assert_eq!(page["seq"], 2);
        assert_eq!(page["exit_code"], 7);
        assert_eq!(script_log(&pty::Scroll::default(), 100)["truncated"], false);
        assert_eq!(script_status(None)["state"], "not_started");
        assert!(script_kind(&json!({"kind":"terminal"})).is_err());
        assert_eq!(script_kind(&json!({"kind":"setup"})).unwrap(), "setup");
    }

    #[test]
    fn ownership_is_per_conversation_not_workspace_membership() {
        let mut board = board();
        board.delegations.push(delegation());
        assert!(owned(&board, &Client::conversation("coordinator"), "worker").is_ok());
        assert!(owned(&board, &Client::conversation("other-coordinator"), "worker").is_err());
        assert!(owned(
            &board,
            &Client::conversation("coordinator"),
            "person-created-tab"
        )
        .is_err());
        assert!(owned(&board, &Client::conversation("worker"), "worker").is_err());
        assert!(owned(&board, &Client::conversation("coordinator"), "parent").is_err());
        board.workspaces[0].tabs.retain(|t| t.id != "coordinator");
        assert!(owned(&board, &Client::conversation("coordinator"), "worker").is_err());
    }

    #[test]
    fn independent_clients_use_scoped_projects_without_a_parent_conversation() {
        let mut board = board();
        board.projects = serde_json::from_value(json!([
            {"id":"project", "name":"repo", "path":"/repo"},
            {"id":"other", "name":"other", "path":"/other"}
        ]))
        .unwrap();
        let client = Client {
            id: "client:external".into(),
            conversation: None,
            projects: vec!["/repo".into()],
        };
        let internal = Client::conversation("coordinator");
        let (repos, _) = source(&board, &internal, &json!({})).unwrap();
        assert_eq!(repos[0].worktree, "/worktree");
        assert!(source(&board, &internal, &json!({"project_id":"other"})).is_err());
        // External authority does not depend on any coordinator tab or process.
        board.workspaces.remove(0);
        assert!(source(&board, &client, &json!({})).is_err());
        assert!(source(&board, &client, &json!({"project_id":"other"})).is_err());
        assert!(source(&board, &client, &json!({"project_id":"/repo"})).is_err());
        let (repos, choice) = source(&board, &client, &json!({"project_id":"project"})).unwrap();
        assert_eq!(repos[0].path, "/repo");
        assert_eq!(repos[0].worktree, "/repo");
        assert!(choice.permission.is_none());
        let (provider, model, _) = worker_choice(choice, &json!({"provider":"codex"})).unwrap();
        assert_eq!(provider, crate::state::ProviderId::Codex);
        assert!(model.is_empty());
        let mut d = delegation();
        d.owner = client.id.clone();
        d.repository_heads.insert("/repo".into(), "commit".into());
        board.delegations.push(d);
        let restored: Board =
            serde_json::from_value(serde_json::to_value(&board).unwrap()).unwrap();
        assert!(owned(&restored, &client, "worker").is_ok());
        assert!(owned(&restored, &internal, "worker").is_err());
        let other = Client {
            id: "client:other".into(),
            ..client.clone()
        };
        assert!(owned(&restored, &other, "worker").is_err());
        assert!(owned(&restored, &client, "person-created-tab").is_err());
        let narrowed = Client {
            projects: vec!["/other".into()],
            ..client.clone()
        };
        assert!(owned(&restored, &narrowed, "worker").is_err());
        board.projects[0].path = "/replaced".into();
        assert!(source(&board, &client, &json!({"project_id":"project"})).is_err());
    }

    #[test]
    fn old_boards_default_to_no_delegations_and_new_boards_roundtrip() {
        let mut board = board();
        assert!(board.delegations.is_empty());
        board.delegations.push(delegation());
        let mut restored: Board =
            serde_json::from_value(serde_json::to_value(&board).unwrap()).unwrap();
        restored.revive();
        assert!(owned(&restored, &Client::conversation("coordinator"), "worker").is_ok());
        assert_eq!(restored.workspaces[1].stage, "manual stage");
        assert_eq!(restored.delegations[0].request_key, "first");
    }

    #[test]
    fn completion_waits_for_background_tasks_and_unknown_is_not_empty() {
        let mut d = delegation();
        let run_id = d.executions[0].id.clone();
        assert!(d.background.is_none());
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}));
        d.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        assert_eq!(d.executions[0].id, run_id);
        assert_eq!(d.executions[0].state, "running");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("ok"));
        assert_eq!(d.background.as_ref().unwrap().len(), 1);
        d.observe(&json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.background, Some(vec![]));
    }

    #[test]
    fn a_held_execution_never_reports_completion_while_sends_are_rejected() {
        let mut workspace = board().workspaces.remove(1);
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}));
        d.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        workspace.tabs[0].status = Status::Rodando;
        assert_ne!(d.executions[0].state, "completed");
        assert!(check_send(&workspace, &d).is_err());
        d.observe(&json!({"type":"background.changed","tasks":[]}));
        workspace.tabs[0].status = Status::Pronta;
        assert_eq!(d.executions[0].state, "completed");
        assert!(check_send(&workspace, &d).is_ok());
    }

    #[test]
    fn an_interruption_completes_the_execution_without_waiting_for_background_tasks() {
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}));
        d.observe(&json!({"type":"turn.completed","outcome":"interrupted"}));
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("interrupted"));
        assert!(d.background.is_none());
    }

    #[test]
    fn a_resumed_turn_discards_the_outcome_its_background_tasks_were_holding() {
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}));
        d.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        assert_eq!(d.executions[0].outcome.as_deref(), Some("ok"));
        d.observe(&json!({"type":"assistant.started"}));
        assert!(d.executions[0].outcome.is_none());
        d.observe(&json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].state, "running");
        d.observe(&json!({"type":"turn.completed","outcome":"error"}));
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("error"));
    }

    #[test]
    fn draining_background_alone_does_not_complete_an_unfinished_turn() {
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}));
        d.observe(&json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions[0].state, "running");
        assert!(d.executions[0].outcome.is_none());
    }

    #[test]
    fn requests_and_process_restarts_do_not_invent_completion() {
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"request.opened","requestId":"q1","kind":"question"}));
        d.observe(&json!({"type":"request.opened","requestId":"q1","kind":"question"}));
        assert_eq!(d.requests.len(), 1);
        d.observe(&json!({"type":"request.closed","requestId":"q1"}));
        assert!(d.requests.is_empty());
        d.reconcile_restart(false);
        assert_eq!(d.executions[0].state, "stopped");
        assert!(d.executions[0].outcome.is_none());
        assert!(d.background.is_none());
    }

    #[test]
    fn pending_messages_keep_their_execution_id_when_recovered() {
        let mut d = delegation();
        let id = d.executions[0].id.clone();
        d.reconcile_restart(true);
        d.observe(&json!({"type":"session.state","state":"starting"}));
        d.observe(&json!({"type":"session.state","state":"busy"}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].id, id);
        assert_eq!(d.executions[0].state, "running");
    }

    #[test]
    fn subsequent_person_turns_and_background_continuations_get_distinct_executions() {
        let mut d = delegation();
        d.observe(&json!({"type":"session.state","state":"busy"}));
        d.observe(&json!({"type":"turn.completed","outcome":"interrupted"}));
        d.observe(&json!({"type":"session.state","state":"busy"}));
        assert_eq!(d.executions[1].source, "conversation");
        d.observe(&json!({"type":"turn.completed","outcome":"ok"}));
        d.observe(&json!({"type":"assistant.started"}));
        d.observe(&json!({"type":"assistant.started"}));
        assert_eq!(d.executions.len(), 3);
        assert_eq!(d.executions[2].source, "background");
        assert_eq!(d.executions[2].state, "running");
        assert_ne!(d.executions[0].id, d.executions[1].id);
    }

    #[test]
    fn overlapping_person_input_preserves_the_active_execution_until_completion() {
        for outcome in ["ok", "error", "interrupted"] {
            let mut d = delegation();
            let id = d.executions[0].id.clone();
            d.observe(&json!({"type":"session.state","state":"busy"}));
            d.observe(&json!({"type":"assistant.started"}));
            for _ in 0..2 {
                d.observe(&json!({"type":"session.state","state":"busy"}));
                d.observe(&json!({"type":"assistant.started"}));
            }
            assert_eq!(d.executions.len(), 1);
            assert_eq!(d.executions[0].id, id);
            assert_eq!(d.executions[0].source, "coordinator");
            assert_eq!(d.executions[0].state, "running");

            // Persisted execution identities retain the same completion behavior.
            let mut d: Delegation =
                serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
            d.observe(&json!({"type":"turn.completed","outcome":outcome}));
            assert_eq!(d.executions[0].id, id);
            assert_eq!(d.executions[0].state, "completed");
            assert_eq!(d.executions[0].outcome.as_deref(), Some(outcome));

            // A provider that responds again after completion gets a separate observed execution.
            d.observe(&json!({"type":"assistant.started"}));
            d.observe(&json!({"type":"turn.completed","outcome":"ok"}));
            assert_eq!(d.executions.len(), 2);
            assert_ne!(d.executions[1].id, id);
            assert!(d.executions.iter().all(|run| run.state == "completed"));
        }
    }

    #[test]
    fn worker_defaults_follow_the_owner_tab_provider_and_model() {
        let mut board = board();
        board.workspaces[0].tabs[0].choice = Some(crate::state::Choice {
            agent: crate::state::ProviderId::Codex,
            model: "chosen-model".into(),
            effort: "high".into(),
        });
        let (provider, model, effort) = worker_choice(
            board.workspaces[0].launch_of("coordinator", &session::ResolvedTools::default()),
            &json!({}),
        )
        .unwrap();
        assert_eq!(provider, crate::state::ProviderId::Codex);
        assert_eq!(model, "chosen-model");
        assert_eq!(effort, "high");
        let (provider, model, effort) = worker_choice(
            board.workspaces[0].launch_of("coordinator", &session::ResolvedTools::default()),
            &json!({"provider":"claude"}),
        )
        .unwrap();
        assert_eq!(provider, crate::state::ProviderId::Claude);
        assert!(model.is_empty() && effort.is_empty());
        assert!(worker_choice(
            board.workspaces[0].launch_of("coordinator", &session::ResolvedTools::default()),
            &json!({"provider":"unknown"})
        )
        .is_err());
    }

    #[test]
    fn creation_retries_are_scoped_to_owner_and_reject_changed_arguments() {
        let mut board = board();
        board.delegations.push(delegation());
        assert_eq!(
            previous_creation(&board, "coordinator", "first", "hash")
                .unwrap()
                .unwrap()
                .id,
            "worker"
        );
        assert!(previous_creation(&board, "coordinator", "first", "changed").is_err());
        assert!(
            previous_creation(&board, "other-coordinator", "first", "changed")
                .unwrap()
                .is_none()
        );
        assert!(previous_creation(&board, "coordinator", "second", "hash")
            .unwrap()
            .is_none());
    }

    #[test]
    fn send_rejects_busy_questions_pending_input_and_background_without_changing_stage() {
        let mut workspace = board().workspaces.remove(1);
        let mut d = delegation();
        assert!(check_send(&workspace, &d).is_ok());
        for status in [Status::Rodando, Status::Querendo] {
            workspace.tabs[0].status = status;
            assert!(check_send(&workspace, &d).is_err());
        }
        workspace.tabs[0].status = Status::Desligada;
        assert!(check_send(&workspace, &d).is_ok());
        workspace.tabs[0].pending_prompt = Some("pending".into());
        assert!(check_send(&workspace, &d).is_err());
        workspace.tabs[0].pending_prompt = None;
        d.background = Some(vec![json!({"id":"native-child"})]);
        assert!(check_send(&workspace, &d).is_err());
        d.background = Some(vec![]);
        assert!(check_send(&workspace, &d).is_ok());
        workspace.archived = true;
        assert!(check_send(&workspace, &d).is_err());
        assert_eq!(workspace.stage, "manual stage");
    }

    #[test]
    fn pagination_and_message_inputs_are_bounded() {
        assert!(required(&json!({"text":" "}), "text", 100).is_err());
        assert!(required(&json!({"text":"12345"}), "text", 4).is_err());
        assert!(page(&json!({"limit":-1}), "limit", 20, 100).is_err());
        assert!(page(&json!({"limit":101}), "limit", 20, 100).is_err());
        assert_eq!(page(&json!({}), "limit", 20, 100).unwrap(), 20);
        assert_ne!(digest("first message"), digest("changed message"));
    }
}
