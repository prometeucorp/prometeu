//! Opt-in native conformance. Only transcripts reported by this test's authenticated hooks are read.
use super::*;
use prometeu_core::{
    actions::Permission,
    session::launch::{Launch, LaunchRequest, StartMode},
    terminal::{TerminalControl, TerminalFactory, TerminalSize, TerminalWait},
};
use prometeu_protocols::terminal_status::{
    ProgramStatusEvent, ProgramStatusParser, ProgramStatusState, ProgramStatusStore,
    PROGRAM_STATUS_QUERY_REPLY,
};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::mpsc::RecvTimeoutError;

const CHILD_ENV: &str = "PROMETEU_NATIVE_TERMINAL_SMOKE_CHILD";
const MARKER: &str = "PROMETEU_TERMINAL_SMOKE";
const PROMPT: &str = "Reply exactly PROMETEU_TERMINAL_SMOKE. Do not use tools.";
const TOOL_PROMPT: &str = "Use the shell tool once to run exactly `printf PROMETEU_TERMINAL_TOOL_SMOKE` in the current scratch workspace. Do not run any other command or use another tool. Then reply exactly PROMETEU_TERMINAL_SMOKE.";

#[test]
#[ignore = "uses an installed authenticated CLI and a bounded real model request"]
fn native_claude_start_and_resume() {
    smoke(ProviderId::Claude, "native_claude_start_and_resume");
}
#[test]
#[ignore = "uses an installed authenticated CLI and a bounded real model request"]
fn native_codex_start_and_resume() {
    smoke(ProviderId::Codex, "native_codex_start_and_resume");
}
#[test]
#[ignore = "uses an installed authenticated CLI and a bounded real model request"]
fn native_antigravity_start_and_resume() {
    smoke(
        ProviderId::Antigravity,
        "native_antigravity_start_and_resume",
    );
}
#[test]
#[ignore = "uses an installed authenticated CLI and a bounded real model request"]
fn native_cursor_start_and_resume() {
    smoke(ProviderId::Cursor, "native_cursor_start_and_resume");
}

#[test]
#[ignore = "captures synthetic native drafts without submitting a model request"]
fn native_cursor_paste_rendering() {
    smoke(ProviderId::Cursor, "native_cursor_paste_rendering");
}

#[test]
#[ignore = "captures synthetic native drafts without submitting a model request"]
fn native_claude_paste_rendering() {
    smoke(ProviderId::Claude, "native_claude_paste_rendering");
}

#[test]
#[ignore = "captures a synthetic literal placeholder without submitting a model request"]
fn native_claude_composer_styles() {
    smoke(ProviderId::Claude, "native_claude_composer_styles");
}

fn draft_samples() -> Vec<(&'static str, String)> {
    vec![
        ("wrapped", "PROMETEU_WRAPPED_".repeat(24)),
        (
            "multiline",
            "First synthetic line.\nSecond synthetic line.\nThird synthetic line.".into(),
        ),
        ("unicode", "São Paulo 中文 🐣 e\u{301} ".repeat(8)),
        (
            "scrolled",
            (0..14)
                .map(|n| format!("Synthetic line {n:02}.\n"))
                .collect(),
        ),
        ("chip", "PROMETEU_PASTE_CHIP\n".repeat(70)),
    ]
}

