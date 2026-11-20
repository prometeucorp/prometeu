//! Interactive CLI preparation. Native screens and input remain owned by each provider.
#[cfg(test)]
mod mcp_smoke;

use crate::{accounts, i18n, paths, state::ProviderId};
use prometeu_core::{
    actions::Permission,
    session::launch::{LaunchRequest, StartMode},
};
use prometeu_profiles::ProfileBackend;
use prometeu_protocols::catalog::{command_output, CatalogProcess};
use prometeu_tools::{CodexPlugins, StartupTools};
use serde_json::{json, Map, Value};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

pub struct TerminalProviders {
    pub profiles: Arc<dyn ProfileBackend>,
    pub tools: Arc<dyn StartupTools>,
}

impl prometeu_core::session::provider::ProviderPreparation<PreparedTerminalAgent>
    for TerminalProviders
{
    fn prepare(&self, request: &LaunchRequest) -> Result<PreparedTerminalAgent, String> {
        prepare(request, self.profiles.as_ref(), self.tools.as_ref())
    }
}

pub struct PreparedTerminalAgent {
    pub command: Command,
    /// Codex owns a private app-server, configured with hooks before its thread is opened.
    pub engine: Option<crate::terminal_agent_codex::NativeEnginePlan>,
    /// Explicit app-server command with the same invocation config, used only for hook discovery.
    pub hook_probe: Option<Command>,
    pub profile: accounts::Profile,
    pub resumed: bool,
    /// Known native identity. Providers that allocate it at startup report it through their hooks.
    pub provider_session: Option<String>,
    /// Providers without a system-instruction flag install these as native rules before spawning.
    pub initial_instructions: Option<String>,
}

impl PreparedTerminalAgent {
    /// Native SQLite stores are scoped by the captured effective account home and session id.
    pub fn antigravity_store_root(&self) -> Option<PathBuf> {
        if self.profile.provider != ProviderId::Antigravity {
            return None;
        }
        let home = self
            .command
            .get_envs()
            .find_map(|(key, value)| (key == "HOME").then_some(value))
            .flatten()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.profile.home.clone());
        home.is_absolute()
            .then(|| home.join(".gemini/antigravity-cli/conversations"))
    }

    /// Cursor hashes process.cwd(), so preparation captures the physical workspace once for
    /// the child, --workspace, transcript slug and store. Environment is captured as well.
    pub fn cursor_store_root(&self) -> Option<PathBuf> {
        if self.profile.provider != ProviderId::Cursor {
            return None;
        }
        let environment = |name: &str| {
            self.command
                .get_envs()
                .find_map(|(key, value)| (key == name).then_some(value))
                .flatten()
                .and_then(|value| value.to_str())
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        };
        let config = environment("CURSOR_CONFIG_DIR")
            .or_else(|| environment("XDG_CONFIG_HOME").map(|path| path.join("cursor")))
            .unwrap_or_else(|| {
                environment("HOME")
                    .unwrap_or_else(|| self.profile.home.clone())
                    .join(".cursor")
            });
        let config = lexical_absolute(&config)?;
        let cwd = lexical_absolute(self.command.get_current_dir()?)?;
        let digest = md5::compute(cwd.to_str()?.as_bytes());
        Some(config.join("chats").join(format!("{digest:x}")))
    }

    /// Storage roots captured from this account and the final invocation. Hook paths are still
    /// required to name the exact native session and pass the observer's no-follow file open.
    pub fn observation_roots(&self) -> Result<Vec<PathBuf>, String> {
        let environment = |name: &str| {
            self.command
                .get_envs()
                .find_map(|(key, value)| (key == name).then(|| value.map(PathBuf::from)))
                .flatten()
        };
        let home = || environment("HOME").unwrap_or_else(|| self.profile.home.clone());
        let roots = match self.profile.provider {
            ProviderId::Claude => vec![environment("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| self.profile.home.clone())
                .join("projects")],
            ProviderId::Codex => {
                let home = environment("CODEX_HOME").unwrap_or_else(|| self.profile.home.clone());
                vec![home.join("sessions"), home.join("archived_sessions")]
            }
            ProviderId::Antigravity => vec![home().join(".gemini/antigravity-cli/brain")],
            ProviderId::Cursor => {
                let cwd = self
                    .command
                    .get_current_dir()
                    .and_then(Path::to_str)
                    .ok_or_else(|| i18n::t("err.terminalAgent.unsupported"))?;
                // Cursor 2026.11.13 hook discovery uses homedir/.cursor independently of the
                // CURSOR_CONFIG_DIR and CURSOR_DATA_DIR configuration/artifact directories.
                let mut slug = String::new();
                for ch in cwd.chars() {
                    if ch.is_ascii_alphanumeric() {
                        slug.push(ch);
                    } else if !slug.is_empty() && !slug.ends_with('-') {
                        slug.push('-');
                    }
                }
                vec![home()
                    .join(".cursor/projects")
                    .join(slug.trim_end_matches('-'))
                    .join("agent-transcripts")]
            }
            ProviderId::RetiredGemini => return Err(i18n::t("err.provider.retired")),
        };
        let mut authorized = Vec::new();
        for root in roots {
            if !root.is_absolute()
                || !root
                    .components()
                    .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
            {
                return Err(i18n::t("err.terminalAgent.unsupported"));
            }
            // Account profiles deliberately share transcript directories with their base account.
            // Capture only this precise root's target; do not authorize the account home itself.
            let resolved = match std::fs::canonicalize(&root) {
                Ok(resolved) => Some(resolved),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(i18n::io(error)),
            };
            for path in std::iter::once(root).chain(resolved) {
                if !authorized.contains(&path) {
                    authorized.push(path);
                }
            }
        }
        Ok(authorized)
    }
}

fn lexical_absolute(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Normal(_) => resolved.push(component),
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) => return None,
        }
    }
    Some(resolved)
}

fn capture_cursor_environment(command: &mut Command, home: &Path) {
    for name in [
        "HOME",
        "CURSOR_CONFIG_DIR",
        "CURSOR_DATA_DIR",
        "XDG_CONFIG_HOME",
    ] {
        if command.get_envs().any(|(key, _)| key == name) {
            continue;
        }
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        } else if name == "HOME" {
            command.env(name, home);
        } else {
            command.env_remove(name);
        }
    }
}

