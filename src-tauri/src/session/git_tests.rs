use super::*;

#[test]
fn concurrent_status_consumers_share_a_scan_and_retry_once_after_invalidation() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    let slot = Arc::new(Slot::<usize>::default());
    let count = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = {
        let slot = slot.clone();
        let count = count.clone();
        std::thread::spawn(move || {
            slot.read(Duration::from_secs(30), || {
                let call = count.fetch_add(1, Ordering::SeqCst) + 1;
                if call == 1 {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                }
                Ok(call)
            })
            .unwrap()
        })
    };
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let second = {
        let slot = slot.clone();
        let count = count.clone();
        std::thread::spawn(move || {
            slot.read(Duration::from_secs(30), || {
                Ok(count.fetch_add(1, Ordering::SeqCst) + 1)
            })
            .unwrap()
        })
    };
    slot.invalidate();
    let other = Slot::<usize>::default();
    assert_eq!(other.read(Duration::from_secs(30), || Ok(9)).unwrap(), 9);
    release_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap(), 2);
    assert_eq!(second.join().unwrap(), 2);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn saved_files_invalidate_immediately_and_external_edits_appear_after_fallback() {
    use std::time::Duration;
    let repo = Repository::new();
    repo.write("tracked.txt", "initial\n");
    repo.commit("initial");
    let first = cached_status(&repo.0, "main").unwrap();
    assert!(first.changes.is_empty());
    repo.write("tracked.txt", "changed\n");
    invalidate_path(&repo.0.join("tracked.txt"));
    let changed = cached_status(&repo.0, "main").unwrap();
    assert!(changed
        .changes
        .iter()
        .any(|file| file.path == "tracked.txt"));
    repo.write("external.txt", "external\n");
    std::thread::sleep(STATUS_TTL + Duration::from_millis(50));
    let external = cached_status(&repo.0, "main").unwrap();
    assert!(external
        .changes
        .iter()
        .any(|file| file.path == "external.txt"));
}

#[test]
fn a_commit_token_never_describes_an_older_files_scan() {
    let repo = Repository::new();
    repo.write("tracked.txt", "initial\n");
    repo.commit("initial");
    repo.write("secret.env", "token\n");
    // Files marks cache porcelain in which the new file is still untracked.
    assert!(raw_status(&repo.0).unwrap().contains("?? secret.env"));
    // An agent stages it without an app invalidation.
    repo.git(&["add", "secret.env"]);
    let shown = cached_status(&repo.0, "main").unwrap();
    assert_eq!(entries(&shown.staged), [("secret.env", "A")]);
    assert_eq!(shown.index, fingerprint(&repo.0).unwrap());
    // Files marks reuse the snapshot that Changes just read.
    assert!(raw_slot(&repo.0).fresh(PORCELAIN_TTL));
}

#[test]
fn a_failed_scan_reaches_its_waiters_but_not_later_readers() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    let ttl = Duration::from_secs(30);
    let slot = Arc::new(Slot::<usize>::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let first = {
        let slot = slot.clone();
        std::thread::spawn(move || {
            slot.read(ttl, || {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                Err("busy".to_string())
            })
        })
    };
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let scans = Arc::new(AtomicUsize::new(0));
    let waiter = {
        let slot = slot.clone();
        let scans = scans.clone();
        std::thread::spawn(move || {
            slot.read(ttl, || {
                scans.fetch_add(1, Ordering::SeqCst);
                Ok(1)
            })
        })
    };
    // Usually the waiter joins the running scan; if it arrives later it scans on its own, since
    // the error is never reused. Both interleavings must hold the same guarantees.
    std::thread::sleep(Duration::from_millis(50));
    release_tx.send(()).unwrap();
    assert_eq!(first.join().unwrap(), Err("busy".to_string()));
    match waiter.join().unwrap() {
        Err(error) => {
            assert_eq!(error, "busy");
            assert_eq!(scans.load(Ordering::SeqCst), 0);
            assert_eq!(slot.read(ttl, || Ok(7)), Ok(7));
        }
        Ok(value) => {
            assert_eq!(value, 1);
            assert_eq!(scans.load(Ordering::SeqCst), 1);
            assert_eq!(slot.read(ttl, || Ok(7)), Ok(1));
        }
    }
}

