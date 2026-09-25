//! Local Git operations distinguish the index from working-tree changes. They never switch
//! workspace branches; branch lifecycle remains with worktrees.

use super::{default_base, worktree_of_branch};
use crate::i18n;
use crate::lock::lock;
use crate::state::Repo;
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;

// ponytail: serialize Git mutations across the app; use per-repository locks when concurrent
// operations on separate repositories are needed.
static MUTATION: Mutex<()> = Mutex::new(());
const TEXT_LIMIT: usize = 400_000;
const STATUS_TTL: Duration = Duration::from_secs(2);
const PORCELAIN_TTL: Duration = Duration::from_secs(1);

struct SlotState<T> {
    generation: u64,
    scanned: u64,
    at: Option<Instant>,
    value: Option<Result<T, String>>,
    scanning: bool,
}

struct Slot<T> {
    state: Mutex<SlotState<T>>,
    changed: Condvar,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(SlotState {
                generation: 0,
                scanned: 0,
                at: None,
                value: None,
                scanning: false,
            }),
            changed: Condvar::new(),
        }
    }
}

impl<T: Clone> Slot<T> {
    fn invalidate(&self) {
        let mut state = lock(&self.state);
        state.generation += 1;
        state.at = None;
        self.changed.notify_all();
    }

    fn read(
        &self,
        ttl: Duration,
        mut compute: impl FnMut() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut state = lock(&self.state);
        loop {
            if state.scanned == state.generation && state.at.is_some_and(|at| at.elapsed() < ttl) {
                return state
                    .value
                    .as_ref()
                    .expect("scanned slot has a value")
                    .clone();
            }
            if state.scanning {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                continue;
            }
            state.scanning = true;
            break;
        }
        drop(state);
        loop {
            let generation = lock(&self.state).generation;
            let result = compute();
            let mut state = lock(&self.state);
            if state.generation != generation {
                // An app mutation arrived during the scan. Keep ownership of the slot and run
                // exactly one follow-up before any waiter can observe the stale result.
                drop(state);
                continue;
            }
            state.scanned = generation;
            state.at = Some(Instant::now());
            state.value = Some(result.clone());
            state.scanning = false;
            self.changed.notify_all();
            return result;
        }
    }
}

fn cache_key(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

fn raw_slots() -> &'static Mutex<HashMap<PathBuf, Arc<Slot<String>>>> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Arc<Slot<String>>>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

type StatusSlots = Mutex<HashMap<(PathBuf, String), Arc<Slot<GitStatus>>>>;

fn status_slots() -> &'static StatusSlots {
    static SLOTS: OnceLock<StatusSlots> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn raw_status(root: &Path) -> Result<String, String> {
    let key = cache_key(root);
    let slot = lock(raw_slots())
        .entry(key)
        .or_insert_with(|| Arc::new(Slot::default()))
        .clone();
    slot.read(PORCELAIN_TTL, || {
        run(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ],
        )
    })
}

fn cached_status(root: &Path, base: &str) -> Result<GitStatus, String> {
    let key = (cache_key(root), base.to_string());
    let slot = lock(status_slots())
        .entry(key)
        .or_insert_with(|| Arc::new(Slot::default()))
        .clone();
    slot.read(STATUS_TTL, || status_with(root, base, || raw_status(root)))
}

pub fn invalidate_path(path: &Path) {
    let path = cache_key(path);
    for (root, slot) in lock(raw_slots()).iter() {
        if path.starts_with(root) || root.starts_with(&path) {
            slot.invalidate();
        }
    }
    for ((root, _), slot) in lock(status_slots()).iter() {
        if path.starts_with(root) || root.starts_with(&path) {
            slot.invalidate();
        }
    }
}

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

fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.args(["--literal-pathspecs", "-C"]).arg(root);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn output(root: &Path, args: &[&str]) -> Result<Output, String> {
    command(root).args(args).output().map_err(i18n::io)
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let result = output(root, args)?;
    if !result.status.success() {
        return Err(i18n::ta(
            "err.git.command",
            &[(
                "cause",
                String::from_utf8_lossy(&result.stderr).trim().to_string(),
            )],
        ));
    }
    String::from_utf8(result.stdout).map_err(|_| i18n::t("err.session.binary"))
}

