#![cfg(unix)]
use prometeu_bridge::{
    RuntimeConnector, RuntimeEvents, RuntimeLauncher, StdioConnector, Target, WslLauncher,
};
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

struct LocalLauncher;
impl RuntimeLauncher for LocalLauncher {
    fn launch(&self, target: &Target) -> Result<Child, String> {
        Command::new(&target.executable)
            .args([
                "--root",
                &target.root,
                "--workdir",
                &target.workdir,
                "--codex",
                &target.codex,
            ])
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
fn until(events: &mpsc::Receiver<Value>, kind: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = events
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if frame["event"]["type"] == kind {
            return frame;
        }
    }
}
#[test]
fn bridge_streams_without_polling_and_resumes_after_disconnect() {
    roundtrip(Arc::new(LocalLauncher), "Test".into());
}
#[test]
#[ignore = "requires explicit WSL distribution with this checkout and Python 3"]
fn actual_wsl_transport_roundtrip() {
    roundtrip(
        Arc::new(WslLauncher),
        std::env::var("PROMETEU_TEST_WSL_DISTRIBUTION")
            .expect("select the distribution containing this checkout"),
    );
}
fn roundtrip(launcher: Arc<dyn RuntimeLauncher>, distribution: String) {
    let base = std::env::temp_dir().join(format!("prometeu-bridge-{}", uuid::Uuid::new_v4()));
    let workdir = base.join("project with spaces ' ação");
    std::fs::create_dir_all(&workdir).unwrap();
    let codex = base.join("codex fixture");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution,
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_string_lossy().into(),
        workdir: workdir.to_string_lossy().into(),
        codex: codex.to_string_lossy().into(),
    };
    let connector = StdioConnector { launcher };
    let (tx, rx) = mpsc::channel();
    let mut client = connector
        .connect(&target, Arc::new(Events(Mutex::new(tx))))
        .unwrap();
    assert!(client.snapshot().unwrap().generation.is_none());
    let first = client.start().unwrap();
    until(&rx, "session.identity");
    client.send("bridge first turn".into()).unwrap();
    until(&rx, "turn.completed");
    assert!(client
        .snapshot()
        .unwrap()
        .snapshot
        .text
        .contains("bridge first turn"));
    client.stop().unwrap();
    client.shutdown().unwrap();
    drop(client);
    let (tx, rx) = mpsc::channel();
    let mut resumed = connector
        .connect(&target, Arc::new(Events(Mutex::new(tx))))
        .unwrap();
    let second = resumed.start().unwrap();
    assert!(second.resuming);
    assert_ne!(first.generation, second.generation);
    assert_eq!(
        until(&rx, "session.identity")["event"]["providerSession"],
        "fixture-thread"
    );
    resumed.send("bridge resumed turn".into()).unwrap();
    until(&rx, "turn.completed");
    // Dropping the client closes stdin and waits for the host's cleanup and root lease release.
    drop(resumed);
    assert!(prometeu_runtime::store::Store::open(
        &std::path::PathBuf::from(&target.root),
        &workdir
    )
    .is_ok());
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn bootstrap_failure_reports_runtime_diagnostics_and_releases_child() {
    let connector = StdioConnector {
        launcher: Arc::new(LocalLauncher),
    };
    let (tx, _) = mpsc::channel();
    let target = Target {
        distribution: "Test".into(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: "/unused".into(),
        workdir: "/nonexistent-prometeu-fixture".into(),
        codex: "/unused".into(),
    };
    let error = connector
        .connect(&target, Arc::new(Events(Mutex::new(tx))))
        .err()
        .unwrap();
    assert!(error.contains("No such file"), "{error}");
}

#[test]
#[ignore = "requires explicit WSL distribution and authenticated Codex executable"]
fn actual_wsl_codex_recalls_after_reconnect() {
    let base = std::env::temp_dir().join(format!("prometeu-wsl-live-{}", uuid::Uuid::new_v4()));
    let workdir = base.join("project");
    std::fs::create_dir_all(&workdir).unwrap();
    let target = Target {
        distribution: std::env::var("PROMETEU_TEST_WSL_DISTRIBUTION").unwrap(),
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_string_lossy().into(),
        workdir: workdir.to_string_lossy().into(),
        codex: std::env::var("PROMETEU_TEST_CODEX").unwrap(),
    };
    let connector = StdioConnector {
        launcher: Arc::new(WslLauncher),
    };
    let mut identity = Value::Null;
    for (index, prompt) in [
        "Reply with exactly WSL_BRIDGE_RECALL_OK. Do not use tools.",
        "What exact marker did you reply with in your previous message? Reply only with that marker. Do not use tools.",
    ].into_iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        let mut client = connector.connect(&target, Arc::new(Events(Mutex::new(tx)))).unwrap();
        assert_eq!(client.start().unwrap().resuming, index > 0);
        let current = until(&rx, "session.identity")["event"]["providerSession"].clone();
        if index == 0 { identity = current; } else { assert_eq!(identity, current); }
        client.send(prompt.into()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        let mut answer = String::new();
        loop {
            let frame = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())).unwrap();
            if frame["event"]["type"] == "assistant.block" && frame["event"]["block"]["kind"] == "text" {
                answer.push_str(frame["event"]["block"]["text"].as_str().unwrap());
            }
            if frame["event"]["type"] == "turn.completed" {
                assert_eq!(frame["event"]["outcome"], "ok");
                break;
            }
        }
        assert_eq!(answer.trim(), "WSL_BRIDGE_RECALL_OK");
        client.shutdown().unwrap();
    }
    std::fs::remove_dir_all(base).unwrap();
}

