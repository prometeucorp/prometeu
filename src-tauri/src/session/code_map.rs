//! Workspace-scoped desktop admission for the shared local code map.
use crate::{lock::lock, AppState};
use prometeu_tools::search::CodeMapSnapshot;
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
pub async fn workspace_code_map(
    state: State<'_, AppState>,
    id: String,
    root: Option<String>,
    query: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    force: Option<bool>,
) -> Result<CodeMapSnapshot, String> {
    let workdir = PathBuf::from(prometeu_core::files::code_map_root(
        &lock(&state.board),
        &id,
    )?);
    let map = state.code_map.clone();
    tauri::async_runtime::spawn_blocking(move || {
        map.snapshot(
            &workdir,
            root.as_deref(),
            query.as_deref().unwrap_or_default(),
            path.as_deref(),
            line,
            force.unwrap_or(false),
        )
    })
    .await
    .map_err(crate::i18n::io)?
}