#[test]
fn unused_expired_slots_are_pruned() {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    let slots: Mutex<HashMap<&str, Arc<Slot<usize>>>> = Mutex::new(HashMap::new());
    let ttl = Duration::from_millis(20);
    slot(&slots, "removed", ttl).read(ttl, || Ok(1)).unwrap();
    let held = slot(&slots, "held", ttl);
    held.read(ttl, || Ok(2)).unwrap();
    std::thread::sleep(ttl * 2);
    slot(&slots, "current", ttl);
    let keys = lock(&slots).keys().copied().collect::<Vec<_>>();
    assert!(!keys.contains(&"removed"));
    assert!(keys.contains(&"held"));
    assert!(keys.contains(&"current"));
}

struct Repository(PathBuf);

impl Repository {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("prometeu-git-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let repo = Self(root);
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.name", "Git test"]);
        repo.git(&["config", "user.email", "git@example.test"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.git(&["config", "core.autocrlf", "false"]);
        repo.git(&[
            "config",
            "core.hooksPath",
            repo.0.join("no-hooks").to_str().unwrap(),
        ]);
        repo.git(&["config", "merge.conflictstyle", "merge"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        run(&self.0, args).unwrap_or_else(|error| panic!("git {args:?}: {error}"))
    }

    fn write(&self, path: &str, text: &str) {
        std::fs::write(self.0.join(path), text).unwrap();
    }

    fn read(&self, path: &str) -> String {
        std::fs::read_to_string(self.0.join(path)).unwrap()
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-qm", message]);
    }

    fn act(&self, operation: GitAction, paths: &[&str]) -> Result<(), String> {
        action(
            &self.0,
            operation,
            &paths
                .iter()
                .map(|path| path.to_string())
                .collect::<Vec<_>>(),
            None,
            None,
            None,
        )
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entries(files: &[GitFile]) -> Vec<(&str, &str)> {
    files
        .iter()
        .map(|file| (file.path.as_str(), file.status.as_str()))
        .collect()
}

#[test]
fn status_and_diffs_separate_index_worktree_and_untracked_paths() {
    let repo = Repository::new();
    repo.write("a.txt", "base\n");
    repo.write("gone.txt", "gone\n");
    repo.commit("base");
    repo.write("a.txt", "staged\n");
    repo.act(GitAction::Stage, &["a.txt"]).unwrap();
    repo.write("a.txt", "worktree\n");
    std::fs::remove_file(repo.0.join("gone.txt")).unwrap();
    repo.write("new ç\nfile.txt", "untracked\n");

    let value = status(&repo.0, "main").unwrap();
    assert!(value.has_head);
    assert_eq!(value.branch.as_deref(), Some("main"));
    assert_eq!(entries(&value.staged), [("a.txt", "M")]);
    assert_eq!(
        entries(&value.changes),
        [("a.txt", "M"), ("gone.txt", "D"), ("new ç\nfile.txt", "?")]
    );
    assert!(value.conflicts.is_empty());

    let staged = diff(&repo.0, DiffScope::Staged, Some("a.txt"), None).unwrap();
    assert_eq!(staged.files.len(), 1);
    assert!(staged.files[0].patch.contains("-base\n+staged"));
    let changes = diff(&repo.0, DiffScope::Changes, None, None).unwrap();
    assert_eq!(changes.files.len(), 3);
    let changed = changes
        .files
        .iter()
        .find(|file| file.path == "a.txt")
        .unwrap();
    assert!(changed.patch.contains("-staged\n+worktree"));
    assert!(
        changes
            .files
            .iter()
            .find(|file| file.path == "gone.txt")
            .unwrap()
            .deleted
    );
    let new = changes
        .files
        .iter()
        .find(|file| file.path == "new ç\nfile.txt")
        .unwrap();
    assert!(new.new_file);
    assert_eq!(new.added, 1);

    repo.act(GitAction::Unstage, &["a.txt"]).unwrap();
    assert_eq!(repo.read("a.txt"), "worktree\n");
    assert!(status(&repo.0, "main").unwrap().staged.is_empty());
    repo.act(GitAction::Stage, &["gone.txt"]).unwrap();
    assert_eq!(
        entries(&status(&repo.0, "main").unwrap().staged),
        [("gone.txt", "D")]
    );
    assert!(!repo.0.join("gone.txt").exists());
}

#[test]
fn unborn_branch_can_unstage_partial_content_without_changing_worktree() {
    let repo = Repository::new();
    repo.write("new.txt", "index\n");
    let initial = status(&repo.0, "").unwrap();
    assert!(!initial.has_head);
    assert!(history(&repo.0).unwrap().is_empty());
    repo.act(GitAction::Stage, &["new.txt"]).unwrap();
    repo.write("new.txt", "latest worktree\n");
    let partial = status(&repo.0, "").unwrap();
    assert_eq!(entries(&partial.staged), [("new.txt", "A")]);
    assert_eq!(entries(&partial.changes), [("new.txt", "M")]);
    assert!(diff(&repo.0, DiffScope::Staged, None, None).unwrap().files[0].new_file);
    repo.act(GitAction::Unstage, &["new.txt"]).unwrap();
    assert_eq!(repo.read("new.txt"), "latest worktree\n");
    let value = status(&repo.0, "").unwrap();
    assert!(value.staged.is_empty());
    assert_eq!(entries(&value.changes), [("new.txt", "?")]);
}

#[test]
fn commit_requires_reviewed_index_and_leaves_unstaged_content_untouched() {
    let repo = Repository::new();
    repo.write("a.txt", "base\n");
    repo.commit("base");
    let head = repo.git(&["rev-parse", "HEAD"]);
    repo.write("a.txt", "index\n");
    repo.act(GitAction::Stage, &["a.txt"]).unwrap();
    let reviewed = fingerprint(&repo.0).unwrap();
    repo.write("a.txt", "latest worktree\n");
    assert_eq!(fingerprint(&repo.0).unwrap(), reviewed);
    repo.write("b.txt", "another process staged this\n");
    repo.git(&["add", "--", "b.txt"]);
    let result = action(
        &repo.0,
        GitAction::Commit,
        &[],
        Some("reviewed commit"),
        Some(&reviewed),
        None,
    );
    assert_eq!(result.unwrap_err(), i18n::t("err.git.changed"));
    assert_eq!(repo.git(&["rev-parse", "HEAD"]), head);
    repo.write("untracked.txt", "keep\n");
    let reviewed = fingerprint(&repo.0).unwrap();
    action(
        &repo.0,
        GitAction::Commit,
        &[],
        Some("index only"),
        Some(&reviewed),
        None,
    )
    .unwrap();
    assert_eq!(repo.git(&["show", "HEAD:a.txt"]), "index\n");
    assert_eq!(repo.read("a.txt"), "latest worktree\n");
    assert_eq!(repo.read("untracked.txt"), "keep\n");
    assert_eq!(repo.git(&["log", "-1", "--format=%s"]).trim(), "index only");
    assert_eq!(
        entries(&status(&repo.0, "").unwrap().changes),
        [("a.txt", "M"), ("untracked.txt", "?")]
    );
}

#[test]
fn branch_comparison_and_commit_history_exclude_index_and_worktree() {
    let repo = Repository::new();
    repo.write("a.txt", "base\n");
    repo.commit("base");
    let base = revision(&repo.0, "HEAD").unwrap();
    repo.git(&["checkout", "-qb", "feature"]);
    repo.write("a.txt", "committed\n");
    repo.commit("feature");
    repo.write("a.txt", "staged\n");
    repo.act(GitAction::Stage, &["a.txt"]).unwrap();
    repo.write("a.txt", "worktree\n");
    repo.write("untracked.txt", "local\n");

    let comparison = diff(&repo.0, DiffScope::Compare, None, Some("main")).unwrap();
    assert_eq!(comparison.base, base);
    assert_eq!(comparison.files.len(), 1);
    assert!(comparison.files[0].patch.contains("-base\n+committed"));
    assert!(!comparison.files[0].dirty);
    let commits = history(&repo.0).unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].subject, "feature");
    let first = diff(&repo.0, DiffScope::Commit, None, Some(&commits[1].oid)).unwrap();
    assert_eq!(first.files.len(), 1);
    assert!(first.files[0].new_file);
    assert!(first.files[0].patch.contains("+base"));
}

#[test]
fn publish_and_push_use_explicit_branch_despite_global_push_settings() {
    let repo = Repository::new();
    let remote = Repository::new();
    let bare = remote.0.join("remote.git");
    remote.git(&["init", "--bare", "-q", bare.to_str().unwrap()]);
    repo.write("a.txt", "first\n");
    repo.commit("first");
    repo.git(&["config", "push.followTags", "true"]);
    repo.git(&[
        "-c",
        "tag.gpgsign=false",
        "tag",
        "-a",
        "private-before-publish",
        "-m",
        "private tag",
    ]);
    repo.git(&["remote", "add", "origin", bare.to_str().unwrap()]);
    assert_eq!(
        repo.act(GitAction::Push, &[]).unwrap_err(),
        i18n::t("err.git.upstream")
    );
    assert!(action(
        &repo.0,
        GitAction::Publish,
        &[],
        None,
        None,
        Some("unknown")
    )
    .is_err());
    action(&repo.0, GitAction::Publish, &[], None, None, Some("origin")).unwrap();
    assert_eq!(run(&bare, &["tag", "--list"]).unwrap(), "");
    assert_eq!(
        status(&repo.0, "main").unwrap().upstream.as_deref(),
        Some("origin/main")
    );

    repo.git(&["branch", "unrelated"]);
    repo.git(&["config", "push.default", "nothing"]);
    repo.git(&[
        "config",
        "remote.origin.push",
        "refs/heads/unrelated:refs/heads/unrelated",
    ]);
    repo.write("a.txt", "second\n");
    repo.commit("second");
    repo.git(&[
        "-c",
        "tag.gpgsign=false",
        "tag",
        "-a",
        "private-before-push",
        "-m",
        "private tag",
    ]);
    assert_eq!(status(&repo.0, "main").unwrap().ahead, 1);
    assert!(history(&repo.0).unwrap()[0].outgoing);
    repo.act(GitAction::Push, &[]).unwrap();
    assert_eq!(run(&bare, &["tag", "--list"]).unwrap(), "");
    assert_eq!(
        revision(&bare, "refs/heads/main").unwrap(),
        revision(&repo.0, "HEAD").unwrap()
    );
    assert!(revision(&bare, "refs/heads/unrelated").is_err());
    assert_eq!(status(&repo.0, "main").unwrap().ahead, 0);
    repo.write("a.txt", "uncommitted\n");
    assert_eq!(
        repo.act(GitAction::Pull, &[]).unwrap_err(),
        i18n::t("err.git.dirtyPull")
    );
    assert_eq!(repo.read("a.txt"), "uncommitted\n");
}

#[test]
fn paths_are_literal_and_cannot_escape_repository() {
    let repo = Repository::new();
    for path in ["*", ":(glob)*", "--all", "a[1].txt", "a1.txt"] {
        repo.write(path, "text\n");
    }
    repo.act(GitAction::Stage, &["*"]).unwrap();
    assert_eq!(entries(&status(&repo.0, "").unwrap().staged), [("*", "A")]);
    repo.act(GitAction::Stage, &[":(glob)*"]).unwrap();
    repo.act(GitAction::Stage, &["--all"]).unwrap();
    let staged = diff(&repo.0, DiffScope::Staged, Some(":(glob)*"), None).unwrap();
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].path, ":(glob)*");
    let changes = diff(&repo.0, DiffScope::Changes, Some("a[1].txt"), None).unwrap();
    assert_eq!(changes.files.len(), 1);
    assert_eq!(changes.files[0].path, "a[1].txt");

    for path in [
        "",
        "../outside",
        "/tmp/outside",
        "a/../../outside",
        ".git/config",
        "dir/.GiT/config",
        "bad\0path",
    ] {
        assert!(valid_path(&repo.0, path).is_err(), "accepted {path:?}");
        assert!(
            diff(&repo.0, DiffScope::Changes, Some(path), None).is_err(),
            "diff accepted {path:?}"
        );
    }
    #[cfg(unix)]
    {
        let outside = Repository::new();
        outside.write("secret.txt", "outside\n");
        std::os::unix::fs::symlink(&outside.0, repo.0.join("escape")).unwrap();
        assert!(valid_path(&repo.0, "escape/secret.txt").is_err());
        assert!(repo.act(GitAction::Stage, &["escape/secret.txt"]).is_err());
        assert_eq!(outside.read("secret.txt"), "outside\n");
    }
}

#[test]
fn conflict_sources_keep_whitespace_and_resolution_refuses_stale_text() {
    let repo = Repository::new();
    repo.write("conflict.txt", "base\n");
    repo.write("untouched.txt", "keep me\n");
    repo.commit("base");
    repo.git(&["checkout", "-qb", "other"]);
    repo.write("conflict.txt", "  theirs  \n\n");
    repo.commit("theirs");
    repo.git(&["checkout", "-q", "main"]);
    repo.write("conflict.txt", "  ours  \n\n");
    repo.commit("ours");
    assert!(run(&repo.0, &["merge", "--no-edit", "other"]).is_err());
    let value = status(&repo.0, "main").unwrap();
    assert_eq!(entries(&value.conflicts), [("conflict.txt", "U")]);
    assert!(value.staged.is_empty());
    assert!(value.changes.is_empty());
    let snapshot = conflict(&repo.0, "conflict.txt").unwrap();
    assert_eq!(snapshot.ours.as_deref(), Some("  ours  \n\n"));
    assert_eq!(snapshot.theirs.as_deref(), Some("  theirs  \n\n"));
    assert_eq!(repo.read("conflict.txt"), snapshot.current);
    assert!(resolve(
        &repo.0,
        "conflict.txt",
        &snapshot.current,
        "<<<<<<< main\nours\n=======\ntheirs\n>>>>>>> other\n"
    )
    .is_err());
    assert_eq!(repo.read("conflict.txt"), snapshot.current);
    repo.write("conflict.txt", "external edit\n");
    assert_eq!(
        resolve(&repo.0, "conflict.txt", &snapshot.current, "resolved\n").unwrap_err(),
        i18n::t("err.session.changed")
    );
    assert_eq!(repo.read("conflict.txt"), "external edit\n");
    resolve(&repo.0, "conflict.txt", "external edit\n", "resolved\n").unwrap();
    let value = status(&repo.0, "main").unwrap();
    assert!(value.conflicts.is_empty());
    assert_eq!(entries(&value.staged), [("conflict.txt", "M")]);
    assert_eq!(repo.git(&["show", ":conflict.txt"]), "resolved\n");
    assert_eq!(repo.read("untouched.txt"), "keep me\n");
}

#[test]
fn keeping_ours_can_finish_merge_without_staged_file_changes() {
    let repo = Repository::new();
    repo.write("conflict.txt", "base\n");
    repo.commit("base");
    repo.git(&["checkout", "-qb", "other"]);
    repo.write("conflict.txt", "theirs\n");
    repo.commit("theirs");
    let other = revision(&repo.0, "HEAD").unwrap();
    repo.git(&["checkout", "-q", "main"]);
    repo.write("conflict.txt", "ours\n");
    repo.commit("ours");
    let ours = revision(&repo.0, "HEAD").unwrap();
    assert!(run(&repo.0, &["merge", "--no-edit", "other"]).is_err());
    assert!(status(&repo.0, "main").unwrap().merging);
    let conflict = conflict(&repo.0, "conflict.txt").unwrap();
    resolve(
        &repo.0,
        "conflict.txt",
        &conflict.current,
        conflict.ours.as_deref().unwrap(),
    )
    .unwrap();
    let reviewed = status(&repo.0, "main").unwrap();
    assert!(reviewed.conflicts.is_empty());
    assert!(reviewed.staged.is_empty());
    assert!(reviewed.merging);
    action(
        &repo.0,
        GitAction::Commit,
        &[],
        Some("keep ours"),
        Some(&reviewed.index),
        None,
    )
    .unwrap();
    assert_eq!(
        repo.git(&["log", "-1", "--format=%P"]).trim(),
        format!("{ours} {other}")
    );
    assert!(!status(&repo.0, "main").unwrap().merging);
    assert_eq!(repo.read("conflict.txt"), "ours\n");
    assert!(repo.git(&["diff", "HEAD^", "HEAD"]).is_empty());
}

#[test]
fn discard_restores_the_index_version_and_removes_only_listed_untracked_files() {
    let repo = Repository::new();
    repo.write(".gitignore", "ignored.log\n");
    repo.write("a.txt", "base\n");
    repo.write("gone.txt", "gone\n");
    repo.write("only-staged.txt", "base\n");
    repo.commit("base");
    repo.write("a.txt", "staged\n");
    repo.act(GitAction::Stage, &["a.txt"]).unwrap();
    repo.write("a.txt", "worktree\n");
    repo.write("only-staged.txt", "staged\n");
    repo.act(GitAction::Stage, &["only-staged.txt"]).unwrap();
    std::fs::remove_file(repo.0.join("gone.txt")).unwrap();
    repo.write("new.txt", "untracked\n");
    repo.write("kept.txt", "untracked\n");
    repo.write("ignored.log", "ignored\n");

    assert_eq!(
        repo.act(GitAction::Discard, &[]),
        Err(i18n::t("err.git.selection"))
    );
    // A path without unstaged work has nothing to discard; staged content is not the target.
    assert_eq!(
        repo.act(GitAction::Discard, &["only-staged.txt"]),
        Err(i18n::t("err.git.changed"))
    );
    assert_eq!(
        repo.act(GitAction::Discard, &["ignored.log"]),
        Err(i18n::t("err.git.changed"))
    );
    assert!(repo.act(GitAction::Discard, &["../a.txt"]).is_err());

    repo.act(GitAction::Discard, &["a.txt", "gone.txt", "new.txt"])
        .unwrap();
    assert_eq!(repo.read("a.txt"), "staged\n");
    assert_eq!(repo.read("gone.txt"), "gone\n");
    assert!(!repo.0.join("new.txt").exists());
    assert_eq!(repo.read("kept.txt"), "untracked\n");
    assert_eq!(repo.read("ignored.log"), "ignored\n");
    let value = status(&repo.0, "main").unwrap();
    assert_eq!(
        entries(&value.staged),
        [("a.txt", "M"), ("only-staged.txt", "M")]
    );
    assert_eq!(entries(&value.changes), [("kept.txt", "?")]);
}

fn marks_of(root: &Path, repos: &[PathBuf]) -> Vec<(String, String)> {
    let mut marks: Vec<_> = tree_marks(root, repos)
        .into_iter()
        .map(|file| (file.path, file.status))
        .collect();
    marks.sort();
    marks
}

#[test]
fn tree_marks_list_new_files_one_by_one_without_ignored_ones() {
    let repo = Repository::new();
    repo.write("kept.txt", "one\n");
    repo.write("gone.txt", "one\n");
    repo.write("staged.txt", "one\n");
    repo.commit("start");
    repo.write("kept.txt", "two\n");
    std::fs::remove_file(repo.0.join("gone.txt")).unwrap();
    repo.write("new.txt", "new\n");
    repo.write("added.txt", "new\n");
    repo.git(&["add", "added.txt"]);
    repo.write("staged.txt", "two\n");
    repo.git(&["add", "staged.txt"]);
    // Staged as new, then removed from disk (`AD`).
    repo.write("vanished.txt", "new\n");
    repo.git(&["add", "vanished.txt"]);
    std::fs::remove_file(repo.0.join("vanished.txt")).unwrap();
    std::fs::create_dir_all(repo.0.join("fresh/deep")).unwrap();
    repo.write("fresh/deep/file.txt", "new\n");
    repo.write("fresh/.env", "secret\n");
    repo.write(".gitignore", ".env\n");

    assert_eq!(
        marks_of(&repo.0, std::slice::from_ref(&repo.0)),
        [
            (".gitignore", "A"),
            ("added.txt", "A"),
            ("fresh/deep/file.txt", "A"),
            ("gone.txt", "D"),
            ("kept.txt", "M"),
            ("new.txt", "A"),
            ("staged.txt", "M"),
            ("vanished.txt", "D"),
        ]
        .map(|(path, status)| (path.to_string(), status.to_string()))
    );
}

#[test]
fn tree_marks_are_relative_to_the_tree_root() {
    let repo = Repository::new();
    std::fs::create_dir_all(repo.0.join("api/src")).unwrap();
    repo.write("api/src/main.rs", "one\n");
    repo.write("top.txt", "one\n");
    repo.commit("start");
    repo.write("api/src/main.rs", "two\n");
    repo.write("top.txt", "two\n");

    // A project registered on a subfolder sees only its own changes, relative to itself.
    let sub = repo.0.join("api");
    assert_eq!(
        marks_of(&sub, std::slice::from_ref(&sub)),
        [("src/main.rs".to_string(), "M".to_string())]
    );
    // A grouping folder above several worktrees prefixes each repository's folder.
    let parent = repo.0.parent().unwrap();
    let name = repo.0.file_name().unwrap().to_string_lossy();
    assert_eq!(
        marks_of(parent, std::slice::from_ref(&repo.0)),
        [
            (format!("{name}/api/src/main.rs"), "M".to_string()),
            (format!("{name}/top.txt"), "M".to_string()),
        ]
    );
    // Outside Git there is nothing to mark.
    let plain = std::env::temp_dir().join(format!("prometeu-plain-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&plain).unwrap();
    assert!(tree_marks(&plain, std::slice::from_ref(&plain)).is_empty());
    std::fs::remove_dir_all(&plain).unwrap();
}

#[test]
fn tree_mark_prefers_conflicts_then_new_files() {
    assert_eq!(tree_mark("UU"), "U");
    assert_eq!(tree_mark("AA"), "U");
    assert_eq!(tree_mark("AM"), "A");
    assert_eq!(tree_mark("MD"), "D");
    // Staged as new, then deleted from disk: only a deleted row can show it.
    assert_eq!(tree_mark("AD"), "D");
    assert_eq!(tree_mark("D "), "D");
    assert_eq!(tree_mark("RM"), "M");
}

#[test]
fn restore_deleted_brings_files_and_folders_back_from_head() {
    let repo = Repository::new();
    std::fs::create_dir_all(repo.0.join("docs/deep")).unwrap();
    repo.write("docs/deep/a.md", "a\n");
    repo.write("docs/b.md", "b\n");
    repo.write("kept.md", "kept\n");
    repo.write("staged.md", "staged\n");
    repo.commit("start");
    std::fs::remove_dir_all(repo.0.join("docs")).unwrap();
    repo.git(&["rm", "-q", "staged.md"]);
    repo.write("kept.md", "edited\n");
    let repos = [repo.0.clone()];

    restore_deleted(&repo.0, &repos, "docs").unwrap();
    restore_deleted(&repo.0, &repos, "staged.md").unwrap();
    assert_eq!(repo.read("docs/deep/a.md"), "a\n");
    assert_eq!(repo.read("docs/b.md"), "b\n");
    assert_eq!(repo.read("staged.md"), "staged\n");

    // An entry still on disk is refused, so its edits cannot be thrown away.
    assert!(restore_deleted(&repo.0, &repos, "kept.md").is_err());
    assert_eq!(repo.read("kept.md"), "edited\n");
    assert!(restore_deleted(&repo.0, &repos, "../outside").is_err());
    assert_eq!(
        marks_of(&repo.0, &repos),
        [("kept.md".to_string(), "M".to_string())]
    );
}

#[test]
fn restore_deleted_finds_the_repository_under_a_grouping_folder() {
    let repo = Repository::new();
    repo.write("gone.md", "gone\n");
    repo.commit("start");
    std::fs::remove_file(repo.0.join("gone.md")).unwrap();
    let parent = repo.0.parent().unwrap();
    let name = repo.0.file_name().unwrap().to_string_lossy();

    restore_deleted(
        parent,
        std::slice::from_ref(&repo.0),
        &format!("{name}/gone.md"),
    )
    .unwrap();
    assert_eq!(repo.read("gone.md"), "gone\n");
}

/// Staged edits of a file that then left the disk come back as staged, not replaced by `HEAD`.
#[test]
fn restore_deleted_keeps_staged_edits() {
    let repo = Repository::new();
    repo.write("notes.md", "committed\n");
    repo.commit("start");
    repo.write("notes.md", "staged\n");
    repo.git(&["add", "notes.md"]);
    std::fs::remove_file(repo.0.join("notes.md")).unwrap();
    let repos = [repo.0.clone()];

    restore_deleted(&repo.0, &repos, "notes.md").unwrap();
    assert_eq!(repo.read("notes.md"), "staged\n");
    assert_eq!(
        repo.git(&["diff", "--cached", "--name-only"]).trim(),
        "notes.md"
    );
    assert_eq!(repo.git(&["diff", "--name-only"]).trim(), "");
}

/// A folder mixes both sources file by file: staged content where the index has it, `HEAD` where
/// the deletion was staged. A file only in the index, added and then removed from disk, returns.
#[test]
fn restore_deleted_mixes_index_and_head_inside_a_folder() {
    let repo = Repository::new();
    std::fs::create_dir_all(repo.0.join("docs")).unwrap();
    repo.write("docs/edited.md", "committed\n");
    repo.write("docs/removed.md", "removed\n");
    repo.commit("start");
    repo.write("docs/edited.md", "staged\n");
    repo.write("docs/added.md", "added\n");
    repo.git(&["add", "docs"]);
    repo.git(&["rm", "-q", "docs/removed.md"]);
    std::fs::remove_dir_all(repo.0.join("docs")).unwrap();
    let repos = [repo.0.clone()];

    restore_deleted(&repo.0, &repos, "docs").unwrap();
    assert_eq!(repo.read("docs/edited.md"), "staged\n");
    assert_eq!(repo.read("docs/added.md"), "added\n");
    assert_eq!(repo.read("docs/removed.md"), "removed\n");
    assert_eq!(
        repo.git(&["diff", "--cached", "--name-only"]).trim(),
        "docs/added.md\ndocs/edited.md"
    );
    assert!(restore_deleted(&repo.0, &repos, "never").is_err());
}