fn terminal_roundtrip(launcher: Arc<dyn RuntimeLauncher>, distribution: String) {
    let base = std::env::temp_dir().join(format!("prometeu-terminal-{}", uuid::Uuid::new_v4()));
    let workdir = base.join("project with spaces");
    std::fs::create_dir_all(&workdir).unwrap();
    let codex = base.join("codex");
    std::fs::write(&codex, include_str!("fake_codex.py")).unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o700)).unwrap();
    let target = Target {
        distribution,
        executable: env!("CARGO_BIN_EXE_prometeu-runtime").into(),
        root: base.join("root").to_string_lossy().into(),
        workdir: workdir.to_string_lossy().into(),
        codex: codex.to_string_lossy().into(),
    };
    let connector = StdioConnector { launcher };
    let (tx, rx) = mpsc::channel();
    let mut client = connector
        .connect(&target, Arc::new(Events(Mutex::new(tx))))
        .unwrap();
    assert!(client.terminal_supported());
    assert!(client.terminal_open(0, 24).is_err());
    let opened = client.terminal_open(80, 24).unwrap();
    assert!(client.terminal_open(80, 24).is_err());
    assert!(client
        .terminal_write("stale".into(), b"do not write".to_vec())
        .is_err());
    assert!(client
        .terminal_write(opened.id.clone(), vec![0; 4097])
        .is_err());
    client.terminal_resize(opened.id.clone(), 93, 37).unwrap();
    client.terminal_write(opened.id.clone(), b"stty -echo; printf 'ready-%s\\n' terminal; pwd; stty size; printf '\\377\\303'; printf '\\251'; printf 'end-%s\\n' bytes\r".to_vec()).unwrap();
    let mut bytes = vec![];
    let mut seq = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !bytes
        .windows(b"end-bytes".len())
        .any(|window| window == b"end-bytes")
    {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if frame["terminal"]["kind"] == "output" {
            assert_eq!(frame["terminal"]["id"], opened.id);
            assert_eq!(frame["terminal"]["seq"], seq + 1);
            seq += 1;
            client.terminal_acknowledge(opened.id.clone(), seq).unwrap();
            bytes.extend(
                serde_json::from_value::<Vec<u8>>(frame["terminal"]["data"].clone()).unwrap(),
            );
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("ready-terminal"));
    assert!(text.contains(workdir.to_str().unwrap()));
    assert!(text.contains("37 93"));
    assert!(bytes.windows(3).any(|window| window == [255, 195, 169]));
    let snapshot = client.terminal_snapshot(opened.id.clone()).unwrap();
    assert!(snapshot.seq >= seq);
    assert!(snapshot.running);
    assert!(snapshot
        .data
        .windows(3)
        .any(|window| window == [255, 195, 169]));
    assert!(client
        .terminal_acknowledge(opened.id.clone(), u64::MAX)
        .is_err());
    client
        .terminal_acknowledge(opened.id.clone(), snapshot.seq)
        .unwrap();
    client
        .terminal_write(
            opened.id.clone(),
            b"head -c 1048576 /dev/zero; printf 'flow-%s\\n' complete\r".to_vec(),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut delivered = Vec::new();
    while !delivered
        .windows(b"flow-complete".len())
        .any(|bytes| bytes == b"flow-complete")
    {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if frame["terminal"]["kind"] != "output" {
            continue;
        }
        delivered
            .extend(serde_json::from_value::<Vec<u8>>(frame["terminal"]["data"].clone()).unwrap());
        client
            .terminal_acknowledge(
                opened.id.clone(),
                frame["terminal"]["seq"].as_u64().unwrap(),
            )
            .unwrap();
    }
    assert!(delivered.len() > 1024 * 1024);
    assert!(
        client
            .terminal_snapshot(opened.id.clone())
            .unwrap()
            .data
            .len()
            <= 512 * 1024
    );
    client.start().unwrap();
    until(&rx, "session.identity");
    client.send("conversation beside terminal".into()).unwrap();
    until(&rx, "turn.completed");
    client.stop().unwrap();
    assert!(client.terminal_snapshot(opened.id.clone()).unwrap().running);
    client
        .terminal_write(opened.id.clone(), b"exit 7\r".to_vec())
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if frame["terminal"]["kind"] == "closed" {
            assert_eq!(frame["terminal"]["code"], 7);
            break;
        }
    }
    let exited = client.terminal_snapshot(opened.id.clone()).unwrap();
    assert!(!exited.running);
    assert_eq!(exited.code, Some(7));
    assert!(client
        .terminal_write(opened.id.clone(), b"after exit".to_vec())
        .is_err());
    client.terminal_close(opened.id.clone()).unwrap();
    assert!(client.terminal_snapshot(opened.id.clone()).is_err());
    let second = client.terminal_open(80, 24).unwrap();
    assert_ne!(second.id, opened.id);
    client.start().unwrap();
    until(&rx, "session.identity");
    client.terminal_close(second.id).unwrap();
    client
        .send("conversation after terminal close".into())
        .unwrap();
    until(&rx, "turn.completed");
    let held = client.terminal_open(80, 24).unwrap();
    client
        .terminal_write(
            held.id.clone(),
            b"stty -icanon -echo; head -c 1048576 /dev/zero; sleep 30\r".to_vec(),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .unwrap();
        if frame["terminal"]["id"] == held.id && frame["terminal"]["seq"] == 64 {
            break;
        }
    }
    assert_eq!(client.terminal_snapshot(held.id.clone()).unwrap().seq, 64);
    let mut saturated = false;
    for _ in 0..128 {
        if client
            .terminal_write(held.id.clone(), vec![b'x'; 4096])
            .is_err()
        {
            saturated = true;
            break;
        }
    }
    assert!(saturated, "blocked native input must fill a bounded queue");
    assert_eq!(client.terminal_snapshot(held.id.clone()).unwrap().seq, 64);

    // EOF must release a reader waiting for output credits, too.
    let closing = std::time::Instant::now();
    // EOF must wait for PTY cleanup before releasing the runtime root.
    drop(client);
    assert!(closing.elapsed() < Duration::from_secs(9));
    assert!(prometeu_runtime::store::Store::open(
        &std::path::PathBuf::from(&target.root),
        &workdir
    )
    .is_ok());
    std::fs::remove_dir_all(base).unwrap();
}
#[test]
fn terminal_bytes_resize_conversation_isolation_and_eof_cleanup() {
    terminal_roundtrip(Arc::new(LocalLauncher), "Test".into());
}
#[test]
#[ignore = "requires explicit WSL distribution with this checkout and Python 3"]
fn actual_wsl_terminal_roundtrip() {
    terminal_roundtrip(
        Arc::new(WslLauncher),
        std::env::var("PROMETEU_TEST_WSL_DISTRIBUTION").unwrap(),
    );
}

