//! Explicit, bounded Git effects. The host owns immutable grants, validation/approval evidence,
//! resource serialization and durable pre-effect intents; this adapter rechecks their concrete tree.
use super::{
    adapters,
    workspace::{self, Reservation},
};
use prometeu_core::command::CommandRunner;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommitResult {
    pub head_sha: String,
    pub changed_files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublishResult {
    pub head_sha: String,
    pub repository: String,
    pub branch: String,
}

fn paths(
    runner: &dyn CommandRunner<Command>,
    root: &Path,
    args: &[&str],
) -> Result<BTreeSet<String>, String> {
    let output = workspace::command(runner, root, args)?;
    if !output.success {
        return Err("automation_publication_git_failed".into());
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| "automation_publication_path_encoding")?;
    let mut result = BTreeSet::new();
    for name in text.split('\0').filter(|s| !s.is_empty()) {
        let path = Path::new(name);
        if !path.components().all(|p| matches!(p, Component::Normal(_)))
            || path.components().any(|p| p.as_os_str() == ".git")
        {
            return Err("automation_publication_path_invalid".into());
        }
        result.insert(name.to_owned());
        if result.len() > 50_000 {
            return Err("automation_publication_too_many_files".into());
        }
    }
    Ok(result)
}

/// Content fingerprint survives validation artifacts and a commit. The host separately binds the
/// exact local HEAD. Ignored build artifacts are excluded; tracked deletions and executable bits
/// are included, and symlinks/hardlinks cannot expose files outside the owned checkout.
pub(super) fn source_fingerprint(
    reservation: &Reservation,
    runner: &dyn CommandRunner<Command>,
) -> Result<String, String> {
    source_fingerprint_at(&reservation.path, runner)
}

pub(super) fn source_fingerprint_at(
    root: &Path,
    runner: &dyn CommandRunner<Command>,
) -> Result<String, String> {
    let files = paths(
        runner,
        root,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    for name in files {
        let path = root.join(&name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                hasher.update((name.len() as u64).to_le_bytes());
                hasher.update(name.as_bytes());
                if !meta.is_file()
                    || meta.file_type().is_symlink()
                    || std::fs::canonicalize(&path).ok().as_ref() != Some(&path)
                {
                    return Err("automation_publication_unsafe_file".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.nlink() != 1 {
                        return Err("automation_publication_unsafe_file".into());
                    }
                    hasher.update((meta.mode() & 0o111).to_le_bytes());
                }
                total = total
                    .checked_add(meta.len())
                    .ok_or("automation_publication_size")?;
                if total > 64 * 1024 * 1024 {
                    return Err("automation_publication_size".into());
                }
                let bytes =
                    std::fs::read(&path).map_err(|_| "automation_publication_file_changed")?;
                if bytes.len() as u64 != meta.len() {
                    return Err("automation_publication_file_changed".into());
                }
                hasher.update([1]);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Deleted paths contribute nothing, matching the source tree after commit.
            }
            Err(_) => return Err("automation_publication_file_unavailable".into()),
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn verify_content(
    reservation: &Reservation,
    expected: &str,
    runner: &dyn CommandRunner<Command>,
) -> Result<(), String> {
    if expected.len() != 64 || source_fingerprint(reservation, runner)? != expected {
        return Err(
            "automation_publication_validation_stale: Source files changed after validation".into(),
        );
    }
    Ok(())
}

/// Commit only this run's validated changes. Any interruption preserves the index and worktree
/// for inspection; a durable host intent prevents automatic replay of an ambiguous commit.
pub(super) fn commit(
    reservation: &Reservation,
    expected_head: &str,
    expected_source_fingerprint: &str,
    message: &str,
    runner: &dyn CommandRunner<Command>,
) -> Result<CommitResult, String> {
    if message.trim().is_empty() || message.len() > 16 * 1024 || message.contains('\0') {
        return Err("automation_commit_message_invalid".into());
    }
    workspace::verify_at_head(reservation, expected_head, runner)?;
    let root = &reservation.path;
    let git_dir = PathBuf::from(workspace::git(
        runner,
        root,
        &["rev-parse", "--absolute-git-dir"],
    )?);
    // Both locks are acquired before rechecking HEAD. Other Git checkout/commit operations must
    // wait or fail; our mutation always targets the reserved branch explicitly, never HEAD.
    let _head_lock = GitLock::acquire(git_dir.join("HEAD.lock"))?;
    let mut index_lock = GitLock::acquire(git_dir.join("index.lock"))?;
    workspace::verify_at_head(reservation, expected_head, runner)?;
    verify_content(reservation, expected_source_fingerprint, runner)?;
    // Staging must not execute a project-configured clean/process filter outside worker policy.
    let filters = workspace::command(
        runner,
        root,
        &["config", "--get-regexp", "^filter\\..*\\.(clean|process)$"],
    )?;
    if filters.success && !filters.stdout.is_empty() {
        return Err("automation_commit_filters_unsupported".into());
    }
    if !paths(
        runner,
        root,
        &[
            "diff",
            "--cached",
            "--no-ext-diff",
            "--name-only",
            "-z",
            "--",
        ],
    )?
    .is_empty()
    {
        return Err(
            "automation_commit_index_changed: Existing staged changes need inspection".into(),
        );
    }
    let mut files = paths(
        runner,
        root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-renames",
            "--name-only",
            "-z",
            "HEAD",
            "--",
        ],
    )?;
    files.extend(paths(
        runner,
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )?);
    if files.is_empty() {
        return Err("automation_commit_no_changes".into());
    }
    if files.len() > 4096 || files.iter().map(String::len).sum::<usize>() > 256 * 1024 {
        return Err("automation_commit_too_many_changes".into());
    }
    let temporary = TemporaryIndex::create(&git_dir)?;
    index_git(runner, root, &temporary.path, &["read-tree", expected_head])?;
    let mut args = vec!["--literal-pathspecs", "add", "--"];
    args.extend(files.iter().map(String::as_str));
    index_git(runner, root, &temporary.path, &args)?;
    verify_content(reservation, expected_source_fingerprint, runner)?;
    workspace::verify_at_head(reservation, expected_head, runner)?;
    let tree = index_git(runner, root, &temporary.path, &["write-tree"])?;
    let head_sha = workspace::git(
        runner,
        root,
        &[
            "-c",
            "commit.gpgSign=false",
            "commit-tree",
            &tree,
            "-p",
            expected_head,
            "-m",
            message,
        ],
    )?;
    // An unexpected branch change leaves at most an unreachable commit object, never a commit on
    // the person's new branch. The reference compare-and-swap also rejects a moved reserved ref.
    workspace::verify_at_head(reservation, expected_head, runner)?;
    verify_content(reservation, expected_source_fingerprint, runner)?;
    index_lock.copy_from(&temporary.path)?;
    let reference = format!("refs/heads/{}", reservation.branch);
    // Execute from the registered clone so Git does not try to relock this worktree's HEAD to
    // append its reflog. The explicit reserved branch still gets its own normal CAS and reflog.
    workspace::git(
        runner,
        &reservation.source,
        &[
            "update-ref",
            "-m",
            "automation validated commit",
            &reference,
            &head_sha,
            expected_head,
        ],
    )?;
    index_lock.install(&git_dir.join("index"))?;
    workspace::verify_at_head(reservation, &head_sha, runner)?;
    if workspace::git(runner, root, &["rev-parse", &format!("{head_sha}^")])? != expected_head {
        return Err("automation_commit_parent_changed".into());
    }
    verify_content(reservation, expected_source_fingerprint, runner)?;
    if !workspace::git(
        runner,
        root,
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err("automation_commit_tree_changed".into());
    }
    Ok(CommitResult {
        head_sha,
        changed_files: files.into_iter().collect(),
    })
}

/// Publish only an exact validated commit to the original same-repository PR head. Forks fail
/// closed: the base repository URL must never be used as a guessed fork publication destination.
pub(super) fn publish(
    reservation: &Reservation,
    expected_head: &str,
    expected_source_fingerprint: &str,
    runner: &dyn CommandRunner<Command>,
) -> Result<PublishResult, String> {
    let token_override = ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()));
    publish_inner(
        reservation,
        expected_head,
        expected_source_fingerprint,
        runner,
        token_override,
    )
}

fn publish_inner(
    reservation: &Reservation,
    expected_head: &str,
    expected_source_fingerprint: &str,
    runner: &dyn CommandRunner<Command>,
    token_override: bool,
) -> Result<PublishResult, String> {
    let pr = reservation
        .github
        .as_ref()
        .ok_or("automation_publish_pull_request_required")?;
    if pr.identity.is_empty()
        || !pr
            .identity
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("automation_publish_identity_invalid".into());
    }
    if token_override {
        return Err("automation_publish_token_auth_unsupported: Publication requires the saved GitHub CLI account, without token environment overrides".into());
    }
    workspace::require_no_filters(reservation, runner)?;
    workspace::verify_at_head(reservation, expected_head, runner)?;
    verify_content(reservation, expected_source_fingerprint, runner)?;
    let root = &reservation.path;
    if !workspace::git(
        runner,
        root,
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err("automation_publish_uncommitted_changes".into());
    }
    if !workspace::command(
        runner,
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &reservation.starting_sha,
            expected_head,
        ],
    )?
    .success
    {
        return Err("automation_publish_not_fast_forward".into());
    }
    let reference = format!("refs/heads/{}", pr.head_branch);
    workspace::git(runner, root, &["check-ref-format", &reference])?;
    let url = format!("https://github.com/{}.git", pr.repository);
    let pull_url = format!("https://github.com/{}/pull/{}", pr.repository, pr.number);
    adapters::require_identity(runner, &pr.identity)?;
    let current = adapters::gh(
        runner,
        &[
            "pr",
            "view",
            &pull_url,
            "--json",
            "headRefOid,headRefName,isCrossRepository,state",
        ],
    )?;
    if current["isCrossRepository"].as_bool() != Some(false) {
        return Err("automation_publish_fork_unsupported: Publication requires a PR branch in the selected repository".into());
    }
    if current["headRefOid"].as_str() != Some(&reservation.starting_sha)
        || current["headRefName"].as_str() != Some(&pr.head_branch)
        || current["state"].as_str() != Some("OPEN")
    {
        return Err("automation_publish_remote_changed".into());
    }
    // Use only github.com and the saved gh account; no remote name or URL comes from model output.
    let rewrites = workspace::command(
        runner,
        root,
        &[
            "config",
            "--get-regexp",
            "^url\\..*\\.(insteadof|pushinsteadof)$",
        ],
    )?;
    if rewrites.success && !rewrites.stdout.is_empty() {
        return Err("automation_publish_url_rewrite_unsupported".into());
    }
    let username = format!("credential.https://github.com.username={}", pr.identity);
    let transport = [
        "-c",
        &username,
        "-c",
        "credential.helper=",
        "-c",
        "credential.helper=!gh auth git-credential",
        "-c",
        "protocol.allow=never",
        "-c",
        "protocol.https.allow=always",
        "-c",
        "protocol.file.allow=never",
        "-c",
        "protocol.ext.allow=never",
    ];
    let mut query = transport.to_vec();
    query.extend(["ls-remote", "--exit-code", "--refs", "--", &url, &reference]);
    let remote = transport_git(runner, root, &query)?;
    let mut lines = remote.lines();
    let expected_remote = format!("{}\t{reference}", reservation.starting_sha);
    if lines.next() != Some(expected_remote.as_str()) || lines.next().is_some() {
        return Err("automation_publish_remote_changed".into());
    }
    workspace::verify_at_head(reservation, expected_head, runner)?;
    verify_content(reservation, expected_source_fingerprint, runner)?;
    let refspec = format!("{expected_head}:{reference}");
    let lease = format!(
        "--force-with-lease={reference}:{}",
        reservation.starting_sha
    );
    let mut push = transport.to_vec();
    push.extend([
        "push",
        "--porcelain",
        "--no-verify",
        &lease,
        "--",
        &url,
        &refspec,
    ]);
    transport_git(runner, root, &push)?;
    Ok(PublishResult {
        head_sha: expected_head.into(),
        repository: pr.repository.clone(),
        branch: pr.head_branch.clone(),
    })
}

