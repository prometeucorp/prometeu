use super::*;
use crate::conversation::{
    stream::{ConversationEvents, Lines, TranscriptStore},
    Clock,
};
use crate::lock::lock;
use crate::session::output::{ExecutionObservation, SessionOutput};
use crate::session::pump::{PendingInput, PumpDiagnostics, PumpReactions};
use crate::tasks::Task;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

#[derive(Default)]
struct Effects {
    frames: Mutex<Vec<Value>>,
    order: Mutex<Vec<String>>,
    identity: ProcessIdentity,
    exits: Mutex<Vec<Result<ProcessExit, String>>>,
}
impl Effects {
    fn record(&self, value: &str) {
        lock(&self.order).push(value.into());
    }
}
impl TranscriptStore for Effects {
    fn load(&self) -> Result<String, String> {
        Ok(String::new())
    }
    fn append(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}
impl ConversationEvents for Effects {
    fn emit(&self, text: &str, _: u64) -> Result<(), String> {
        let frame: Value = serde_json::from_str(text).unwrap();
        self.record(frame["type"].as_str().unwrap());
        lock(&self.frames).push(frame);
        Ok(())
    }
}
impl ExecutionObservation for Effects {
    fn observe(&self, _: &Value) {}
    fn publish(&self, _: &Value) {}
}
impl PumpReactions for Effects {
    fn react(&self, _: &Value, _: &AtomicBool) -> bool {
        false
    }
}
impl PendingInput for Effects {
    fn queued(&self) -> bool {
        false
    }
    fn setup_running(&self) -> bool {
        false
    }
    fn schedule(&self) {
        panic!("no pending input");
    }
}
impl PumpDiagnostics for Effects {
    fn storage_error(&self, error: &str) {
        panic!("{error}");
    }
}
impl WorkerLifecycle for Effects {
    fn initialization_failed(&self, error: &str) {
        assert_eq!(error, "unsupported");
        self.record("init-error");
    }
    fn exited(&self, identity: &ProcessIdentity, result: Result<ProcessExit, String>) {
        assert_eq!(identity, &self.identity);
        self.record("exit");
        lock(&self.exits).push(result);
    }
}
struct Capture;
impl SessionTelemetry for Capture {
    fn observe(&mut self, _: &Value) {}
}
struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> u64 {
        42
    }
}
struct Waiter {
    effects: Arc<Effects>,
    fail: bool,
}
impl ProcessWait for Waiter {
    fn wait(&mut self) -> Result<ProcessExit, String> {
        self.effects.record("wait");
        match self.fail {
            true => Err("wait failed".into()),
            false => Ok(ProcessExit { code: Some(7) }),
        }
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        self.effects.record("drop");
    }
}
#[derive(Default)]
struct Tasks {
    queue: Mutex<Vec<Task>>,
    calls: AtomicUsize,
    reject: Option<usize>,
}
impl TaskExecutor for Tasks {
    fn spawn(&self, task: Task) -> Result<(), String> {
        if Some(self.calls.fetch_add(1, Ordering::Relaxed)) == self.reject {
            drop(task);
            return Err("no worker available".into());
        }
        lock(&self.queue).push(task);
        Ok(())
    }
}
impl Tasks {
    fn run(&self) {
        let tasks = std::mem::take(&mut *lock(&self.queue));
        for task in tasks {
            task();
        }
    }
}
fn fixture(fail_wait: bool) -> (ConversationWorkers, SessionPump<Capture>, Arc<Effects>) {
    let effects = Arc::new(Effects::default());
    let pump = SessionPump::new(
        Arc::new(SessionOutput {
            lines: Arc::new(Mutex::new(Lines::default())),
            gone: Arc::new(AtomicBool::new(false)),
            turn: AtomicBool::new(false),
            store: effects.clone(),
            events: effects.clone(),
            execution: effects.clone(),
        }),
        Capture,
        Arc::new(FixedClock),
        effects.clone(),
        effects.clone(),
        effects.clone(),
    );
    let translate = Box::new(|line: &str| {
        assert_eq!(line, "native output");
        vec![
            json!({"v":1,"type":"assistant.started","messageId":"m"}).to_string(),
            json!({"v":1,"type":"turn.completed","outcome":"ok"}).to_string(),
        ]
    });
    let workers = ConversationWorkers {
        stdout: Box::new(vec!["native output\n".into()].into_iter()),
        stderr: Box::new(vec!["filtered".into(), "  ".into(), "native error".into()].into_iter()),
        waiter: Box::new(Waiter {
            effects: effects.clone(),
            fail: fail_wait,
        }),
        identity: effects.identity.clone(),
        translate,
        stderr_line: |line| (line != "filtered").then(|| line.to_uppercase()),
    };
    (workers, pump, effects)
}

