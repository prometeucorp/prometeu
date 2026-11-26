use super::*;
use std::{
    io::Write,
    os::unix::{fs::PermissionsExt, net::UnixListener},
};

const NATIVE: &str = "01a1e971-98a0-7853-b01d-9671f67a2596";
const CHILD: &str = "PROMETEU_CODEX_ENGINE_TEST";

#[test]
fn application_exit_reaps_engines_while_conversation_handles_remain_alive() {
    const CHILD: &str = "PROMETEU_CODEX_EXIT_TEST";
    const TEST: &str = "terminal_agent_codex::tests::application_exit_reaps_engines_while_conversation_handles_remain_alive";
    if std::env::var_os(CHILD).is_none() {
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap()
            .success());
        return;
    }
    let mut retained = Vec::new();
    for _ in 0..2 {
        let fixture = Fixture::new();
        let engine = Arc::new(
            fixture
                .plan("success", false)
                .start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
                .unwrap(),
        );
        let input = engine.input();
        retained.push((fixture, engine, input));
    }
    let starting_fixture = Fixture::new();
    let plan = starting_fixture.plan("wait-for-exit", false);
    let starting = std::thread::spawn(move || {
        plan.start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
            .unwrap()
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !starting_fixture.root.join("waiting-bootstrap").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    // AppState/Chat and an adapter port remain alive exactly as they can during Tauri Exit.
    // Concurrent explicit stop also has to await the same owning worker's reaping.
    let concurrent = retained[0].1.clone();
    let stopping = std::thread::spawn(move || concurrent.stop());
    let (exited, wait_exit) = mpsc::channel();
    let exiting = std::thread::spawn(move || {
        shutdown();
        exited.send(()).unwrap();
    });
    while lifecycle::admit().is_ok() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        wait_exit.try_recv().is_err(),
        "exit waits for an admitted bootstrap"
    );
    std::fs::write(starting_fixture.root.join("release-bootstrap"), "").unwrap();
    let started_during_exit = starting.join().unwrap();
    wait_exit.recv_timeout(Duration::from_secs(3)).unwrap();
    exiting.join().unwrap();
    assert!(!started_during_exit.running());
    starting_fixture.assert_reaped();
    for (fixture, engine, input) in &mut retained {
        assert!(!engine.running());
        fixture.assert_reaped();
        assert!(input
            .write_all(b"{\"method\":\"account/rateLimits/read\",\"id\":1}\n")
            .is_err());
        engine.stop();
    }
    stopping.join().unwrap();
    shutdown();
    let fixture = Fixture::new();
    assert!(matches!(
        fixture
            .plan("success", false)
            .start_bounded(&mut Command::new("codex"), Duration::from_secs(3)),
        Err(Error::Closed)
    ));
    assert!(
        !fixture.root.join("record").exists(),
        "exit forbids another native spawn"
    );
}

