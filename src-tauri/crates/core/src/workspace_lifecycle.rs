//! Existing board mutations shared by desktop and execution hosts. Effects stay with the host.
use crate::{
    board::{Board, Choice, ShareRights, Status},
    error::code,
};

pub enum Change {
    /// Sharing consent: who views and comments, the owner's own devices, and who may act (ADR 0090). Turning
    /// sharing off clears every grant.
    Share {
        shared: bool,
        audience: Option<Vec<String>>,
        remote_control: bool,
        team: Option<String>,
        rights: Option<ShareRights>,
    },
    Rename(String),
    RenameTab {
        tab: String,
        title: String,
    },
    Pin(bool),
    Unread(bool),
    Archive(bool),
    Finish,
    Remove,
    Retune {
        tab: String,
        choice: Choice,
    },
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
        Change::Share {
            shared,
            audience,
            remote_control,
            team,
            rights,
        } => {
            workspace.shared = shared;
            workspace.share_team = if shared { team } else { None };
            workspace.audience = if shared { audience } else { None };
            workspace.remote_control = shared && remote_control;
            // A share saved from now on records its rights, even when empty; `None` marks one from before them.
            workspace.rights = shared.then(|| rights.unwrap_or_default());
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

#[cfg(test)]
mod share_tests {
    use super::*;
    use serde_json::json;

    fn board(workspace: serde_json::Value) -> Board {
        serde_json::from_value(json!({ "stages": ["Working"], "workspaces": [workspace] })).unwrap()
    }

    #[test]
    fn shares_from_before_rights_load_without_them_and_new_consent_records_them() {
        // A board written before ADR 0090: shared with the organization, no rights field.
        let mut board = board(json!({
            "id": "first", "title": "Task", "repo": "/repo", "repo_name": "repo", "branch": "task",
            "worktree": "/worktree", "stage": "Working", "shared": true, "audience": null,
            "share_team": "organization:org:member", "remote_control": true
        }));
        let workspace = &board.workspaces[0];
        assert!(workspace.shared && workspace.remote_control);
        assert_eq!(workspace.rights, None);
        assert_eq!(
            serde_json::to_value(&board).unwrap()["workspaces"][0]["rights"],
            json!(null)
        );

        let rights = ShareRights {
            send: vec!["bob".into()],
            control: vec![],
        };
        apply(
            &mut board,
            "first",
            Change::Share {
                shared: true,
                audience: Some(vec!["bob".into()]),
                remote_control: false,
                team: Some("organization:org:member".into()),
                rights: Some(rights.clone()),
            },
        )
        .unwrap();
        let saved = serde_json::to_value(&board).unwrap();
        assert_eq!(
            saved["workspaces"][0]["rights"],
            json!({ "send": ["bob"], "control": [] })
        );
        let reloaded: Board = serde_json::from_value(saved).unwrap();
        assert_eq!(reloaded.workspaces[0].rights, Some(rights));

        apply(
            &mut board,
            "first",
            Change::Share {
                shared: true,
                audience: None,
                remote_control: false,
                team: Some("organization:org:member".into()),
                rights: None,
            },
        )
        .unwrap();
        assert_eq!(board.workspaces[0].rights, Some(ShareRights::default()));

        apply(
            &mut board,
            "first",
            Change::Share {
                shared: false,
                audience: Some(vec!["bob".into()]),
                remote_control: true,
                team: Some("organization:org:member".into()),
                rights: Some(ShareRights {
                    send: vec!["bob".into()],
                    control: vec!["bob".into()],
                }),
            },
        )
        .unwrap();
        let workspace = &board.workspaces[0];
        assert!(!workspace.shared && !workspace.remote_control);
        assert_eq!(
            (
                workspace.audience.clone(),
                workspace.share_team.clone(),
                workspace.rights.clone()
            ),
            (None, None, None)
        );
    }
}
