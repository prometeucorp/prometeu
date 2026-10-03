use super::*;
use crate::conversation::stream::{ConversationEvents, Lines, TranscriptStore};
use crate::session::output::ExecutionObservation;
use serde_json::json;
use std::sync::Weak;

#[derive(Default)]
struct Effects {
    lines: Arc<Mutex<Lines>>,
    capture: Mutex<Weak<Mutex<Capture>>>,
    admission: Mutex<()>,
    order: Mutex<Vec<String>>,
    settled: AtomicBool,
    queued: AtomicBool,
    setup: AtomicBool,
    fail_store: AtomicBool,
}
impl Effects {
    fn effect(&self, name: &str) {
        lock(&self.order).push(name.into());
    }
    fn outside_locks(&self) {
        assert!(self.lines.try_lock().is_ok());
        assert!(self.admission.try_lock().is_ok());
        assert!(lock(&self.capture).upgrade().unwrap().try_lock().is_ok());
    }
}
impl TranscriptStore for Effects {
    fn load(&self) -> Result<String, String> {
        Ok(String::new())
    }
    fn append(&self, text: &str) -> Result<(), String> {
        assert!(!text.contains("telemetry"));
        assert!(self.lines.try_lock().is_err());
        self.effect("store");
        match self.fail_store.load(Ordering::Relaxed) {
            true => Err("disk full".into()),
            false => Ok(()),
        }
    }
}
impl ConversationEvents for Effects {
    fn emit(&self, text: &str, _: u64) -> Result<(), String> {
        assert!(!text.contains("telemetry"));
        assert!(self.lines.try_lock().is_err());
        self.effect("event");
        Ok(())
    }
}
impl ExecutionObservation for Effects {
    fn observe(&self, _: &Value) {
        assert!(self.lines.try_lock().is_err());
        self.effect("observe");
    }
    fn publish(&self, _: &Value) {
        self.outside_locks();
        self.effect("publish");
    }
}
impl PumpReactions for Effects {
    fn react(&self, frame: &Value, ready: &AtomicBool) -> bool {
        self.outside_locks();
        self.effect("react");
        if frame["type"] == "commands.updated" && !ready.swap(true, Ordering::Relaxed) {
            self.effect("ready");
        }
        self.settled.load(Ordering::Relaxed)
    }
}
impl PendingInput for Effects {
    fn queued(&self) -> bool {
        self.outside_locks();
        self.queued.load(Ordering::Relaxed)
    }
    fn setup_running(&self) -> bool {
        self.outside_locks();
        self.setup.load(Ordering::Relaxed)
    }
    fn schedule(&self) {
        self.outside_locks();
        self.effect("schedule");
    }
}
impl PumpDiagnostics for Effects {
    fn storage_error(&self, error: &str) {
        self.effect(error);
    }
}
struct Capture(Arc<Effects>);
impl SessionTelemetry for Capture {
    fn observe(&mut self, _: &Value) {
        assert!(self.0.lines.try_lock().is_ok());
        assert!(lock(&self.0.capture).upgrade().unwrap().try_lock().is_err());
        self.0.effect("capture");
    }
}
impl CommandTelemetry for Capture {
    type Prepared = u64;
    fn generation(&self) -> u64 {
        7
    }
    fn accepted(&mut self, prepared: u64, generation: u64, _: &[Value]) {
        assert_eq!((prepared, generation), (12, 7));
        assert!(self.0.lines.try_lock().is_ok());
        assert!(self.0.admission.try_lock().is_ok());
        self.0.effect("accepted");
    }
}
struct Clock;
impl crate::conversation::Clock for Clock {
    fn now(&self) -> u64 {
        42
    }
}
fn fixture() -> (SessionPump<Capture>, Arc<Effects>) {
    let effects = Arc::new(Effects::default());
    let output = Arc::new(SessionOutput {
        lines: effects.lines.clone(),
        gone: Arc::new(AtomicBool::new(false)),
        turn: AtomicBool::new(false),
        store: effects.clone(),
        events: effects.clone(),
        execution: effects.clone(),
    });
    let pump = SessionPump::new(
        output,
        Capture(effects.clone()),
        Arc::new(Clock),
        effects.clone(),
        effects.clone(),
        effects.clone(),
    );
    *lock(&effects.capture) = Arc::downgrade(&pump.telemetry);
    (pump, effects)
}

