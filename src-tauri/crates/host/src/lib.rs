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

pub mod naming;
pub mod transcript;

/// Hook variables the host sets for the conversations it launches itself. A desktop's relay
/// replaces the socket, token and nonce with its own bridge's, so the host only needs to name the
/// conversation and its provider (`terminal_agent_hooks.rs`).
pub const HOOK_SESSION_ENV: &str = "PROMETEU_TERMINAL_HOOK_SESSION";
pub const HOOK_PROVIDER_ENV: &str = "PROMETEU_TERMINAL_HOOK_PROVIDER";
pub const HOOK_TOKEN_ENV: &str = "PROMETEU_TERMINAL_HOOK_TOKEN";
pub const HOOK_NONCE_ENV: &str = "PROMETEU_TERMINAL_HOOK_NONCE";

/// The longest a Claude `PermissionRequest` hook may wait for a decision.
pub const DECISION_WAIT_SECS: u64 = 300;

/// Claude's settings layer that routes every observed hook to `helper`. The desktop and the host
/// both launch Claude on a machine, and they must configure it identically so either can observe
/// a conversation the other started (ADR 0103).
pub fn claude_hook_settings(helper: &str) -> serde_json::Value {
    let hooks = [
        "SessionStart",
        "UserPromptSubmit",
        "Stop",
        "PermissionRequest",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "SessionEnd",
        "StopFailure",
        "Notification",
    ]
    .into_iter()
    .map(|name| {
        let timeout = if name == "PermissionRequest" {
            DECISION_WAIT_SECS + 5
        } else {
            3
        };
        (
            name.to_string(),
            serde_json::json!([{"hooks":[{"type":"command","command":helper,"timeout":timeout}]}]),
        )
    })
    .collect::<serde_json::Map<_, _>>();
    // This terminal is owned by Prometeu. Claude also gates its native OSC 7501 capability
    // on this title preference. A private invocation layer enables the status channel without
    // modifying user settings; the verified native bool parser treats "0" as false.
    // In this verified build eager flush awaits the transcript write queue before the engine's
    // final result reaches the TUI. The observer still requires persisted final content.
    serde_json::json!({"hooks": hooks, "env": {"CLAUDE_CODE_DISABLE_TERMINAL_TITLE": "0", "CLAUDE_CODE_EAGER_FLUSH": "1"}})
}

/// Stable non-cryptographic FNV-1a, used for persisted path suffixes.
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// A branch's worktree directory name: slashes flattened, plus a stable suffix when flattening
/// changed the name so flattened branches never collide. Names without slashes stay unchanged.
pub fn worktree_dir_name(branch: &str) -> String {
    let flat = branch.replace('/', "-");
    match flat == branch {
        true => flat,
        false => format!("{flat}-{:06x}", fnv1a(branch) & 0xff_ffff),
    }
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

    #[test]
    fn worktree_names_flatten_branches_with_a_stable_suffix() {
        assert_eq!(worktree_dir_name("main"), "main");
        let name = worktree_dir_name("prometeu/cipreste-sereno-2081");
        assert!(name.starts_with("prometeu-cipreste-sereno-2081-"));
        assert_eq!(name.len(), "prometeu-cipreste-sereno-2081-".len() + 6);
        assert_ne!(name, worktree_dir_name("prometeu-cipreste/sereno-2081"));
    }

    #[test]
    fn claude_settings_route_every_hook_to_the_helper() {
        let settings = claude_hook_settings("'/h' hook");
        let stop = &settings["hooks"]["Stop"][0]["hooks"][0];
        assert_eq!(stop["command"], "'/h' hook");
        assert_eq!(stop["timeout"], 3);
        assert_eq!(
            settings["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"],
            DECISION_WAIT_SECS + 5
        );
    }
}
