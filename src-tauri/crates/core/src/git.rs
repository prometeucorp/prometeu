//! Existing Git application contract and injected native repository operations.
use crate::board::{Board, Repo, Status};
use crate::error::code;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct GitFile {
    pub path: String,
    pub status: String,
}

#[derive(Clone, Default, Serialize)]
pub struct GitStatus {
    pub repo: usize,
    pub name: String,
    pub branch: Option<String>,
    pub base: String,
    pub upstream: Option<String>,
    pub remotes: Vec<String>,
    pub ahead: u32,
    pub behind: u32,
    pub has_head: bool,
    pub merging: bool,
    pub index: String,
    pub staged: Vec<GitFile>,
    pub changes: Vec<GitFile>,
    pub conflicts: Vec<GitFile>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    Staged,
    Changes,
    Compare,
    Commit,
}

#[derive(Serialize)]
pub struct GitDiff {
    pub base: String,
    pub head: String,
    pub files: Vec<FileChange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitAction {
    Stage,
    Unstage,
    Commit,
    Fetch,
    Pull,
    Push,
    Publish,
    /// Throw away unstaged work: tracked paths return to their index version and untracked ones
    /// are removed. Staged content and conflicts are never touched.
    Discard,
}

#[derive(Serialize)]
pub struct GitCommit {
    pub oid: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub outgoing: bool,
}

#[derive(Serialize)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    pub remote: bool,
    pub worktree: Option<String>,
    pub workspace: Option<String>,
}

#[derive(Serialize)]
pub struct GitConflict {
    pub current: String,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

#[derive(Serialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub new_file: bool,
    pub deleted: bool,
    pub dirty: bool,
    pub patch: String,
}

pub struct Mutation<'a> {
    pub operation: GitAction,
    pub paths: &'a [String],
    pub message: Option<&'a str>,
    pub expected: Option<&'a str>,
    pub remote: Option<&'a str>,
}
pub trait RepositoryGit: Send + Sync {
    fn file_base(&self, root: &Path, repos: &[PathBuf], rel: &str) -> Option<String>;
    fn status(&self, repos: &[Repo]) -> Vec<GitStatus>;
    fn diff(
        &self,
        repo: &Repo,
        scope: DiffScope,
        path: Option<&str>,
        reference: Option<&str>,
    ) -> Result<GitDiff, String>;
    fn action(&self, repo: &Repo, mutation: Mutation<'_>) -> Result<(), String>;
    fn history(&self, repo: &Repo) -> Result<Vec<GitCommit>, String>;
    fn branches(&self, repo: &Repo, board: &Board) -> Result<Vec<GitBranch>, String>;
    fn conflict(&self, repo: &Repo, path: &str) -> Result<GitConflict, String>;
    fn resolve(&self, repo: &Repo, path: &str, was: &str, text: &str) -> Result<(), String>;
    fn tree(&self, root: &Path, repos: &[PathBuf]) -> Vec<GitFile>;
    fn restore(&self, root: &Path, repos: &[PathBuf], rel: &str) -> Result<(), String>;
    fn invalidate(&self, path: &Path);
}
/// Keep workspace admission identical at both host boundaries.
pub fn workspace_repos<'a>(board: &'a Board, id: &str) -> Result<&'a [Repo], String> {
    board
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned && !w.preparing && w.failed.is_none())
        .map(|w| w.repos.as_slice())
        .ok_or_else(|| code("err.session.noWorkspace"))
}
pub fn repository<'a>(board: &'a Board, id: &str, repo: usize) -> Result<&'a Repo, String> {
    workspace_repos(board, id)?
        .get(repo)
        .ok_or_else(|| code("err.session.noWorkspace"))
}
pub fn admit_mutation(board: &Board, id: &str, operation: &GitAction) -> Result<(), String> {
    if matches!(operation, GitAction::Pull | GitAction::Discard)
        && board
            .workspaces
            .iter()
            .any(|w| w.id == id && w.tabs.iter().any(|t| t.status == Status::Rodando))
    {
        return Err(code("err.git.agent"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_keeps_workspace_availability_separate_from_running_agent_protection() {
        let workspace = serde_json::from_value(serde_json::json!({
            "id":"workspace", "title":"Test", "repo":"/project", "repo_name":"project",
            "branch":"main", "worktree":"/checkout", "stage":"Fazendo",
            "repos":[{"path":"/project", "name":"project", "worktree":"/checkout"}],
            "tabs":[{"id":"session", "title":"Test", "status":"rodando"}]
        }))
        .unwrap();
        let mut board = Board {
            workspaces: vec![workspace],
            ..Default::default()
        };
        assert!(repository(&board, "workspace", 0).is_ok());
        assert!(repository(&board, "workspace", 1).is_err());
        assert!(repository(&board, "missing", 0).is_err());
        for operation in [GitAction::Pull, GitAction::Discard] {
            assert_eq!(
                admit_mutation(&board, "workspace", &operation),
                Err(code("err.git.agent"))
            );
        }
        assert!(admit_mutation(&board, "workspace", &GitAction::Stage).is_ok());
        board.workspaces[0].tabs[0].status = Status::Pronta;
        assert!(admit_mutation(&board, "workspace", &GitAction::Discard).is_ok());
        board.workspaces[0].preparing = true;
        assert!(repository(&board, "workspace", 0).is_err());
        board.workspaces[0].preparing = false;
        board.workspaces[0].failed = Some("preparation failed".into());
        assert!(repository(&board, "workspace", 0).is_err());
        board.workspaces[0].failed = None;
        board.workspaces[0].cleaned = true;
        assert!(repository(&board, "workspace", 0).is_err());
    }
}