pub fn prepare(
    request: &LaunchRequest,
    profiles: &dyn ProfileBackend,
    tools: &dyn StartupTools,
) -> Result<PreparedTerminalAgent, String> {
    let provider = request.settings.agent;
    let profile = profiles.resolve(&accounts::selected(provider)?)?;
    if matches!(provider, ProviderId::Claude | ProviderId::Codex) {
        profiles.prepare(&profile)?;
    }
    let mut prepared = match provider {
        ProviderId::Claude => prepare_claude(request, profile, profiles, tools),
        ProviderId::Codex => prepare_codex(request, profile, profiles, tools),
        ProviderId::Antigravity => prepare_antigravity(request, profile, profiles),
        ProviderId::Cursor => prepare_cursor(request, profile, profiles, tools),
        ProviderId::RetiredGemini => Err(i18n::t("err.provider.retired")),
    }?;
    drop_claude_vars(&mut prepared.command);
    if let Some(engine) = prepared.engine.as_mut() {
        drop_claude_vars(&mut engine.command);
    }
    if let Some(probe) = prepared.hook_probe.as_mut() {
        drop_claude_vars(probe);
    }
    Ok(prepared)
}

fn previous(request: &LaunchRequest) -> Option<String> {
    match &request.mode {
        StartMode::Fresh => None,
        StartMode::Resume { provider_session } => provider_session.clone(),
    }
}

fn help(program: &str) -> Result<String, String> {
    command_output(
        &prometeu_process::query::UnixQueryLauncher,
        // Antigravity's Go flag parser writes successful help to stderr. A fixed redirect keeps
        // the shared bounded query transport; the executable is a separate shell argument.
        Command::new("/bin/sh").args(["-c", "exec \"$1\" --help 2>&1", "prometeu-help", program]),
        Duration::from_secs(10),
    )
    .map_err(|_| i18n::t("err.terminalAgent.version"))
}

fn require_flags(help: &str, flags: &[&str]) -> Result<(), String> {
    if flags.iter().all(|flag| help.contains(flag)) {
        Ok(())
    } else {
        Err(i18n::t("err.terminalAgent.version"))
    }
}

fn prepare_claude(
    request: &LaunchRequest,
    profile: accounts::Profile,
    profiles: &dyn ProfileBackend,
    tools: &dyn StartupTools,
) -> Result<PreparedTerminalAgent, String> {
    let cwd = Path::new(&request.worktree);
    let resume = previous(request).or_else(|| {
        (matches!(request.mode, StartMode::Resume { .. })
            && paths::transcript(&request.session, cwd).exists())
        .then(|| request.session.clone())
    });
    if profile.managed && !crate::claude::account_status_at(&profile, cwd)?.connected {
        return Err(i18n::t("err.account.disconnected"));
    }
    let launch = &request.settings;
    if launch.mcp.is_some() && launch.mcp_inherits_base {
        crate::mcp::requires_connectors(crate::mcp::connectors().is_some())?;
    }
    let packages = launch.plugin_packages();
    let selected = tools.claude(
        &request.session,
        cwd,
        launch.mcp.as_deref(),
        packages.as_deref(),
    )?;
    let mut command = claude_command(request, resume.as_deref(), selected);
    profiles.apply(&profile, &mut command)?;
    Ok(PreparedTerminalAgent {
        command,
        engine: None,
        hook_probe: None,
        profile,
        resumed: resume.is_some(),
        provider_session: Some(resume.unwrap_or_else(|| request.session.clone())),
        initial_instructions: None,
    })
}

fn claude_command(
    request: &LaunchRequest,
    resume: Option<&str>,
    selected: prometeu_tools::ClaudeTools,
) -> Command {
    let launch = &request.settings;
    let mut command = Command::new("claude");
    command.current_dir(&request.worktree).args([
        if resume.is_some() {
            "--resume"
        } else {
            "--session-id"
        },
        resume.unwrap_or(&request.session),
    ]);
    if launch.plan {
        command.args(["--permission-mode", "plan"]);
        // The availability flag opens a bypass consent dialog even while starting in Plan.
        // Ask must preserve the native manual policy after the person exits Plan as well.
        if launch.permission != Some(Permission::Ask) {
            command.arg("--allow-dangerously-skip-permissions");
        }
    } else if launch.permission == Some(Permission::Ask) {
        // An omitted mode can inherit native Auto. Ask must keep the explicit manual policy.
        command.args(["--permission-mode", "default"]);
    } else {
        command.arg("--dangerously-skip-permissions");
    }
    if !launch.instructions.is_empty() {
        command.args(["--append-system-prompt", &launch.instructions]);
    }
    option(&mut command, "--model", &launch.model);
    option(&mut command, "--effort", &launch.effort);
    if let Some(config) = selected.mcp_config {
        command
            .arg("--mcp-config")
            .arg(config)
            .arg("--strict-mcp-config");
    }
    command.args(selected.plugin_args);
    command
}

fn prepare_codex(
    request: &LaunchRequest,
    profile: accounts::Profile,
    profiles: &dyn ProfileBackend,
    tools: &dyn StartupTools,
) -> Result<PreparedTerminalAgent, String> {
    // The hook bridge enforces the minimum version. Newer releases are allowed: the engine
    // bootstrap verifies identity and fails closed if the private protocol changed.
    require_flags(&help("codex")?, &["--remote"])?;
    let packages = request.settings.plugin_packages();
    let plugins = tools.codex_plugins(
        request
            .settings
            .config_scope
            .as_deref()
            .unwrap_or(&request.workspace),
        packages.as_deref(),
        &profile,
    )?;
    let mcp = tools.codex_mcp(&request.session, request.settings.mcp.as_deref())?;
    let mut engine_command = Command::new("codex");
    profiles.apply(&profile, &mut engine_command)?;
    configure_codex(&mut engine_command, request, &plugins, mcp.as_ref());
    let mut command = Command::new("codex");
    command.current_dir(&request.worktree);
    profiles.apply(&profile, &mut command)?;
    for (key, value) in engine_command.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
    let mut hook_probe = Command::new("codex");
    hook_probe.arg("app-server").current_dir(&request.worktree);
    profiles.apply(&profile, &mut hook_probe)?;
    configure_codex_session(&mut hook_probe, request, &plugins, mcp.as_ref());
    if !plugins.hook_ids.is_empty() {
        trust_codex_hooks(request, &profile, profiles, &plugins, mcp.as_ref())?;
    }
    let provider_session = previous(request);
    let engine = crate::terminal_agent_codex::NativeEnginePlan::new(
        engine_command,
        request,
        provider_session.clone(),
    );
    Ok(PreparedTerminalAgent {
        command,
        engine: Some(engine),
        hook_probe: Some(hook_probe),
        profile,
        resumed: provider_session.is_some(),
        provider_session,
        initial_instructions: None,
    })
}

