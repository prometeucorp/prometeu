//! Opt-in stop conformance. Each request and PTY belongs to an empty, disposable workspace.
use super::*;
use prometeu_core::{
    actions::Permission,
    session::launch::{Launch, LaunchRequest, StartMode},
    terminal::{TerminalControl, TerminalFactory, TerminalSize},
};
use prometeu_process::terminal_state::{Client, Key, Size, Stream};
use prometeu_protocols::terminal_status::{
    ProgramStatusEvent, ProgramStatusParser, ProgramStatusState, ProgramStatusStore,
    PROGRAM_STATUS_QUERY_REPLY,
};
use serde_json::json;
use std::{
    io::{Read, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const CHILD: &str = "PROMETEU_INTERRUPT_SMOKE_CHILD";
const PROMPT: &str = "Count slowly from 1 to 100, one number per line. Do not use tools or execute commands. Stop after 100.";

#[test]
#[ignore = "uses an installed authenticated Claude and one bounded no-tool model request"]
fn native_claude_interrupt_keeps_the_session_alive() {
    smoke(ProviderId::Claude, "claude");
}

#[test]
#[ignore = "uses an installed authenticated Codex and one bounded no-tool model request"]
fn native_codex_interrupt_keeps_the_session_alive() {
    smoke(ProviderId::Codex, "codex");
}

#[test]
#[ignore = "uses an installed authenticated Antigravity and one bounded no-tool model request"]
fn native_antigravity_interrupt_keeps_the_session_alive() {
    smoke(ProviderId::Antigravity, "antigravity");
}

#[test]
#[ignore = "uses an installed authenticated Cursor and one bounded no-tool model request"]
fn native_cursor_interrupt_keeps_the_session_alive() {
    smoke(ProviderId::Cursor, "cursor");
}

fn smoke(provider: ProviderId, name: &str) {
    if std::env::var_os(CHILD).is_none() {
        let root = PathBuf::from("/tmp")
            .join(format!("prometeu-interrupt-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("terminal_agent_controls::smoke::native_{name}_interrupt_keeps_the_session_alive"), "--ignored", "--nocapture"])
            .env(CHILD, "1").env("PROMETEU_ROOT", root.join("app")).current_dir(&root)
            .output().unwrap();
        let _ = std::fs::remove_dir_all(&root);
        print!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "stop smoke failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    crate::install_crypto();
    let captured = crate::terminal_agent_hooks::smoke::captured_profile(provider).unwrap();
    crate::accounts::change(|registry| {
        if let Some((account, _)) = &captured {
            registry.accounts.push(account.clone());
            registry.active.insert(name.into(), account.id.clone());
            return Ok(());
        }
        if !registry
            .accounts
            .iter()
            .any(|account| account.provider == provider)
        {
            registry.accounts.push(crate::accounts::Account {
                id: name.into(),
                provider,
                revision: 0,
                auth_method: Some("external".into()),
                key_suffix: None,
                identity: Default::default(),
            });
        }
        registry.active.insert(name.into(), name.into());
        Ok(())
    })
    .unwrap();
    let root = std::env::current_dir().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut settings = Launch {
        agent: provider,
        permission: (provider != ProviderId::Codex).then_some(Permission::Ask),
        mcp: Some(Vec::new()),
        plugins: Some(Vec::new()),
        skills: Some(Vec::new()),
        ..Default::default()
    };
    if let Ok(catalog) =
        crate::agents::models(&prometeu_process::query::UnixQueryLauncher, provider)
    {
        let model = catalog
            .models
            .iter()
            .find(|model| match provider {
                ProviderId::Claude => model.id.contains("sonnet"),
                ProviderId::Antigravity => model.id.contains("flash"),
                _ => false,
            })
            .or_else(|| catalog.models.iter().find(|model| !model.additional));
        if let Some(model) = model {
            settings.model = model.id.clone();
            settings.effort = model
                .efforts
                .iter()
                .find(|value| value.as_str() == "low")
                .or_else(|| model.efforts.first())
                .cloned()
                .unwrap_or_default();
            if provider == ProviderId::Cursor {
                settings.effort.clear();
            }
        }
    }
    let request = LaunchRequest {
        session: uuid::Uuid::new_v4().to_string(),
        workspace: uuid::Uuid::new_v4().to_string(),
        worktree: workspace.to_string_lossy().into_owned(),
        settings,
        mode: StartMode::Fresh,
    };
    let result = run(
        provider,
        name,
        &request,
        captured.map(|(_, profile)| profile),
    );
    assert!(
        result.is_ok(),
        "stop evidence incomplete: {}",
        result.err().unwrap_or_default()
    );
}

fn run(
    provider: ProviderId,
    name: &str,
    request: &LaunchRequest,
    selected: Option<crate::accounts::Profile>,
) -> Result<(), String> {
    let profiles = crate::terminal_agent_hooks::smoke::SmokeProfiles {
        selected,
        native: crate::account_profiles::native(),
    };
    let mut prepared = crate::terminal_agent_launch::prepare(
        request,
        &profiles,
        &crate::tool_materialization::native(),
    )
    .map_err(|_| "native preparation failed")?;
    let version = prometeu_protocols::catalog::command_output(
        &prometeu_process::query::UnixQueryLauncher,
        Command::new(prepared.command.get_program()).arg("--version"),
        Duration::from_secs(5),
    )
    .map_err(|_| "native version query failed")?;
    let controls =
        NativeControls::verified(provider, &version).map_err(|_| "unverified native build")?;
    if provider == ProviderId::Claude && prepared.profile.managed {
        let directory = crate::paths::root().join("native-claude-config");
        crate::paths::ensure_private_dir(&directory)
            .map_err(|_| "private native config directory failed")?;
        crate::paths::write_private(
            &directory.join(".claude.json"),
            &json!({"theme":"dark", "hasCompletedOnboarding":true}).to_string(),
        )
        .map_err(|_| "private native preferences failed")?;
        prepared
            .command
            .env("CLAUDE_CONFIG_DIR", &directory)
            .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", &prepared.profile.home);
    }
    if provider == ProviderId::Codex {
        prepared.command.args([
            "-c",
            "approval_policy=\"on-request\"",
            "-c",
            "sandbox_mode=\"read-only\"",
        ]);
        if let Some(probe) = prepared.hook_probe.as_mut() {
            probe.args([
                "-c",
                "approval_policy=\"on-request\"",
                "-c",
                "sandbox_mode=\"read-only\"",
            ]);
        }
    }
    if provider == ProviderId::Cursor {
        let config = crate::paths::root().join("native-cursor-config");
        crate::paths::ensure_private_dir(&config)?;
        prepared.command.env("CURSOR_CONFIG_DIR", config);
    }
    let (mut bridge, hooks) = crate::terminal_agent_hooks::HookBridge::start(
        provider,
        &request.session,
        &uuid::Uuid::new_v4().to_string(),
        prepared.provider_session.as_deref(),
    )?;
    bridge.configure_with_helper(
        &mut prepared.command,
        prepared.hook_probe.as_mut(),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/Prometeu"),
    )?;
    if matches!(
        provider,
        ProviderId::Codex | ProviderId::Claude | ProviderId::Cursor
    ) {
        prepared.command.arg(PROMPT);
    } else {
        prepared.command.args(["--prompt-interactive", PROMPT]);
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
    let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/terminal-state/dist")
        .join(format!(
            "prometeu-terminal-state-{}-apple-darwin",
            std::env::consts::ARCH
        ));
    let mut helper = Client::launch(Command::new(executable), Duration::from_secs(5))
        .map_err(|_| "screen helper unavailable")?;
    let mut stream = Stream::create(
        &mut helper,
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
    .map_err(|_| "screen create failed")?;
    let mut terminal = OwnedTerminal::open(command)?;
    let mut parser = ProgramStatusParser::default();
    let mut status = ProgramStatusStore::default();
    let mut chunk = 0;
    let mut interrupted = false;
    let mut trust_confirmed = false;
    let mut pending_trust: Option<(Instant, bool)> = None;
    let mut hooks_skipped = false;
    let mut post_stop = false;
    let mut prompt_active = false;
    let mut prompt_in_native_ui = false;
    let mut model_output_observed = false;
    let mut stop_reasons = Vec::new();
    let mut evidence = None;

    let mut statuses = Vec::new();
    let mut passed = false;
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        let bytes = match terminal.output.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => bytes,
            Err(RecvTimeoutError::Timeout) => Vec::new(),
            Err(RecvTimeoutError::Disconnected) => break,
        };
        // PSP precedes DA responses: a provider may use DA as its capability-flush barrier.
        for event in parser.feed(&bytes) {
            match event {
                ProgramStatusEvent::Query => terminal.write(PROGRAM_STATUS_QUERY_REPLY)?,
                ProgramStatusEvent::Report(report) => {
                    if report.id.is_none() {
                        statuses.push(format!("{:?}", report.state));
                        post_stop |= interrupted
                            && matches!(
                                report.state,
                                ProgramStatusState::Idle | ProgramStatusState::Done
                            );
                    }
                    status.apply(report);
                }
            }
        }
        chunk += 1;
        for reply in stream
            .write_chunk(&mut helper, chunk, &bytes)
            .map_err(|_| "screen write failed")?
            .replies
        {
            terminal.write(&reply)?;
        }
        let snapshot = stream
            .snapshot(&mut helper)
            .map_err(|_| "screen snapshot failed")?
            .state;
        let visible = snapshot.viewport_lines.join("\n");
        let lower = visible.to_lowercase();
        if provider == ProviderId::Cursor {
            let compact: String = visible.chars().filter(|ch| !ch.is_whitespace()).collect();
            let expected: String = PROMPT.chars().filter(|ch| !ch.is_whitespace()).collect();
            prompt_in_native_ui |= compact.contains(&expected);
            model_output_observed |= snapshot.viewport_lines.iter().any(|line| {
                line.trim()
                    .parse::<u16>()
                    .is_ok_and(|n| (1..=100).contains(&n))
            });
        }
        while let Ok(event) = hooks.try_recv() {
            if provider == ProviderId::Cursor
                && interrupted
                && event.agent_id.is_none()
                && event.kind == crate::terminal_agent_hooks::NativeHookKind::StopObserved
            {
                post_stop |= event.reason.as_deref() == Some("aborted");
                if let Some(reason) = event.reason {
                    stop_reasons.push(reason);
                }
            }
            if event.agent_id.is_none()
                && matches!(
                    event.kind,
                    crate::terminal_agent_hooks::NativeHookKind::PromptSubmitted
                        | crate::terminal_agent_hooks::NativeHookKind::ModelInvocationStarted
                )
            {
                prompt_active = true;
            }
        }
        if provider == ProviderId::Codex
            && !hooks_skipped
            && status.snapshot().root.is_none()
            && lower.contains("review hooks")
            && lower.contains("esc skip")
        {
            // rust-v0.169.0 startup_hooks_review: cancel continues without trusting and never
            // invokes write_hook_trusts. No global hook approval is part of this stop test.
            terminal.write(b"\x1b")?;
            hooks_skipped = true;
            println!("stop smoke codex: continued without trusting startup hooks");
            continue;
        }
        if provider == ProviderId::Cursor
            && !trust_confirmed
            && snapshot
                .viewport_lines
                .iter()
                .any(|line| line.trim().trim_matches('│').trim() == request.worktree)
            && snapshot.viewport_lines.iter().any(|line| {
                matches!(
                    line.trim().trim_matches('│').trim(),
                    "[a] Trust this workspace" | "▶ [a] Trust this workspace"
                )
            })
        {
            terminal.write(b"a")?;
            trust_confirmed = true;
        }
        if !trust_confirmed
            && pending_trust.is_none()
            && visible.contains(&request.worktree)
            && ["trust this", "trust the", "trust files", "trust folder"]
                .iter()
                .any(|word| lower.contains(word))
            && lower.contains("yes")
        {
            if provider == ProviderId::Claude && lower.contains("no, exit") {
                // The native trust menu initially selects No and guards early keystrokes.
                // Keep draining protocol queries while waiting to select this scratch folder.
                pending_trust = Some((Instant::now() + Duration::from_millis(1200), false));
            } else {
                terminal.write(b"\r")?;
                trust_confirmed = true;
            }
        }
        if let Some((at, selected)) = pending_trust {
            if Instant::now() >= at {
                terminal.write(if selected { b"\r" } else { b"\x1b[B" })?;
                if selected {
                    pending_trust = None;
                    trust_confirmed = true;
                } else {
                    pending_trust = Some((Instant::now() + Duration::from_millis(200), true));
                }
            }
        }
        let active = provider == ProviderId::Cursor
            && prompt_in_native_ui
            && model_output_observed
            && lower.contains("working")
            || status
                .snapshot()
                .root
                .is_some_and(|report| report.state == ProgramStatusState::Working);
        let hint = snapshot.viewport_lines.iter().any(|line| {
            let text = line.to_lowercase();
            (text.contains("esc") || provider == ProviderId::Cursor && text.contains("ctrl+c"))
                && ["interrupt", "cancel", "stop"]
                    .iter()
                    .any(|word| text.contains(word))
                && !text.contains("starting mcp")
        });
        let running_hooks = provider == ProviderId::Claude
            && snapshot.viewport_lines.iter().any(|line| {
                let text = line.to_lowercase();
                text.contains("running ") && text.contains("hook")
            });
        let coherent_rest = if provider == ProviderId::Cursor {
            crate::native_ui_readiness::NativeUiReadiness::verified(provider, &version)
                .is_some_and(|view| view.ready(&snapshot))
                || cursor_restored_prompt(&snapshot)
        } else {
            provider != ProviderId::Claude
                || snapshot
                    .viewport_lines
                    .get(usize::from(snapshot.normal.cursor.y))
                    .is_some_and(|line| line.trim_start().starts_with('❯'))
        };
        if !interrupted
            && active
            && (prompt_active || provider == ProviderId::Cursor && prompt_in_native_ui)
            && hint
            && !running_hooks
            && snapshot.pending_escape.is_empty()
            && snapshot.pending_utf8.is_empty()
        {
            // Wait for the current control profile to accept a painted frame; no key is retried.
            let Ok(interrupt) = controls.decision(&snapshot, true) else {
                continue;
            };
            capture(name, version.trim(), "busy", request, &snapshot)?;
            // One key only: never retry or fall back to Ctrl+C if interruption is not observed.
            terminal.write(interrupt)?;
            interrupted = true;
            println!(
                "stop smoke {name}: observed active native hint; sent the advertised key once"
            );
        } else if interrupted && post_stop && !hint && coherent_rest && terminal.running() {
            capture(name, version.trim(), "interrupted", request, &snapshot)?;
            evidence = Some(json!({
                "provider": name, "version": version.trim(), "prompt": PROMPT,
                "promptObservedByHook": prompt_active, "promptObservedInNativeUi": prompt_in_native_ui,
                "modelOutputObservedBeforeInterrupt": model_output_observed,
                "interrupt": if provider == ProviderId::Cursor { "ctrl-c" } else { "escape" },
                "interruptCount": 1, "nativeRootStates": statuses,
                "explicitPostInterruptIdleOrDone": provider != ProviderId::Cursor && post_stop,
                "nativeRootStopReasons": stop_reasons,
                "explicitPostInterruptAbort": provider == ProviderId::Cursor && post_stop,
                "processRemainsAlive": true,
                "interruptedPromptRestoredAsDraft": provider == ProviderId::Cursor && cursor_restored_prompt(&snapshot),
                "settings": if provider == ProviderId::Codex {
                    json!({"approvalPolicy":"on-request", "sandboxMode":"read-only"})
                } else {
                    json!({"permission":"ask"})
                },
                "startupHooksSkippedWithoutTrust": hooks_skipped
            }));
            println!("stop smoke {name}: explicit native stop observed; process remains alive");
            passed = true;
            break;
        }
    }
    if !passed {
        let snapshot = stream
            .snapshot(&mut helper)
            .map_err(|_| "screen snapshot failed")?
            .state;
        capture(name, version.trim(), "incomplete", request, &snapshot)?;
    }
    terminal.close();
    if !terminal.reaped.load(Ordering::Acquire) {
        return Err("native process did not reap".into());
    }
    if let Some(mut evidence) = evidence {
        evidence["processReaped"] = json!(true);
        crate::paths::write_private(
            &fixture_directory().join(format!("{name}-result.json")),
            &serde_json::to_string_pretty(&evidence).map_err(|_| "evidence encode failed")?,
        )
        .map_err(|_| "evidence write failed")?;
    }
    bridge.close();
    stream
        .dispose(&mut helper)
        .map_err(|_| "screen dispose failed")?;
    helper.shutdown().map_err(|_| "screen shutdown failed")?;
    if passed {
        Ok(())
    } else {
        Err(format!("provider={name} prompt_active={prompt_active} prompt_ui={prompt_in_native_ui} model_output={model_output_observed} trust={trust_confirmed} interrupted={interrupted} explicit_post_stop={post_stop} statuses={statuses:?}"))
    }
}

fn cursor_restored_prompt(snapshot: &Snapshot) -> bool {
    let lines = &snapshot.viewport_lines;
    let Some(row) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("→ "))
    else {
        return false;
    };
    row > 0
        && row + 4 < lines.len()
        && lines[row].trim().strip_prefix("→ ") == Some(PROMPT)
        && lines[row - 1].trim().is_empty()
        && lines[row + 1].trim().is_empty()
        && lines[row + 2].trim().is_empty()
        && !lines[row + 3].trim().is_empty()
        && !lines[row + 4].trim().is_empty()
        && snapshot.modes.cursor_hidden
        && snapshot.normal.cursor.x == 0
        && usize::from(snapshot.normal.cursor.y) == row + 5
        && snapshot.pending_escape.is_empty()
        && snapshot.pending_utf8.is_empty()
}

fn capture(
    name: &str,
    version: &str,
    phase: &str,
    request: &LaunchRequest,
    state: &Snapshot,
) -> Result<(), String> {
    let screen = match state.active {
        BufferKind::Normal => &state.normal,
        BufferKind::Alternate => state.alternate.as_ref().ok_or("missing alternate screen")?,
    };
    let home = crate::paths::home().to_string_lossy().into_owned();
    let lines = state
        .viewport_lines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let lower = line.to_lowercase();
            let structural = row >= usize::from(screen.cursor.y).saturating_sub(3)
                || line.trim().is_empty()
                || line
                    .trim()
                    .starts_with(['>', '›', '❯', '→', '─', '│', '╭', '╰', '┌', '└'])
                || [
                    "esc",
                    "interrupt",
                    "cancel",
                    "openai codex",
                    "antigravity",
                    "cursor agent",
                    "ctrl+c",
                    "working",
                    "thinking",
                    "token",
                    "context",
                    "count slowly",
                    "do not use tools",
                    "read-only",
                    "invalid",
                    "error:",
                    "unknown flag",
                ]
                .iter()
                .any(|word| lower.contains(word));
            if structural && !line.contains('@') {
                let mut line = line
                    .replace(&request.worktree, "/tmp/interrupt-smoke/workspace")
                    .replace(&home, "/home/test-user");
                if let Some((_, value)) = line.split_once("weekly limit: ") {
                    if let Some((percentage, _)) = value.split_once('%') {
                        line = line
                            .replace(&format!("weekly limit: {percentage}%"), "weekly limit: 50%");
                    }
                }
                line
            } else {
                String::new()
            }
        })
        .collect::<Vec<_>>();
    let value = json!({"provider":name,"version":version,"phase":phase,"cols":state.cols,"rows":state.rows,
        "cursor":screen.cursor,"cursorHidden":state.modes.cursor_hidden,"bracketedPaste":state.modes.bracketed_paste_mode,
        "pendingEscape":!state.pending_escape.is_empty(),"pendingUtf8":!state.pending_utf8.is_empty(),"title":"",
        "lines":lines,"styles":state.viewport_styles});
    let directory = fixture_directory();
    std::fs::create_dir_all(&directory).map_err(|_| "fixture directory failed")?;
    crate::paths::write_private(
        &directory.join(format!("{name}-{phase}.json")),
        &serde_json::to_string_pretty(&value).map_err(|_| "fixture encode failed")?,
    )
    .map_err(|_| "fixture write failed".into())
}

fn fixture_directory() -> PathBuf {
    std::env::var_os("PROMETEU_INTERRUPT_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/terminal_agent_controls/fixtures")
        })
}

