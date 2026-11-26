//! Opt-in native UI proofs in isolated scratch sessions; no global configuration mutation.
use super::*;
use prometeu_core::terminal::{TerminalFactory, TerminalSize};
use prometeu_process::terminal_state::protocol::{BufferKind, Cursor};
use prometeu_process::terminal_state::{Client as ScreenClient, Key, Size, Snapshot, Stream};
use std::{io::Read, os::unix::fs::PermissionsExt, path::Path, thread::JoinHandle};

#[path = "decision_smoke.rs"]
mod decision;

#[test]
#[ignore = "requires an explicit owned app-conformance fixture; starts no model turn"]
fn native_engine_opens_app_conformance_workspace_without_inference() {
    const CHILD: &str = "PROMETEU_CODEX_ENGINE_APP_PROOF_CHILD";
    const TEST: &str = "terminal_agent_codex::smoke::native_engine_opens_app_conformance_workspace_without_inference";
    let root = PathBuf::from(std::env::var_os("PROMETEU_CODEX_ENGINE_APP_PROOF").unwrap());
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("prometeu-app-native-conformance-"));
    let env: Value =
        serde_json::from_slice(&std::fs::read(root.join("launch-env.json")).unwrap()).unwrap();
    let cwd = root.join("workspace-codex");
    assert!(cwd.is_dir());
    if std::env::var_os(CHILD).is_none() {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", TEST, "--ignored", "--nocapture"])
            .env(CHILD, "1");
        for (key, value) in env["set"].as_object().unwrap() {
            child.env(key, value.as_str().unwrap());
        }
        for key in env["remove"].as_array().unwrap() {
            child.env_remove(key.as_str().unwrap());
        }
        assert!(
            child.status().unwrap().success(),
            "full native preparation failed"
        );
        return;
    }
    crate::install_crypto();
    crate::adopt_login_path();
    use prometeu_core::session::launch::{Launch, StartMode};
    let request = if let Ok(session) = std::env::var("PROMETEU_CODEX_ENGINE_APP_TAB") {
        let board: crate::state::Board = serde_json::from_slice(
            &std::fs::read(crate::paths::root().join("board.json")).unwrap(),
        )
        .unwrap();
        let workspace = board.workspace_of(&session).expect("owned fixture tab");
        let tab = workspace.tabs.iter().find(|tab| tab.id == session).unwrap();
        let tools = crate::session::resolve_workspace_tools(
            &board.tools,
            &board.tool_trust,
            workspace,
            crate::state::ProviderId::Codex,
        );
        let settings = crate::session::workspace_launch_of(workspace, &session, &tools);
        assert_eq!(settings.agent, crate::state::ProviderId::Codex);
        assert_eq!(workspace.worktree, cwd.to_string_lossy());
        assert!(tab.kickoff.is_none());
        LaunchRequest {
            session,
            workspace: workspace.id.clone(),
            worktree: workspace.worktree.clone(),
            settings,
            mode: StartMode::Resume {
                provider_session: Some(tab.agent_session.clone().unwrap()),
            },
        }
    } else {
        LaunchRequest {
            session: uuid::Uuid::new_v4().to_string(),
            workspace: uuid::Uuid::new_v4().to_string(),
            worktree: cwd.to_string_lossy().into(),
            mode: StartMode::Fresh,
            settings: Launch {
                agent: crate::state::ProviderId::Codex,
                effort: "low".into(),
                permission: Some(Permission::Ask),
                plan: false,
                mcp: Some(vec![]),
                plugins: Some(vec![]),
                skills: Some(vec![]),
                ..Default::default()
            },
        }
    };
    let mut prepared = crate::terminal_agent_launch::prepare(
        &request,
        &crate::account_profiles::native(),
        &crate::tool_materialization::native(),
    )
    .unwrap();
    let (mut bridge, _hooks) = crate::terminal_agent_hooks::HookBridge::start(
        crate::state::ProviderId::Codex,
        &request.session,
        &uuid::Uuid::new_v4().to_string(),
        match &request.mode {
            StartMode::Fresh => None,
            StartMode::Resume { provider_session } => provider_session.as_deref(),
        },
    )
    .unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/Prometeu");
    assert!(helper.is_file());
    bridge
        .configure_with_helper(
            &mut prepared.engine.as_mut().unwrap().command,
            prepared.hook_probe.as_mut(),
            &helper,
        )
        .unwrap();
    let engine = prepared
        .engine
        .take()
        .unwrap()
        .start_inner(&mut prepared.command)
        .unwrap();
    assert_eq!(
        engine.configuration()["approvalPolicy"],
        if request.settings.permission == Some(Permission::Ask) {
            "untrusted"
        } else {
            "never"
        }
    );
    assert_eq!(engine.configuration()["reasoningEffort"], "low");
    assert_eq!(
        engine.configuration()["collaborationMode"]["mode"],
        if request.settings.plan {
            "plan"
        } else {
            "default"
        }
    );
    match &request.mode {
        StartMode::Fresh => assert!(engine.history().is_empty()),
        StartMode::Resume { provider_session } => {
            assert_eq!(Some(engine.thread_id()), provider_session.as_deref());
        }
    }
    engine.stop();
    assert!(!engine.running());
    bridge.close();
}