#[test]
fn initialization_precedes_workers_and_exit_follows_output_wait_and_cleanup() {
    let (workers, pump, effects) = fixture(false);
    let tasks = Tasks::default();
    let mut input = |frame: &Value, transcript: &str| {
        assert_eq!(*frame, json!({"v":1,"type":"commands.list"}));
        assert_eq!(transcript, "");
        assert_eq!(*lock(&effects.order), ["session.state"]);
        Ok(vec![json!({"v":1,"type":"commands.updated","commands":[]})])
    };
    workers
        .start(&mut input, pump, effects.clone(), &tasks)
        .unwrap();
    assert_eq!(*lock(&effects.order), ["session.state", "commands.updated"]);
    assert_eq!(tasks.calls.load(Ordering::Relaxed), 2);
    tasks.run();
    assert_eq!(
        *lock(&effects.order),
        [
            "session.state",
            "commands.updated",
            "system.notice",
            "assistant.started",
            "turn.completed",
            "wait",
            "drop",
            "exit"
        ]
    );
    assert_eq!(*lock(&effects.exits), [Ok(ProcessExit { code: Some(7) })]);
    let frames = lock(&effects.frames);
    assert_eq!(frames[0]["state"], "starting");
    assert_eq!(frames[0]["at"], 42);
    assert_eq!(frames[2]["detail"], "NATIVE ERROR");
    assert_eq!(frames[2]["code"], "provider.stderr");
    assert_eq!(frames[2]["level"], "error");
}

#[test]
fn initialization_failure_is_diagnostic_and_wait_failure_still_releases_the_owner() {
    let (workers, pump, effects) = fixture(true);
    let tasks = Tasks::default();
    let mut input = |_: &Value, _: &str| Err("unsupported".into());
    workers
        .start(&mut input, pump, effects.clone(), &tasks)
        .unwrap();
    assert_eq!(*lock(&effects.order), ["session.state", "init-error"]);
    tasks.run();
    assert_eq!(*lock(&effects.exits), [Err("wait failed".into())]);
    assert!(lock(&effects.order).ends_with(&["wait".into(), "drop".into(), "exit".into()]));
}

#[test]
fn rejected_first_or_second_worker_releases_the_waiter_without_publishing_exit() {
    for reject in [0, 1] {
        let (workers, pump, effects) = fixture(false);
        let tasks = Tasks {
            reject: Some(reject),
            ..Default::default()
        };
        let mut input = |_: &Value, _: &str| Ok(vec![]);
        assert_eq!(
            workers.start(&mut input, pump, effects.clone(), &tasks),
            Err("no worker available".into())
        );
        assert_eq!(
            lock(&effects.order).iter().filter(|s| *s == "drop").count(),
            1
        );
        tasks.run();
        assert!(lock(&effects.exits).is_empty());
        assert!(!lock(&effects.order).iter().any(|s| s == "wait"));
    }
}

#[test]
fn retired_output_is_suppressed_while_waiting_and_exit_reporting_still_complete() {
    let (workers, pump, effects) = fixture(false);
    let tasks = Tasks::default();
    let retired = pump.output.gone.clone();
    let mut input = |_: &Value, _: &str| Ok(vec![]);
    workers
        .start(&mut input, pump, effects.clone(), &tasks)
        .unwrap();
    retired.store(true, Ordering::Relaxed);
    tasks.run();
    assert_eq!(
        *lock(&effects.order),
        ["session.state", "wait", "drop", "exit"]
    );
}
