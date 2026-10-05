//! Recoverable, run-owned worktrees. The host persists reservations before calling `prepare`
//! and holds its source-resource execution gate throughout preparation and worker execution.
use super::adapters;
use prometeu_core::{
    automation::Run,
    board::{Board, Project, Workspace},
    command::{CommandOutput, CommandPolicy, CommandRunner, OutputPolicy},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Reservation {
    pub run_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub source: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub starting_sha: String,
    pub source_key: String,
    pub(super) github: Option<PullRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PullRequest {
    pub repository: String,
    pub number: u64,
    pub identity: String,
    pub head_branch: String,
}

fn sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn mapping(run: &Run) -> Result<(&str, Option<PullRequest>), String> {
    let project = if run.workflow.scope.targets.is_empty() {
        run.workflow
            .scope
            .project_id
            .as_deref()
            .ok_or("automation_project_required")?
    } else {
        super::scope(run)?.0
    };
    let github = if run.event.get("headSha").is_some() {
        let (_, repository, identity) = super::scope(run)?;
        if !adapters::valid_repository(repository)
            || run.event["repository"].as_str() != Some(repository)
            || run.event["identity"].as_str() != Some(identity)
            || !run.event["headSha"].as_str().is_some_and(sha)
        {
            return Err("automation_workspace_event_mismatch".into());
        }
        Some(PullRequest {
            repository: repository.into(),
            number: run.event["number"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("automation_pull_number")?,
            identity: identity.into(),
            head_branch: run.event["headRefName"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("automation_branch_required")?
                .into(),
        })
    } else {
        // A PR event without a head is never allowed to fall back to the local clone's HEAD.
        if run.workflow.nodes.iter().any(|node| matches!(&node.config,
            prometeu_core::automation::NodeConfig::Trigger { event, .. } if event == "github.authored_pr"))
            || run.event.get("repository").is_some() || run.event.get("number").is_some()
        {
            return Err("automation_head_sha".into());
        }
        if run.workflow.nodes.iter().any(|node| matches!(&node.config,
            prometeu_core::automation::NodeConfig::Trigger { event, .. } if event == "linear.assigned_issue")) {
            let linear_project = run.workflow.scope.linear_project_id.as_deref().filter(|s| !s.is_empty())
                .ok_or("automation_linear_mapping_required")?;
            let identity = run.workflow.scope.identity.as_deref().filter(|s| !s.is_empty())
                .ok_or("automation_identity_required")?;
            if run.event["project"]["id"].as_str() != Some(linear_project)
                || run.event["identity"].as_str() != Some(identity) {
                return Err("automation_linear_mapping_required".into());
            }
        }
        None
    };
    Ok((project, github))
}

fn project_source(projects: &[Project], id: &str) -> Result<PathBuf, String> {
    let project = projects
        .iter()
        .find(|p| p.id == id)
        .ok_or("automation_project_missing")?;
    std::fs::canonicalize(&project.path).map_err(|_| "automation_project_missing".into())
}

/// Read-only planning. In particular, no fetch or worktree write precedes the durable reservation.
pub(super) fn reserve(
    root: &Path,
    run: &Run,
    projects: &[Project],
    runner: &dyn CommandRunner<Command>,
) -> Result<Reservation, String> {
    if !uuid::Uuid::parse_str(&run.id).is_ok_and(|id| id.to_string() == run.id)
        || !root.is_absolute()
    {
        return Err("automation_workspace_identity".into());
    }
    let (project_id, github) = mapping(run)?;
    let source = project_source(projects, project_id)?;
    let top = git(runner, &source, &["rev-parse", "--show-toplevel"])?;
    if std::fs::canonicalize(top).ok().as_ref() != Some(&source) {
        return Err("automation_project_git_root".into());
    }
    let starting_sha = if github.is_some() {
        run.event["headSha"]
            .as_str()
            .ok_or("automation_head_sha")?
            .into()
    } else {
        git(runner, &source, &["rev-parse", "--verify", "HEAD^{commit}"])?
    };
    if !sha(&starting_sha) {
        return Err("automation_head_sha".into());
    }
    let source_key = github.as_ref().map_or_else(
        || format!("project:{project_id}"),
        |pr| format!("github:{}:branch:{}", pr.repository, pr.head_branch),
    );
    Ok(Reservation {
        run_id: run.id.clone(),
        workspace_id: run.id.clone(),
        project_id: project_id.into(),
        source,
        path: root.join("automations/worktrees").join(&run.id),
        branch: format!("prometeu-automation/{}", run.id),
        starting_sha,
        source_key,
        github,
    })
}

impl Reservation {
    /// Revalidate persisted ownership without resolving a new Linear starting commit.
    pub(super) fn validate_for(
        &self,
        root: &Path,
        run: &Run,
        projects: &[Project],
    ) -> Result<(), String> {
        let (project, github) = mapping(run)?;
        if !uuid::Uuid::parse_str(&run.id).is_ok_and(|id| id.to_string() == run.id)
            || self.run_id != run.id
            || self.workspace_id != run.id
            || self.project_id != project
            || self.source != project_source(projects, project)?
            || self.path != root.join("automations/worktrees").join(&run.id)
            || self.branch != format!("prometeu-automation/{}", run.id)
            || !sha(&self.starting_sha)
            || self.github != github
            || self.source_key
                != github.as_ref().map_or_else(
                    || format!("project:{project}"),
                    |pr| format!("github:{}:branch:{}", pr.repository, pr.head_branch),
                )
            || (github.is_some()
                && run.event["headSha"].as_str() != Some(self.starting_sha.as_str()))
        {
            return Err("automation_workspace_reservation_mismatch".into());
        }
        Ok(())
    }
}

pub(super) fn command(
    runner: &dyn CommandRunner<Command>,
    dir: &Path,
    args: &[&str],
) -> Result<CommandOutput, String> {
    command_with_env(runner, dir, args, &[])
}

pub(super) fn command_with_env(
    runner: &dyn CommandRunner<Command>,
    dir: &Path,
    args: &[&str],
    environment: &[(&str, Option<&str>)],
) -> Result<CommandOutput, String> {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0");
    // A provider/launcher environment must not redirect these operations to another repository.
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_CONFIG_") {
            command.env_remove(key);
        }
    }
    for (key, value) in environment {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    runner
        .run(
            &mut command,
            &[],
            CommandPolicy {
                timeout: Duration::from_secs(60),
                stdout: OutputPolicy::Capture { limit: 1024 * 1024 },
                stderr: OutputPolicy::Capture { limit: 1024 * 1024 },
            },
        )
        .map_err(|_| "automation_git_unavailable".into())
}

pub(super) fn git(
    runner: &dyn CommandRunner<Command>,
    dir: &Path,
    args: &[&str],
) -> Result<String, String> {
    let output = command(runner, dir, args)?;
    if !output.success {
        return Err(
            "automation_git_failed: Check repository access and the reserved worktree".into(),
        );
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "automation_git_response".into())
}

fn current_pull(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    if let Some(pr) = &reservation.github {
        adapters::require_identity(runner, &pr.identity)?;
        let current = adapters::pr_status(runner, &pr.repository, pr.number)?;
        if current["headRefOid"].as_str() != Some(&reservation.starting_sha)
            || current["headRefName"].as_str() != Some(&pr.head_branch)
            || current["state"].as_str() != Some("OPEN")
        {
            return Err(
                "automation_pr_head_changed: The pull request no longer matches this run".into(),
            );
        }
    }
    Ok(())
}

fn common_dir(runner: &dyn CommandRunner<Command>, dir: &Path) -> Result<PathBuf, String> {
    let path = git(runner, dir, &["rev-parse", "--git-common-dir"])?;
    std::fs::canonicalize(dir.join(path)).map_err(|_| "automation_worktree_repository".into())
}

fn verify_local(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
    require_clean: bool,
) -> Result<(), String> {
    verify_local_head(
        reservation,
        &reservation.starting_sha,
        runner,
        require_clean,
    )
}

fn verify_local_head(
    reservation: &Reservation,
    expected_head: &str,
    runner: &dyn CommandRunner<Command>,
    require_clean: bool,
) -> Result<(), String> {
    if !sha(expected_head) {
        return Err("automation_head_sha".into());
    }
    let path = &reservation.path;
    let meta = std::fs::symlink_metadata(path).map_err(|_| "automation_worktree_missing")?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || std::fs::canonicalize(path).ok().as_deref() != Some(path.as_path())
        || !std::fs::symlink_metadata(path.join(".git"))
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        || common_dir(runner, path)? != common_dir(runner, &reservation.source)?
        || !git(
            runner,
            &reservation.source,
            &["worktree", "list", "--porcelain", "-z"],
        )?
        .split('\0')
        .any(|entry| {
            entry
                .strip_prefix("worktree ")
                .is_some_and(|entry| Path::new(entry) == path)
        })
        || git(runner, path, &["rev-parse", "--verify", "HEAD^{commit}"])? != expected_head
        || git(runner, path, &["symbolic-ref", "--short", "HEAD"])? != reservation.branch
        || (require_clean
            && !git(
                runner,
                path,
                &[
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignored",
                ],
            )?
            .is_empty())
    {
        return Err("automation_worktree_changed: Inspect this workspace; its files, branch or commit changed".into());
    }
    Ok(())
}

/// Verification is also required immediately before each restricted worker receives the path.
pub(super) fn verify(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    current_pull(reservation, runner)?;
    require_no_filters(reservation, runner)?;
    verify_local(reservation, runner, true)
}

/// The committed head is a separate durable result; retain the immutable event starting SHA.
pub(super) fn verify_at_head(
    reservation: &Reservation,
    expected_head: &str,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    current_pull(reservation, runner)?;
    verify_local_head(reservation, expected_head, runner, false)
}

pub(super) fn require_no_filters(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    let filters = command(
        runner,
        &reservation.source,
        &[
            "config",
            "--get-regexp",
            "^filter\\..*\\.(clean|smudge|process)$",
        ],
    )?;
    if filters.success && !filters.stdout.is_empty() {
        return Err("automation_git_filters_unsupported: Configured Git filters require manual workspace preparation".into());
    }
    Ok(())
}

/// Recover only a reserved branch/path at the pinned commit. Never reset, clean, stash or force.
pub(super) fn prepare(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    current_pull(reservation, runner)?;
    require_no_filters(reservation, runner)?;
    if std::fs::symlink_metadata(&reservation.path).is_ok() {
        return verify_local(reservation, runner, true);
    }
    let parent = reservation
        .path
        .parent()
        .ok_or("automation_workspace_path")?;
    // Reject linked ancestors before create_dir_all or permission updates can touch their target.
    let ancestor = parent
        .ancestors()
        .find(|path| std::fs::symlink_metadata(path).is_ok())
        .ok_or("automation_workspace_path")?;
    if std::fs::canonicalize(ancestor).ok().as_deref() != Some(ancestor) {
        return Err("automation_workspace_path".into());
    }
    crate::paths::ensure_private_dir(parent)?;
    if std::fs::canonicalize(parent).ok().as_deref() != Some(parent) {
        return Err("automation_workspace_path".into());
    }
    if let Some(pr) = &reservation.github {
        let rewrites = command(
            runner,
            &reservation.source,
            &[
                "config",
                "--get-regexp",
                "^url\\..*\\.(insteadof|pushinsteadof)$",
            ],
        )?;
        if rewrites.success && !rewrites.stdout.is_empty() {
            return Err("automation_git_url_rewrite_unsupported".into());
        }
        let reference = format!("refs/prometeu/automations/{}", reservation.run_id);
        let fetched = format!("refs/pull/{}/head:{reference}", pr.number);
        let url = format!("https://github.com/{}.git", pr.repository);
        let username = format!("credential.https://github.com.username={}", pr.identity);
        git(
            runner,
            &reservation.source,
            &[
                "-c",
                "credential.helper=",
                "-c",
                "credential.helper=!gh auth git-credential",
                "-c",
                &username,
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.https.allow=always",
                "-c",
                "protocol.file.allow=never",
                "-c",
                "protocol.ext.allow=never",
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                "--",
                &url,
                &fetched,
            ],
        )?;
        if git(
            runner,
            &reservation.source,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )? != reservation.starting_sha
        {
            return Err(
                "automation_pr_head_changed: The fetched pull request differs from this run".into(),
            );
        }
    }
    let reference = format!("refs/heads/{}", reservation.branch);
    let existing = command(
        runner,
        &reservation.source,
        &["show-ref", "--verify", "--quiet", &reference],
    )?;
    let path = reservation
        .path
        .to_str()
        .ok_or("automation_workspace_path")?;
    if existing.success {
        if git(
            runner,
            &reservation.source,
            &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
        )? != reservation.starting_sha
        {
            return Err("automation_worktree_changed: The reserved branch was changed".into());
        }
        git(
            runner,
            &reservation.source,
            &["worktree", "add", "--", path, &reservation.branch],
        )?;
    } else {
        git(
            runner,
            &reservation.source,
            &[
                "worktree",
                "add",
                "-b",
                &reservation.branch,
                "--",
                path,
                &reservation.starting_sha,
            ],
        )?;
    }
    verify(reservation, runner)
}

fn register_in_board(
    board: &mut Board,
    reservation: &Reservation,
    title: &str,
    preparing: bool,
    error: Option<String>,
) -> Result<(), String> {
    if let Some(workspace) = board
        .workspaces
        .iter_mut()
        .find(|w| w.id == reservation.workspace_id)
    {
        if workspace.project != reservation.project_id
            || Path::new(&workspace.worktree) != reservation.path
            || Path::new(&workspace.repo) != reservation.source
            || workspace.branch != reservation.branch
            || workspace.cleaned
            || workspace.archived
        {
            return Err(
                "automation_workspace_changed: Restore or inspect the reserved workspace".into(),
            );
        }
        workspace.preparing = preparing;
        workspace.failed = error;
        return Ok(());
    }
    let project = board
        .projects
        .iter()
        .find(|p| p.id == reservation.project_id)
        .ok_or("automation_project_missing")?;
    // Deserialization supplies the established board defaults, with no sharing or conversation.
    let workspace: Workspace = serde_json::from_value(json!({
        "id": reservation.workspace_id, "title": title, "project": reservation.project_id,
        "repo": reservation.source, "repo_name": project.name, "branch": reservation.branch,
        "worktree": reservation.path,
        "repos": [{"path": reservation.source, "name": project.name, "worktree": reservation.path, "base": reservation.starting_sha, "pr": null}],
        "stage": board.stages.first().cloned().unwrap_or_else(|| "Preparando".into()),
        "preparing": preparing, "failed": error, "mcp": [], "plugins": [], "skills": [], "tabs": []
    })).map_err(|_| "automation_workspace_schema")?;
    board.workspaces.push(workspace);
    Ok(())
}

/// Durable visible reservation without the launcher's scripts, interactive session or selection.
pub(super) fn register(
    app: &tauri::AppHandle,
    reservation: &Reservation,
    title: &str,
    preparing: bool,
    error: Option<String>,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    register_in_board(
        &mut crate::lock::lock(&state.board),
        reservation,
        title,
        preparing,
        error,
    )?;
    crate::state::persist_now(app)?;
    crate::state::publish(app);
    Ok(())
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;
