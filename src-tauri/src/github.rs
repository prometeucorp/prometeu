//! GitHub CLI integration owns PR discovery, caching, and opening at the network/process boundary.
//! A multi-repository workspace has one PR per repository, each on the shared branch with its own
//! history.

use crate::domain::Pr;
use crate::lock::lock;
use crate::state::{publish, Repo, Workspace};
use crate::{i18n, AppState};
use prometeu_core::command::{CommandError, CommandPolicy, CommandRunner, OutputPolicy};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, State};

/// Lifecycle evidence stays at the discovery boundary; the board's PR presentation is unchanged.
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrObservation {
    #[serde(flatten)]
    pub pr: Pr,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    #[serde(skip)]
    pub observed_after: u64,
}

/// GitHub's lifecycle fields use whole-second UTC. Validate that bounded wire shape before the
/// shared conversion so malformed dates cannot become plausible tenure evidence.
pub(crate) fn lifecycle_timestamp(value: Option<&str>) -> Option<u64> {
    let value = value?;
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || !bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        })
    {
        return None;
    }
    let year = value[0..4].parse::<u32>().ok()?;
    let month = value[5..7].parse::<usize>().ok()?;
    let day = value[8..10].parse::<u32>().ok()?;
    let hour = value[11..13].parse::<u32>().ok()?;
    let minute = value[14..16].parse::<u32>().ok()?;
    let second = value[17..19].parse::<u32>().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > days[month - 1]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    crate::usage::rfc3339(value)?.checked_mul(1000)
}

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
    let mut histories = Vec::new();
    let workspace = workspace_copy(&state, &id);
    let found: Vec<(String, Option<Pr>)> = repos_of(&state, &id)
        .iter()
        .map(|repo| {
            let worktree = Path::new(&repo.worktree);
            if let Some(url) = workspace
                .as_ref()
                .and_then(|workspace| linked_pr_url(workspace, repo))
            {
                return (
                    repo.name.clone(),
                    view(worktree, url).ok().map(|observation| observation.pr),
                );
            }
            let pr = head_branch(worktree).and_then(|branch| {
                let observations = list(worktree, &["--head", &branch, "--limit", "100"]).ok()?;
                let selected = pick_observed(&observations, &branch);
                let complete = observations.len() < 100;
                histories.push((repo.path.clone(), branch, observations, complete));
                selected
            });
            (repo.name.clone(), pr)
        })
        .collect();
    remember(&app, &state, &id, found, generation);
    for (path, branch, observations, complete) in histories {
        crate::telemetry::associate_history(
            &app,
            generation,
            &id,
            &path,
            &branch,
            &observations,
            complete,
        );
    }
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
    let mut found: Vec<(String, String, Option<Pr>)> = Vec::new();
    for workspace in &alive {
        for repo in &workspace.repos {
            if let Some(url) = linked_pr_url(workspace, repo) {
                found.push((
                    workspace.id.clone(),
                    repo.name.clone(),
                    view(Path::new(&repo.path), url)
                        .ok()
                        .map(|observation| observation.pr),
                ));
                continue;
            }
            let branch =
                head_branch(Path::new(&repo.worktree)).unwrap_or_else(|| workspace.branch.clone());
            by_clone.entry(repo.path.clone()).or_default().push((
                workspace.id.clone(),
                repo.name.clone(),
                branch,
            ));
        }
    }

    let mut histories = Vec::new();
    for (clone, list) in by_clone {
        let Ok(prs) = list_repo(Path::new(&clone)) else {
            continue;
        };
        for (id, name, branch) in list {
            let (observations, complete) = if prs.len() < 60 {
                (prs.clone(), true)
            } else {
                match self::list(Path::new(&clone), &["--head", &branch, "--limit", "100"]) {
                    Ok(observations) => {
                        let complete = observations.len() < 100;
                        (observations, complete)
                    }
                    Err(_) => (prs.clone(), false),
                }
            };
            found.push((id.clone(), name, pick_observed(&observations, &branch)));
            histories.push((id, clone.clone(), branch, observations, complete));
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
    for (workspace, path, branch, observations, complete) in histories {
        crate::telemetry::associate_history(
            app,
            generation,
            &workspace,
            &path,
            &branch,
            &observations,
            complete,
        );
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
    if let Some(url) = linked_pr_url(&workspace, &repo) {
        return crate::oauth::browse(url).map_err(i18n::io);
    }
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
    pick_observed(
        &list(worktree, &["--head", branch, "--limit", "5"]).ok()?,
        branch,
    )
}
fn pick_observed(prs: &[PrObservation], branch: &str) -> Option<Pr> {
    pick(
        &prs.iter()
            .map(|observation| observation.pr.clone())
            .collect::<Vec<_>>(),
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

/// The primary repository retains the originating PR identity even on an isolated review branch.
fn linked_pr_url<'a>(workspace: &'a Workspace, repo: &Repo) -> Option<&'a str> {
    if repo.path != workspace.repo {
        return None;
    }
    pull_url(workspace.issue.as_ref().map(|issue| issue.url.as_str()))
}

fn pull_url(url: Option<&str>) -> Option<&str> {
    let url = url?;
    crate::github_issues::target(url)
        .filter(|(_, kind, _)| kind == "pull")
        .map(|_| url)
}

fn view(dir: &Path, url: &str) -> Result<PrObservation, ()> {
    let observed_after = crate::conversation::now();
    let mut command = Command::new("gh");
    command
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .args([
            "pr",
            "view",
            url,
            "--json",
            "number,title,isDraft,state,headRefName,createdAt,closedAt,mergedAt",
        ]);
    let bytes = bounded_output(command, Duration::from_secs(15)).ok_or(())?;
    decode_view(&bytes, url, observed_after)
}

fn decode_view(bytes: &[u8], url: &str, observed_after: u64) -> Result<PrObservation, ()> {
    let mut observation: PrObservation = serde_json::from_slice(bytes).map_err(|_| ())?;
    let (_, _, number) = crate::github_issues::target(url).ok_or(())?;
    if observation.pr.number != number {
        return Err(());
    }
    observation.observed_after = observed_after;
    Ok(observation)
}

fn list_repo(repo: &Path) -> Result<Vec<PrObservation>, ()> {
    list(repo, &["--limit", "60"])
}

fn list(dir: &Path, extra: &[&str]) -> Result<Vec<PrObservation>, ()> {
    let observed_after = crate::conversation::now();
    let mut command = Command::new("gh");
    command
        .current_dir(dir)
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "number,title,isDraft,state,headRefName,createdAt,closedAt,mergedAt",
        ])
        .args(extra);
    let bytes = bounded_output(command, Duration::from_secs(15)).ok_or(())?;
    let mut observations = serde_json::from_slice::<Vec<PrObservation>>(&bytes).map_err(|_| ())?;
    for observation in &mut observations {
        observation.observed_after = observed_after;
    }
    Ok(observations)
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

pub(crate) fn head_branch(repo: &Path) -> Option<String> {
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
    use super::{bounded_output, lifecycle_timestamp, pick, scannable, write, ScanGate};
    use crate::domain::Pr;
    use crate::state::Repo;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    #[test]
    fn only_canonical_pull_urls_override_branch_discovery() {
        assert_eq!(
            super::pull_url(Some("https://github.com/org/.github/pull/42")),
            Some("https://github.com/org/.github/pull/42")
        );
        for url in [
            "https://github.com/org/repo/issues/42",
            "https://github.com.evil/org/repo/pull/42",
            "https://github.com/org/repo/pull/42?x=y",
            "https://linear.app/team/issue/42",
        ] {
            assert!(super::pull_url(Some(url)).is_none());
        }
        assert!(super::pull_url(None).is_none());
    }

    #[test]
    fn explicit_pull_identity_does_not_depend_on_the_local_review_branch() {
        let payload = br#"{"number":42,"title":"Fork fix","state":"OPEN","headRefName":"main"}"#;
        let observation =
            super::decode_view(payload, "https://github.com/org/repo/pull/42", 123).unwrap();
        assert_eq!(observation.pr.number, 42);
        assert_eq!(observation.pr.head_ref_name, "main");
        assert_eq!(observation.observed_after, 123);
        assert!(super::decode_view(payload, "https://github.com/org/repo/pull/43", 123).is_err());
    }

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
    fn lifecycle_timestamps_require_valid_calendar_evidence() {
        assert_eq!(
            lifecycle_timestamp(Some("2024-02-29T00:00:00Z")),
            Some(1709164800000)
        );
        for value in [
            "2023-02-29T00:00:00Z",
            "2024-02-30T00:00:00Z",
            "2024-00-10T00:00:00Z",
            "2024-13-10T00:00:00Z",
            "2024-01-00T00:00:00Z",
            "2024-01-01T24:00:00Z",
            "2024-01-01T00:60:00Z",
            "2024-01-01T00:00:60Z",
            "999999999999999999999-01-01T00:00:00Z",
            "PRIVATE",
        ] {
            assert_eq!(lifecycle_timestamp(Some(value)), None, "{value}");
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
pub fn task_snapshot(
    runner: &dyn CommandRunner<Command>,
    ws: &Workspace,
    run: &crate::actions::Run,
) -> Result<TaskSnapshot, String> {
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
        let linked = linked_pr_url(ws, repo);
        let repository = linked.and_then(crate::github_issues::target);
        let what = linked
            .map(str::to_owned)
            .or_else(|| run.prs.get(&repo.name).map(u64::to_string))
            .or_else(|| head_branch(dir))
            .ok_or_else(|| i18n::t("err.session.noPr"))?;
        // Linked PRs retain their explicit identity on synthetic review branches.
        // A missing ordinary PR means keep waiting; transport failures remain visible.
        if linked.is_none() && !run.prs.contains_key(&repo.name) {
            let all = task_gh(
                runner,
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
            runner,
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
        if repository
            .as_ref()
            .is_some_and(|(_, _, expected)| *expected != number)
        {
            return Err(i18n::t("err.actions.response"));
        }
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
                    let args: &[&str] = if linked.is_some() {
                        &["api", "--hostname", "github.com", "user"]
                    } else {
                        &["api", "user"]
                    };
                    viewer = Some(
                        task_gh(runner, dir, args)?["login"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or_else(|| i18n::t("err.actions.response"))?
                            .to_string(),
                    );
                    viewer.as_ref().unwrap()
                }
            };
            let repository = repository
                .as_ref()
                .map(|(name, _, _)| name.as_str())
                .unwrap_or("{owner}/{repo}");
            for (kind, path) in [
                (
                    "comment",
                    format!("repos/{repository}/issues/{number}/comments?per_page=100"),
                ),
                (
                    "review",
                    format!("repos/{repository}/pulls/{number}/reviews?per_page=100"),
                ),
                (
                    "inline",
                    format!("repos/{repository}/pulls/{number}/comments?per_page=100"),
                ),
            ] {
                let mut args = vec!["api", "--paginate", "--slurp", &path];
                if linked.is_some() {
                    args.extend(["--hostname", "github.com"]);
                }
                let pages = task_gh(runner, dir, &args)?;
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
fn task_gh(
    runner: &dyn CommandRunner<Command>,
    dir: &Path,
    args: &[&str],
) -> Result<serde_json::Value, String> {
    let mut command = Command::new("gh");
    command.current_dir(dir).args(args);
    let output = runner
        .run(
            &mut command,
            &[],
            CommandPolicy {
                timeout: std::time::Duration::from_secs(30),
                stdout: OutputPolicy::Capture {
                    limit: 8 * 1024 * 1024,
                },
                stderr: OutputPolicy::Capture {
                    limit: 8 * 1024 * 1024,
                },
            },
        )
        .map_err(|error| match error {
            CommandError::Timeout => i18n::t("err.actions.timeout"),
            CommandError::OutputLimit | CommandError::InvalidOutput => {
                i18n::t("err.actions.response")
            }
            CommandError::Unavailable => i18n::io("gh executable not found"),
            CommandError::Io(cause) => i18n::io(cause),
        })?;
    if !output.success {
        return Err(i18n::io(String::from_utf8_lossy(&output.stderr)));
    }
    serde_json::from_slice(&output.stdout).map_err(i18n::io)
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

#[cfg(test)]
mod command_port_tests {
    use super::*;
    use prometeu_core::command::CommandOutput;
    struct Runner(u8);
    impl CommandRunner<Command> for Runner {
        fn run(
            &self,
            request: &mut Command,
            input: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.get_program(), "gh");
            assert_eq!(request.get_current_dir(), Some(Path::new("/fixture")));
            assert!(input.is_empty());
            assert_eq!(policy.timeout, std::time::Duration::from_secs(30));
            assert_eq!(
                policy.stdout,
                OutputPolicy::Capture {
                    limit: 8 * 1024 * 1024
                }
            );
            match self.0 {
                0 => Ok(CommandOutput {
                    success: true,
                    stdout: br#"{"number":42}"#.to_vec(),
                    stderr: vec![],
                }),
                1 => Err(CommandError::Timeout),
                2 => Err(CommandError::OutputLimit),
                _ => Ok(CommandOutput {
                    success: false,
                    stdout: vec![],
                    stderr: b"private CLI error".to_vec(),
                }),
            }
        }
    }
    struct LinkedRunner(std::sync::Mutex<Vec<Vec<String>>>);
    impl CommandRunner<Command> for LinkedRunner {
        fn run(
            &self,
            request: &mut Command,
            _: &[u8],
            _: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            let args: Vec<_> = request
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            self.0.lock().unwrap().push(args.clone());
            let data = match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["pr", "view", "https://github.com/upstream/repo/pull/42", "--json", _] => {
                    serde_json::json!({"number":42,"state":"OPEN","headRefOid":"abc","url":"https://github.com/upstream/repo/pull/42","statusCheckRollup":[]})
                }
                ["api", "--hostname", "github.com", "user"] => {
                    serde_json::json!({"login":"viewer"})
                }
                ["api", "--paginate", "--slurp", path, "--hostname", "github.com"] => {
                    assert!([
                        "repos/upstream/repo/issues/42/comments?per_page=100",
                        "repos/upstream/repo/pulls/42/reviews?per_page=100",
                        "repos/upstream/repo/pulls/42/comments?per_page=100",
                    ]
                    .contains(path));
                    serde_json::json!([[]])
                }
                _ => panic!("unexpected GitHub query: {args:?}"),
            };
            Ok(CommandOutput {
                success: true,
                stdout: serde_json::to_vec(&data).unwrap(),
                stderr: vec![],
            })
        }
    }

    #[test]
    fn linked_review_monitor_uses_upstream_identity_without_branch_discovery() {
        let ws: Workspace = serde_json::from_value(serde_json::json!({
            "id":"review", "title":"Review", "repo":"/fork", "repo_name":"repo",
            "branch":"github-pr-42-isolated", "worktree":"/fixture", "stage":"work",
            "repos":[{"path":"/fork", "name":"repo", "worktree":"/fixture", "base":"upstream/main"}],
            "issue":{"id":"github:upstream/repo/pull/42", "identifier":"upstream/repo#42", "title":"Review", "url":"https://github.com/upstream/repo/pull/42"}
        })).unwrap();
        let mut catalog = crate::actions::Catalog::default();
        catalog.initialize_defaults();
        let mut run: crate::actions::Run = serde_json::from_value(serde_json::json!({
            "command":"review", "profile":catalog.profiles[0], "paused":false,
            "done":false, "turns":0, "checked_at":0, "error":null
        }))
        .unwrap();
        run.profile.watch = Some(prometeu_core::actions::Watch {
            comments: true,
            ci: true,
            interval_seconds: 30,
            max_turns: 3,
        });
        let runner = LinkedRunner(std::sync::Mutex::new(vec![]));
        let snapshot = task_snapshot(&runner, &ws, &run).unwrap();
        assert_eq!(snapshot.prs.get("repo"), Some(&42));
        assert!(!snapshot.closed);
        assert_eq!(runner.0.lock().unwrap().len(), 5);
        // A stale number from an older branch must not override the explicit linked PR.
        run.prs.insert("repo".into(), 9);
        assert_eq!(
            task_snapshot(&runner, &ws, &run).unwrap().prs.get("repo"),
            Some(&42)
        );
    }

    #[test]
    fn task_queries_keep_json_results_and_existing_error_classification() {
        assert_eq!(
            task_gh(&Runner(0), Path::new("/fixture"), &["api", "user"]).unwrap()["number"],
            42
        );
        for (mode, error) in [
            (1, i18n::t("err.actions.timeout")),
            (2, i18n::t("err.actions.response")),
            (3, i18n::io("private CLI error")),
        ] {
            assert_eq!(
                task_gh(&Runner(mode), Path::new("/fixture"), &["api", "user"]),
                Err(error)
            );
        }
    }
}