fn smoke(provider: ProviderId, name: &str) {
    if std::env::var_os(CHILD_ENV).is_none() {
        let root =
            PathBuf::from("/tmp").join(format!("prometeu-native-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("terminal_agent_hooks::smoke::{name}"),
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env("PROMETEU_ROOT", root.join("app"))
            .current_dir(&root)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|child| child.wait_with_output());
        if std::env::var_os("PROMETEU_SMOKE_KEEP_PROOF").is_none() {
            let _ = std::fs::remove_dir_all(&root);
        }
        let output = result.unwrap();
        // Child output contains only test summaries and fixed classifications, never native bytes.
        print!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "native smoke failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    crate::install_crypto();
    let captured = captured_profile(provider).unwrap();
    crate::accounts::change(|registry| {
        if let Some((account, _)) = &captured {
            registry.accounts.push(account.clone());
            registry
                .active
                .insert(provider_key(provider)?.into(), account.id.clone());
            return Ok(());
        }
        if !registry
            .accounts
            .iter()
            .any(|account| account.provider == provider)
        {
            registry.accounts.push(crate::accounts::Account {
                id: provider_key(provider)?.into(),
                provider,
                revision: 0,
                auth_method: Some("external".into()),
                key_suffix: None,
                identity: Default::default(),
            });
        }
        registry.active.insert(
            provider_key(provider)?.into(),
            provider_key(provider)?.into(),
        );
        Ok(())
    })
    .unwrap();
    let root = std::env::current_dir().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/Prometeu");
    assert!(
        helper.is_file(),
        "build the desktop helper before running native smoke"
    );
    let mut settings = Launch {
        agent: provider,
        permission: Some(Permission::Ask),
        mcp: None,
        plugins: None,
        skills: None,
        ..Default::default()
    };
    if let Ok(catalog) =
        crate::agents::models(&prometeu_process::query::UnixQueryLauncher, provider)
    {
        let preferred = match provider {
            ProviderId::Claude => "sonnet",
            ProviderId::Antigravity => "flash",
            _ => "",
        };
        if let Some(model) = catalog
            .models
            .iter()
            .find(|model| !preferred.is_empty() && model.id.contains(preferred))
            .or_else(|| catalog.models.iter().find(|model| !model.additional))
        {
            settings.model = model.id.clone();
            if provider != ProviderId::Cursor {
                settings.effort = model
                    .efforts
                    .iter()
                    .find(|effort| effort.as_str() == "low")
                    .or_else(|| model.efforts.first())
                    .cloned()
                    .unwrap_or_default();
            }
        }
    }
    println!(
        "native smoke provider={} configured_model={} configured_effort={}",
        provider_key(provider).unwrap(),
        settings.model,
        settings.effort
    );
    if std::env::var_os("PROMETEU_SMOKE_TOOLS").is_none()
        && (provider == ProviderId::Antigravity
            || (provider == ProviderId::Cursor
                && std::env::var_os("PROMETEU_SMOKE_NATIVE_RULES").is_some()))
    {
        settings.instructions =
            format!("For the native rule smoke check, reply exactly {MARKER}. Do not use tools.");
    }
    let mut request = LaunchRequest {
        session: uuid::Uuid::new_v4().to_string(),
        workspace: uuid::Uuid::new_v4().to_string(),
        worktree: workspace.to_string_lossy().into_owned(),
        settings,
        mode: StartMode::Fresh,
    };
    let drafts_only = name.ends_with("_paste_rendering") || name.ends_with("_composer_styles");
    let profile = captured.as_ref().map(|(_, profile)| profile);
    let first = run_one(&request, &helper, drafts_only, profile, None).unwrap();
    if drafts_only {
        assert_eq!(
            first.draft_captures,
            if name.ends_with("_composer_styles") {
                1
            } else {
                draft_samples().len()
            }
        );
        return;
    }
    assert!(
        first.passed(),
        "fresh native evidence incomplete: {}",
        first.summary()
    );
    if std::env::var_os("PROMETEU_SMOKE_SINGLE_TURN").is_some() {
        return;
    }
    request.mode = StartMode::Resume {
        provider_session: first.native_session.clone(),
    };
    if !request.settings.instructions.is_empty() {
        request.settings.instructions = "For the native rule smoke check, reply exactly PROMETEU_TERMINAL_SMOKE_RESUME. Do not use tools.".into();
    }
    let resumed = run_one(
        &request,
        &helper,
        false,
        profile,
        first.transcript.as_deref(),
    )
    .unwrap();
    assert_eq!(
        resumed.native_session, first.native_session,
        "native resume changed identity"
    );
    assert!(
        resumed.passed(),
        "resumed native evidence incomplete: {}",
        resumed.summary()
    );
}

pub(crate) fn captured_profile(
    provider: ProviderId,
) -> Result<Option<(crate::accounts::Account, crate::accounts::Profile)>, String> {
    use prometeu_core::accounts::AccountStore;
    let Some(root) = std::env::var_os("PROMETEU_SMOKE_SELECTED_ACCOUNT_ROOT").map(PathBuf::from)
    else {
        return Ok(None);
    };
    if provider != ProviderId::Claude || !root.is_absolute() {
        return Err("selected-account smoke requires an absolute Claude account root".into());
    }
    let registry = crate::account_store::FileAccountStore::new(root.clone())
        .load()?
        .ok_or("selected-account registry is absent")?;
    registry.validate()?;
    let id = registry
        .active
        .get("claude")
        .ok_or("selected Claude account is absent")?;
    let mut account = registry
        .accounts
        .iter()
        .find(|account| account.id == *id && account.provider == provider)
        .cloned()
        .ok_or("selected Claude account is not registered")?;
    if id == "claude" {
        return Err("selected-account smoke expects an existing managed profile".into());
    }
    let profile = crate::accounts::Profile {
        id: account.id.clone(),
        provider,
        revision: account.revision,
        managed: true,
        home: root.join("accounts").join(id),
    };
    if !profile.home.is_dir() {
        return Err("selected-account profile is absent".into());
    }
    // The isolated test registry needs selection metadata, never identity details or credentials.
    account.identity = Default::default();
    println!("native smoke captured the existing selected managed account");
    Ok(Some((account, profile)))
}

pub(crate) struct SmokeProfiles {
    pub(crate) selected: Option<crate::accounts::Profile>,
    pub(crate) native: prometeu_profiles::NativeProfiles,
}
impl prometeu_profiles::ProfileBackend for SmokeProfiles {
    fn resolve(
        &self,
        account: &crate::accounts::Account,
    ) -> Result<crate::accounts::Profile, String> {
        match &self.selected {
            Some(profile) if profile.id == account.id && profile.provider == account.provider => {
                Ok(profile.clone())
            }
            Some(_) => Err("smoke account selection changed".into()),
            None => self.native.resolve(account),
        }
    }
    fn prepare(&self, profile: &crate::accounts::Profile) -> Result<(), String> {
        if self.selected.is_some() {
            // Use the already materialized profile in place. Do not copy authentication or
            // rewrite the person's managed settings merely to run this isolated smoke.
            if profile.home.is_dir() {
                Ok(())
            } else {
                Err("captured profile disappeared".into())
            }
        } else {
            self.native.prepare(profile)
        }
    }
    fn apply(
        &self,
        profile: &crate::accounts::Profile,
        command: &mut Command,
    ) -> Result<(), String> {
        self.native.apply(profile, command)
    }
}

struct SmokeClock;
impl prometeu_core::conversation::Clock for SmokeClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[derive(Default)]
struct Evidence {
    draft_captures: usize,
    native_session: Option<String>,
    transcript: Option<PathBuf>,
    hook_kinds: Vec<NativeHookKind>,
    hook_metadata: Vec<Value>,
    assistant_response: bool,
    stop_observed: bool,
    completion_required: bool,
    completion_proven: bool,
    output_bytes: usize,
    reports: usize,
    queries: usize,
    statuses: Vec<String>,
    followup_submitted: bool,
    trust_confirmed: bool,
    classification: Vec<&'static str>,
}
impl Evidence {
    fn passed(&self) -> bool {
        self.native_session.is_some()
            && self.transcript.is_some()
            && self.assistant_response
            && self.stop_observed
            && (!self.completion_required || self.completion_proven)
    }
    fn summary(&self) -> String {
        format!("identity={} transcript={} assistant_response={} stop_observed={} completion_proven={} followup_submitted={} hooks={:?} terminal_reports={} terminal_queries={} statuses={:?} output_bytes={} trust_confirmed={} classifications={:?}", self.native_session.is_some(),self.transcript.is_some(),self.assistant_response,self.stop_observed,self.completion_proven,self.followup_submitted,self.hook_kinds,self.reports,self.queries,self.statuses,self.output_bytes,self.trust_confirmed,self.classification)
    }
}

fn run_one(
    request: &LaunchRequest,
    helper: &Path,
    drafts_only: bool,
    profile: Option<&crate::accounts::Profile>,
    resume_path: Option<&Path>,
) -> Result<Evidence, String> {
    let schema_helper = if std::env::var_os("PROMETEU_SMOKE_HOOK_SCHEMA").is_some() {
        Some(hook_schema_helper(helper)?)
    } else {
        None
    };
    let helper = schema_helper.as_deref().unwrap_or(helper);
    println!("native preparation starting");
    let owned = LaunchRequest {
        session: request.session.clone(),
        workspace: request.workspace.clone(),
        worktree: request.worktree.clone(),
        settings: request.settings.clone(),
        mode: request.mode.clone(),
    };
    let (sender, result) = mpsc::channel();
    let profile = profile.cloned();
    std::thread::spawn(move || {
        let profiles = SmokeProfiles {
            selected: profile,
            native: crate::account_profiles::native(),
        };
        let prepared = crate::terminal_agent_launch::prepare(
            &owned,
            &profiles,
            &crate::tool_materialization::native(),
        );
        let _ = sender.send(prepared);
    });
    let mut prepared = result
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "native preparation deadline".to_owned())??;
    if std::env::var_os("PROMETEU_SMOKE_PRIVATE_CLAUDE_CONFIG").is_some() {
        if request.settings.agent != ProviderId::Claude || !prepared.profile.managed {
            return Err(
                "private Claude smoke configuration requires its captured managed account".into(),
            );
        }
        let directory = paths::root().join("native-claude-config");
        paths::ensure_private_dir(&directory)
            .map_err(|_| "private native config directory failed")?;
        // The verified build separates credential storage from native preferences. These are
        // synthetic local UI preferences, not copied account state; workspace trust stays unset.
        paths::write_private(
            &directory.join(".claude.json"),
            &json!({"theme":"dark", "hasCompletedOnboarding":true}).to_string(),
        )
        .map_err(|_| "private native preferences failed")?;
        prepared
            .command
            .env("CLAUDE_CONFIG_DIR", &directory)
            .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", &prepared.profile.home);
        println!("native smoke uses private preferences and the existing credential directory; tool inheritance is not under test");
    }
    println!("native preparation complete; hook configuration starting");
    let mut marker = if matches!(request.mode, StartMode::Fresh) {
        MARKER
    } else {
        "PROMETEU_TERMINAL_SMOKE_RESUME"
    };
    let prompt = if std::env::var_os("PROMETEU_SMOKE_TOOLS").is_some() {
        if !matches!(request.mode, StartMode::Fresh)
            || !request.settings.instructions.is_empty()
            || std::env::var_os("PROMETEU_SMOKE_SINGLE_TURN").is_none()
        {
            return Err(
                "tool smoke requires a fresh single turn without additional instructions".into(),
            );
        }
        TOOL_PROMPT.to_owned()
    } else if !request.settings.instructions.is_empty() {
        "Perform the configured native rule smoke check. Do not use tools.".to_owned()
    } else if matches!(request.mode, StartMode::Fresh) {
        PROMPT.to_owned()
    } else {
        format!("Reply exactly {marker}. Do not use tools.")
    };
    let (mut bridge, hooks) = HookBridge::start(
        request.settings.agent,
        &request.session,
        &uuid::Uuid::new_v4().to_string(),
        prepared.provider_session.as_deref(),
    )?;
    let hook_command = if let Some(engine) = prepared.engine.as_mut() {
        &mut engine.command
    } else {
        &mut prepared.command
    };
    if let Err(error) =
        bridge.configure_with_helper(hook_command, prepared.hook_probe.as_mut(), helper)
    {
        if let Some(probe) = prepared.hook_probe.as_mut() {
            if let Ok(inventory) = codex_hooks_list(probe) {
                let metadata=inventory["data"].as_array().into_iter().flatten()
                    .flat_map(|row|row["hooks"].as_array().into_iter().flatten())
                    .filter(|hook|hook["source"]=="sessionFlags")
                    .map(|hook|json!({"event":hook["eventName"],"trusted":hook["trustStatus"],"enabled":hook["enabled"],"managed":hook["isManaged"],"timeout":hook["timeoutSec"],"source_path_matches":hook["sourcePath"]=="/<session-flags>/config.toml"}))
                    .collect::<Vec<_>>();
                println!("native hook preflight metadata: {}", json!(metadata));
            } else {
                println!("native hook preflight RPC rejected");
            }
        }
        return Err(error);
    }
    if let Some(instructions) = prepared.initial_instructions.take() {
        bridge.configure_instructions(&mut prepared.command, &instructions)?;
        println!("native instructions configured before first prompt");
    }
    let engine = prepared
        .engine
        .take()
        .map(|plan| plan.start(&mut prepared.command))
        .transpose()?;
    let engine_events = engine.as_ref().and_then(|engine| engine.take_events());
    if let Some(engine) = engine.as_ref() {
        prepared.provider_session = Some(engine.thread_id().to_owned());
    }
    let completion_required = std::env::var_os("PROMETEU_SMOKE_COMPLETION_FENCE").is_some();
    let source_batches = Arc::new(Mutex::new(VecDeque::new()));
    let source_journal = if completion_required && request.settings.agent == ProviderId::Cursor {
        let mut witness = crate::terminal_agent_observation::CursorStopWitness::new(
            &prepared.observation_roots()?,
        )
        .map_err(|error| error.code().to_owned())?;
        bridge.set_cursor_stop_capture(Box::new(move |event| witness.capture(event)));
        Some(crate::chat::terminal::ingestion::JournalStore::open(
            &paths::root().join("native-source.jsonl"),
            &paths::root().join("native-source.sqlite"),
            crate::terminal_agent_journal::JournalBinding {
                app_session: request.session.clone(),
                provider: "cursor".into(),
                native_session: prepared.provider_session.clone(),
            },
            matches!(request.mode, StartMode::Fresh),
        )?)
    } else {
        None
    };
    let mut observation = if completion_required {
        use crate::terminal_agent_observation::{
            ObservationCallbacks, ObservationConfig, ObservationStart, TerminalAgentObservation,
        };
        let mut observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider: request.settings.agent,
                session: request.session.clone(),
                process_nonce: bridge.identity.nonce.clone(),
                expected_native_session: prepared.provider_session.clone(),
                start: if matches!(request.mode, StartMode::Fresh) {
                    ObservationStart::Fresh
                } else {
                    ObservationStart::Resume
                },
                authorized_roots: prepared.observation_roots()?,
            },
            ObservationCallbacks {
                live: Box::new(|_| {}),
                native_baseline: Box::new(|_| {}),
                accepted_prompt: Box::new(|_| {}),
                observed_hook: Box::new(|_| {}),
                warning: Box::new(|code| println!("native observation warning={code}")),
            },
            Arc::new(SmokeClock),
        )
        .map_err(|error| error.code().to_owned())?;
        if request.settings.agent == ProviderId::Cursor {
            observer
                .configure_cursor_store(
                    crate::terminal_agent_observation::CursorObservationConfig {
                        chats_root: prepared
                            .cursor_store_root()
                            .ok_or("missing captured Cursor source root")?,
                        cli_version: bridge.native_version().unwrap_or_default().into(),
                        staging_root: paths::root().join("native-source-snapshots"),
                    },
                )
                .map_err(|error| error.code().to_owned())?;
            let batches = source_batches.clone();
            observer
                .set_ingestion(Box::new(move |batch| {
                    crate::lock::lock(&batches).push_back(batch)
                }))
                .map_err(|error| error.code().to_owned())?;
        } else if !observer.configure_completion(bridge.native_version().unwrap_or_default()) {
            return Err("native completion fence is not verified for this build".into());
        }
        if let Some(path) = resume_path {
            observer
                .resume_from(path.into())
                .map_err(|error| error.code().to_owned())?;
            observer.poll().map_err(|error| error.code().to_owned())?;
            if !observer.baseline_ready() {
                return Err("native resume baseline incomplete".into());
            }
        }
        if request.settings.agent != ProviderId::Cursor {
            observer
                .expect_submission(&prompt, "smoke-initial")
                .map_err(|error| error.code().to_owned())?;
        }
        Some(observer)
    } else {
        None
    };
    let ui_readiness = bridge.native_version().and_then(|version| {
        crate::native_ui_readiness::NativeUiReadiness::verified(request.settings.agent, version)
    });
    let styles_only = request.settings.agent == ProviderId::Claude
        && std::env::var_os("PROMETEU_SMOKE_COMPOSER_STYLES").is_some();
    if styles_only {
        prepared
            .command
            .env_remove("NO_COLOR")
            .env("FORCE_COLOR", "1");
    }
    let ui_input = ui_readiness.is_some()
        && (drafts_only
            || request.settings.agent == ProviderId::Codex
            || std::env::var_os("PROMETEU_SMOKE_UI_INPUT").is_some());
    if request.settings.agent == ProviderId::Codex && !ui_input {
        return Err("native Codex smoke requires verified remote composer readiness".into());
    }
    let mut initial_submitted = !ui_input;
    let mut pending_enter: Option<String> = None;
    let mut tool_approved = false;
    let mut pending_trust: Option<(Instant, bool)> = None;
    let draft_samples = draft_samples();
    let mut draft_case = 0;
    let mut draft_stage = 0;
    let mut draft_since = Instant::now();
    let mut draft_base_seq = 0;
    let mut draft_probe_phase =
        if ui_input && std::env::var_os("PROMETEU_CAPTURE_NATIVE_DRAFT").is_some() {
            0
        } else {
            3
        };
    let mut draft_probe_styles = None;
    if request.settings.agent == ProviderId::Antigravity {
        prepared.command.args(["--prompt-interactive", &prompt]);
    } else if !ui_input {
        prepared.command.arg(&prompt);
    }
    let mut command = portable_pty::CommandBuilder::new(prepared.command.get_program());
    prometeu_process::terminal::own_terminal(&mut command, Some(env!("CARGO_PKG_VERSION")));
    command.args(prepared.command.get_args());
    command.cwd(&request.worktree);
    for (key, value) in prepared.command.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    use prometeu_process::terminal_state::{Client, Key, Size, Stream};
    let screen_helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/terminal-state/dist")
        .join(format!(
            "prometeu-terminal-state-{}-apple-darwin",
            std::env::consts::ARCH
        ));
    let mut emulator = Client::launch(Command::new(screen_helper), Duration::from_secs(5))
        .map_err(|_| "native smoke screen helper unavailable".to_owned())?;
    let mut stream = Stream::create(
        &mut emulator,
        Key {
            session: request.session.clone(),
            generation: uuid::Uuid::new_v4().to_string(),
        },
        Size {
            cols: 120,
            rows: 40,
        },
        1000,
    )
    .map_err(|_| "native smoke screen creation failed".to_owned())?;
    let mut chunk_seq = 0;
    let mut terminal = SmokeTerminal::open(command)?;
    println!("native PTY started");
    let mut evidence = Evidence {
        completion_required,
        ..Default::default()
    };
    let mut screen = Vec::new();
    let mut parser = ProgramStatusParser::default();
    let mut status = ProgramStatusStore::default();
    let timeout = std::env::var("PROMETEU_SMOKE_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| (1..=45).contains(seconds))
        .unwrap_or(if styles_only { 12 } else { 45 });
    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut response_observed_at: Option<Instant> = None;
    while Instant::now() < deadline {
        match terminal.output.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                evidence.output_bytes += bytes.len();
                chunk_seq += 1;
                for event in parser.feed(&bytes) {
                    match event {
                        ProgramStatusEvent::Query => {
                            evidence.queries += 1;
                            terminal
                                .input
                                .as_mut()
                                .ok_or("native input closed")?
                                .write_all(PROGRAM_STATUS_QUERY_REPLY)
                                .map_err(|_| "native status reply failed".to_owned())?;
                        }
                        ProgramStatusEvent::Report(report) => {
                            if let Some(observer) = observation.as_mut() {
                                observer.observe_program_status(&report);
                            }
                            evidence.reports += 1;
                            evidence.statuses.push(format!(
                                "{}:{:?}:{:?}",
                                if report.id.is_none() { "root" } else { "child" },
                                report.state,
                                report.kind
                            ));
                            status.apply(report);
                        }
                    }
                }
                let applied = stream
                    .write_chunk(&mut emulator, chunk_seq, &bytes)
                    .map_err(|_| "native screen helper write failed".to_owned())?;
                if !applied.replies.is_empty() {
                    println!(
                        "native terminal capability replies={}",
                        applied.replies.len()
                    );
                }
                for reply in applied.replies {
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(&reply)
                        .map_err(|_| "native capability reply failed".to_owned())?;
                }
                if std::env::var_os("PROMETEU_CAPTURE_NATIVE_UI").is_some() {
                    let snapshot = stream
                        .snapshot(&mut emulator)
                        .map_err(|_| "native fixture snapshot failed".to_owned())?;
                    let visible = snapshot.state.viewport_lines.join("\n").to_lowercase();
                    let phase = if visible.contains("trust") && !evidence.trust_confirmed {
                        Some("trust")
                    } else if visible.contains("esc to interrupt")
                        || visible
                            .chars()
                            .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
                    {
                        Some("busy")
                    } else {
                        None
                    };
                    if let Some(phase) = phase {
                        capture_ui(
                            request,
                            bridge.native_version().unwrap_or("unknown"),
                            phase,
                            &snapshot.state,
                        )?;
                    }
                }
                screen.extend(bytes);
                if screen.len() > 524_288 {
                    screen.drain(..screen.len() - 524_288);
                }
                if let Some(input) = (!evidence.trust_confirmed && pending_trust.is_none())
                    .then(|| {
                        scratch_trust_prompt(&screen, &request.worktree, request.settings.agent)
                    })
                    .flatten()
                {
                    println!(
                        "native scratch trust action={}",
                        if input.len() > 1 {
                            "select-yes-confirm"
                        } else {
                            "confirm"
                        }
                    );
                    let writer = terminal.input.as_mut().ok_or("native input closed")?;
                    if input == b"\x1b[B\r" {
                        // Keep capability queries flowing during the native trust key guard.
                        // Selection and confirmation remain separate native input events.
                        pending_trust = Some((Instant::now() + Duration::from_millis(1200), false));
                    } else {
                        writer
                            .write_all(input)
                            .map_err(|_| "native trust input failed".to_owned())?;
                        evidence.trust_confirmed = true;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some((ready_at, selected)) = pending_trust {
            if Instant::now() >= ready_at {
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(if selected { b"\r" } else { b"\x1b[B" })
                    .map_err(|_| "native trust input failed".to_owned())?;
                if selected {
                    pending_trust = None;
                    evidence.trust_confirmed = true;
                } else {
                    pending_trust = Some((Instant::now() + Duration::from_millis(100), true));
                }
            }
        }
        while let Ok(event) = hooks.try_recv() {
            if evidence.hook_metadata.len() < 64 {
                evidence.hook_metadata.push(json!({
                    "ordinal": evidence.hook_metadata.len(),
                    "kind": format!("{:?}", event.kind),
                    "generation_id": event.generation_id,
                    "step_index": event.step_index,
                    "initial_num_steps": event.initial_num_steps,
                    "invocation_number": event.invocation_number,
                    "execution_number": event.execution_number,
                    "fully_idle": event.fully_idle,
                    "continuation_active": event.continuation_active,
                    "reason": event.reason,
                    "prompt_matches": event.prompt.as_deref() == Some(prompt.as_str()),
                    "text_matches": event.text.as_deref() == Some(marker),
                    "text_bytes": event.text.as_ref().map(String::len),
                    "text_trim_matches": event.text.as_deref().is_some_and(|text| text.trim() == marker),
                }));
            }
            if let Some(observer) = observation.as_mut() {
                observer
                    .accept_hook(event.clone())
                    .map_err(|error| error.code().to_owned())?;
            }
            if event.agent_id.is_some() {
                continue;
            }
            if let Some(expected) = &evidence.native_session {
                if expected != &event.native_session {
                    return Err("native identity changed".into());
                }
            }
            evidence.native_session = Some(event.native_session.clone());
            if let Some(path) = event.transcript_path {
                if !path.is_absolute() || !path.to_string_lossy().contains(&event.native_session) {
                    return Err(
                        "native hook did not authorize a session-specific transcript".into(),
                    );
                }
                evidence.transcript = Some(path);
            }
            if !evidence.hook_kinds.contains(&event.kind) {
                println!("native hook observed {:?}", event.kind);
                evidence.hook_kinds.push(event.kind);
            }
            if event.kind == NativeHookKind::AssistantResponse {
                if let Some(text) = event.text.as_deref() {
                    println!(
                        "native assistant metadata bytes={} contains_expected_marker={}",
                        text.len(),
                        text.contains(marker)
                    );
                }
            }
            if event.kind == NativeHookKind::StopObserved {
                evidence.stop_observed = true;
            }
            if matches!(
                event.kind,
                NativeHookKind::AssistantResponse | NativeHookKind::StopObserved
            ) && event
                .text
                .as_deref()
                .is_some_and(|text| text.trim() == marker)
            {
                evidence.assistant_response = true;
            }
        }
        if let Some(events) = engine_events.as_ref() {
            while let Ok(event) = events.try_recv() {
                event?;
            }
        }
        if styles_only {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native style snapshot failed".to_owned())?;
            let protocol_idle = status
                .snapshot()
                .root
                .is_some_and(|report| report.state == ProgramStatusState::Idle);
            // The status protocol is authoritative here; a PSP-aware Claude omits the title prefix.
            // This fixture-only path never sends Enter. The exact native signed-out footer
            // does not prevent local draft editing; production admission continues to veto it.
            let mut draft_view = snapshot.state.clone();
            for line in &mut draft_view.viewport_lines {
                if line.trim() == "Not logged in · Run /login" {
                    line.clear();
                }
            }
            let ready = protocol_idle
                && evidence.native_session.is_some()
                && ui_readiness.is_some_and(|check| check.ready_with_native_idle(&draft_view));
            let cursor = match snapshot.state.active {
                prometeu_process::terminal_state::protocol::BufferKind::Normal => {
                    &snapshot.state.normal.cursor
                }
                prometeu_process::terminal_state::protocol::BufferKind::Alternate => {
                    &snapshot
                        .state
                        .alternate
                        .as_ref()
                        .ok_or("missing native alternate screen")?
                        .cursor
                }
            };
            match draft_stage {
                0 if ready => {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "empty-placeholder",
                        &snapshot.state,
                    )?;
                    let literal = snapshot.state.viewport_lines[usize::from(cursor.y)]
                        .trim()
                        .trim_start_matches('❯')
                        .trim();
                    if literal.is_empty() {
                        continue;
                    }
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(format!("\x1b[200~{literal}\x1b[201~").as_bytes())
                        .map_err(|_| "style paste failed")?;
                    draft_base_seq = snapshot.state.seq;
                    draft_stage = 1;
                }
                1 if snapshot.state.seq > draft_base_seq && cursor.x > 2 => {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "literal-draft",
                        &snapshot.state,
                    )?;
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(b"\x01")
                        .map_err(|_| "style home failed")?;
                    draft_base_seq = snapshot.state.seq;
                    draft_stage = 2;
                }
                2 if snapshot.state.seq > draft_base_seq && cursor.x == 2 => {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "literal-home",
                        &snapshot.state,
                    )?;
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(b"\x05\x15")
                        .map_err(|_| "style clear failed")?;
                    draft_base_seq = snapshot.state.seq;
                    draft_stage = 3;
                }
                3 if snapshot.state.seq > draft_base_seq && ready => {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "cleared-draft",
                        &snapshot.state,
                    )?;
                    evidence.draft_captures = 1;
                    break;
                }
                _ => {}
            }
            continue;
        }
        if drafts_only {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native draft snapshot failed".to_owned())?;
            let ready = ui_readiness.is_some_and(|check| check.ready(&snapshot.state));
            match draft_stage {
                0 if ready => {
                    if draft_case == 0 {
                        capture_ui(
                            request,
                            bridge.native_version().unwrap_or("unknown"),
                            "empty-placeholder",
                            &snapshot.state,
                        )?;
                    }
                    let (_, text) = &draft_samples[draft_case];
                    let input = format!("\x1b[200~{text}\x1b[201~");
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(input.as_bytes())
                        .map_err(|_| "native draft capture paste failed".to_owned())?;
                    draft_base_seq = snapshot.state.seq;
                    draft_since = Instant::now();
                    draft_stage = 1;
                }
                1 if snapshot.state.seq > draft_base_seq
                    && snapshot.state.pending_escape.is_empty() =>
                {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        draft_samples[draft_case].0,
                        &snapshot.state,
                    )?;
                    // This dwell only collects a test fixture. It never grants admission or
                    // submits Enter; every synthetic draft is cleared without model inference.
                    if draft_since.elapsed() > Duration::from_millis(300) {
                        draft_stage = 2;
                    }
                }
                2 if ready => {
                    evidence.draft_captures += 1;
                    draft_case += 1;
                    if draft_case == draft_samples.len() {
                        break;
                    }
                    draft_stage = 0;
                }
                2 => {
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(b"\x15")
                        .map_err(|_| "native draft capture clear failed".to_owned())?;
                }
                _ => {}
            }
            continue;
        }
        if let Some(expected) = pending_enter.as_ref() {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native input acknowledgement snapshot failed".to_owned())?;
            if ui_readiness
                .is_some_and(|readiness| readiness.paste_applied(&snapshot.state, expected))
            {
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(b"\r")
                    .map_err(|_| "native input Enter failed".to_owned())?;
                pending_enter = None;
                println!("native Enter submitted after exact draft paint");
            }
        }
        if request.settings.agent == ProviderId::Cursor
            && bridge.native_version() == Some("2026.11.13-eacade1")
            && prompt == TOOL_PROMPT
            && !tool_approved
            && initial_submitted
            && pending_enter.is_none()
        {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native tool approval snapshot failed".to_owned())?;
            let lines = &snapshot.state.viewport_lines;
            let command = lines.iter().rposition(|line| {
                line.split_whitespace().collect::<Vec<_>>().join(" ")
                    == "$ printf PROMETEU_TERMINAL_TOOL_SMOKE in ."
            });
            if command.is_some_and(|row| {
                lines[row + 1..]
                    .iter()
                    .any(|line| line.trim() == "→ Run (once) (y)")
            }) {
                // This explicit smoke permits only its fixed, side-effect-free command once.
                // The native permission policy and persistent allowlist remain in force.
                if std::env::var_os("PROMETEU_CAPTURE_NATIVE_UI").is_some() {
                    capture_ui(
                        request,
                        "2026.11.13-eacade1",
                        "tool-approval",
                        &snapshot.state,
                    )?;
                }
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(b"y")
                    .map_err(|_| "native smoke approval failed".to_owned())?;
                tool_approved = true;
                println!("native smoke approved its exact printf command once");
            }
        }
        if request.settings.agent == ProviderId::Antigravity
            && bridge.native_version() == Some("1.5.10")
            && prompt == TOOL_PROMPT
            && !tool_approved
            && initial_submitted
        {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native tool approval snapshot failed".to_owned())?;
            let lines = &snapshot.state.viewport_lines;
            let clean = |line: &str| line.trim().trim_matches(['│', '┃']).trim().to_owned();
            let exact_command = lines.iter().any(|line| {
                matches!(
                    clean(line).as_str(),
                    "printf PROMETEU_TERMINAL_TOOL_SMOKE"
                        | "$ printf PROMETEU_TERMINAL_TOOL_SMOKE"
                        | "Command: printf PROMETEU_TERMINAL_TOOL_SMOKE"
                )
            });
            let first_choice = lines.iter().any(|line| {
                clean(line).trim_start_matches(['>', '›', '❯', '→']).trim() == "1. Yes, run command"
            });
            let ordinary_request = lines.iter().any(|line| line.contains("Run this command?"))
                && !lines.iter().any(|line| {
                    line.contains("sandbox bypass") || line.contains("administrator elevation")
                });
            if exact_command && first_choice && ordinary_request {
                // ToolConfirmationModel starts at choice zero; choices() on 1.5.10 puts the
                // one-shot run there. No other input has changed this test-owned dialog.
                capture_ui(request, "1.5.10", "tool-approval", &snapshot.state)?;
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(b"\r")
                    .map_err(|_| "native smoke approval failed".to_owned())?;
                tool_approved = true;
                println!("native smoke approved its exact printf command once");
            }
        }
        if !initial_submitted {
            let snapshot = stream
                .snapshot(&mut emulator)
                .map_err(|_| "native readiness snapshot failed".to_owned())?;
            if draft_probe_phase == 1 {
                if draft_probe_styles.as_ref() != Some(&snapshot.state.viewport_styles) {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "literal-draft",
                        &snapshot.state,
                    )?;
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(b"\x15")
                        .map_err(|_| "native draft clear failed".to_owned())?;
                    draft_probe_phase = 2;
                }
                continue;
            }
            if ui_readiness.is_some_and(|readiness| readiness.ready(&snapshot.state)) {
                if draft_probe_phase == 0 {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "empty-placeholder",
                        &snapshot.state,
                    )?;
                    let line = snapshot
                        .state
                        .viewport_lines
                        .iter()
                        .rfind(|line| line.trim().starts_with(['❯', '→']))
                        .ok_or("native composer missing")?;
                    let literal = line.trim().trim_start_matches(['❯', '→']).trim();
                    if literal.is_empty() {
                        return Err("native draft probe requires a placeholder".into());
                    }
                    draft_probe_styles = Some(snapshot.state.viewport_styles.clone());
                    let draft = format!("\x1b[200~{literal}\x1b[201~");
                    terminal
                        .input
                        .as_mut()
                        .ok_or("native input closed")?
                        .write_all(draft.as_bytes())
                        .map_err(|_| "native literal draft failed".to_owned())?;
                    draft_probe_phase = 1;
                    continue;
                }
                if draft_probe_phase == 2 {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "cleared-draft",
                        &snapshot.state,
                    )?;
                    draft_probe_phase = 3;
                    println!("native placeholder/literal/clear styles captured without submission");
                }
                if std::env::var_os("PROMETEU_CAPTURE_NATIVE_UI").is_some() {
                    capture_ui(
                        request,
                        bridge.native_version().unwrap_or("unknown"),
                        "initial-ready",
                        &snapshot.state,
                    )?;
                }
                let input = if request.settings.agent == ProviderId::Cursor {
                    pending_enter = Some(prompt.clone());
                    format!("\x1b[200~{prompt}\x1b[201~")
                } else {
                    format!("\x1b[200~{prompt}\x1b[201~\r")
                };
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(input.as_bytes())
                    .map_err(|_| "native initial UI submission failed".to_owned())?;
                initial_submitted = true;
                println!("native first prompt submitted after verified UI composer");
            }
        }
        if let Some(observer) = observation.as_mut() {
            observer.poll().map_err(|error| error.code().to_owned())?;
            if let Some(journal) = source_journal.as_ref() {
                if let Some(native) = observer.native_session() {
                    journal.bind_native(native)?;
                }
                while let Some(mut batch) = crate::lock::lock(&source_batches).pop_front() {
                    // Persist the actual source using the shared storage-only ingestion path.
                    // This harness does not claim to exercise Tauri publication or app echoes.
                    batch.phase = prometeu_protocols::terminal_transcript::TranscriptPhase::Replay;
                    journal.replay(&batch)?;
                    observer
                        .acknowledge_ingestion(&batch.id)
                        .map_err(|error| error.code().to_owned())?;
                }
                if let Some(proof) = observer
                    .cursor_completion()
                    .map_err(|error| error.code().to_owned())?
                {
                    let snapshot = stream
                        .snapshot(&mut emulator)
                        .map_err(|_| "native completion snapshot failed")?;
                    if ui_readiness.is_some_and(|ready| ready.ready(&snapshot.state)) {
                        use prometeu_core::conversation::stream::TranscriptStore;
                        let offset = journal.load()?.lines().count() as u64;
                        let frame = prometeu_core::conversation::event(
                            "turn.completed",
                            1,
                            json!({"outcome":"ok"}),
                        );
                        journal.commit_completion(
                            &proof.native_source,
                            &frame.to_string(),
                            offset,
                        )?;
                        observer.acknowledge_cursor_completion(&proof.request_id);
                        evidence.completion_proven = true;
                        println!("native Cursor source committed with pre-ACK stop, persisted success marker and empty composer");
                    }
                }
            }
            if observer.take_persisted_completion().is_some() {
                evidence.completion_proven = true;
                println!("native final source and exact Done report passed the runtime persistence fence");
            }
        }
        if let Some(path) = &evidence.transcript {
            evidence.assistant_response |=
                transcript_has_response(path, request.settings.agent, marker);
        }
        if evidence.passed() {
            let observed = response_observed_at.get_or_insert_with(Instant::now);
            let protocol_ready = status.snapshot().root.is_some_and(|report| {
                matches!(
                    report.state,
                    ProgramStatusState::Idle | ProgramStatusState::Done
                )
            });
            let ui_ready = if let Some(readiness) = ui_readiness {
                let snapshot = stream
                    .snapshot(&mut emulator)
                    .map_err(|_| "native readiness snapshot failed".to_owned())?;
                readiness.ready(&snapshot.state)
            } else {
                false
            };
            let ready = protocol_ready || ui_ready;
            if ready
                && std::env::var_os("PROMETEU_SMOKE_SINGLE_TURN").is_none()
                && !evidence.followup_submitted
                && matches!(request.mode, StartMode::Fresh)
                && request.settings.instructions.is_empty()
            {
                marker = "PROMETEU_TERMINAL_SMOKE_FOLLOWUP";
                let text = format!("Reply exactly {marker}. Do not use tools.");
                if let Some(observer) = observation.as_mut() {
                    observer
                        .expect_submission(&text, "smoke-followup")
                        .map_err(|error| error.code().to_owned())?;
                    evidence.completion_proven = false;
                }
                let followup = if request.settings.agent == ProviderId::Cursor {
                    pending_enter = Some(text.clone());
                    format!("\x1b[200~{text}\x1b[201~")
                } else {
                    format!("\x1b[200~{text}\x1b[201~\r")
                };
                terminal
                    .input
                    .as_mut()
                    .ok_or("native input closed")?
                    .write_all(followup.as_bytes())
                    .map_err(|_| "native followup failed".to_owned())?;
                evidence.followup_submitted = true;
                evidence.assistant_response = false;
                evidence.stop_observed = false;
                response_observed_at = None;
                println!("native followup submitted after explicit status or verified UI composer");
            } else if ready || observed.elapsed() > Duration::from_secs(1) {
                break;
            }
        }
    }
    if std::env::var_os("PROMETEU_CAPTURE_NATIVE_UI").is_some() {
        let snapshot = stream
            .snapshot(&mut emulator)
            .map_err(|_| "native fixture snapshot failed".to_owned())?;
        capture_ui(
            request,
            bridge.native_version().unwrap_or("unknown"),
            if ui_readiness.is_some_and(|ready| ready.ready(&snapshot.state)) {
                "rest"
            } else {
                "final"
            },
            &snapshot.state,
        )?;
    }
    println!("native PTY closing");
    terminal.close();
    if let Some(engine) = engine.as_ref() {
        engine.stop();
        if engine.running() {
            return Err("native engine did not stop".into());
        }
    }
    println!("native PTY closed; bridge closing");
    bridge.close();
    stream
        .dispose(&mut emulator)
        .map_err(|_| "native screen dispose failed".to_owned())?;
    emulator
        .shutdown()
        .map_err(|_| "native screen shutdown failed".to_owned())?;
    if request.settings.agent == ProviderId::Claude
        && evidence.trust_confirmed
        && evidence.hook_kinds.is_empty()
    {
        let compact = plain_screen(&screen)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase();
        println!(
            "native trust diagnostics yes_selected={} no_selected={}",
            compact.contains("❯yes,itrustthisfolder"),
            compact.contains("❯no,exit")
        );
    }
    if std::env::var_os("PROMETEU_SMOKE_KEEP_PROOF").is_some() {
        let path = paths::root()
            .parent()
            .ok_or("missing smoke root")?
            .join("native-proof.json");
        let environment = [
            "HOME",
            "CURSOR_CONFIG_DIR",
            "CURSOR_DATA_DIR",
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_SECURESTORAGE_CONFIG_DIR",
        ]
        .into_iter()
        .filter_map(|name| {
            prepared
                .command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| {
                    value.map(|value| (name, value.to_string_lossy().into_owned()))
                })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
        let proof = json!({"app_session":request.session,"provider":provider_key(request.settings.agent)?,"native_session":evidence.native_session,"transcript":evidence.transcript,"worktree":request.worktree,"profile_home":prepared.profile.home,"environment":environment,"version":bridge.native_version(),"test_root":paths::root().parent(),"hooks":evidence.hook_metadata});
        paths::write_private(
            &path,
            &serde_json::to_string_pretty(&proof).map_err(|_| "proof encoding failed")?,
        )
        .map_err(|_| "proof write failed")?;
        println!("native smoke proof path={}", path.display());
    }
    let plain = String::from_utf8_lossy(&screen).to_lowercase();
    for (pattern, classification) in [
        ("trust", "trust"),
        ("sign in", "sign-in"),
        ("log in", "log-in"),
        ("login", "login"),
        ("not authenticated", "not-authenticated"),
        ("model not", "model-unavailable"),
        ("invalid model", "invalid-model"),
        ("unknown model", "unknown-model"),
        ("permission", "permission"),
        ("error", "error"),
        ("connecting", "connecting"),
        ("press enter", "press-enter"),
        ("select", "selection"),
    ] {
        if plain.contains(pattern) {
            evidence.classification.push(classification);
        }
    }
    println!(
        "native phase {}: {}",
        if matches!(request.mode, StartMode::Fresh) {
            "fresh"
        } else {
            "resume"
        },
        evidence.summary()
    );
    Ok(evidence)
}

