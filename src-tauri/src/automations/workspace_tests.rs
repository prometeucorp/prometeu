use super::*;
use prometeu_core::{
    automation::{NodeConfig, RunStatus},
    command::CommandError,
};
use std::sync::Mutex;

/// Every gh response and fetch is injected; Git commands only touch temporary local repositories.
#[derive(Default)]
struct LocalRunner {
    current_sha: Mutex<Option<String>>,
    fetched_sha: Mutex<Option<String>>,
    calls: Mutex<Vec<Vec<String>>>,
}
impl CommandRunner<Command> for LocalRunner {
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
        self.calls.lock().unwrap().push(args.clone());
        if request.get_program() == "gh" {
            let value = if args.first().map(String::as_str) == Some("api") {
                json!({"login":"owner"})
            } else {
                json!({"headRefOid":self.current_sha.lock().unwrap().clone().unwrap(),"headRefName":"feature","state":"OPEN"})
            };
            return Ok(CommandOutput {
                success: true,
                stdout: serde_json::to_vec(&value).unwrap(),
                stderr: vec![],
            });
        }
        assert_eq!(request.get_program(), "git");
        let output = if args.iter().any(|a| a == "fetch") {
            assert!(args.contains(&"https://github.com/owner/repo.git".to_owned()));
            assert!(args.contains(&"--no-write-fetch-head".to_owned()));
            let reference = args
                .last()
                .unwrap()
                .strip_prefix("refs/pull/42/head:")
                .unwrap();
            Command::new("git")
                .current_dir(request.get_current_dir().unwrap())
                .args([
                    "update-ref",
                    reference,
                    self.fetched_sha.lock().unwrap().as_deref().unwrap(),
                ])
                .output()
        } else {
            assert!(!args.iter().any(|arg| arg.contains("://")));
            request.output()
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
    source: PathBuf,
    projects: Vec<Project>,
    runner: LocalRunner,
    initial: String,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("automation-workspace-{}", uuid::Uuid::new_v4()));
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        let root = root.canonicalize().unwrap();
        let source = source.canonicalize().unwrap();
        local_git(&source, &["init", "--initial-branch=main"]);
        local_git(
            &source,
            &["config", "user.email", "automation@example.invalid"],
        );
        local_git(&source, &["config", "user.name", "Automation test"]);
        std::fs::write(source.join("file.txt"), "initial\n").unwrap();
        local_git(&source, &["add", "file.txt"]);
        local_git(
            &source,
            &["-c", "commit.gpgsign=false", "commit", "-m", "initial"],
        );
        let initial = local_git(&source, &["rev-parse", "HEAD"]);
        let projects = vec![Project {
            id: "registered-project".into(),
            name: "Example".into(),
            path: source.to_str().unwrap().into(),
        }];
        Self {
            root,
            source,
            projects,
            runner: LocalRunner::default(),
            initial,
        }
    }
    fn run(&self, event: &str) -> Run {
        let mut workflow = super::super::catalog::templates().remove(0);
        workflow.scope.targets.clear();
        workflow.scope.project_id = Some("registered-project".into());
        workflow.scope.repository = Some("owner/repo".into());
        workflow.scope.identity = Some("owner".into());
        workflow.scope.linear_project_id = Some("linear-project".into());
        for node in &mut workflow.nodes {
            if let NodeConfig::Trigger { event: trigger, .. } = &mut node.config {
                *trigger = event.into();
            }
        }
        Run {
            id: uuid::Uuid::new_v4().to_string(),
            workflow,
            status: RunStatus::Queued,
            event: json!({"identity":"owner","project":{"id":"linear-project"}}),
            event_key: "test".into(),
            resource_key: None,
            created_at: 0,
            updated_at: 0,
            history: vec![],
            approvals: vec![],
            outputs: Default::default(),
            completed_ports: Default::default(),
            measured_cost_usd: 0.0,
            cost_unknown: false,
        }
    }
    fn reservation(&self) -> Reservation {
        reserve(
            &self.root,
            &self.run("linear.assigned_issue"),
            &self.projects,
            &self.runner,
        )
        .unwrap()
    }
    fn pull_run(&self) -> Run {
        local_git(&self.source, &["checkout", "-b", "feature"]);
        std::fs::write(self.source.join("file.txt"), "pull request\n").unwrap();
        local_git(&self.source, &["add", "file.txt"]);
        local_git(
            &self.source,
            &["-c", "commit.gpgsign=false", "commit", "-m", "pull request"],
        );
        let head = local_git(&self.source, &["rev-parse", "HEAD"]);
        local_git(&self.source, &["checkout", "main"]);
        *self.runner.current_sha.lock().unwrap() = Some(head.clone());
        *self.runner.fetched_sha.lock().unwrap() = Some(head.clone());
        let mut run = self.run("github.authored_pr");
        run.event = json!({"repository":"owner/repo","identity":"owner","number":42,"headRefName":"feature","headSha":head});
        run
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn local_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "local git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn linear_reserves_committed_head_and_reuses_without_copying_source_edits() {
    let fixture = Fixture::new();
    std::fs::write(fixture.source.join("file.txt"), "uncommitted source\n").unwrap();
    let reservation = fixture.reservation();
    assert_eq!(reservation.starting_sha, fixture.initial);
    assert!(!reservation.path.exists());
    let restored: Reservation =
        serde_json::from_slice(&serde_json::to_vec(&reservation).unwrap()).unwrap();
    prepare(&restored, &fixture.runner).unwrap();
    prepare(&restored, &fixture.runner).unwrap();
    assert_eq!(
        std::fs::read_to_string(restored.path.join("file.txt")).unwrap(),
        "initial\n"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.source.join("file.txt")).unwrap(),
        "uncommitted source\n"
    );
    assert_eq!(
        fixture
            .runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|args| args.windows(2).any(|pair| pair == ["worktree", "add"]))
            .count(),
        1
    );
}

