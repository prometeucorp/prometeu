//! Drives the real binary the way the desktop does over SSH: through `attach` processes that come
//! and go while the host and its terminals keep running.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_prometeu-host");

fn root() -> PathBuf {
    // Short: Unix socket paths are limited to about 100 bytes on macOS.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from("/tmp").join(format!("ph-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Attached {
    child: Child,
    stdin: ChildStdin,
    frames: mpsc::Receiver<Value>,
    next: u64,
}

impl Attached {
    /// An attachment from a desktop that predates device identities.
    fn new(root: &Path) -> Self {
        Self::spawn(root, &[])
    }

    /// An attachment from the desktop with this device ID.
    fn device(root: &Path, device: &str) -> Self {
        Self::spawn(
            root,
            &["--device", device, "--label", &format!("Mac {device}")],
        )
    }

    fn spawn(root: &Path, identity: &[&str]) -> Self {
        let mut child = Command::new(BIN)
            .args(["attach", "--root"])
            .arg(root)
            .args(identity)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, frames) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str(&line) {
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let attached = Self {
            child,
            stdin,
            frames,
            next: 0,
        };
        let ready = attached.until(|f| f["event"] == "ready");
        assert_eq!(ready["protocol"], 1);
        attached
    }

    fn until(&self, mut accept: impl FnMut(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frame = self
                .frames
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("frame before deadline");
            if accept(&frame) {
                return frame;
            }
        }
    }