fn hook_schema_helper(helper: &Path) -> Result<PathBuf, String> {
    let path = paths::root().join("native-hook-schema-helper");
    let output = paths::root().join("native-hook-schema.jsonl");
    let helper = serde_json::to_string(helper.to_str().ok_or("invalid helper path")?)
        .map_err(|_| "helper encoding failed")?;
    let output = serde_json::to_string(output.to_str().ok_or("invalid schema path")?)
        .map_err(|_| "schema path encoding failed")?;
    let script = format!(
        r##"#!/usr/bin/python3
import json, os, subprocess, sys
data = sys.stdin.buffer.read(1048577)
if len(data) > 1048576:
    sys.exit(1)
def schema(value, depth=0):
    if depth > 3:
        return {{"type": type(value).__name__}}
    if isinstance(value, dict):
        return {{key: schema(item, depth+1) for key,item in value.items()}}
    if isinstance(value, str):
        return {{"type":"string", "bytes":len(value.encode()), "marker":value == "PROMETEU_TERMINAL_SMOKE", "trim_marker":value.strip() == "PROMETEU_TERMINAL_SMOKE"}}
    return {{"type":type(value).__name__}}
try:
    payload = json.loads(data)
    line = json.dumps({{"schema":schema(payload)}}).encode()+b"\n"
    fd = os.open({output}, os.O_CREAT|os.O_WRONLY|os.O_APPEND|os.O_NOFOLLOW, 0o600)
    os.write(fd,line)
    os.close(fd)
except (OSError, ValueError):
    sys.exit(1)
result = subprocess.run([{helper},*sys.argv[1:]],input=data)
sys.exit(result.returncode)
"##
    );
    paths::write_private(&path, &script).map_err(|_| "schema helper write failed")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "schema helper permissions failed")?;
    Ok(path)
}