#[test]
fn native_identity_is_not_a_picker_or_unbounded_argument() {
    assert!(valid_id("01a1e971-98a0-7853-b01d-9671f67a2596"));
    for id in ["", "--last", "path/to/thread", "thread\nnext"] {
        assert!(!valid_id(id));
    }
    assert!(!valid_id(&"x".repeat(129)));
}

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("prometeu-engine-test-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let binary = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .replace('\'', "'\\''");
        // The parent validates as soon as bind exposes the socket. A later chmod alone leaves
        // a race where it correctly rejects the fixture's inherited group/world permissions.
        std::fs::write(root.join("engine"), format!(
            "#!/bin/sh\numask 077\nexport PROMETEU_CODEX_ENGINE_SOCKET=\"$2\"\nexec '{binary}' --exact terminal_agent_codex::tests::fake_engine_child --nocapture\n"
        )).unwrap();
        std::fs::set_permissions(root.join("engine"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        Self { root }
    }
    fn plan(&self, scenario: &str, resume: bool) -> NativeEnginePlan {
        let mut command = Command::new(self.root.join("engine"));
        command
            .env(CHILD, scenario)
            .env("PROMETEU_CODEX_ENGINE_RECORD", self.root.join("record"));
        NativeEnginePlan {
            command,
            settings: Settings {
                cwd: self.root.to_string_lossy().into(),
                resume: resume.then(|| NATIVE.into()),
                model: "fixture-model".into(),
                effort: "low".into(),
                instructions: "Keep the fixture instructions.".into(),
                ask: true,
                plan: true,
            },
            host: None,
        }
    }
    fn records(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("record"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn assert_reaped(&self) {
        let records = self.records();
        let pid = records[0]["pid"].as_i64().unwrap() as libc::pid_t;
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io_error(), Some(libc::ESRCH));
        assert!(!PathBuf::from(records[0]["socket"].as_str().unwrap())
            .parent()
            .unwrap()
            .exists());
    }
}
fn io_error() -> Option<i32> {
    std::io::Error::last_os_error().raw_os_error()
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn engine_acknowledges_exact_settings_before_native_resume_and_reaps_on_drop() {
    let fixture = Fixture::new();
    let mut tui = Command::new("codex");
    let engine = fixture
        .plan("success", false)
        .start_bounded(&mut tui, Duration::from_secs(3))
        .unwrap();
    assert_eq!(engine.thread_id(), NATIVE);
    assert!(engine.running());
    let args: Vec<_> = tui
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args[0], "--remote");
    assert!(args[1].starts_with("unix:///tmp/prometeu-codex-"));
    assert_eq!(&args[2..], &["resume", NATIVE]);
    let events = engine.take_events().unwrap();
    assert!(engine.take_events().is_none());
    let mut received = Vec::new();
    while let Ok(value) = events.recv_timeout(Duration::from_millis(80)) {
        received.push(value.unwrap());
    }
    assert!(received
        .iter()
        .any(|event| event["method"] == "thread/settings/updated"));
    let records = fixture.records();
    let methods: Vec<_> = records
        .iter()
        .filter_map(|event| event["method"].as_str())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "thread/start",
            "thread/name/set",
            "thread/settings/update",
            "thread/resume",
            "thread/turns/list"
        ]
    );
    assert_eq!(
        records
            .iter()
            .find(|event| event["method"] == "thread/start")
            .unwrap()["params"]["approvalPolicy"],
        "untrusted"
    );
    drop(engine);
    fixture.assert_reaped();
}

#[test]
fn resume_never_adopts_another_thread_or_releases_input_without_settings_ack() {
    for (scenario, resume) in [("wrong-thread", true), ("wrong-ack", false)] {
        let fixture = Fixture::new();
        let mut tui = Command::new("codex");
        let result = fixture
            .plan(scenario, resume)
            .start_bounded(&mut tui, Duration::from_secs(3));
        assert!(result.is_err());
        assert!(tui.get_args().next().is_none());
        fixture.assert_reaped();
    }
}

#[test]
fn resume_reapplies_selected_model_effort_ask_and_plan_before_releasing_tui() {
    use prometeu_core::session::launch::{Launch, StartMode};
    let fixture = Fixture::new();
    let request = LaunchRequest {
        session: "app".into(),
        workspace: "workspace".into(),
        worktree: fixture.root.to_string_lossy().into(),
        mode: StartMode::Resume {
            provider_session: Some(NATIVE.into()),
        },
        settings: Launch {
            model: "selected-model".into(),
            effort: "high".into(),
            permission: Some(Permission::Ask),
            plan: true,
            instructions: "Preserve selected instructions".into(),
            ..Default::default()
        },
    };
    let plan = NativeEnginePlan::new(
        fixture.plan("success", true).command,
        &request,
        Some(NATIVE.into()),
    );
    let mut tui = Command::new("codex");
    let engine = plan
        .start_bounded(&mut tui, Duration::from_secs(3))
        .unwrap();
    assert_eq!(engine.configuration()["model"], "selected-model");
    assert_eq!(engine.configuration()["reasoningEffort"], "high");
    assert_eq!(engine.configuration()["approvalPolicy"], "untrusted");
    assert_eq!(engine.configuration()["collaborationMode"]["mode"], "plan");
    let records = fixture.records();
    let resumed = records
        .iter()
        .find(|frame| frame["method"] == "thread/resume")
        .unwrap();
    assert_eq!(
        resumed["params"]["developerInstructions"],
        "Preserve selected instructions"
    );
    assert_eq!(resumed["params"]["model"], "selected-model");
    assert!(!tui
        .get_args()
        .any(|arg| arg == "--model" || arg == "--sandbox" || arg == "--ask-for-approval"));
    drop(engine);
    fixture.assert_reaped();
}

