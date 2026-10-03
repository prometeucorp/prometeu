//! Native path indexing and shared desktop ranking for quick open and composer suggestions.
use prometeu_core::files::{Entry, ProjectSearch};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct NativeSearch {
    cache: Arc<Cache>,
}
impl ProjectSearch for NativeSearch {
    fn find(
        &self,
        root: &Path,
        repositories: &[PathBuf],
        query: &str,
        recent: &[String],
        files: bool,
    ) -> Vec<Entry> {
        // Include execution roots, not presentation IDs, so one host cannot reuse another root's index.
        let key = std::iter::once(root.to_path_buf())
            .chain(repositories.iter().cloned())
            .collect::<Vec<_>>();
        let root = root.to_path_buf();
        let repos = repositories.to_vec();
        let fresh = under(&root, recent);
        let all = cached(&self.cache, &key, move || scan(&root, &repos));
        let q = query.to_ascii_lowercase().into_bytes();
        ranked(&all, &q, &fresh, files)
            .into_iter()
            .take(MOST)
            .map(|span| Entry {
                name: all.name(span).to_string(),
                path: all.text(span).to_string(),
                dir: span.dir,
            })
            .collect()
    }
}

/// Shortlist, score and order every matching path, best first. With `files`, directories never
/// enter the shortlist, so they cannot take the place of a matching file.
fn ranked<'a>(
    all: &'a Corpus,
    q: &[u8],
    fresh: &HashMap<String, i32>,
    files: bool,
) -> Vec<&'a Span> {
    let short = shortlist(all, q, fresh, files);
    let mut hits: Vec<(i32, u32, u32, &Span)> = short
        .into_iter()
        .filter_map(|at| {
            let span = &all.at[at];
            let points = points(q, all, span)? + fresh.get(all.text(span)).copied().unwrap_or(0);
            Some((-points, span.depth, span.to - span.from, span))
        })
        .collect();
    hits.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(all.text(a.3).cmp(all.text(b.3)))
    });
    hits.into_iter().map(|(_, _, _, span)| span).collect()
}

/// Limit suggestion rows to fit the composer menu.
const MOST: usize = 40;

/* Prepared path data */

/// Pack all paths into one text buffer with offsets to avoid per-path allocation overhead. Fold
/// ASCII case during comparison rather than storing a second lowercase copy.
struct Corpus {
    raw: String,
    at: Vec<Span>,
}

/// Record path offsets and metadata; exclude the directory marker slash from the path slice.
struct Span {
    from: u32,
    name_at: u32,
    to: u32,
    depth: u32,
    dir: bool,
}

impl Corpus {
    /// The original path text.
    fn text(&self, at: &Span) -> &str {
        &self.raw[at.from as usize..at.to as usize]
    }
    fn bytes(&self, at: &Span) -> &[u8] {
        self.text(at).as_bytes()
    }
    fn name(&self, at: &Span) -> &str {
        &self.raw[at.name_at as usize..at.to as usize]
    }
    fn name_bytes(&self, at: &Span) -> &[u8] {
        self.name(at).as_bytes()
    }
}

fn corpus(paths: Vec<String>) -> Corpus {
    let mut raw = String::with_capacity(paths.iter().map(|p| p.len()).sum());
    let mut at = Vec::with_capacity(paths.len());
    for path in paths {
        let dir = path.ends_with('/');
        let text = path.trim_end_matches('/');
        let from = raw.len() as u32;
        at.push(Span {
            from,
            name_at: from + text.rfind('/').map(|slash| slash + 1).unwrap_or(0) as u32,
            to: from + text.len() as u32,
            depth: text.matches('/').count() as u32,
            dir,
        });
        raw.push_str(text);
    }
    Corpus { raw, at }
}

