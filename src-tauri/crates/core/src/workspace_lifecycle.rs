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
            workspace.archived = archived;
            if archived {
                stopped(workspace);
            }
        }
        Change::Finish => {
            if let Some(stage) = final_stage {
                workspace.stage = stage;
            }
            workspace.archived = true;
            stopped(workspace);
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