fn optional(root: &Path, args: &[&str]) -> Option<String> {
    run(root, args)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn workspace_repos(state: &State<AppState>, id: &str) -> Result<Vec<Repo>, String> {
    let board = lock(&state.board);
    let workspace = board
        .workspaces
        .iter()
        .find(|workspace| {
            workspace.id == id
                && !workspace.cleaned
                && !workspace.preparing
                && workspace.failed.is_none()
        })
        .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    Ok(workspace.repos.clone())
}

fn repository(state: &State<AppState>, id: &str, repo: usize) -> Result<Repo, String> {
    workspace_repos(state, id)?
        .get(repo)
        .cloned()
        .ok_or_else(|| i18n::t("err.session.noWorkspace"))
}

fn fingerprint(root: &Path) -> Result<String, String> {
    let mut hash = DefaultHasher::new();
    run(root, &["ls-files", "--stage", "-z"])?.hash(&mut hash);
    optional(root, &["rev-parse", "--verify", "HEAD"]).hash(&mut hash);
    Ok(format!("{:x}", hash.finish()))
}

type StatusGroups = (Vec<GitFile>, Vec<GitFile>, Vec<GitFile>);

fn parse_status(text: &str) -> Result<StatusGroups, String> {
    let (mut staged, mut changes, mut conflicts) = (Vec::new(), Vec::new(), Vec::new());
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let bytes = entry.as_bytes();
        if bytes.len() < 4 || bytes[2] != b' ' || !entry.is_char_boundary(3) {
            return Err(i18n::t("err.git.status"));
        }
        let pair = &entry[..2];
        let file = |status: &str| GitFile {
            path: entry[3..].to_string(),
            status: status.to_string(),
        };
        if pair.contains('U') || pair == "AA" || pair == "DD" {
            conflicts.push(file("U"));
        } else if pair == "??" {
            changes.push(file("?"));
        } else {
            if bytes[0] != b' ' {
                staged.push(file(&entry[..1]));
            }
            if bytes[1] != b' ' {
                changes.push(file(&entry[1..2]));
            }
        }
    }
    Ok((staged, changes, conflicts))
}

fn status(root: &Path, base: &str) -> Result<GitStatus, String> {
    status_with(root, base, || {
        run(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--no-renames",
            ],
        )
    })
}

fn status_with(
    root: &Path,
    base: &str,
    porcelain: impl FnOnce() -> Result<String, String>,
) -> Result<GitStatus, String> {
    let before = fingerprint(root)?;
    let (staged, changes, conflicts) = parse_status(&porcelain()?)?;
    let branch = optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let has_head = optional(root, &["rev-parse", "--verify", "HEAD"]).is_some();
    let upstream = optional(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    let (ahead, behind) = if upstream.is_some() {
        let counts = run(
            root,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        )?;
        let mut counts = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
        (counts.next().unwrap_or(0), counts.next().unwrap_or(0))
    } else {
        (0, 0)
    };
    let index = fingerprint(root)?;
    if index != before {
        return Err(i18n::t("err.git.changed"));
    }
    Ok(GitStatus {
        branch,
        has_head,
        upstream,
        ahead,
        behind,
        staged,
        changes,
        conflicts,
        merging: optional(root, &["rev-parse", "--quiet", "--verify", "MERGE_HEAD"]).is_some(),
        base: if base.is_empty() {
            default_base(root)
        } else {
            base.to_string()
        },
        remotes: run(root, &["remote"])?
            .lines()
            .map(str::to_string)
            .collect(),
        index,
        ..GitStatus::default()
    })
}

#[tauri::command(async)]
pub fn workspace_git_status(state: State<AppState>, id: String) -> Result<Vec<GitStatus>, String> {
    Ok(workspace_repos(&state, &id)?
        .iter()
        .enumerate()
        .map(|(index, repo)| {
            let mut value =
                cached_status(Path::new(&repo.worktree), &repo.base).unwrap_or_else(|error| {
                    GitStatus {
                        error: Some(error),
                        ..GitStatus::default()
                    }
                });
            value.repo = index;
            value.name = repo.name.clone();
            value
        })
        .collect())
}

/// Collapse one porcelain entry into the file tree's mark. Untracked and added files read as new;
/// a deletion on either side as deleted; conflicts win over everything else. A file deleted from
/// the worktree reads as deleted even when its addition is staged (`AD`): it is no longer on disk,
/// so only a deleted row can show it.
fn tree_mark(pair: &str) -> &'static str {
    if pair.contains('U') || pair == "AA" || pair == "DD" {
        "U"
    } else if pair.as_bytes().get(1) == Some(&b'D') {
        "D"
    } else if pair == "??" || pair.contains('A') {
        "A"
    } else if pair.contains('D') {
        "D"
    } else {
        "M"
    }
}

