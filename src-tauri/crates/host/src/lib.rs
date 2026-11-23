//! Remote execution host reached over SSH (ADR 0101). The desktop keeps the board, the
//! conversation journal and the interface; this host owns the processes that must outlive the
//! desktop's connection: terminal agents, their hook relay and the transcript files they write.
//!
//! The wire format is NDJSON. A client sends `{"id", "method", "params"}` and receives
//! `{"id", "result"}` or `{"id", "error"}`; unsolicited frames carry an `event` field. The
//! complete contract is `docs/contracts/remote-host.md`.

/// Bumped on incompatible wire changes. The socket name carries it, so hosts speaking different
/// protocols never answer each other's clients.
pub const PROTOCOL: u32 = 1;

/// Directory under the remote home that holds installed builds, sockets and logs.
pub const ROOT: &str = ".prometeu-remote";

/// Every environment variable with this prefix travels from a hook process to the desktop, which
/// owns their meaning (`terminal_agent_hooks.rs`).
pub const HOOK_ENV_PREFIX: &str = "PROMETEU_TERMINAL_HOOK_";

/// The largest hook payload a native CLI may hand to its hook command.
pub const HOOK_INPUT_LIMIT: usize = 1_048_576;

/// One NDJSON line, in either direction.
pub const LINE_LIMIT: usize = 8 * 1_048_576;

pub fn socket_name() -> String {
    format!("host-v{PROTOCOL}.sock")
}

/// Claude stores a session transcript under `projects/<slug>/`, where the slug is the working
/// directory with every character outside `[A-Za-z0-9]` replaced by `-`.
pub fn claude_project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

/// systemd nests slices by dashes, so a workspace slice is a child of `prometeu.slice`. UUID
/// dashes are dropped to keep exactly one level.
pub fn workspace_slice(workspace: &str) -> Option<String> {
    let id: String = workspace.chars().filter(|ch| *ch != '-').collect();
    (!id.is_empty() && id.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .then(|| format!("prometeu-w{id}.slice"))
}

/// Identifiers that become file or unit names: terminals, watches and workspaces.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_slug_matches_native_directories() {
        assert_eq!(
            claude_project_slug("/home/me/.prometeu-remote/worktrees/app-feat.x"),
            "-home-me--prometeu-remote-worktrees-app-feat-x"
        );
    }

    #[test]
    fn workspace_slices_nest_once_under_prometeu() {
        assert_eq!(
            workspace_slice("6f1c-2a").as_deref(),
            Some("prometeu-w6f1c2a.slice")
        );
        assert_eq!(workspace_slice("../x"), None);
        assert_eq!(workspace_slice(""), None);
    }

    #[test]
    fn ids_never_name_paths() {
        assert!(valid_id("terminal-1_a"));
        assert!(!valid_id("../a"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
    }
}
