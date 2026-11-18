//! Opt-in native proofs with explicit replies scoped to an owned scratch session.
use super::*;
use crate::terminal_agent_hooks::{
    smoke::{captured_profile, SmokeProfiles, SmokeTerminal},
    HookBridge,
};
use prometeu_core::{
    actions::Permission,
    session::launch::{Launch, LaunchRequest, StartMode},
};
use prometeu_process::terminal_state::{Client, Key, Size, Stream};
use prometeu_protocols::terminal_status::{
    ProgramStatusEvent, ProgramStatusParser, PROGRAM_STATUS_QUERY_REPLY,
};
use std::{
    io::Write,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

const CHILD: &str = "PROMETEU_DECISION_SMOKE_CHILD";
const QUESTION_PROMPT: &str = "Use AskUserQuestion exactly once to ask 'Choose the smoke label' with exactly two options Blue and Green, with brief descriptions. Do not use any other tool, read files, or execute commands. After the answer, reply with exactly that label.";

#[test]
#[ignore = "uses an authenticated Claude and one bounded synthetic question"]
fn native_claude_question_uses_explicit_hook_reply() {
    smoke(Case::HookQuestion);
}

#[test]
#[ignore = "uses an authenticated Claude and one bounded synthetic question to verify the native dialog race"]
fn native_claude_local_answer_wins_over_late_hook_reply() {
    smoke(Case::LocalQuestion);
}

const COMMAND: &str = "printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt";
const MARKER: &str = "PROMETEU_TERMINAL_TOOL_SMOKE";
const DENIED: &str = "PROMETEU_PLAN_DENIED";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    HookQuestion,
    LocalQuestion,
    Bash,
    ExitPlanDeny,
}
impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::HookQuestion => "native_claude_question_uses_explicit_hook_reply",
            Self::LocalQuestion => "native_claude_local_answer_wins_over_late_hook_reply",
            Self::Bash => "native_claude_bash_uses_explicit_hook_approval",
            Self::ExitPlanDeny => "native_claude_exit_plan_uses_explicit_hook_denial",
        }
    }
    fn prompt(self) -> String {
        match self {
            Self::HookQuestion | Self::LocalQuestion => QUESTION_PROMPT.into(),
            Self::Bash => format!("Use Bash exactly once to run exactly `{COMMAND}` in the current scratch workspace. Do not run any other command, read any file, or use any other tool. After it succeeds, reply exactly {MARKER}."),
            Self::ExitPlanDeny => format!("This is a no-op plan approval transport test. You are already in plan mode. Call ExitPlanMode exactly once with no requested command permissions; no plan file is necessary for this test. You may load that deferred native tool using ToolSearch only with query select:ExitPlanMode and max_results 1. Do not read or write files, run commands, or use any other tool. If exit is denied, do not retry or implement anything; reply exactly {DENIED}."),
        }
    }
}

#[test]
#[ignore = "uses authenticated Claude and one literal printf in an owned scratch directory"]
fn native_claude_bash_uses_explicit_hook_approval() {
    smoke(Case::Bash);
}

#[test]
#[ignore = "uses authenticated Claude and explicitly denies one no-op plan exit"]
fn native_claude_exit_plan_uses_explicit_hook_denial() {
    smoke(Case::ExitPlanDeny);
}