#[test]
fn output_capture_precedes_reentrant_effects_and_storage_failure_does_not_stop_delivery() {
    let (pump, effects) = fixture();
    effects.fail_store.store(true, Ordering::Relaxed);
    pump.feed(
        &json!({"v":1,"type":"assistant.block","messageId":"a","block":{"kind":"text","text":"hello"},"telemetry":{"private":1}})
            .to_string(),
    );
    assert_eq!(
        *lock(&effects.order),
        [
            "observe",
            "store",
            "event",
            "capture",
            "disk full",
            "publish",
            "react"
        ]
    );
    assert_eq!(lock(&effects.lines).seq, 1);
    assert!(pump.output.turn.load(Ordering::Relaxed));
}

#[test]
fn clones_share_readiness_and_retirement_and_private_usage_never_enters_history() {
    let (pump, effects) = fixture();
    let clone = pump.clone();
    pump.feed(r#"{"v":1,"type":"commands.updated","commands":[]}"#);
    clone.feed(r#"{"v":1,"type":"commands.updated","commands":[]}"#);
    assert_eq!(
        lock(&effects.order)
            .iter()
            .filter(|e| *e == "ready")
            .count(),
        1
    );
    let seq = lock(&effects.lines).seq;
    lock(&effects.order).clear();
    clone.feed(r#"{"type":"telemetry.usage","telemetry":{"private":1}}"#);
    assert_eq!(lock(&effects.lines).seq, seq);
    assert_eq!(*lock(&effects.order), ["capture", "publish", "react"]);
    lock(&effects.order).clear();
    pump.feed("invalid json");
    pump.output.gone.store(true, Ordering::Relaxed);
    clone.feed(r#"{"v":1,"type":"turn.completed","outcome":"ok"}"#);
    assert!(lock(&effects.order).is_empty());
    assert_eq!(lock(&effects.lines).seq, seq);
}

#[test]
fn pending_input_requires_settlement_a_queue_and_finished_setup() {
    for (settled, queued, setup, expected) in [
        (false, true, false, false),
        (true, false, false, false),
        (true, true, true, false),
        (true, true, false, true),
    ] {
        let (pump, effects) = fixture();
        effects.settled.store(settled, Ordering::Relaxed);
        effects.queued.store(queued, Ordering::Relaxed);
        effects.setup.store(setup, Ordering::Relaxed);
        pump.feed(r#"{"v":1,"type":"turn.completed","outcome":"ok"}"#);
        assert_eq!(
            lock(&effects.order).iter().any(|e| e == "schedule"),
            expected
        );
    }
}

#[test]
fn command_capture_releases_admission_and_transcript_before_reactions() {
    let (pump, effects) = fixture();
    let mut input = |_: &Value, _: &str| {
        assert!(effects.admission.try_lock().is_err());
        assert!(effects.lines.try_lock().is_err());
        assert!(pump.telemetry.try_lock().is_err());
        assert!(pump.output.turn.load(Ordering::Relaxed));
        effects.effect("send");
        Ok(vec![])
    };
    pump.capture(12, || {
        let _admission = lock(&effects.admission);
        pump.command(
            &json!({"v":1,"type":"message.send","text":"hello"}),
            &mut input,
        )
    })
    .unwrap();
    assert_eq!(
        *lock(&effects.order),
        [
            "send", "observe", "event", "observe", "store", "event", "accepted", "publish",
            "react", "publish", "react"
        ]
    );
    assert_eq!(lock(&effects.lines).seq, 2);
}

#[test]
fn failed_write_restores_the_turn_without_capture_or_echo_and_admission_errors_do_nothing() {
    for active in [false, true] {
        let (pump, effects) = fixture();
        pump.output.turn.store(active, Ordering::Relaxed);
        let mut input = |_: &Value, _: &str| Err("broken pipe".to_string());
        assert_eq!(
            pump.capture(12, || pump.command(
                &json!({"v":1,"type":"message.send","text":"hello"}),
                &mut input
            )),
            Err("broken pipe".into())
        );
        assert_eq!(pump.output.turn.load(Ordering::Relaxed), active);
        assert_eq!(
            pump.capture(12, || Err("replaced".into())),
            Err("replaced".into())
        );
        assert!(lock(&effects.order).is_empty());
        assert_eq!(lock(&effects.lines).seq, 0);
    }
}