#[test]
fn history_pages_are_bound_ordered_and_reject_repeated_native_turns() {
    let fixture = Fixture::new();
    let engine = fixture
        .plan("history", true)
        .start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
        .unwrap();
    assert_eq!(
        engine
            .history()
            .iter()
            .map(|turn| turn["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["turn-one", "turn-two"]
    );
    let pages: Vec<_> = fixture
        .records()
        .into_iter()
        .filter(|frame| frame["method"] == "thread/turns/list")
        .collect();
    assert_eq!(pages.len(), 2);
    assert_eq!(
        pages[0]["params"],
        json!({"threadId":NATIVE,"cursor":null,"limit":16,"sortDirection":"asc","itemsView":"full"})
    );
    assert_eq!(pages[1]["params"]["cursor"], "next");
    drop(engine);
    fixture.assert_reaped();
    let fixture = Fixture::new();
    let mut tui = Command::new("codex");
    assert!(fixture
        .plan("history-duplicate", true)
        .start_bounded(&mut tui, Duration::from_secs(3))
        .is_err());
    assert_eq!(tui.get_args().count(), 0);
    fixture.assert_reaped();
}

#[test]
fn inline_images_survive_live_delivery_and_cold_history_replay() {
    for resume in [false, true] {
        let fixture = Fixture::new();
        let engine = fixture
            .plan("inline-image", resume)
            .start_bounded(&mut Command::new("codex"), Duration::from_secs(5))
            .unwrap();
        let item = inline_image_item();
        assert_eq!(engine.history()[0]["items"][0], item);
        let events = engine.take_events().unwrap();
        loop {
            let event = events
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            if event["method"] == "item/completed" {
                assert_eq!(event["params"]["item"], item);
                break;
            }
        }
        assert!(engine.running());
        drop(engine);
        fixture.assert_reaped();
    }
}

fn inline_image_item() -> Value {
    json!({"id":"image-input","type":"userMessage","content":[{
        "type":"image","image_url":format!("data:image/png;base64,{}", "A".repeat(2 * 1024 * 1024))
    }]})
}

#[test]
fn oversized_native_frame_reports_limit_and_reaps_engine() {
    let fixture = Fixture::new();
    let mut tui = Command::new("codex");
    assert!(matches!(
        fixture
            .plan("oversized-frame", true)
            .start_bounded(&mut tui, Duration::from_secs(3)),
        Err(Error::Limit)
    ));
    assert_eq!(tui.get_args().count(), 0);
    fixture.assert_reaped();
}

#[test]
fn unsolicited_native_requests_wait_for_explicit_response_and_resolved_ids_expire() {
    let fixture = Fixture::new();
    let engine = fixture
        .plan("request", false)
        .start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
        .unwrap();
    let events = engine.take_events().unwrap();
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        if event["method"] == "item/commandExecution/requestApproval" {
            break;
        }
    }
    assert!(fixture
        .records()
        .iter()
        .all(|value| value.get("method").is_some() || value.get("pid").is_some()));
    assert!(engine
        .respond(json!("unknown-request"), json!({"decision":"decline"}))
        .is_err());
    engine
        .respond(json!("native-approval"), json!({"decision":"decline"}))
        .unwrap();
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        if event["method"] == "serverRequest/resolved" {
            break;
        }
    }
    assert!(engine
        .respond(json!("native-approval"), json!({"decision":"accept"}))
        .is_err());
    let records = fixture.records();
    assert_eq!(
        records
            .iter()
            .filter(|value| value.get("result").is_some())
            .count(),
        1
    );
    engine.stop();
    engine.stop();
    fixture.assert_reaped();
}