/// Fold ASCII case only; non-ASCII bytes must match as written.
fn low(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

/* Candidate filtering */

/// Cheaply rank name prefixes, contiguous matches, subsequences, and path matches before detailed
/// scoring. Filter independent chunks in parallel for large repositories, retaining only a bounded
/// candidate set.
fn shortlist(all: &Corpus, q: &[u8], fresh: &HashMap<String, i32>, files: bool) -> Vec<usize> {
    let hands = match all.at.len() {
        // Small repositories avoid parallel-dispatch overhead.
        0..=20_000 => 1,
        _ => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(8),
    };
    let each = all.at.len().div_ceil(hands.max(1));
    let keep: Vec<&str> = fresh.keys().map(|k| k.as_str()).collect();

    let mut found: Vec<Rough> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..hands)
            .map(|hand| {
                let first = (hand * each).min(all.at.len());
                let slice = &all.at[first..((hand + 1) * each).min(all.at.len())];
                let keep = &keep;
                scope.spawn(move || {
                    let mut best: BinaryHeap<Rough> = BinaryHeap::with_capacity(SHORT + 1);
                    for (at, span) in slice.iter().enumerate() {
                        if files && span.dir {
                            continue;
                        }
                        let Some(rank) = rough(q, all, span, keep) else {
                            continue;
                        };
                        let cand = Rough {
                            rank,
                            depth: span.depth,
                            len: span.to - span.from,
                            at: first + at,
                        };
                        // Keep the worst retained candidate at the heap top so inferior candidates
                        // can be rejected with one comparison.
                        if best.len() >= SHORT {
                            if best.peek().is_some_and(|worst| *worst <= cand) {
                                continue;
                            }
                            best.pop();
                        }
                        best.push(cand);
                    }
                    best.into_vec()
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .flatten()
            .collect()
    });

    found.sort_unstable();
    found.into_iter().map(|r| r.at).collect()
}

/// Bound candidates per worker so detailed-scoring cost does not grow with repository size.
const SHORT: usize = 256;

/// Estimate match quality cheaply. Higher is better; None rejects the path.
fn rough(q: &[u8], all: &Corpus, span: &Span, keep: &[&str]) -> Option<u8> {
    // Recently used files bypass candidate trimming so the detailed scorer can apply their
    // relevance boost.
    if !keep.is_empty() && keep.contains(&all.text(span)) {
        return Some(5);
    }
    if q.is_empty() {
        return Some(0);
    }
    // Check subsequence presence before substring placement because most paths lack the required
    // letters and can be rejected cheaply.
    let name = all.name_bytes(span);
    if !q.contains(&b'/') && subsequence(q, name) {
        return Some(if starts(q, name) {
            4
        } else if window(q, name) {
            3
        } else {
            2
        });
    }
    if !subsequence(q, all.bytes(span)) {
        return None;
    }
    Some(window(q, all.bytes(span)).into())
}

/// Order heap candidates with the worst first for efficient eviction.
#[derive(PartialEq, Eq)]
struct Rough {
    rank: u8,
    depth: u32,
    len: u32,
    at: usize,
}

impl Ord for Rough {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .rank
            .cmp(&self.rank)
            .then(self.depth.cmp(&other.depth))
            .then(self.len.cmp(&other.len))
            .then(self.at.cmp(&other.at))
    }
}

impl PartialOrd for Rough {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Test whether text starts with the already lowercase query.
fn starts(q: &[u8], text: &[u8]) -> bool {
    text.len() >= q.len() && text.iter().zip(q).all(|(a, b)| low(*a) == *b)
}

/// Test for a contiguous occurrence of the already lowercase query.
fn window(q: &[u8], text: &[u8]) -> bool {
    q.len() <= text.len()
        && text
            .windows(q.len())
            .any(|w| w.iter().zip(q).all(|(a, b)| low(*a) == *b))
}

/// Test whether query bytes appear in order, with gaps allowed.
fn subsequence(q: &[u8], text: &[u8]) -> bool {
    let mut left = q.iter();
    let mut want = left.next();
    for c in text {
        if Some(&low(*c)) == want.copied().as_ref() {
            want = left.next();
            if want.is_none() {
                return true;
            }
        }
    }
    want.is_none()
}

/* Detailed scoring */

/// Score filename and full path, weighting the filename twice as strongly. Queries containing a
/// slash use only the full path. Higher scores are better; None rejects, while an empty query
/// treats paths equally.
fn points(q: &[u8], all: &Corpus, span: &Span) -> Option<i32> {
    if q.is_empty() {
        return Some(0);
    }
    if q.contains(&b'/') {
        return fuzzy(q, all.bytes(span));
    }
    let on_path = fuzzy(q, all.bytes(span))?;
    // A filename match also matches the full path. Without one, retain only the full-path score.
    let name = all.name_bytes(span);
    let Some(on_name) = fuzzy(q, name) else {
        return Some(on_path);
    };
    Some(on_name * 2 + on_path / 4 + whole(q, name))
}

/// Prepare an individual path for scoring in tests, outside the hot loop.
#[cfg(test)]
fn score(query: &str, path: &str) -> Option<i32> {
    let all = corpus(vec![path.to_string()]);
    points(&query.to_ascii_lowercase().into_bytes(), &all, &all.at[0])
}

/// Reward matching the complete filename stem so user.rb ranks above a longer users directory.
/// Ignore the extension for this bonus.
fn whole(q: &[u8], name: &[u8]) -> i32 {
    let stem = match name.iter().rposition(|b| *b == b'.') {
        Some(at) if at > 0 => &name[..at],
        _ => name,
    };
    if stem.len() == q.len() && starts(q, stem) {
        return WHOLE;
    }
    match starts(q, stem) {
        true => STARTS,
        false => 0,
    }
}

/// Bonuses for the full filename stem and a matching filename prefix.
const WHOLE: i32 = 64;
const STARTS: i32 = 24;

/// Base reward for each matched byte.
const MATCH: i32 = 16;
/// Reward path or word boundaries after slash, underscore, hyphen, dot, or space, allowing initials
/// to locate nested paths.
const BOUNDARY: i32 = 8;
/// Treat a lowercase-to-uppercase transition as a word boundary in camelCase names.
const CAMEL: i32 = 6;
/// Reward consecutive matches.
const CONSEC: i32 = 8;
/// Penalize skipped bytes between matched positions.
const GAP: i32 = 1;
/// Bound scored text length because dynamic-programming cost grows with query length times path
/// length.
const LONGEST: usize = 260;

const NEVER: i32 = i32::MIN / 4;

/// Use dynamic programming to choose the best ordered match, rewarding boundaries and consecutive
/// bytes. Greedy matching can select an early weak occurrence and miss a later filename match.
/// Preserve original case for boundary detection, fold comparisons bytewise, and retain only two
/// matrix rows.
fn fuzzy(q: &[u8], text: &[u8]) -> Option<i32> {
    let n = text.len().min(LONGEST);
    if q.len() > n {
        return None;
    }

    // Compute the positional reward for matching at text index j.
    let place = |j: usize| -> i32 {
        if j == 0 {
            return BOUNDARY;
        }
        if matches!(text[j - 1], b'/' | b'_' | b'-' | b'.' | b' ') {
            return BOUNDARY;
        }
        match text[j - 1].is_ascii_lowercase() && text[j].is_ascii_uppercase() {
            true => CAMEL,
            false => 0,
        }
    };

    // exact[j] stores the best score ending at exactly j for the query prefix processed so far.
    let mut exact = vec![NEVER; n];
    for (i, want) in q.iter().enumerate() {
        let mut next = vec![NEVER; n];
        // carry tracks the best preceding match after deducting gaps to the current position.
        let mut carry = NEVER;
        for j in 0..n {
            if j > 0 {
                carry = carry.saturating_sub(GAP).max(exact[j - 1]);
            }
            if low(text[j]) != *want {
                continue;
            }
            next[j] = match i {
                // The first match may start anywhere without penalizing the path prefix.
                0 => MATCH + place(j),
                _ if carry <= NEVER => continue,
                _ => {
                    let apart = carry + MATCH + place(j);
                    let glued = match j > 0 && exact[j - 1] > NEVER {
                        true => exact[j - 1] + MATCH + place(j) + CONSEC,
                        false => NEVER,
                    };
                    apart.max(glued)
                }
            };
        }
        exact = next;
    }
    exact.into_iter().max().filter(|best| *best > NEVER)
}

/* Recent file relevance */

/// Boost only the most recent files, with descending weight. Normalize absolute and relative paths
/// against the workspace root and exclude paths outside it.
/// Normalize all recent paths relative to the workspace root.
fn under(root: &Path, recent: &[String]) -> HashMap<String, i32> {
    let mut out = HashMap::new();
    for (at, raw) in recent.iter().take(RECENT_MOST).enumerate() {
        let rel = match Path::new(raw).strip_prefix(root) {
            Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
            // Keep relative paths only when they do not traverse upward.
            Err(_) if !raw.starts_with('/') && !raw.starts_with("..") => raw.clone(),
            Err(_) => continue,
        };
        let points = RECENT - (at as i32) * RECENT_STEP;
        out.entry(rel).or_insert(points);
    }
    out
}

/// Limit recent-file boosts and keep their weight below a strong explicitly typed filename match.
const RECENT_MOST: usize = 12;
const RECENT: i32 = 120;
const RECENT_STEP: i32 = 8;

/* Path cache */

/// Serve cached paths immediately after expiry and refresh in the background. Large git ls-files
/// calls must not block typing; newly created files can appear on a subsequent request.
const FRESH: Duration = Duration::from_secs(30);

struct Shelf {
    at: Instant,
    /// Track last use so large inactive workspace indexes can be evicted.
    used: Instant,
    list: Arc<Corpus>,
    /// Avoid starting a second refresh while one is already running.
    filling: bool,
}

/// Retain only a small set of recently searched workspaces.
const KEEP: usize = 4;

/// Evict least recently used indexes until the cache fits its limit.
fn forget(cache: &mut HashMap<Vec<PathBuf>, Shelf>) {
    while cache.len() > KEEP {
        let Some(oldest) = cache
            .iter()
            .filter(|(_, shelf)| !shelf.filling)
            .min_by_key(|(_, shelf)| shelf.used)
            .map(|(id, _)| id.clone())
        else {
            return;
        };
        cache.remove(&oldest);
    }
}

type Cache = Mutex<HashMap<Vec<PathBuf>, Shelf>>;

/// Wait for the initial index only. Later requests return the cached index immediately, even while
/// refreshing it.
fn cached(
    shelf: &Arc<Cache>,
    id: &[PathBuf],
    make: impl FnOnce() -> Vec<String> + Send + 'static,
) -> Arc<Corpus> {
    {
        let mut cache = shelf.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(have) = cache.get_mut(id) {
            have.used = Instant::now();
            let list = Arc::clone(&have.list);
            if have.at.elapsed() >= FRESH && !have.filling {
                have.filling = true;
                let id = id.to_vec();
                let shelf = shelf.clone();
                std::thread::spawn(move || {
                    let fresh = Arc::new(corpus(make()));
                    let mut cache = shelf.lock().unwrap_or_else(|e| e.into_inner());
                    let used = cache
                        .get(&id)
                        .map(|old| old.used)
                        .unwrap_or_else(Instant::now);
                    cache.insert(
                        id,
                        Shelf {
                            at: Instant::now(),
                            used,
                            list: fresh,
                            filling: false,
                        },
                    );
                    forget(&mut cache);
                });
            }
            return list;
        }
    }

    let fresh = Arc::new(corpus(make()));
    let mut cache = shelf.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(
        id.to_vec(),
        Shelf {
            at: Instant::now(),
            used: Instant::now(),
            list: Arc::clone(&fresh),
            filling: false,
        },
    );
    forget(&mut cache);
    fresh
}

/// Use git ls-files for tracked and untracked files with ignore handling; scan non-Git folders
/// directly.
fn list(dir: &Path) -> Vec<String> {
    if dir.join(".git").exists() {
        // Use NUL separators so Git does not quote paths containing spaces or non-ASCII characters.
        return std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["ls-files", "-coz", "--exclude-standard"])
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default()
            .split('\0')
            .filter(|rel| !rel.is_empty())
            .map(str::to_string)
            .collect();
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out
}

/// Skip hidden folders and common generated directories in non-Git scans. ponytail: fixed
/// exclusions and a file limit; parse .gitignore if non-Git folders become a common use case.
const LOOSE_SKIP: [&str; 3] = ["node_modules", "target", "vendor"];
const LOOSE_MAX: usize = 20_000;

fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= LOOSE_MAX {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || LOOSE_SKIP.contains(&name.as_str()) {
            continue;
        }
        let rel = match prefix.is_empty() {
            true => name,
            false => format!("{prefix}/{name}"),
        };
        match entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            true => walk(&entry.path(), &rel, out),
            false => out.push(rel),
        }
    }
}

