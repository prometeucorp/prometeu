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
fn durable_ingestion_commits_public_bytes_before_live_effects_without_a_second_append() {
    let (pump, effects) = fixture();
    lock(&effects.lines).absorb(r#"{"v":1,"type":"user.message","content":[]}"#);
    effects.settled.store(true, Ordering::Relaxed);
    effects.queued.store(true, Ordering::Relaxed);
    pump.output.turn.store(true, Ordering::Relaxed);
    let text = json!({"v":1,"type":"turn.completed","outcome":"ok","telemetry":{"private":1},"providerDurationMs":17}).to_string();
    let delivered = pump
        .feed_committed(&text, |public, position| {
            assert!(effects.lines.try_lock().is_err());
            assert!(pump.telemetry.try_lock().is_err());
            assert!(pump.output.turn.load(Ordering::Relaxed));
            assert!(lock(&effects.order).is_empty());
            assert_eq!(position, 1);
            let frame: Value = serde_json::from_str(public).unwrap();
            assert_eq!(frame["type"], "turn.completed");
            assert!(frame.get("telemetry").is_none());
            assert!(frame.get("providerDurationMs").is_none());
            effects.effect("commit");
            Ok(CommitOutcome::Appended)
        })
        .unwrap();
    assert!(delivered);
    assert_eq!(
        *lock(&effects.order),
        ["commit", "observe", "event", "capture", "publish", "react", "schedule"]
    );
    assert_eq!(lock(&effects.lines).end(), 2);
    assert_eq!(lock(&effects.lines).seq, 2);
    assert!(!pump.output.turn.load(Ordering::Relaxed));
}

#[test]
fn duplicate_and_failed_durable_ingestion_cannot_close_turn_capture_or_notify() {
    for failed in [false, true] {
        let (pump, effects) = fixture();
        pump.output.turn.store(true, Ordering::Relaxed);
        effects.settled.store(true, Ordering::Relaxed);
        effects.queued.store(true, Ordering::Relaxed);
        let result = pump.feed_committed(
            r#"{"v":1,"type":"turn.completed","outcome":"ok"}"#,
            |_, _| {
                if failed {
                    Err("journal conflict".into())
                } else {
                    Ok(CommitOutcome::AlreadyCommitted)
                }
            },
        );
        assert_eq!(
            result,
            if failed {
                Err("journal conflict".into())
            } else {
                Ok(false)
            }
        );
        assert!(lock(&effects.order).is_empty());
        assert!(pump.output.turn.load(Ordering::Relaxed));
        assert_eq!(lock(&effects.lines).seq, 0);
        assert_eq!(lock(&effects.lines).end(), 0);
    }
}

#[test]
fn durable_ingestion_keeps_ephemeral_and_private_frames_out_of_the_journal() {
    let (pump, effects) = fixture();
    for text in [
        r#"{"v":1,"type":"execution.observed","scope":"root","state":"working"}"#,
        r#"{"v":1,"type":"telemetry.usage","telemetry":{"private":1}}"#,
    ] {
        assert!(pump
            .feed_committed(text, |_, _| panic!(
                "nonpersistent frame has no durable line"
            ))
            .unwrap());
    }
    assert_eq!(lock(&effects.lines).seq, 1);
    assert_eq!(lock(&effects.lines).end(), 0);
    assert!(pump.output.turn.load(Ordering::Relaxed));
    assert!(!lock(&effects.order).iter().any(|item| item == "store"));
    pump.output.gone.store(true, Ordering::Relaxed);
    assert!(!pump
        .feed_committed(
            r#"{"v":1,"type":"turn.completed","outcome":"ok"}"#,
            |_, _| panic!("retired generation cannot commit")
        )
        .unwrap());
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

#[test]
fn durable_commands_commit_the_whole_canonical_batch_before_busy_echo_or_capture() {
    for outcome in [
        Ok(CommitOutcome::Appended),
        Ok(CommitOutcome::AlreadyCommitted),
        Err("disk full".to_owned()),
    ] {
        let (pump, effects) = fixture();
        let mut input = |_: &Value, _: &str| {
            assert!(!pump.output.turn.load(Ordering::Relaxed));
            effects.effect("send");
            Ok(vec![
                json!({"v":1,"type":"system.notice","at":42,"code":"fixture","level":"info","detail":"accepted"}),
            ])
        };
        let expected = outcome.clone();
        let result = pump.capture_committed(12, || {
            let _admission = lock(&effects.admission);
            pump.command_committed(
                &json!({"v":1,"type":"message.send","text":"hello"}),
                &mut input,
                |lines, offset| {
                    assert_eq!(offset, 0);
                    assert_eq!(
                        lines.len(),
                        2,
                        "busy state is ephemeral, user and notice commit together"
                    );
                    assert!(lines[0].contains("user.message"));
                    assert!(!pump.output.turn.load(Ordering::Relaxed));
                    assert_eq!(*lock(&effects.order), ["send"]);
                    effects.effect("commit");
                    expected
                },
            )
        });
        if outcome == Ok(CommitOutcome::Appended) {
            assert!(result.is_ok());
            assert!(pump.output.turn.load(Ordering::Relaxed));
            assert_eq!(lock(&effects.lines).end(), 2);
            assert_eq!(
                lock(&effects.order)
                    .iter()
                    .filter(|effect| effect.as_str() == "accepted")
                    .count(),
                1
            );
            assert!(!lock(&effects.order).iter().any(|effect| effect == "store"));
        } else {
            assert_eq!(result, outcome.map(|_| ()));
            assert!(!pump.output.turn.load(Ordering::Relaxed));
            assert_eq!(lock(&effects.lines).seq, 0);
            assert_eq!(*lock(&effects.order), ["send", "commit"]);
        }
    }
}

#[test]
fn durable_port_applies_to_clones_and_notices_but_preserves_ephemeral_delivery() {
    struct Commit(Arc<Effects>);
    impl CanonicalCommit for Commit {
        fn commit(&self, lines: &[String], offset: u64) -> Result<CommitOutcome, String> {
            assert_eq!(offset, 0);
            assert_eq!(lines.len(), 1);
            assert!(!lines[0].contains("telemetry"));
            self.0.effect("commit");
            Err("disk full".into())
        }
    }
    let (pump, effects) = fixture();
    let pump = pump.with_commit(Arc::new(Commit(effects.clone())));
    pump.clone()
        .feed(r#"{"v":1,"type":"system.notice","telemetry":{"private":1}}"#);
    assert_eq!(*lock(&effects.order), ["commit", "disk full"]);
    assert_eq!(lock(&effects.lines).seq, 0);
    lock(&effects.order).clear();
    pump.feed(r#"{"v":1,"type":"execution.observed","scope":"root","state":"working"}"#);
    assert!(!lock(&effects.order).iter().any(|effect| effect == "commit"));
    assert!(pump.output.turn.load(Ordering::Relaxed));
    assert_eq!(lock(&effects.lines).seq, 1);
}

#[test]
fn native_acceptance_preserves_source_frame_and_captures_only_after_durable_commit() {
    let (pump, effects) = fixture();
    let frame = json!({"v":1,"type":"user.message","at":9,"content":[{"kind":"text","text":"Native input"}]});
    pump.capture_committed(12, || {
        pump.accept_committed(&frame, |line, offset| {
            assert_eq!(offset, 0);
            assert_eq!(serde_json::from_str::<Value>(line).unwrap(), frame);
            assert!(lock(&effects.order).is_empty());
            effects.effect("commit");
            Ok(CommitOutcome::Appended)
        })
    })
    .unwrap();
    assert_eq!(lock(&effects.lines).end(), 1);
    lock(&effects.order).clear();
    pump.capture_committed(12, || {
        pump.accept_committed(&frame, |_, _| Ok(CommitOutcome::AlreadyCommitted))
    })
    .unwrap();
    assert!(lock(&effects.order).is_empty());
    assert_eq!(lock(&effects.lines).end(), 1);
}

#[test]
fn committed_historical_revision_only_updates_public_lines_without_reopening_activity() {
    let (pump, effects) = fixture();
    assert!(pump.revise_committed(r#"{"v":1,"type":"assistant.block","at":1,"messageId":"old","index":0,"block":{"kind":"text","text":"Final"}}"#,|_,offset| {
        assert_eq!(offset,0); effects.effect("commit"); Ok(CommitOutcome::Appended)
    }).unwrap());
    assert_eq!(*lock(&effects.order), ["commit", "event"]);
    assert_eq!(lock(&effects.lines).end(), 1);
    assert!(!pump.output.turn.load(Ordering::Relaxed));
    lock(&effects.order).clear();
    assert!(pump.revise_committed(r#"{"v":1,"type":"tool.completed","at":2,"toolId":"old-tool","output":"Final result","failed":false}"#, |_,offset| {
        assert_eq!(offset,1); effects.effect("commit"); Ok(CommitOutcome::Appended)
    }).unwrap());
    assert_eq!(*lock(&effects.order), ["commit", "event"]);
    assert_eq!(lock(&effects.lines).end(), 2);
    assert!(!pump.output.turn.load(Ordering::Relaxed));
}

#[test]
fn source_receipts_gate_ephemeral_capture_and_delivery_without_retaining_metadata() {
    for kind in ["telemetry.usage", "context.updated"] {
        let (pump, effects) = fixture();
        let text = json!({"v":1,"type":kind,"at":1,"used":12,"window":100}).to_string();
        assert!(pump
            .feed_observed_committed(&text, |line, offset| {
                assert!(line.is_none());
                assert_eq!(offset, 0);
                assert!(lock(&effects.order).is_empty());
                Err("receipt unavailable".into())
            })
            .is_err());
        assert!(lock(&effects.order).is_empty());
        assert!(!pump
            .feed_observed_committed(&text, |line, _| {
                assert!(line.is_none());
                Ok(CommitOutcome::AlreadyCommitted)
            })
            .unwrap());
        assert!(lock(&effects.order).is_empty());
        assert!(pump
            .feed_observed_committed(&text, |line, _| {
                assert!(line.is_none());
                effects.effect("receipt");
                Ok(CommitOutcome::Appended)
            })
            .unwrap());
        let order = lock(&effects.order);
        assert_eq!(order.first().map(String::as_str), Some("receipt"));
        assert!(order.iter().any(|effect| effect == "capture"));
        assert!(!order.iter().any(|effect| effect == "store"));
        assert_eq!(lock(&effects.lines).end(), 0);
    }
}