#[test]
fn worktree_and_application_capabilities_refuse_older_hosts_without_sending_a_mutation() {
    struct Legacy;
    impl RuntimeLauncher for Legacy {
        fn launch(&self, _: &Target) -> Result<Child, String> {
            Command::new("python3")
                .args([
                    "-u",
                    "-c",
                    r#"
import json,sys
print(json.dumps({"v":1,"lifecycle":"ready","provider":"codex","executable":"/older-runtime","capabilities":["workspaces.v1"]}))
for line in sys.stdin:
    request=json.loads(line)
    assert request['action']['method'] in ['stop','shutdown'], 'unexpected mutation'
    print(json.dumps({'v':1,'id':request['id'],'result':{'stopped':True}}))
    if request['action']['method'] == 'shutdown': break
"#,
                ])
                .env_remove("PYTHONHOME")
                .env_remove("PYTHONPATH")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())
        }
    }
    let target = Target {
        distribution: "Test".into(),
        executable: "/unused".into(),
        root: "/private".into(),
        workdir: "/project".into(),
        codex: "/codex".into(),
    };
    let (tx, _) = mpsc::channel();
    let mut client = StdioConnector {
        launcher: Arc::new(Legacy),
    }
    .connect(&target, Arc::new(Events(Mutex::new(tx))))
    .unwrap();
    let error = client
        .workspace_worktree(prometeu_bridge::workspaces::WorktreeRequest {
            title: "New".into(),
            path: "/project".into(),
            branch: "feature/test".into(),
            base: "HEAD".into(),
        })
        .err()
        .unwrap();
    assert_eq!(error, "workspace_worktree_unsupported");
    assert_eq!(
        client
            .application(
                "new_tab".into(),
                serde_json::json!({"workspace":"primary","prompt":"Do not send"})
            )
            .unwrap_err(),
        "application_unsupported"
    );
    client.stop().unwrap();
    client.shutdown().unwrap();
}
