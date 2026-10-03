//! Bounded Git preparation for new-branch workspaces. No shell or desktop dependencies.
use crate::workspaces::LinuxFolders;
use prometeu_core::{
    command::{CommandPolicy, CommandRunner, OutputPolicy},
    workspaces::{PreparedWorktree, WorkspaceFolders, WorkspaceWorktrees, WorktreeRequest},
};
use std::{
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct GitWorktrees {
    pub root: PathBuf,
    pub runner: Arc<dyn CommandRunner<Command>>,
}
impl WorkspaceWorktrees for GitWorktrees {
    fn prepare(&self, id: &str, request: &WorktreeRequest) -> Result<PreparedWorktree, String> {
        if !uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id) {
            return Err("workspace_catalog_invalid".into());
        }
        let source = LinuxFolders.inspect(&request.path)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let git = |args: &[&str]| -> Result<String, String> {
            let mut command = Command::new("git");
            command
                .arg("-C")
                .arg(&source.path)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0");
            for key in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_COMMON_DIR",
            ] {
                command.env_remove(key);
            }
            let output = self
                .runner
                .run(
                    &mut command,
                    &[],
                    CommandPolicy {
                        timeout: deadline.saturating_duration_since(Instant::now()),
                        stdout: OutputPolicy::Capture { limit: 256 * 1024 },
                        stderr: OutputPolicy::Capture { limit: 256 * 1024 },
                    },
                )
                .map_err(|e| format!("{e:?}"))?;
            if !output.success {
                return Err(String::from_utf8_lossy(&output.stderr).trim().into());
            }
            String::from_utf8(output.stdout)
                .map(|s| s.strip_suffix('\n').unwrap_or(&s).to_owned())
                .map_err(|e| e.to_string())
        };
        let repository =
            git(&["rev-parse", "--show-toplevel"]).map_err(|_| "workspace_repository_invalid")?;
        let project = LinuxFolders.inspect(&repository)?;
        let branch = request.branch.trim();
        // Full ref validation avoids --branch's checkout-history expansion and option parsing.
        if branch.is_empty() || branch.starts_with('-') || branch == "HEAD" || branch.len() > 240 {
            return Err("workspace_branch_invalid".into());
        }
        git(&["check-ref-format", &format!("refs/heads/{branch}")])
            .map_err(|_| "workspace_branch_invalid")?;
        let base = request.base.trim();
        if base.is_empty() || base.len() > 1024 || base.contains('\0') {
            return Err("workspace_base_invalid".into());
        }
        let commit = git(&[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ])
        .map_err(|_| "workspace_base_invalid")?;
        let parent = self.root.join("checkouts");
        std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
        let path = parent.canonicalize().map_err(|e| e.to_string())?.join(id);
        let path = path.to_str().ok_or("workspace_folder_invalid")?.to_owned();
        // -b refuses preexisting branches; Git also refuses existing checkout paths.
        // On timeout/error, retain every artifact and report the path. External hooks or
        // processes may already have written files, so forced rollback is not safe.
        git(&["worktree", "add", "-b", branch, "--", &path, &commit])
            .map_err(|error| format!("workspace_worktree_failed: {path}\n{error}"))?;
        Ok(PreparedWorktree {
            project,
            path,
            branch: branch.into(),
            base: base.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_process::command::UnixCommandRunner;
    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn git(path: &std::path::Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "commit.gpgsign=false"])
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    #[test]
    fn creates_from_selected_commit_preserving_dirty_source_and_refusing_existing_branches() {
        let fixture =
            Fixture(std::env::temp_dir().join(format!("worktree ' ação-{}", uuid::Uuid::new_v4())));
        let repo = fixture.0.join("repo");
        std::fs::create_dir_all(repo.join("nested")).unwrap();
        let alias = fixture.0.join("source alias");
        std::os::unix::fs::symlink(&repo, &alias).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.name", "Test"]);
        git(&repo, &["config", "user.email", "test@example.invalid"]);
        std::fs::write(repo.join("tracked"), "initial").unwrap();
        git(&repo, &["add", "tracked"]);
        git(&repo, &["commit", "-m", "initial"]);
        let original = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::write(repo.join("tracked"), "second").unwrap();
        git(&repo, &["commit", "-am", "second"]);
        std::fs::write(repo.join("tracked"), "uncommitted").unwrap();
        std::fs::write(repo.join("untracked"), "keep").unwrap();
        let before = git(&repo, &["status", "--porcelain"]);
        let adapter = GitWorktrees {
            root: fixture.0.join("private"),
            runner: Arc::new(UnixCommandRunner),
        };
        let mut request = WorktreeRequest {
            title: "Feature".into(),
            path: alias.join("nested").to_str().unwrap().into(),
            branch: "feature/isolated".into(),
            base: "HEAD~1".into(),
        };
        let prepared = adapter
            .prepare(&uuid::Uuid::new_v4().to_string(), &request)
            .unwrap();
        assert_eq!(
            prepared.project.path,
            repo.canonicalize().unwrap().to_str().unwrap()
        );
        let checkout = std::path::Path::new(&prepared.path);
        assert_eq!(git(checkout, &["rev-parse", "HEAD"]), original);
        assert_eq!(
            git(checkout, &["branch", "--show-current"]),
            "feature/isolated"
        );
        assert_eq!(
            std::fs::read_to_string(checkout.join("tracked")).unwrap(),
            "initial"
        );
        assert!(!checkout.join("untracked").exists());
        assert_eq!(git(&repo, &["branch", "--show-current"]), "main");
        assert_eq!(git(&repo, &["status", "--porcelain"]), before);
        let error = adapter
            .prepare(&uuid::Uuid::new_v4().to_string(), &request)
            .err()
            .unwrap();
        assert!(error.starts_with("workspace_worktree_failed:"));
        assert_eq!(git(checkout, &["rev-parse", "HEAD"]), original);
        request.branch = "-f".into();
        assert_eq!(
            adapter
                .prepare(&uuid::Uuid::new_v4().to_string(), &request)
                .err()
                .unwrap(),
            "workspace_branch_invalid"
        );
        request.branch = "feature/missing".into();
        request.base = "--help".into();
        assert_eq!(
            adapter
                .prepare(&uuid::Uuid::new_v4().to_string(), &request)
                .err()
                .unwrap(),
            "workspace_base_invalid"
        );
        assert_eq!(git(&repo, &["status", "--porcelain"]), before);
        assert_eq!(
            std::fs::read_dir(fixture.0.join("private/checkouts"))
                .unwrap()
                .count(),
            1
        );
        struct Interrupted;
        impl CommandRunner<Command> for Interrupted {
            fn run(
                &self,
                command: &mut Command,
                input: &[u8],
                policy: CommandPolicy,
            ) -> Result<prometeu_core::command::CommandOutput, prometeu_core::command::CommandError>
            {
                let args: Vec<_> = command.get_args().collect();
                if args.iter().any(|a| *a == "worktree") {
                    let separator = args.iter().position(|a| *a == "--").unwrap();
                    let path = std::path::Path::new(args[separator + 1]);
                    std::fs::create_dir_all(path).unwrap();
                    std::fs::write(path.join("external-write"), "preserve after timeout").unwrap();
                    return Err(prometeu_core::command::CommandError::Timeout);
                }
                UnixCommandRunner.run(command, input, policy)
            }
        }
        request.base = "HEAD".into();
        let interrupted = GitWorktrees {
            root: adapter.root.clone(),
            runner: Arc::new(Interrupted),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let error = interrupted.prepare(&id, &request).err().unwrap();
        assert!(error.contains("Timeout"));
        assert!(error.contains(&id));
        assert_eq!(
            std::fs::read_to_string(
                adapter
                    .root
                    .join("checkouts")
                    .join(id)
                    .join("external-write")
            )
            .unwrap(),
            "preserve after timeout"
        );
    }
}