#[test]
fn github_uses_fetched_event_head_and_never_selected_clone_head() {
    let fixture = Fixture::new();
    let run = fixture.pull_run();
    let reservation = reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).unwrap();
    assert_ne!(reservation.starting_sha, fixture.initial);
    assert!(!fixture
        .runner
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|args| args.iter().any(|a| a == "fetch")));
    prepare(&reservation, &fixture.runner).unwrap();
    assert_eq!(
        std::fs::read_to_string(reservation.path.join("file.txt")).unwrap(),
        "pull request\n"
    );
    assert_eq!(
        local_git(&fixture.source, &["rev-parse", "HEAD"]),
        fixture.initial
    );
    assert_eq!(
        local_git(&reservation.path, &["symbolic-ref", "--short", "HEAD"]),
        reservation.branch
    );
}

#[test]
fn changed_current_or_fetched_pr_head_never_prepares_wrong_code() {
    let fixture = Fixture::new();
    let run = fixture.pull_run();
    let reservation = reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).unwrap();
    *fixture.runner.current_sha.lock().unwrap() = Some(fixture.initial.clone());
    assert!(prepare(&reservation, &fixture.runner)
        .unwrap_err()
        .starts_with("automation_pr_head_changed"));
    assert!(!reservation.path.exists());
    *fixture.runner.current_sha.lock().unwrap() = Some(reservation.starting_sha.clone());
    *fixture.runner.fetched_sha.lock().unwrap() = Some(fixture.initial.clone());
    assert!(prepare(&reservation, &fixture.runner)
        .unwrap_err()
        .starts_with("automation_pr_head_changed"));
    assert!(!reservation.path.exists());
}

#[test]
fn partial_branch_creation_recovers_without_reset_and_dirty_reuse_is_refused() {
    let fixture = Fixture::new();
    let reservation = fixture.reservation();
    local_git(
        &fixture.source,
        &["branch", &reservation.branch, &reservation.starting_sha],
    );
    prepare(&reservation, &fixture.runner).unwrap();
    std::fs::write(reservation.path.join("file.txt"), "user edits\n").unwrap();
    assert!(prepare(&reservation, &fixture.runner).is_err());
    assert!(verify(&reservation, &fixture.runner).is_err());
    verify_at_head(&reservation, &reservation.starting_sha, &fixture.runner).unwrap();
    assert_eq!(
        std::fs::read_to_string(reservation.path.join("file.txt")).unwrap(),
        "user edits\n"
    );
    local_git(&reservation.path, &["checkout", "-b", "user-branch"]);
    assert!(verify_at_head(&reservation, &reservation.starting_sha, &fixture.runner).is_err());
    assert_eq!(
        local_git(&reservation.path, &["symbolic-ref", "--short", "HEAD"]),
        "user-branch"
    );
}