/// Changed paths under `root`, relative to it, for each repository directory in `repos`. Untracked
/// files are listed one by one, like the Changes pane, so ignored files inside a new folder stay
/// unmarked. A directory that is not inside a Git repository contributes nothing.
fn tree_marks(root: &Path, repos: &[PathBuf]) -> Vec<GitFile> {
    let mut marks = Vec::new();
    for dir in repos {
        let Ok(place) = dir.strip_prefix(root) else {
            continue;
        };
        let place = place.to_string_lossy().replace('\\', "/");
        // Porcelain paths are relative to the repository top level, which may sit above `dir`.
        let Some(inner) = run(dir, &["rev-parse", "--show-prefix"]).ok() else {
            continue;
        };
        let inner = inner.trim_end_matches('\n');
        let text = if inner.is_empty() {
            raw_status(dir)
        } else {
            run(
                dir,
                &[
                    "status",
                    "--porcelain=v1",
                    "-z",
                    "--untracked-files=all",
                    "--no-renames",
                    "--",
                    ".",
                ],
            )
        };
        let Ok(text) = text else {
            continue;
        };
        for entry in text.split('\0') {
            if entry.len() < 4 || !entry.is_char_boundary(3) {
                continue;
            }
            let Some(path) = entry[3..].strip_prefix(inner) else {
                continue;
            };
            marks.push(GitFile {
                path: match place.is_empty() {
                    true => path.to_string(),
                    false => format!("{place}/{path}"),
                },
                status: tree_mark(&entry[..2]).to_string(),
            });
        }
    }
    marks
}

/// Git marks for the side file tree. Accepts a workspace or a project id, like `list_dir`, and
/// never fails: a tree outside Git simply has no marks. Async keeps the scan off the main thread.
#[tauri::command(async)]
pub fn tree_git_status(state: State<AppState>, id: String) -> Vec<GitFile> {
    tree_repos(&state, &id)
        .map(|(root, repos)| tree_marks(&root, &repos))
        .unwrap_or_default()
}

/// The file tree's root and the repository directories under it: a workspace's worktrees, or the
/// project folder itself.
fn tree_repos(state: &State<AppState>, id: &str) -> Option<(PathBuf, Vec<PathBuf>)> {
    let root = super::cwd_of(state, id)?;
    let repos = workspace_repos(state, id)
        .map(|repos| {
            repos
                .iter()
                .map(|repo| PathBuf::from(&repo.worktree))
                .collect()
        })
        .unwrap_or_else(|_| vec![root.clone()]);
    Some((root, repos))
}