fn smoke(case: Case) {
    if std::env::var_os(CHILD).is_none() {
        let root =
            PathBuf::from("/tmp").join(format!("prometeu-decision-smoke-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let name = case.name();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("terminal_agent_decisions::smoke::{name}"),
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("PROMETEU_ROOT", root.join("app"))
            .current_dir(&root)
            .output()
            .unwrap();
        if std::env::var_os("PROMETEU_SMOKE_KEEP_PROOF").is_some() {
            println!("owned decision proof: {}", root.display());
        } else {
            let _ = std::fs::remove_dir_all(&root);
        }
        print!("{}", String::from_utf8_lossy(&output.stdout));
        assert!(
            output.status.success(),
            "decision smoke failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    crate::install_crypto();
    let (account, profile) = captured_profile(ProviderId::Claude)
        .unwrap()
        .expect("existing selected managed Claude account required");
    crate::accounts::change(|registry| {
        registry.active.insert("claude".into(), account.id.clone());
        registry.accounts.push(account);
        Ok(())
    })
    .unwrap();
    let root = std::env::current_dir().unwrap();
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let request = LaunchRequest {
        session: uuid::Uuid::new_v4().to_string(),
        workspace: uuid::Uuid::new_v4().to_string(),
        worktree: workspace.to_string_lossy().into_owned(),
        mode: StartMode::Fresh,
        settings: Launch {
            agent: ProviderId::Claude,
            model: "sonnet".into(),
            effort: "low".into(),
            permission: Some(Permission::Ask),
            plan: case == Case::ExitPlanDeny,
            mcp: Some(vec![]),
            plugins: Some(vec![]),
            skills: Some(vec![]),
            ..Default::default()
        },
    };
    let result = run(&request, profile, case);
    assert!(
        result.is_ok(),
        "decision conformance incomplete: {}",
        result.err().unwrap_or_default()
    );
}

fn run(
    request: &LaunchRequest,
    profile: crate::accounts::Profile,
    case: Case,
) -> Result<(), String> {
    let profiles = SmokeProfiles {
        selected: Some(profile),
        native: crate::account_profiles::native(),
    };
    let mut prepared = crate::terminal_agent_launch::prepare(
        request,
        &profiles,
        &crate::tool_materialization::native(),
    )
    .map_err(|_| "native preparation failed")?;
    let directory = crate::paths::root().join("native-claude-config");
    crate::paths::ensure_private_dir(&directory).map_err(|_| "private config failed")?;
    crate::paths::write_private(
        &directory.join(".claude.json"),
        &json!({"theme":"dark","hasCompletedOnboarding":true}).to_string(),
    )
    .map_err(|_| "private preferences failed")?;
    prepared
        .command
        .env("CLAUDE_CONFIG_DIR", &directory)
        .env("CLAUDE_SECURESTORAGE_CONFIG_DIR", &prepared.profile.home);
    let (mut bridge, hooks) = HookBridge::start(
        ProviderId::Claude,
        &request.session,
        &uuid::Uuid::new_v4().to_string(),
        prepared.provider_session.as_deref(),
    )?;
    let decisions = bridge.enable_claude_decisions()?;
    bridge.configure_with_helper(
        &mut prepared.command,
        prepared.hook_probe.as_mut(),
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/Prometeu"),
    )?;
    if bridge.native_version() != Some("2.1.326 (Claude Code)") {
        return Err("unverified Claude version".into());
    }
    prepared.command.arg(case.prompt());
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
    let mut emulator = Client::launch(Command::new(executable), Duration::from_secs(5))
        .map_err(|_| "screen helper unavailable")?;
    let mut screen = Stream::create(
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
    .map_err(|_| "screen create failed")?;
    let mut terminal = SmokeTerminal::open(command)?;
    let result = match case {
        Case::HookQuestion | Case::LocalQuestion => exercise(
            &mut terminal,
            &mut emulator,
            &mut screen,
            &decisions,
            &hooks,
            request,
            case == Case::LocalQuestion,
        ),
        Case::Bash | Case::ExitPlanDeny => exercise_permission(
            &mut terminal,
            &mut emulator,
            &mut screen,
            &decisions,
            &hooks,
            request,
            case,
        ),
    };
    let control = terminal.control.clone().ok_or("missing PTY control")?;
    terminal.close();
    bridge.close();
    if control.running() {
        return Err("owned native process remained alive after reap".into());
    }
    result
}

fn write(terminal: &mut SmokeTerminal, bytes: &[u8]) -> Result<(), String> {
    terminal
        .input
        .as_mut()
        .ok_or("input closed")?
        .write_all(bytes)
        .map_err(|_| "input write failed".into())
}

fn exercise(
    terminal: &mut SmokeTerminal,
    emulator: &mut Client,
    screen: &mut Stream,
    decisions: &Arc<Decisions>,
    hooks: &mpsc::Receiver<NativeHookEvent>,
    request: &LaunchRequest,
    local: bool,
) -> Result<(), String> {
    let mut parser = ProgramStatusParser::default();
    let mut chunk = 0;
    let mut trust = None;
    let mut trusted = false;
    let mut opened = None;
    let mut reply = None;
    let mut delivered = false;
    let mut closed_outcome = None;
    let mut reply_outcome = None;
    let mut observed_stop = false;
    let mut native_result = None;
    let mut local_submitted = false;
    let mut native_answer = false;
    let mut local_entered = false;
    let mut tool_id = None;
    let deadline = Instant::now() + Duration::from_secs(35);
    while Instant::now() < deadline {
        let bytes = match terminal.output.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => bytes,
            Err(mpsc::RecvTimeoutError::Timeout) => vec![],
            Err(_) => return Err("native process exited".into()),
        };
        for event in parser.feed(&bytes) {
            if matches!(event, ProgramStatusEvent::Query) {
                write(terminal, PROGRAM_STATUS_QUERY_REPLY)?;
            }
        }
        if !bytes.is_empty() {
            chunk += 1;
            for bytes in screen
                .write_chunk(emulator, chunk, &bytes)
                .map_err(|_| "screen write failed")?
                .replies
            {
                write(terminal, &bytes)?;
            }
        }
        let snapshot = screen
            .snapshot(emulator)
            .map_err(|_| "snapshot failed")?
            .state;
        let visible = snapshot.viewport_lines.join("\n");
        let lower = visible.to_lowercase();
        if !trusted
            && trust.is_none()
            && visible.contains(&request.worktree)
            && lower.contains("yes")
            && lower.contains("no, exit")
        {
            trust = Some((Instant::now() + Duration::from_millis(1200), false));
        }
        if let Some((at, selected)) = trust {
            if Instant::now() >= at {
                write(terminal, if selected { b"\r" } else { b"\x1b[B" })?;
                if selected {
                    trust = None;
                    trusted = true;
                } else {
                    trust = Some((Instant::now() + Duration::from_millis(200), true));
                }
            }
        }
        while let Ok(hook) = hooks.try_recv() {
            if hook.kind == NativeHookKind::StopObserved && hook.agent_id.is_none() {
                observed_stop = true;
            }
            if hook.kind == NativeHookKind::ToolFinished
                && hook.tool_name.as_deref() == Some("AskUserQuestion")
            {
                let response = hook
                    .tool_response
                    .as_ref()
                    .ok_or("question result absent")?;
                let text = response.to_string();
                if response
                    .pointer("/answers/Choose the smoke label")
                    .and_then(Value::as_str)
                    != Some("Blue")
                    && (!text.contains("Blue") || text.contains("Green"))
                {
                    return Err("native answer differs from the explicitly selected Blue".into());
                }
                native_result = Some(response.clone());
                tool_id = hook.tool_use_id;
                native_answer = true;
                println!("native question returned the explicit Blue answer");
            }
        }
        for event in decisions.drain() {
            if event["type"] == "request.opened" {
                if event["tool"] != "AskUserQuestion" {
                    return Err("unexpected tool requested; no permission response sent".into());
                }
                opened = Some(event);
            } else if event["type"] == "request.closed" {
                delivered = event["outcome"] == "resolved";
                closed_outcome = Some(event["outcome"].clone());
            }
        }
        if let Some(event) = opened.as_ref() {
            if !local && reply.is_none() && reply_outcome.is_none() {
                let registry = decisions.clone();
                let id = event["requestId"].clone();
                reply = Some(std::thread::spawn(move || {
                    registry.respond(&json!({"type":"request.respond","requestId":id,"response":{"outcome":"answer","answers":{"Choose the smoke label":"Blue"}}}))
                }));
            }
            if local
                && !local_entered
                && visible.contains("Choose the smoke label")
                && visible.contains("Blue")
                && visible.contains("Green")
                && lower.contains("enter")
            {
                // Only this test's synthetic choice is selected, never a tool approval menu.
                write(terminal, b"\r")?;
                local_entered = true;
                println!("native question selected its first Blue option");
            }
            if local
                && local_entered
                && !native_answer
                && !local_submitted
                && lower.contains("submit")
                && lower.contains("blue")
            {
                write(terminal, b"\r")?;
                local_submitted = true;
            }
            if local && native_answer && reply.is_none() && reply_outcome.is_none() {
                let registry = decisions.clone();
                let id = event["requestId"].clone();
                reply = Some(std::thread::spawn(move || {
                    registry.respond(&json!({"type":"request.respond","requestId":id,"response":{"outcome":"answer","answers":{"Choose the smoke label":"Green"}}}))
                }));
            }
        }
        if reply
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            reply_outcome = Some(
                reply
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| "reply worker panicked")?
                    .is_ok(),
            );
        }
        if native_answer && observed_stop && closed_outcome.is_some() && reply_outcome.is_some() {
            if !local && (reply_outcome != Some(true) || !delivered) {
                return Err("explicit reply was not delivered".into());
            }
            assert!(tool_id.is_some());
            println!(
                "native question conformance passed; reply_delivered={}, card_outcome={}",
                reply_outcome.unwrap(),
                closed_outcome.as_ref().unwrap()
            );
            if std::env::var_os("PROMETEU_CAPTURE_DECISION_PROOF").is_some() {
                let target = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/terminal_agent_decisions/fixtures");
                std::fs::create_dir_all(&target).map_err(|_| "proof directory failed")?;
                let proof = json!({"version":"2.1.326", "scenario":if local {"local-answer-before-hook"}else{"explicit-hook-answer"}, "selectedAnswer":"Blue", "hookAnswer":if local {"Green"}else{"Blue"}, "hookReplyDelivered":reply_outcome.unwrap(), "cardOutcome":closed_outcome, "nativeToolResponse":native_result, "subsequentRootStopObserved":true});
                std::fs::write(
                    target.join(if local {
                        "claude-local-winner.json"
                    } else {
                        "claude-hook-answer.json"
                    }),
                    serde_json::to_vec_pretty(&proof).unwrap(),
                )
                .map_err(|_| "proof write failed")?;
            }
            return Ok(());
        }
    }
    Err(format!(
        "question deadline (opened={}, local={}, native_answer={}, delivered={})",
        opened.is_some(),
        local_entered,
        native_answer,
        delivered
    ))
}

fn valid_bash_input(input: &Value) -> bool {
    let Some(input) = input.as_object() else {
        return false;
    };
    input.get("command").and_then(Value::as_str) == Some(COMMAND)
        && ["run_in_background", "dangerouslyDisableSandbox"]
            .iter()
            .all(|key| input.get(*key).is_none_or(|value| value == &json!(false)))
        && input.keys().all(|key| {
            matches!(
                key.as_str(),
                "command"
                    | "description"
                    | "timeout"
                    | "run_in_background"
                    | "dangerouslyDisableSandbox"
            )
        })
}

fn plan_widget(visible: &str) -> bool {
    (visible.contains("Ready to code?") && visible.contains("Would you like to proceed?"))
        || (visible.contains("Exit plan mode?")
            && visible.contains("Claude wants to exit plan mode")
            && visible.contains("1. Yes, and switch to default (ask each time) for this session")
            && visible.contains("2. No"))
}

fn exercise_permission(
    terminal: &mut SmokeTerminal,
    emulator: &mut Client,
    screen: &mut Stream,
    decisions: &Arc<Decisions>,
    hooks: &mpsc::Receiver<NativeHookEvent>,
    request: &LaunchRequest,
    case: Case,
) -> Result<(), String> {
    let bash = case == Case::Bash;
    let expected_tool = if bash { "Bash" } else { "ExitPlanMode" };
    let mut parser = ProgramStatusParser::default();
    let mut chunk = 0;
    let mut trust = None;
    let mut trusted = false;
    let mut opened: Option<Value> = None;
    let mut reply: Option<std::thread::JoinHandle<Result<Vec<Value>, String>>> = None;
    let mut reply_outcome = None;
    let mut closed = false;
    let mut observed_stop = false;
    let mut tool_finished = false;
    let mut tool_failed = false;
    let mut widget_seen = false;
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        let bytes = match terminal.output.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => bytes,
            Err(mpsc::RecvTimeoutError::Timeout) => vec![],
            Err(_) => return Err("native process exited".into()),
        };
        for event in parser.feed(&bytes) {
            if matches!(event, ProgramStatusEvent::Query) {
                write(terminal, PROGRAM_STATUS_QUERY_REPLY)?;
            }
        }
        if !bytes.is_empty() {
            chunk += 1;
            for bytes in screen
                .write_chunk(emulator, chunk, &bytes)
                .map_err(|_| "screen write failed")?
                .replies
            {
                write(terminal, &bytes)?;
            }
        }
        let snapshot = screen
            .snapshot(emulator)
            .map_err(|_| "snapshot failed")?
            .state;
        let visible = snapshot.viewport_lines.join("\n");
        let lower = visible.to_lowercase();
        if !trusted
            && trust.is_none()
            && visible.contains(&request.worktree)
            && lower.contains("yes")
            && lower.contains("no, exit")
        {
            trust = Some((Instant::now() + Duration::from_millis(1200), false));
        }
        if let Some((at, selected)) = trust {
            if Instant::now() >= at {
                write(terminal, if selected { b"\r" } else { b"\x1b[B" })?;
                if selected {
                    trust = None;
                    trusted = true;
                } else {
                    trust = Some((Instant::now() + Duration::from_millis(200), true));
                }
            }
        }
        while let Ok(hook) = hooks.try_recv() {
            if hook.agent_id.is_some() {
                return Err("unexpected subagent in the bounded decision smoke".into());
            }
            if hook.kind == NativeHookKind::ToolStarted
                && hook.tool_name.as_deref() != Some(expected_tool)
            {
                let native_discovery = !bash
                    && hook.tool_name.as_deref() == Some("ToolSearch")
                    && hook.tool_input.as_ref()
                        == Some(&json!({"query":"select:ExitPlanMode","max_results":1}));
                if !native_discovery {
                    return Err("unexpected tool started; no further response sent".into());
                }
            }
            if hook.kind == NativeHookKind::ToolFinished
                && hook.tool_name.as_deref() == Some(expected_tool)
            {
                if !bash {
                    return Err("ExitPlanMode succeeded despite the explicit denial".into());
                }
                if !hook.tool_input.as_ref().is_some_and(valid_bash_input)
                    || hook.tool_use_id.is_none()
                    || hook.tool_response.is_none()
                {
                    return Err("native Bash result did not identify the literal command".into());
                }
                tool_finished = true;
            }
            if hook.kind == NativeHookKind::ToolFailed
                && hook.tool_name.as_deref() == Some(expected_tool)
            {
                tool_failed = true;
                if bash {
                    return Err("approved literal Bash command failed".into());
                }
            }
            if hook.kind == NativeHookKind::StopObserved {
                let expected = if bash { MARKER } else { DENIED };
                observed_stop = hook.text.as_deref().map(str::trim) == Some(expected);
            }
        }
        for event in decisions.drain() {
            match event["type"].as_str() {
                Some("request.opened") => {
                    if opened.is_some()
                        || event["tool"] != expected_tool
                        || event["kind"] != if bash { "approval" } else { "plan" }
                        || bash && !valid_bash_input(&event["input"])
                    {
                        return Err("unexpected request; no permission response sent".into());
                    }
                    opened = Some(event);
                }
                Some("request.closed") => {
                    if opened
                        .as_ref()
                        .is_some_and(|opened| event["requestId"] == opened["requestId"])
                    {
                        if event["outcome"] != "resolved" {
                            return Err("decision hook closed without response delivery".into());
                        }
                        closed = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(event) = opened.as_ref() {
            let widget = if bash {
                visible.contains(COMMAND) && visible.contains("Do you want to proceed?")
            } else {
                plan_widget(&visible)
            };
            if widget && reply.is_none() && reply_outcome.is_none() {
                widget_seen = true;
                let registry = decisions.clone();
                let id = event["requestId"].clone();
                let response = if bash {
                    json!({"outcome":"allow"})
                } else {
                    json!({"outcome":"deny","message":format!("{DENIED}. Do not retry, implement anything, or use other tools. Reply exactly {DENIED}.")})
                };
                reply = Some(std::thread::spawn(move || {
                    registry.respond(
                        &json!({"type":"request.respond","requestId":id,"response":response}),
                    )
                }));
            }
        }
        if reply
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
        {
            reply_outcome = Some(
                reply
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| "reply worker panicked")?
                    .is_ok(),
            );
            if reply_outcome != Some(true) {
                return Err("explicit permission reply was not delivered".into());
            }
        }
        if observed_stop && closed && reply_outcome == Some(true) && (!bash || tool_finished) {
            let output = Path::new(&request.worktree).join("native-approval.txt");
            if bash {
                let metadata =
                    std::fs::symlink_metadata(&output).map_err(|_| "command output absent")?;
                if !metadata.is_file()
                    || std::fs::read(&output).map_err(|_| "command output unreadable")?
                        != MARKER.as_bytes()
                {
                    return Err("command did not write the exact scratch marker".into());
                }
            } else if output.exists() {
                return Err("denied no-op plan wrote an unexpected file".into());
            }
            let event = opened.as_ref().ok_or("missing opened request")?;
            if decisions.respond(&json!({"type":"request.respond","requestId":event["requestId"],"response":{"outcome":"allow"}})).is_ok() {
                return Err("closed request accepted a duplicate response".into());
            }
            let proof = json!({"version":"2.1.326","scenario":if bash {"explicit-bash-approval"}else{"explicit-plan-denial"},"nativeWidgetObservedBeforeReply":widget_seen,"exactInputChecked":true,"hookReplyDelivered":true,"cardOutcome":"resolved","nativeToolFinished":tool_finished,"nativeToolFailed":tool_failed,"exactScratchOutput":bash,"subsequentRootStopMarker":if bash {MARKER}else{DENIED},"duplicateReplyRejected":true});
            if std::env::var_os("PROMETEU_CAPTURE_DECISION_PROOF").is_some() {
                let name = if bash {
                    "claude-hook-bash.json"
                } else {
                    "claude-hook-plan-deny.json"
                };
                let target = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src/terminal_agent_decisions/fixtures")
                    .join(name);
                std::fs::write(target, serde_json::to_vec_pretty(&proof).unwrap())
                    .map_err(|_| "proof write failed")?;
            }
            println!("native decision conformance passed: tool={expected_tool}, widget=true, delivery=true, duplicate=false");
            return Ok(());
        }
    }
    let snapshot = screen
        .snapshot(emulator)
        .map_err(|_| "final snapshot failed")?
        .state;
    crate::paths::write_private(
        &crate::paths::root().join("last-native-ui.json"),
        &serde_json::to_string(&snapshot).map_err(|_| "snapshot serialization failed")?,
    )
    .map_err(|_| "private snapshot failed")?;
    Err(format!("decision deadline (tool={expected_tool}, opened={}, widget={widget_seen}, reply={reply_outcome:?}, closed={closed}, finished={tool_finished}, failed={tool_failed}, stop={observed_stop})",opened.is_some()))
}

#[test]
fn bash_smoke_rejects_extra_commands_and_execution_overrides() {
    assert!(valid_bash_input(
        &json!({"command":COMMAND,"description":"Write the smoke marker"})
    ));
    for command in [
        format!("{COMMAND}; pwd"),
        format!("{COMMAND}\npwd"),
        COMMAND.replace("native-approval.txt", "../native-approval.txt"),
    ] {
        assert!(!valid_bash_input(&json!({"command":command})));
    }
    for field in ["run_in_background", "dangerouslyDisableSandbox", "unknown"] {
        assert!(!valid_bash_input(&json!({"command":COMMAND,field:true})));
    }
}

#[test]
fn plan_smoke_recognizes_the_captured_empty_plan_dialog() {
    let visible = "Exit plan mode?\nClaude wants to exit plan mode\n❯ 1. Yes, and switch to default (ask each time) for this session\n2. No";
    assert!(plan_widget(visible));
    assert!(!plan_widget("Claude wants to exit plan mode"));
    assert!(!plan_widget(
        "WARNING: Claude Code running in Bypass Permissions mode"
    ));
}
