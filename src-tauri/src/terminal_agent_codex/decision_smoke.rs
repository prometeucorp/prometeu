//! Native decisions answered through the same attached adapter as app controls.
use super::*;
use prometeu_protocols::codex::{Link, Start};

#[test]
#[ignore = "one native Plan question in an isolated authenticated scratch; no filesystem tools"]
fn native_question_accepts_an_explicit_attached_answer() {
    run(Case::Question);
}

#[test]
#[ignore = "one explicit native command approval; writes only a marker inside an isolated scratch"]
fn native_command_accepts_an_explicit_attached_approval() {
    run(Case::Command);
}

const COMMAND: &str = "printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt";
#[derive(Clone, Copy)]
enum Case {
    Question,
    Command,
}
impl Case {
    fn plan(self) -> bool {
        matches!(self, Self::Question)
    }
    fn instructions(self) -> &'static str {
        match self {
            Self::Question => "This is an isolated native question test. Use only request_user_input, exactly once. Never read files, run commands, use network tools, or create a plan. After the answer, reply with exactly the selected color and finish.",
            Self::Command => "This is an isolated native command approval test. Use the shell execution tool exactly once to run the exact command requested. Do not inspect files, run any other commands, use network tools, edit files through other tools, or create a plan. After that command completes, reply exactly PROMETEU_TERMINAL_SMOKE and finish.",
        }
    }
    fn prompt(self) -> String {
        match self {
            Self::Question => "Use request_user_input once to ask exactly 'Which test color?' with options 'Blue' and 'Green'. Wait for my answer, then reply exactly the selected color. Do not use any other tool.".into(),
            Self::Command => format!("Use the shell execution tool exactly once to run `{COMMAND}` in the current scratch workspace. Request approval and wait for it if needed. Do not run any other command or use any other tool. Then reply exactly PROMETEU_TERMINAL_SMOKE."),
        }
    }
    fn method(self) -> &'static str {
        match self {
            Self::Question => "item/tool/requestUserInput",
            Self::Command => "item/commandExecution/requestApproval",
        }
    }
    fn visible(self, lines: &[String]) -> bool {
        match self {
            Self::Question => {
                lines.iter().any(|line| line.trim() == "Which test color?")
                    && lines
                        .iter()
                        .any(|line| line.trim().starts_with("› 1. Blue"))
                    && lines.iter().any(|line| line.trim().starts_with("2. Green"))
                    && lines.iter().any(|line| line.contains("to submit answer"))
            }
            Self::Command => {
                lines
                    .iter()
                    .any(|line| line.trim() == "Would you like to run the following command?")
                    && lines
                        .iter()
                        .any(|line| line.trim().strip_prefix("$ ") == Some(COMMAND))
                    && lines
                        .iter()
                        .any(|line| line.trim().starts_with("› 1. Yes, proceed"))
            }
        }
    }
    fn response(self, event: &Value) -> Value {
        match self {
            Self::Question => {
                assert_eq!(event["kind"], "question");
                let questions = event["input"]["questions"].as_array().unwrap();
                assert_eq!(questions.len(), 1);
                assert_eq!(questions[0]["question"], "Which test color?");
                json!({"outcome":"answer","answers":{"Which test color?":"Blue"}})
            }
            Self::Command => {
                assert_eq!(event["kind"], "approval");
                assert_eq!(event["tool"], "Bash");
                assert!(
                    event["input"]["command"]
                        .as_str()
                        .is_some_and(exact_command),
                    "unexpected presented command"
                );
                json!({"outcome":"allow"})
            }
        }
    }
    fn final_text(self) -> &'static str {
        match self {
            Self::Question => "Blue",
            Self::Command => "PROMETEU_TERMINAL_SMOKE",
        }
    }
}

fn exact_command(command: &str) -> bool {
    command == COMMAND
        || ["/bin/zsh", "/bin/bash", "/bin/sh"].iter().any(|shell| {
            ["-lc", "-c"].iter().any(|flag| {
                ['\'', '"']
                    .iter()
                    .any(|quote| command == format!("{shell} {flag} {quote}{COMMAND}{quote}"))
            })
        })
}

