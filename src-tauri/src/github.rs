//! PR discovery, caching and opening through the GitHub App credential (ADR 0088). A
//! multi-repository workspace has one PR per repository, each on the shared branch with its own
//! history. Repositories without the App installed, or without a connection, keep their last known
//! PR.

use crate::domain::Pr;
use crate::lock::lock;
use crate::state::{publish, Repo, Workspace};
use crate::{i18n, AppState};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
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
                    view(url).ok().map(|observation| observation.pr),
                );
            }
            let pr = head_branch(worktree).and_then(|branch| {
                let observations = list(worktree, &branch, 100).ok()?;
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

/// Query GitHub once per clone, covering all its workspaces. Network failures and incomplete responses
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
                    view(url).ok().map(|observation| observation.pr),
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
                match self::list(Path::new(&clone), &branch, 100) {
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

/// Open the requested repository's PR, or the primary PR, without sending its URL over IPC.
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
    // Prefer the persisted PR number, including after worktree cleanup. Otherwise resolve the
    // worktree's current branch.
    let dir = if workspace.cleaned {
        &repo.path
    } else {
        &repo.worktree
    };
    let (base, _) = repository(Path::new(dir)).ok_or_else(|| i18n::t("err.session.noPr"))?;
    let number = match repo.pr.as_ref() {
        Some(pr) => pr.number,
        None if !workspace.cleaned => {
            let branch = head_branch(Path::new(&repo.worktree))
                .ok_or_else(|| i18n::t("err.session.noPr"))?;
            pr_for_branch(Path::new(&repo.worktree), &branch)
                .ok_or_else(|| i18n::t("err.session.noPr"))?
                .number
        }
        None => return Err(i18n::t("err.session.noPr")),
    };
    crate::oauth::browse(&format!("https://github.com/{base}/pull/{number}")).map_err(i18n::io)
}

pub(crate) fn pr_for_branch(worktree: &Path, branch: &str) -> Option<Pr> {
    pick_observed(&list(worktree, branch, 5).ok()?, branch)
}
fn pick_observed(prs: &[PrObservation], branch: &str) -> Option<Pr> {
    pick(
        &prs.iter()
            .map(|observation| observation.pr.clone())
            .collect::<Vec<_>>(),
        branch,
    )
}

/// Prefer an open PR over a closed one for the same branch, then retain the newest in response
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

fn view(url: &str) -> Result<PrObservation, ()> {
    let observed_after = crate::conversation::now();
    let (repository, _, number) = crate::github_issues::target(url).ok_or(())?;
    let value = crate::github_auth::get(&format!("/repos/{repository}/pulls/{number}"), &[])
        .map_err(|_| ())?;
    let observation = observation(&value, observed_after).ok_or(())?;
    (observation.pr.number == number)
        .then_some(observation)
        .ok_or(())
}

/// Map a REST pull request to the board's PR, keeping lifecycle timestamps for telemetry.
fn observation(value: &serde_json::Value, observed_after: u64) -> Option<PrObservation> {
    let text = |key: &str| value[key].as_str().map(str::to_owned);
    let state = match (value["state"].as_str()?, value["merged_at"].is_string()) {
        ("open", _) => "OPEN",
        (_, true) => "MERGED",
        _ => "CLOSED",
    };
    Some(PrObservation {
        pr: Pr {
            number: value["number"].as_u64()?,
            title: text("title")?,
            is_draft: value["draft"].as_bool().unwrap_or(false),
            state: state.into(),
            head_ref_name: value["head"]["ref"].as_str()?.into(),
        },
        created_at: text("created_at"),
        closed_at: text("closed_at"),
        merged_at: text("merged_at"),
        observed_after,
    })
}

/// The base repository (upstream when present, otherwise origin) and the owner that pushes
/// branches (origin), from the clone's GitHub remotes.
fn repository(dir: &Path) -> Option<(String, String)> {
    let remote = |name: &str| {
        let output = Command::new("git")
            .current_dir(dir)
            .args(["remote", "get-url", name])
            .output()
            .ok()?;
        output.status.success().then_some(())?;
        crate::github_issues::remote_repository(&String::from_utf8_lossy(&output.stdout))
    };
    let origin = remote("origin");
    let base = remote("upstream").or_else(|| origin.clone())?;
    let owner = origin
        .as_deref()
        .unwrap_or(&base)
        .split_once('/')?
        .0
        .to_owned();
    Some((base, owner))
}

fn list_repo(repo: &Path) -> Result<Vec<PrObservation>, ()> {
    pulls(repo, None, 60)
}

fn list(dir: &Path, branch: &str, limit: u32) -> Result<Vec<PrObservation>, ()> {
    pulls(dir, Some(branch), limit)
}

fn pulls(dir: &Path, branch: Option<&str>, limit: u32) -> Result<Vec<PrObservation>, ()> {
    let observed_after = crate::conversation::now();
    let (base, owner) = repository(dir).ok_or(())?;
    let mut query = vec![
        ("state", "all".to_owned()),
        ("sort", "created".to_owned()),
        ("direction", "desc".to_owned()),
        ("per_page", limit.to_string()),
    ];
    if let Some(branch) = branch {
        query.push(("head", format!("{owner}:{branch}")));
    }
    let value = crate::github_auth::get(&format!("/repos/{base}/pulls"), &query).map_err(|_| ())?;
    let rows = value.as_array().ok_or(())?;
    Ok(rows
        .iter()
        .filter_map(|row| observation(row, observed_after))
        .collect())
}

/// Persist GitHub results for each repository on the board.
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
    use super::{lifecycle_timestamp, observation, pick, scannable, write, ScanGate};
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
    fn rest_pull_requests_map_to_board_states_and_lifecycle() {
        let open = serde_json::json!({"number":42,"title":"Fork fix","state":"open","draft":true,
            "head":{"ref":"main"},"created_at":"2026-10-01T00:00:00Z","merged_at":null});
        let found = observation(&open, 123).unwrap();
        assert_eq!(
            (found.pr.number, found.pr.state.as_str(), found.pr.is_draft),
            (42, "OPEN", true)
        );
        assert_eq!(found.pr.head_ref_name, "main");
        assert_eq!(found.observed_after, 123);
        assert_eq!(found.created_at.as_deref(), Some("2026-10-01T00:00:00Z"));
        let mut merged = open.clone();
        merged["state"] = "closed".into();
        merged["merged_at"] = "2026-10-02T00:00:00Z".into();
        assert_eq!(observation(&merged, 0).unwrap().pr.state, "MERGED");
        merged["merged_at"] = serde_json::Value::Null;
        assert_eq!(observation(&merged, 0).unwrap().pr.state, "CLOSED");
        assert!(observation(&serde_json::json!({"number":1}), 0).is_none());
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
