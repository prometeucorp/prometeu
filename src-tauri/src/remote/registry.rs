//! The machine's registry of projects and workspaces (ADR 0103), shared with the agents that drive
//! its host. This app adopts what agents create there and records what the person creates, so
//! both sides see the same workspaces; the host is the registry's only writer.
use crate::state::{Choice, ConversationMode, Project, ProviderId, Repo, Status, Tab, Workspace};
use crate::AppState;
use prometeu_core::actions::Permission;
use prometeu_core::lock::lock;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use super::connection::Connection;

#[derive(Deserialize, Default)]
struct Creator {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct RegisteredProject {
    path: String,
    name: String,
    #[serde(default)]
    origin: Option<String>,
}

#[derive(Deserialize)]
struct RegisteredConversation {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    permission: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    effort: String,
}

#[derive(Deserialize)]
struct RegisteredWorkspace {
    id: String,
    project: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    base: String,
    worktree: String,
    #[serde(default)]
    created_by: Creator,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    keep_branch: bool,
    #[serde(default)]
    conversations: Vec<RegisteredConversation>,
}

#[derive(Deserialize, Default)]
struct Listing {
    #[serde(default)]
    projects: Vec<RegisteredProject>,
    #[serde(default)]
    workspaces: Vec<RegisteredWorkspace>,
}

/// What a synchronization changed that the caller must act on.
#[derive(Default)]
pub struct Synced {
    /// Conversations new to the board, with their workspace's working directory.
    pub adopted: Vec<(String, String, ProviderId)>,
}

/// Bring the board in line with the machine's registry: add the projects, workspaces and
/// conversations agents created, archive workspaces they closed, and record on the machine what
/// this app knows that the registry lacks. A host without a registry (an older build) changes
/// nothing. The caller serializes synchronizations of one machine.
pub fn sync(app: &AppHandle, connection: &Arc<Connection>) -> Synced {
    let mut synced = Synced::default();
    let Some(listing) = connection
        .request("registry_list", json!({"codex_agents": true}))
        .ok()
        .and_then(|value| serde_json::from_value::<Listing>(value).ok())
    else {
        return synced;
    };
    let machine = connection.id.as_str();
    let codex_agents = supports_codex(connection);
    let state = app.state::<AppState>();
    let mut stopped = Vec::new();
    let mut puts = Vec::new();
    let mut closes = Vec::new();
    let mut changed = false;
    {
        let mut board = lock(&state.board);
        // Read under the board lock, which every dismissal also holds, so a removal made while
        // the listing was on its way is seen.
        let dismissed = dismissed(machine);
        for project in &listing.projects {
            if !dismissed.contains(&super::workspace::project_id(machine, &project.path)) {
                changed |= adopt_project(&mut board.projects, machine, project);
            }
        }
        // IDs the board uses anywhere: an ID from this machine never takes over another entry.
        let taken: HashSet<String> = board
            .workspaces
            .iter()
            .flat_map(|ws| {
                std::iter::once(ws.id.clone()).chain(ws.tabs.iter().map(|t| t.id.clone()))
            })
            .collect();
        for registered in &listing.workspaces {
            let index = board.workspaces.iter().position(|w| w.id == registered.id);
            let Some(index) = index else {
                if registered.closed || dismissed.contains(&registered.id) {
                    continue;
                }
                let project = super::workspace::project_id(machine, &registered.project);
                if !board.projects.iter().any(|p| p.id == project) {
                    adopt_project(
                        &mut board.projects,
                        machine,
                        &RegisteredProject {
                            path: registered.project.clone(),
                            name: basename(&registered.project),
                            origin: None,
                        },
                    );
                }
                let mut ws = workspace(machine, registered);
                ws.tabs
                    .retain(|t| !taken.contains(&t.id) && !dismissed.contains(&t.id));
                ws.active = ws.tabs.last().map(|t| t.id.clone());
                synced.adopted.extend(ws.tabs.iter().map(|t| {
                    (
                        t.id.clone(),
                        ws.worktree.clone(),
                        t.choice.as_ref().map_or(ws.agent, |c| c.agent),
                    )
                }));
                board.workspaces.push(ws);
                changed = true;
                continue;
            };
            let ws = &mut board.workspaces[index];
            if ws.host.as_deref() != Some(machine) {
                continue;
            }
            if ws.cleaned && !registered.closed {
                // This app removed the checkout, but the close never reached the machine.
                closes.push(ws.id.clone());
                continue;
            }
            let mut added = false;
            for conversation in &registered.conversations {
                if ws.tabs.iter().any(|t| t.id == conversation.id)
                    || taken.contains(&conversation.id)
                    || dismissed.contains(&conversation.id)
                {
                    continue;
                }
                let Some(tab) = tab(conversation) else {
                    continue;
                };
                let provider = tab.choice.as_ref().map_or(ws.agent, |choice| choice.agent);
                ws.tabs.push(tab);
                if !registered.closed {
                    synced
                        .adopted
                        .push((conversation.id.clone(), ws.worktree.clone(), provider));
                }
                added = true;
            }
            if registered.closed {
                // An agent closed it: the checkout and branch are gone on the machine.
                if !ws.cleaned {
                    ws.archived = true;
                    ws.cleaned = true;
                    for tab in &mut ws.tabs {
                        tab.status = Status::Desligada;
                        tab.note = None;
                        stopped.push(tab.id.clone());
                    }
                    changed = true;
                }
                changed |= added;
            } else if added {
                // An agent works here again: show it, as a new message would.
                ws.archived = false;
                ws.unread = true;
                changed = true;
            }
        }
        // Record what the registry lacks: projects and workspaces from before it existed, and
        // conversations whose record was lost on the way.
        for project in board
            .projects
            .iter()
            .filter(|p| p.host.as_deref() == Some(machine))
        {
            if !listing.projects.iter().any(|p| p.path == project.path) {
                puts.push(project_record(project));
            }
        }
        for ws in &board.workspaces {
            let recordable = ws.host.as_deref() == Some(machine)
                && !ws.cleaned
                && !ws.preparing
                && ws.failed.is_none();
            let complete = listing
                .workspaces
                .iter()
                .find(|w| w.id == ws.id)
                .is_some_and(|w| {
                    w.closed
                        || recorded_tabs(ws, codex_agents)
                            .all(|t| w.conversations.iter().any(|c| c.id == t.id))
                });
            if recordable && !complete {
                puts.push(record(ws, codex_agents));
            }
        }
    }
    for tab in &stopped {
        crate::chat::kill(&state, tab);
    }
    if changed {
        crate::state::publish(app);
    }
    for params in puts {
        let _ = connection.request("registry_put", params);
    }
    for id in closes {
        close(connection, &id);
    }
    synced
}

fn adopt_project(projects: &mut Vec<Project>, machine: &str, project: &RegisteredProject) -> bool {
    let id = super::workspace::project_id(machine, &project.path);
    if projects.iter().any(|p| p.id == id) {
        return false;
    }
    projects.push(Project {
        id,
        name: project.name.clone(),
        path: project.path.clone(),
        group: None,
        host: Some(machine.to_string()),
        origin: project
            .origin
            .as_deref()
            .and_then(super::workspace::normalize_origin),
    });
    true
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("repo")
        .to_string()
}

fn choice(conversation: &RegisteredConversation) -> Option<Choice> {
    let agent = match conversation.provider.as_str() {
        "" | "claude" => ProviderId::Claude,
        "codex" => ProviderId::Codex,
        // Unknown providers must never become Claude through the board's compatibility fallback.
        _ => return None,
    };
    Some(Choice {
        agent,
        model: conversation.model.clone(),
        effort: conversation.effort.clone(),
    })
}

fn tab(conversation: &RegisteredConversation) -> Option<Tab> {
    let choice = choice(conversation)?;
    let codex = choice.agent == ProviderId::Codex;
    if codex
        && conversation
            .thread_id
            .as_ref()
            .is_none_or(|id| id.is_empty())
    {
        // An unfinished launch has no native identity to adopt.
        return None;
    }
    Some(Tab {
        conversation_mode: ConversationMode::Terminal,
        plan: false,
        permission: Some(if conversation.permission == "auto" {
            Permission::Auto
        } else {
            Permission::Ask
        }),
        task: None,
        id: conversation.id.clone(),
        agent_session: codex.then(|| conversation.thread_id.clone()).flatten(),
        title: conversation.title.clone(),
        // The reattachment that follows adoption reports the running ones.
        status: Status::Desligada,
        note: None,
        pending_prompt: None,
        native_dispatch: None,
        tokens: None,
        context_tokens: None,
        context_window: None,
        choice: Some(choice),
        kickoff: None,
        interrupted: false,
        cuts: Vec::new(),
    })
}

fn workspace(machine: &str, registered: &RegisteredWorkspace) -> Workspace {
    let name = basename(&registered.project);
    let tabs: Vec<Tab> = registered.conversations.iter().filter_map(tab).collect();
    let choice = registered
        .conversations
        .iter()
        .find_map(choice)
        .unwrap_or_default();
    Workspace {
        id: registered.id.clone(),
        title: if registered.title.is_empty() {
            registered.branch.clone()
        } else {
            registered.title.clone()
        },
        project: super::workspace::project_id(machine, &registered.project),
        repo: registered.project.clone(),
        repo_name: name.clone(),
        branch: registered.branch.clone(),
        worktree: registered.worktree.clone(),
        repos: vec![Repo {
            path: registered.project.clone(),
            name,
            worktree: registered.worktree.clone(),
            base: registered.base.clone(),
            pr: None,
        }],
        archived: false,
        pinned: false,
        unread: true,
        model: choice.model,
        effort: choice.effort,
        agent: choice.agent,
        port: None,
        issue: None,
        pr: None,
        cleaned: false,
        preserve_branches: if registered.keep_branch {
            vec![registered.project.clone()]
        } else {
            Vec::new()
        },
        shared: false,
        share_team: None,
        audience: None,
        remote_control: false,
        rights: None,
        confirm_messages: true,
        preparing: false,
        failed: None,
        mcp: None,
        plugins: None,
        skills: None,
        active: tabs.last().map(|t| t.id.clone()),
        tabs,
        host: Some(machine.to_string()),
        creator: (registered.created_by.kind == "agent")
            .then(|| registered.created_by.name.clone().unwrap_or_default()),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Only providers the host can operate enter its registry. Codex also needs its native thread
/// identity, so a concurrent synchronization cannot adopt a launch before its bootstrap ends.
fn recorded_tabs(ws: &Workspace, codex_agents: bool) -> impl Iterator<Item = &Tab> {
    ws.tabs.iter().filter(
        move |t| match t.choice.as_ref().map_or(ws.agent, |c| c.agent) {
            ProviderId::Claude => true,
            ProviderId::Codex => {
                codex_agents && t.agent_session.as_ref().is_some_and(|id| !id.is_empty())
            }
            _ => false,
        },
    )
}

/// Earlier hosts ignore the provider field and would resume Codex records with Claude.
fn supports_codex(connection: &Connection) -> bool {
    connection
        .ready()
        .is_some_and(|ready| ready["codex_agents"].as_bool() == Some(true))
}

/// The registry record of a board workspace. The host keeps what it already holds, so the
/// timestamps and creator here only matter for a workspace new to it.
fn record(ws: &Workspace, codex_agents: bool) -> Value {
    let primary = ws.primary();
    // Remote conversations run without prompts unless the tab asks for them.
    let permission = |p: Option<Permission>| match p {
        Some(Permission::Ask) => "ask",
        _ => "auto",
    };
    let conversations: Vec<Value> = recorded_tabs(ws, codex_agents)
        .map(|t| {
            let (provider, model, effort) = t.choice.as_ref().map_or(
                (ws.agent, &ws.model, &ws.effort),
                |choice| (choice.agent, &choice.model, &choice.effort),
            );
            let effort = if provider == ProviderId::Codex {
                crate::agents::effort(effort)
            } else {
                effort.as_str()
            };
            json!({
                "id": t.id, "title": t.title, "created_at": now(),
                "permission": permission(t.permission), "provider": provider,
                "thread_id": if provider == ProviderId::Codex { t.agent_session.as_deref() } else { None },
                "model": model, "effort": effort,
            })
        })
        .collect();
    json!({
        "project": {
            "path": primary.path, "name": primary.name, "added_at": now(),
            "added_by": {"kind": "desktop"},
        },
        "workspace": {
            "id": ws.id, "project": primary.path, "title": ws.title, "branch": ws.branch,
            "base": primary.base, "worktree": primary.worktree, "created_at": now(),
            "created_by": {"kind": "desktop"},
            "keep_branch": ws.preserve_branches.contains(&primary.path),
            "conversations": conversations,
        },
    })
}

/// Record a remote workspace with its conversations once a new tab is on the board, so agents on
/// the machine see the person's work. Recording only tabs already on the board keeps a
/// synchronization from adopting a conversation this app is still launching. Runs in the
/// background; a lost record is sent again by the next synchronization.
pub fn record_later(app: &AppHandle, workspace: &str) {
    let (machine, ws) = {
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        let Some(ws) = board.workspaces.iter().find(|w| w.id == workspace) else {
            return;
        };
        let Some(machine) = ws.host.clone() else {
            return;
        };
        (machine, ws.clone())
    };
    let Ok(connection) = app.state::<super::Service>().connected(&machine) else {
        return;
    };
    let params = record(&ws, supports_codex(&connection));
    std::thread::spawn(move || {
        let _ = connection.request("registry_put", params);
    });
}

fn project_record(project: &Project) -> Value {
    json!({"project": {
        "path": project.path, "name": project.name, "added_at": now(),
        "added_by": {"kind": "desktop"},
    }})
}

/// Record a project added on the machine.
pub fn record_project(connection: &Connection, project: &Project) {
    let _ = connection.request("registry_put", project_record(project));
}

/// Tell the machine a workspace's checkout is gone, so agents stop offering it.
pub fn close(connection: &Connection, workspace: &str) {
    let _ = connection.request("registry_close", json!({"id": workspace}));
}

fn dismissed_path(machine: &str) -> PathBuf {
    crate::paths::root()
        .join("remote")
        .join(machine)
        .join("dismissed.json")
}

/// Workspaces and projects the person removed from the board and conversations whose tab they
/// closed: the registry still lists them, and they must not come back. Workspace and conversation
/// IDs are UUIDs and project IDs start with `remote:`, so one set holds all three.
fn dismissed(machine: &str) -> HashSet<String> {
    std::fs::read_to_string(dismissed_path(machine))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remember a removed workspace or project, or a closed tab. Callers hold the board lock while they change the
/// board, so a synchronization never reads the set between the change and this write.
pub fn dismiss(machine: &str, id: &str) {
    let mut ids = dismissed(machine);
    if ids.insert(id.to_string()) {
        save_dismissed(machine, &ids);
    }
}

/// Forget a dismissal when the person adds the entry again (a project, by its board ID).
pub fn undismiss(machine: &str, id: &str) {
    let mut ids = dismissed(machine);
    if ids.remove(id) {
        save_dismissed(machine, &ids);
    }
}

fn save_dismissed(machine: &str, ids: &HashSet<String>) {
    let path = dismissed_path(machine);
    if let Some(parent) = path.parent() {
        if crate::paths::ensure_private_dir(parent).is_err() {
            return;
        }
    }
    if let Ok(body) = serde_json::to_string(&ids) {
        let _ = crate::paths::write_private(&path, &body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(creator: &str) -> RegisteredWorkspace {
        serde_json::from_value(json!({
            "id": "w1", "project": "/home/me/code/app", "title": "", "branch": "prometeu/cedro-novo-0007",
            "base": "origin/main", "worktree": "/home/me/.prometeu-remote/worktrees/app/x",
            "created_by": {"kind": creator, "name": "openbot"}, "keep_branch": true,
            "conversations": [{"id": "c1", "title": "Fix it", "permission": "auto", "state": "idle"}],
            "unknown": 1,
        }))
        .unwrap()
    }

    #[test]
    fn an_agents_workspace_becomes_a_terminal_board_entry_with_its_creator() {
        let ws = workspace("vps", &registered("agent"));
        assert_eq!(ws.project, "remote:vps:/home/me/code/app");
        assert_eq!(ws.title, "prometeu/cedro-novo-0007");
        assert_eq!(ws.creator.as_deref(), Some("openbot"));
        assert_eq!(ws.host.as_deref(), Some("vps"));
        assert_eq!(ws.preserve_branches, ["/home/me/code/app"]);
        assert_eq!(ws.active.as_deref(), Some("c1"));
        let tab = &ws.tabs[0];
        assert!(tab.conversation_mode == ConversationMode::Terminal);
        assert!(tab.permission == Some(Permission::Auto));
        assert_eq!(ws.agent, ProviderId::Claude);
        assert!(ws.model.is_empty());
        assert!(ws.effort.is_empty());
        assert!(tab.agent_session.is_none());
        assert_eq!(tab.choice.as_ref().unwrap().agent, ProviderId::Claude);
        assert!(workspace("vps", &registered("desktop")).creator.is_none());
    }

    #[test]
    fn records_carry_each_conversations_permission() {
        let mut ws = workspace("vps", &registered("agent"));
        let mut second = ws.tabs[0].clone();
        second.id = "c2".into();
        second.permission = None;
        ws.tabs[0].permission = Some(Permission::Ask);
        ws.tabs.push(second);
        let value = record(&ws, true);
        let conversations = &value["workspace"]["conversations"];
        assert_eq!(conversations[0]["permission"], "ask");
        assert_eq!(conversations[1]["id"], "c2");
        assert_eq!(conversations[1]["permission"], "auto");
        assert_eq!(value["workspace"]["keep_branch"], true);
        assert_eq!(value["workspace"]["project"], "/home/me/code/app");
        assert_eq!(value["project"]["path"], "/home/me/code/app");
    }

    #[test]
    fn unbound_codex_and_unsupported_providers_stay_out_of_the_registry() {
        let mut ws = workspace("vps", &registered("agent"));
        let mut codex = ws.tabs[0].clone();
        codex.id = "c2".into();
        codex.choice = Some(Choice {
            agent: ProviderId::Codex,
            model: String::new(),
            effort: String::new(),
        });
        ws.tabs.push(codex);
        let mut cursor = ws.tabs[0].clone();
        cursor.id = "c3".into();
        cursor.agent_session = Some("cursor-session".into());
        cursor.choice.as_mut().unwrap().agent = ProviderId::Cursor;
        ws.tabs.push(cursor);
        let conversations = record(&ws, true)["workspace"]["conversations"].clone();
        assert_eq!(conversations.as_array().unwrap().len(), 1);
        assert_eq!(conversations[0]["id"], "c1");

        // A bound Codex tab inherits the workspace selection when it has no override.
        ws.agent = ProviderId::Codex;
        ws.model = "codex-model".into();
        ws.effort = "high".into();
        ws.tabs[0].choice = None;
        ws.tabs[1].choice = None;
        ws.tabs[1].agent_session = Some("native-thread".into());
        let conversations = record(&ws, true)["workspace"]["conversations"].clone();
        assert_eq!(conversations.as_array().unwrap().len(), 1);
        assert_eq!(conversations[0]["id"], "c2");
        assert_eq!(conversations[0]["provider"], "codex");
        assert_eq!(conversations[0]["thread_id"], "native-thread");
        assert_eq!(conversations[0]["model"], "codex-model");
        assert_eq!(conversations[0]["effort"], "high");
        ws.effort = "ultracode".into();
        let value = record(&ws, true);
        assert_eq!(value["workspace"]["conversations"][0]["effort"], "ultra");
        let saved: RegisteredWorkspace =
            serde_json::from_value(value["workspace"].clone()).unwrap();
        assert_eq!(
            workspace("another-computer", &saved).tabs[0]
                .choice
                .as_ref()
                .unwrap()
                .effort,
            "ultra"
        );
    }

    #[test]
    fn mixed_providers_keep_their_native_identity_and_selection_after_round_trip() {
        let mut source = registered("agent");
        source.conversations.insert(
            0,
            serde_json::from_value(json!({
                "id": "codex-tab", "title": "Codex task", "permission": "auto",
                "provider": "codex", "thread_id": "native-thread",
                "model": "codex-model", "effort": "high",
            }))
            .unwrap(),
        );
        let ws = workspace("vps", &source);
        assert_eq!(ws.agent, ProviderId::Codex);
        assert_eq!(ws.model, "codex-model");
        assert_eq!(ws.effort, "high");
        assert_eq!(ws.tabs[0].agent_session.as_deref(), Some("native-thread"));
        assert_eq!(ws.tabs[0].choice.as_ref().unwrap().agent, ProviderId::Codex);
        // A legacy Claude record never inherits the first conversation's Codex selection.
        assert_eq!(
            ws.tabs[1].choice.as_ref().unwrap().agent,
            ProviderId::Claude
        );
        assert!(ws.tabs[1].agent_session.is_none());

        let wire = record(&ws, true);
        let saved = &wire["workspace"]["conversations"];
        assert_eq!(saved[0]["thread_id"], "native-thread");
        assert_eq!(saved[1]["provider"], "claude");
        assert!(saved[1]["thread_id"].is_null());
        let legacy = record(&ws, false);
        let legacy_conversations = legacy["workspace"]["conversations"].as_array().unwrap();
        assert_eq!(legacy_conversations.len(), 1);
        assert_eq!(legacy_conversations[0]["id"], "c1");
        let restored: RegisteredWorkspace =
            serde_json::from_value(wire["workspace"].clone()).unwrap();
        let restored = workspace("second-computer", &restored);
        for (actual, expected) in restored.tabs.iter().zip(&ws.tabs) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.agent_session, expected.agent_session);
            assert!(actual.choice == expected.choice);
        }
    }

    #[test]
    fn adoption_waits_for_a_codex_identity_and_skips_unknown_providers() {
        let mut source = registered("agent");
        for (id, provider, thread) in [
            ("pending", "codex", None),
            ("empty", "codex", Some("")),
            ("future", "future-provider", Some("native-thread")),
        ] {
            source.conversations.push(
                serde_json::from_value(json!({
                    "id": id, "provider": provider, "thread_id": thread,
                }))
                .unwrap(),
            );
        }
        let ws = workspace("vps", &source);
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].id, "c1");
        source.conversations.swap(0, 1);
        let pending = workspace("vps", &source);
        assert_eq!(pending.agent, ProviderId::Codex);
        assert_eq!(pending.tabs.len(), 1);
        assert_eq!(
            pending.tabs[0].choice.as_ref().unwrap().agent,
            ProviderId::Claude
        );
    }
}