fn capture_ui(
    request: &LaunchRequest,
    version: &str,
    phase: &str,
    snapshot: &prometeu_process::terminal_state::Snapshot,
) -> Result<(), String> {
    use prometeu_process::terminal_state::protocol::BufferKind;
    let screen = match snapshot.active {
        BufferKind::Normal => &snapshot.normal,
        BufferKind::Alternate => snapshot
            .alternate
            .as_ref()
            .ok_or("missing native active screen")?,
    };
    let home = crate::paths::home().to_string_lossy().into_owned();
    let sanitize = |text: &str| {
        text.replace(&request.worktree, "/tmp/native-smoke/workspace")
            .replace(&home, "/home/test-user")
    };
    let lines = snapshot
        .viewport_lines
        .iter()
        .map(|line| {
            let lower = line.to_lowercase();
            let structural = line.trim().is_empty()
                || line.contains("PROMETEU_")
                || lower.contains("synthetic line")
                || lower.contains("são paulo")
                || line.contains("中文")
                || line.contains('🐣')
                || lower.contains("native rule smoke check")
                || lower.contains("do not use tools")
                || lower.contains("claude code")
                || lower.contains("cursor agent")
                || lower.contains("authentication")
                || lower.contains("login")
                || lower.contains("trust")
                || lower.contains("workspace")
                || lower.contains("yes,")
                || lower.contains("no, exit")
                || lower.contains("confirm")
                || lower.contains("cancel")
                || lower.contains("shortcuts")
                || lower.contains("interrupt")
                || lower.contains("ctrl+")
                || lower.contains("shift+")
                || lower.contains("tokens")
                || lower.contains("context")
                || lower.contains("theme")
                || lower.contains("color mode")
                || lower.contains("press enter")
                || line.trim().starts_with([
                    '>', '❯', '→', '─', '│', '╭', '╰', '┌', '└', '┐', '┘', '├', '┤',
                ])
                || line
                    .chars()
                    .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch));
            if structural {
                sanitize(line)
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>();
    let title = if snapshot.title.starts_with('✳') {
        "✳ Native smoke"
    } else if snapshot.title.starts_with("* ") {
        "* Native smoke"
    } else if snapshot.title.contains("Cursor") {
        "Cursor Agent"
    } else if snapshot.title.is_empty() {
        ""
    } else {
        "Native smoke"
    };
    let root = std::env::var_os("PROMETEU_NATIVE_UI_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/native_ui_readiness/fixtures")
        });
    std::fs::create_dir_all(&root).map_err(|_| "native fixture directory failed".to_owned())?;
    let phase_name = if matches!(request.mode, StartMode::Fresh) {
        "fresh"
    } else {
        "resume"
    };
    let path = root.join(format!(
        "{}-{phase_name}-{phase}.json",
        provider_key(request.settings.agent)?
    ));
    let veto_tags = snapshot
        .viewport_lines
        .iter()
        .enumerate()
        .flat_map(|(row, line)| {
            let lower = line.to_lowercase();
            [
                "esc to interrupt",
                "ctrl+c to stop",
                "running stop hooks",
                "working",
                "accessing workspace",
                "trust this",
                "trust the",
                "trusting workspace",
                "trust required",
                "run this command?",
                "run (once)",
                "run everything",
                "skip & tell the agent",
                "allowlist?",
                "permission",
                "approve",
                "approval",
                "sign in",
                "log in",
                "login",
                "authentication",
                "enter to confirm",
                "esc to cancel",
                "press enter",
                "select an option",
                "waiting for",
            ]
            .into_iter()
            .filter(move |word| lower.contains(word))
            .map(move |word| json!({"row":row,"tag":word}))
        })
        .collect::<Vec<_>>();
    let value = json!({"vetoTags":veto_tags,"version":version,"provider":provider_key(request.settings.agent)?,"title":title,"cols":snapshot.cols,"rows":snapshot.rows,"cursor":screen.cursor,"cursorHidden":snapshot.modes.cursor_hidden,"bracketedPaste":snapshot.modes.bracketed_paste_mode,"pendingEscape":!snapshot.pending_escape.is_empty(),"pendingUtf8":!snapshot.pending_utf8.is_empty(),"lines":lines,"styles":snapshot.viewport_styles,"characterColumns":snapshot.viewport_character_columns});
    paths::write_private(
        &path,
        &serde_json::to_string_pretty(&value)
            .map_err(|_| "native fixture encoding failed".to_owned())?,
    )
    .map_err(|_| "native fixture write failed".to_owned())
}

