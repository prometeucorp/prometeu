use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    dirs::home_dir().expect("sem HOME")
}

/// Separate development and installed-app storage and worktree paths. Compile-time debug/release
/// selection prevents both apps from modifying the same board or transcripts.
fn suffix() -> &'static str {
    if cfg!(debug_assertions) {
        "-dev"
    } else {
        ""
    }
}

/// Root for application-owned data outside user repositories.
pub fn root() -> PathBuf {
    if let Ok(p) = std::env::var("PROMETEU_ROOT") {
        return PathBuf::from(p);
    }
    home().join(format!(".prometeu{}", suffix()))
}

/// Store team configuration and credentials privately, outside frontend localStorage. The frontend
/// owns relay transport.
pub fn team_path() -> PathBuf {
    root().join("team.json")
}

pub use prometeu_files::private::{ensure_private_dir, write_private, write_private_bytes};

/// Keep editor-visible worktrees outside the private state directory, with distinct
/// development/release roots to prevent path and transcript collisions.
pub fn worktree_dir(repo_name: &str, branch: &str) -> PathBuf {
    home()
        .join("prometeu")
        .join(format!("worktrees{}", suffix()))
        .join(repo_name)
        .join(dir_name(branch))
}

/// Group multi-repository worktrees under a directory named for all repositories, with each clone
/// name as a child, keeping it distinct from single-repository paths.
pub fn multi_dir(names: &[String], branch: &str) -> PathBuf {
    home()
        .join("prometeu")
        .join(format!("worktrees{}", suffix()))
        .join(names.join("+"))
        .join(dir_name(branch))
}

/// Calculate the exact legacy Prometheus root only for imported-workspace validation. Prometeu
/// never creates directories there.
pub(crate) fn prometheus_multi_dir(names: &[String], branch: &str) -> PathBuf {
    home()
        .join("prometheus")
        .join("worktrees")
        .join(names.join("+"))
        .join(dir_name(branch))
}

/// Flatten branch slashes for readable directory names, adding a stable suffix when flattening
/// would collide with an already flattened branch. Names without slashes remain unchanged for
/// compatibility.
fn dir_name(branch: &str) -> String {
    let flat = branch.replace('/', "-");
    match flat == branch {
        true => flat,
        false => format!("{flat}-{:06x}", fnv1a(branch) & 0xff_ffff),
    }
}

/// Use stable non-cryptographic FNV-1a for persisted path suffixes and deterministic port selection
/// without another dependency.
pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Claude derives transcript directories from the sanitized working path and creates files only
/// after the first message. Empty sessions therefore cannot resume. Codex's canonical app
/// transcript lives under paths::chat_log, separate from its native rollout. Both survive worktree
/// deletion.
pub fn chat_log(id: &str) -> PathBuf {
    root().join("chats").join(format!("{id}.jsonl"))
}

pub fn transcript(id: &str, cwd: &Path) -> PathBuf {
    // Account profiles share projects, so account changes preserve existing conversation identities
    // and paths.
    transcript_in(&crate::claude::user_home().join("projects"), id, cwd)
}

fn transcript_in(projects: &Path, id: &str, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    projects.join(slug).join(format!("{id}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_replaces_non_alphanumeric_characters() {
        let path = transcript("abc", Path::new("/Users/ana/.prometeu/wt/x_1"));
        assert!(
            path.ends_with("-Users-ana--prometeu-wt-x-1/abc.jsonl"),
            "{}",
            path.display()
        );
    }

    /// Multi-repository grouping directories remain beside single-repository paths, with named
    /// child worktrees.
    #[test]
    fn multi_repository_directories_join_repository_names() {
        let dir = multi_dir(&["back".into(), "front".into()], "feat/x");
        let one = worktree_dir("back", "feat/x");
        assert_eq!(dir.parent().unwrap().file_name().unwrap(), "back+front");
        assert_eq!(dir.file_name(), one.file_name());
        assert_eq!(
            dir.parent().unwrap().parent(),
            one.parent().unwrap().parent()
        );
    }

    /// Preserve branch names without slashes and distinguish flattened slash-containing names from
    /// existing flat names.
    #[test]
    fn branches_with_slashes_do_not_collide_with_flattened_names() {
        assert_eq!(dir_name("feat-x"), "feat-x");
        assert_ne!(dir_name("feat/x"), dir_name("feat-x"));
        assert!(
            dir_name("feat/x").starts_with("feat-x-"),
            "{}",
            dir_name("feat/x")
        );
        // Path derivation must remain stable because the board persists its result.
        assert_eq!(dir_name("feat/x"), dir_name("feat/x"));
        assert_ne!(dir_name("a/b"), dir_name("a/c"));
    }

    #[cfg(unix)]
    #[test]
    fn private_state_uses_restricted_permissions_and_atomic_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("prometeu-private-{}", uuid::Uuid::new_v4()));
        let file = root.join("nested/state.json");
        write_private(&file, "first").unwrap();
        write_private(&file, "second").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "second");
        assert_eq!(
            std::fs::metadata(file.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
            1
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
