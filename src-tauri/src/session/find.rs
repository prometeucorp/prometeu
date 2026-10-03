//! Desktop root admission for the shared native path search.
use super::{cwd_of, repos_of};
use crate::AppState;
use prometeu_core::files::Entry;
use tauri::State;

#[tauri::command(async)]
pub fn find_paths(
    state: State<AppState>,
    id: String,
    query: String,
    recent: Vec<String>,
    files: Option<bool>,
) -> Vec<Entry> {
    let Some(root) = cwd_of(&state, &id) else {
        return Vec::new();
    };
    let repositories = repos_of(&state, &id)
        .into_iter()
        .map(|repo| repo.worktree.into())
        .collect::<Vec<_>>();
    state.project_search.find(
        &root,
        &repositories,
        &query,
        &recent,
        files.unwrap_or(false),
    )
}