/// Bring a deleted tree entry back to disk. Only an entry that is gone from disk qualifies, so this
/// can never discard edits to a file that still exists. The index wins over `HEAD`: a file whose
/// edits were staged before it left the disk comes back with that staged content, still staged, and
/// only a path whose deletion was staged too comes back from `HEAD`, in the index and on disk. A
/// folder mixes both, file by file.
fn restore_deleted(root: &Path, repos: &[PathBuf], rel: &str) -> Result<(), String> {
    let (dir, inner) = repos
        .iter()
        .filter_map(|dir| {
            let place = dir
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            let inner = match place.is_empty() {
                true => rel,
                false => rel.strip_prefix(&place)?.strip_prefix('/')?,
            };
            Some((dir, inner))
        })
        .next()
        .ok_or_else(|| i18n::t("err.session.outside"))?;
    let path = valid_path(dir, inner)?;
    if path.symlink_metadata().is_ok() {
        return Err(i18n::ta("err.files.exists", &[("name", inner.to_string())]));
    }
    let listed = |args: &[&str]| -> Vec<String> {
        run(dir, args)
            .map(|out| {
                out.split('\0')
                    .filter(|path| !path.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let staged = listed(&["ls-files", "-z", "--cached", "--", inner]);
    // An unborn branch has no HEAD to list; the index is then the only source.
    let committed = listed(&["ls-tree", "-r", "-z", "--name-only", "HEAD", "--", inner]);
    let from_head: Vec<&str> = committed
        .iter()
        .filter(|path| !staged.contains(path))
        .map(String::as_str)
        .collect();
    if staged.is_empty() && from_head.is_empty() {
        return Err(i18n::t("err.session.outside"));
    }
    if !staged.is_empty() {
        run(dir, &["restore", "--worktree", "--", inner])?;
    }
    if !from_head.is_empty() {
        let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
        args.extend(from_head);
        run(dir, &args)?;
    }
    Ok(())
}

/// Restore a file or folder the tree shows as deleted. Accepts a workspace or a project id.
#[tauri::command(async)]
pub fn tree_restore(state: State<AppState>, id: String, rel: String) -> Result<(), String> {
    let _guard = MUTATION.try_lock().map_err(|_| i18n::t("err.git.busy"))?;
    let (root, repos) =
        tree_repos(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let result = restore_deleted(&root, &repos, &rel);
    invalidate_path(&root);
    result
}

fn valid_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') || Path::new(path).components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git"))
    }) {
        return Err(i18n::t("err.session.outside"));
    }
    let root = root.canonicalize().map_err(i18n::io)?;
    let file = root.join(path);
    // The final component may be a tracked symlink. Parent directories must remain inside the
    // repository, including for deleted-file operations.
    let mut parent = file.parent();
    while let Some(dir) = parent {
        if dir.exists() {
            if !dir.canonicalize().map_err(i18n::io)?.starts_with(&root) {
                return Err(i18n::t("err.session.outside"));
            }
            break;
        }
        parent = dir.parent();
    }
    Ok(file)
}

fn revision(root: &Path, name: &str) -> Result<String, String> {
    run(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{name}^{{commit}}"),
        ],
    )
    .map(|text| text.trim().to_string())
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
    pub files: Vec<super::diff::FileChange>,
}

fn diff(
    root: &Path,
    scope: DiffScope,
    path: Option<&str>,
    reference: Option<&str>,
) -> Result<GitDiff, String> {
    if let Some(path) = path {
        valid_path(root, path)?;
    }
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "-U3",
    ];
    let mut base = String::new();
    let mut head = String::new();
    match scope {
        DiffScope::Staged => args.push("--cached"),
        DiffScope::Changes => (),
        DiffScope::Compare => {
            head = revision(root, "HEAD")?;
            let reference = revision(root, reference.unwrap_or("HEAD"))?;
            base = run(root, &["merge-base", &reference, &head])?
                .trim()
                .to_string();
            args.extend([base.as_str(), head.as_str()]);
        }
        DiffScope::Commit => {
            head = revision(root, reference.unwrap_or("HEAD"))?;
            base = optional(root, &["rev-parse", "--verify", &format!("{head}^")]).unwrap_or(
                run(root, &["hash-object", "-t", "tree", "--stdin"])?
                    .trim()
                    .to_string(),
            );
            args.extend([base.as_str(), head.as_str()]);
        }
    }
    let mut names = args.clone();
    names.extend(["--name-only", "-z", "--"]);
    if let Some(path) = path {
        names.push(path);
    }
    let mut paths: Vec<String> = run(root, &names)?
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    let untracked = if matches!(scope, DiffScope::Changes) {
        let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
        if let Some(path) = path {
            args.push(path);
        }
        run(root, &args)?
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    paths.extend(untracked.iter().cloned());
    let mut files = Vec::new();
    for path in paths {
        let is_new = untracked.contains(&path);
        let raw = if is_new {
            let file = valid_path(root, &path)?;
            if std::fs::symlink_metadata(&file)
                .map_err(i18n::io)?
                .file_type()
                .is_symlink()
            {
                String::new()
            } else {
                let metadata = std::fs::metadata(&file).map_err(i18n::io)?;
                if metadata.len() > TEXT_LIMIT as u64 {
                    String::new()
                } else {
                    let text = std::fs::read_to_string(file).unwrap_or_default();
                    if text.contains('\0') {
                        String::new()
                    } else {
                        format!(
                            "@@ -0,0 +1,{} @@\n{}",
                            text.lines().count(),
                            text.lines()
                                .map(|line| format!("+{line}\n"))
                                .collect::<String>()
                        )
                    }
                }
            }
        } else {
            let mut command = args.clone();
            command.extend(["--", &path]);
            run(root, &command)?
        };
        let new_file = is_new || raw.contains("\nnew file mode ");
        let deleted = raw.contains("\ndeleted file mode ");
        let patch = raw
            .lines()
            .skip_while(|line| !line.starts_with("@@"))
            .collect::<Vec<_>>()
            .join("\n");
        files.push(super::diff::FileChange {
            path,
            added: patch.lines().filter(|line| line.starts_with('+')).count() as u32,
            removed: patch.lines().filter(|line| line.starts_with('-')).count() as u32,
            new_file,
            deleted,
            dirty: matches!(scope, DiffScope::Staged | DiffScope::Changes),
            patch: if patch.len() > TEXT_LIMIT {
                String::new()
            } else {
                patch
            },
        });
    }
    Ok(GitDiff { base, head, files })
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
    let repo = repository(&state, &id, repo)?;
    let reference = reference.or_else(|| {
        matches!(scope, DiffScope::Compare).then(|| {
            if repo.base.is_empty() {
                default_base(Path::new(&repo.worktree))
            } else {
                repo.base.clone()
            }
        })
    });
    diff(
        Path::new(&repo.worktree),
        scope,
        path.as_deref(),
        reference.as_deref(),
    )
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

fn action(
    root: &Path,
    action: GitAction,
    paths: &[String],
    message: Option<&str>,
    expected: Option<&str>,
    remote: Option<&str>,
) -> Result<(), String> {
    let current = status(root, "")?;
    match action {
        GitAction::Stage | GitAction::Unstage => {
            if paths.is_empty() {
                return Err(i18n::t("err.git.selection"));
            }
            for path in paths {
                valid_path(root, path)?;
                let source = if matches!(action, GitAction::Unstage) {
                    &current.staged
                } else {
                    &current.changes
                };
                let conflict = matches!(action, GitAction::Stage)
                    && current.conflicts.iter().any(|file| &file.path == path);
                if !source.iter().any(|file| &file.path == path) && !conflict {
                    return Err(i18n::t("err.git.changed"));
                }
                if conflict {
                    let file = valid_path(root, path)?;
                    if std::fs::symlink_metadata(&file).is_ok_and(|metadata| metadata.is_file()) {
                        if let Ok(text) = std::fs::read_to_string(file) {
                            if conflict_markers(&text) {
                                return Err(i18n::t("err.git.conflicts"));
                            }
                        }
                    }
                }
            }
            let mut args = match action {
                GitAction::Stage => vec!["add", "--"],
                _ if current.has_head => vec!["restore", "--staged", "--"],
                _ => vec!["rm", "--cached", "--force", "--quiet", "--"],
            };
            args.extend(paths.iter().map(String::as_str));
            run(root, &args)?;
        }
        GitAction::Discard => {
            if paths.is_empty() {
                return Err(i18n::t("err.git.selection"));
            }
            let (mut tracked, mut untracked) = (Vec::new(), Vec::new());
            for path in paths {
                valid_path(root, path)?;
                let file = current
                    .changes
                    .iter()
                    .find(|file| &file.path == path)
                    .filter(|_| !current.conflicts.iter().any(|file| &file.path == path))
                    .ok_or_else(|| i18n::t("err.git.changed"))?;
                if file.status == "?" {
                    untracked.push(path.as_str());
                } else {
                    tracked.push(path.as_str());
                }
            }
            if !tracked.is_empty() {
                // Without `--source`, restore reads the index, so staged content survives.
                let mut args = vec!["restore", "--worktree", "--"];
                args.extend(tracked);
                run(root, &args)?;
            }
            if !untracked.is_empty() {
                // Clean refuses ignored files and directories without extra flags; status lists
                // untracked files one by one, so only those exact paths go.
                let mut args = vec!["clean", "--force", "--quiet", "--"];
                args.extend(untracked);
                run(root, &args)?;
            }
        }
        GitAction::Commit => {
            if current.branch.is_none() {
                return Err(i18n::t("err.git.detached"));
            }
            if !current.conflicts.is_empty() {
                return Err(i18n::t("err.git.conflicts"));
            }
            if (current.staged.is_empty() && !current.merging)
                || message.unwrap_or("").trim().is_empty()
            {
                return Err(i18n::t("err.git.selection"));
            }
            if expected != Some(&current.index) {
                return Err(i18n::t("err.git.changed"));
            }
            run(
                root,
                &["-c", "core.editor=true", "commit", "-m", message.unwrap()],
            )?;
        }
        GitAction::Fetch => {
            run(root, &["fetch", "--all"])?;
        }
        GitAction::Pull => {
            if current.branch.is_none() {
                return Err(i18n::t("err.git.detached"));
            }
            if current.upstream.is_none() {
                return Err(i18n::t("err.git.upstream"));
            }
            if !current.staged.is_empty()
                || !current.changes.is_empty()
                || !current.conflicts.is_empty()
            {
                return Err(i18n::t("err.git.dirtyPull"));
            }
            run(
                root,
                &[
                    "-c",
                    "merge.autoStash=false",
                    "pull",
                    "--ff-only",
                    "--no-rebase",
                ],
            )?;
        }
        GitAction::Push | GitAction::Publish => {
            let branch = current.branch.ok_or_else(|| i18n::t("err.git.detached"))?;
            if !current.has_head {
                return Err(i18n::t("err.git.selection"));
            }
            if matches!(action, GitAction::Publish) {
                let remote = remote
                    .filter(|remote| current.remotes.iter().any(|known| known == remote))
                    .ok_or_else(|| i18n::t("err.git.upstream"))?;
                run(
                    root,
                    &[
                        "push",
                        "--no-follow-tags",
                        "--set-upstream",
                        "--",
                        remote,
                        &format!("HEAD:refs/heads/{branch}"),
                    ],
                )?;
            } else {
                if current.upstream.is_none() {
                    return Err(i18n::t("err.git.upstream"));
                }
                let tracking = run(
                    root,
                    &[
                        "for-each-ref",
                        "--format=%(upstream:remotename)%00%(upstream:remoteref)",
                        &format!("refs/heads/{branch}"),
                    ],
                )?;
                let (remote, reference) = tracking
                    .trim()
                    .split_once('\0')
                    .ok_or_else(|| i18n::t("err.git.upstream"))?;
                run(
                    root,
                    &[
                        "push",
                        "--no-follow-tags",
                        "--",
                        remote,
                        &format!("HEAD:{reference}"),
                    ],
                )?;
            }
        }
    }
    Ok(())
}

#[tauri::command(async)]
// Keep optional action fields flat in the typed IPC contract.
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
    let _guard = MUTATION.try_lock().map_err(|_| i18n::t("err.git.busy"))?;
    // Both rewrite files an agent may be editing mid-turn.
    if matches!(operation, GitAction::Pull | GitAction::Discard)
        && lock(&state.board).workspaces.iter().any(|workspace| {
            workspace.id == id
                && workspace
                    .tabs
                    .iter()
                    .any(|tab| tab.status == crate::state::Status::Rodando)
        })
    {
        return Err(i18n::t("err.git.agent"));
    }
    let repo = repository(&state, &id, repo)?;
    let result = action(
        Path::new(&repo.worktree),
        operation,
        &paths,
        message.as_deref(),
        expected.as_deref(),
        remote.as_deref(),
    );
    invalidate_path(Path::new(&repo.worktree));
    result
}

#[derive(Serialize)]
pub struct GitCommit {
    pub oid: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub outgoing: bool,
}

#[tauri::command(async)]
pub fn workspace_git_history(
    state: State<AppState>,
    id: String,
    repo: usize,
) -> Result<Vec<GitCommit>, String> {
    let repo = repository(&state, &id, repo)?;
    history(Path::new(&repo.worktree))
}

fn history(root: &Path) -> Result<Vec<GitCommit>, String> {
    if optional(root, &["rev-parse", "--verify", "HEAD"]).is_none() {
        return Ok(Vec::new());
    }
    let outgoing = optional(root, &["rev-list", "@{upstream}..HEAD"]).unwrap_or_default();
    Ok(run(
        root,
        &["log", "-100", "--format=%H%x00%s%x00%an%x00%aI%x00"],
    )?
    .split('\0')
    .collect::<Vec<_>>()
    .chunks_exact(4)
    .map(|parts| {
        let oid = parts[0].trim().to_string();
        let is_outgoing = outgoing.lines().any(|line| line == oid);
        GitCommit {
            oid,
            subject: parts[1].to_string(),
            author: parts[2].to_string(),
            date: parts[3].to_string(),
            outgoing: is_outgoing,
        }
    })
    .collect())
}

#[derive(Serialize)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    pub remote: bool,
    pub worktree: Option<String>,
    pub workspace: Option<String>,
}