#[test]
#[ignore = "uses installed Codex and copied external authentication, but sends no inference request"]
fn native_remote_tui_preserves_ask_and_plan_on_fresh_and_cold_resume() {
    let (cleanup, home, cwd, helper) = scratch();
    let mut thread = None;
    for phase in ["fresh", "cold-resume"] {
        let mut command = Command::new("codex");
        command
            .arg("app-server")
            .current_dir(&cwd)
            .env("CODEX_HOME", &home)
            .env_remove("CLAUDECODE");
        let plan = NativeEnginePlan {
            command,
            settings: Settings {
                cwd: cwd.to_string_lossy().into(),
                resume: thread.clone(),
                model: String::new(),
                effort: "low".into(),
                instructions: String::new(),
                ask: true,
                plan: true,
            },
            host: None,
        };
        let mut tui = Command::new("codex");
        tui.current_dir(&cwd)
            .env("CODEX_HOME", &home)
            .env_remove("CLAUDECODE");
        let engine = plan.start_inner(&mut tui).unwrap();
        assert_eq!(engine.configuration()["thread"]["status"]["type"], "idle");
        if let Some(previous) = &thread {
            assert_eq!(engine.thread_id(), previous);
        }
        thread = Some(engine.thread_id().to_owned());
        let events = engine.take_events().unwrap();
        let mut screen =
            ScreenClient::launch(Command::new(&helper), Duration::from_secs(5)).unwrap();
        let mut stream = Stream::create(
            &mut screen,
            Key {
                session: "engine-smoke".into(),
                generation: phase.into(),
            },
            Size {
                cols: 120,
                rows: 40,
            },
            1000,
        )
        .unwrap();
        let started = prometeu_process::terminal::UnixTerminalFactory
            .open(
                pty_command(&tui),
                TerminalSize {
                    cols: 120,
                    rows: 40,
                },
            )
            .unwrap();
        let mut input = started.input;
        let mut output = started.output;
        let control = started.control;
        let mut waiter = started.waiter;
        let wait = std::thread::spawn(move || {
            let _ = waiter.wait();
        });
        let (send, receive) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut bytes = [0; 8192];
            while let Ok(count) = output.read(&mut bytes) {
                if count == 0 || send.send(bytes[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut terminal = TerminalCleanup {
            control,
            wait: Some(wait),
            reader: Some(reader),
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut chunk = 0;
        let mut snapshot = None;
        while Instant::now() < deadline {
            if let Ok(bytes) = receive.recv_timeout(Duration::from_millis(50)) {
                chunk += 1;
                let applied = stream.write_chunk(&mut screen, chunk, &bytes).unwrap();
                for reply in applied.replies {
                    input.write_all(&reply).unwrap();
                    input.flush().unwrap();
                }
                let current = stream.snapshot(&mut screen).unwrap().state;
                if current
                    .viewport_lines
                    .iter()
                    .any(|line| line.contains("Plan mode"))
                {
                    snapshot = Some(current);
                    break;
                }
            }
            assert!(
                engine.running(),
                "private engine exited before native UI readiness"
            );
        }
        let snapshot = snapshot.expect("native remote TUI did not render Plan mode");
        assert!(terminal.control.running());
        let mut methods = Vec::new();
        let acked = engine.configuration()["approvalPolicy"] == "untrusted"
            && engine.configuration()["collaborationMode"]["mode"] == "plan";
        while let Ok(event) = events.try_recv() {
            let event = event.unwrap();
            if let Some(method) = event["method"].as_str() {
                assert!(!matches!(
                    method,
                    "turn/started" | "item/started" | "turn/completed"
                ));
                methods.push(method.to_owned());
            }
            if event["method"] == "thread/settings/updated" {
                assert_eq!(event["params"]["threadId"], thread.as_deref().unwrap());
                assert_eq!(
                    event["params"]["threadSettings"]["approvalPolicy"],
                    "untrusted"
                );
                assert_eq!(
                    event["params"]["threadSettings"]["collaborationMode"]["mode"],
                    "plan"
                );
            }
        }
        assert!(acked);
        if std::env::var_os("PROMETEU_CAPTURE_NATIVE_ENGINE").is_some() {
            assert!(
                snapshot
                    .viewport_lines
                    .iter()
                    .all(|line| !line.contains('@')),
                "native screen contains an account identifier"
            );
            crate::paths::write_private(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "src/terminal_agent_codex/fixtures/codex-remote-{phase}-ready.json"
                )),
                &serde_json::to_string_pretty(
                    &json!({"version":VERIFIED_VERSION,"snapshot":snapshot}),
                )
                .unwrap(),
            )
            .unwrap();
            let lines: Vec<_> = snapshot
                .viewport_lines
                .iter()
                .map(|line| {
                    if line.contains('@') {
                        String::new()
                    } else {
                        line.replace(cwd.to_str().unwrap(), "/tmp/engine-smoke/workspace")
                            .replace(home.to_str().unwrap(), "/tmp/engine-smoke/codex")
                    }
                })
                .collect();
            let proof = json!({"version":VERIFIED_VERSION,"phase":phase,"approvalPolicy":"untrusted",
                "sandboxPolicy":"dangerFullAccess","collaborationMode":"plan","effort":"low",
                "exactNativeThread":true,"nativeTuiAlive":true,"inferenceRequested":false,
                "notificationMethods":methods,"lines":lines});
            crate::paths::write_private(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("src/terminal_agent_codex/fixtures/{phase}.json")),
                &serde_json::to_string_pretty(&proof).unwrap(),
            )
            .unwrap();
        }
        if phase == "fresh" {
            input
                .write_all(b"\x1b[200~Ask Codex to do anything\x1b[201~")
                .unwrap();
            input.flush().unwrap();
            wait_paint(
                &mut input,
                &receive,
                &mut stream,
                &mut screen,
                &mut chunk,
                |snapshot| active_cursor(snapshot).x > 2,
            );
            input.write_all(b"\x1b[H").unwrap();
            input.flush().unwrap();
            let literal = wait_paint(
                &mut input,
                &receive,
                &mut stream,
                &mut screen,
                &mut chunk,
                |snapshot| active_cursor(snapshot).x == 2,
            );
            capture_screen("codex-remote-literal-home", &literal);
            input.write_all(b"\x1b[F\x15").unwrap();
            input.flush().unwrap();
            let cleared = wait_paint(
                &mut input,
                &receive,
                &mut stream,
                &mut screen,
                &mut chunk,
                |snapshot| {
                    let row = usize::from(active_cursor(snapshot).y);
                    active_cursor(snapshot).x == 2
                        && snapshot.viewport_styles[row]
                            .iter()
                            .any(|style| style.dim && style.start <= 2 && style.end > 2)
                },
            );
            capture_screen("codex-remote-cleared", &cleared);
            for event in events.try_iter() {
                assert!(!matches!(
                    event.unwrap()["method"].as_str(),
                    Some("turn/started" | "item/started" | "turn/completed")
                ));
            }
        }
        terminal.close();
        stream.dispose(&mut screen).unwrap();
        screen.shutdown().unwrap();
        engine.stop();
        assert!(!engine.running());
    }
    drop(cleanup);
}

#[test]
#[ignore = "one bounded no-tool native TUI inference in an isolated authenticated scratch"]
fn native_remote_turn_records_rpc_and_rollout_identity() {
    let (cleanup, home, cwd, helper) = scratch();
    let mut command = Command::new("codex");
    command
        .arg("app-server")
        .current_dir(&cwd)
        .env("CODEX_HOME", &home)
        .env_remove("CLAUDECODE");
    let plan = NativeEnginePlan {
        command,
        settings: Settings {
            cwd: cwd.to_string_lossy().into(),
            resume: None,
            model: String::new(),
            effort: "low".into(),
            instructions: String::new(),
            ask: true,
            plan: false,
        },
        host: None,
    };
    let mut tui = Command::new("codex");
    tui.current_dir(&cwd)
        .env("CODEX_HOME", &home)
        .env_remove("CLAUDECODE");
    let engine = plan.start_inner(&mut tui).unwrap();
    let thread = engine.thread_id().to_owned();
    let events = engine.take_events().unwrap();
    let mut screen = ScreenClient::launch(Command::new(&helper), Duration::from_secs(5)).unwrap();
    let mut stream = Stream::create(
        &mut screen,
        Key {
            session: "engine-identity".into(),
            generation: "one".into(),
        },
        Size {
            cols: 120,
            rows: 40,
        },
        1000,
    )
    .unwrap();
    let started = prometeu_process::terminal::UnixTerminalFactory
        .open(
            pty_command(&tui),
            TerminalSize {
                cols: 120,
                rows: 40,
            },
        )
        .unwrap();
    let mut input = started.input;
    let mut output = started.output;
    let mut waiter = started.waiter;
    let wait = std::thread::spawn(move || {
        let _ = waiter.wait();
    });
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = [0; 8192];
        while let Ok(count) = output.read(&mut bytes) {
            if count == 0 || send.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });
    let mut terminal = TerminalCleanup {
        control: started.control,
        wait: Some(wait),
        reader: Some(reader),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut chunk = 0;
    let mut sent = false;
    let mut completed = false;
    let mut frames = Vec::new();
    while Instant::now() < deadline && !completed {
        if let Ok(bytes) = receive.recv_timeout(Duration::from_millis(20)) {
            chunk += 1;
            for reply in stream
                .write_chunk(&mut screen, chunk, &bytes)
                .unwrap()
                .replies
            {
                input.write_all(&reply).unwrap();
                input.flush().unwrap();
            }
            if !sent
                && stream
                    .snapshot(&mut screen)
                    .unwrap()
                    .state
                    .viewport_lines
                    .iter()
                    .any(|line| line.starts_with("› "))
            {
                input.write_all(b"\x1b[200~Reply exactly PROMETEU_NATIVE_ID_OK. Do not use tools.\x1b[201~\r").unwrap();
                input.flush().unwrap();
                sent = true;
            }
        }
        while let Ok(event) = events.try_recv() {
            let event = event.unwrap();
            assert!(
                event.get("id").is_none() || event.get("method").is_none(),
                "unexpected native decision request"
            );
            if event["params"]["threadId"].as_str() == Some(&thread) {
                completed |= event["method"] == "turn/completed";
                frames.push(event);
                assert!(frames.len() < 2000);
            }
        }
        assert!(engine.running());
    }
    assert!(sent && completed, "bounded native turn did not finish");
    terminal.close();
    stream.dispose(&mut screen).unwrap();
    screen.shutdown().unwrap();
    engine.stop();
    assert!(!engine.running());
    let mut resume = Command::new("codex");
    resume
        .arg("app-server")
        .current_dir(&cwd)
        .env("CODEX_HOME", &home)
        .env_remove("CLAUDECODE");
    let resumed = NativeEnginePlan {
        command: resume,
        settings: Settings {
            cwd: cwd.to_string_lossy().into(),
            resume: Some(thread.clone()),
            model: String::new(),
            effort: "low".into(),
            instructions: String::new(),
            ask: true,
            plan: false,
        },
        host: None,
    }
    .start_inner(&mut Command::new("codex"))
    .unwrap();
    assert_eq!(resumed.history().len(), 1);
    let consumed = crate::chat::terminal::codex::real_proof(&thread, &frames, resumed.history());
    resumed.stop();
    assert!(!resumed.running());
    let mut pending = vec![home.join("sessions")];
    let mut rollout = None;
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry.file_name().to_string_lossy().contains(&thread) {
                rollout = Some(entry.path());
            }
        }
    }
    let raw = std::fs::read_to_string(rollout.expect("owned native rollout")).unwrap();
    assert!(raw.len() < 1024 * 1024);
    let records: Vec<Value> = raw
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let items: Vec<Value> = frames
        .iter()
        .filter(|frame| frame["method"] == "item/completed")
        .map(|frame| json!({"turnId":frame["params"]["turnId"],"item":frame["params"]["item"]}))
        .collect();
    let native: Vec<Value> = records
        .iter()
        .filter_map(|record| {
            let payload = &record["payload"];
            match (record["type"].as_str(), payload["type"].as_str()) {
                (
                    Some("event_msg"),
                    Some("user_message" | "item_completed" | "task_started" | "task_complete"),
                ) => Some(payload.clone()),
                (Some("response_item"), Some("message")) if payload["role"] == "assistant" => {
                    Some(payload.clone())
                }
                _ => None,
            }
        })
        .collect();
    let user = items
        .iter()
        .find(|item| item["item"]["type"] == "userMessage")
        .expect("native user item");
    let assistant = items
        .iter()
        .find(|item| item["item"]["type"] == "agentMessage")
        .expect("native assistant item");
    assert_eq!(assistant["item"]["text"], "PROMETEU_NATIVE_ID_OK");
    let proof = json!({"version":VERIFIED_VERSION,"oneNativeTurn":true,"engineAndTuiReaped":true,
        "rpcUser":user,"rpcAssistant":assistant,"rollout":native,"consumer":consumed});
    if std::env::var_os("PROMETEU_CAPTURE_NATIVE_ENGINE").is_some() {
        crate::paths::write_private(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/terminal_agent_codex/fixtures/turn-identity.json"),
            &serde_json::to_string_pretty(&proof).unwrap(),
        )
        .unwrap();
    }
    drop(cleanup);
}

