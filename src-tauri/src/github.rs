//! GitHub CLI integration owns PR discovery, caching, and opening at the network/process boundary.
//! A multi-repository workspace has one PR per repository, each on the shared branch with its own
//! history.

use crate::domain::Pr;
use crate::lock::lock;
use crate::state::{publish, Repo, Workspace};
use crate::{i18n, AppState};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, State};

#[derive(Default)]
struct ScanProgress {
    running: bool,
    pending: bool,
}

#[derive(Default)]
struct ScanGate(Mutex<ScanProgress>);

impl ScanGate {
    fn request(&self, mut scan: impl FnMut()) {
        {
            let mut progress = lock(&self.0);
            if progress.running {
                progress.pending = true;
                return;
            }
            progress.running = true;
        }
        loop {
            scan();
            let mut progress = lock(&self.0);
            if !progress.pending {
                progress.running = false;
                return;
            }
            progress.pending = false;
        }
    }
}

fn scan_gate() -> &'static ScanGate {
    static GATE: OnceLock<ScanGate> = OnceLock::new();
    GATE.get_or_init(ScanGate::default)
}

/// Refresh each repository's PR when opening the workspace; periodic refresh remains a fallback.
#[tauri::command(async)]
pub fn pr_open(app: AppHandle, state: State<AppState>, id: String) {
    let generation = lock(&state.telemetry).generation;
    let found: Vec<(String, Option<Pr>)> = repos_of(&state, &id)
        .iter()
        .map(|repo| {
            let worktree = Path::new(&repo.worktree);
            let pr = head_branch(worktree).and_then(|branch| pr_for_branch(worktree, &branch));
            (repo.name.clone(), pr)
        })
        .collect();
    remember(&app, &state, &id, found, generation);
}

/// Query gh once per clone, covering all its workspaces. Network failures and incomplete responses
/// must preserve the last known board state.
#[tauri::command(async)]
pub fn refresh_prs(app: AppHandle, state: State<AppState>) {
    scan_gate().request(|| scan_prs(&app, &state));
}

fn scan_prs(app: &AppHandle, state: &State<AppState>) {
    let generation = lock(&state.telemetry).generation;
    let alive: Vec<Workspace> = lock(&state.board)
        .workspaces
        .iter()
        .filter(|workspace| scannable(workspace.cleaned, workspace.archived, &workspace.branch))
        .cloned()
        .collect();

    // Group clone paths with workspace IDs, repository names, and live worktree branches. Read the
    // current branch because a persisted workspace name can become stale after branch renaming or
    // switching.
    let mut by_clone: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for workspace in &alive {
        for repo in &workspace.repos {
            let branch =
                head_branch(Path::new(&repo.worktree)).unwrap_or_else(|| workspace.branch.clone());
            by_clone.entry(repo.path.clone()).or_default().push((
                workspace.id.clone(),
                repo.name.clone(),
                branch,
            ));
        }
    }

    let mut found: Vec<(String, String, Option<Pr>)> = Vec::new();
    for (clone, list) in by_clone {
        let Ok(prs) = list_repo(Path::new(&clone)) else {
            continue;
        };
        for (id, name, branch) in list {
            found.push((id, name, pick(&prs, &branch)));
        }
    }

    let mut moved = false;
    let mut associated = Vec::new();
    {
        let mut board = lock(&state.board);
        for (id, name, pr) in found {
            let Some(workspace) = board.workspace_mut(&id) else {
                continue;
            };
            let Some(repo) = workspace.repos.iter_mut().find(|repo| repo.name == name) else {
                continue;
            };
            if let Some(pr) = &pr {
                if repo.pr.as_ref().is_none_or(|old| old.number != pr.number) {
                    associated.push((id.clone(), repo.path.clone(), pr.number));
                }
            }
            moved |= write(repo, pr);
        }
    }
    if moved {
        publish(app);
        for (workspace, path, number) in associated {
            crate::telemetry::associate(app, generation, &workspace, &[(path, number)]);
        }
    }
}

fn scannable(cleaned: bool, archived: bool, branch: &str) -> bool {
    !cleaned && !archived && !branch.is_empty()
}