fn scratch_trust_prompt(
    bytes: &[u8],
    workspace: &str,
    provider: ProviderId,
) -> Option<&'static [u8]> {
    let raw = plain_screen(bytes);
    let compact = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    // Confirmation is scoped to this exact empty test folder and never applies to a tool prompt.
    if !compact.contains(workspace) {
        return None;
    }
    let lower = raw.to_lowercase();
    let compact_lower = compact.to_lowercase();
    if provider == ProviderId::Claude
        && compact_lower.contains("accessingworkspace:")
        && compact_lower.contains("yes,itrustthisfolder")
        && compact_lower.contains("no,exit")
    {
        if compact_lower.contains("❯no,exit") {
            return Some(b"\x1b[B\r");
        }
        if compact_lower.contains("❯yes,itrustthisfolder") {
            return Some(b"\r");
        }
        return None;
    }
    if provider == ProviderId::Cursor
        && lower.contains("workspace trust required")
        && lower.contains("trust this workspace")
    {
        return Some(if lower.contains("don't enable all mcp") {
            b"w"
        } else {
            b"a"
        });
    }
    (["trust this", "trust the", "trust files", "trust folder"]
        .iter()
        .any(|phrase| lower.contains(phrase))
        && lower.contains("yes"))
    .then_some(b"\r".as_slice())
}

