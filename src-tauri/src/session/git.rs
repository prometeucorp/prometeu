//! Desktop admission and IPC over the shared injected Git repository service.
use crate::{lock::lock, AppState};
use prometeu_core::git::{self, Mutation};
pub use prometeu_core::git::{
    DiffScope, GitAction, GitBranch, GitCommit, GitConflict, GitDiff, GitFile, GitStatus,
};
use std::path::{Path, PathBuf};
use tauri::State;

#[tauri::command(async)]
pub fn workspace_git_status(state: State<AppState>, id: String) -> Result<Vec<GitStatus>, String> {
    let repos = git::workspace_repos(&lock(&state.board), &id)?.to_vec();
    Ok(state.repository_git.status(&repos))
}
#[tauri::command(async)]
pub fn workspace_git_diff(
    state: State<AppState>,
    id: String,
    repo: usize,
    scope: DiffScope,
    path: Option<String>,
    reference: Option<String>,
) -> Result<GitDiff, String> {
    let repo = git::repository(&lock(&state.board), &id, repo)?.clone();
    state
        .repository_git
        .diff(&repo, scope, path.as_deref(), reference.as_deref())
}
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn workspace_git_action(
    state: State<AppState>,
    id: String,
    repo: usize,
    operation: GitAction,
    paths: Vec<String>,
    message: Option<String>,
    expected: Option<String>,
    remote: Option<String>,
) -> Result<(), String> {
    let repo = {
        let board = lock(&state.board);
        git::admit_mutation(&board, &id, &operation)?;
        git::repository(&board, &id, repo)?.clone()
    };
    state.repository_git.action(
        &repo,
        Mutation {
            operation,
            paths: &paths,
            message: message.as_deref(),
            expected: expected.as_deref(),
            remote: remote.as_deref(),
        },
    )
}
#[tauri::command(async)]
pub fn workspace_git_history(
    state: State<AppState>,
    id: String,
    repo: usize,
) -> Result<Vec<GitCommit>, String> {
    let repo = git::repository(&lock(&state.board), &id, repo)?.clone();
    state.repository_git.history(&repo)
}
#[tauri::command(async)]
pub fn workspace_git_branches(
    state: State<AppState>,
    id: String,
    repo: usize,
) -> Result<Vec<GitBranch>, String> {
    let board = lock(&state.board).clone();
    let repo = git::repository(&board, &id, repo)?;
    state.repository_git.branches(repo, &board)
}
#[tauri::command(async)]
pub fn workspace_git_conflict(
    state: State<AppState>,
    id: String,
    repo: usize,
    path: String,
) -> Result<GitConflict, String> {
    let repo = git::repository(&lock(&state.board), &id, repo)?.clone();
    state.repository_git.conflict(&repo, &path)
}
#[tauri::command(async)]
pub fn workspace_git_resolve(
    state: State<AppState>,
    id: String,
    repo: usize,
    path: String,
    was: String,
    text: String,
) -> Result<(), String> {
    let repo = git::repository(&lock(&state.board), &id, repo)?.clone();
    state.repository_git.resolve(&repo, &path, &was, &text)
}
fn tree_repos(state: &State<AppState>, id: &str) -> Option<(PathBuf, Vec<PathBuf>)> {
    let root = super::cwd_of(state, id)?;
    let repos = git::workspace_repos(&lock(&state.board), id)
        .map(|repos| repos.iter().map(|r| PathBuf::from(&r.worktree)).collect())
        .unwrap_or_else(|_| vec![root.clone()]);
    Some((root, repos))
}
#[tauri::command(async)]
pub fn tree_git_status(state: State<AppState>, id: String) -> Vec<GitFile> {
    tree_repos(&state, &id)
        .map(|(root, repos)| state.repository_git.tree(&root, &repos))
        .unwrap_or_default()
}
#[tauri::command(async)]
pub fn tree_restore(state: State<AppState>, id: String, rel: String) -> Result<(), String> {
    let (root, repos) =
        tree_repos(&state, &id).ok_or_else(|| crate::i18n::t("err.session.noWorkspace"))?;
    state.repository_git.restore(Path::new(&root), &repos, &rel)
}