#[tauri::command(async)]
pub fn workspace_git_branches(
    state: State<AppState>,
    id: String,
    repo: usize,
) -> Result<Vec<GitBranch>, String> {
    let repo = repository(&state, &id, repo)?;
    let root = Path::new(&repo.worktree);
    let current = optional(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let branches = run(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    let board = lock(&state.board).clone();
    Ok(branches
        .lines()
        .filter_map(|line| {
            let (reference, name) = line.split_once('\0')?;
            if reference.ends_with("/HEAD") {
                return None;
            }
            let remote = reference.starts_with("refs/remotes/");
            let worktree = if remote {
                None
            } else {
                worktree_of_branch(root, name)
            };
            let workspace = worktree
                .as_ref()
                .and_then(|path| {
                    board.workspaces.iter().find(|workspace| {
                        !workspace.cleaned
                            && workspace
                                .repos
                                .iter()
                                .any(|repo| super::same_path(Path::new(&repo.worktree), path))
                    })
                })
                .map(|workspace| workspace.id.clone());
            Some(GitBranch {
                name: name.to_string(),
                current: current.as_deref() == Some(name),
                remote,
                worktree: worktree.map(|path| path.to_string_lossy().into_owned()),
                workspace,
            })
        })
        .collect())
}

#[derive(Serialize)]
pub struct GitConflict {
    pub current: String,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

fn conflict(root: &Path, path: &str) -> Result<GitConflict, String> {
    if !status(root, "")?
        .conflicts
        .iter()
        .any(|file| file.path == path)
    {
        return Err(i18n::t("err.git.changed"));
    }
    let file = valid_path(root, path)?;
    let current = if let Ok(metadata) = std::fs::symlink_metadata(&file) {
        if !metadata.is_file() || metadata.len() > TEXT_LIMIT as u64 {
            return Err(i18n::t("err.session.binary"));
        }
        std::fs::read_to_string(file).map_err(i18n::io)?
    } else {
        String::new()
    };
    let blob = |stage| -> Result<Option<String>, String> {
        let output = output(root, &["show", &format!(":{stage}:{path}")])?;
        if !output.status.success() {
            return Ok(None);
        }
        String::from_utf8(output.stdout)
            .map(Some)
            .map_err(|_| i18n::t("err.session.binary"))
    };
    let ours = blob(2)?;
    let theirs = blob(3)?;
    if [&current]
        .into_iter()
        .chain(ours.iter())
        .chain(theirs.iter())
        .any(|text| text.contains('\0') || text.len() > TEXT_LIMIT)
    {
        return Err(i18n::t("err.session.binary"));
    }
    Ok(GitConflict {
        current,
        ours,
        theirs,
    })
}

#[tauri::command(async)]
pub fn workspace_git_conflict(
    state: State<AppState>,
    id: String,
    repo: usize,
    path: String,
) -> Result<GitConflict, String> {
    let repo = repository(&state, &id, repo)?;
    conflict(Path::new(&repo.worktree), &path)
}

fn resolve(root: &Path, path: &str, was: &str, text: &str) -> Result<(), String> {
    let current = conflict(root, path)?;
    if current.current != was {
        return Err(i18n::t("err.session.changed"));
    }
    if text.len() > TEXT_LIMIT || conflict_markers(text) {
        return Err(i18n::t("err.git.conflicts"));
    }
    let file = valid_path(root, path)?;
    std::fs::write(file, text).map_err(i18n::io)?;
    run(root, &["add", "--", path])?;
    Ok(())
}

fn conflict_markers(text: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with("<<<<<<< ") || line == "=======" || line.starts_with(">>>>>>> ")
    })
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
    let _guard = MUTATION.try_lock().map_err(|_| i18n::t("err.git.busy"))?;
    let repo = repository(&state, &id, repo)?;
    let result = resolve(Path::new(&repo.worktree), &path, &was, &text);
    invalidate_path(Path::new(&repo.worktree));
    result
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