fn configure_codex(
    command: &mut Command,
    request: &LaunchRequest,
    plugins: &CodexPlugins,
    mcp: Option<&prometeu_tools::mcp::CodexMcp>,
) {
    command.current_dir(&request.worktree).arg("app-server");
    configure_codex_session(command, request, plugins, mcp);
}

fn configure_codex_session(
    command: &mut Command,
    request: &LaunchRequest,
    plugins: &CodexPlugins,
    mcp: Option<&prometeu_tools::mcp::CodexMcp>,
) {
    let launch = &request.settings;
    codex_tools(command, plugins, mcp);
    if !launch.model.trim().is_empty() {
        config_string(command, "model", launch.model.trim());
    }
    let effort = crate::agents::effort(&launch.effort);
    if !effort.trim().is_empty() {
        config_string(command, "model_reasoning_effort", effort.trim());
    }
    if !launch.instructions.is_empty() {
        config_string(command, "developer_instructions", &launch.instructions);
    }
    // The public config no longer accepts untrusted. The private engine applies the exact
    // requested policy through native thread RPC before any TUI or prompt can reach it.
    config_string(command, "approval_policy", "never");
    config_string(command, "sandbox_mode", "danger-full-access");
}

fn codex_tools(
    command: &mut Command,
    plugins: &CodexPlugins,
    mcp: Option<&prometeu_tools::mcp::CodexMcp>,
) {
    command.args(["--enable", "default_mode_request_user_input"]);
    command.args(["-c", "suppress_unstable_features_warning=true"]);
    if let Some(home) = &plugins.home {
        command.env("CODEX_HOME", home);
    }
    if !plugins.ids.is_empty() {
        command.args(["--enable", "plugins", "--enable", "hooks"]);
    }
    if let Some((servers, environment)) = mcp {
        command.args(["-c", &format!("mcp_servers={servers}")]);
        command.envs(environment.iter().map(|(key, value)| (key, value)));
    }
}

fn trust_codex_hooks(
    request: &LaunchRequest,
    profile: &accounts::Profile,
    profiles: &dyn ProfileBackend,
    plugins: &CodexPlugins,
    mcp: Option<&prometeu_tools::mcp::CodexMcp>,
) -> Result<(), String> {
    // Reuse the app-server trust protocol without opening a thread or running inference. Trust
    // only the selected plugin's current hash, never the invocation-wide bypass flag.
    let mut command = Command::new("codex");
    command.arg("app-server").current_dir(&request.worktree);
    profiles.apply(profile, &mut command)?;
    codex_tools(&mut command, plugins, mcp);
    drop_claude_vars(&mut command);
    let error = |cause: prometeu_core::agents::CatalogError| {
        i18n::ta(
            "err.plugin.codex.hooks",
            &[("cause", format!("Codex app-server ({})", cause.code))],
        )
    };
    let mut process = CatalogProcess::spawn(
        &prometeu_process::query::UnixQueryLauncher,
        &mut command,
        Duration::from_secs(20),
    )
    .map_err(error)?;
    process.send(json!({"id": 1, "method": "initialize", "params": {
        "clientInfo": {"name": "prometeu", "title": "Prometeu", "version": env!("CARGO_PKG_VERSION")},
        "capabilities": {"experimentalApi": true}
    }})).map_err(error)?;
    process.response(1).map_err(error)?;
    process
        .send(json!({"method": "initialized", "params": {}}))
        .map_err(error)?;
    process
        .send(json!({"id": 2, "method": "hooks/list", "params": {"cwds": [&request.worktree]}}))
        .map_err(error)?;
    let trusts = selected_hook_trust(&process.response(2).map_err(error)?, plugins)?;
    if !trusts.is_empty() {
        process
            .send(json!({"id": 3, "method": "config/batchWrite", "params": {
                "edits": [{"keyPath": "hooks.state", "value": trusts, "mergeStrategy": "upsert"}],
                "reloadUserConfig": true,
            }}))
            .map_err(error)?;
        process.response(3).map_err(error)?;
    }
    Ok(())
}