fn transport_git(
    runner: &dyn CommandRunner<Command>,
    root: &Path,
    args: &[&str],
) -> Result<String, String> {
    let output = workspace::command_with_env(
        runner,
        root,
        args,
        &[
            ("GH_TOKEN", None),
            ("GITHUB_TOKEN", None),
            ("GH_HOST", Some("github.com")),
        ],
    )?;
    if !output.success {
        return Err("automation_publish_transport_failed: Check the saved GitHub CLI account and the unchanged PR head".into());
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "automation_publish_transport_response".into())
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;

struct GitLock {
    path: Option<PathBuf>,
    file: std::fs::File,
}
impl GitLock {
    fn acquire(path: PathBuf) -> Result<Self, String> {
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&path)
            .map_err(|_| "automation_commit_git_busy: Git is already changing this workspace")?;
        Ok(Self {
            path: Some(path),
            file,
        })
    }
    fn copy_from(&mut self, source: &Path) -> Result<(), String> {
        let mut input =
            std::fs::File::open(source).map_err(|_| "automation_commit_index_unavailable")?;
        std::io::copy(&mut input, &mut self.file)
            .map_err(|_| "automation_commit_index_unavailable")?;
        self.file
            .sync_all()
            .map_err(|_| "automation_commit_index_unavailable".into())
    }
    fn install(&mut self, destination: &Path) -> Result<(), String> {
        std::fs::rename(
            self.path
                .as_ref()
                .ok_or("automation_commit_index_unavailable")?,
            destination,
        )
        .map_err(|_| "automation_commit_index_unavailable")?;
        self.path = None;
        Ok(())
    }
}
impl Drop for GitLock {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if !std::fs::symlink_metadata(path)
                    .ok()
                    .zip(self.file.metadata().ok())
                    .is_some_and(|(path, held)| {
                        path.dev() == held.dev() && path.ino() == held.ino()
                    })
                {
                    return;
                }
            }
            let _ = std::fs::remove_file(path);
        }
    }
}
struct TemporaryIndex {
    directory: PathBuf,
    path: PathBuf,
}
impl TemporaryIndex {
    fn create(git_dir: &Path) -> Result<Self, String> {
        let directory = git_dir.join(format!("automation-index-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).map_err(|_| "automation_commit_index_unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|_| "automation_commit_index_unavailable")?;
        }
        Ok(Self {
            path: directory.join("index"),
            directory,
        })
    }
}
impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.directory.join("index.lock"));
        let _ = std::fs::remove_dir(&self.directory);
    }
}
fn index_git(
    runner: &dyn CommandRunner<Command>,
    root: &Path,
    index: &Path,
    args: &[&str],
) -> Result<String, String> {
    let index = index
        .to_str()
        .ok_or("automation_publication_path_encoding")?;
    let output =
        workspace::command_with_env(runner, root, args, &[("GIT_INDEX_FILE", Some(index))])?;
    if !output.success {
        return Err("automation_commit_index_failed".into());
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| "automation_commit_index_response".into())
}