#[test]
fn command_smoke_allows_only_its_literal_payload_and_known_shell_wrapper() {
    assert!(exact_command(COMMAND));
    assert!(exact_command(&format!("/bin/zsh -lc '{COMMAND}'")));
    for other in [
        format!("{COMMAND}; pwd"),
        format!("{COMMAND}\nprintf extra"),
        format!("/bin/zsh -lc '{COMMAND}' extra"),
        "printf PROMETEU_TERMINAL_TOOL_SMOKE > ../native-approval.txt".into(),
        "printf PROMETEU_TERMINAL_TOOL_SMOKE > /tmp/native-approval.txt".into(),
    ] {
        assert!(!exact_command(&other));
    }
}

fn run(case: Case) {
    let (_cleanup, home, cwd, helper) = scratch();
    let mut command = Command::new("codex");
    command
        .arg("app-server")
        .current_dir(&cwd)
        .env("CODEX_HOME", &home)
        .env_remove("CLAUDECODE");
    let mut tui = Command::new("codex");
    tui.current_dir(&cwd)
        .env("CODEX_HOME", &home)
        .env_remove("CLAUDECODE");
    let engine = NativeEnginePlan {
        command,
        settings: Settings {
            cwd: cwd.to_string_lossy().into(),
            resume: None,
            model: String::new(),
            effort: "low".into(),
            instructions: case.instructions().into(),
            ask: true,
            plan: case.plan(),
        },
        host: None,
    }
    .start_inner(&mut tui)
    .unwrap();
    assert_eq!(engine.configuration()["approvalPolicy"], "untrusted");
    assert_eq!(
        engine.configuration()["collaborationMode"]["mode"],
        if case.plan() { "plan" } else { "default" }
    );
    let mut link = Link::attached(
        engine.input(),
        Start {
            cwd: cwd.to_string_lossy().into(),
            resume: None,
            model: engine.configuration()["model"].as_str().unwrap().into(),
            effort: "low".into(),
            plugin_ids: vec![],
            plugin_hook_ids: vec![],
            permission: Some(prometeu_core::actions::Permission::Ask),
            instructions: String::new(),
        },
        engine.thread_id().into(),
        engine.incarnation.clone(),
        |_, en| en.into(),
    );
    let events = engine.take_events().unwrap();
    let mut screen = ScreenClient::launch(Command::new(&helper), Duration::from_secs(5)).unwrap();
    let mut stream = Stream::create(
        &mut screen,
        Key {
            session: "engine-decision".into(),
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
    let mut request: Option<Value> = None;
    let mut response: Option<Value> = None;
    let mut resolved = false;
    let mut question_visible = false;
    let mut answered = false;
    let mut native_resolved = false;
    let mut command_completed = false;
    let mut completed = false;
    let mut final_text = None;
    while Instant::now() < deadline && !completed {
        if let Ok(bytes) = receive.recv_timeout(Duration::from_millis(10)) {
            chunk += 1;
            for reply in stream
                .write_chunk(&mut screen, chunk, &bytes)
                .unwrap()
                .replies
            {
                input.write_all(&reply).unwrap();
                input.flush().unwrap();
            }
            let snapshot = stream.snapshot(&mut screen).unwrap().state;
            if !sent
                && snapshot
                    .viewport_lines
                    .iter()
                    .any(|line| line.starts_with("› "))
            {
                input
                    .write_all(format!("\x1b[200~{}\x1b[201~\r", case.prompt()).as_bytes())
                    .unwrap();
                input.flush().unwrap();
                sent = true;
            }
            question_visible |= request.is_some() && case.visible(&snapshot.viewport_lines);
        }
        while let Ok(raw) = events.try_recv() {
            let raw = raw.unwrap();
            if raw.get("id").is_some() && raw.get("method").is_some() {
                assert_eq!(raw["method"], case.method(), "unexpected native request");
                assert_eq!(raw["params"]["threadId"], engine.thread_id());
                assert!(request.is_none(), "more than one native decision request");
                if matches!(case, Case::Command) {
                    assert!(
                        raw["params"]["command"].as_str().is_some_and(exact_command),
                        "unexpected native command; nothing approved"
                    );
                    let native_cwd =
                        Path::new(raw["params"]["cwd"].as_str().expect("approval cwd missing"));
                    assert_eq!(
                        native_cwd.canonicalize().unwrap(),
                        cwd.canonicalize().unwrap()
                    );
                }
                request = Some(raw.clone());
            }
            if raw["method"] == "serverRequest/resolved" {
                assert_eq!(raw["params"]["threadId"], engine.thread_id());
                assert_eq!(raw["params"]["requestId"], request.as_ref().unwrap()["id"]);
                native_resolved = true;
            }
            if raw["method"] == "item/completed"
                && raw["params"]["item"]["type"] == "commandExecution"
            {
                assert!(
                    matches!(case, Case::Command),
                    "unexpected native shell call"
                );
                assert!(
                    answered && native_resolved,
                    "command executed without an explicit resolved approval"
                );
                let item = &raw["params"]["item"];
                assert_eq!(item["id"], request.as_ref().unwrap()["params"]["itemId"]);
                assert_eq!(item["status"], "completed");
                assert_eq!(item["exitCode"], 0);
                assert!(!command_completed, "more than one native command completed");
                command_completed = true;
            }
            if raw["method"] == "item/completed" && raw["params"]["item"]["type"] == "agentMessage"
            {
                final_text = Some(raw["params"]["item"]["text"].clone());
            }
            if raw["method"] == "turn/completed" {
                assert_eq!(raw["params"]["threadId"], engine.thread_id());
                assert_eq!(raw["params"]["turn"]["status"], "completed");
                completed = true;
            }
            for event in link.on_line(&raw.to_string()) {
                if event["type"] == "request.opened" {
                    response = Some(
                        json!({"v":1,"type":"request.respond","requestId":event["requestId"],
                        "response":case.response(&event)}),
                    );
                }
                if event["type"] == "request.closed" {
                    assert_eq!(event["outcome"], "resolved");
                    resolved = true;
                }
            }
        }
        if question_visible {
            if let Some(answer) = response.take() {
                link.write(&answer).unwrap();
                answered = true;
                assert!(
                    link.write(&answer).is_err(),
                    "a consumed decision must reject another answer"
                );
            }
        }
        assert!(engine.running() && terminal.control.running());
    }
    assert!(sent && question_visible && request.is_some() && answered && native_resolved && resolved && completed,
        "native decision did not complete: sent={sent}, visible={question_visible}, request={}, answered={answered}, native_resolved={native_resolved}, resolved={resolved}, completed={completed}", request.is_some());
    assert_eq!(final_text, Some(json!(case.final_text())));
    if matches!(case, Case::Command) {
        assert!(command_completed);
        let marker = cwd.join("native-approval.txt");
        assert!(std::fs::symlink_metadata(&marker)
            .unwrap()
            .file_type()
            .is_file());
        assert_eq!(
            std::fs::read(&marker).unwrap(),
            b"PROMETEU_TERMINAL_TOOL_SMOKE"
        );
    }
    terminal.close();
    assert!(!terminal.control.running());
    stream.dispose(&mut screen).unwrap();
    screen.shutdown().unwrap();
    engine.stop();
    assert!(!engine.running());
    if std::env::var_os("PROMETEU_CAPTURE_NATIVE_ENGINE").is_some() {
        crate::paths::write_private(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join(if case.plan() {"src/terminal_agent_codex/fixtures/question-answer.json"} else {"src/terminal_agent_codex/fixtures/command-approval.json"}),
            &serde_json::to_string_pretty(&json!({"version":VERIFIED_VERSION,
                "nativeDecisionVisible":true,"answeredThroughAttachedAdapter":true,"nativeResolutionObserved":true,
                "repeatedAnswerRejected":true,"oneNativeDecision":true,"finalText":case.final_text(),
                "nativeCommandCompleted":command_completed,"exactScratchFileWritten":command_completed,
                "engineAndTuiReaped":true})).unwrap(),
        ).unwrap();
    }
}
