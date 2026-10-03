//! Real pipes and a controlled subprocess exercise the adapter without a model or credentials.

use super::*;
use crate::lock::lock;
use prometeu_protocols::codex::Link;
use serde_json::{json, Value};
use std::os::unix::process::CommandExt;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

struct Peer {
    child: Arc<Mutex<Child>>,
    cancel: Option<mpsc::Sender<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl Peer {
    fn spawn(mode: &str) -> Self {
        let child = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/codex/fixtures/process-peer.mjs"))
            .arg(mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .expect("Node is required for the controlled stdio peer");
        let child = Arc::new(Mutex::new(child));
        let watched = child.clone();
        let (cancel, cancelled) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            if cancelled.recv_timeout(Duration::from_secs(10)).is_err() {
                let mut child = lock(&watched);
                if child.try_wait().ok().flatten().is_none() {
                    // Checking and signalling under the same lock prevents PID reuse after reaping.
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                }
            }
        });
        Self {
            child,
            cancel: Some(cancel),
            watchdog: Some(watchdog),
        }
    }

    fn cancel_deadline(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.join().unwrap();
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.cancel_deadline();
        let mut child = lock(&self.child);
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn run(mode: &str, resume: Option<String>) -> (Vec<Value>, i32) {
    let mut peer = Peer::spawn(mode);
    let stdin = lock(&peer.child).stdin.take().unwrap();
    let stdout = prometeu_process::output_lines(lock(&peer.child).stdout.take().unwrap());
    let stderr = prometeu_process::output_lines(lock(&peer.child).stderr.take().unwrap());
    let mut link = Link::new(
        Box::new(stdin),
        contract::start(resume),
        i18n::pick,
        env!("CARGO_PKG_VERSION"),
    );
    // Input queued before the handshake must be delivered once the thread identity is known.
    let mut events = link
        .write(&json!({"v":1,"type":"message.send","text":"Review the patch"}))
        .unwrap();
    let mut interrupted = false;
    for line in stdout {
        let incoming = link.on_line(&line);
        let delta = incoming
            .iter()
            .any(|event| event["type"] == "assistant.delta");
        events.extend(incoming);
        if mode == "interrupt" && delta && !interrupted {
            events.extend(link.write(&json!({"v":1,"type":"turn.interrupt"})).unwrap());
            interrupted = true;
        }
    }
    link.close();
    let stderr = stderr.collect::<Vec<_>>().join("\n");
    // A process can close its pipes without exiting; keep the deadline armed until it is reaped.
    let status = loop {
        if let Some(status) = lock(&peer.child).try_wait().unwrap() {
            break status;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    peer.cancel_deadline();
    let code = status.code().expect("stdio peer timed out or was killed");
    assert_eq!(code, if mode == "crash" { 23 } else { 0 }, "{stderr}");
    assert!(events
        .iter()
        .any(|event| event["providerSession"] == "thread"));
    assert!(events.iter().any(|event| event["delta"] == "Reviewed ✓"));
    (events, code)
}

#[test]
fn process_transport_interrupts_and_resumes_with_the_provider_identity() {
    let (first, _) = run("interrupt", None);
    assert!(first
        .iter()
        .any(|event| event["type"] == "turn.completed" && event["outcome"] == "interrupted"));
    let identity = first
        .iter()
        .find_map(|event| event["providerSession"].as_str())
        .unwrap();
    let (resumed, _) = run("resume", Some(identity.into()));
    assert_eq!(
        resumed
            .iter()
            .filter(|event| event["type"] == "turn.completed")
            .count(),
        1
    );
    assert!(resumed.iter().any(|event| event["outcome"] == "ok"));
    assert!(resumed
        .iter()
        .any(|event| event["block"]["text"] == "Reviewed ✓"));
}

#[test]
fn process_transport_does_not_invent_completion_after_partial_output_and_exit() {
    let (events, code) = run("crash", None);
    assert_eq!(code, 23);
    assert!(!events.iter().any(|event| event["type"] == "turn.completed"));
    let (resumed, _) = run("resume", Some("thread".into()));
    assert!(resumed.iter().any(|event| event["outcome"] == "ok"));
}