/// Add directory suggestions implied by indexed files because Git lists files only. Mark directory
/// entries with a trailing slash.
fn scan(root: &Path, repos: &[PathBuf]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let mut dirs: HashSet<String> = HashSet::new();

    let mut collect = |dir: &Path, prefix: &str| {
        for rel in list(dir) {
            let path = match prefix.is_empty() {
                true => rel,
                false => format!("{prefix}/{rel}"),
            };
            let mut at = 0;
            while let Some(slash) = path[at..].find('/') {
                at += slash + 1;
                dirs.insert(path[..at].to_string());
            }
            files.push(path);
        }
    };

    match repos.is_empty() {
        // Without registered repositories, scan the workspace directory directly.
        true => collect(root, ""),
        false => {
            for repo in repos {
                let dir = repo;
                // Single-repository paths need no repository prefix.
                let prefix = match dir.strip_prefix(root) {
                    Ok(rest) => rest.to_string_lossy().replace('\\', "/"),
                    Err(_) => String::new(),
                };
                collect(dir, &prefix);
            }
        }
    }

    files.extend(dirs);
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn injected_search_keeps_roots_and_host_caches_independent() {
        let root = std::env::temp_dir().join(format!("path-hosts-{}", uuid::Uuid::new_v4()));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("first.md"), "").unwrap();
        std::fs::write(second.join("second.md"), "").unwrap();
        let search = NativeSearch::default();
        assert_eq!(search.find(&first, &[], "", &[], true)[0].path, "first.md");
        assert_eq!(
            search.find(&second, &[], "", &[], true)[0].path,
            "second.md"
        );
        let multiple = search.find(&root, &[first.clone(), second], "", &[], true);
        assert_eq!(
            multiple
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            ["first/first.md", "second/second.md"]
        );
        std::fs::remove_file(first.join("first.md")).unwrap();
        std::fs::write(first.join("replacement.md"), "").unwrap();
        // A fresh host never inherits another host's still-fresh index.
        assert_eq!(
            NativeSearch::default().find(&first, &[], "", &[], true)[0].path,
            "replacement.md"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Test the complete filtering and scoring pipeline because candidate limits can discard a path
    /// before detailed scoring.
    fn best(query: &str, paths: &[&str]) -> Vec<String> {
        ranked(query, paths, &HashMap::new())
    }

    fn ranked(query: &str, paths: &[&str], fresh: &HashMap<String, i32>) -> Vec<String> {
        let all = corpus(paths.iter().map(|p| p.to_string()).collect());
        let q: Vec<u8> = query.to_ascii_lowercase().into_bytes();
        super::ranked(&all, &q, fresh, false)
            .into_iter()
            .map(|s| all.text(s).to_string())
            .collect()
    }

    #[test]
    fn filename_matches_outweigh_the_rest_of_the_path() {
        let all = ["app/user/legacy/parser.rb", "app/models/user.rb"];
        assert_eq!(
            best("user", &all),
            ["app/models/user.rb", "app/user/legacy/parser.rb"]
        );
    }

    #[test]
    fn word_prefixes_outweigh_interior_letters() {
        // Prefer ur at word boundaries in user_repo over interior matches in nature.
        assert!(fuzzy(b"ur", b"user_repo.rb") > fuzzy(b"ur", b"nature.rb"));
    }

    /// Path initials such as amtr can locate deeply nested files without typing each directory.
    #[test]
    fn path_initials_find_the_file() {
        assert!(score("amtr", "app/models/transcriber.rb").is_some());
    }

    /// Filename matches outrank matches scattered across path components, even at boundaries.
    #[test]
    fn filename_matches_outrank_path_matches() {
        let all = ["app/models/transcriber.rb", "lib/parametros.rb"];
        assert_eq!(best("amtr", &all)[0], "lib/parametros.rb");
    }

    #[test]
    fn contiguous_matches_outrank_scattered_letters() {
        let all = ["app/user.rb", "app/utilities/serializer.rb"];
        assert_eq!(best("user", &all)[0], "app/user.rb");
    }

    #[test]
    fn camel_case_creates_word_boundaries() {
        let all = ["src/userRepo.ts", "src/nature.ts"];
        assert_eq!(best("ur", &all)[0], "src/userRepo.ts");
    }

    #[test]
    fn does_not_greedily_accept_the_first_letter() {
        // Skip an early weak occurrence when a later filename provides the better match.
        assert!(score("usr", "under/models/user.rb").is_some());
        let all = ["under/models/user.rb", "under/models/superset.rb"];
        assert_eq!(best("usr", &all)[0], "under/models/user.rb");
    }

    #[test]
    fn slashes_search_the_entire_path() {
        let all = ["app/models/user.rb", "user_app/x.rb", "app/views/user.erb"];
        assert_eq!(best("app/mod", &all), ["app/models/user.rb"]);
    }

    #[test]
    fn excludes_paths_missing_query_letters() {
        assert!(score("zzz", "app/models/transcriber.rb").is_none());
    }

    /// A complete filename stem outranks a longer prefix match; extensions do not affect that
    /// bonus.
    #[test]
    fn exact_filenames_outrank_shared_prefixes() {
        let all = ["db/seeds/users/", "app/models/user.rb"];
        assert_eq!(best("user", &all)[0], "app/models/user.rb");
        assert!(whole(b"user", b"user.rb") > whole(b"user", b"users"));
    }

    #[test]
    fn empty_queries_match_everything() {
        assert_eq!(best("", &["b.rb", "a.rb"]), ["a.rb", "b.rb"]);
    }

    #[test]
    fn uppercase_does_not_prevent_matches() {
        assert!(score("readme", "README.md").is_some());
        assert!(score("CLAUDE", "CLAUDE.md").is_some());
    }

    /// Recent files gain weight only if their path still matches the query.
    #[test]
    fn recent_agent_edits_rank_higher() {
        let all = ["app/models/user.rb", "spec/models/user_spec.rb"];
        assert_eq!(best("user", &all)[0], "app/models/user.rb");

        let fresh = HashMap::from([("spec/models/user_spec.rb".to_string(), RECENT)]);
        assert_eq!(ranked("user", &all, &fresh)[0], "spec/models/user_spec.rb");
        assert_eq!(ranked("zzz", &all, &fresh), Vec::<String>::new());
    }

    /// Normalize absolute and relative recent paths while rejecting paths outside the workspace.
    #[test]
    fn recent_files_become_root_relative_paths() {
        let root = Path::new("/tmp/ws");
        let fresh = under(
            root,
            &[
                "/tmp/ws/app/models/user.rb".to_string(),
                "spec/user_spec.rb".to_string(),
                "/etc/hosts".to_string(),
            ],
        );
        assert_eq!(fresh.get("app/models/user.rb"), Some(&RECENT));
        assert_eq!(
            fresh.get("spec/user_spec.rb"),
            Some(&(RECENT - RECENT_STEP))
        );
        assert!(fresh.keys().all(|k| !k.contains("hosts")));
    }

    /// Verify tracked and untracked Git paths, ignored-file exclusion, and synthesized directory
    /// entries against a real repository.
    #[test]
    fn git_scans_include_known_files() {
        let root = std::env::temp_dir().join(format!("prometeu-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app/models")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        std::fs::write(root.join("app/models/user.rb"), "").unwrap();
        std::fs::write(root.join("node_modules/x/y.js"), "").unwrap();
        // Untracked files are included without git add through ls-files -co.
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .output()
            .unwrap();
        assert!(out.status.success());

        let found = scan(&root, &[]);
        assert!(found.contains(&"app/models/user.rb".to_string()));
        assert!(found.contains(&"app/".to_string()));
        assert!(found.contains(&"app/models/".to_string()));
        assert!(!found.iter().any(|p| p.contains("node_modules")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Non-Git scans skip hidden and generated directories.
    #[test]
    fn scans_directories_without_git() {
        let root = std::env::temp_dir().join(format!("prometeu-solta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app/models")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join(".cache")).unwrap();
        std::fs::write(root.join("app/models/user.rb"), "").unwrap();
        std::fs::write(root.join("node_modules/x/y.js"), "").unwrap();
        std::fs::write(root.join(".cache/z.bin"), "").unwrap();

        let found = scan(&root, &[]);
        assert!(found.contains(&"app/models/user.rb".to_string()));
        assert!(found.contains(&"app/models/".to_string()));
        assert!(!found.iter().any(|p| p.contains("node_modules")));
        assert!(!found.iter().any(|p| p.contains(".cache")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Strong matches must survive bounded filtering among thousands of weaker monorepo candidates.
    #[test]
    fn shortlisting_preserves_good_matches_among_poor_matches() {
        let mut paths: Vec<String> = (0..50_000)
            .map(|n| format!("vendor/lib{n}/user_helper_stub.rb"))
            .collect();
        paths.push("app/models/user.rb".to_string());
        let all = corpus(paths);
        let short = shortlist(&all, b"user", &HashMap::new(), false);
        assert!(short
            .iter()
            .any(|at| all.text(&all.at[*at]) == "app/models/user.rb"));
    }

    /// Recent files reach detailed scoring even when ordinary candidate trimming would discard
    /// them.
    #[test]
    fn shortlisting_reserves_space_for_recent_files() {
        let mut paths: Vec<String> = (0..50_000).map(|n| format!("app/user{n}.rb")).collect();
        paths.push("vendor/deep/nested/legacy/u_s_e_r.rb".to_string());
        let all = corpus(paths);
        let fresh = HashMap::from([("vendor/deep/nested/legacy/u_s_e_r.rb".to_string(), RECENT)]);
        let short = shortlist(&all, b"user", &fresh, false);
        assert!(short
            .iter()
            .any(|at| all.text(&all.at[*at]) == "vendor/deep/nested/legacy/u_s_e_r.rb"));
    }

    /// Quick open asks for files only; many better-ranked directories must not push a matching
    /// file past the candidate and row limits.
    #[test]
    fn files_only_keeps_directories_from_crowding_out_files() {
        let mut paths: Vec<String> = (0..5_000).map(|n| format!("user{n}/")).collect();
        paths.push("deep/nested/legacy/user_profile.rb".to_string());
        let all = corpus(paths);
        let with_dirs: Vec<&str> = super::ranked(&all, b"user", &HashMap::new(), false)
            .into_iter()
            .take(MOST)
            .map(|s| all.text(s))
            .collect();
        assert!(!with_dirs.contains(&"deep/nested/legacy/user_profile.rb"));
        let only: Vec<&Span> = super::ranked(&all, b"user", &HashMap::new(), true)
            .into_iter()
            .take(MOST)
            .collect();
        assert_eq!(only.len(), 1);
        assert!(!only[0].dir);
        assert_eq!(all.text(only[0]), "deep/nested/legacy/user_profile.rb");
    }

    /// Evict the least recently used workspace index to bound memory.
    #[test]
    fn shelf_caches_only_a_bounded_number_of_workspaces() {
        let mut cache: HashMap<Vec<PathBuf>, Shelf> = HashMap::new();
        for n in 0..KEEP + 3 {
            cache.insert(
                vec![PathBuf::from(format!("ws{n}"))],
                Shelf {
                    at: Instant::now(),
                    used: Instant::now() + Duration::from_secs(n as u64),
                    list: Arc::new(corpus(Vec::new())),
                    filling: false,
                },
            );
        }
        forget(&mut cache);
        assert_eq!(cache.len(), KEEP);
        // Retain the most recently used indexes.
        assert!(cache.contains_key(&vec![PathBuf::from(format!("ws{}", KEEP + 2))]));
        assert!(!cache.contains_key(&vec![PathBuf::from("ws0")]));
    }
}
