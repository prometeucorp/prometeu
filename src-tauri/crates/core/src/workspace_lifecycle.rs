//! Existing board mutations shared by desktop and execution hosts. Effects stay with the host.
use crate::{
    board::{Board, Choice, Status},
    error::code,
};

pub enum Change {
    Rename(String),
    RenameTab { tab: String, title: String },
    Pin(bool),
    Unread(bool),
    Archive(bool),
    Finish,
    Remove,
    Retune { tab: String, choice: Choice },
    Cleaned,
}

pub fn apply(board: &mut Board, id: &str, change: Change) -> Result<(), String> {
    let final_stage = board.stages.last().cloned();
    let workspace = board
        .workspace_mut(id)
        .ok_or_else(|| code("err.session.noWorkspace"))?;
    match change {
        Change::Rename(title) => rename(&mut workspace.title, &title),
        Change::RenameTab { tab, title } => {
            let tab = workspace
                .tabs
                .iter_mut()
                .find(|t| t.id == tab)
                .ok_or_else(|| code("err.session.noTab"))?;
            rename(&mut tab.title, &title);
        }
        Change::Pin(pinned) => workspace.pinned = pinned,
        Change::Unread(unread) => workspace.unread = unread,
        Change::Retune { tab, choice } => workspace.retune(&tab, choice)?,
        Change::Archive(archived) => {
            set_archived(workspace, archived);
        }
        Change::Finish => {
            if let Some(stage) = final_stage {
                workspace.stage = stage;
            }
            set_archived(workspace, true);
        }
        Change::Cleaned => {
            workspace.cleaned = true;
            stopped(workspace);
        }
        Change::Remove => board.workspaces.retain(|w| w.id != id),
    }
    Ok(())
}
fn rename(destination: &mut String, title: &str) {
    if !title.trim().is_empty() {
        *destination = title.trim().into();
    }
}
fn stopped(workspace: &mut crate::board::Workspace) {
    for tab in &mut workspace.tabs {
        tab.status = Status::Desligada;
        tab.note = None;
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleanable {
    pub id: String,
    pub title: String,
    pub repo_name: String,
    pub branch: String,
    pub worktree: String,
    pub size_kb: u64,
    pub pr: Option<u64>,
    pub blocked: Option<String>,
}
pub fn has_worktree(ws: &crate::board::Workspace) -> bool {
    ws.archived && !ws.cleaned && ws.worktree != ws.repo
}
pub trait WorktreeCleanup: Send + Sync {
    fn inspect(&self, workspace: &crate::board::Workspace) -> Cleanable;
    fn check(&self, workspace: &crate::board::Workspace, force: bool) -> Result<(), String>;
    /// Host validates any grouping root and stops execution before removing worktrees.
    fn remove(&self, workspace: &crate::board::Workspace) -> Result<(), String>;
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct ArchiveEffects {
    pub stop_tabs: Vec<String>,
    pub changed: bool,
}

pub fn finish(board: &mut Board, id: &str) {
    let last = board.stages.last().cloned();
    if let (Some(stage), Some(workspace)) = (last, board.workspace_mut(id)) {
        workspace.stage = stage;
    }
}

pub fn archive(board: &mut Board, id: &str, archived: bool) -> ArchiveEffects {
    let Some(workspace) = board.workspace_mut(id) else {
        return ArchiveEffects::default();
    };
    set_archived(workspace, archived)
}

fn set_archived(workspace: &mut crate::board::Workspace, archived: bool) -> ArchiveEffects {
    let mut effects = ArchiveEffects {
        changed: workspace.archived != archived,
        ..Default::default()
    };
    workspace.archived = archived;
    if archived {
        effects.stop_tabs = workspace.tabs.iter().map(|tab| tab.id.clone()).collect();
        stopped(workspace);
    }
    effects
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use serde_json::{json, Value};

    fn board() -> Board {
        serde_json::from_value(json!({
            "stages": ["Working", "Done"],
            "workspaces": [{
                "id": "first", "title": "Task", "repo": "/repo", "repo_name": "repo",
                "branch": "task", "worktree": "/worktree", "stage": "Working",
                "active": "running", "shared": true,
                "tabs": [
                    {"id": "running", "title": "Build", "status": "rodando", "note": "Testing", "agent_session": "native-thread"},
                    {"id": "waiting", "title": "Review", "status": "querendo", "pending_prompt": "Continue", "plan": true}
                ]
            }, {
                "id": "other", "title": "Other", "repo": "/other", "repo_name": "other",
                "branch": "other", "worktree": "/other", "stage": "Working"
            }]
        })).unwrap()
    }

    #[test]
    fn archiving_preserves_sessions_files_stage_and_unrelated_workspaces() {
        let mut board = board();
        let mut expected = serde_json::to_value(&board).unwrap();
        expected["workspaces"][0]["archived"] = json!(true);
        for tab in expected["workspaces"][0]["tabs"].as_array_mut().unwrap() {
            tab["status"] = json!("desligada");
            tab["note"] = Value::Null;
        }
        assert_eq!(
            archive(&mut board, "first", true),
            ArchiveEffects {
                changed: true,
                stop_tabs: vec!["running".into(), "waiting".into()],
            }
        );
        assert_eq!(serde_json::to_value(&board).unwrap(), expected);
        // Repeated archive still requests shutdown, but emits no second journey fact.
        assert_eq!(
            archive(&mut board, "first", true),
            ArchiveEffects {
                changed: false,
                stop_tabs: vec!["running".into(), "waiting".into()],
            }
        );
        expected["workspaces"][0]["archived"] = json!(false);
        assert_eq!(
            archive(&mut board, "first", false),
            ArchiveEffects {
                changed: true,
                stop_tabs: vec![],
            }
        );
        assert_eq!(serde_json::to_value(&board).unwrap(), expected);
    }

    #[test]
    fn finishing_uses_the_persons_final_stage_without_changing_agent_state() {
        let mut board = board();
        let mut expected = serde_json::to_value(&board).unwrap();
        expected["workspaces"][0]["stage"] = json!("Done");
        finish(&mut board, "first");
        assert_eq!(serde_json::to_value(&board).unwrap(), expected);
        board.stages.clear();
        let before = serde_json::to_value(&board).unwrap();
        finish(&mut board, "first");
        finish(&mut board, "missing");
        assert_eq!(
            archive(&mut board, "missing", true),
            ArchiveEffects::default()
        );
        assert_eq!(serde_json::to_value(&board).unwrap(), before);
    }
}
