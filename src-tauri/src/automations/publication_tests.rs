use super::super::workspace::PullRequest;
use super::*;
use prometeu_core::command::{CommandError, CommandOutput, CommandPolicy};
use serde_json::json;
use std::{path::PathBuf, sync::Mutex};

struct Runner {
    remote: PathBuf,
    remote_head: Mutex<String>,
    fork: bool,
    pushes: Mutex<Vec<Vec<String>>>,
    race_rewind: bool,
    switch_head_at_commit: bool,
}
impl CommandRunner<Command> for Runner {
    fn run(
        &self,
        request: &mut Command,
        _: &[u8],
        _: CommandPolicy,
    ) -> Result<CommandOutput, CommandError> {
        let args: Vec<String> = request
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        if request.get_program() == "gh" {
            let value = if args.first().is_some_and(|a| a == "api") {
                json!({"login":"owner"})
            } else {
                json!({"headRefOid":self.remote_head.lock().unwrap().clone(),"headRefName":"feature","state":"OPEN","isCrossRepository":self.fork})
            };
            return Ok(CommandOutput {
                success: true,
                stdout: serde_json::to_vec(&value).unwrap(),
                stderr: vec![],
            });
        }
        assert_eq!(request.get_program(), "git");
        if self.switch_head_at_commit && args.iter().any(|arg| arg == "commit-tree") {
            let path = request.get_current_dir().unwrap();
            let head = local_git(path, &["rev-parse", "HEAD"]);
            local_git(path, &["update-ref", "refs/heads/personal", &head]);
            let git_dir = local_git(path, &["rev-parse", "--absolute-git-dir"]);
            // Model even an actor bypassing Git's locks. The explicit branch CAS must never
            // accidentally dereference this new HEAD and commit to the person's branch.
            std::fs::write(
                Path::new(&git_dir).join("HEAD"),
                "ref: refs/heads/personal\n",
            )
            .unwrap();
        }

        let output = if let Some(index) = args
            .iter()
            .position(|arg| arg == "push" || arg == "ls-remote")
        {
            assert!(args.contains(&"https://github.com/owner/repo.git".to_owned()));
            assert!(args.contains(&"credential.helper=".to_owned()));
            assert!(args.contains(&"credential.https://github.com.username=owner".to_owned()));
            for key in ["GH_TOKEN", "GITHUB_TOKEN"] {
                assert!(request
                    .get_envs()
                    .any(|(name, value)| name == key && value.is_none()));
            }
            let mut local: Vec<String> = args[index..].to_vec();
            for value in &mut local {
                if value == "https://github.com/owner/repo.git" {
                    *value = self.remote.to_str().unwrap().into();
                }
            }
            if args[index] == "push" {
                assert!(args
                    .iter()
                    .any(|arg| arg.starts_with("--force-with-lease=refs/heads/feature:")));
                assert!(!args.iter().any(|arg| arg == "--force" || arg == "+"));
                self.pushes.lock().unwrap().push(args.clone());
                if self.race_rewind {
                    let head = self.remote_head.lock().unwrap().clone();
                    let parent = local_git(&self.remote, &["rev-parse", &format!("{head}^")]);
                    local_git(&self.remote, &["update-ref", "refs/heads/feature", &parent]);
                }
            }
            isolate_git_config(&mut Command::new("git"))
                .args(["-c", "core.hooksPath=/dev/null"])
                .args(&local)
                .current_dir(request.get_current_dir().unwrap())
                .output()
        } else {
            assert!(!args.iter().any(|arg| arg.contains("://")));
            isolate_git_config(request).output()
        }
        .map_err(|error| CommandError::Io(error.to_string()))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}
struct Fixture {
    root: PathBuf,
    reservation: Reservation,
    runner: Runner,
}
impl Fixture {
    fn new(github: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("automation-publication-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        let remote = root.join("remote.git");
        std::fs::create_dir_all(&source).unwrap();
        let root = root.canonicalize().unwrap();
        let source = source.canonicalize().unwrap();
        local_git(&source, &["init", "--initial-branch=main"]);
        local_git(&source, &["config", "user.name", "Automation test"]);
        local_git(
            &source,
            &["config", "user.email", "automation@example.invalid"],
        );
        std::fs::write(source.join("file.txt"), "initial\n").unwrap();
        std::fs::write(source.join(".gitignore"), "build/\n").unwrap();
        local_git(&source, &["add", "."]);
        local_git(
            &source,
            &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
        );
        std::fs::write(source.join("second.txt"), "second\n").unwrap();
        local_git(&source, &["add", "."]);
        local_git(
            &source,
            &["-c", "commit.gpgsign=false", "commit", "-m", "second"],
        );
        let initial = local_git(&source, &["rev-parse", "HEAD"]);
        local_git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        local_git(
            &source,
            &["push", remote.to_str().unwrap(), "HEAD:refs/heads/feature"],
        );
        let id = uuid::Uuid::new_v4().to_string();
        let mut reservation = Reservation {
            run_id: id.clone(),
            workspace_id: id.clone(),
            project_id: "project".into(),
            source,
            path: root.join("automations/worktrees").join(&id),
            branch: format!("prometeu-automation/{id}"),
            starting_sha: initial.clone(),
            source_key: "project:project".into(),
            github: None,
        };
        let runner = Runner {
            remote,
            remote_head: Mutex::new(initial),
            fork: false,
            pushes: Mutex::new(vec![]),
            race_rewind: false,
            switch_head_at_commit: false,
        };
        workspace::prepare(&reservation, &runner).unwrap();
        if github {
            reservation.github = Some(PullRequest {
                repository: "owner/repo".into(),
                number: 42,
                identity: "owner".into(),
                head_branch: "feature".into(),
            });
        }
        Self {
            root,
            reservation,
            runner,
        }
    }
    fn edit(&self) {
        std::fs::write(self.reservation.path.join("file.txt"), "fixed\n").unwrap();
    }
    fn commit(&self) -> (CommitResult, String) {
        self.edit();
        let stamp = source_fingerprint(&self.reservation, &self.runner).unwrap();
        let result = commit(
            &self.reservation,
            &self.reservation.starting_sha,
            &stamp,
            "Fix validated behavior",
            &self.runner,
        )
        .unwrap();
        (result, stamp)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
/// Test Git processes read only each temporary repository's local configuration. Apply this
/// after production command construction, which intentionally clears inherited config overrides.
/// Preserve its private GIT_INDEX_FILE and other repository-specific execution settings.
fn isolate_git_config(command: &mut Command) -> &mut Command {
    let overrides: Vec<_> = std::env::vars_os()
        .map(|(key, _)| key)
        .chain(command.get_envs().map(|(key, _)| key.to_owned()))
        .filter(|key| key == "GIT_CONFIG" || key.to_string_lossy().starts_with("GIT_CONFIG_"))
        .collect();
    for key in overrides {
        command.env_remove(key);
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
}

fn local_git(root: &Path, args: &[&str]) -> String {
    let output = isolate_git_config(&mut Command::new("git"))
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn source_evidence_tracks_bytes_and_survives_artifacts_and_deletion_commit() {
    let fixture = Fixture::new(false);
    let before = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    fixture.edit();
    let edited = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    assert_ne!(before, edited);
    std::fs::create_dir(fixture.reservation.path.join("build")).unwrap();
    std::fs::write(fixture.reservation.path.join("build/output"), "artifact").unwrap();
    assert_eq!(
        source_fingerprint(&fixture.reservation, &fixture.runner).unwrap(),
        edited
    );
    std::fs::remove_file(fixture.reservation.path.join("second.txt")).unwrap();
    std::fs::write(fixture.reservation.path.join("new.txt"), "new\n").unwrap();
    let validated = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    let result = commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &validated,
        "Apply validated changes",
        &fixture.runner,
    )
    .unwrap();
    assert_eq!(
        source_fingerprint(&fixture.reservation, &fixture.runner).unwrap(),
        validated
    );
    assert_eq!(
        local_git(&fixture.reservation.path, &["rev-parse", "HEAD^"]),
        fixture.reservation.starting_sha
    );
    assert_eq!(
        result.changed_files,
        vec!["file.txt", "new.txt", "second.txt"]
    );
}

#[test]
fn stale_evidence_and_preexisting_index_cannot_commit() {
    let fixture = Fixture::new(false);
    let stamp = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    fixture.edit();
    assert!(commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &stamp,
        "Fix",
        &fixture.runner
    )
    .is_err());
    let stamp = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    local_git(&fixture.reservation.path, &["add", "file.txt"]);
    assert!(commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &stamp,
        "Fix",
        &fixture.runner
    )
    .unwrap_err()
    .starts_with("automation_commit_index_changed"));
    assert_eq!(
        local_git(&fixture.reservation.path, &["rev-parse", "HEAD"]),
        fixture.reservation.starting_sha
    );
}

#[test]
fn clean_filter_is_refused_and_hooks_are_disabled() {
    let fixture = Fixture::new(false);
    let sentinel = fixture.root.join("executed");
    fixture.edit();
    let stamp = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    local_git(
        &fixture.reservation.source,
        &[
            "config",
            "filter.external.clean",
            &format!("touch '{}'", sentinel.display()),
        ],
    );
    assert!(commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &stamp,
        "Fix",
        &fixture.runner
    )
    .unwrap_err()
    .starts_with("automation_commit_filters_unsupported"));
    assert!(!sentinel.exists());
    local_git(
        &fixture.reservation.source,
        &["config", "--unset", "filter.external.clean"],
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = fixture.reservation.source.join(".git/hooks/pre-commit");
        std::fs::write(
            &hook,
            format!("#!/bin/sh\ntouch '{}'\nexit 1\n", sentinel.display()),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &stamp,
        "Fix",
        &fixture.runner,
    )
    .unwrap();
    assert!(!sentinel.exists());
}

#[test]
fn publishes_exact_validated_commit_to_original_branch_using_atomic_lease() {
    let fixture = Fixture::new(true);
    let (committed, stamp) = fixture.commit();
    let result = publish(
        &fixture.reservation,
        &committed.head_sha,
        &stamp,
        &fixture.runner,
    )
    .unwrap();
    assert_eq!(result.head_sha, committed.head_sha);
    assert_eq!(
        local_git(&fixture.runner.remote, &["rev-parse", "refs/heads/feature"]),
        committed.head_sha
    );
    assert_eq!(fixture.runner.pushes.lock().unwrap().len(), 1);
}

#[test]
fn fork_and_stale_remote_are_refused_without_push() {
    let mut fixture = Fixture::new(true);
    let (committed, stamp) = fixture.commit();
    fixture.runner.fork = true;
    assert!(publish(
        &fixture.reservation,
        &committed.head_sha,
        &stamp,
        &fixture.runner
    )
    .unwrap_err()
    .starts_with("automation_publish_fork_unsupported"));
    fixture.runner.fork = false;
    let parent = local_git(
        &fixture.runner.remote,
        &["rev-parse", "refs/heads/feature^"],
    );
    local_git(
        &fixture.runner.remote,
        &["update-ref", "refs/heads/feature", &parent],
    );
    assert!(publish(
        &fixture.reservation,
        &committed.head_sha,
        &stamp,
        &fixture.runner
    )
    .unwrap_err()
    .starts_with("automation_publish_remote_changed"));
    assert!(fixture.runner.pushes.lock().unwrap().is_empty());
}

#[test]
fn atomic_lease_rejects_remote_rewind_after_precheck() {
    let mut fixture = Fixture::new(true);
    let (committed, stamp) = fixture.commit();
    fixture.runner.race_rewind = true;
    assert!(publish(
        &fixture.reservation,
        &committed.head_sha,
        &stamp,
        &fixture.runner
    )
    .is_err());
    assert_eq!(fixture.runner.pushes.lock().unwrap().len(), 1);
    let parent = local_git(
        &fixture.reservation.path,
        &[
            "rev-parse",
            &format!("{}^", fixture.reservation.starting_sha),
        ],
    );
    assert_eq!(
        local_git(&fixture.runner.remote, &["rev-parse", "refs/heads/feature"]),
        parent
    );
}

// Keep fixture execution independent of CI token variables without changing global process state.
fn publish(
    reservation: &Reservation,
    head: &str,
    stamp: &str,
    runner: &dyn CommandRunner<Command>,
) -> Result<PublishResult, String> {
    super::publish_inner(reservation, head, stamp, runner, false)
}

#[test]
fn environment_token_auth_cannot_bypass_saved_cli_identity() {
    let fixture = Fixture::new(true);
    let (committed, stamp) = fixture.commit();
    assert!(super::publish_inner(
        &fixture.reservation,
        &committed.head_sha,
        &stamp,
        &fixture.runner,
        true
    )
    .unwrap_err()
    .starts_with("automation_publish_token_auth_unsupported"));
    assert!(fixture.runner.pushes.lock().unwrap().is_empty());
}

#[test]
fn branch_intervention_during_commit_cannot_change_personal_or_reserved_branch() {
    let mut fixture = Fixture::new(false);
    fixture.edit();
    let stamp = source_fingerprint(&fixture.reservation, &fixture.runner).unwrap();
    let git_dir = local_git(
        &fixture.reservation.path,
        &["rev-parse", "--absolute-git-dir"],
    );
    let original_index = std::fs::read(Path::new(&git_dir).join("index")).unwrap();
    fixture.runner.switch_head_at_commit = true;
    assert!(commit(
        &fixture.reservation,
        &fixture.reservation.starting_sha,
        &stamp,
        "Fix",
        &fixture.runner
    )
    .is_err());
    assert_eq!(
        local_git(
            &fixture.reservation.path,
            &["symbolic-ref", "--short", "HEAD"]
        ),
        "personal"
    );
    assert_eq!(
        local_git(
            &fixture.reservation.path,
            &["rev-parse", "refs/heads/personal"]
        ),
        fixture.reservation.starting_sha
    );
    assert_eq!(
        local_git(
            &fixture.reservation.path,
            &[
                "rev-parse",
                &format!("refs/heads/{}", fixture.reservation.branch)
            ]
        ),
        fixture.reservation.starting_sha
    );
    assert_eq!(
        std::fs::read(Path::new(&git_dir).join("index")).unwrap(),
        original_index
    );
    assert_eq!(
        std::fs::read_to_string(fixture.reservation.path.join("file.txt")).unwrap(),
        "fixed\n"
    );
}

#[test]
fn fixture_commands_ignore_parent_git_configuration_without_weakening_local_guards() {
    let fixture = Fixture::new(false);
    let config = fixture.root.join("parent-gitconfig");
    std::fs::write(
        &config,
        "[filter \"ci-global\"]\n\tclean = false\n\tprocess = false\n[url \"https://unrelated.invalid/\"]\n\tinsteadOf = https://github.com/\n",
    )
    .unwrap();
    for test in [
        "automations::publication::tests::publishes_exact_validated_commit_to_original_branch_using_atomic_lease",
        "automations::publication::tests::clean_filter_is_refused_and_hooks_are_disabled",
    ] {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_CONFIG_SYSTEM", &config)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "filter.ci-environment.clean")
            .env("GIT_CONFIG_VALUE_0", "false")
            .output()
            .unwrap();
        assert!(
            child.status.success() && String::from_utf8_lossy(&child.stdout).contains(test),
            "fixture failed under parent Git config:\n{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
    }
}