fn selected_hook_trust(
    response: &Value,
    plugins: &CodexPlugins,
) -> Result<Map<String, Value>, String> {
    let selected = plugins
        .ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expected = plugins
        .hook_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut found = HashSet::new();
    let mut invalid = HashSet::new();
    let mut managed_disabled = HashSet::new();
    let mut trusts = Map::new();
    for hook in response["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
    {
        let Some(plugin) = hook["pluginId"].as_str().filter(|id| selected.contains(id)) else {
            continue;
        };
        found.insert(plugin);
        let enabled = hook["enabled"].as_bool() == Some(true);
        match hook["trustStatus"].as_str() {
            Some("managed") => {
                if !enabled {
                    managed_disabled.insert(plugin);
                }
            }
            Some("trusted") if enabled => {}
            _ => match (hook["key"].as_str(), hook["currentHash"].as_str()) {
                (Some(key), Some(hash)) if !key.is_empty() && !hash.is_empty() => {
                    trusts.insert(
                        key.to_string(),
                        json!({"trusted_hash": hash, "enabled": true}),
                    );
                }
                _ => {
                    invalid.insert(plugin);
                }
            },
        }
    }
    for (key, missing) in [
        (
            "err.plugin.codex.hooksMissing",
            expected.difference(&found).copied().collect::<Vec<_>>(),
        ),
        (
            "err.plugin.codex.hooksInvalid",
            invalid.into_iter().collect(),
        ),
        (
            "err.plugin.codex.hooksManaged",
            managed_disabled.into_iter().collect(),
        ),
    ] {
        if !missing.is_empty() {
            let mut missing = missing;
            missing.sort_unstable();
            return Err(i18n::ta(key, &[("plugins", missing.join(", "))]));
        }
    }
    Ok(trusts)
}

fn prepare_antigravity(
    request: &LaunchRequest,
    profile: accounts::Profile,
    profiles: &dyn ProfileBackend,
) -> Result<PreparedTerminalAgent, String> {
    // --settings accepts settings.json, not MCP/plugin definitions. Native installations are
    // inherited; explicit hub tools must fail until a session-scoped native injection exists.
    if [
        &request.settings.mcp,
        &request.settings.plugins,
        &request.settings.skills,
    ]
    .iter()
    .any(|items| nonempty(items))
    {
        return Err(i18n::t("err.antigravity.unsupported"));
    }
    let available = help("agy")?;
    require_flags(&available, &["--add-dir", "--conversation"])?;
    if request.settings.permission == Some(Permission::Ask) {
        require_flags(&available, &["--settings"])?;
    }
    if request.settings.plan {
        require_flags(&available, &["--mode"])?;
    }
    let mut command = antigravity_command(request);
    profiles.apply(&profile, &mut command)?;
    let provider_session = previous(request);
    Ok(PreparedTerminalAgent {
        command,
        engine: None,
        hook_probe: None,
        profile,
        resumed: provider_session.is_some(),
        provider_session,
        initial_instructions: instructions(request),
    })
}

fn antigravity_command(request: &LaunchRequest) -> Command {
    let mut command = Command::new("agy");
    command
        .current_dir(&request.worktree)
        .args(["--add-dir", &request.worktree]);
    let launch = &request.settings;
    option(&mut command, "--model", &launch.model);
    option(&mut command, "--effort", &launch.effort);
    if launch.permission == Some(Permission::Ask) {
        // An omitted flag inherits the native account's Auto/always-proceed preferences. Keep
        // review explicit for this invocation while preserving its native allow/ask/deny rules.
        command.args([
            "--settings",
            r#"{"agentMode":"","toolPermission":"request-review"}"#,
        ]);
    }
    if launch.plan {
        command.args(["--mode", "plan"]);
    } else if launch.permission != Some(Permission::Ask) {
        command.arg("--dangerously-skip-permissions");
    }
    if let Some(session) = previous(request) {
        command.args(["--conversation", &session]);
    }
    command
}

fn prepare_cursor(
    request: &LaunchRequest,
    profile: accounts::Profile,
    profiles: &dyn ProfileBackend,
    tools: &dyn StartupTools,
) -> Result<PreparedTerminalAgent, String> {
    if [&request.settings.plugins, &request.settings.skills]
        .iter()
        .any(|items| nonempty(items))
        || !request.settings.effort.trim().is_empty()
        || request.settings.instructions.encode_utf16().count() > 10_000
    {
        // Cursor encodes effort in model IDs. Native prompt-hook context has a hard UTF-16
        // limit; refuse truncation instead of silently dropping configured instructions.
        return Err(i18n::t("err.cursor.unsupported"));
    }
    let mcp = tools.cursor_mcp(&request.session, request.settings.mcp.as_deref())?;
    let plugin = if let Some(servers) = mcp.filter(|servers| !servers.is_empty()) {
        require_flags(&help("cursor-agent")?, &["--plugin-dir"])?;
        let directory = paths::root()
            .join("terminal-agents")
            .join(&request.session)
            .join("cursor-mcp");
        write_cursor_mcp(&directory, &servers)?;
        Some(directory)
    } else {
        None
    };
    let workspace = std::fs::canonicalize(&request.worktree).map_err(i18n::io)?;
    let mut command = cursor_command(request, &workspace, plugin.as_deref());
    profiles.apply(&profile, &mut command)?;
    capture_cursor_environment(&mut command, &profile.home);
    let provider_session = previous(request);
    Ok(PreparedTerminalAgent {
        command,
        engine: None,
        hook_probe: None,
        profile,
        resumed: provider_session.is_some(),
        provider_session,
        initial_instructions: instructions(request),
    })
}

fn cursor_command(request: &LaunchRequest, workspace: &Path, plugin: Option<&Path>) -> Command {
    let mut command = Command::new("cursor-agent");
    command
        .current_dir(workspace)
        .arg("--workspace")
        .arg(workspace);
    let launch = &request.settings;
    option(&mut command, "--model", &launch.model);
    if launch.plan {
        command.args(["--mode", "plan"]);
    }
    if launch.permission != Some(Permission::Ask) {
        command.arg("--force");
    }
    if let Some(plugin) = plugin {
        command.arg("--plugin-dir").arg(plugin);
    }
    if let Some(session) = previous(request) {
        command.args(["--resume", &session]);
    }
    command
}

fn write_cursor_mcp(directory: &Path, servers: &[Value]) -> Result<(), String> {
    let body = cursor_mcp_config(servers)?;
    paths::ensure_private_dir(directory)?;
    let manifest = directory.join(".cursor-plugin");
    paths::ensure_private_dir(&manifest)?;
    paths::write_private(
        &manifest.join("plugin.json"),
        &json!({
            "name": "prometeu-session-tools", "version": "1.0.0", "mcpServers": "./mcp.json"
        })
        .to_string(),
    )?;
    // OAuth headers and stdio environments stay in a private file, never in process arguments.
    paths::write_private(&directory.join("mcp.json"), &body.to_string())
}

fn cursor_mcp_config(servers: &[Value]) -> Result<Value, String> {
    let mut configurations = Map::new();
    for server in servers {
        let name = server["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| i18n::t("err.mcp.badConfig"))?;
        let mut config = server
            .as_object()
            .cloned()
            .ok_or_else(|| i18n::t("err.mcp.badConfig"))?;
        config.remove("name");
        for key in ["env", "headers"] {
            if let Some(values) = config.remove(key) {
                let mut pairs = Map::new();
                for pair in values
                    .as_array()
                    .ok_or_else(|| i18n::t("err.mcp.badConfig"))?
                {
                    let name = pair["name"]
                        .as_str()
                        .ok_or_else(|| i18n::t("err.mcp.badConfig"))?;
                    let value = pair["value"]
                        .as_str()
                        .ok_or_else(|| i18n::t("err.mcp.badConfig"))?;
                    pairs.insert(name.to_string(), Value::String(value.to_string()));
                }
                config.insert(key.to_string(), Value::Object(pairs));
            }
        }
        configurations.insert(name.to_string(), Value::Object(config));
    }
    Ok(json!({"mcpServers": configurations}))
}

fn option(command: &mut Command, name: &str, value: &str) {
    if !value.trim().is_empty() {
        command.args([name, value.trim()]);
    }
}

fn config_string(command: &mut Command, key: &str, value: &str) {
    command.args([
        "-c",
        &format!("{key}={}", toml::Value::String(value.to_string())),
    ]);
}

fn nonempty(selection: &Option<Vec<String>>) -> bool {
    selection.as_ref().is_some_and(|items| !items.is_empty())
}

fn instructions(request: &LaunchRequest) -> Option<String> {
    (!request.settings.instructions.is_empty()).then(|| request.settings.instructions.clone())
}

fn drop_claude_vars(command: &mut Command) {
    let explicit = command
        .get_envs()
        .map(|(key, _)| key.to_os_string())
        .collect::<HashSet<_>>();
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| key.starts_with("CLAUDE")) && !explicit.contains(&key) {
            command.env_remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::session::launch::Launch;
    use prometeu_tools::ClaudeTools;
    use std::{ffi::OsStr, path::PathBuf};

    fn request(provider: ProviderId) -> LaunchRequest {
        LaunchRequest {
            session: "e0b473a4-d9c3-4573-9985-ad6a121747c9".into(),
            workspace: "workspace-1".into(),
            worktree: "/tmp/Work tree".into(),
            settings: Launch {
                agent: provider,
                ..Launch::default()
            },
            mode: StartMode::Fresh,
        }
    }

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn value<'a>(arguments: &'a [String], flag: &str) -> Option<&'a str> {
        arguments
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }

    fn no_claude_tools() -> ClaudeTools {
        ClaudeTools {
            mcp_config: None,
            plugin_args: vec![],
        }
    }

    fn no_codex_plugins() -> CodexPlugins {
        CodexPlugins {
            home: None,
            ids: vec![],
            hook_ids: vec![],
        }
    }

    fn codex_command(
        request: &LaunchRequest,
        plugins: &CodexPlugins,
        mcp: Option<&prometeu_tools::mcp::CodexMcp>,
    ) -> Command {
        let mut command = Command::new("codex");
        configure_codex(&mut command, request, plugins, mcp);
        command
    }

    fn observed(provider: ProviderId, home: &Path, command: Command) -> PreparedTerminalAgent {
        PreparedTerminalAgent {
            command,
            engine: None,
            hook_probe: None,
            profile: accounts::Profile {
                id: "account".into(),
                provider,
                home: home.into(),
                managed: false,
                revision: 0,
            },
            resumed: false,
            provider_session: None,
            initial_instructions: None,
        }
    }

    #[test]
    fn antigravity_store_root_uses_only_the_captured_effective_home() {
        let mut command = Command::new("agy");
        command.env("HOME", "/captured/native-home");
        let prepared = observed(ProviderId::Antigravity, Path::new("/profile"), command);
        assert_eq!(
            prepared.antigravity_store_root(),
            Some(PathBuf::from(
                "/captured/native-home/.gemini/antigravity-cli/conversations"
            ))
        );
        let prepared = observed(
            ProviderId::Antigravity,
            Path::new("/profile"),
            Command::new("agy"),
        );
        assert_eq!(
            prepared.antigravity_store_root(),
            Some(PathBuf::from(
                "/profile/.gemini/antigravity-cli/conversations"
            ))
        );
        let other = observed(
            ProviderId::Cursor,
            Path::new("/profile"),
            Command::new("cursor-agent"),
        );
        assert!(other.antigravity_store_root().is_none());
    }

    #[test]
    fn observation_roots_follow_only_precise_profile_transcript_symlinks() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "prometeu-observation-roots-{}",
            uuid::Uuid::new_v4()
        ));
        let home = root.join("account");
        let target = root.join("base/projects");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        symlink(&target, home.join("projects")).unwrap();
        let prepared = observed(ProviderId::Claude, &home, Command::new("claude"));
        let roots = prepared.observation_roots().unwrap();
        assert_eq!(
            roots,
            vec![home.join("projects"), target.canonicalize().unwrap()]
        );
        assert!(!roots.contains(&home));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cursor_store_uses_native_config_precedence_and_captured_workspace_hash() {
        let profile = Path::new("/profile");
        let cwd = "/tmp/workspace/linked/../original";
        let digest = format!("{:x}", md5::compute(b"/tmp/workspace/original"));
        for (config, xdg, expected) in [
            (Some("/private/config"), Some("/xdg"), "/private/config"),
            (Some("  "), Some("/xdg"), "/xdg/cursor"),
            (None, Some("/xdg"), "/xdg/cursor"),
            (None, Some(""), "/effective/.cursor"),
            (None, None, "/effective/.cursor"),
        ] {
            let mut command = Command::new("cursor-agent");
            command.current_dir(cwd).env("HOME", "/effective");
            if let Some(config) = config {
                command.env("CURSOR_CONFIG_DIR", config);
            }
            if let Some(xdg) = xdg {
                command.env("XDG_CONFIG_HOME", xdg);
            }
            let prepared = observed(ProviderId::Cursor, profile, command);
            assert_eq!(
                prepared.cursor_store_root(),
                Some(Path::new(expected).join("chats").join(&digest))
            );
        }
        let mut command = Command::new("cursor-agent");
        command
            .current_dir(cwd)
            .env("CURSOR_CONFIG_DIR", "relative");
        assert!(observed(ProviderId::Cursor, profile, command)
            .cursor_store_root()
            .is_none());
        assert!(
            observed(ProviderId::Claude, profile, Command::new("claude"))
                .cursor_store_root()
                .is_none()
        );
    }

    #[test]
    fn cursor_environment_capture_preserves_explicit_values_and_removals() {
        let mut command = Command::new("cursor-agent");
        command
            .env("HOME", "/captured")
            .env("CURSOR_CONFIG_DIR", "/captured/config")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("CURSOR_DATA_DIR");
        capture_cursor_environment(&mut command, Path::new("/unused"));
        let values: Vec<_> = command.get_envs().collect();
        assert!(values.contains(&("HOME".as_ref(), Some("/captured".as_ref()))));
        assert!(values.contains(&(
            "CURSOR_CONFIG_DIR".as_ref(),
            Some("/captured/config".as_ref())
        )));
        assert!(values.contains(&("XDG_CONFIG_HOME".as_ref(), None)));
        assert!(values.contains(&("CURSOR_DATA_DIR".as_ref(), None)));
    }

    #[test]
    fn observation_roots_use_effective_codex_home_and_provider_scoped_native_storage() {
        let missing = PathBuf::from(format!(
            "/tmp/prometeu-observation-roots-{}",
            uuid::Uuid::new_v4()
        ));
        let profile = missing.join("profile");
        let effective = missing.join("derived");
        let mut command = Command::new("codex");
        command.env("CODEX_HOME", &effective);
        assert_eq!(
            observed(ProviderId::Codex, &profile, command)
                .observation_roots()
                .unwrap(),
            vec![
                effective.join("sessions"),
                effective.join("archived_sessions")
            ]
        );
        let mut command = Command::new("agy");
        command.env("HOME", &effective);
        assert_eq!(
            observed(ProviderId::Antigravity, &profile, command)
                .observation_roots()
                .unwrap(),
            vec![effective.join(".gemini/antigravity-cli/brain")]
        );
        let mut command = Command::new("cursor-agent");
        command
            .current_dir("/tmp/my workspace/à-b/")
            .env("CURSOR_DATA_DIR", "/irrelevant/artifacts");
        assert_eq!(
            observed(ProviderId::Cursor, &profile, command)
                .observation_roots()
                .unwrap(),
            vec![profile.join(".cursor/projects/tmp-my-workspace-b/agent-transcripts")]
        );
    }

    #[test]
    fn observation_roots_reject_relative_or_traversing_configuration() {
        for home in ["relative", "/tmp/root/../other"] {
            let mut command = Command::new("codex");
            command.env("CODEX_HOME", home);
            assert!(observed(ProviderId::Codex, Path::new("/account"), command)
                .observation_roots()
                .is_err());
        }
    }

    #[test]
    fn claude_preserves_identity_cwd_instructions_and_native_tools_without_headless_flags() {
        let mut request = request(ProviderId::Claude);
        request.settings.model = " opus ".into();
        request.settings.effort = " high ".into();
        request.settings.permission = Some(Permission::Ask);
        request.settings.instructions = "Preserve whitespace\nand `literal $(text)`".into();
        let command = claude_command(
            &request,
            None,
            ClaudeTools {
                mcp_config: Some("/tmp/private MCP.json".into()),
                plugin_args: vec!["--plugin-dir".into(), "/tmp/local plugin".into()],
            },
        );
        let arguments = args(&command);
        assert_eq!(command.get_program(), "claude");
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new(&request.worktree))
        );
        assert_eq!(
            value(&arguments, "--session-id"),
            Some(request.session.as_str())
        );
        assert_eq!(value(&arguments, "--model"), Some("opus"));
        assert_eq!(value(&arguments, "--effort"), Some("high"));
        assert_eq!(value(&arguments, "--permission-mode"), Some("default"));
        assert_eq!(
            value(&arguments, "--append-system-prompt"),
            Some(request.settings.instructions.as_str())
        );
        assert_eq!(
            value(&arguments, "--mcp-config"),
            Some("/tmp/private MCP.json")
        );
        assert_eq!(value(&arguments, "--plugin-dir"), Some("/tmp/local plugin"));
        assert!(arguments.contains(&"--strict-mcp-config".into()));
        for absent in [
            "-p",
            "--input-format",
            "--output-format",
            "--permission-prompt-tool",
            "--dangerously-skip-permissions",
        ] {
            assert!(!arguments.iter().any(|arg| arg == absent), "{absent}");
        }
    }

    #[test]
    fn claude_resume_is_exact_and_plan_does_not_start_with_bypass() {
        let mut request = request(ProviderId::Claude);
        request.settings.plan = true;
        let arguments = args(&claude_command(
            &request,
            Some("native-session"),
            no_claude_tools(),
        ));
        assert_eq!(value(&arguments, "--resume"), Some("native-session"));
        assert_eq!(value(&arguments, "--permission-mode"), Some("plan"));
        assert!(arguments.contains(&"--allow-dangerously-skip-permissions".into()));
        assert!(!arguments.contains(&"--session-id".into()));
        assert!(!arguments.contains(&"--dangerously-skip-permissions".into()));
    }

    #[test]
    fn claude_ask_plan_never_enables_bypass_on_start_or_resume() {
        let mut request = request(ProviderId::Claude);
        request.settings.plan = true;
        request.settings.permission = Some(Permission::Ask);
        for resume in [None, Some("native-plan")] {
            let arguments = args(&claude_command(&request, resume, no_claude_tools()));
            assert_eq!(value(&arguments, "--permission-mode"), Some("plan"));
            assert!(!arguments.contains(&"--allow-dangerously-skip-permissions".into()));
            assert!(!arguments.contains(&"--dangerously-skip-permissions".into()));
        }
    }

    #[test]
    fn codex_preserves_profile_tools_and_permission_without_hook_trust_bypass() {
        let mut request = request(ProviderId::Codex);
        request.settings.model = " gpt-example ".into();
        request.settings.effort = "ultracode".into();
        request.settings.permission = Some(Permission::Auto);
        request.settings.instructions = "Keep \"quotes\"\nand newlines".into();
        request.mode = StartMode::Resume {
            provider_session: Some("native-thread".into()),
        };
        let plugins = CodexPlugins {
            home: Some(PathBuf::from("/tmp/derived account")),
            ids: vec!["selected@prometeu".into()],
            hook_ids: vec!["selected@prometeu".into()],
        };
        let mcp = (
            "{remote={url=\"https://example.test/mcp\",bearer_token_env_var=\"MCP_TOKEN\"}}".into(),
            vec![("MCP_TOKEN".into(), "private-token".into())],
        );
        let command = codex_command(&request, &plugins, Some(&mcp));
        let arguments = args(&command);
        assert_eq!(command.get_program(), "codex");
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new(&request.worktree))
        );
        assert_eq!(arguments.first().map(String::as_str), Some("app-server"));
        assert!(!arguments.contains(&"resume".into()));
        let configs = arguments
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| toml::from_str::<toml::Value>(&pair[1]).unwrap())
            .collect::<Vec<_>>();
        assert!(
            configs
                .iter()
                .any(|config| config.get("model").and_then(toml::Value::as_str)
                    == Some("gpt-example"))
        );
        assert!(configs.iter().any(|config| config
            .get("sandbox_mode")
            .and_then(toml::Value::as_str)
            == Some("danger-full-access")));
        assert!(configs.iter().any(|config| config
            .get("model_reasoning_effort")
            .and_then(toml::Value::as_str)
            == Some("ultra")));
        assert!(configs.iter().any(|config| config
            .get("approval_policy")
            .and_then(toml::Value::as_str)
            == Some("never")));
        assert!(configs.iter().any(|config| config
            .get("developer_instructions")
            .and_then(toml::Value::as_str)
            == Some(&request.settings.instructions)));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME"
                && value == Some(OsStr::new("/tmp/derived account"))));
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "MCP_TOKEN" && value == Some(OsStr::new("private-token"))));
        assert!(!arguments.iter().any(|arg| arg.contains("private-token")));
        assert!(!arguments.contains(&"--dangerously-bypass-hook-trust".into()));
        let mut probe = Command::new("codex");
        probe.arg("app-server").current_dir(&request.worktree);
        configure_codex_session(&mut probe, &request, &plugins, Some(&mcp));
        let probe_arguments = args(&probe);
        let probe_configs = probe_arguments
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| toml::from_str::<toml::Value>(&pair[1]).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(probe_configs, configs);
        assert!(!probe_arguments.iter().any(|arg| matches!(
            arg.as_str(),
            "--no-daemon" | "resume" | "--model" | "--sandbox"
        )));
        assert_eq!(
            probe.get_envs().collect::<Vec<_>>(),
            command.get_envs().collect::<Vec<_>>()
        );
    }

    #[test]
    fn codex_fresh_and_resume_without_identity_never_open_the_session_picker() {
        let mut request = request(ProviderId::Codex);
        for mode in [
            StartMode::Fresh,
            StartMode::Resume {
                provider_session: None,
            },
        ] {
            request.mode = mode;
            let arguments = args(&codex_command(&request, &no_codex_plugins(), None));
            assert!(!arguments.contains(&"resume".into()));
            assert!(!arguments.contains(&"--last".into()));
            assert!(arguments
                .iter()
                .any(|arg| arg == "approval_policy=\"never\""));
        }
    }

    #[test]
    fn hook_trust_checks_expected_plugins_and_only_enables_their_current_hashes() {
        let plugins = CodexPlugins {
            home: None,
            ids: vec!["selected".into()],
            hook_ids: vec!["selected".into()],
        };
        let hook = json!({"pluginId": "selected", "key": "selected/hook", "currentHash": "current", "trustStatus": "untrusted", "enabled": false});
        let response = |hook: Value| {
            json!({"data": [{"hooks": [
                hook,
                {"pluginId":"unselected","key":"unselected/hook","currentHash":"other","trustStatus":"untrusted","enabled":false}
            ]}]})
        };
        assert_eq!(
            selected_hook_trust(&response(hook.clone()), &plugins).unwrap(),
            json!({"selected/hook":{"trusted_hash":"current","enabled":true}})
                .as_object()
                .unwrap()
                .clone()
        );
        assert!(selected_hook_trust(&json!({"data":[]}), &plugins)
            .unwrap_err()
            .contains("hooksMissing"));
        let mut invalid = hook.clone();
        invalid.as_object_mut().unwrap().remove("currentHash");
        assert!(selected_hook_trust(&response(invalid), &plugins)
            .unwrap_err()
            .contains("hooksInvalid"));
        let mut managed = hook;
        managed["trustStatus"] = json!("managed");
        assert!(selected_hook_trust(&response(managed), &plugins)
            .unwrap_err()
            .contains("hooksManaged"));
    }

    #[test]
    fn external_providers_receive_native_model_resume_workspace_and_plan_flags() {
        let mut request = request(ProviderId::Antigravity);
        request.settings.model = "native-model[effort=high]".into();
        request.settings.effort = "high".into();
        request.settings.plan = true;
        request.settings.permission = Some(Permission::Ask);
        request.mode = StartMode::Resume {
            provider_session: Some("external-chat".into()),
        };
        let agy = args(&antigravity_command(&request));
        assert_eq!(value(&agy, "--add-dir"), Some(request.worktree.as_str()));
        assert_eq!(value(&agy, "--conversation"), Some("external-chat"));
        assert_eq!(
            value(&agy, "--model"),
            Some(request.settings.model.as_str())
        );
        assert_eq!(value(&agy, "--effort"), Some("high"));
        assert_eq!(value(&agy, "--mode"), Some("plan"));
        assert_eq!(
            serde_json::from_str::<Value>(value(&agy, "--settings").unwrap()).unwrap(),
            json!({"agentMode":"","toolPermission":"request-review"})
        );
        assert!(!agy.contains(&"--dangerously-skip-permissions".into()));
        let cursor = args(&cursor_command(
            &request,
            Path::new(&request.worktree),
            Some(Path::new("/tmp/session plugin")),
        ));
        assert_eq!(
            value(&cursor, "--workspace"),
            Some(request.worktree.as_str())
        );
        assert_eq!(value(&cursor, "--resume"), Some("external-chat"));
        assert_eq!(
            value(&cursor, "--model"),
            Some(request.settings.model.as_str())
        );
        assert_eq!(value(&cursor, "--mode"), Some("plan"));
        assert_eq!(value(&cursor, "--plugin-dir"), Some("/tmp/session plugin"));
    }

    #[test]
    fn cursor_mcp_plugin_keeps_credentials_private_and_preserves_transports() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("prometeu-cursor-terminal-{}", uuid::Uuid::new_v4()));
        let servers = vec![
            json!({"name":"local","command":"node","args":["/tmp/mcp.js"],"env":[{"name":"API_KEY","value":"private"}]}),
            json!({"name":"remote","type":"http","url":"https://example.test/mcp","headers":[{"name":"Authorization","value":"Bearer private"}]}),
            json!({"name":"events","type":"sse","url":"https://example.test/events","headers":[]}),
        ];
        write_cursor_mcp(&directory, &servers).unwrap();
        let body: Value =
            serde_json::from_str(&std::fs::read_to_string(directory.join("mcp.json")).unwrap())
                .unwrap();
        assert_eq!(body["mcpServers"]["local"]["env"]["API_KEY"], "private");
        assert_eq!(
            body["mcpServers"]["remote"]["headers"]["Authorization"],
            "Bearer private"
        );
        assert_eq!(body["mcpServers"]["remote"]["type"], "http");
        assert_eq!(body["mcpServers"]["events"]["type"], "sse");
        assert!(body["mcpServers"]["local"].get("name").is_none());
        for relative in ["mcp.json", ".cursor-plugin/plugin.json"] {
            assert_eq!(
                std::fs::metadata(directory.join(relative))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn configuration_rejects_missing_capabilities_and_malformed_mcp_entries() {
        assert!(require_flags("Codex CLI --no-daemon", &["--no-daemon"]).is_ok());
        assert!(require_flags("Codex CLI", &["--no-daemon"]).is_err());
        assert!(cursor_mcp_config(&[json!({"name":"bad","headers":[{"name":"Auth"}]})]).is_err());
        assert!(!nonempty(&None));
        assert!(!nonempty(&Some(vec![])));
        assert!(nonempty(&Some(vec!["tool".into()])));
    }

    #[test]
    fn preparation_captures_profiles_and_resume_state_without_starting_agents() {
        const CHILD: &str = "PROMETEU_TERMINAL_PREPARATION_TEST";
        if std::env::var_os(CHILD).is_none() {
            use std::os::unix::fs::PermissionsExt;
            let root = std::env::temp_dir().join(format!(
                "prometeu-terminal-prepare-{}",
                uuid::Uuid::new_v4()
            ));
            let bin = root.join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            for program in ["codex", "agy"] {
                let path = bin.join(program);
                std::fs::write(&path, "#!/bin/sh\nif [ \"$1\" = --help ]; then\n  printf '%s\\n' '--remote --no-daemon --add-dir --conversation --mode'\n  exit 0\nfi\nif [ \"$1\" = --version ]; then\n  printf '%s\\n' 'codex-cli 0.169.0'\n  exit 0\nfi\nexit 99\n").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "terminal_agent_launch::tests::preparation_captures_profiles_and_resume_state_without_starting_agents", "--nocapture"])
                .env(CHILD, "1")
                .env("HOME", &root)
                .env("PROMETEU_ROOT", root.join("app"))
                .env("CLAUDE_CONFIG_DIR", root.join("claude"))
                .env("CODEX_HOME", root.join("codex"))
                .env("CURSOR_CONFIG_DIR", root.join("cursor"))
                .env_remove("CURSOR_DATA_DIR")
                .env("CLAUDECODE", "inherited-parent")
                .env("PATH", bin)
                .current_dir(&root)
                .output().unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        accounts::change(|registry| {
            for provider in [ProviderId::Antigravity, ProviderId::Cursor] {
                let key = prometeu_core::accounts::key(provider);
                registry.accounts.push(accounts::Account {
                    id: key.into(),
                    provider,
                    revision: 7,
                    auth_method: Some("external".into()),
                    key_suffix: None,
                    identity: Default::default(),
                });
                registry.active.insert(key.into(), key.into());
            }
            Ok(())
        })
        .unwrap();
        let profiles = crate::account_profiles::native();
        let tools = crate::tool_materialization::native();
        let root = std::env::current_dir().unwrap();
        let physical_cursor_workspace = root.join("native-target/workspace");
        std::fs::create_dir_all(root.join("native-target/deep")).unwrap();
        std::fs::create_dir_all(&physical_cursor_workspace).unwrap();
        std::os::unix::fs::symlink(root.join("native-target/deep"), root.join("workspace-link"))
            .unwrap();
        let cursor_alias = root.join("workspace-link/../workspace");
        assert!(!root.join("workspace").exists());
        for provider in [
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Antigravity,
            ProviderId::Cursor,
        ] {
            let mut request = request(provider);
            request.worktree = std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if provider == ProviderId::Cursor {
                request.worktree = cursor_alias.to_str().unwrap().into();
            }
            request.settings.instructions = "Use the captured task profile.".into();
            let fresh = prepare(&request, &profiles, &tools).unwrap();
            if provider == ProviderId::Cursor {
                assert_cursor_workspace_capture(&fresh, &physical_cursor_workspace);
                assert_eq!(request.worktree, cursor_alias.to_str().unwrap());
            }
            assert!(!fresh.resumed);
            assert_eq!(fresh.profile.provider, provider);
            assert!(!fresh.profile.managed);
            assert!(fresh
                .command
                .get_envs()
                .any(|(key, value)| key == "CLAUDECODE" && value.is_none()));
            if provider == ProviderId::Claude {
                assert_eq!(
                    fresh.provider_session.as_deref(),
                    Some(request.session.as_str())
                );
            } else {
                assert!(fresh.provider_session.is_none());
            }
            let pending = matches!(provider, ProviderId::Antigravity | ProviderId::Cursor);
            assert_eq!(fresh.initial_instructions.is_some(), pending);
            request.mode = StartMode::Resume {
                provider_session: Some("captured-native-id".into()),
            };
            let resumed = prepare(&request, &profiles, &tools).unwrap();
            if provider == ProviderId::Cursor {
                assert_cursor_workspace_capture(&resumed, &physical_cursor_workspace);
            }
            assert!(resumed.resumed);
            assert_eq!(
                resumed.initial_instructions.as_deref(),
                pending.then_some(request.settings.instructions.as_str())
            );
            assert_eq!(
                resumed.provider_session.as_deref(),
                Some("captured-native-id")
            );
        }
        let mut unsupported = request(ProviderId::Antigravity);
        unsupported.settings.mcp = Some(vec!["explicit-server".into()]);
        assert!(prepare(&unsupported, &profiles, &tools)
            .err()
            .unwrap()
            .contains("err.antigravity.unsupported"));
        unsupported.settings.agent = ProviderId::Cursor;
        unsupported.settings.plugins = Some(vec!["explicit-plugin".into()]);
        assert!(prepare(&unsupported, &profiles, &tools)
            .err()
            .unwrap()
            .contains("err.cursor.unsupported"));
        let mut codex = request(ProviderId::Codex);
        codex.settings.plan = true;
        assert!(prepare(&codex, &profiles, &tools).unwrap().engine.is_some());
        codex.settings.plan = false;
        codex.settings.permission = Some(Permission::Ask);
        assert!(prepare(&codex, &profiles, &tools).unwrap().engine.is_some());
    }

    fn assert_cursor_workspace_capture(prepared: &PreparedTerminalAgent, physical: &Path) {
        let physical = physical.canonicalize().unwrap();
        let cwd = physical.to_str().unwrap();
        assert_eq!(prepared.command.get_current_dir(), Some(physical.as_path()));
        assert_eq!(value(&args(&prepared.command), "--workspace"), Some(cwd));
        let config = PathBuf::from(std::env::var_os("CURSOR_CONFIG_DIR").unwrap());
        let digest = format!("{:x}", md5::compute(cwd.as_bytes()));
        assert_eq!(
            prepared.cursor_store_root(),
            Some(config.join("chats").join(digest))
        );
        let slug = cwd
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(
            prepared.observation_roots().unwrap(),
            vec![home
                .join(".cursor/projects")
                .join(slug)
                .join("agent-transcripts")]
        );
    }
}