#[test]
fn scratch_trust_requires_exact_path_and_selects_the_explicit_claude_yes_option() {
    let screen=b"Accessing\x1b[12Gworkspace:\n/tmp/owned-smoke/workspace\n\xe2\x9d\xafNo,\x1b[8Gexit\nYes,\x1b[9GI\x1b[11Gtrust\x1b[17Gthis\x1b[22Gfolder";
    assert_eq!(
        scratch_trust_prompt(screen, "/tmp/owned-smoke/workspace", ProviderId::Claude),
        Some(b"\x1b[B\r".as_slice())
    );
    assert_eq!(
        scratch_trust_prompt(screen, "/tmp/another/workspace", ProviderId::Claude),
        None
    );
    assert_eq!(
        scratch_trust_prompt(
            b"/tmp/owned-smoke/workspace Run this command? Yes",
            "/tmp/owned-smoke/workspace",
            ProviderId::Claude
        ),
        None
    );
}

fn plain_screen(bytes: &[u8]) -> String {
    let mut result = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            result.push(bytes[index]);
            index += 1;
            continue;
        }
        index += 1;
        match bytes.get(index) {
            Some(b'[') => {
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            Some(b']') => {
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == 7 {
                        index += 1;
                        break;
                    }
                    if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            Some(_) => index += 1,
            None => {}
        }
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn transcript_has_response(path: &Path, provider: ProviderId, marker: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut bytes = Vec::new();
    if file
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > 4 * 1024 * 1024
    {
        return false;
    }
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .any(|row| {
            let text = match provider {
                ProviderId::Claude if row["type"] == "assistant" => {
                    Some(&row["message"]["content"])
                }
                ProviderId::Codex
                    if row["type"] == "response_item" && row["payload"]["role"] == "assistant" =>
                {
                    Some(&row["payload"]["content"])
                }
                ProviderId::Cursor if row["role"] == "assistant" => {
                    Some(&row["message"]["content"])
                }
                ProviderId::Antigravity
                    if row["source"] == "MODEL" && row["type"] == "PLANNER_RESPONSE" =>
                {
                    Some(&row["content"])
                }
                _ => None,
            };
            text.is_some_and(|text| {
                text.as_str().is_some_and(|text| text.trim() == marker)
                    || text.as_array().is_some_and(|parts| {
                        parts.iter().any(|part| {
                            part["text"]
                                .as_str()
                                .is_some_and(|text| text.trim() == marker)
                        })
                    })
            })
        })
}

pub(crate) struct SmokeTerminal {
    pub(crate) input: Option<Box<dyn Write + Send>>,
    pub(crate) output: Receiver<Vec<u8>>,
    pub(crate) control: Option<Arc<dyn TerminalControl>>,
    waiter: Option<Box<dyn TerminalWait>>,
    reader: Option<JoinHandle<()>>,
}
impl SmokeTerminal {
    pub(crate) fn open(command: portable_pty::CommandBuilder) -> Result<Self, String> {
        let started = prometeu_process::terminal::ResponsiveTerminalFactory
            .open(
                command,
                TerminalSize {
                    cols: 120,
                    rows: 40,
                },
            )
            .map_err(|_| "native PTY failed".to_owned())?;
        let (sender, output) = mpsc::channel();
        let mut native = started.output;
        let reader = std::thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                match native.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(size) => {
                        if sender.send(bytes[..size].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            input: Some(started.input),
            output,
            control: Some(started.control),
            waiter: Some(started.waiter),
            reader: Some(reader),
        })
    }
    pub(crate) fn close(&mut self) {
        if let Some(control) = self.control.as_ref() {
            control.close();
        }
        self.input.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.control.take();
        if let Some(mut waiter) = self.waiter.take() {
            let _ = waiter.wait();
        }
    }
}
impl Drop for SmokeTerminal {
    fn drop(&mut self) {
        self.close();
    }
}