/// Open the requested repository's PR, or the primary PR. Let gh discover and open the URL without
/// sending it over IPC.
#[tauri::command(async)]
pub fn open_pr(state: State<AppState>, id: String, repo: String) -> Result<(), String> {
    let workspace =
        workspace_copy(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let repo = workspace
        .repos
        .iter()
        .find(|candidate| candidate.name == repo)
        .cloned()
        .unwrap_or_else(|| workspace.primary());
    // Prefer the persisted PR number, including after worktree cleanup when gh runs from the clone.
    // Otherwise resolve the worktree's current branch.
    let (dir, what) = match (workspace.cleaned, repo.pr.as_ref()) {
        (false, None) => (
            repo.worktree.clone(),
            head_branch(Path::new(&repo.worktree)).ok_or_else(|| i18n::t("err.session.noPr"))?,
        ),
        (false, Some(pr)) => (repo.worktree.clone(), pr.number.to_string()),
        (true, Some(pr)) => (repo.path.clone(), pr.number.to_string()),
        (true, None) => return Err(i18n::t("err.session.noPr")),
    };
    let ok = Command::new("gh")
        .current_dir(&dir)
        .args(["pr", "view", what.as_str(), "--web"])
        .status()
        .map_err(i18n::io)?
        .success();
    ok.then_some(()).ok_or_else(|| i18n::t("err.session.noPr"))
}

pub(crate) fn pr_for_branch(worktree: &Path, branch: &str) -> Option<Pr> {
    pick(
        &list(worktree, &["--head", branch, "--limit", "5"]).ok()?,
        branch,
    )
}

/// Prefer an open PR over a closed one for the same branch, then retain the newest in gh response
/// order.
pub(crate) fn pick(prs: &[Pr], branch: &str) -> Option<Pr> {
    let mine = || prs.iter().filter(|pr| pr.head_ref_name == branch);
    mine()
        .find(|pr| pr.open())
        .or_else(|| mine().next())
        .cloned()
}

fn list_repo(repo: &Path) -> Result<Vec<Pr>, ()> {
    list(repo, &["--limit", "60"])
}

fn list(dir: &Path, extra: &[&str]) -> Result<Vec<Pr>, ()> {
    let mut command = Command::new("gh");
    command
        .current_dir(dir)
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "number,title,isDraft,state,headRefName",
        ])
        .args(extra);
    let bytes = bounded_output(command, Duration::from_secs(15)).ok_or(())?;
    serde_json::from_slice::<Vec<Pr>>(&bytes).map_err(|_| ())
}

/// A slow or abandoned CLI cannot hold the general scan indefinitely. The process group also
/// releases a descendant that inherited stdout, and the reader drains output while gh runs.
fn bounded_output(mut command: Command, timeout: Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::sync::mpsc;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let pid = child.id() as i32;
    let mut stdout = child.stdout.take()?;
    let (data_tx, data_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes);
        let _ = data_tx.send(result.map(|_| bytes));
    });
    let (exit_tx, exit_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = exit_tx.send(child.wait());
    });
    let status = exit_rx.recv_timeout(timeout);
    if matches!(&status, Ok(Ok(exit)) if exit.success()) {
        if let Ok(Ok(bytes)) = data_rx.recv_timeout(Duration::from_millis(200)) {
            if bytes.len() <= 2 * 1024 * 1024 {
                return Some(bytes);
            }
        }
    }
    // The direct process may have exited while a descendant still holds the output pipe.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    if status.is_err() {
        let _ = exit_rx.recv_timeout(Duration::from_secs(2));
    }
    None
}

/// Persist gh results for each repository on the board.
fn remember(
    app: &AppHandle,
    state: &State<AppState>,
    id: &str,
    found: Vec<(String, Option<Pr>)>,
    generation: u64,
) {
    let mut moved = false;
    let mut associated = Vec::new();
    {
        let mut board = lock(&state.board);
        let Some(workspace) = board.workspace_mut(id) else {
            return;
        };
        for (name, pr) in found {
            let Some(repo) = workspace.repos.iter_mut().find(|repo| repo.name == name) else {
                continue;
            };
            if let Some(pr) = &pr {
                if repo.pr.as_ref().is_none_or(|old| old.number != pr.number) {
                    associated.push((repo.path.clone(), pr.number));
                }
            }
            moved |= write(repo, pr);
        }
    }
    if moved {
        publish(app);
        crate::telemetry::associate(app, generation, id, &associated);
    }
}

