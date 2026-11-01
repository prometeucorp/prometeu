//! Native provider preparation, with no desktop handle or conversation process ownership.
use crate::{accounts, i18n, paths, state::ProviderId};
use prometeu_core::conversation::stream::TranscriptStore;
use prometeu_core::process::ProcessControl;
use prometeu_core::session::launch::{LaunchRequest, StartMode};
use prometeu_core::session::provider::{AgentProtocol, ProviderPreparation};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

type Connect =
    Box<dyn FnOnce(Box<dyn Write + Send>, Arc<dyn ProcessControl>) -> AgentProtocol + Send>;

pub struct PreparedAgent {
    pub command: Command,
    pub store: Arc<dyn TranscriptStore>,
    pub profile: accounts::Profile,
    pub connect: Connect,
    pub stderr_line: fn(&str) -> Option<String>,
    pub spawn_error: &'static str,
    pub resumed: bool,
}

pub struct NativeProviders {
    pub profiles: Arc<dyn prometeu_profiles::ProfileBackend>,
    pub tools: Arc<dyn prometeu_tools::StartupTools>,
}
type Prepare = fn(
    &LaunchRequest,
    &dyn prometeu_profiles::ProfileBackend,
    &dyn prometeu_tools::StartupTools,
) -> Result<PreparedAgent, String>;
const PROVIDERS: &[(ProviderId, Prepare)] = &[
    (ProviderId::Claude, claude),
    (ProviderId::Codex, codex),
    (ProviderId::Antigravity, antigravity),
    (ProviderId::Cursor, cursor),
];
impl ProviderPreparation<PreparedAgent> for NativeProviders {
    fn prepare(&self, request: &LaunchRequest) -> Result<PreparedAgent, String> {
        let prepare = PROVIDERS
            .iter()
            .find(|(id, _)| *id == request.settings.agent)
            .map(|(_, prepare)| prepare)
            .ok_or_else(|| i18n::t("err.provider.retired"))?;
        let mut prepared = prepare(request, self.profiles.as_ref(), self.tools.as_ref())?;
        drop_claude_vars(&mut prepared.command);
        Ok(prepared)
    }
}
fn claude(
    request: &LaunchRequest,
    profiles: &dyn prometeu_profiles::ProfileBackend,
    tools: &dyn prometeu_tools::StartupTools,
) -> Result<PreparedAgent, String> {
    let worktree = Path::new(&request.worktree);
    let resume = matches!(request.mode, StartMode::Resume { .. })
        && paths::transcript(&request.session, worktree).exists();
    crate::claude::prepare(
        &request.session,
        worktree,
        resume,
        &request.settings,
        profiles,
        tools,
    )
}
fn previous(mode: &StartMode) -> Option<String> {
    match mode {
        StartMode::Fresh => None,
        StartMode::Resume { provider_session } => provider_session.clone(),
    }
}
fn codex(
    request: &LaunchRequest,
    profiles: &dyn prometeu_profiles::ProfileBackend,
    tools: &dyn prometeu_tools::StartupTools,
) -> Result<PreparedAgent, String> {
    crate::codex::prepare(
        &request.session,
        &request.workspace,
        Path::new(&request.worktree),
        previous(&request.mode),
        &request.settings,
        profiles,
        tools,
    )
}
fn cursor(
    request: &LaunchRequest,
    profiles: &dyn prometeu_profiles::ProfileBackend,
    tools: &dyn prometeu_tools::StartupTools,
) -> Result<PreparedAgent, String> {
    crate::cursor::prepare(
        &request.session,
        Path::new(&request.worktree),
        previous(&request.mode),
        &request.settings,
        profiles,
        tools,
    )
}
fn antigravity(
    request: &LaunchRequest,
    profiles: &dyn prometeu_profiles::ProfileBackend,
    _tools: &dyn prometeu_tools::StartupTools,
) -> Result<PreparedAgent, String> {
    crate::antigravity::prepare(
        &request.session,
        Path::new(&request.worktree),
        previous(&request.mode),
        &request.settings,
        profiles,
    )
}