fn active_cursor(snapshot: &Snapshot) -> &Cursor {
    match snapshot.active {
        BufferKind::Normal => &snapshot.normal.cursor,
        BufferKind::Alternate => &snapshot.alternate.as_ref().unwrap().cursor,
    }
}
fn capture_screen(label: &str, snapshot: &Snapshot) {
    if std::env::var_os("PROMETEU_CAPTURE_NATIVE_ENGINE").is_some() {
        assert!(snapshot
            .viewport_lines
            .iter()
            .all(|line| !line.contains('@')));
        crate::paths::write_private(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("src/terminal_agent_codex/fixtures/{label}.json")),
            &serde_json::to_string_pretty(&json!({"version":VERIFIED_VERSION,"snapshot":snapshot}))
                .unwrap(),
        )
        .unwrap();
    }
}
fn wait_paint(
    input: &mut impl Write,
    receive: &mpsc::Receiver<Vec<u8>>,
    stream: &mut Stream,
    screen: &mut ScreenClient,
    chunk: &mut u64,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(bytes) = receive.recv_timeout(Duration::from_millis(20)) {
            *chunk += 1;
            for reply in stream.write_chunk(screen, *chunk, &bytes).unwrap().replies {
                input.write_all(&reply).unwrap();
                input.flush().unwrap();
            }
            let snapshot = stream.snapshot(screen).unwrap().state;
            if predicate(&snapshot) {
                return snapshot;
            }
        }
    }
    panic!("native composer did not paint the requested input state");
}

