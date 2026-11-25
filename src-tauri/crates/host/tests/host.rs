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
    fn new(root: &Path) -> Self {
        let mut child = Command::new(BIN)
            .args(["attach", "--root"])
            .arg(root)
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
    second.detach();
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