#[test]
fn arbitrary_existing_directory_is_preserved_and_cannot_be_adopted() {
    let fixture = Fixture::new();
    let reservation = fixture.reservation();
    std::fs::create_dir_all(&reservation.path).unwrap();
    std::fs::write(reservation.path.join("keep.txt"), "keep").unwrap();
    assert!(prepare(&reservation, &fixture.runner).is_err());
    assert_eq!(
        std::fs::read_to_string(reservation.path.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn mapping_and_reservation_revalidation_prevent_wrong_project_or_pr_fallback() {
    let fixture = Fixture::new();
    let mut run = fixture.run("github.authored_pr");
    assert!(reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).is_err());
    run = fixture.run("linear.assigned_issue");
    let reservation = reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).unwrap();
    reservation
        .validate_for(&fixture.root, &run, &fixture.projects)
        .unwrap();
    run.event["project"]["id"] = json!("other-project");
    assert!(reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).is_err());
    assert!(reservation
        .validate_for(&fixture.root, &run, &fixture.projects)
        .is_err());
    run.event["project"]["id"] = json!("linear-project");
    let mut tampered = reservation.clone();
    tampered.path = fixture.source.clone();
    assert!(tampered
        .validate_for(&fixture.root, &run, &fixture.projects)
        .is_err());
    run.workflow.scope.project_id = Some("unregistered".into());
    assert!(reserve(&fixture.root, &run, &fixture.projects, &fixture.runner).is_err());
}

#[test]
fn board_registration_is_visible_idempotent_and_preserves_personal_choices() {
    let fixture = Fixture::new();
    let reservation = fixture.reservation();
    let mut board = Board {
        projects: fixture.projects.clone(),
        ..Board::default()
    };
    register_in_board(&mut board, &reservation, "Automation task", true, None).unwrap();
    assert_eq!(board.workspaces.len(), 1);
    let workspace = &mut board.workspaces[0];
    assert!(workspace.preparing && workspace.tabs.is_empty());
    assert!(!workspace.archived && !workspace.shared);
    assert_eq!(workspace.project, "registered-project");
    workspace.title = "My title".into();
    workspace.stage = "Code review".into();
    register_in_board(
        &mut board,
        &reservation,
        "Ignored title",
        false,
        Some("prepare failed".into()),
    )
    .unwrap();
    assert_eq!(board.workspaces.len(), 1);
    assert_eq!(board.workspaces[0].title, "My title");
    assert_eq!(board.workspaces[0].stage, "Code review");
    assert_eq!(
        board.workspaces[0].failed.as_deref(),
        Some("prepare failed")
    );
    register_in_board(&mut board, &reservation, "Ignored title", false, None).unwrap();
    assert!(board.workspaces[0].failed.is_none());
    board.workspaces[0].archived = true;
    assert!(register_in_board(&mut board, &reservation, "Ignored title", true, None).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_destination_is_never_followed() {
    let fixture = Fixture::new();
    let reservation = fixture.reservation();
    std::fs::create_dir_all(reservation.path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&fixture.source, &reservation.path).unwrap();
    assert!(prepare(&reservation, &fixture.runner).is_err());
    assert!(reservation
        .path
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        local_git(&fixture.source, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
}

#[test]
fn configured_checkout_filter_is_refused_without_executing_it() {
    let fixture = Fixture::new();
    let sentinel = fixture.root.join("filter-executed");
    std::fs::write(
        fixture.source.join(".gitattributes"),
        "*.txt filter=sentinel\n",
    )
    .unwrap();
    local_git(&fixture.source, &["add", ".gitattributes"]);
    local_git(
        &fixture.source,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "configure attribute",
        ],
    );
    local_git(
        &fixture.source,
        &[
            "config",
            "filter.sentinel.smudge",
            &format!("touch '{}'", sentinel.display()),
        ],
    );
    let reservation = fixture.reservation();
    assert!(prepare(&reservation, &fixture.runner)
        .unwrap_err()
        .starts_with("automation_git_filters_unsupported"));
    assert!(!sentinel.exists());
    assert!(!reservation.path.exists());
}
