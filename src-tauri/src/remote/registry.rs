//! The machine's registry of projects and workspaces (ADR 0103), shared with the agents that drive
//! its host. This app adopts what agents create there and records what the person creates, so
//! both sides see the same workspaces; the host is the registry's only writer.
use crate::state::{ConversationMode, Project, ProviderId, Repo, Status, Tab, Workspace};
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
    pub adopted: Vec<(String, String)>,
}

/// Bring the board in line with the machine's registry: add the projects, workspaces and
/// conversations agents created, archive workspaces they closed, and record on the machine what
/// this app knows that the registry lacks. A host without a registry (an older build) changes
/// nothing. The caller serializes synchronizations of one machine.
pub fn sync(app: &AppHandle, connection: &Arc<Connection>) -> Synced {
    let mut synced = Synced::default();
    let Some(listing) = connection
        .request("registry_list", json!({}))
        .ok()
        .and_then(|value| serde_json::from_value::<Listing>(value).ok())
    else {
        return synced;
    };
    let machine = connection.id.as_str();
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
                synced
                    .adopted
                    .extend(ws.tabs.iter().map(|t| (t.id.clone(), ws.worktree.clone())));
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
                ws.tabs.push(tab(conversation));
                if !registered.closed {
                    synced
                        .adopted
                        .push((conversation.id.clone(), ws.worktree.clone()));
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
                        || recorded_tabs(ws).all(|t| w.conversations.iter().any(|c| c.id == t.id))
                });
            if recordable && !complete {
                puts.push(record(ws));
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

fn tab(conversation: &RegisteredConversation) -> Tab {
    Tab {
        conversation_mode: ConversationMode::Terminal,
        plan: false,
        permission: Some(if conversation.permission == "auto" {
            Permission::Auto
        } else {
            Permission::Ask
        }),
        task: None,
        id: conversation.id.clone(),
        agent_session: None,
        title: conversation.title.clone(),
        // The reattachment that follows adoption reports the running ones.
        status: Status::Desligada,
        note: None,
        pending_prompt: None,
        native_dispatch: None,
        tokens: None,
        context_tokens: None,
        context_window: None,
        choice: None,
        kickoff: None,
        interrupted: false,
        cuts: Vec::new(),
    }
}

fn workspace(machine: &str, registered: &RegisteredWorkspace) -> Workspace {
    let name = basename(&registered.project);
    let tabs: Vec<Tab> = registered.conversations.iter().map(tab).collect();
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
        model: String::new(),
        effort: String::new(),
        agent: ProviderId::Claude,
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

/// The conversations agent clients on the machine may follow: they read Claude transcripts and
/// resume Claude only (ADR 0103), so other providers' tabs stay out of the registry.
fn recorded_tabs(ws: &Workspace) -> impl Iterator<Item = &Tab> {
    ws.tabs
        .iter()
        .filter(move |t| t.choice.as_ref().map_or(ws.agent, |c| c.agent) == ProviderId::Claude)
}

/// The registry record of a board workspace. The host keeps what it already holds, so the
/// timestamps and creator here only matter for a workspace new to it.
fn record(ws: &Workspace) -> Value {
    let primary = ws.primary();
    // Claude runs without prompts unless the tab asks for them.
    let permission = |p: Option<Permission>| match p {
        Some(Permission::Ask) => "ask",
        _ => "auto",
    };
    let conversations: Vec<Value> = recorded_tabs(ws)
        .map(|t| json!({"id": t.id, "title": t.title, "created_at": now(), "permission": permission(t.permission)}))
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
    let (machine, params) = {
        let state = app.state::<AppState>();
        let board = lock(&state.board);
        let Some(ws) = board.workspaces.iter().find(|w| w.id == workspace) else {
            return;
        };
        let Some(machine) = ws.host.clone() else {
            return;
        };
        (machine, record(ws))
    };
    let Ok(connection) = app.state::<super::Service>().connected(&machine) else {
        return;
    };
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
        let value = record(&ws);
        let conversations = &value["workspace"]["conversations"];
        assert_eq!(conversations[0]["permission"], "ask");
        assert_eq!(conversations[1]["id"], "c2");
        assert_eq!(conversations[1]["permission"], "auto");
        assert_eq!(value["workspace"]["keep_branch"], true);
        assert_eq!(value["workspace"]["project"], "/home/me/code/app");
        assert_eq!(value["project"]["path"], "/home/me/code/app");
    }

    #[test]
    fn agent_clients_never_see_conversations_they_cannot_resume() {
        let mut ws = workspace("vps", &registered("agent"));
        let mut codex = ws.tabs[0].clone();
        codex.id = "c2".into();
        codex.choice = Some(crate::state::Choice {
            agent: ProviderId::Codex,
            model: String::new(),
            effort: String::new(),
        });
        ws.tabs.push(codex);
        let conversations = record(&ws)["workspace"]["conversations"].clone();
        assert_eq!(conversations.as_array().unwrap().len(), 1);
        assert_eq!(conversations[0]["id"], "c1");
        // A workspace whose first conversation runs Codex is still offered to agents.
        ws.agent = ProviderId::Codex;
        ws.tabs[1].choice = None;
        let conversations = record(&ws)["workspace"]["conversations"].clone();
        assert_eq!(conversations, json!([]));
    }
}
