//! Validate and persist workspace tool selections without touching running conversations.
//! Commands translate errors and publish; resolved selections apply at the next process spawn.

use crate::board::Board;
use crate::lock::lock;
use crate::selection::Selection;
use serde_json::Value;
use std::sync::Mutex;

/// Standalone skills use plugin packages, but only belong to the skills selection axis.
#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    Mcp,
    Plugins,
    Skills,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Invalid {
    Payload,
    Axis,
}

/// Null restores inheritance; objects select only IDs belonging to the requested axis.
pub fn selection(value: Value, axis: Axis) -> Result<Option<Selection>, Invalid> {
    let selection = match value {
        Value::Null => return Ok(None),
        other @ Value::Object(_) => {
            serde_json::from_value::<Selection>(other).map_err(|_| Invalid::Payload)?
        }
        _ => return Err(Invalid::Payload),
    };
    let misplaced = |id: &str| match axis {
        Axis::Skills => !id.starts_with("skill-"),
        Axis::Plugins => id.starts_with("skill-"),
        Axis::Mcp => false,
    };
    if selection
        .add
        .iter()
        .chain(&selection.remove)
        .any(|id| misplaced(id))
    {
        return Err(Invalid::Axis);
    }
    Ok(Some(selection))
}

/// Change only the supplied axis, regardless of tab activity. A missing workspace is unchanged.
/// Running and idle processes keep their original tools until a later spawn or stopped resume.
pub fn change(board: &Mutex<Board>, id: &str, axis: Axis, selection: Option<Selection>) {
    let _ = apply(&mut lock(board), id, axis, selection);
}

/// Preserve the existing absent / explicit null / replacement IPC distinction.
pub fn patch(body: &Value, name: &str, axis: Axis) -> Result<Option<Option<Selection>>, String> {
    let body = body
        .as_object()
        .ok_or_else(|| crate::error::code("err.tools.badPayload"))?;
    body.get(name)
        .cloned()
        .map(|value| {
            selection(value, axis).map_err(|invalid| {
                crate::error::code(match invalid {
                    Invalid::Payload => "err.tools.badPayload",
                    Invalid::Axis => "err.tools.badAxis",
                })
            })
        })
        .transpose()
}
pub fn apply(
    board: &mut Board,
    id: &str,
    axis: Axis,
    selection: Option<Selection>,
) -> Result<(), String> {
    let workspace = board
        .workspace_mut(id)
        .ok_or_else(|| crate::error::code("err.session.noWorkspace"))?;
    match axis {
        Axis::Mcp => workspace.mcp = selection,
        Axis::Plugins => workspace.plugins = selection,
        Axis::Skills => workspace.skills = selection,
    }
    Ok(())
}
pub fn global(board: &mut Board, body: &Value) -> Result<(), String> {
    let mcp = patch(body, "mcp", Axis::Mcp)?;
    let plugins = patch(body, "plugins", Axis::Plugins)?;
    let skills = patch(body, "skills", Axis::Skills)?;
    if let Some(value) = mcp {
        board.tools.mcp = value;
    }
    if let Some(value) = plugins {
        board.tools.plugins = value;
    }
    if let Some(value) = skills {
        board.tools.skills = value;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn board() -> Board {
        serde_json::from_value(json!({
            "stages": [],
            "workspaces": [{
                "id": "w", "title": "w", "repo": "", "repo_name": "", "branch": "",
                "worktree": "", "stage": "",
                "mcp": { "add": ["original-mcp"] },
                "plugins": { "add": ["original-plugin"] },
                "skills": { "add": ["skill-original"] },
                "tabs": [
                    { "id": "running", "title": "running", "status": "rodando" },
                    { "id": "waiting", "title": "waiting", "status": "querendo" },
                    { "id": "queued", "title": "queued", "status": "pronta", "pending_prompt": "" },
                    { "id": "idle", "title": "idle", "status": "pronta" }
                ]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn workspace_tools_change_only_the_selected_axis_and_preserve_active_tabs() {
        for (axis, field, id) in [
            (Axis::Mcp, "mcp", "new-mcp"),
            (Axis::Plugins, "plugins", "new-plugin"),
            (Axis::Skills, "skills", "skill-new"),
        ] {
            for selection in [
                None,
                Some(Selection::only(vec![])),
                Some(Selection {
                    add: vec![id.into()],
                    ..Default::default()
                }),
            ] {
                let board = Mutex::new(board());
                let mut expected = serde_json::to_value(&*lock(&board)).unwrap();
                expected["workspaces"][0][field] = serde_json::to_value(&selection).unwrap();
                change(&board, "w", axis, selection);
                assert_eq!(serde_json::to_value(&*lock(&board)).unwrap(), expected);
            }
        }
    }

    #[test]
    fn missing_workspace_has_no_effects() {
        let board = Mutex::new(board());
        let before = serde_json::to_value(&*lock(&board)).unwrap();
        change(&board, "missing", Axis::Mcp, Some(Selection::only(vec![])));
        assert_eq!(serde_json::to_value(&*lock(&board)).unwrap(), before);
    }
}