    /// Assert that no frame `accept` would take arrives for a while.
    fn quiet(&self, mut accept: impl FnMut(&Value) -> bool, wait: Duration) {
        let deadline = Instant::now() + wait;
        while let Ok(frame) = self
            .frames
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            assert!(!accept(&frame), "unexpected frame {frame}");
        }
    }

    /// Confirm a hook the way the desktop does once it handled it.
    fn confirm(&mut self, hook: &Value) {
        self.call("hook_reply", json!({"hook": hook["hook"], "stdout": ""}));
    }

    /// Send without waiting for the reply; returns the request id.
    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"id": self.next, "method": method, "params": params})
        )
        .unwrap();
        self.next
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        writeln!(
            self.stdin,
            "{}",
            json!({"id": id, "method": method, "params": params})
        )
        .unwrap();
        let reply = self.until(|f| f["id"] == id);
        assert!(reply.get("error").is_none(), "{method}: {reply}");
        reply["result"].clone()
    }

    /// Collect a terminal's output frames until `needle` appears.
    fn output_until(&self, id: &str, needle: &str, mut text: String) -> (String, u64) {
        let mut seq = 0;
        while !text.contains(needle) {
            let frame = self.until(|f| f["event"] == "output" && f["id"] == id);
            seq = frame["seq"].as_u64().unwrap();
            text.push_str(&String::from_utf8_lossy(
                &STANDARD.decode(frame["data"].as_str().unwrap()).unwrap(),
            ));
        }
        (text, seq)
    }

    fn detach(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn decode(value: &Value) -> String {
    String::from_utf8_lossy(&STANDARD.decode(value.as_str().unwrap()).unwrap()).into()
}

fn shutdown(root: &Path) {
    let mut client = Attached::new(root);
    let _ = client.call("shutdown", json!({"force": true}));
    client.detach();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn terminals_survive_detachment_and_replay_the_gap() {
    let root = root();
    let mut first = Attached::new(&root);
    let cwd = std::env::temp_dir();
    first.call(
        "terminal_open",
        json!({"id": "t1", "argv": ["/bin/sh", "-c", "echo first; read line; echo got-$line; sleep 0.3; echo after-detach; sleep 30"],
               "cwd": cwd, "cols": 80, "rows": 24}),
    );
    let attached = first.call("terminal_attach", json!({"id": "t1", "seq": 0}));
    let (text, _) = first.output_until("t1", "first", decode(&attached["data"]));
    assert!(text.contains("first"));
    first.call(
        "terminal_write",
        json!({"id": "t1", "data": STANDARD.encode("ping\n")}),
    );
    let (_, seq) = first.output_until("t1", "got-ping", String::new());
    assert!(seq > 0);
    // The desktop goes away while the program keeps producing output.
    first.detach();
    std::thread::sleep(Duration::from_millis(800));

    let mut second = Attached::new(&root);
    let list = second.call("terminal_list", json!({}));
    assert_eq!(list[0]["id"], "t1");
    assert_eq!(list[0]["running"], true);
    let replay = second.call("terminal_attach", json!({"id": "t1", "seq": seq}));
    assert_eq!(replay["reset"], false);
    let text = decode(&replay["data"]);
    assert!(text.contains("after-detach"), "{text:?}");
    assert!(
        !text.contains("got-ping"),
        "only the gap is replayed: {text:?}"
    );

    second.call("terminal_signal", json!({"id": "t1", "signal": "kill"}));
    second.until(|f| f["event"] == "exit" && f["id"] == "t1");
    let stopped = second.call("terminal_list", json!({}));
    assert_eq!(stopped[0]["id"], "t1");
    assert_eq!(stopped[0]["pid"], list[0]["pid"]);
    assert_eq!(stopped[0]["running"], false);
    second.detach();
    shutdown(&root);
}

#[test]
fn stopping_a_listed_process_never_kills_a_replacement_under_the_same_id() {
    let root = root();
    let mut client = Attached::new(&root);
    let params = json!({"id": "same-id", "argv": ["/bin/sh", "-c", "read line"],
        "cwd": std::env::temp_dir(), "cols": 80, "rows": 24});
    client.call("terminal_open", params.clone());
    let original = client.call("terminal_list", json!({}))[0].clone();
    assert!(original["instance"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    client.call(
        "terminal_write",
        json!({"id": "same-id", "data": STANDARD.encode("done\n")}),
    );
    let wait_for_exit = |client: &mut Attached| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.call("terminal_list", json!({}))[0]["running"] == true {
            assert!(Instant::now() < deadline, "terminal should exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for_exit(&mut client);
    client.call("terminal_open", params);
    let replacement = client.call("terminal_list", json!({}))[0].clone();
    assert_ne!(original["instance"], replacement["instance"]);

    for (params, expected) in [
        (
            json!({"id": "same-id", "instance": original["instance"]}),
            "terminal_changed",
        ),
        (json!({"id": "same-id"}), "missing instance"),
    ] {
        let request = client.send("terminal_stop", params);
        assert_eq!(
            client.until(|frame| frame["id"] == request)["error"],
            expected
        );
        let listed = client.call("terminal_list", json!({}));
        assert_eq!(listed[0]["instance"], replacement["instance"]);
        assert_eq!(listed[0]["running"], true);
    }

    client.call(
        "terminal_stop",
        json!({"id": "same-id", "instance": replacement["instance"]}),
    );
    wait_for_exit(&mut client);
    client.detach();
    shutdown(&root);
}

#[test]
fn hooks_queue_while_detached_and_decisions_reach_the_attached_client() {
    let root = root();
    let mut client = Attached::new(&root);
    let cwd = std::env::temp_dir();
    client.call(
        "terminal_open",
        json!({"id": "s1", "argv": ["/bin/sh", "-c", "sleep 30"], "cwd": cwd, "cols": 80, "rows": 24}),
    );
    client.call("terminal_attach", json!({"id": "s1", "seq": 0}));
    let hook = |payload: &str| {
        let mut child = Command::new(BIN)
            .args(["hook", "--root"])
            .arg(&root)
            .env("PROMETEU_TERMINAL_HOOK_SESSION", "s1")
            .env("PROMETEU_TERMINAL_HOOK_TOKEN", "secret")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.as_bytes())
            .unwrap();
        child
    };

    // A permission request waits for the desktop's decision and prints it.
    let waiting = hook(r#"{"hook_event_name":"PermissionRequest"}"#);
    let frame = client.until(|f| f["event"] == "hook");
    assert_eq!(frame["permission"], true);
    assert_eq!(frame["env"]["PROMETEU_TERMINAL_HOOK_TOKEN"], "secret");
    client.call(
        "hook_reply",
        json!({"hook": frame["hook"], "stdout": STANDARD.encode("{\"decision\":1}\n")}),
    );
    let output = waiting.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":1}\n"
    );

    // Without a desktop, ordinary hooks return at once and are delivered on reattachment.
    client.detach();
    let quick = hook(r#"{"hook_event_name":"Stop"}"#);
    let started = Instant::now();
    let output = quick.wait_with_output().unwrap();
    assert!(output.stdout.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
    let mut client = Attached::new(&root);
    client.call("terminal_attach", json!({"id": "s1", "seq": 0}));
    let queued = client.until(|f| f["event"] == "hook");
    assert_eq!(queued["permission"], false);
    assert!(decode(&queued["payload"]).contains("Stop"));

    client.call("terminal_signal", json!({"id": "s1", "signal": "kill"}));
    client.detach();
    shutdown(&root);
}

#[test]
fn watches_stream_appends_from_the_client_offsets() {
    let root = root();
    let dir = root.join("transcripts");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.jsonl"), "one\n").unwrap();
    std::fs::write(dir.join("sub/b.jsonl"), "nested\n").unwrap();
    let mut client = Attached::new(&root);
    client.call(
        "watch",
        json!({"id": "w1", "dir": dir, "depth": 2, "offsets": {"a.jsonl": 4}}),
    );
    let nested = client.until(|f| f["event"] == "file" && f["path"] == "sub/b.jsonl");
    assert_eq!(decode(&nested["data"]), "nested\n");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join("a.jsonl"))
        .unwrap();
    file.write_all(b"two\n").unwrap();
    let appended = client.until(|f| f["event"] == "file" && f["path"] == "a.jsonl");
    assert_eq!(appended["offset"], 4);
    assert_eq!(decode(&appended["data"]), "two\n");

    // A flush answers only after the bytes written before it are on the stream.
    file.write_all(b"three\n").unwrap();
    let flush = client.send("watch_flush", json!({"id": "w1"}));
    let mut flushed = String::new();
    loop {
        let frame = client.until(|f| f["id"] == flush || f["event"] == "file");
        if frame["id"] == flush {
            assert!(frame.get("error").is_none(), "{frame}");
            break;
        }
        flushed.push_str(&decode(&frame["data"]));
    }
    assert_eq!(flushed, "three\n");

    let ran = client.call(
        "exec",
        json!({"argv": ["/bin/sh", "-c", "printf %s \"$X\"; exit 3"], "env": {"X": "y"}}),
    );
    assert_eq!(ran["code"], 3);
    assert_eq!(decode(&ran["stdout"]), "y");
    let target = root.join("deep/file.json");
    client.call(
        "file_write",
        json!({"path": target, "data": STANDARD.encode("{}"), "mode": 0o600}),
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
    client.detach();
    shutdown(&root);
}

#[test]
fn a_new_attachment_replaces_the_previous_one() {
    let root = root();
    let first = Attached::new(&root);
    let second = Attached::new(&root);
    first.until(|f| f["event"] == "replaced");
    first.detach();
    second.detach();
    shutdown(&root);
}

#[test]
fn requests_keep_their_order_without_waiting_for_replies() {
    let root = root();
    let mut client = Attached::new(&root);
    let cwd = std::env::temp_dir();
    let open = client.send(
        "terminal_open",
        json!({"id": "o1", "argv": ["/bin/sh", "-c", "echo ordered; sleep 30"], "cwd": cwd, "cols": 80, "rows": 24}),
    );
    let attach = client.send("terminal_attach", json!({"id": "o1", "seq": 0}));
    let opened = client.until(|f| f["id"] == open);
    assert!(opened.get("error").is_none(), "{opened}");
    let attached = client.until(|f| f["id"] == attach);
    assert!(attached.get("error").is_none(), "{attached}");
    client.call("terminal_signal", json!({"id": "o1", "signal": "kill"}));
    client.detach();
    shutdown(&root);
}

#[test]
fn a_terminal_reports_its_last_output_before_its_exit() {
    let root = root();
    let mut client = Attached::new(&root);
    let cwd = std::env::temp_dir();
    // Attach first, so every byte arrives as a live frame racing the exit.
    // Several megabytes keep the PTY buffer full up to the moment the process exits.
    let script = r"sleep 0.3; head -c 1000000 /dev/zero | tr '\0' x; echo; echo the-end";
    client.call(
        "terminal_open",
        json!({"id": "t3", "argv": ["/bin/sh", "-c", script], "cwd": cwd, "cols": 80, "rows": 24}),
    );
    let attached = client.call("terminal_attach", json!({"id": "t3", "seq": 0}));
    let mut text = decode(&attached["data"]);
    loop {
        let frame =
            client.until(|f| f["id"] == "t3" && (f["event"] == "output" || f["event"] == "exit"));
        if frame["event"] == "exit" {
            break;
        }
        text.push_str(&decode(&frame["data"]));
    }
    assert!(text.contains("the-end"), "output after exit was lost");
    client.detach();
    shutdown(&root);
}

#[test]
fn an_update_waits_for_a_running_command() {
    let root = root();
    let mut client = Attached::new(&root);
    // A command such as `git worktree add` runs before any terminal exists; retiring the host
    // then would lose its outcome.
    let exec = client.send("exec", json!({"argv": ["sleep", "1"], "timeout_ms": 5000}));
    std::thread::sleep(Duration::from_millis(300));
    let refused = client.send("shutdown", json!({}));
    let reply = client.until(|f| f["id"] == refused);
    assert_eq!(reply["error"], "host_busy", "{reply}");
    let done = client.until(|f| f["id"] == exec);
    assert_eq!(done["result"]["code"], 0, "{done}");
    let accepted = client.send("shutdown", json!({}));
    let reply = client.until(|f| f["id"] == accepted);
    assert!(reply.get("error").is_none(), "{reply}");
}

/// Run the hook helper the way Claude does for the conversation `session`.
fn run_hook(root: &Path, session: &str, payload: &str) -> Child {
    let mut child = Command::new(BIN)
        .args(["hook", "--root"])
        .arg(root)
        .env("PROMETEU_TERMINAL_HOOK_SESSION", session)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child
}

fn open_shell(client: &mut Attached, id: &str, script: &str) {
    client.call(
        "terminal_open",
        json!({"id": id, "argv": ["/bin/sh", "-c", script], "cwd": std::env::temp_dir(),
               "cols": 80, "rows": 24}),
    );
}

#[test]
fn desktops_on_different_devices_stay_attached_and_see_the_same_output() {
    let root = root();
    let mut first = Attached::device(&root, "mac-a");
    let mut second = Attached::device(&root, "mac-b");
    open_shell(&mut first, "m1", "while read line; do echo got-$line; done");
    first.call("terminal_attach", json!({"id": "m1", "seq": 0}));
    second.call("terminal_attach", json!({"id": "m1", "seq": 0}));
    first.call(
        "terminal_write",
        json!({"id": "m1", "data": STANDARD.encode("one\n")}),
    );
    first.output_until("m1", "got-one", String::new());
    second.output_until("m1", "got-one", String::new());
    second.call(
        "terminal_write",
        json!({"id": "m1", "data": STANDARD.encode("two\n")}),
    );
    first.output_until("m1", "got-two", String::new());
    second.output_until("m1", "got-two", String::new());

    let listed = first.call("clients_list", json!({}));
    let devices: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["device"].as_str().unwrap())
        .collect();
    assert_eq!(devices, ["mac-a", "mac-b"]);
    assert_eq!(listed[0]["current"], true);
    assert_eq!(listed[1]["label"], "Mac mac-b");
    first.quiet(|f| f["event"] == "replaced", Duration::from_millis(200));
    second.quiet(|f| f["event"] == "replaced", Duration::from_millis(200));

    // The same desktop coming back after sleep replaces only its own stale attachment.
    let mut again = Attached::device(&root, "mac-b");
    second.until(|f| f["event"] == "replaced");
    first.quiet(|f| f["event"] == "replaced", Duration::from_millis(200));
    assert_eq!(
        again
            .call("clients_list", json!({}))
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // A desktop can disconnect another one; it is told it was replaced.
    let detached = first.send("client_detach", json!({"device": "mac-a"}));
    assert_eq!(
        first.until(|f| f["id"] == detached)["error"],
        "unknown_client"
    );
    first.call("client_detach", json!({"device": "mac-b"}));
    again.until(|f| f["event"] == "replaced");

    first.call("terminal_signal", json!({"id": "m1", "signal": "kill"}));
    first.detach();
    second.detach();
    again.detach();
    shutdown(&root);
}

#[test]
fn every_device_receives_each_hook_once_including_those_it_missed() {
    let root = root();
    let mut first = Attached::device(&root, "mac-a");
    let mut second = Attached::device(&root, "mac-b");
    open_shell(&mut first, "h1", "sleep 30");
    first.call("terminal_attach", json!({"id": "h1", "seq": 0}));
    second.call("terminal_attach", json!({"id": "h1", "seq": 0}));
    let payload = |name: &str| format!(r#"{{"hook_event_name":"{name}"}}"#);
    let hook_named = |name: &'static str| {
        move |f: &Value| f["event"] == "hook" && decode(&f["payload"]).contains(name)
    };

    run_hook(&root, "h1", &payload("UserPromptSubmit"))
        .wait()
        .unwrap();
    let hook = first.until(hook_named("UserPromptSubmit"));
    first.confirm(&hook);
    let hook = second.until(hook_named("UserPromptSubmit"));
    second.confirm(&hook);

    // The second desktop goes away, perhaps after the host queued the next hook to it; the first
    // keeps receiving hooks live.
    second.detach();
    run_hook(&root, "h1", &payload("Stop")).wait().unwrap();
    let hook = first.until(hook_named("Stop"));
    first.confirm(&hook);

    // It comes back and receives only what it did not confirm.
    let mut second = Attached::device(&root, "mac-b");
    second.call("terminal_attach", json!({"id": "h1", "seq": 0}));
    second.until(hook_named("Stop"));
    second.quiet(hook_named("UserPromptSubmit"), Duration::from_millis(300));

    // A device this terminal never served receives only hooks no desktop confirmed.
    let mut third = Attached::device(&root, "mac-c");
    third.call("terminal_attach", json!({"id": "h1", "seq": 0}));
    third.quiet(|f| f["event"] == "hook", Duration::from_millis(300));

    first.call("terminal_signal", json!({"id": "h1", "signal": "kill"}));
    for client in [first, second, third] {
        client.detach();
    }
    shutdown(&root);
}

#[test]
fn the_first_decision_wins_and_the_other_desktops_withdraw_theirs() {
    let root = root();
    let mut first = Attached::device(&root, "mac-a");
    let mut second = Attached::device(&root, "mac-b");
    open_shell(&mut first, "p1", "sleep 30");
    first.call("terminal_attach", json!({"id": "p1", "seq": 0}));
    second.call("terminal_attach", json!({"id": "p1", "seq": 0}));
    let request = r#"{"hook_event_name":"PermissionRequest"}"#;

    let waiting = run_hook(&root, "p1", request);
    let asked = first.until(|f| f["event"] == "hook" && f["permission"] == true);
    second.until(|f| f["event"] == "hook" && f["hook"] == asked["hook"]);
    second.call(
        "hook_reply",
        json!({"hook": asked["hook"], "stdout": STANDARD.encode("{\"decision\":2}\n")}),
    );
    let resolved = first.until(|f| f["event"] == "hook_resolved");
    assert_eq!(resolved["hook"], asked["hook"]);
    // A late answer changes nothing.
    first.call(
        "hook_reply",
        json!({"hook": asked["hook"], "stdout": STANDARD.encode("{\"decision\":1}\n")}),
    );
    let output = waiting.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"decision\":2}\n"
    );

    // A desktop answering with nothing abstains; once all have, Claude's own dialog answers.
    let waiting = run_hook(&root, "p1", request);
    let asked = first.until(|f| f["event"] == "hook" && f["permission"] == true);
    first.call("hook_reply", json!({"hook": asked["hook"], "stdout": ""}));
    let started = Instant::now();
    second.until(|f| f["event"] == "hook" && f["hook"] == asked["hook"]);
    second.detach();
    let output = waiting.wait_with_output().unwrap();
    assert!(output.stdout.is_empty());
    assert!(started.elapsed() < Duration::from_secs(5));

    first.call("terminal_signal", json!({"id": "p1", "signal": "kill"}));
    first.detach();
    shutdown(&root);
}

#[test]
fn an_exited_terminal_stays_until_every_follower_collected_it() {
    let root = root();
    let mut first = Attached::device(&root, "mac-a");
    let mut second = Attached::device(&root, "mac-b");
    open_shell(&mut first, "f1", "read line");
    first.call("terminal_attach", json!({"id": "f1", "seq": 0}));
    second.call("terminal_attach", json!({"id": "f1", "seq": 0}));
    first.call(
        "terminal_write",
        json!({"id": "f1", "data": STANDARD.encode("done\n")}),
    );
    first.until(|f| f["event"] == "exit" && f["id"] == "f1");
    second.until(|f| f["event"] == "exit" && f["id"] == "f1");
    first.call("terminal_forget", json!({"id": "f1"}));
    assert_eq!(second.call("terminal_list", json!({}))[0]["id"], "f1");
    second.call("terminal_forget", json!({"id": "f1"}));
    assert_eq!(first.call("terminal_list", json!({})), json!([]));

    // A follower that leaves without collecting it no longer holds it.
    open_shell(&mut first, "f2", "read line");
    first.call("terminal_attach", json!({"id": "f2", "seq": 0}));
    second.call("terminal_attach", json!({"id": "f2", "seq": 0}));
    first.call(
        "terminal_write",
        json!({"id": "f2", "data": STANDARD.encode("done\n")}),
    );
    first.until(|f| f["event"] == "exit" && f["id"] == "f2");
    first.call("terminal_forget", json!({"id": "f2"}));
    second.detach();
    let deadline = Instant::now() + Duration::from_secs(5);
    while first.call("terminal_list", json!({})) != json!([]) {
        assert!(Instant::now() < deadline, "the terminal should be dropped");
        std::thread::sleep(Duration::from_millis(20));
    }
    first.detach();
    shutdown(&root);
}

#[test]
fn the_desktop_that_types_drives_the_terminal_size() {
    let root = root();
    let mut first = Attached::device(&root, "mac-a");
    let mut second = Attached::device(&root, "mac-b");
    open_shell(
        &mut first,
        "z1",
        "while read line; do echo size-$(stty size | tr ' ' x); done",
    );
    first.call("terminal_attach", json!({"id": "z1", "seq": 0}));
    second.call("terminal_attach", json!({"id": "z1", "seq": 0}));
    let write = |client: &mut Attached| {
        client.call(
            "terminal_write",
            json!({"id": "z1", "data": STANDARD.encode("\n")}),
        );
    };
    // The opener drives: another window's size waits until that desktop types.
    second.call(
        "terminal_resize",
        json!({"id": "z1", "cols": 100, "rows": 30}),
    );
    write(&mut first);
    first.output_until("z1", "size-24x80", String::new());
    write(&mut second);
    second.output_until("z1", "size-30x100", String::new());
    // Now the first desktop's resizes wait, until it types again.
    first.call(
        "terminal_resize",
        json!({"id": "z1", "cols": 90, "rows": 20}),
    );
    write(&mut second);
    second.output_until("z1", "size-30x100", String::new());
    write(&mut first);
    first.output_until("z1", "size-20x90", String::new());

    first.call("terminal_signal", json!({"id": "z1", "signal": "kill"}));
    first.detach();
    second.detach();
    shutdown(&root);
}

#[test]
fn a_desktop_that_stops_reading_is_dropped_instead_of_buffered() {
    let root = root();
    let mut reader = Attached::device(&root, "mac-a");
    // An attachment whose stdout nobody reads, like a desktop asleep behind a half-open connection.
    let mut stalled = Command::new(BIN)
        .args(["attach", "--root"])
        .arg(&root)
        .args(["--device", "mac-b"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stalled_in = stalled.stdin.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while reader
        .call("clients_list", json!({}))
        .as_array()
        .unwrap()
        .len()
        < 2
    {
        assert!(
            Instant::now() < deadline,
            "the second desktop should attach"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    open_shell(
        &mut reader,
        "b1",
        "sleep 0.3; head -c 40000000 /dev/zero | tr '\\0' x; sleep 30",
    );
    writeln!(
        stalled_in,
        "{}",
        json!({"id": 1, "method": "terminal_attach", "params": {"id": "b1", "seq": 0}})
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while reader
        .call("clients_list", json!({}))
        .as_array()
        .unwrap()
        .len()
        > 1
    {
        assert!(
            Instant::now() < deadline,
            "the stalled desktop should be dropped"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    reader.call("terminal_signal", json!({"id": "b1", "signal": "kill"}));
    let _ = stalled.kill();
    let _ = stalled.wait();
    reader.detach();
    shutdown(&root);
}

#[test]
fn the_host_reports_the_release_of_the_app_that_started_it() {
    let root = root();
    let mut child = Command::new(BIN)
        .args(["attach", "--root"])
        .arg(&root)
        .args(["--device", "mac-a", "--release", "9.9.9"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let ready: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(ready["release"], "9.9.9");
    assert_eq!(ready["clients"], true);
    let _ = child.kill();
    let _ = child.wait();
    shutdown(&root);
}
