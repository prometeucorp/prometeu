//! First registered execution adapter: the same Codex protocol used by desktop.
use prometeu_core::session::{
    launch::{LaunchRequest, StartMode},
    provider::{AgentProtocol, ProviderPreparation},
};
use prometeu_protocols::codex::{self, Start};
use std::{io::Write, path::PathBuf, process::Command};
pub struct Prepared {
    pub command: Command,
    pub connect: Box<dyn FnOnce(Box<dyn Write + Send>) -> AgentProtocol + Send>,
    pub stderr_line: fn(&str) -> Option<String>,
}
pub struct CodexPreparation {
    pub workspace: String,
    pub session: String,
    pub selection: std::sync::Arc<dyn prometeu_core::tool_resolution::ToolSelection>,
    pub tools: std::sync::Arc<dyn prometeu_tools::StartupTools>,
    pub profile: prometeu_profiles::Profile,
    pub accounts: std::sync::Arc<prometeu_core::accounts::AccountRegistry>,
    pub executable: PathBuf,
    pub model: String,
    pub permission: prometeu_core::actions::Permission,
    pub effort: String,
}
impl ProviderPreparation<Prepared> for CodexPreparation {
    fn prepare(&self, request: &LaunchRequest) -> Result<Prepared, String> {
        let account = self
            .accounts
            .active(prometeu_core::board::ProviderId::Codex)?;
        if account.id != "codex" {
            return Err(prometeu_core::error::code("err.account.external"));
        }
        let selection = self.selection.resolve(&self.workspace)?;
        let packages = selection.plugin_packages();
        let plugins =
            self.tools
                .codex_plugins(&self.workspace, packages.as_deref(), &self.profile)?;
        let mut command = Command::new(&self.executable);
        command.current_dir(&request.worktree).args([
            "app-server",
            "--enable",
            "default_mode_request_user_input",
            "-c",
            "suppress_unstable_features_warning=true",
        ]);
        if let Some(home) = &plugins.home {
            command.env("CODEX_HOME", home);
        }
        if !plugins.ids.is_empty() {
            command.args(["--enable", "plugins", "--enable", "hooks"]);
        }
        if let Some((servers, environment)) = self
            .tools
            .codex_mcp(&self.session, selection.mcp.as_deref())?
        {
            command
                .args(["-c", &format!("mcp_servers={servers}")])
                .envs(environment);
        }
        let resume = match &request.mode {
            StartMode::Fresh => None,
            StartMode::Resume { provider_session } => provider_session.clone(),
        };
        let start = Start {
            cwd: request.worktree.clone(),
            resume,
            model: self.model.clone(),
            effort: self.effort.clone(),
            plugin_ids: plugins.ids,
            plugin_hook_ids: plugins.hook_ids,
            permission: Some(self.permission),
            instructions: String::new(),
        };
        Ok(Prepared {
            command,
            connect: Box::new(move |input| {
                codex::connect(input, start, |_, en| en.into(), env!("CARGO_PKG_VERSION"))
            }),
            stderr_line: codex::process_stderr,
        })
    }
}