struct OwnedTerminal {
    input: Option<Box<dyn Write + Send>>,
    output: Receiver<Vec<u8>>,
    control: Option<Arc<dyn TerminalControl>>,
    waiter: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
    reaped: Arc<AtomicBool>,
}
impl OwnedTerminal {
    fn open(command: portable_pty::CommandBuilder) -> Result<Self, String> {
        let started = prometeu_process::terminal::UnixTerminalFactory
            .open(
                command,
                TerminalSize {
                    cols: 120,
                    rows: 40,
                },
            )
            .map_err(|_| "PTY open failed")?;
        let (sender, output) = mpsc::channel();
        let mut native = started.output;
        let reader = std::thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                match native.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => {
                        if sender.send(bytes[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
        let mut waiter = started.waiter;
        let reaped = Arc::new(AtomicBool::new(false));
        let recorded = reaped.clone();
        let waiter = std::thread::spawn(move || {
            recorded.store(waiter.wait().is_ok(), Ordering::Release);
        });
        Ok(Self {
            input: Some(started.input),
            output,
            control: Some(started.control),
            waiter: Some(waiter),
            reader: Some(reader),
            reaped,
        })
    }
    fn write(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.input
            .as_mut()
            .ok_or("PTY input closed")?
            .write_all(bytes)
            .map_err(|_| "PTY input failed".into())
    }
    fn running(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.running())
    }
    fn close(&mut self) {
        if let Some(control) = self.control.as_ref() {
            control.close();
        }
        self.input.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.control.take();
        if let Some(waiter) = self.waiter.take() {
            let _ = waiter.join();
        }
    }
}
impl Drop for OwnedTerminal {
    fn drop(&mut self) {
        self.close();
    }
}