#[test]
fn bounded_native_stream_failure_retires_the_engine() {
    let fixture = Fixture::new();
    let engine = fixture
        .plan("overflow", false)
        .start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while engine.running() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!engine.running());
    engine.stop();
    fixture.assert_reaped();
}

#[test]
fn attached_protocol_port_cannot_start_or_retarget_native_turns() {
    let fixture = Fixture::new();
    let engine = fixture
        .plan("success", false)
        .start_bounded(&mut Command::new("codex"), Duration::from_secs(3))
        .unwrap();
    let events = engine.take_events().unwrap();
    let mut input = engine.input();
    for frame in [
        json!({"id":1,"method":"initialize","params":{}}),
        json!({"id":2,"method":"turn/start","params":{"threadId":NATIVE}}),
        json!({"id":3,"method":"turn/interrupt","params":{"threadId":"another-thread","turnId":"turn"}}),
    ] {
        assert!(writeln!(input, "{frame}").is_err());
    }
    writeln!(
        input,
        "{}",
        json!({"id":4,"method":"account/rateLimits/read","params":{}})
    )
    .unwrap();
    writeln!(input,"{}",json!({"id":5,"method":"turn/interrupt","params":{"threadId":NATIVE,"turnId":"native-turn"}})).unwrap();
    loop {
        let event = events
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        if event["id"] == 5 {
            break;
        }
    }
    assert_eq!(
        fixture
            .records()
            .iter()
            .filter(|value| value["method"] == "initialize")
            .count(),
        1
    );
    assert!(!fixture
        .records()
        .iter()
        .any(|value| value["method"] == "turn/start"));
    engine.stop();
    assert!(writeln!(
        input,
        "{}",
        json!({"id":6,"method":"account/rateLimits/read","params":{}})
    )
    .is_err());
    fixture.assert_reaped();
}

