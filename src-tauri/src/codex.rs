//! Adapt Codex app-server JSON-RPC over stdio into canonical ConversationEventV1 events and
//! translate commands in the opposite direction. Persist the returned thread ID in
//! Tab.agent_session for resume. The app stores its rendered transcript separately from native
//! Codex rollouts. Implement /compact through thread/compact/start and /context from tokenUsage;
//! reject unsupported slash commands. Default sessions use approvalPolicy never and an unrestricted
//! sandbox. Enable default_mode_request_user_input so ordinary turns can ask questions. Link
//! remains independent of threads and AppHandle for protocol tests.

use crate::{accounts, agents, i18n, paths};
use prometeu_core::session::launch::Launch;
use prometeu_protocols::codex::{process_stderr, Start};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

mod account;
pub use account::{account_probe, login, user_home};

/// Prepare codex app-server configuration for a tab. Resume uses the previously returned thread ID; absence starts
/// a new conversation.
pub fn prepare(
    id: &str,
    workspace: &str,
    worktree: &Path,
    resume: Option<String>,
    launch: &Launch,
    profiles: &dyn prometeu_profiles::ProfileBackend,
    tools: &dyn prometeu_tools::StartupTools,
) -> Result<crate::agent_launch::PreparedAgent, String> {
    let profile = profiles.resolve(&accounts::selected(crate::state::ProviderId::Codex)?)?;
    profiles.prepare(&profile)?;
    // Standalone skills ride the plugin-package pipeline, so Codex materializes them together with
    // the selected plugins.
    let packages = launch.plugin_packages();
    let selected_plugins = tools.codex_plugins(
        launch.config_scope.as_deref().unwrap_or(workspace),
        packages.as_deref(),
        &profile,
    )?;
    let mut cmd = Command::new("codex");
    profiles.apply(&profile, &mut cmd)?;
    cmd.args(["app-server", "--enable", "default_mode_request_user_input"]);
    cmd.args(["-c", "suppress_unstable_features_warning=true"]);
    if let Some(home) = &selected_plugins.home {
        cmd.env("CODEX_HOME", home);
    }
    // Sessions without plugins must not require newer plugin features, preserving conversation and
    // MCP support on older installations.
    if !selected_plugins.ids.is_empty() {
        cmd.args(["--enable", "plugins", "--enable", "hooks"]);
    }
    cmd.current_dir(worktree);
    // Inject selected MCP configuration through -c and process environment secrets; see
    // mcp::codex_config. Materialization failure must stop startup rather than silently omit
    // selected tools.
    if let Some((servers, env)) = tools.codex_mcp(id, launch.mcp.as_deref())? {
        cmd.args(["-c", &format!("mcp_servers={servers}")]);
        for (key, value) in env {
            cmd.env(key, value);
        }
    }
    let log = paths::chat_log(id);
    let resumed = resume.is_some();
    let start = Start {
        cwd: worktree.display().to_string(),
        resume,
        model: launch.model.trim().to_string(),
        effort: agents::effort(&launch.effort).to_string(),
        plugin_ids: selected_plugins.ids,
        plugin_hook_ids: selected_plugins.hook_ids,
        permission: launch.permission,
        instructions: launch.instructions.clone(),
    };
    Ok(crate::agent_launch::PreparedAgent {
        command: cmd,
        store: Arc::new(crate::transcript_store::FileTranscriptStore::new(
            log.clone(),
            log,
        )),
        profile,
        connect: Box::new(move |stdin, _| {
            prometeu_protocols::codex::connect(stdin, start, i18n::pick, env!("CARGO_PKG_VERSION"))
        }),
        stderr_line: process_stderr,
        spawn_error: "err.codex.spawn",
        resumed,
    })
}
