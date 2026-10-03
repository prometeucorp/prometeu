//! Local Git operations distinguish the index from working-tree changes. They never switch
//! workspace branches; branch lifecycle remains with worktrees.

use prometeu_core::board::{Board, Repo};
use prometeu_core::git::*;
use prometeu_core::lock::lock;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ponytail: serialize Git mutations across the app; use per-repository locks when concurrent
// operations on separate repositories are needed.
static MUTATION: Mutex<()> = Mutex::new(());
const TEXT_LIMIT: usize = 400_000;
/// Matches `read_file`'s limit: a larger file never reaches the editor.
const BASE_LIMIT: usize = 2 * 1024 * 1024;
const STATUS_TTL: Duration = Duration::from_secs(2);
const PORCELAIN_TTL: Duration = Duration::from_secs(1);

struct SlotState<T> {
    generation: u64,
    scanned: u64,
    /// Counts published scans, so a waiter takes the result of the scan it joined even when that
    /// result is an error that later readers must not reuse.
    completed: u64,
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
                completed: 0,
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

    fn generation(&self) -> u64 {
        lock(&self.state).generation
    }

    fn fresh(&self, ttl: Duration) -> bool {
        let state = lock(&self.state);
        state.scanned == state.generation && state.at.is_some_and(|at| at.elapsed() < ttl)
    }

    /// Publish a value computed outside `read`, unless a scan owns the slot or an invalidation
    /// arrived after `generation` was read.
    fn offer(&self, generation: u64, value: T) {
        let mut state = lock(&self.state);
        if state.scanning || state.generation != generation {
            return;
        }
        state.scanned = generation;
        state.completed += 1;
        state.at = Some(Instant::now());
        state.value = Some(Ok(value));
    }

    fn read(
        &self,
        ttl: Duration,
        mut compute: impl FnMut() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut state = lock(&self.state);
        let mut joined = None;
        loop {
            if state.scanned == state.generation
                && (state.at.is_some_and(|at| at.elapsed() < ttl)
                    || joined.is_some_and(|seen| seen != state.completed))
            {
                return state
                    .value
                    .as_ref()
                    .expect("scanned slot has a value")
                    .clone();
            }
            if state.scanning {
                joined.get_or_insert(state.completed);
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
                // An app mutation arrived during the scan. Keep ownership of the slot and scan
                // again until one scan finishes without an invalidation, so no waiter observes a
                // stale result.
                drop(state);
                continue;
            }
            state.scanned = generation;
            state.completed += 1;
            // An error reaches the waiters of this scan only; the next reader scans again.
            state.at = result.is_ok().then(Instant::now);
            state.value = Some(result.clone());
            state.scanning = false;
            self.changed.notify_all();
            return result;
        }
    }
}