#[test]
fn fake_engine_child() {
    let Ok(scenario) = std::env::var(CHILD) else {
        return;
    };
    let socket = std::env::var("PROMETEU_CODEX_ENGINE_SOCKET").unwrap();
    let socket = socket.strip_prefix("unix://").unwrap();
    let listener = UnixListener::bind(socket).unwrap();
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut record =
        std::fs::File::create(std::env::var_os("PROMETEU_CODEX_ENGINE_RECORD").unwrap()).unwrap();
    writeln!(
        record,
        "{}",
        json!({"pid":std::process::id(),"socket":socket})
    )
    .unwrap();
    let (stream, _) = listener.accept().unwrap();
    let mut socket = tungstenite::accept(stream).unwrap();
    let mut settings = json!({});
    let mut cwd = Value::Null;
    let mut model = json!("fixture-model");
    let mut effort = json!("low");
    loop {
        let Ok(tungstenite::Message::Text(text)) = socket.read() else {
            break;
        };
        let message: Value = serde_json::from_str(&text).unwrap();
        writeln!(record, "{message}").unwrap();
        record.flush().unwrap();
        if message["id"] == "native-approval" && message.get("method").is_none() {
            socket
                .send(tungstenite::Message::Text(
                    json!({"method":"serverRequest/resolved",
                "params":{"threadId":NATIVE,"requestId":"native-approval"}})
                    .to_string()
                    .into(),
                ))
                .unwrap();
            continue;
        }
        let Some(id) = message.get("id") else {
            continue;
        };
        if scenario == "oversized-frame" {
            // Announce an oversized text frame without allocating its payload. The client
            // must reject its length before reading or allocating the body.
            let stream = socket.get_mut();
            stream.write_all(&[0x81, 127]).unwrap();
            stream
                .write_all(&(64 * 1024 * 1024 + 1u64).to_be_bytes())
                .unwrap();
            continue;
        }
        if scenario == "wait-for-exit" && message["method"] == "initialize" {
            let root = PathBuf::from(std::env::var_os("PROMETEU_CODEX_ENGINE_RECORD").unwrap());
            let root = root.parent().unwrap();
            std::fs::write(root.join("waiting-bootstrap"), "").unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while !root.join("release-bootstrap").exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if message["params"]["cwd"].is_string() {
            cwd = message["params"]["cwd"].clone();
        }
        if message["params"]["model"].is_string() {
            model = message["params"]["model"].clone();
        }
        if message["method"] == "thread/settings/update" {
            settings = message["params"].clone();
            model = settings["collaborationMode"]["settings"]["model"].clone();
            effort = settings["effort"].clone();
        }
        let result = match message["method"].as_str().unwrap() {
            "thread/start" | "thread/resume" => {
                json!({"thread":{"id":if scenario == "wrong-thread" {
                "01a1e971-98a0-7853-b01d-9671f67a2597" } else {NATIVE}},
                "cwd":cwd,"model":model,"reasoningEffort":effort,
                "approvalPolicy":if scenario == "wrong-ack" {json!("on-request")} else {settings["approvalPolicy"].clone()},
                "sandbox":settings["sandboxPolicy"],"collaborationMode":settings["collaborationMode"]})
            }
            "thread/turns/list" if matches!(scenario.as_str(), "history" | "history-duplicate") => {
                if message["params"]["cursor"].is_null() {
                    json!({"data":[{"id":"turn-one","status":"completed","items":[]}],"nextCursor":"next"})
                } else {
                    json!({"data":[{"id":if scenario == "history" {"turn-two"} else {"turn-one"},"status":"completed","items":[]}],"nextCursor":null})
                }
            }
            "thread/turns/list" if scenario == "inline-image" => {
                json!({"data":[{"id":"image-turn","items":[inline_image_item()]}],"nextCursor":null})
            }
            "thread/turns/list" => json!({"data":[],"nextCursor":null}),
            _ => json!({}),
        };
        socket
            .send(tungstenite::Message::Text(
                json!({"id":id,"result":result}).to_string().into(),
            ))
            .unwrap();
        if message["method"] == "thread/settings/update" {
            let mut observed = settings.clone();
            observed.as_object_mut().unwrap().remove("threadId");
            if scenario == "wrong-ack" {
                observed["approvalPolicy"] = "on-request".into();
            }
            socket
                .send(tungstenite::Message::Text(
                    json!({"method":"thread/settings/updated",
                "params":{"threadId":NATIVE,"threadSettings":observed}})
                    .to_string()
                    .into(),
                ))
                .unwrap();
            if scenario == "request" {
                socket
                    .send(tungstenite::Message::Text(
                        json!({"id":"native-approval",
                    "method":"item/commandExecution/requestApproval", "params":{"threadId":NATIVE}})
                        .to_string()
                        .into(),
                    ))
                    .unwrap();
            }
        }
        if scenario == "inline-image" && message["method"] == "thread/turns/list" {
            socket
                .send(tungstenite::Message::Text(
                    json!({"method":"item/completed","params":{
                        "threadId":NATIVE,"turnId":"image-turn","item":inline_image_item()
                    }})
                    .to_string()
                    .into(),
                ))
                .unwrap();
        }
        if scenario == "overflow" && message["method"] == "thread/turns/list" {
            for index in 0..QUEUE_LIMIT + 5 {
                if socket
                    .send(tungstenite::Message::Text(
                        json!({"method":"fixture/event","params":{"index":index}})
                            .to_string()
                            .into(),
                    ))
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}