/// Preserve explicit values (including MCP credentials) while dropping inherited Claude variables.
fn drop_claude_vars(cmd: &mut Command) {
    let explicit: std::collections::HashSet<_> =
        cmd.get_envs().map(|(key, _)| key.to_os_string()).collect();
    for (key, _) in std::env::vars() {
        if key.starts_with("CLAUDE") && !explicit.contains(std::ffi::OsStr::new(&key)) {
            cmd.env_remove(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparation_preserves_resume_and_transcript_ownership_without_agent_processes() {
        const CHILD: &str = "PROMETEU_PROVIDER_PREPARATION_TEST";
        if std::env::var_os(CHILD).is_none() {
            let root =
                std::env::temp_dir().join(format!("prometeu-provider-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "agent_launch::tests::preparation_preserves_resume_and_transcript_ownership_without_agent_processes", "--nocapture"])
                .env(CHILD, "1")
                .env("HOME", &root)
                .env("PROMETEU_ROOT", root.join("app"))
                .env("CLAUDE_CONFIG_DIR", root.join("claude"))
                .env("CODEX_HOME", root.join("codex"))
                .env("PATH", root.join("no-agent-binaries"))
                .current_dir(&root)
                .output().unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let worktree = std::env::current_dir().unwrap();
        let mut request = LaunchRequest {
            session: "preparation-test".into(),
            workspace: "workspace".into(),
            worktree: worktree.to_string_lossy().into_owned(),
            settings: Default::default(),
            mode: StartMode::Resume {
                provider_session: None,
            },
        };
        let native = NativeProviders {
            profiles: Arc::new(crate::account_profiles::native()),
            tools: Arc::new(crate::tool_materialization::native()),
        };
        struct RefusePreparation;
        impl prometeu_profiles::ProfileBackend for RefusePreparation {
            fn resolve(&self, account: &accounts::Account) -> Result<accounts::Profile, String> {
                crate::account_profiles::native().resolve(account)
            }
            fn prepare(&self, _: &accounts::Profile) -> Result<(), String> {
                Err("profile preparation refused".into())
            }
            fn apply(&self, _: &accounts::Profile, _: &mut Command) -> Result<(), String> {
                panic!("failed preparation must stop command configuration")
            }
        }
        let refused = NativeProviders {
            profiles: Arc::new(RefusePreparation),
            tools: Arc::new(crate::tool_materialization::native()),
        };
        assert_eq!(
            refused.prepare(&request).err(),
            Some("profile preparation refused".into())
        );
        let providers: &dyn ProviderPreparation<PreparedAgent> = &native;
        let fresh = providers.prepare(&request).unwrap();
        assert!(!fresh.resumed);
        assert_eq!(fresh.command.get_program(), "claude");
        assert_eq!(fresh.command.get_current_dir(), Some(worktree.as_path()));
        assert!(fresh.command.get_args().any(|arg| arg == "--session-id"));
        let history = paths::transcript(&request.session, &worktree);
        std::fs::create_dir_all(history.parent().unwrap()).unwrap();
        std::fs::write(&history, "native history\n").unwrap();
        let resumed = providers.prepare(&request).unwrap();
        assert!(resumed.resumed);
        assert!(resumed.command.get_args().any(|arg| arg == "--resume"));
        resumed.store.append("canonical event").unwrap();
        assert_eq!(
            std::fs::read_to_string(&history).unwrap(),
            "native history\n"
        );
        request.mode = StartMode::Fresh;
        assert!(!providers.prepare(&request).unwrap().resumed);
        request.settings.agent = ProviderId::Codex;
        let fresh = providers.prepare(&request).unwrap();
        assert!(!fresh.resumed);
        assert_eq!(fresh.command.get_program(), "codex");
        assert!(fresh.command.get_args().any(|arg| arg == "app-server"));
        request.mode = StartMode::Resume {
            provider_session: Some("native-thread".into()),
        };
        let resumed = providers.prepare(&request).unwrap();
        assert!(resumed.resumed);
        resumed.store.append("canonical event").unwrap();
        assert_eq!(
            std::fs::read_to_string(paths::chat_log(&request.session)).unwrap(),
            "canonical event\n"
        );
        injected_tools_preserve_startup_artifacts_and_failure_order(&request);
        bundled_search_preserves_native_mcp_inheritance(&native, &worktree);
        cursor_search_reaches_new_and_resumed_sessions(&native, &worktree);
        request.settings.agent = ProviderId::RetiredGemini;
        assert!(providers.prepare(&request).is_err());
    }

    fn bundled_search_preserves_native_mcp_inheritance(native: &NativeProviders, worktree: &Path) {
        for provider in [ProviderId::Claude, ProviderId::Codex] {
            for selection in [None, Some(vec![])] {
                for mode in [
                    StartMode::Fresh,
                    StartMode::Resume {
                        provider_session: None,
                    },
                ] {
                    let mut request = LaunchRequest {
                        session: "search-inheritance".into(),
                        workspace: "workspace".into(),
                        worktree: worktree.to_string_lossy().into_owned(),
                        settings: Default::default(),
                        mode,
                    };
                    request.settings.agent = provider;
                    request.settings.mcp = selection.clone();
                    let prepared = native.prepare(&request).unwrap();
                    let args = prepared
                        .command
                        .get_args()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    if provider == ProviderId::Claude {
                        assert_eq!(args.iter().filter(|arg| *arg == "--mcp-config").count(), 1);
                        assert_eq!(
                            args.iter().any(|arg| arg == "--strict-mcp-config"),
                            selection.is_some()
                        );
                        let search = args
                            .iter()
                            .find_map(|arg| serde_json::from_str::<serde_json::Value>(arg).ok())
                            .unwrap();
                        let servers = search["mcpServers"].as_object().unwrap();
                        assert!(!servers.is_empty());
                        assert!(servers
                            .values()
                            .all(|server| server["args"][0] == "--prometeu-search-mcp"));
                    } else {
                        assert_eq!(
                            args.iter().any(|arg| arg == "mcp_servers={}"),
                            selection.is_some()
                        );
                        assert!(args
                            .iter()
                            .any(|arg| arg.starts_with("mcp_servers.prometeu-search")
                                && arg.contains("--prometeu-search-mcp")));
                    }
                }
            }
        }
    }

    fn cursor_search_reaches_new_and_resumed_sessions(native: &NativeProviders, worktree: &Path) {
        #[derive(Clone, Default)]
        struct Output(Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Output {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        struct UnusedProcess;
        impl ProcessControl for UnusedProcess {
            fn system_id(&self) -> u32 {
                unreachable!()
            }
            fn running(&self) -> bool {
                unreachable!()
            }
            fn interrupt(&self) {
                unreachable!()
            }
            fn terminate(&self) {
                unreachable!()
            }
            fn kill(&self) {
                unreachable!()
            }
            fn wait_exit(&self, _: std::time::Duration) -> bool {
                unreachable!()
            }
        }
        for selection in [None, Some(vec![])] {
            for (mode, method) in [
                (StartMode::Fresh, "session/new"),
                (
                    StartMode::Resume {
                        provider_session: Some("cursor-session".into()),
                    },
                    "session/load",
                ),
            ] {
                let mut request = LaunchRequest {
                    session: "cursor-search".into(),
                    workspace: "workspace".into(),
                    worktree: worktree.to_string_lossy().into_owned(),
                    settings: Default::default(),
                    mode,
                };
                request.settings.agent = ProviderId::Cursor;
                request.settings.mcp = selection.clone();
                let prepared = native.prepare(&request).unwrap();
                let output = Output::default();
                let mut protocol =
                    (prepared.connect)(Box::new(output.clone()), Arc::new(UnusedProcess));
                (protocol.translate)(
                    r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#,
                );
                (protocol.translate)(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
                let bytes = output.0.lock().unwrap();
                let sent: serde_json::Value = serde_json::from_str(
                    std::str::from_utf8(&bytes).unwrap().lines().last().unwrap(),
                )
                .unwrap();
                assert_eq!(sent["method"], method);
                assert_eq!(sent["params"]["cwd"], request.worktree);
                let servers = sent["params"]["mcpServers"].as_array().unwrap();
                assert!(!servers.is_empty());
                assert!(servers
                    .iter()
                    .all(|server| server["args"][0] == "--prometeu-search-mcp"));
            }
        }
    }

    fn injected_tools_preserve_startup_artifacts_and_failure_order(base: &LaunchRequest) {
        use prometeu_tools::{mcp::AcpMcp, mcp::CodexMcp, ClaudeTools, CodexPlugins, StartupTools};
        use std::sync::Mutex;
        type Calls = Arc<Mutex<Vec<&'static str>>>;
        struct Profiles(Calls);
        impl prometeu_profiles::ProfileBackend for Profiles {
            fn resolve(&self, account: &accounts::Account) -> Result<accounts::Profile, String> {
                self.0.lock().unwrap().push("resolve");
                crate::account_profiles::native().resolve(account)
            }
            fn prepare(&self, _: &accounts::Profile) -> Result<(), String> {
                self.0.lock().unwrap().push("prepare");
                Ok(())
            }
            fn apply(&self, _: &accounts::Profile, command: &mut Command) -> Result<(), String> {
                self.0.lock().unwrap().push("apply");
                command.env("CODEX_HOME", "/profile-home");
                Ok(())
            }
        }
        struct Tools {
            calls: Calls,
            fail: Option<&'static str>,
        }
        impl Tools {
            fn record(&self, phase: &'static str) -> Result<(), String> {
                self.calls.lock().unwrap().push(phase);
                match self.fail {
                    Some(fail) if fail == phase => Err(format!("refused:{phase}")),
                    _ => Ok(()),
                }
            }
        }
        impl StartupTools for Tools {
            fn claude(
                &self,
                session: &str,
                workdir: &Path,
                mcp: Option<&[String]>,
                packages: Option<&[String]>,
            ) -> Result<ClaudeTools, String> {
                assert_eq!(session, "preparation-test");
                assert_eq!(workdir, std::env::current_dir().unwrap());
                assert_eq!(mcp.unwrap(), ["server"]);
                assert_eq!(packages.unwrap(), ["plugin", "skill"]);
                self.record("claude-tools")?;
                Ok(ClaudeTools {
                    mcp_config: Some("/private/selected.json".into()),
                    plugin_args: vec!["--plugin-dir".into(), "/packages/selected".into()],
                })
            }
            fn codex_plugins(
                &self,
                scope: &str,
                packages: Option<&[String]>,
                profile: &accounts::Profile,
            ) -> Result<CodexPlugins, String> {
                assert_eq!(scope, "frozen-scope");
                assert_eq!(packages.unwrap(), ["plugin", "skill"]);
                assert_eq!(profile.provider, ProviderId::Codex);
                self.record("codex-plugins")?;
                Ok(CodexPlugins {
                    home: Some("/derived-home".into()),
                    ids: vec!["plugin@market".into()],
                    hook_ids: vec!["plugin@market".into()],
                })
            }
            fn codex_mcp(
                &self,
                session: &str,
                chosen: Option<&[String]>,
            ) -> Result<Option<CodexMcp>, String> {
                assert_eq!(session, "preparation-test");
                assert_eq!(chosen.unwrap(), ["server"]);
                self.record("codex-mcp")?;
                Ok(Some((
                    "{server={url=\"https://example.test\"}}".into(),
                    vec![("CLAUDE_EXPLICIT_SECRET".into(), "private-token".into())],
                )))
            }
            fn cursor_mcp(
                &self,
                session: &str,
                chosen: Option<&[String]>,
            ) -> Result<Option<AcpMcp>, String> {
                assert_eq!(session, "preparation-test");
                assert_eq!(chosen.unwrap(), ["server"]);
                self.record("cursor-mcp")?;
                Ok(Some(vec![
                    serde_json::json!({"name":"server","command":"npx","args":[],"env":[]}),
                ]))
            }
        }
        for (provider, fail, expected) in [
            (
                ProviderId::Claude,
                None,
                vec!["claude-tools", "resolve", "prepare", "apply"],
            ),
            (
                ProviderId::Claude,
                Some("claude-tools"),
                vec!["claude-tools"],
            ),
            (
                ProviderId::Codex,
                None,
                vec!["resolve", "prepare", "codex-plugins", "apply", "codex-mcp"],
            ),
            (
                ProviderId::Codex,
                Some("codex-plugins"),
                vec!["resolve", "prepare", "codex-plugins"],
            ),
            (
                ProviderId::Codex,
                Some("codex-mcp"),
                vec!["resolve", "prepare", "codex-plugins", "apply", "codex-mcp"],
            ),
        ] {
            let calls = Calls::default();
            let native = NativeProviders {
                profiles: Arc::new(Profiles(calls.clone())),
                tools: Arc::new(Tools {
                    calls: calls.clone(),
                    fail,
                }),
            };
            let mut request = LaunchRequest {
                session: base.session.clone(),
                workspace: base.workspace.clone(),
                worktree: base.worktree.clone(),
                settings: base.settings.clone(),
                mode: StartMode::Fresh,
            };
            request.settings.agent = provider;
            request.settings.mcp = Some(vec!["server".into()]);
            request.settings.plugins = Some(vec!["plugin".into()]);
            request.settings.skills = Some(vec!["skill".into()]);
            request.settings.config_scope = Some("frozen-scope".into());
            let result = native.prepare(&request);
            assert_eq!(*calls.lock().unwrap(), expected);
            match fail {
                Some(phase) => assert_eq!(result.err(), Some(format!("refused:{phase}"))),
                None => {
                    let prepared = result.unwrap();
                    let args: Vec<_> = prepared
                        .command
                        .get_args()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .collect();
                    match provider {
                        ProviderId::Claude => {
                            assert!(args.windows(3).any(|args| args
                                == [
                                    "--mcp-config",
                                    "/private/selected.json",
                                    "--strict-mcp-config"
                                ]));
                            assert!(args
                                .windows(2)
                                .any(|args| args == ["--plugin-dir", "/packages/selected"]));
                        }
                        ProviderId::Codex => {
                            assert!(args
                                .windows(4)
                                .any(|args| args == ["--enable", "plugins", "--enable", "hooks"]));
                            assert!(args
                                .iter()
                                .any(|arg| arg
                                    == "mcp_servers={server={url=\"https://example.test\"}}"));
                            let env: std::collections::HashMap<_, _> =
                                prepared.command.get_envs().collect();
                            assert_eq!(
                                env[std::ffi::OsStr::new("CODEX_HOME")],
                                Some(std::ffi::OsStr::new("/derived-home"))
                            );
                            assert_eq!(
                                env[std::ffi::OsStr::new("CLAUDE_EXPLICIT_SECRET")],
                                Some(std::ffi::OsStr::new("private-token"))
                            );
                            assert!(!args.iter().any(|arg| arg.contains("private-token")));
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        crate::accounts::change(|data| {
            if !data
                .accounts
                .iter()
                .any(|account| account.provider == ProviderId::Cursor)
            {
                data.accounts.push(crate::accounts::Account {
                    id: "cursor".into(),
                    provider: ProviderId::Cursor,
                    revision: 0,
                    auth_method: Some("external".into()),
                    key_suffix: None,
                    identity: crate::accounts::Identity::default(),
                });
            }
            data.select("cursor")
        })
        .unwrap();
        for (fail, plugins, expected) in [
            (None, None, vec!["cursor-mcp", "resolve", "apply"]),
            (Some("cursor-mcp"), None, vec!["cursor-mcp"]),
            (None, Some(vec!["plugin".into()]), vec![]),
        ] {
            let calls = Calls::default();
            let native = NativeProviders {
                profiles: Arc::new(Profiles(calls.clone())),
                tools: Arc::new(Tools {
                    calls: calls.clone(),
                    fail,
                }),
            };
            let mut request = LaunchRequest {
                session: base.session.clone(),
                workspace: base.workspace.clone(),
                worktree: base.worktree.clone(),
                settings: base.settings.clone(),
                mode: StartMode::Fresh,
            };
            request.settings.agent = ProviderId::Cursor;
            request.settings.mcp = Some(vec!["server".into()]);
            request.settings.plugins = plugins;
            request.settings.skills = None;
            let result = native.prepare(&request);
            assert_eq!(*calls.lock().unwrap(), expected);
            if fail.is_some() {
                assert_eq!(result.err(), Some("refused:cursor-mcp".into()));
            } else if expected.is_empty() {
                assert_eq!(result.err(), Some(crate::i18n::t("err.cursor.unsupported")));
            } else {
                assert_eq!(result.unwrap().command.get_program(), "cursor-agent");
            }
        }
    }

    /// Caller-supplied environment values, including MCP secrets, survive inherited Claude-variable
    /// cleanup.
    #[test]
    fn caller_environment_reaches_the_process() {
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(r#"printf '%s|%s' "$PROMETEU_MCP_X_AUTHORIZATION" "$CLAUDE_CODE_CHILD_SESSION""#);
        cmd.env("PROMETEU_MCP_X_AUTHORIZATION", "Bearer abracadabra");
        drop_claude_vars(&mut cmd);
        let out = cmd.output().expect("shell");
        std::env::remove_var("CLAUDE_CODE_CHILD_SESSION");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "Bearer abracadabra|");
    }
}