/// Return the slot for `key`, dropping unused slots whose value expired: such a slot behaves like
/// a new one, so removed or cleaned worktrees do not keep their last scan in memory.
fn slot<K: Eq + std::hash::Hash, T: Clone>(
    slots: &Mutex<HashMap<K, Arc<Slot<T>>>>,
    key: K,
    ttl: Duration,
) -> Arc<Slot<T>> {
    let mut slots = lock(slots);
    slots.retain(|_, slot| Arc::strong_count(slot) > 1 || slot.fresh(ttl));
    slots.entry(key).or_default().clone()
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

const PORCELAIN: [&str; 5] = [
    "status",
    "--porcelain=v1",
    "-z",
    "--untracked-files=all",
    "--no-renames",
];

fn raw_slot(root: &Path) -> Arc<Slot<String>> {
    slot(raw_slots(), cache_key(root), PORCELAIN_TTL)
}

fn raw_status(root: &Path) -> Result<String, String> {
    raw_slot(root).read(PORCELAIN_TTL, || run(root, &PORCELAIN))
}

fn cached_status(root: &Path, base: &str) -> Result<GitStatus, String> {
    let key = (cache_key(root), base.to_string());
    slot(status_slots(), key, STATUS_TTL).read(STATUS_TTL, || {
        // The commit token must describe the listed files, so Changes reads its own porcelain
        // inside the fingerprint bracket and never reuses an older Files scan. Files marks may
        // reuse this snapshot instead.
        let raw = raw_slot(root);
        let generation = raw.generation();
        status_with(root, base, || {
            let text = run(root, &PORCELAIN)?;
            raw.offer(generation, text.clone());
            Ok(text)
        })
    })
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

fn command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.args(["--literal-pathspecs", "-C"]).arg(root);
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command
}

fn output(root: &Path, args: &[&str]) -> Result<Output, String> {
    command(root).args(args).output().map_err(io_error)
}

fn run(root: &Path, args: &[&str]) -> Result<String, String> {
    let result = output(root, args)?;
    if !result.status.success() {
        return Err(prometeu_core::error::with_args(
            "err.git.command",
            &[(
                "cause",
                String::from_utf8_lossy(&result.stderr).trim().to_string(),
            )],
        ));
    }
    String::from_utf8(result.stdout).map_err(|_| prometeu_core::error::code("err.session.binary"))
}

fn optional(root: &Path, args: &[&str]) -> Option<String> {
    run(root, args)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
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
            return Err(prometeu_core::error::code("err.git.status"));
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
    status_with(root, base, || run(root, &PORCELAIN))
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
        return Err(prometeu_core::error::code("err.git.changed"));
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

/// Bring a deleted tree entry back to disk. Only an entry that is gone from disk qualifies, so this
/// can never discard edits to a file that still exists. The index wins over `HEAD`: a file whose
/// edits were staged before it left the disk comes back with that staged content, still staged, and
/// only a path whose deletion was staged too comes back from `HEAD`, in the index and on disk. A
/// folder mixes both, file by file.
/// The repository directory holding the tree path `rel`, and `rel` relative to it.
fn repo_of<'a>(root: &Path, repos: &'a [PathBuf], rel: &'a str) -> Option<(&'a PathBuf, &'a str)> {
    repos.iter().find_map(|dir| {
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
}

/// The committed text the editor compares a file with. A file Git does not know yet, untracked or
/// only staged, compares with an empty text, so every line reads as new. `None` means there is no
/// comparison to draw: outside Git, ignored, binary, or too large.
fn file_base_text(root: &Path, repos: &[PathBuf], rel: &str) -> Option<String> {
    let (dir, inner) = repo_of(root, repos, rel)?;
    valid_path(dir, inner).ok()?;
    // `./` resolves the path from `dir`, which may sit below the repository top level.
    let spec = format!("HEAD:./{inner}");
    if let Ok(oid) = run(dir, &["rev-parse", "--verify", "-q", &spec]) {
        let oid = oid.trim();
        let size: usize = run(dir, &["cat-file", "-s", oid])
            .ok()?
            .trim()
            .parse()
            .ok()?;
        if size > BASE_LIMIT {
            return None;
        }
        return run(dir, &["cat-file", "blob", oid]).ok();
    }
    let known = run(
        dir,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            inner,
        ],
    )
    .ok()?;
    (!known.is_empty()).then(String::new)
}

fn restore_deleted(root: &Path, repos: &[PathBuf], rel: &str) -> Result<(), String> {
    let (dir, inner) = repo_of(root, repos, rel)
        .ok_or_else(|| prometeu_core::error::code("err.session.outside"))?;
    let path = valid_path(dir, inner)?;
    if path.symlink_metadata().is_ok() {
        return Err(prometeu_core::error::with_args(
            "err.files.exists",
            &[("name", inner.to_string())],
        ));
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
        return Err(prometeu_core::error::code("err.session.outside"));
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

fn valid_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') || Path::new(path).components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git"))
    }) {
        return Err(prometeu_core::error::code("err.session.outside"));
    }
    let root = root.canonicalize().map_err(io_error)?;
    let file = root.join(path);
    // The final component may be a tracked symlink. Parent directories must remain inside the
    // repository, including for deleted-file operations.
    let mut parent = file.parent();
    while let Some(dir) = parent {
        if dir.exists() {
            if !dir.canonicalize().map_err(io_error)?.starts_with(&root) {
                return Err(prometeu_core::error::code("err.session.outside"));
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
                .map_err(io_error)?
                .file_type()
                .is_symlink()
            {
                String::new()
            } else {
                let metadata = std::fs::metadata(&file).map_err(io_error)?;
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
        files.push(FileChange {
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
                return Err(prometeu_core::error::code("err.git.selection"));
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
                    return Err(prometeu_core::error::code("err.git.changed"));
                }
                if conflict {
                    let file = valid_path(root, path)?;
                    if std::fs::symlink_metadata(&file).is_ok_and(|metadata| metadata.is_file()) {
                        if let Ok(text) = std::fs::read_to_string(file) {
                            if conflict_markers(&text) {
                                return Err(prometeu_core::error::code("err.git.conflicts"));
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
                return Err(prometeu_core::error::code("err.git.selection"));
            }
            let (mut tracked, mut untracked) = (Vec::new(), Vec::new());
            for path in paths {
                valid_path(root, path)?;
                let file = current
                    .changes
                    .iter()
                    .find(|file| &file.path == path)
                    .filter(|_| !current.conflicts.iter().any(|file| &file.path == path))
                    .ok_or_else(|| prometeu_core::error::code("err.git.changed"))?;
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
                return Err(prometeu_core::error::code("err.git.detached"));
            }
            if !current.conflicts.is_empty() {
                return Err(prometeu_core::error::code("err.git.conflicts"));
            }
            if (current.staged.is_empty() && !current.merging)
                || message.unwrap_or("").trim().is_empty()
            {
                return Err(prometeu_core::error::code("err.git.selection"));
            }
            if expected != Some(&current.index) {
                return Err(prometeu_core::error::code("err.git.changed"));
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
                return Err(prometeu_core::error::code("err.git.detached"));
            }
            if current.upstream.is_none() {
                return Err(prometeu_core::error::code("err.git.upstream"));
            }
            if !current.staged.is_empty()
                || !current.changes.is_empty()
                || !current.conflicts.is_empty()
            {
                return Err(prometeu_core::error::code("err.git.dirtyPull"));
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
            let branch = current
                .branch
                .ok_or_else(|| prometeu_core::error::code("err.git.detached"))?;
            if !current.has_head {
                return Err(prometeu_core::error::code("err.git.selection"));
            }
            if matches!(action, GitAction::Publish) {
                let remote = remote
                    .filter(|remote| current.remotes.iter().any(|known| known == remote))
                    .ok_or_else(|| prometeu_core::error::code("err.git.upstream"))?;
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
                    return Err(prometeu_core::error::code("err.git.upstream"));
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
                    .ok_or_else(|| prometeu_core::error::code("err.git.upstream"))?;
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

fn conflict(root: &Path, path: &str) -> Result<GitConflict, String> {
    if !status(root, "")?
        .conflicts
        .iter()
        .any(|file| file.path == path)
    {
        return Err(prometeu_core::error::code("err.git.changed"));
    }
    let file = valid_path(root, path)?;
    let current = if let Ok(metadata) = std::fs::symlink_metadata(&file) {
        if !metadata.is_file() || metadata.len() > TEXT_LIMIT as u64 {
            return Err(prometeu_core::error::code("err.session.binary"));
        }
        std::fs::read_to_string(file).map_err(io_error)?
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
            .map_err(|_| prometeu_core::error::code("err.session.binary"))
    };
    let ours = blob(2)?;
    let theirs = blob(3)?;
    if [&current]
        .into_iter()
        .chain(ours.iter())
        .chain(theirs.iter())
        .any(|text| text.contains('\0') || text.len() > TEXT_LIMIT)
    {
        return Err(prometeu_core::error::code("err.session.binary"));
    }
    Ok(GitConflict {
        current,
        ours,
        theirs,
    })
}

fn resolve(root: &Path, path: &str, was: &str, text: &str) -> Result<(), String> {
    let current = conflict(root, path)?;
    if current.current != was {
        return Err(prometeu_core::error::code("err.session.changed"));
    }
    if text.len() > TEXT_LIMIT || conflict_markers(text) {
        return Err(prometeu_core::error::code("err.git.conflicts"));
    }
    let file = valid_path(root, path)?;
    std::fs::write(file, text).map_err(io_error)?;
    run(root, &["add", "--", path])?;
    Ok(())
}

fn conflict_markers(text: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with("<<<<<<< ") || line == "=======" || line.starts_with(">>>>>>> ")
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

fn io_error(cause: impl ToString) -> String {
    prometeu_core::error::with_args("err.io", &[("cause", cause.to_string())])
}
fn default_base(root: &Path) -> String {
    prometeu_core::repository::branches(
        |args| run(root, args).unwrap_or_default(),
        root.join(".git").exists(),
    )
    .default
}

fn worktree_of_branch(repo: &Path, branch: &str) -> Option<PathBuf> {
    let want = format!("branch refs/heads/{branch}");
    let listed = run(repo, &["worktree", "list", "--porcelain"]).ok()?;
    let mut at = None;
    for line in listed.lines() {
        match line.strip_prefix("worktree ") {
            Some(path) => at = Some(PathBuf::from(path)),
            None if line == want => return at,
            None => {}
        }
    }
    None
}

fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

/// Native Git, shared by the original desktop and the WSL runtime. All process/file effects stay
/// behind the repository port; caches retain their canonical-worktree keys and mutation lock.
pub struct NativeGit;
impl RepositoryGit for NativeGit {
    fn file_base(&self, root: &Path, repos: &[PathBuf], rel: &str) -> Option<String> {
        file_base_text(root, repos, rel)
    }
    fn status(&self, repos: &[Repo]) -> Vec<GitStatus> {
        repos
            .iter()
            .enumerate()
            .map(|(index, repo)| {
                let mut value = cached_status(Path::new(&repo.worktree), &repo.base)
                    .unwrap_or_else(|error| GitStatus {
                        error: Some(error),
                        ..Default::default()
                    });
                value.repo = index;
                value.name = repo.name.clone();
                value
            })
            .collect()
    }
    fn diff(
        &self,
        repo: &Repo,
        scope: DiffScope,
        path: Option<&str>,
        reference: Option<&str>,
    ) -> Result<GitDiff, String> {
        let fallback = (reference.is_none() && matches!(scope, DiffScope::Compare)).then(|| {
            match repo.base.is_empty() {
                true => default_base(Path::new(&repo.worktree)),
                false => repo.base.clone(),
            }
        });
        let reference = reference.or(fallback.as_deref());
        diff(Path::new(&repo.worktree), scope, path, reference)
    }
    fn action(&self, repo: &Repo, mutation: Mutation<'_>) -> Result<(), String> {
        let _guard = MUTATION
            .try_lock()
            .map_err(|_| prometeu_core::error::code("err.git.busy"))?;
        let root = Path::new(&repo.worktree);
        let result = action(
            root,
            mutation.operation,
            mutation.paths,
            mutation.message,
            mutation.expected,
            mutation.remote,
        );
        invalidate_path(root);
        result
    }
    fn history(&self, repo: &Repo) -> Result<Vec<GitCommit>, String> {
        history(Path::new(&repo.worktree))
    }
    fn branches(&self, repo: &Repo, board: &Board) -> Result<Vec<GitBranch>, String> {
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
        Ok(branches
            .lines()
            .filter_map(|line| {
                let (reference, name) = line.split_once('\0')?;
                if reference.ends_with("/HEAD") {
                    return None;
                }
                let remote = reference.starts_with("refs/remotes/");
                let worktree = match remote {
                    true => None,
                    false => worktree_of_branch(root, name),
                };
                let workspace = worktree
                    .as_ref()
                    .and_then(|path| {
                        board.workspaces.iter().find(|w| {
                            !w.cleaned
                                && w.repos
                                    .iter()
                                    .any(|r| same_path(Path::new(&r.worktree), path))
                        })
                    })
                    .map(|w| w.id.clone());
                Some(GitBranch {
                    name: name.into(),
                    current: current.as_deref() == Some(name),
                    remote,
                    worktree: worktree.map(|p| p.to_string_lossy().into_owned()),
                    workspace,
                })
            })
            .collect())
    }
    fn conflict(&self, repo: &Repo, path: &str) -> Result<GitConflict, String> {
        conflict(Path::new(&repo.worktree), path)
    }
    fn resolve(&self, repo: &Repo, path: &str, was: &str, text: &str) -> Result<(), String> {
        let _guard = MUTATION
            .try_lock()
            .map_err(|_| prometeu_core::error::code("err.git.busy"))?;
        let root = Path::new(&repo.worktree);
        let result = resolve(root, path, was, text);
        invalidate_path(root);
        result
    }
    fn tree(&self, root: &Path, repos: &[PathBuf]) -> Vec<GitFile> {
        tree_marks(root, repos)
    }
    fn restore(&self, root: &Path, repos: &[PathBuf], rel: &str) -> Result<(), String> {
        let _guard = MUTATION
            .try_lock()
            .map_err(|_| prometeu_core::error::code("err.git.busy"))?;
        let result = restore_deleted(root, repos, rel);
        invalidate_path(root);
        result
    }
    fn invalidate(&self, path: &Path) {
        invalidate_path(path);
    }
}

pub mod cleanup;
