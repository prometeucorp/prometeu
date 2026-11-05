//! Local delegation use cases. Ownership is a durable client identity; a workspace is only
//! the environment. This module schedules no work and never grants control over unrelated tabs.

use crate::lock::lock;
use crate::mcp_access::Client;
use crate::state::{Board, Status};
use crate::{chat, conversation, dock, pty, session, AppState};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::process::Command;
use tauri::{AppHandle, Emitter, Manager};

pub use prometeu_core::delegation::{Delegation, Execution};

fn new_execution(source: &str) -> Execution {
    Execution {
        id: uuid::Uuid::new_v4().to_string(),
        state: "queued".into(),
        source: source.into(),
        accepted_at: conversation::now(),
        outcome: None,
        request_hash: None,
    }
}

fn observe_delegation(delegation: &mut Delegation, event: &Value) -> bool {
    // A resumed turn invalidates the outcome recorded while background tasks held completion;
    // otherwise a later drain would report the old result for a turn still running.
    if conversation::agent_activity(event) {
        if let Some(run) = delegation
            .executions
            .last_mut()
            .filter(|r| r.state == "running")
        {
            run.outcome = None;
        }
    }
    match event["type"].as_str() {
        Some("session.state") if event["state"] == "busy" => {
            // Accepted follow-ups do not identify a separate provider turn. Keep the active
            // execution until its terminal event rather than orphaning its coordinator ID.
            if delegation
                .executions
                .last()
                .is_none_or(|r| !matches!(r.state.as_str(), "queued" | "running"))
            {
                delegation.executions.push(new_execution("conversation"));
            }
            delegation.executions.last_mut().unwrap().state = "running".into();
        }
        Some("session.state") if event["state"] == "starting" => {
            delegation.background = None;
            delegation.requests.clear();
        }
        Some("assistant.started")
            if delegation
                .executions
                .last()
                .is_none_or(|r| r.state == "completed" || r.state == "stopped") =>
        {
            let mut run = new_execution("background");
            run.state = "running".into();
            delegation.executions.push(run);
        }
        Some("turn.completed") => {
            // Native subagents outlive the main turn. Record the outcome, but hold completion
            // until they drain: a caller must never read a finished execution from a
            // conversation that still rejects its next message as busy.
            // An interruption ends the children too; the last observation of them is stale.
            let interrupted = event["outcome"] == "interrupted";
            if interrupted {
                delegation.background = None;
            }
            let running = working(&delegation.background);
            if let Some(run) = delegation
                .executions
                .last_mut()
                .filter(|r| r.state == "running")
            {
                run.outcome = event["outcome"].as_str().map(str::to_string);
                if !running {
                    run.state = "completed".into();
                }
            }
            delegation.requests.clear();
        }
        Some("background.changed") => {
            delegation.background = event["tasks"].as_array().cloned();
            // Draining alone never completes a turn; only a held terminal settles here.
            if !working(&delegation.background) {
                if let Some(run) = delegation
                    .executions
                    .last_mut()
                    .filter(|r| r.state == "running" && r.outcome.is_some())
                {
                    run.state = "completed".into();
                }
            }
        }
        Some("request.opened") => {
            delegation
                .requests
                .retain(|r| r["requestId"] != event["requestId"]);
            delegation.requests.push(event.clone());
        }
        Some("request.closed") => {
            delegation
                .requests
                .retain(|r| r["requestId"] != event["requestId"]);
        }
        _ => return false,
    }
    true
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
        observe_delegation(d, event);
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
        session::workspace_launch_of(parent, conversation, &session::ResolvedTools::default()),
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
    // Resolved like the worker's launch: without a tab override, as on the first tab or before
    // preparation creates it, the workspace's settings apply.
    let launch =
        ws.map(|w| session::workspace_launch_of(w, &d.id, &session::ResolvedTools::default()));
    json!({
        "agent_id": d.id, "conversation_id": d.id, "workspace_id": d.workspace,
        "task": d.task, "execution": d.executions.last(),
        // Empty model/effort mean the provider CLI's default.
        "provider": launch.as_ref().map(|l| l.agent), "model": launch.as_ref().map(|l| &l.model),
        "effort": launch.as_ref().map(|l| &l.effort),
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

/// Providers a delegated worker can run on, in `list_agents` order. Cursor stays a coordinator only.
const WORKER_PROVIDERS: [crate::state::ProviderId; 3] = [
    crate::state::ProviderId::Claude,
    crate::state::ProviderId::Codex,
    crate::state::ProviderId::Antigravity,
];

/// The worker's provider is always explicit: neither a model nor the coordinator's own provider
/// selects it. Naming the inherited provider keeps its model and effort unless they are passed.
fn worker_choice(
    choice: session::Launch,
    args: &Value,
) -> Result<(crate::state::ProviderId, String, String), String> {
    let provider = args
        .get("provider")
        .and_then(Value::as_str)
        .and_then(|name| {
            WORKER_PROVIDERS
                .into_iter()
                .find(|p| prometeu_core::accounts::key(*p) == name)
        })
        .ok_or("invalid_argument: provider")?;
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

/// Worker choices are checked against the provider's live catalog before any workspace exists. A
/// model never selects its provider: a mismatch is rejected with the closest IDs of the chosen
/// provider instead of reaching a CLI that cannot run it. `model` and `effort` are the worker's
/// effective values, `inherited` is the provider whose launch settings it inherits, and `args`
/// tells what the caller passed. Returns the effort to launch with.
fn check_choice(
    provider: crate::state::ProviderId,
    inherited: crate::state::ProviderId,
    model: &str,
    effort: &str,
    args: &Value,
    catalog: impl FnOnce() -> Result<crate::agents::ModelCatalog, crate::agents::CatalogError>,
) -> Result<String, String> {
    // Naming the inherited provider without a model or effort keeps the inherited choice as is;
    // only other choices pay for a live catalog query.
    if provider == inherited && args.get("model").is_none() && args.get("effort").is_none() {
        return Ok(effort.into());
    }
    let name = prometeu_core::accounts::key(provider);
    let catalog = catalog().map_err(|error| {
        format!(
            "provider_unavailable: {name} could not list its models ({}). Check that its CLI is installed and has an account in Prometeu, or call list_agents.",
            error.code
        )
    })?;
    // Older Codex choices saved the app's `ultracode` spelling, which the launch sends as `ultra`;
    // compare and keep the value the CLI receives.
    let effort = match provider {
        crate::state::ProviderId::Codex => crate::agents::effort(effort),
        _ => effort,
    };
    let found = catalog.models.iter().find(|m| m.id == model);
    if found.is_none() && args.get("model").is_some() {
        let wanted = model.to_ascii_lowercase();
        let close: Vec<&str> = catalog
            .models
            .iter()
            .filter(|m| {
                let id = m.id.to_ascii_lowercase();
                id.contains(&wanted) || wanted.contains(&id)
            })
            .map(|m| m.id.as_str())
            .collect();
        let offered = if close.is_empty() {
            catalog
                .models
                .iter()
                .map(|m| m.id.as_str())
                .take(30)
                .collect()
        } else {
            close
        };
        let offered = if offered.is_empty() {
            "no models".to_string()
        } else {
            offered.join(", ")
        };
        return Err(format!(
            "model_not_available: '{model}' is not a model ID offered by provider {name}. A model never changes the provider: pass the provider that offers it. {name} offers: {offered}. Call list_agents for every provider's exact IDs."
        ));
    }
    let explicit = args.get("effort").is_some();
    match found {
        Some(found) if !effort.is_empty() && !found.efforts.iter().any(|e| e == effort) => {
            // `effort` cannot be passed empty, so an inherited effort the chosen model lacks falls
            // back to the CLI default instead of making that model unreachable.
            if !explicit {
                return Ok(String::new());
            }
            Err(if found.efforts.is_empty() {
                format!("effort_not_available: '{model}' has no separate effort levels; omit effort")
            } else {
                format!(
                    "effort_not_available: '{model}' supports {}",
                    found.efforts.join(", ")
                )
            })
        }
        // Without a catalogued model (the CLI default, or an inherited model outside the catalog),
        // only the efforts the provider lists for any model bound an explicit effort.
        None if explicit
            && !catalog
                .models
                .iter()
                .any(|m| m.efforts.iter().any(|e| e == effort)) =>
        {
            Err(format!(
                "effort_not_available: no {name} model lists '{effort}'. Pass a model with one of its efforts from list_agents, or omit effort."
            ))
        }
        _ => Ok(effort.into()),
    }
}

/// Live worker catalog for coordinators. Providers are queried in parallel so the response stays
/// within one catalog timeout; no board lock is held meanwhile.
fn list_agents(app: &AppHandle) -> Result<Value, String> {
    let launcher = app.state::<AppState>().query_launcher.clone();
    let catalogs: Vec<_> = std::thread::scope(|scope| {
        let queries: Vec<_> = WORKER_PROVIDERS
            .into_iter()
            .map(|provider| {
                let launcher = launcher.as_ref();
                scope.spawn(move || (provider, crate::agents::models(launcher, provider)))
            })
            .collect();
        queries
            .into_iter()
            .filter_map(|query| query.join().ok())
            .collect()
    });
    Ok(json!({
        "providers": catalogs.into_iter().map(|(provider, catalog)| match catalog {
            Ok(catalog) => json!({
                "provider": provider, "available": true,
                "models": catalog.models.iter()
                    .map(|m| json!({"id": m.id, "label": m.label, "efforts": m.efforts}))
                    .collect::<Vec<_>>(),
            }),
            Err(error) => json!({
                "provider": provider, "available": false, "models": [], "error": error.code,
            }),
        }).collect::<Vec<_>>(),
    }))
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
    let inherited = choice.agent;
    let (provider, model, effort) = worker_choice(choice, args)?;
    let effort = check_choice(provider, inherited, &model, &effort, args, || {
        crate::agents::models(state.query_launcher.as_ref(), provider)
    })?;
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
        executions: vec![new_execution("coordinator")],
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
    use crate::analytics::{self, DelegationAction, DelegationClient, Event};
    let result = call_inner(app, client, name, args);
    let action = match name {
        "delegate" => Some(DelegationAction::Create),
        "send_message" => Some(DelegationAction::Send),
        "interrupt" => Some(DelegationAction::Interrupt),
        "list_delegations" | "get_delegation" | "get_execution" | "read_conversation" => {
            Some(DelegationAction::Inspect)
        }
        "list_files" | "read_file" => Some(DelegationAction::Files),
        "run_workspace_script" => match args["kind"].as_str() {
            Some("setup") => Some(DelegationAction::Setup),
            Some("run") => Some(DelegationAction::Run),
            _ => None,
        },
        "open_workspace_preview" => Some(DelegationAction::Preview),
        _ => None,
    };
    if result.is_ok() {
        if let Some(action) = action {
            analytics::track(Event::DelegationActionUsed {
                action,
                client: if client.conversation.is_some() {
                    DelegationClient::Internal
                } else {
                    DelegationClient::External
                },
            });
        }
    }
    result
}

fn call_inner(app: &AppHandle, client: &Client, name: &str, args: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    client.validate(&lock(&state.board))?;
    if name == "delegate" {
        return create(app, client, args);
    }
    if name == "list_agents" {
        return list_agents(app);
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
            let gate = chat::input_gate(&state, id);
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
            let mut run = new_execution("coordinator");
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
            let result = chat::send(app.clone(), id.into(), text.into(), true);
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
            executions: vec![new_execution("coordinator")],
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
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}),
        );
        observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"ok"}));
        assert_eq!(d.executions[0].id, run_id);
        assert_eq!(d.executions[0].state, "running");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("ok"));
        assert_eq!(d.background.as_ref().unwrap().len(), 1);
        observe_delegation(&mut d, &json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.background, Some(vec![]));
    }

    #[test]
    fn a_held_execution_never_reports_completion_while_sends_are_rejected() {
        let mut workspace = board().workspaces.remove(1);
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}),
        );
        observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"ok"}));
        workspace.tabs[0].status = Status::Rodando;
        assert_ne!(d.executions[0].state, "completed");
        assert!(check_send(&workspace, &d).is_err());
        observe_delegation(&mut d, &json!({"type":"background.changed","tasks":[]}));
        workspace.tabs[0].status = Status::Pronta;
        assert_eq!(d.executions[0].state, "completed");
        assert!(check_send(&workspace, &d).is_ok());
    }

    #[test]
    fn an_interruption_completes_the_execution_without_waiting_for_background_tasks() {
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}),
        );
        observe_delegation(
            &mut d,
            &json!({"type":"turn.completed","outcome":"interrupted"}),
        );
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("interrupted"));
        assert!(d.background.is_none());
    }

    #[test]
    fn a_resumed_turn_discards_the_outcome_its_background_tasks_were_holding() {
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}),
        );
        observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"ok"}));
        assert_eq!(d.executions[0].outcome.as_deref(), Some("ok"));
        observe_delegation(&mut d, &json!({"type":"assistant.started"}));
        assert!(d.executions[0].outcome.is_none());
        observe_delegation(&mut d, &json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].state, "running");
        observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"error"}));
        assert_eq!(d.executions[0].state, "completed");
        assert_eq!(d.executions[0].outcome.as_deref(), Some("error"));
    }

    #[test]
    fn draining_background_alone_does_not_complete_an_unfinished_turn() {
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"background.changed","tasks":[{"id":"child","description":"review","toolId":null}]}),
        );
        observe_delegation(&mut d, &json!({"type":"background.changed","tasks":[]}));
        assert_eq!(d.executions[0].state, "running");
        assert!(d.executions[0].outcome.is_none());
    }

    #[test]
    fn requests_and_process_restarts_do_not_invent_completion() {
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"request.opened","requestId":"q1","kind":"question"}),
        );
        observe_delegation(
            &mut d,
            &json!({"type":"request.opened","requestId":"q1","kind":"question"}),
        );
        assert_eq!(d.requests.len(), 1);
        observe_delegation(&mut d, &json!({"type":"request.closed","requestId":"q1"}));
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
        observe_delegation(&mut d, &json!({"type":"session.state","state":"starting"}));
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        assert_eq!(d.executions.len(), 1);
        assert_eq!(d.executions[0].id, id);
        assert_eq!(d.executions[0].state, "running");
    }

    #[test]
    fn subsequent_person_turns_and_background_continuations_get_distinct_executions() {
        let mut d = delegation();
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        observe_delegation(
            &mut d,
            &json!({"type":"turn.completed","outcome":"interrupted"}),
        );
        observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
        assert_eq!(d.executions[1].source, "conversation");
        observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"ok"}));
        observe_delegation(&mut d, &json!({"type":"assistant.started"}));
        observe_delegation(&mut d, &json!({"type":"assistant.started"}));
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
            observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
            observe_delegation(&mut d, &json!({"type":"assistant.started"}));
            for _ in 0..2 {
                observe_delegation(&mut d, &json!({"type":"session.state","state":"busy"}));
                observe_delegation(&mut d, &json!({"type":"assistant.started"}));
            }
            assert_eq!(d.executions.len(), 1);
            assert_eq!(d.executions[0].id, id);
            assert_eq!(d.executions[0].source, "coordinator");
            assert_eq!(d.executions[0].state, "running");

            // Persisted execution identities retain the same completion behavior.
            let mut d: Delegation =
                serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
            observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":outcome}));
            assert_eq!(d.executions[0].id, id);
            assert_eq!(d.executions[0].state, "completed");
            assert_eq!(d.executions[0].outcome.as_deref(), Some(outcome));

            // A provider that responds again after completion gets a separate observed execution.
            observe_delegation(&mut d, &json!({"type":"assistant.started"}));
            observe_delegation(&mut d, &json!({"type":"turn.completed","outcome":"ok"}));
            assert_eq!(d.executions.len(), 2);
            assert_ne!(d.executions[1].id, id);
            assert!(d.executions.iter().all(|run| run.state == "completed"));
        }
    }

    #[test]
    fn worker_defaults_follow_the_owner_tab_model_on_its_provider() {
        let mut board = board();
        board.workspaces[0].tabs[0].choice = Some(crate::state::Choice {
            agent: crate::state::ProviderId::Codex,
            model: "chosen-model".into(),
            effort: "high".into(),
        });
        let owner = || {
            session::workspace_launch_of(
                &board.workspaces[0],
                "coordinator",
                &session::ResolvedTools::default(),
            )
        };
        let (provider, model, effort) =
            worker_choice(owner(), &json!({"provider":"codex"})).unwrap();
        assert_eq!(provider, crate::state::ProviderId::Codex);
        assert_eq!(model, "chosen-model");
        assert_eq!(effort, "high");
        let (provider, model, effort) =
            worker_choice(owner(), &json!({"provider":"claude"})).unwrap();
        assert_eq!(provider, crate::state::ProviderId::Claude);
        assert!(model.is_empty() && effort.is_empty());
        // The provider is never inherited, not even the owner's own.
        for args in [json!({}), json!({"provider":"unknown"})] {
            assert_eq!(
                worker_choice(owner(), &args).unwrap_err(),
                "invalid_argument: provider"
            );
        }
    }

    #[test]
    fn explicit_projects_inherit_nothing_from_the_conversation() {
        use crate::state::ProviderId::Codex;
        let mut board = board();
        board.projects =
            serde_json::from_value(json!([{"id":"project", "name":"repo", "path":"/repo"}]))
                .unwrap();
        board.workspaces[0].agent = Codex;
        board.workspaces[0].model = "gpt-model".into();
        let internal = Client::conversation("coordinator");
        let (_, conversation) = source(&board, &internal, &json!({})).unwrap();
        assert_eq!(
            worker_choice(conversation, &json!({"provider":"codex"})).unwrap(),
            (Codex, "gpt-model".into(), String::new())
        );
        let (_, project) = source(&board, &internal, &json!({"project_id":"project"})).unwrap();
        assert_eq!(
            worker_choice(project, &json!({"provider":"codex"})).unwrap(),
            (Codex, String::new(), String::new())
        );
    }

    #[test]
    fn workers_can_run_on_antigravity_but_not_cursor() {
        use crate::state::ProviderId::{Antigravity, Claude, Cursor};
        let launch = session::Launch::default();
        let (provider, model, effort) =
            worker_choice(launch.clone(), &json!({"provider":"antigravity"})).unwrap();
        assert_eq!(provider, Antigravity);
        assert!(model.is_empty() && effort.is_empty());
        assert!(worker_choice(launch, &json!({"provider":"cursor"})).is_err());
        // A Cursor coordinator must name a worker provider and never passes its own model on.
        let cursor = session::Launch {
            agent: Cursor,
            ..coordinator("cursor-model", "high")
        };
        assert!(worker_choice(cursor.clone(), &json!({})).is_err());
        assert_eq!(
            worker_choice(cursor, &json!({"provider":"claude"})).unwrap(),
            (Claude, String::new(), String::new())
        );
    }

    fn catalog(models: &[(&str, &[&str])]) -> crate::agents::ModelCatalog {
        crate::agents::ModelCatalog {
            models: models
                .iter()
                .map(|(id, efforts)| crate::agents::Model {
                    id: (*id).into(),
                    label: (*id).into(),
                    efforts: efforts.iter().map(|e| (*e).into()).collect(),
                    additional: false,
                })
                .collect(),
            fetched_at: 0,
        }
    }

    /// Resolve a worker like `create`: inherit from the coordinator, then check the choice.
    fn choose(
        coordinator: &session::Launch,
        args: Value,
        catalog: impl FnOnce() -> Result<crate::agents::ModelCatalog, crate::agents::CatalogError>,
    ) -> Result<(crate::state::ProviderId, String, String), String> {
        let (provider, model, effort) = worker_choice(coordinator.clone(), &args)?;
        let effort = check_choice(provider, coordinator.agent, &model, &effort, &args, catalog)?;
        Ok((provider, model, effort))
    }

    fn coordinator(model: &str, effort: &str) -> session::Launch {
        session::Launch {
            model: model.into(),
            effort: effort.into(),
            ..Default::default()
        }
    }

    #[test]
    fn explicit_models_must_belong_to_the_chosen_provider() {
        use crate::state::ProviderId::Claude;
        let claude = || Ok(catalog(&[("opus", &["low", "high"]), ("haiku", &[])]));
        let antigravity = || {
            Ok(catalog(&[
                ("gemini-3.8-flash-high", &[]),
                ("gemini-3.8-flash-low", &[]),
                ("gpt-oss-120b-medium", &[]),
            ]))
        };
        let person = coordinator("", "");
        assert_eq!(
            choose(
                &person,
                json!({"provider":"claude","model":"opus","effort":"high"}),
                claude
            )
            .unwrap(),
            (Claude, "opus".into(), "high".into())
        );
        assert!(choose(&person, json!({"provider":"claude"}), claude).is_ok());
        // As in the reported failure, a Gemini model sent to Claude gets Claude's IDs back.
        let error = choose(
            &person,
            json!({"provider":"claude","model":"gemini-3.8-flash"}),
            claude,
        )
        .unwrap_err();
        assert!(error.starts_with("model_not_available:"), "{error}");
        assert!(
            error.contains("list_agents") && error.contains("opus, haiku"),
            "{error}"
        );
        // Close IDs of the chosen provider guide the retry.
        let error = choose(
            &person,
            json!({"provider":"antigravity","model":"gemini-3.8-flash"}),
            antigravity,
        )
        .unwrap_err();
        assert!(
            error.contains("gemini-3.8-flash-high, gemini-3.8-flash-low")
                && !error.contains("gpt-oss"),
            "{error}"
        );
        assert!(choose(
            &person,
            json!({"provider":"antigravity","model":"gemini-3.8-flash-high"}),
            antigravity
        )
        .is_ok());
        for args in [
            json!({"provider":"claude","model":"opus","effort":"max"}),
            json!({"provider":"claude","model":"haiku","effort":"low"}),
        ] {
            assert!(choose(&person, args, claude)
                .unwrap_err()
                .starts_with("effort_not_available:"));
        }
        let error = choose(&person, json!({"provider":"codex"}), || {
            Err(crate::agents::CatalogError::new("noAccount"))
        })
        .unwrap_err();
        assert!(error.starts_with("provider_unavailable: codex"), "{error}");
    }

    #[test]
    fn efforts_are_checked_against_the_model_the_worker_runs() {
        use crate::state::ProviderId::Claude;
        let claude = || {
            Ok(catalog(&[
                ("opus", &["low", "high", "max"]),
                ("sonnet", &["low", "high"]),
                ("haiku", &[]),
            ]))
        };
        // An explicit effort is checked against the inherited model.
        let error = choose(
            &coordinator("haiku", ""),
            json!({"provider":"claude","effort":"high"}),
            claude,
        )
        .unwrap_err();
        assert!(
            error.starts_with("effort_not_available: 'haiku'"),
            "{error}"
        );
        assert_eq!(
            choose(
                &coordinator("sonnet", ""),
                json!({"provider":"claude","effort":"low"}),
                claude
            )
            .unwrap(),
            (Claude, "sonnet".into(), "low".into())
        );
        // An inherited effort survives only on an explicit model that lists it.
        assert_eq!(
            choose(
                &coordinator("opus", "high"),
                json!({"provider":"claude","model":"sonnet"}),
                claude
            )
            .unwrap(),
            (Claude, "sonnet".into(), "high".into())
        );
        for model in ["sonnet", "haiku"] {
            assert_eq!(
                choose(
                    &coordinator("opus", "max"),
                    json!({"provider":"claude","model":model}),
                    claude
                )
                .unwrap(),
                (Claude, model.into(), String::new())
            );
        }
        // Without a catalogued model, the provider's listed efforts bound an explicit effort.
        for model in ["", "claude-opus-outside-catalog"] {
            let effort = |effort: &str| json!({"provider":"claude","effort":effort});
            assert!(choose(&coordinator(model, ""), effort("max"), claude).is_ok());
            let error = choose(&coordinator(model, ""), effort("ultra"), claude).unwrap_err();
            assert!(
                error.starts_with("effort_not_available: no claude model"),
                "{error}"
            );
        }
        let antigravity = || Ok(catalog(&[("gemini-3.8-flash-high", &[])]));
        assert!(choose(
            &coordinator("", ""),
            json!({"provider":"antigravity","effort":"high"}),
            antigravity
        )
        .unwrap_err()
        .starts_with("effort_not_available:"));
        // Codex efforts are checked as launched: the old `ultracode` spelling is `ultra`.
        let codex = || {
            Ok(catalog(&[
                ("gpt-model", &["high", "ultra"]),
                ("gpt-mini", &["high"]),
            ]))
        };
        let old = session::Launch {
            agent: crate::state::ProviderId::Codex,
            ..coordinator("gpt-model", "ultracode")
        };
        assert_eq!(
            choose(&old, json!({"provider":"codex","model":"gpt-model"}), codex).unwrap(),
            (
                crate::state::ProviderId::Codex,
                "gpt-model".into(),
                "ultra".into()
            )
        );
        assert_eq!(
            choose(&old, json!({"provider":"codex","model":"gpt-mini"}), codex)
                .unwrap()
                .2,
            ""
        );
        assert_eq!(
            choose(
                &coordinator("", ""),
                json!({"provider":"codex","model":"gpt-model","effort":"ultracode"}),
                codex
            )
            .unwrap()
            .2,
            "ultra"
        );
        // Naming the inherited provider alone keeps the inherited choice without a catalog query.
        for owner in [
            coordinator("haiku", "high"),
            coordinator("claude-opus-outside-catalog", "max"),
        ] {
            assert_eq!(
                choose(&owner, json!({"provider":"claude"}), || unreachable!()).unwrap(),
                (Claude, owner.model.clone(), owner.effort.clone())
            );
        }
    }

    #[test]
    fn summaries_report_the_settings_the_worker_runs_with() {
        let mut board = board();
        board.delegations.push(delegation());
        board.workspaces[1].agent = crate::state::ProviderId::Antigravity;
        board.workspaces[1].model = "gemini-3.8-flash-high".into();
        let settings = |board: &Board| {
            let summary = summary(board, &board.delegations[0]);
            (
                summary["provider"].clone(),
                summary["model"].clone(),
                summary["effort"].clone(),
            )
        };
        // The worker's first tab has no override, so it runs with the workspace's settings.
        assert_eq!(
            settings(&board),
            (
                json!("antigravity"),
                json!("gemini-3.8-flash-high"),
                json!("")
            )
        );
        board.workspaces[1].tabs[0].choice = Some(crate::state::Choice {
            agent: crate::state::ProviderId::Codex,
            model: "gpt-model".into(),
            effort: "high".into(),
        });
        assert_eq!(
            settings(&board),
            (json!("codex"), json!("gpt-model"), json!("high"))
        );
        // During preparation the tab does not exist yet.
        board.workspaces[1].tabs.retain(|t| t.id != "worker");
        assert_eq!(
            settings(&board),
            (
                json!("antigravity"),
                json!("gemini-3.8-flash-high"),
                json!("")
            )
        );
        board.workspaces.remove(1);
        assert_eq!(settings(&board), (json!(null), json!(null), json!(null)));
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
