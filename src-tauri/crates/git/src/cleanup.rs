//! Existing native cleanup checks and Git effects, shared by desktop and WSL.
use prometeu_core::{
    board::Workspace,
    domain::Pr,
    error::{code, with_args},
    workspace_lifecycle::{Cleanable, WorktreeCleanup},
};
use std::{
    path::{Path, PathBuf},
    process::Command,
};
pub struct NativeCleanup;
impl WorktreeCleanup for NativeCleanup {
    fn inspect(&self, ws: &Workspace) -> Cleanable {
        Cleanable {
            id: ws.id.clone(),
            title: ws.title.clone(),
            repo_name: ws.repo_name.clone(),
            branch: ws.branch.clone(),
            worktree: ws.worktree.clone(),
            size_kb: size_of(Path::new(&ws.worktree)),
            pr: ws.prs().next().map(|(_, p)| p.number),
            blocked: check(ws).err(),
        }
    }
    fn check(&self, ws: &Workspace, force: bool) -> Result<(), String> {
        match force {
            true => hard(ws),
            false => check(ws),
        }
    }
    fn remove(&self, ws: &Workspace) -> Result<(), String> {
        // Remove each repository's worktree through its own clone.
        for r in &ws.repos {
            let repo = PathBuf::from(&r.path);
            let wt = PathBuf::from(&r.worktree);
            if wt.exists() {
                // Force removes ignored files such as node_modules, target, and .env, plus uncommitted
                // changes only when the person explicitly approved losing them.
                let out = Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(["worktree", "remove", "--force"])
                    .arg(&wt)
                    .output()
                    .map_err(|e| with_args("err.git.spawn", &[("cause", e.to_string())]))?;
                if !out.status.success() {
                    return Err(with_args(
                        "err.git",
                        &[
                            ("command", "git worktree remove".into()),
                            (
                                "cause",
                                String::from_utf8_lossy(&out.stderr).trim().to_string(),
                            ),
                        ],
                    ));
                }
            }
            // Use -D after checking merge safety, or when force explicitly permits loss. A branch
            // deletion failure can leave a harmless ref after successful worktree cleanup, so it must
            // not turn that cleanup into an error.
            if !ws.branch.is_empty() && !ws.preserve_branches.contains(&r.path) {
                let _ = git(&repo, &["branch", "-D", &ws.branch]);
            }
            let _ = git(&repo, &["worktree", "prune"]);
        }
        Ok(())
    }
}
/// Return a translated error code when cleanup is unsafe. A missing worktree passes so cleanup can
/// reconcile the board with disk.
pub fn check(ws: &Workspace) -> Result<(), String> {
    hard(ws)?;
    // Every repository must pass before removing any part of a workspace.
    for r in &ws.repos {
        let wt = PathBuf::from(&r.worktree);
        if !wt.exists() {
            continue;
        }
        let dirty = git(&wt, &["status", "--porcelain"]).lines().count();
        if dirty > 0 {
            return Err(with_args("err.cleanup.dirty", &[("n", dirty.to_string())]));
        }
        if !ws.preserve_branches.contains(&r.path) && !merged(r.pr.as_ref(), &wt) {
            return Err(with_args(
                "err.cleanup.unmerged",
                &[("branch", ws.branch.clone())],
            ));
        }
    }
    Ok(())
}

/// Force never bypasses archiving or permits deleting the original clone.
pub fn hard(ws: &Workspace) -> Result<(), String> {
    // Archiving runs the repository's archive script while the worktree exists. Disk cleanup must
    // follow that step.
    if !ws.archived {
        return Err(code("err.cleanup.notArchived"));
    }
    if ws.worktree == ws.repo {
        return Err(code("err.cleanup.isRepo"));
    }
    Ok(())
}

/// Work is safe when GitHub reports the PR merged or Git reports the branch is an ancestor of its
/// target. The latter also supports merges outside GitHub and machines without gh.
fn merged(pr: Option<&Pr>, wt: &Path) -> bool {
    if pr.is_some_and(|pr| pr.merged()) {
        return true;
    }
    let head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let target = if head.is_empty() {
        "origin/main".to_string()
    } else {
        head
    };
    has_commit(wt, &target) && git_ok(wt, &["merge-base", "--is-ancestor", "HEAD", &target])
}

/// Use the system du command for kilobytes. Unknown size becomes zero because an unavailable
/// estimate must not block cleanup.
fn size_of(wt: &Path) -> u64 {
    if !wt.exists() {
        return 0;
    }
    let out = Command::new("du").arg("-sk").arg(wt).output().ok();
    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
    })
    .unwrap_or(0)
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}
fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}
fn has_commit(dir: &Path, reference: &str) -> bool {
    git_ok(
        dir,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
}
