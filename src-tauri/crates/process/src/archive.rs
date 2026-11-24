//! Foreground archive hooks for desktop lifecycle commands.
use std::{
    path::Path,
    process::{Command, ExitStatus, Stdio},
};

/// Return only after the hook exits, so the caller may safely offer worktree deletion.
/// Hooks must keep resource teardown in the foreground, rather than detach it.
pub fn run(
    workdir: &Path,
    command: &str,
    environment: impl IntoIterator<Item = (String, String)>,
) -> std::io::Result<ExitStatus> {
    Command::new("/bin/sh")
        .args(["-lc", command])
        .current_dir(workdir)
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_returns_only_after_the_hook_finishes_using_the_worktree() {
        let root = std::env::temp_dir().join(format!("prometeu-archive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("resource"), "resource configuration").unwrap();
        let status = run(
            &root,
            "sleep 0.1; cat resource > finished; printf '%s' \"$ARCHIVE_TEST\" >> finished; exit 7",
            [("ARCHIVE_TEST".into(), " released".into())],
        )
        .unwrap();
        // A nonzero exit still means the hook has finished; callers retain their exit policy.
        assert_eq!(status.code(), Some(7));
        assert_eq!(
            std::fs::read_to_string(root.join("finished")).unwrap(),
            "resource configuration released"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
