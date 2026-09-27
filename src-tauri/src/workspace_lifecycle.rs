//! Archive decisions use board state only; the command owns scripts, process shutdown and events.

use crate::state::{Board, Status};

#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveEffects {
    pub stop_tabs: Vec<String>,
    pub changed: bool,
}

pub(crate) fn finish(board: &mut Board, id: &str) {
    let last = board.stages.last().cloned();
    if let (Some(stage), Some(workspace)) = (last, board.workspace_mut(id)) {
        workspace.stage = stage;
    }
}

pub(crate) fn archive(board: &mut Board, id: &str, archived: bool) -> ArchiveEffects {
    let Some(workspace) = board.workspace_mut(id) else {
        return ArchiveEffects::default();
    };
    let mut effects = ArchiveEffects {
        changed: workspace.archived != archived,
        ..Default::default()
    };
    workspace.archived = archived;
    if archived {
        effects.stop_tabs = workspace.tabs.iter().map(|tab| tab.id.clone()).collect();
        for tab in &mut workspace.tabs {
            tab.status = Status::Desligada;
            tab.note = None;
        }
    }
    effects
}

#[cfg(test)]
mod tests {
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
