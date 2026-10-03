#![cfg(unix)]
use prometeu_bridge::{
    ResidentWslLauncher, RuntimeConnector, RuntimeEvents, RuntimeLauncher, StdioConnector, Target,
};
use serde_json::Value;
use std::{
    io::{BufRead, Write},
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

struct Local;
impl RuntimeLauncher for Local {
    fn launch(&self, target: &Target) -> Result<Child, String> {
        Command::new(&target.executable)
            .args([
                "--transport",
                "resident",
                "--root",
                &target.root,
                "--workdir",
                &target.workdir,
                "--codex",
                &target.codex,
            ])
            .env(
                "XDG_DATA_HOME",
                std::path::Path::new(&target.root).join("test-desktop-data"),
            )
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())
    }
}
struct Events(Mutex<mpsc::Sender<Value>>);
impl RuntimeEvents for Events {
    fn publish(&self, frame: Value) {
        let _ = self.0.lock().unwrap().send(frame);
    }
}
fn events() -> (Arc<dyn RuntimeEvents>, mpsc::Receiver<Value>) {
    let (tx, rx) = mpsc::channel();
    (Arc::new(Events(Mutex::new(tx))), rx)
}
fn until(rx: &mpsc::Receiver<Value>, kind: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if frame["event"]["type"] == kind {
            return;
        }
    }
}
struct Cleanup {
    armed: bool,
    target: Target,
    launcher: Arc<dyn RuntimeLauncher>,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut client) = (StdioConnector {
            launcher: self.launcher.clone(),
        })
        .connect(&self.target, events().0)
        {
            let _ = client.shutdown();
        }
    }
}
fn roundtrip(launcher: Arc<dyn RuntimeLauncher>, distribution: String) {
    let base = std::env::temp_dir().join(format!("pr-{}", uuid::Uuid::new_v4()));
    let workdir = base.join("project with spaces");
    std::fs::create_dir_all(&workdir).unwrap();
    let codex = base.join("fake codex");
    let script = include_str!("fake_codex.py")
        .replace("import json", "import pathlib, time\npathlib.Path('launches').open('a').write('launch\\n')\nimport json")
        .replace("        if text == \"hold\":", "        if text == 'detached turn':\n            while not pathlib.Path('release').exists(): time.sleep(0.02)\n        if text == \"hold\":");
    std::fs::write(&codex, script).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution,
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_string_lossy().into(),
        workdir: workdir.to_string_lossy().into(),
        codex: codex.to_string_lossy().into(),
    };
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: launcher.clone(),
    };
    let connector = StdioConnector { launcher };
    let (sink, rx) = events();
    let mut client = connector.connect(&target, sink).unwrap();
    assert!(client.terminal_current().unwrap().is_none());
    let generation = client.start().unwrap().generation;
    until(&rx, "session.identity");
    let terminal = client.terminal_open(80, 24).unwrap();
    client.terminal_write(terminal.id.clone(), b"stty -echo; export REATTACH_TOKEN=preserved; while [ ! -f release ]; do sleep 0.02; done; head -c 1048576 /dev/zero; printf 'detached-%s\\n' shell\r".to_vec()).unwrap();
    let error = connector
        .connect(&target, events().0)
        .err()
        .expect("second attachment must fail");
    assert!(error.contains("already has an attached client"), "{error}");
    assert_eq!(
        client.snapshot().unwrap().generation.as_deref(),
        Some(generation.as_str())
    );
    client.send("detached turn".into()).unwrap();
    until(&rx, "assistant.block");
    // Simulate window loss without a graceful request; this must preserve both children.
    drop(client);
    assert!(
        prometeu_runtime::store::Store::open(std::path::Path::new(&target.root), &workdir).is_err()
    );
    std::fs::write(workdir.join("release"), "release").unwrap();
    // No client supplies terminal credits while the detached output exceeds the credit window.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_to_string(base.join("root/transcript.jsonl"))
        .unwrap()
        .contains("turn.completed")
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut wrong = target.clone();
    wrong.codex.push_str("-other");
    let error = connector
        .connect(&wrong, events().0)
        .err()
        .expect("configuration mismatch must fail");
    assert!(error.contains("configuration does not match"), "{error}");
    let (sink, _) = events();
    let mut attached = connector.connect(&target, sink).unwrap();
    let snapshot = attached.snapshot().unwrap();
    assert_eq!(snapshot.generation.as_deref(), Some(generation.as_str()));
    assert_eq!(snapshot.ready, Some(true));
    assert!(snapshot.snapshot.text.contains("detached turn"));
    assert_eq!(
        std::fs::read_to_string(workdir.join("launches")).unwrap(),
        "launch\n"
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = attached.terminal_current().unwrap().unwrap();
        assert_eq!(current.id, terminal.id);
        attached
            .terminal_acknowledge(current.id, current.seq)
            .unwrap();
        if String::from_utf8_lossy(&current.data).contains("detached-shell") {
            assert!(current.seq > 64);
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    attached
        .terminal_write(
            terminal.id.clone(),
            b"printf 'token=%s\\n' \"$REATTACH_TOKEN\"\r".to_vec(),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let current = attached.terminal_current().unwrap().unwrap();
        if String::from_utf8_lossy(&current.data).contains("token=preserved") {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    attached.disconnect().unwrap();
    drop(attached);
    // A malformed attachment and an abruptly killed proxy must not stop the host.
    for malformed in [true, false] {
        let mut proxy = connector.launcher.launch(&target).unwrap();
        let mut hello = String::new();
        std::io::BufReader::new(proxy.stdout.take().unwrap())
            .read_line(&mut hello)
            .unwrap();
        assert!(hello.contains("resident.v1"));
        if malformed {
            proxy.stdin.as_mut().unwrap().write_all(b"\xff\n").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while proxy.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
        } else {
            proxy.kill().unwrap();
            proxy.wait().unwrap();
        }
    }
    let mut last = connector.connect(&target, events().0).unwrap();
    assert_eq!(
        last.snapshot().unwrap().generation.as_deref(),
        Some(generation.as_str())
    );
    last.shutdown().unwrap();
    drop(last);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if prometeu_runtime::store::Store::open(std::path::Path::new(&target.root), &workdir)
            .is_ok()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    // Cleanup only needs to reconnect if an assertion unwinds before explicit shutdown.
    cleanup.armed = false;
    assert!(!base.join("root/resident.sock").exists());
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn resident_keeps_turn_and_shell_alive_across_lost_and_explicit_attachments() {
    roundtrip(Arc::new(Local), "Test".into());
}

/// Inspect the actual resident handshake without the bridge's upgrade policy.
fn executable(target: &Target) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut proxy = Local.launch(target).unwrap();
        let mut hello = String::new();
        std::io::BufReader::new(proxy.stdout.take().unwrap())
            .read_line(&mut hello)
            .unwrap();
        proxy.stdin.take();
        proxy.wait().unwrap();
        let hello: Value = serde_json::from_str(&hello).unwrap();
        if let Some(path) = hello["executable"].as_str() {
            return path.into();
        }
        assert!(Instant::now() < deadline, "{hello}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn replacement_waits_for_retained_conversations_and_shells_then_upgrades_an_idle_host() {
    let base = std::env::temp_dir().join(format!("pu-{}", uuid::Uuid::new_v4()));
    let workdir = base.join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let codex = base.join("provider");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let first = base.join("runtime-first");
    let next = base.join("runtime-next");
    for path in [&first, &next] {
        std::fs::copy(env!("CARGO_BIN_EXE_prometeu-runtime"), path).unwrap();
    }
    let mut target = Target {
        distribution: "Test".into(),
        executable: first.to_str().unwrap().into(),
        root: base.join("root").to_str().unwrap().into(),
        workdir: workdir.to_str().unwrap().into(),
        codex: codex.to_str().unwrap().into(),
    };
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: Arc::new(Local),
    };
    let connector = StdioConnector {
        launcher: Arc::new(Local),
    };
    let (sink, rx) = events();
    let mut client = connector.connect(&target, sink).unwrap();
    assert!(client.connected());
    let generation = client.start().unwrap().generation;
    until(&rx, "session.identity");
    client.disconnect().unwrap();
    assert!(!client.connected());
    target.executable = next.to_str().unwrap().into();
    let mut client = connector.connect(&target, events().0).unwrap();
    assert_eq!(
        client.snapshot().unwrap().generation.as_deref(),
        Some(generation.as_str())
    );
    client.disconnect().unwrap();
    assert_eq!(executable(&target), first.to_str().unwrap());
    let mut client = connector.connect(&target, events().0).unwrap();
    client.stop().unwrap();
    let terminal = client.terminal_open(80, 24).unwrap();
    client.disconnect().unwrap();
    let mut client = connector.connect(&target, events().0).unwrap();
    assert_eq!(client.terminal_current().unwrap().unwrap().id, terminal.id);
    client.disconnect().unwrap();
    assert_eq!(executable(&target), first.to_str().unwrap());
    let mut client = connector.connect(&target, events().0).unwrap();
    client.terminal_close(terminal.id).unwrap();
    // An application dock also owns a live shell independently of the selected context.
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    let workspace = board["workspaces"][0]["id"].as_str().unwrap();
    client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":workspace,"kind":"terminal","cols":80,"rows":24}),
        )
        .unwrap();
    client.disconnect().unwrap();
    let mut client = connector.connect(&target, events().0).unwrap();
    let snapshot = client
        .application(
            "pty_buffer".into(),
            serde_json::json!({"session":format!("{workspace}:terminal"),"snapshot":true}),
        )
        .unwrap();
    assert_eq!(snapshot["running"], true);
    assert!(snapshot["seq"].is_u64());
    client.disconnect().unwrap();
    assert_eq!(executable(&target), first.to_str().unwrap());
    let mut client = connector.connect(&target, events().0).unwrap();
    client
        .application(
            "close_dock".into(),
            serde_json::json!({"id":workspace,"kind":"terminal"}),
        )
        .unwrap();
    client.disconnect().unwrap();
    let mut client = connector.connect(&target, events().0).unwrap();
    assert_eq!(
        client
            .application("load_board".into(), Value::Null)
            .unwrap()["workspaces"][0]["id"],
        workspace
    );
    assert!(!client.snapshot().unwrap().running.unwrap());
    client.disconnect().unwrap();
    assert_eq!(executable(&target), next.to_str().unwrap());
    let mut client = connector.connect(&target, events().0).unwrap();
    assert!(client
        .application(
            "pty_buffer".into(),
            serde_json::json!({"session":format!("{workspace}:terminal"),"snapshot":true})
        )
        .unwrap()
        .is_null());
    client.shutdown().unwrap();
    cleanup.armed = false;
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
#[ignore = "requires explicit WSL distribution containing this checkout"]
fn actual_wsl_resident_reconnect() {
    roundtrip(
        Arc::new(ResidentWslLauncher),
        std::env::var("PROMETEU_TEST_WSL_DISTRIBUTION").unwrap(),
    );
}

#[test]
fn workspace_switching_isolates_execution_and_restores_catalog_after_host_restart() {
    let base = std::env::temp_dir().join(format!("pw-{}", uuid::Uuid::new_v4()));
    let first = base.join("first project");
    let second = base.join("second ' unicode λ");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let codex = base.join("provider");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution: "Test".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_str().unwrap().into(),
        workdir: first.to_str().unwrap().into(),
        codex: codex.to_str().unwrap().into(),
    };
    // A pre-catalog v1 root must retain its original transcript and provider identity.
    let original = "{\"v\":1,\"type\":\"user.message\",\"at\":1,\"content\":[{\"kind\":\"text\",\"text\":\"Existing history\"}]}\n";
    {
        let store = prometeu_runtime::store::Store::open(&base.join("root"), &first).unwrap();
        store.identity("fixture-thread").unwrap();
        std::fs::write(base.join("root/transcript.jsonl"), original).unwrap();
    }
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: Arc::new(Local),
    };
    let connector = StdioConnector {
        launcher: Arc::new(Local),
    };
    let (sink, rx) = events();
    let mut client = connector.connect(&target, sink).unwrap();
    let catalog = client.workspace_list().unwrap();
    assert_eq!(catalog.active, "primary");
    assert_eq!(
        std::fs::read_to_string(base.join("root/transcript.jsonl")).unwrap(),
        original
    );
    assert_eq!(
        client.snapshot().unwrap().provider_session.as_deref(),
        Some("fixture-thread")
    );
    assert!(client
        .workspace_create(
            "Missing".into(),
            base.join("missing").to_str().unwrap().into()
        )
        .is_err());
    assert_eq!(client.workspace_list().unwrap().board.workspaces.len(), 1);
    let catalog = client
        .workspace_create("Secondary".into(), second.to_str().unwrap().into())
        .unwrap();
    let secondary = catalog.board.workspaces[1].id.clone();
    for args in [
        vec!["init", "-b", "main"],
        vec![
            "-c",
            "user.name=Test",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    ] {
        let output = Command::new("git")
            .arg("-C")
            .arg(&first)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let catalog = client
        .workspace_worktree(prometeu_bridge::workspaces::WorktreeRequest {
            title: "Isolated checkout".into(),
            path: first.to_str().unwrap().into(),
            branch: "feature/runtime".into(),
            base: "HEAD".into(),
        })
        .unwrap();
    let isolated = catalog.board.workspaces[2].clone();
    assert_ne!(isolated.worktree, isolated.repo);
    assert_eq!(catalog.active, "primary");
    client.workspace_select(isolated.id.clone()).unwrap();
    client.start().unwrap();
    until(&rx, "session.identity");
    client.send("Inside the isolated checkout".into()).unwrap();
    until(&rx, "turn.completed");
    client.workspace_select("primary".into()).unwrap();

    let first_generation = client.start().unwrap().generation;
    until(&rx, "session.identity");
    let first_terminal = client.terminal_open(80, 24).unwrap();
    client.send("hold".into()).unwrap();
    until(&rx, "assistant.block");
    client.workspace_select(secondary.clone()).unwrap();
    assert_eq!(client.snapshot().unwrap().running, Some(false));
    assert!(client.terminal_current().unwrap().is_none());
    assert!(client
        .terminal_write(first_terminal.id.clone(), b"exit\r".to_vec())
        .is_err());
    let second_generation = client.start().unwrap().generation;
    assert_ne!(first_generation, second_generation);
    until(&rx, "session.identity");
    client.send("Only in the second workspace".into()).unwrap();
    until(&rx, "turn.completed");
    let second_terminal = client.terminal_open(80, 24).unwrap();
    assert_ne!(first_terminal.id, second_terminal.id);
    assert!(client.workspace_select("../outside".into()).is_err());
    assert_eq!(
        client.snapshot().unwrap().generation.as_deref(),
        Some(second_generation.as_str())
    );
    client
        .workspace_stage("primary".into(), "Feito".into())
        .unwrap();
    client.workspace_select("primary".into()).unwrap();
    let first_snapshot = client.snapshot().unwrap();
    assert_eq!(
        first_snapshot.generation.as_deref(),
        Some(first_generation.as_str())
    );
    assert!(!first_snapshot
        .snapshot
        .text
        .contains("Only in the second workspace"));
    assert_eq!(
        client.terminal_current().unwrap().unwrap().id,
        first_terminal.id
    );
    client.workspace_select(secondary.clone()).unwrap();
    client.disconnect().unwrap();
    drop(client);
    let mut client = connector.connect(&target, events().0).unwrap();
    assert_eq!(client.workspace_list().unwrap().active, secondary);
    assert_eq!(
        client.snapshot().unwrap().generation.as_deref(),
        Some(second_generation.as_str())
    );
    assert_eq!(
        client.terminal_current().unwrap().unwrap().id,
        second_terminal.id
    );
    client.shutdown().unwrap();
    drop(client);
    let (sink, rx) = events();
    let mut restarted = connector.connect(&target, sink).unwrap();
    assert_eq!(
        restarted.workspace_list().unwrap().board.workspaces[0].stage,
        "Feito"
    );
    let restored = restarted.workspace_list().unwrap();
    assert_eq!(restored.board.workspaces[2].branch, "feature/runtime");
    assert_eq!(restored.board.workspaces[2].worktree, isolated.worktree);
    let snapshot = restarted.snapshot().unwrap();
    assert_eq!(snapshot.running, Some(false));
    assert!(snapshot
        .snapshot
        .text
        .contains("Only in the second workspace"));
    assert!(restarted.terminal_current().unwrap().is_none());
    assert!(restarted.start().unwrap().resuming);
    until(&rx, "session.identity");
    restarted.shutdown().unwrap();
    drop(restarted);
    cleanup.armed = false;
    assert!(prometeu_runtime::store::Store::open(&base.join("root"), &first).is_ok());
    assert!(prometeu_runtime::store::Store::open(
        &base.join("root/workspaces").join(secondary),
        &second
    )
    .is_ok());
    assert_eq!(
        std::fs::metadata(base.join("root/workspaces.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn application_commands_and_events_address_sessions_without_changing_selection() {
    let base = std::env::temp_dir().join(format!("pa-{}", uuid::Uuid::new_v4()));
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(["-C", project.to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Runtime test"]);
    git(&["config", "user.email", "runtime@example.test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.hooksPath", "no-hooks"]);
    std::fs::write(project.join("notes.txt"), "Original").unwrap();
    git(&["add", "notes.txt"]);
    git(&["commit", "-qm", "initial"]);
    let codex = base.join("provider");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution: "Test".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_str().unwrap().into(),
        workdir: project.to_str().unwrap().into(),
        codex: codex.to_str().unwrap().into(),
    };
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: Arc::new(Local),
    };
    let (sink, rx) = events();
    let mut client = StdioConnector {
        launcher: Arc::new(Local),
    }
    .connect(&target, sink)
    .unwrap();
    let agents = client.application("agents".into(), Value::Null).unwrap();
    assert_eq!(agents["providers"][0]["id"], "codex");
    assert_eq!(agents["providers"][0]["installed"], true);
    let accounts = client.application("accounts".into(), Value::Null).unwrap();
    assert_eq!(accounts["accounts"][0]["email"], "fixture@example.test");
    let models = client
        .application("agent_models".into(), serde_json::json!({"agent":"codex"}))
        .unwrap();
    assert_eq!(models["models"][0]["id"], "fixture");
    let catalog = client
        .workspace_create("Background".into(), target.workdir.clone())
        .unwrap();
    let second = catalog.board.workspaces[1].id.clone();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["workspaces"][1]["id"], second);
    client
        .application(
            "chat_send".into(),
            serde_json::json!({"session":second,"text":"Addressed background conversation"}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if event["application"]["name"] == "chat" {
            let payload = &event["application"]["payload"];
            assert_eq!(payload[0], second);
            let line: Value = serde_json::from_str(payload[1].as_str().unwrap()).unwrap();
            if line["type"] == "turn.completed" {
                break;
            }
        }
    }
    assert_eq!(client.workspace_list().unwrap().active, "primary");
    let primary = client
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":"primary"}),
        )
        .unwrap();
    assert!(!primary["text"]
        .as_str()
        .unwrap()
        .contains("Addressed background"));
    let second_snapshot = client
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":second}),
        )
        .unwrap();
    assert!(second_snapshot["text"]
        .as_str()
        .unwrap()
        .contains("Addressed background"));
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["workspaces"][1]["tabs"][0]["status"], "pronta");
    assert!(client
        .application(
            "chat_send".into(),
            serde_json::json!({"session":"../outside","text":"Wrong"})
        )
        .is_err());
    let sibling = client
        .application(
            "new_tab".into(),
            serde_json::json!({"workspace":second,"prompt":"Sibling conversation","choice":null}),
        )
        .unwrap();
    let sibling_id = sibling["id"].as_str().unwrap().to_owned();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if frame["application"]["name"] == "chat"
            && frame["application"]["payload"][0] == sibling_id
        {
            let line: Value =
                serde_json::from_str(frame["application"]["payload"][1].as_str().unwrap()).unwrap();
            if line["type"] == "turn.completed" {
                break;
            }
        }
    }
    let sibling_snapshot = client
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":sibling_id}),
        )
        .unwrap();
    assert!(sibling_snapshot["text"]
        .as_str()
        .unwrap()
        .contains("Sibling conversation"));
    assert!(!sibling_snapshot["text"]
        .as_str()
        .unwrap()
        .contains("Addressed background"));
    client
        .application(
            "chat_send".into(),
            serde_json::json!({"session":second,"text":"hold"}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let observed = client
            .application("load_board".into(), Value::Null)
            .unwrap();
        if observed["workspaces"][1]["tabs"][0]["status"] == "rodando" {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    for operation in ["pull", "discard"] {
        assert!(client.application("workspace_git_action".into(), serde_json::json!({"id":second,"repo":0,"operation":operation,"paths":["notes.txt"]})).unwrap_err().contains("err.git.agent"));
    }
    client
        .application(
            "close_tab".into(),
            serde_json::json!({"workspace":second,"tab":second}),
        )
        .unwrap();
    std::fs::write(project.join("notes.txt"), "Original").unwrap();
    let read = |client: &mut Box<dyn prometeu_bridge::RuntimeClient>| {
        client.application(
            "read_file".into(),
            serde_json::json!({"id":second,"rel":"notes.txt"}),
        )
    };
    assert_eq!(read(&mut client).unwrap(), "Original");
    client.application("write_file".into(), serde_json::json!({"id":second,"rel":"notes.txt","text":"Edited in Windows","was":"Original"})).unwrap();
    assert!(client
        .application(
            "write_file".into(),
            serde_json::json!({"id":second,"rel":"notes.txt","text":"Stale edit","was":"Original"})
        )
        .unwrap_err()
        .contains("err.session.changed"));
    assert_eq!(read(&mut client).unwrap(), "Edited in Windows");
    let marks = client
        .application("tree_git_status".into(), serde_json::json!({"id":second}))
        .unwrap();
    assert!(marks
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == "notes.txt" && f["status"] == "M"));
    let changes = client
        .application(
            "workspace_git_diff".into(),
            serde_json::json!({"id":second,"repo":0,"scope":"changes"}),
        )
        .unwrap();
    assert!(changes["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("+Edited in Windows"));
    client
        .application(
            "workspace_git_action".into(),
            serde_json::json!({"id":second,"repo":0,"operation":"stage","paths":["notes.txt"]}),
        )
        .unwrap();
    let staged = client
        .application(
            "workspace_git_status".into(),
            serde_json::json!({"id":second}),
        )
        .unwrap();
    assert_eq!(staged[0]["staged"][0]["path"], "notes.txt");
    assert!(client.application("workspace_git_action".into(), serde_json::json!({"id":second,"repo":0,"operation":"commit","paths":[],"message":"Wrong snapshot","expected":"stale"})).unwrap_err().contains("err.git.changed"));
    client.application("write_file".into(), serde_json::json!({"id":second,"rel":"notes.txt","text":"Later worktree edit","was":"Edited in Windows"})).unwrap();
    client.application("workspace_git_action".into(), serde_json::json!({"id":second,"repo":0,"operation":"commit","paths":[],"message":"Native reviewed content","expected":staged[0]["index"]})).unwrap();
    assert_eq!(git(&["show", "HEAD:notes.txt"]), "Edited in Windows");
    assert_eq!(
        std::fs::read_to_string(project.join("notes.txt")).unwrap(),
        "Later worktree edit"
    );
    let history = client
        .application(
            "workspace_git_history".into(),
            serde_json::json!({"id":second,"repo":0}),
        )
        .unwrap();
    assert_eq!(history[0]["subject"], "Native reviewed content");
    let branches = client
        .application(
            "workspace_git_branches".into(),
            serde_json::json!({"id":second,"repo":0}),
        )
        .unwrap();
    assert!(branches
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b["name"] == "main" && b["current"] == true));
    assert!(client
        .application(
            "workspace_git_diff".into(),
            serde_json::json!({"id":second,"repo":99,"scope":"changes"})
        )
        .is_err());
    assert!(client
        .application(
            "workspace_git_action".into(),
            serde_json::json!({"id":second,"repo":0,"operation":"stage","paths":["../outside.txt"]})
        )
        .unwrap_err()
        .contains("err.session.outside"));
    std::fs::remove_file(project.join("notes.txt")).unwrap();
    client
        .application(
            "tree_restore".into(),
            serde_json::json!({"id":second,"rel":"notes.txt"}),
        )
        .unwrap();
    assert_eq!(read(&mut client).unwrap(), "Edited in Windows");
    std::fs::write(project.join("escaped.txt"), vec![0; 2 * 1024 * 1024]).unwrap();
    assert_eq!(
        client
            .application(
                "read_file".into(),
                serde_json::json!({"id":second,"rel":"escaped.txt"})
            )
            .unwrap_err(),
        "response exceeds 8 MiB"
    );
    assert_eq!(read(&mut client).unwrap(), "Edited in Windows");
    std::fs::write(base.join("outside.txt"), "Private").unwrap();
    std::os::unix::fs::symlink(base.join("outside.txt"), project.join("link.txt")).unwrap();
    for rel in ["../outside.txt", "link.txt"] {
        assert!(client
            .application(
                "read_file".into(),
                serde_json::json!({"id":second,"rel":rel})
            )
            .unwrap_err()
            .contains("err.session.outside"));
    }
    std::fs::create_dir_all(project.join(".prometeu")).unwrap();
    std::fs::write(
        project.join(".prometeu/settings.toml"),
        "[scripts]\nrun = \"echo declared script\"\n",
    )
    .unwrap();
    let scripts = client
        .application("workspace_scripts".into(), serde_json::json!({"id":second}))
        .unwrap();
    assert_eq!(scripts["runs"][0]["command"], "echo declared script");
    let first_shell = client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":second,"kind":"terminal","cols":80,"rows":24}),
        )
        .unwrap();
    let second_shell = client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":second,"kind":"terminal-2","cols":80,"rows":24}),
        )
        .unwrap();
    assert_ne!(first_shell, second_shell);
    client
        .application(
            "pty_write".into(),
            serde_json::json!({"session":first_shell,"data":"printf 'FIRST_%s\\n' SHELL; pwd\n"}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let buffer = client
            .application(
                "pty_buffer".into(),
                serde_json::json!({"session":first_shell}),
            )
            .unwrap();
        let bytes: Vec<u8> = serde_json::from_value(buffer).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        if text.contains("FIRST_SHELL") && text.contains(target.workdir.as_str()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "shell did not produce output: {text}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let buffer = client
        .application(
            "pty_buffer".into(),
            serde_json::json!({"session":second_shell}),
        )
        .unwrap();
    let bytes: Vec<u8> = serde_json::from_value(buffer).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("FIRST_SHELL"));
    client
        .application(
            "close_dock".into(),
            serde_json::json!({"id":second,"kind":"terminal"}),
        )
        .unwrap();
    let docks = client
        .application("dock_state".into(), serde_json::json!({"id":second}))
        .unwrap();
    assert_eq!(docks.as_array().unwrap().len(), 2);
    assert!(docks
        .as_array()
        .unwrap()
        .iter()
        .any(|dock| dock["kind"] == "terminal" && dock["alive"] == false));
    std::fs::write(project.join(".prometeu/settings.toml"), r#"[scripts]
setup = "while [ ! -f setup-release ]; do sleep 0.02; done; printf 'setup-finished\n'; exit 7"
[scripts.run]
[scripts.run.server]
default = true
command = "printf 'RUN:%s:%s:%s\n' \"$PROMETEU_PORT\" \"$PORT\" \"$PROMETEU_WORKSPACE_PATH\"; sleep 60"
[scripts.run.other]
command = "echo other"
"#).unwrap();
    client
        .application("set_lang".into(), serde_json::json!({"lang":"en"}))
        .unwrap();
    let creation = Instant::now();
    let created = client.application("create_workspace".into(), serde_json::json!({"cols":80,"rows":24,"draft":{
        "project":target.workdir,"branch":"","base":"","worktree":false,"newBranch":false,
        "title":"Original launcher","stage":"Preparando","prompt":"Initial launcher prompt","inject":["/outside/attached.txt"],
        "agent":"codex","model":"fixture","effort":"high"
    }})).unwrap();
    assert_eq!(created["model"], "fixture");
    assert_eq!(created["effort"], "high");
    assert_eq!(created["failed"], Value::Null);
    assert!(creation.elapsed() < Duration::from_secs(3));
    let initial = client
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":created["tabs"][0]["id"]}),
        )
        .unwrap();
    assert!(!initial["text"]
        .as_str()
        .unwrap()
        .contains("Initial launcher prompt"));
    assert!(client
        .application(
            "chat_send".into(),
            serde_json::json!({"session":created["tabs"][0]["id"],"text":"Too early"})
        )
        .unwrap_err()
        .contains("err.windows.preparing"));
    assert!(client.application("chat_control".into(), serde_json::json!({"session":created["tabs"][0]["id"],"frame":{"v":1,"type":"message.send","text":"Too early"}})).unwrap_err().contains("err.windows.preparing"));
    // Setup and first-input delivery continue without a window or incoming requests.
    drop(client);
    std::fs::write(project.join("setup-release"), "ready").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut client = StdioConnector {
        launcher: Arc::new(Local),
    }
    .connect(&target, events().0)
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let initial = client
            .application(
                "chat_snapshot".into(),
                serde_json::json!({"session":created["tabs"][0]["id"]}),
            )
            .unwrap();
        if initial["text"]
            .as_str()
            .unwrap()
            .contains("Initial launcher prompt")
        {
            assert!(initial["text"]
                .as_str()
                .unwrap()
                .contains("setup exited with code 7"));
            assert!(initial["text"]
                .as_str()
                .unwrap()
                .contains("@/outside/attached.txt"));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let run = client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":created["id"],"kind":"run","cols":80,"rows":24}),
        )
        .unwrap();
    assert_eq!(
        client
            .application(
                "open_dock".into(),
                serde_json::json!({"id":created["id"],"kind":"run","cols":80,"rows":24})
            )
            .unwrap(),
        run
    );
    assert!(client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":created["id"],"kind":"run","name":"other","cols":80,"rows":24})
        )
        .unwrap_err()
        .contains("err.dock.running"));
    loop {
        let bytes: Vec<u8> = serde_json::from_value(
            client
                .application("pty_buffer".into(), serde_json::json!({"session":run}))
                .unwrap(),
        )
        .unwrap();
        if String::from_utf8_lossy(&bytes).contains("RUN:") {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    client
        .application(
            "close_dock".into(),
            serde_json::json!({"id":created["id"],"kind":"run"}),
        )
        .unwrap();
    let bytes: Vec<u8> = serde_json::from_value(
        client
            .application("pty_buffer".into(), serde_json::json!({"session":run}))
            .unwrap(),
    )
    .unwrap();
    let port = created["port"].as_u64().unwrap();
    assert!(
        String::from_utf8_lossy(&bytes).contains(&format!("RUN:{port}:{port}:{}", target.workdir))
    );
    let removed = client
        .application("account_remove".into(), serde_json::json!({"id":"codex"}))
        .unwrap();
    assert!(removed["active"]["codex"].is_null());
    assert!(client
        .application(
            "chat_send".into(),
            serde_json::json!({"session":sibling_id,"text":"Do not send without selection"})
        )
        .unwrap_err()
        .contains("err.account.noActive"));
    client.workspace_select(second.clone()).unwrap();
    client.shutdown().unwrap();
    drop(client);
    let mut restarted = StdioConnector {
        launcher: Arc::new(Local),
    }
    .connect(&target, events().0)
    .unwrap();
    let board = restarted
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["workspaces"][1]["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(board["workspaces"][1]["active"], sibling_id);
    let restored = restarted
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":sibling_id}),
        )
        .unwrap();
    assert!(restored["text"]
        .as_str()
        .unwrap()
        .contains("Sibling conversation"));
    assert!(restarted
        .application(
            "chat_snapshot".into(),
            serde_json::json!({"session":second})
        )
        .is_err());
    let no_account = restarted
        .application("agent_models".into(), serde_json::json!({"agent":"codex"}))
        .unwrap_err();
    assert!(no_account.contains("err.modelsCatalog.noAccount"));
    let attached = restarted
        .application(
            "account_login".into(),
            serde_json::json!({"provider":"codex","method":"external"}),
        )
        .unwrap();
    assert!(attached["active"]["codex"].is_null());
    restarted
        .application("account_select".into(), serde_json::json!({"id":"codex"}))
        .unwrap();
    assert!(restarted
        .application("agent_models".into(), serde_json::json!({"agent":"codex"}))
        .is_ok());
    restarted.shutdown().unwrap();
    drop(restarted);
    cleanup.armed = false;
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn registered_project_files_and_shell_work_without_a_conversation() {
    let base = std::env::temp_dir().join(format!("pp-{}", uuid::Uuid::new_v4()));
    let project = base.join("source");
    let added = base.join("added ' project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&added).unwrap();
    std::fs::write(added.join("notes.txt"), "Project only").unwrap();
    let target = Target {
        distribution: "Test".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_str().unwrap().into(),
        workdir: project.to_str().unwrap().into(),
        codex: "/unused-provider".into(),
    };
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: Arc::new(Local),
    };
    let connector = StdioConnector {
        launcher: Arc::new(Local),
    };
    let mut client = connector.connect(&target, events().0).unwrap();
    let added_path = added.to_str().unwrap();
    let saved = client
        .application("add_project".into(), serde_json::json!({"path":added_path}))
        .unwrap();
    let duplicate = client
        .application(
            "add_project".into(),
            serde_json::json!({"path":format!("{added_path}/.")}),
        )
        .unwrap();
    assert_eq!(saved, duplicate);
    let id = saved["id"].as_str().unwrap();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["projects"].as_array().unwrap().len(), 2);
    assert_eq!(board["workspaces"].as_array().unwrap().len(), 1);
    assert!(client
        .application(
            "add_project".into(),
            serde_json::json!({"path":added.join("notes.txt")})
        )
        .is_err());
    let scripts = client
        .application("workspace_scripts".into(), serde_json::json!({"id":id}))
        .unwrap();
    assert!(scripts["port"].is_null());
    assert!(client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":id,"kind":"setup","cols":80,"rows":24})
        )
        .is_err());
    let file = client
        .application(
            "read_file".into(),
            serde_json::json!({"id":id,"rel":"notes.txt"}),
        )
        .unwrap();
    assert_eq!(file, "Project only");
    for (rel, dir) in [("", true), ("notes.txt", false)] {
        assert_eq!(
            client
                .application("reveal_path".into(), serde_json::json!({"id":id,"rel":rel}))
                .unwrap(),
            serde_json::json!({"path":added.join(rel).canonicalize().unwrap(),"dir":dir})
        );
    }
    assert!(client
        .application(
            "reveal_path".into(),
            serde_json::json!({"id":id,"rel":"missing"})
        )
        .is_err());
    assert!(client
        .application(
            "reveal_path".into(),
            serde_json::json!({"id":id,"rel":".."})
        )
        .is_err());
    client.application("write_file".into(), serde_json::json!({"id":id,"rel":"notes.txt","text":"Project edit","was":"Project only"})).unwrap();
    assert_eq!(
        std::fs::read_to_string(added.join("notes.txt")).unwrap(),
        "Project edit"
    );
    for (rel, dir) in [("drafts", true), ("drafts/review.md", false)] {
        assert_eq!(
            client
                .application(
                    "create_path".into(),
                    serde_json::json!({"id":id,"rel":rel,"dir":dir})
                )
                .unwrap(),
            Value::Null
        );
    }
    client.application("write_file".into(), serde_json::json!({"id":id,"rel":"drafts/review.md","text":"Recover this draft","was":""})).unwrap();
    assert!(client
        .application(
            "create_path".into(),
            serde_json::json!({"id":id,"rel":"drafts/review.md","dir":false})
        )
        .is_err());
    assert!(client
        .application(
            "rename_path".into(),
            serde_json::json!({"id":id,"from":"drafts/review.md","to":"notes.txt"})
        )
        .is_err());
    client
        .application(
            "rename_path".into(),
            serde_json::json!({"id":id,"from":"drafts","to":"reviews"}),
        )
        .unwrap();
    client
        .application(
            "rename_path".into(),
            serde_json::json!({"id":id,"from":"reviews/review.md","to":"reviews/REVIEW.md"}),
        )
        .unwrap();
    let found = client
        .application(
            "find_paths".into(),
            serde_json::json!({"id":id,"query":"review","recent":[],"files":true}),
        )
        .unwrap();
    assert_eq!(
        found,
        serde_json::json!([{"name":"REVIEW.md","path":"reviews/REVIEW.md","dir":false}])
    );
    let all = client
        .application(
            "find_paths".into(),
            serde_json::json!({"id":id,"query":"review","recent":[]}),
        )
        .unwrap();
    assert!(all
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["path"] == "reviews" && entry["dir"] == true));
    assert_eq!(
        client
            .application(
                "find_paths".into(),
                serde_json::json!({"id":"missing","query":"review","recent":[]})
            )
            .unwrap(),
        serde_json::json!([])
    );
    let outside = base.join("outside.txt");
    std::fs::write(&outside, "Outside stays").unwrap();
    std::os::unix::fs::symlink(&outside, added.join("outside-link")).unwrap();
    assert!(client
        .application(
            "reveal_path".into(),
            serde_json::json!({"id":id,"rel":"outside-link"})
        )
        .is_err());
    client
        .application(
            "trash_path".into(),
            serde_json::json!({"id":id,"rel":"outside-link"}),
        )
        .unwrap();
    assert!(added.join("outside-link").symlink_metadata().is_err());
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "Outside stays");
    client
        .application(
            "trash_path".into(),
            serde_json::json!({"id":id,"rel":"reviews"}),
        )
        .unwrap();
    assert!(!added.join("reviews").exists());
    let trash = base.join("root/test-desktop-data/Trash");
    assert_eq!(
        std::fs::read_to_string(trash.join("files/reviews/REVIEW.md")).unwrap(),
        "Recover this draft"
    );
    assert!(
        std::fs::read_to_string(trash.join("info/reviews.trashinfo"))
            .unwrap()
            .contains("reviews")
    );
    assert_eq!(
        std::fs::read_link(trash.join("files/outside-link")).unwrap(),
        outside
    );
    for (command, args) in [
        (
            "create_path",
            serde_json::json!({"id":id,"rel":"../escape","dir":false}),
        ),
        (
            "rename_path",
            serde_json::json!({"id":id,"from":"notes.txt","to":".git"}),
        ),
        ("trash_path", serde_json::json!({"id":id,"rel":""})),
        (
            "trash_path",
            serde_json::json!({"id":"missing","rel":"notes.txt"}),
        ),
    ] {
        assert!(client.application(command.into(), args).is_err());
    }
    assert_eq!(
        std::fs::read_to_string(added.join("notes.txt")).unwrap(),
        "Project edit"
    );
    struct NoWindow;
    impl prometeu_oauth::Consent for NoWindow {
        fn authorize(&self, _: &prometeu_oauth::ConsentRequest) -> Result<String, String> {
            panic!("Binary reads do not authorize")
        }
    }
    impl prometeu_bridge::application::AttachmentClipboard for NoWindow {
        fn files(&self) -> Result<Vec<String>, String> {
            panic!("binary reads never read the clipboard");
        }
    }
    impl prometeu_bridge::paths::ApplicationPaths for NoWindow {
        fn linux(&self, _: &Target, _: &str) -> Result<String, String> {
            panic!("binary reads stay in WSL");
        }
        fn windows(&self, _: &Target, _: &str) -> Result<String, String> {
            panic!("binary reads stay in WSL");
        }
    }
    impl prometeu_bridge::application::FileManager for NoWindow {
        fn reveal(&self, _: &str, _: bool) -> Result<(), String> {
            panic!("binary reads never open another application");
        }
    }
    let app = prometeu_bridge::application::NativeApplication {
        clipboard: Arc::new(NoWindow),
        consent: Arc::new(NoWindow),
        paths: Arc::new(NoWindow),
        files: Arc::new(NoWindow),
    };
    let bytes: Vec<_> = (0..9 * 1024 * 1024 + 3).map(|i| (i % 256) as u8).collect();
    std::fs::write(added.join("large.bin"), &bytes).unwrap();
    let received = app
        .request(
            &target,
            client.as_mut(),
            "read_bytes".into(),
            serde_json::json!({"id":id,"rel":"large.bin"}),
        )
        .unwrap();
    let prometeu_bridge::application::ApplicationResponse::Bytes(received) = received else {
        panic!("application must return binary data");
    };
    assert_eq!(received.len(), bytes.len());
    assert!(received == bytes, "binary contents changed in transit");
    let block = client
        .application(
            "read_bytes".into(),
            serde_json::json!({"id":id,"rel":"large.bin"}),
        )
        .unwrap();
    std::fs::write(added.join("large.bin"), b"changed").unwrap();
    assert!(client
        .application(
            "read_bytes".into(),
            serde_json::json!({"id":id,"rel":"large.bin","offset":262144,"stamp":block["stamp"]})
        )
        .is_err());
    for rel in ["../outside.txt", "missing", ""] {
        assert!(app
            .request(
                &target,
                client.as_mut(),
                "read_bytes".into(),
                serde_json::json!({"id":id,"rel":rel})
            )
            .is_err());
    }
    let shell = client
        .application(
            "open_dock".into(),
            serde_json::json!({"id":id,"kind":"terminal","cols":80,"rows":24}),
        )
        .unwrap();
    client
        .application(
            "pty_write".into(),
            serde_json::json!({"session":shell,"data":"printf 'PROJECT_%s\\n' SHELL; pwd\n"}),
        )
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let buffer = client
            .application("pty_buffer".into(), serde_json::json!({"session":shell}))
            .unwrap();
        let bytes: Vec<u8> = serde_json::from_value(buffer).unwrap();
        let output = String::from_utf8_lossy(&bytes);
        if output.contains("PROJECT_SHELL") && output.contains(added_path) {
            break;
        }
        assert!(Instant::now() < deadline, "{output}");
        std::thread::sleep(Duration::from_millis(20));
    }
    client
        .application(
            "close_dock".into(),
            serde_json::json!({"id":id,"kind":"terminal"}),
        )
        .unwrap();
    client
        .application(
            "remove_project".into(),
            serde_json::json!({"id":target.workdir}),
        )
        .unwrap();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["projects"].as_array().unwrap().len(), 1);
    assert_eq!(board["workspaces"].as_array().unwrap().len(), 1);
    client.shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while base.join("root/resident.sock").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = connector.connect(&target, events().0).unwrap();
    assert_eq!(
        client
            .application("load_board".into(), Value::Null)
            .unwrap()["projects"],
        board["projects"]
    );
    assert!(added.join("notes.txt").exists());
    assert!(project.is_dir());
    client.shutdown().unwrap();
    cleanup.armed = false;
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn application_resident_starts_empty_and_restores_projects_without_a_primary_workspace() {
    struct Application;
    impl RuntimeLauncher for Application {
        fn launch(&self, target: &Target) -> Result<Child, String> {
            Command::new(&target.executable)
                .args([
                    "--transport",
                    "resident",
                    "--catalog",
                    "application",
                    "--root",
                    &target.root,
                    "--workdir",
                    &target.workdir,
                    "--codex",
                    &target.codex,
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())
        }
    }
    let base = std::env::temp_dir().join(format!("pa-{}", uuid::Uuid::new_v4()));
    let home = base.join("home");
    let project = base.join("project");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let target = Target {
        distribution: "local".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("state").to_string_lossy().into(),
        workdir: home.to_string_lossy().into(),
        codex: "/unused".into(),
    };
    let launcher: Arc<dyn RuntimeLauncher> = Arc::new(Application);
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: launcher.clone(),
    };
    let connector = StdioConnector { launcher };
    let mut client = connector.connect(&target, events().0).unwrap();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["projects"], serde_json::json!([]));
    assert_eq!(board["workspaces"], serde_json::json!([]));
    client
        .application("add_project".into(), serde_json::json!({"path":project}))
        .unwrap();
    let skill = serde_json::json!({"id":"review","description":"Before a review","content":"Read the diff."});
    assert_eq!(
        client.application("skill_hub".into(), Value::Null).unwrap(),
        serde_json::json!([])
    );
    assert_eq!(
        client.application("mcp_hub".into(), Value::Null).unwrap(),
        serde_json::json!([])
    );
    assert!(client
        .application(
            "skill_save".into(),
            serde_json::json!({"skill":skill,"revision":1})
        )
        .is_err());
    assert_eq!(
        client.application("skill_hub".into(), Value::Null).unwrap(),
        serde_json::json!([])
    );
    client
        .application(
            "skill_save".into(),
            serde_json::json!({"skill":skill,"revision":null}),
        )
        .unwrap();
    assert_eq!(
        client
            .application("plugin_hub".into(), Value::Null)
            .unwrap()[0]["id"],
        "skill-review"
    );
    let sharing = client
        .application("catalog_state".into(), Value::Null)
        .unwrap();
    assert_eq!(sharing["connected"], false);
    assert_eq!(sharing["shared"], serde_json::json!({}));
    client.shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while base.join("state/resident.sock").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = connector.connect(&target, events().0).unwrap();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["projects"].as_array().unwrap().len(), 1);
    assert_eq!(
        board["projects"][0]["path"],
        project.to_string_lossy().as_ref()
    );
    assert_eq!(board["workspaces"], serde_json::json!([]));
    assert_eq!(
        client.application("skill_hub".into(), Value::Null).unwrap(),
        serde_json::json!([skill])
    );
    let mut edited = skill.clone();
    edited["content"] = serde_json::json!("Run tests after reviewing.");
    client
        .application("skill_save".into(), serde_json::json!({"skill":edited}))
        .unwrap();
    assert_eq!(
        client.application("skill_hub".into(), Value::Null).unwrap(),
        serde_json::json!([edited])
    );
    assert_eq!(
        client
            .application("skill_remove".into(), serde_json::json!({"id":"review"}))
            .unwrap(),
        serde_json::json!([])
    );
    assert_eq!(
        client
            .application("plugin_hub".into(), Value::Null)
            .unwrap(),
        serde_json::json!([])
    );
    assert!(base
        .join("state/skills-packages/review/skills/review/SKILL.md")
        .exists());
    client.shutdown().unwrap();
    cleanup.armed = false;
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn deferred_git_and_checkout_preserve_responsiveness_and_settle_after_detach() {
    use serde_json::json;
    let base = std::env::temp_dir().join(format!("pj-{}", uuid::Uuid::new_v4()));
    let project = base.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&project)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.hooksPath", "/dev/null"]);
    std::fs::write(project.join("notes"), "original").unwrap();
    git(&["add", "notes"]);
    git(&["commit", "-qm", "initial"]);
    let codex = base.join("codex");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution: "Test".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_str().unwrap().into(),
        workdir: project.to_str().unwrap().into(),
        codex: codex.to_str().unwrap().into(),
    };
    let mut cleanup = Cleanup {
        armed: true,
        target: target.clone(),
        launcher: Arc::new(Local),
    };
    let connector = StdioConnector {
        launcher: Arc::new(Local),
    };
    let mut client = connector.connect(&target, events().0).unwrap();
    assert!(client.operations_supported());
    let hooks = base.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let gate = base.join("release");
    let entered = base.join("entered");
    // Always release hooks during unwinding before the cleanup client requests shutdown.
    struct Release(std::path::PathBuf);
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, "");
        }
    }
    let _release = Release(gate.clone());
    let hook = format!(
        "#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep .01; done\n",
        entered.display(),
        gate.display()
    );
    std::fs::write(hooks.join("pre-commit"), &hook).unwrap();
    std::fs::set_permissions(
        hooks.join("pre-commit"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(project.join("notes"), "changed").unwrap();
    git(&["add", "notes"]);
    let status = client
        .application("workspace_git_status".into(), json!({"id":"primary"}))
        .unwrap();
    let started = client.application("application_operation_start".into(), json!({"command":"workspace_git_action","args":{"id":"primary","repo":0,"operation":"commit","paths":[],"message":"deferred once","expected":status[0]["index"]}})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entered.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let now = Instant::now();
    assert!(client
        .application("load_board".into(), Value::Null)
        .unwrap()["workspaces"]
        .is_array());
    let terminal = client.terminal_open(80, 24).unwrap();
    client
        .terminal_write(terminal.id.clone(), b"echo responsive\r".to_vec())
        .unwrap();
    assert!(now.elapsed() < Duration::from_secs(3));
    assert_eq!(
        client
            .application("application_operation_poll".into(), started.clone())
            .unwrap()["done"],
        false
    );
    assert!(client
        .shutdown()
        .unwrap_err()
        .contains("err.windows.operationBusy"));
    assert!(client
        .application(
            "cleanup_worktree".into(),
            json!({"id":"primary","force":true})
        )
        .unwrap_err()
        .contains("err.windows.operationBusy"));
    assert!(client
        .application(
            "chat_send".into(),
            json!({"session":"primary","text":"must not race"})
        )
        .unwrap_err()
        .contains("err.windows.operationBusy"));
    drop(client);
    std::fs::write(&gate, "").unwrap();
    let mut client = connector.connect(&target, events().0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if client
            .application("application_operation_poll".into(), started.clone())
            .unwrap()["done"]
            == true
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(client
        .application("application_operation_poll".into(), started)
        .is_err());
    assert_eq!(
        git(&["log", "--oneline"])
            .lines()
            .filter(|l| l.contains("deferred once"))
            .count(),
        1
    );
    std::fs::remove_file(&gate).unwrap();
    std::fs::remove_file(&entered).unwrap();
    std::fs::write(hooks.join("post-checkout"), hook).unwrap();
    std::fs::set_permissions(
        hooks.join("post-checkout"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let creation = client.application("application_operation_start".into(), json!({"command":"create_workspace","args":{"cols":80,"rows":24,"draft":{"project":target.workdir,"branch":"feature/deferred","base":"main","worktree":true,"title":"Deferred checkout","stage":"Preparando","prompt":"","inject":[],"agent":"codex","model":"fixture"}}})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !entered.exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    client
        .application(
            "rename_workspace".into(),
            json!({"id":"primary","title":"Edited while preparing"}),
        )
        .unwrap();
    assert_eq!(
        client
            .application("application_operation_poll".into(), creation.clone())
            .unwrap()["done"],
        false
    );
    drop(client);
    std::fs::write(&gate, "").unwrap();
    // The resident must commit without a poll, and preserve the independently edited card.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let catalog: Value =
            serde_json::from_slice(&std::fs::read(base.join("root/workspaces.json")).unwrap())
                .unwrap();
        if catalog["board"]["workspaces"].as_array().unwrap().len() == 2 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = connector.connect(&target, events().0).unwrap();
    let board = client
        .application("load_board".into(), Value::Null)
        .unwrap();
    assert_eq!(board["workspaces"][0]["title"], "Edited while preparing");
    let completed = client
        .application("application_operation_poll".into(), creation)
        .unwrap();
    assert_eq!(completed["done"], true);
    assert_eq!(completed["result"]["title"], "Deferred checkout");
    client.shutdown().unwrap();
    cleanup.armed = false;
    drop(client);
    std::fs::remove_dir_all(base).unwrap();
}