/// Update PR metadata only when a matching response exists. Empty or truncated results can reflect
/// network, authentication, or pagination limits and must not erase known PRs.
fn write(repo: &mut Repo, pr: Option<Pr>) -> bool {
    if pr.is_none() && repo.pr.is_some() {
        return false;
    }
    if same(repo.pr.as_ref(), pr.as_ref()) {
        return false;
    }
    repo.pr = pr;
    true
}

fn same(left: Option<&Pr>, right: Option<&Pr>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.number == right.number
                && left.state == right.state
                && left.is_draft == right.is_draft
        }
        _ => false,
    }
}

fn head_branch(repo: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(repo)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !branch.is_empty() && branch != "HEAD").then_some(branch)
}

fn workspace_copy(state: &State<AppState>, id: &str) -> Option<Workspace> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id)
        .cloned()
}

/// Return remaining workspace repositories in saved order, primary first. Cleaned workspaces return
/// an empty list.
fn repos_of(state: &State<AppState>, id: &str) -> Vec<Repo> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|workspace| workspace.id == id && !workspace.cleaned)
        .map(|workspace| workspace.repos.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{bounded_output, pick, scannable, write, ScanGate};
    use crate::domain::Pr;
    use crate::state::Repo;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    #[test]
    fn a_slow_general_scan_queues_only_one_follow_up() {
        let gate = Arc::new(ScanGate::default());
        let scans = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = {
            let gate = gate.clone();
            let scans = scans.clone();
            std::thread::spawn(move || {
                gate.request(|| {
                    let scan = scans.fetch_add(1, Ordering::SeqCst);
                    if scan == 0 {
                        entered_tx.send(()).unwrap();
                        release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    }
                })
            })
        };
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        gate.request(|| panic!("overlapping request must not start a scan"));
        gate.request(|| panic!("a second overlap must not start another scan"));
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(scans.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn archived_cleaned_and_branchless_workspaces_are_excluded() {
        assert!(scannable(false, false, "feature"));
        assert!(!scannable(true, false, "feature"));
        assert!(!scannable(false, true, "feature"));
        assert!(!scannable(false, false, ""));
    }

    #[test]
    fn general_cli_read_has_a_deadline_and_requires_success() {
        let mut slow = std::process::Command::new("sh");
        slow.args(["-c", "sleep 5"]);
        let started = std::time::Instant::now();
        assert!(bounded_output(slow, Duration::from_millis(40)).is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
        let mut failed = std::process::Command::new("sh");
        failed.args(["-c", "printf '[]'; exit 1"]);
        assert!(bounded_output(failed, Duration::from_secs(2)).is_none());
    }

    fn pr(number: u64, branch: &str, state: &str) -> Pr {
        Pr {
            number,
            title: format!("PR {number}"),
            is_draft: false,
            state: state.into(),
            head_ref_name: branch.into(),
        }
    }

    #[test]
    fn open_pull_requests_take_precedence_over_closed_ones_on_the_same_branch() {
        let all = vec![
            pr(9, "outra/coisa", "OPEN"),
            pr(8, "meu/ajuste", "CLOSED"),
            pr(7, "meu/ajuste", "OPEN"),
        ];
        assert_eq!(pick(&all, "meu/ajuste").unwrap().number, 7);
        assert!(pick(&all, "does/not/exist").is_none());
    }

    #[test]
    fn missing_results_do_not_remove_known_pull_requests() {
        let mut repo = Repo {
            path: "/clone".into(),
            name: "repo".into(),
            worktree: "/worktree".into(),
            base: "origin/main".into(),
            pr: Some(pr(7, "meu/ajuste", "OPEN")),
        };
        assert!(!write(&mut repo, None));
        assert_eq!(repo.pr.as_ref().unwrap().number, 7);
        assert!(write(&mut repo, Some(pr(7, "meu/ajuste", "MERGED"))));
        assert_eq!(repo.pr.as_ref().unwrap().state, "MERGED");
        assert!(!write(&mut repo, Some(pr(7, "meu/ajuste", "MERGED"))));
    }

    #[test]
    fn keeps_the_newest_pull_request_when_none_are_open() {
        let all = vec![
            pr(12, "meu/ajuste", "MERGED"),
            pr(4, "meu/ajuste", "CLOSED"),
        ];
        let got = pick(&all, "meu/ajuste").unwrap();
        assert_eq!(got.number, 12);
        assert!(got.merged());
    }
}

pub struct TaskSnapshot {
    pub prs: BTreeMap<String, u64>,
    pub events: BTreeMap<String, String>,
    pub closed: bool,
}

/// The monitor uses read-only gh queries with pagination for comments, reviews, and inline
/// comments. No model calls are involved.
pub fn task_snapshot(ws: &Workspace, run: &crate::actions::Run) -> Result<TaskSnapshot, String> {
    let mut snapshot = TaskSnapshot {
        prs: run.prs.clone(),
        events: BTreeMap::new(),
        closed: true,
    };
    let watch = run
        .profile
        .watch
        .as_ref()
        .ok_or_else(|| i18n::t("err.actions.invalid"))?;
    let mut viewer = None;
    for repo in &ws.repos {
        let dir = Path::new(&repo.worktree);
        let what = run
            .prs
            .get(&repo.name)
            .map(u64::to_string)
            .or_else(|| head_branch(dir))
            .ok_or_else(|| i18n::t("err.session.noPr"))?;
        // A missing PR means keep waiting. Authentication and network errors remain visible.
        if !run.prs.contains_key(&repo.name) {
            let all = task_gh(
                dir,
                &[
                    "pr", "list", "--head", &what, "--state", "open", "--json", "number",
                    "--limit", "1",
                ],
            )?;
            if all.as_array().is_some_and(Vec::is_empty) {
                continue;
            }
        }
        let pr = task_gh(
            dir,
            &[
                "pr",
                "view",
                &what,
                "--json",
                "number,state,headRefOid,url,statusCheckRollup",
            ],
        )?;
        let number = pr["number"]
            .as_u64()
            .ok_or_else(|| i18n::t("err.actions.response"))?;
        snapshot.prs.insert(repo.name.clone(), number);
        if matches!(pr["state"].as_str(), Some("CLOSED" | "MERGED")) {
            continue;
        }
        if pr["state"] != "OPEN" {
            return Err(i18n::t("err.actions.response"));
        }
        snapshot.closed = false;
        let prefix = format!("{}:{number}", repo.name);
        if watch.comments {
            let login = match &viewer {
                Some(login) => login,
                None => {
                    viewer = Some(
                        task_gh(dir, &["api", "user"])?["login"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or_else(|| i18n::t("err.actions.response"))?
                            .to_string(),
                    );
                    viewer.as_ref().unwrap()
                }
            };
            for (kind, path) in [
                (
                    "comment",
                    format!("repos/{{owner}}/{{repo}}/issues/{number}/comments?per_page=100"),
                ),
                (
                    "review",
                    format!("repos/{{owner}}/{{repo}}/pulls/{number}/reviews?per_page=100"),
                ),
                (
                    "inline",
                    format!("repos/{{owner}}/{{repo}}/pulls/{number}/comments?per_page=100"),
                ),
            ] {
                let pages = task_gh(dir, &["api", "--paginate", "--slurp", &path])?;
                let pages = pages
                    .as_array()
                    .ok_or_else(|| i18n::t("err.actions.response"))?;
                for page in pages {
                    for comment in page
                        .as_array()
                        .ok_or_else(|| i18n::t("err.actions.response"))?
                    {
                        if let Some((id, text)) = task_comment(comment, login) {
                            snapshot
                                .events
                                .insert(format!("{prefix}:{kind}:{id}"), text);
                        }
                    }
                }
            }
        }
        if watch.ci {
            let sha = pr["headRefOid"]
                .as_str()
                .ok_or_else(|| i18n::t("err.actions.response"))?;
            if let Some(checks) = pr["statusCheckRollup"].as_array() {
                for check in checks {
                    if let Some((name, result)) = task_check(check) {
                        snapshot.events.insert(
                            format!("{prefix}:ci:{sha}:{name}"),
                            format!(
                                "{}\n{sha}\n{name}: {result}",
                                pr["url"].as_str().unwrap_or("")
                            ),
                        );
                    }
                }
            }
        }
    }
    snapshot.closed &= !snapshot.prs.is_empty();
    Ok(snapshot)
}

fn task_comment(value: &serde_json::Value, viewer: &str) -> Option<(u64, String)> {
    let author = value["user"]["login"].as_str()?;
    if author == viewer {
        return None;
    }
    let body = value["body"].as_str().unwrap_or("");
    let state = value["state"].as_str().unwrap_or("");
    if body.is_empty() && state != "CHANGES_REQUESTED" {
        return None;
    }
    let body: String = body.chars().take(12_000).collect();
    Some((
        value["id"].as_u64()?,
        format!(
            "{}\n{}\n{author} {state}\n{body}",
            value["html_url"].as_str().unwrap_or(""),
            value["updated_at"]
                .as_str()
                .or(value["submitted_at"].as_str())
                .unwrap_or("")
        ),
    ))
}

fn task_check(value: &serde_json::Value) -> Option<(String, String)> {
    let result = value["conclusion"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| value["state"].as_str())?;
    if !matches!(
        result.to_ascii_uppercase().as_str(),
        "SUCCESS"
            | "FAILURE"
            | "ERROR"
            | "TIMED_OUT"
            | "CANCELLED"
            | "ACTION_REQUIRED"
            | "STARTUP_FAILURE"
    ) {
        return None;
    }
    let name = value["name"]
        .as_str()
        .or_else(|| value["context"].as_str())?;
    let url = value["detailsUrl"]
        .as_str()
        .or_else(|| value["targetUrl"].as_str())
        .unwrap_or("");
    Some((format!("{name} {url}"), result.to_string()))
}

/// Bound gh execution time and drain both output pipes so large responses cannot deadlock the
/// process before wait.
fn task_gh(dir: &Path, args: &[&str]) -> Result<serde_json::Value, String> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = Command::new("gh")
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(i18n::io)?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let read = |pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let output = read(Box::new(stdout));
    let errors = read(Box::new(stderr));
    let until = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(i18n::t("err.actions.timeout"));
            }
        }
    };
    let bytes = output
        .join()
        .map_err(|_| i18n::t("err.actions.response"))?
        .map_err(i18n::io)?;
    let errors = errors
        .join()
        .map_err(|_| i18n::t("err.actions.response"))?
        .map_err(i18n::io)?;
    if !status.success() {
        return Err(i18n::io(String::from_utf8_lossy(&errors)));
    }
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(i18n::t("err.actions.response"));
    }
    serde_json::from_slice(&bytes).map_err(i18n::io)
}

#[cfg(test)]
mod task_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn ignores_own_comments_and_incomplete_checks() {
        let comment = json!({ "id": 1, "user": { "login": "owner" }, "body": "done", "html_url": "https://example.test/1", "updated_at": "now" });
        assert!(task_comment(&comment, "owner").is_none());
        assert!(task_comment(&comment, "reviewer")
            .unwrap()
            .1
            .contains("done"));
        assert!(
            task_check(&json!({"name":"test", "conclusion":"", "status":"IN_PROGRESS"})).is_none()
        );
        assert_eq!(
            task_check(&json!({"name":"test", "conclusion":"FAILURE", "detailsUrl":"run/2"}))
                .unwrap()
                .1,
            "FAILURE"
        );
        assert!(task_check(&json!({"context":"lint", "state":"PENDING"})).is_none());
        assert_eq!(
            task_check(&json!({"context":"lint", "state":"SUCCESS"}))
                .unwrap()
                .1,
            "SUCCESS"
        );
    }
}