fn scratch() -> (Cleanup, PathBuf, PathBuf, PathBuf) {
    let version = prometeu_protocols::catalog::command_output(
        &prometeu_process::query::UnixQueryLauncher,
        Command::new("codex").arg("--version"),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!(version.trim(), VERIFIED_VERSION);
    let root =
        PathBuf::from("/tmp").join(format!("prometeu-engine-smoke-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&root)
        .unwrap();
    let cleanup = Cleanup(root.clone());
    let home = root.join("codex");
    let cwd = root.join("workspace");
    for directory in [&home, &cwd] {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(directory)
            .unwrap();
    }
    let external = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap().join(".codex"));
    std::fs::copy(external.join("auth.json"), home.join("auth.json")).unwrap();
    std::fs::set_permissions(
        home.join("auth.json"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let config = format!("approval_policy = \"never\"\nsandbox_mode = \"danger-full-access\"\n[features]\napps = false\nplugins = false\nhooks = false\n[projects.{}]\ntrust_level = \"trusted\"\n", toml::Value::String(cwd.to_string_lossy().into()));
    crate::paths::write_private(&home.join("config.toml"), &config).unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("packages/terminal-state/dist/prometeu-terminal-state-aarch64-apple-darwin");
    assert!(
        helper.is_file(),
        "build the macOS arm64 terminal-state helper first"
    );
    (cleanup, home, cwd, helper)
}

struct Cleanup(PathBuf);
fn pty_command(command: &Command) -> portable_pty::CommandBuilder {
    let mut terminal = portable_pty::CommandBuilder::new(command.get_program());
    prometeu_process::terminal::own_terminal(&mut terminal, Some(env!("CARGO_PKG_VERSION")));
    terminal.args(command.get_args());
    if let Some(cwd) = command.get_current_dir() {
        terminal.cwd(cwd);
    }
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            terminal.env(key, value);
        } else {
            terminal.env_remove(key);
        }
    }
    terminal
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct TerminalCleanup {
    control: Arc<dyn prometeu_core::terminal::TerminalControl>,
    wait: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}
impl TerminalCleanup {
    fn close(&mut self) {
        self.control.close();
        if let Some(wait) = self.wait.take() {
            let _ = wait.join();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
impl Drop for TerminalCleanup {
    fn drop(&mut self) {
        self.close();
    }
}
