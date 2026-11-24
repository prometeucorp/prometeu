use super::*;
use prometeu_core::{
    conversation::stream::ConversationEvents,
    session::{
        output::{CommandTelemetry, ExecutionObservation, SessionOutput, SessionTelemetry},
        pump::{PendingInput, PumpDiagnostics, PumpReactions, SessionPump},
    },
};
use prometeu_protocols::terminal_transcript::TranscriptPhase;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    root: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        Self {
            root: std::env::temp_dir().join(format!("prometeu-ingestion-{}", uuid::Uuid::new_v4())),
        }
    }
    fn open(&self, native: Option<&str>, fresh: bool) -> Arc<JournalStore> {
        self.open_as("cursor", native, fresh)
    }
    fn open_as(&self, provider: &str, native: Option<&str>, fresh: bool) -> Arc<JournalStore> {
        JournalStore::open(
            &self.root.join("canonical.jsonl"),
            &journal_path(&self.root, "app").unwrap(),
            JournalBinding {
                app_session: "app".into(),
                provider: provider.into(),
                native_session: native.map(str::to_owned),
            },
            fresh,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
#[derive(Default)]
struct Effects {
    events: Mutex<Vec<String>>,
    other: Mutex<Vec<&'static str>>,
    work: Mutex<prometeu_core::conversation::work::Work>,
    status: Mutex<Option<prometeu_core::board::Status>>,
}
impl ConversationEvents for Effects {
    fn emit(&self, line: &str, _: u64) -> Result<(), String> {
        lock(&self.events).push(line.into());
        Ok(())
    }
}
impl ExecutionObservation for Effects {
    fn observe(&self, _: &Value) {
        lock(&self.other).push("observe");
    }
    fn publish(&self, _: &Value) {
        lock(&self.other).push("publish");
    }
}
impl PumpReactions for Effects {
    fn react(&self, frame: &Value, _: &AtomicBool) -> bool {
        lock(&self.other).push("react");
        let mut work = lock(&self.work);
        let waited = work.waiting();
        let settled = work.observe(frame);
        if let Some(status) =
            prometeu_core::session::reactions::next_status(frame, settled, waited, &work)
        {
            *lock(&self.status) = Some(status);
        }
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
        panic!()
    }
}
impl PumpDiagnostics for Effects {
    fn storage_error(&self, _: &str) {
        lock(&self.other).push("storage_error");
    }
}
struct Telemetry(Arc<Effects>);
impl SessionTelemetry for Telemetry {
    fn observe(&mut self, _: &Value) {
        lock(&self.0.other).push("telemetry");
    }
}
impl CommandTelemetry for Telemetry {
    type Prepared = ();
    fn generation(&self) -> u64 {
        0
    }
    fn accepted(&mut self, _: (), _: u64, _: &[Value]) {
        lock(&self.0.other).push("accepted");
    }
}
struct Clock;
impl prometeu_core::conversation::Clock for Clock {
    fn now(&self) -> u64 {
        10
    }
}
fn pump(store: &Arc<JournalStore>) -> (SessionPump<Telemetry>, Arc<Effects>) {
    let effects = Arc::new(Effects::default());
    let output = Arc::new(SessionOutput {
        lines: Arc::new(Mutex::new(store.seeded().unwrap())),
        gone: Arc::new(AtomicBool::new(false)),
        turn: AtomicBool::new(false),
        store: store.clone(),
        events: effects.clone(),
        execution: effects.clone(),
    });
    (
        SessionPump::new(
            output,
            Telemetry(effects.clone()),
            Arc::new(Clock),
            effects.clone(),
            effects.clone(),
            effects.clone(),
        )
        .with_commit(store.clone()),
        effects,
    )
}
fn user(id: &str, text: &str) -> NativeCapture {
    let frame = json!({"v":1,"type":"user.message","at":1,"content":[{"kind":"text","text":text}]});
    NativeCapture {
        occurrence: format!("native-user:{id}"),
        version: format!("user-version:{id}"),
        checkpoint: json!({"kind":"user","contentHash":content_hash(&frame)}),
        frame: Some(frame),
        input_binding: None,
    }
}
fn answer(version: &str, text: &str) -> NativeCapture {
    NativeCapture {
        occurrence: "native-answer:step:0".into(),
        version: version.into(),
        checkpoint: json!({"kind":"text","messageId":"native-answer","index":0}),
        frame: Some(
            json!({"v":1,"type":"assistant.block","at":2,"messageId":"native-answer","index":0,"block":{"kind":"text","text":text}}),
        ),
        input_binding: None,
    }
}

#[test]
fn captured_antigravity_escape_stops_pump_and_late_source_content_without_completion() {
    assert_captured_antigravity_escape(include_str!(
        "../../../terminal_agent_controls/fixtures/antigravity-1.5.10-result.json"
    ));
}

#[test]
fn captured_antigravity_file_escape_done_idle_keeps_late_thinking_quiet() {
    assert_captured_antigravity_escape(include_str!(
        "../../../terminal_agent_controls/fixtures/antigravity-1.5.10-tool-interrupted-result.json"
    ));
}

fn assert_captured_antigravity_escape(body: &str) {
    use prometeu_core::board::Status;
    use prometeu_protocols::terminal_status::{ProgramStatusEvent, ProgramStatusParser};
    let capture: Value = serde_json::from_str(body).unwrap();
    let file = capture["exploredFile"] == true;
    assert_eq!(capture["interrupt"], "escape");
    assert_eq!(capture["processRemainsAlive"], true);
    for delayed_user in [false, true] {
        let fixture = Fixture::new();
        let store = fixture.open(Some("native"), false);
        let (live, effects) = pump(&store);
        let (mut transport, bytes) = super::super::tests::fixture();
        let inner = Arc::get_mut(&mut transport).unwrap();
        inner.provider = crate::state::ProviderId::Antigravity;
        inner.journal = Some(store.clone());
        inner.activity = Some(live.output.clone());
        inner.ingestion_ready.store(true, Ordering::Release);
        let mut reports = capture["nativeRootStates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|state| {
                let bytes = format!(
                    "\x1b]7501;state={}\x07",
                    state.as_str().unwrap().to_lowercase()
                );
                let ProgramStatusEvent::Report(report) = ProgramStatusParser::default()
                    .feed(bytes.as_bytes())
                    .remove(0)
                else {
                    panic!("captured root status");
                };
                report
            });
        let first = user("first", "Count slowly");
        let capture_user = || {
            live.accept_committed(first.frame.as_ref().unwrap(), |line, offset| {
                store.commit_record(&first, Some(line), offset)
            })
            .unwrap();
            transport.captured_input(&live);
        };
        assert!(transport.observe_report(reports.next().unwrap(), &live));
        assert!(transport.ready());
        if !delayed_user {
            capture_user();
        }
        transport.observe_report(reports.next().unwrap(), &live);
        assert!(transport.busy());
        assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
        let source_answer = |version, text| {
            let mut record = answer(version, text);
            record.checkpoint["sourceScope"] = json!("record");
            record.checkpoint["sourceUser"] = json!(first.occurrence);
            if file {
                record.checkpoint["kind"] = json!("thinking");
                record.frame.as_mut().unwrap()["block"]["kind"] = json!("thinking");
            }
            record
        };
        if delayed_user {
            transport.write_user_bytes(b"\x1b").unwrap();
            assert_eq!(*lock(&bytes), b"\x1b");
            assert!(transport.busy(), "Escape alone is not stop evidence");
        } else if file {
            for (kind, frame) in [
                (
                    "tool",
                    json!({"v":1,"type":"assistant.block","at":2,"messageId":"native-file","index":2,"block":{"kind":"tool","id":"read-one","name":"read_file","input":{"file_path":"synthetic.txt"}}}),
                ),
                (
                    "result",
                    json!({"v":1,"type":"tool.completed","at":3,"toolId":"read-one","output":"Synthetic rows","error":false,"background":false}),
                ),
            ] {
                let record = NativeCapture {
                    occurrence: format!("native-file:block:2:{kind}"),
                    version: "one".into(),
                    checkpoint: json!({"kind":kind,"messageId":"native-file","index":2,"toolId":"read-one","sourceScope":"record","sourceUser":first.occurrence}),
                    frame: Some(frame),
                    input_binding: None,
                };
                assert_eq!(
                    store.disposition(&record).unwrap(),
                    NativeDisposition::Output
                );
                transport
                    .feed_native_output(&live, &store, &record)
                    .unwrap();
            }
            assert!(live.output.turn.load(Ordering::Acquire));
        } else {
            let partial = source_answer("partial", "1\n2");
            assert_eq!(
                store.disposition(&partial).unwrap(),
                NativeDisposition::Output
            );
            transport
                .feed_native_output(&live, &store, &partial)
                .unwrap();
            assert!(live.output.turn.load(Ordering::Acquire));
        }
        for report in reports {
            transport.observe_report(report, &live);
        }
        assert!(!transport.busy());
        assert!(matches!(*lock(&effects.status), Some(Status::Pronta)));
        if delayed_user {
            capture_user();
            assert!(
                !transport.busy(),
                "late first user claims the already stopped cycle"
            );
            assert!(matches!(*lock(&effects.status), Some(Status::Pronta)));
        }
        assert!(transport.ready());
        lock(&effects.other).clear();
        for version in ["final", "final-revision"] {
            let late = source_answer(version, "1\n2\n3");
            assert_eq!(store.disposition(&late).unwrap(), NativeDisposition::Output);
            transport.feed_native_output(&live, &store, &late).unwrap();
            assert!(!transport.busy());
            assert!(matches!(*lock(&effects.status), Some(Status::Pronta)));
        }
        assert!(
            lock(&effects.other).is_empty(),
            "late source content has no live reactions"
        );
        assert!(store.load().unwrap().contains("1\\n2\\n3"));
        assert!(lock(&effects.events).iter().all(|line| {
            serde_json::from_str::<Value>(line).unwrap()["type"] != "turn.completed"
        }));
        transport.begin_submission("next", "Next task").unwrap();
        transport.send_message("Next task").unwrap();
        assert!(!lock(&transport.execution).root_activity_stopped());
        assert!(!lock(&transport.execution).root_content_idle());
        transport.end_submission("next");
        let next = user("next", "Next task");
        live.accept_committed(next.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(&next, Some(line), offset)
        })
        .unwrap();
        transport.captured_input(&live);
        assert!(transport.busy(), "the next accepted input is a live turn");
        let mut next_answer = answer("next", "A new live response");
        next_answer.occurrence = "next-answer:step:0".into();
        next_answer.checkpoint["messageId"] = json!("next-answer");
        next_answer.checkpoint["sourceScope"] = json!("record");
        next_answer.checkpoint["sourceUser"] = json!(next.occurrence);
        next_answer.frame.as_mut().unwrap()["messageId"] = json!("next-answer");
        assert_eq!(
            store.disposition(&next_answer).unwrap(),
            NativeDisposition::Output
        );
        transport
            .feed_native_output(&live, &store, &next_answer)
            .unwrap();
        assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
    }
}

#[test]
fn stopped_root_keeps_requests_and_child_loss_does_not_revive_late_content() {
    use prometeu_core::board::Status;
    use prometeu_protocols::terminal_status::{ProgramStatusReport, ProgramStatusState};
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let (mut transport, _) = super::super::tests::fixture();
    Arc::get_mut(&mut transport).unwrap().activity = Some(live.output.clone());
    let first = user("first", "Task with child");
    live.accept_committed(first.frame.as_ref().unwrap(), |line, offset| {
        store.commit_record(&first, Some(line), offset)
    })
    .unwrap();
    transport.captured_input(&live);
    let observe = |id: Option<&str>, state| {
        transport.observe_report(
            ProgramStatusReport {
                id: id.map(str::to_owned),
                state,
                app: None,
                kind: None,
                progress: None,
                message: None,
                title: None,
            },
            &live,
        )
    };
    observe(None, ProgramStatusState::Working);
    observe(Some("child"), ProgramStatusState::Working);
    live.feed(&json!({"v":1,"type":"request.opened","at":2,"requestId":"approval","kind":"permission","input":{}}).to_string());
    observe(None, ProgramStatusState::Idle);
    assert!(!live.output.turn.load(Ordering::Acquire));
    assert!(transport.busy(), "the child remains active");
    assert!(lock(&effects.work).busy());
    assert!(lock(&effects.work).waiting());
    assert!(matches!(*lock(&effects.status), Some(Status::Querendo)));
    observe(Some("child"), ProgramStatusState::Clear);
    let events = lock(&effects.events)
        .iter()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert!(events
        .iter()
        .any(|event| event["type"] == "execution.observed" && event["state"] == "unknown"));
    assert!(
        lock(&effects.work).waiting(),
        "native idle never answers a real request"
    );
    let mut late = answer("late", "Stopped partial answer");
    late.checkpoint["sourceScope"] = json!("record");
    late.checkpoint["sourceUser"] = json!(first.occurrence);
    assert_eq!(store.disposition(&late).unwrap(), NativeDisposition::Output);
    lock(&effects.other).clear();
    transport.feed_native_output(&live, &store, &late).unwrap();
    assert!(!live.output.turn.load(Ordering::Acquire));
    assert!(lock(&effects.other).is_empty());
    assert!(matches!(*lock(&effects.status), Some(Status::Querendo)));
}
fn checkpoint() -> NativeCapture {
    NativeCapture {
        occurrence: "native-answer:checkpoint".into(),
        version: "turn-version".into(),
        checkpoint: json!({"kind":"turn","status":"completed"}),
        frame: None,
        input_binding: None,
    }
}
fn batch(id: &str, records: Vec<NativeCapture>) -> NativeIngestionBatch {
    NativeIngestionBatch {
        mode: crate::terminal_agent_observation::NativeIngestionMode::Snapshot,
        id: id.into(),
        phase: TranscriptPhase::Replay,
        source_revision: id.into(),
        records,
        finished: true,
        complete: true,
    }
}
fn send(store: &JournalStore, pump: &SessionPump<Telemetry>, id: &str, text: &str) {
    let normalized = crate::native_ui_readiness::normalize_cursor_submission(text);
    let submission = store.submission(id, text, &normalized).unwrap();
    let mut input = |_: &Value, _: &str| Ok(vec![]);
    pump.capture_committed((), || {
        pump.command_committed(
            &json!({"v":1,"type":"message.send","text":text}),
            &mut input,
            |lines, offset| store.commit_submission(&submission, lines, offset),
        )
    })
    .unwrap();
}

#[test]
fn journal_store_recovers_unbound_normalized_input_and_missing_native_tail_before_seeding() {
    let fixture = Fixture::new();
    let store = fixture.open(None, true);
    let (live, effects) = pump(&store);
    send(&store, &live, "first", "Repeat\r\n\ttext");
    store.bind_native("native").unwrap();
    assert!(!store.caught_up());
    let before = store.load().unwrap();
    assert!(before.contains("\\r\\n\\t"));
    drop(live);
    drop(store);
    let store = fixture.open(Some("native"), false);
    let recovery = batch(
        "root-one",
        vec![
            user("one", "Repeat\n  text"),
            answer("one", "Recovered answer"),
            checkpoint(),
        ],
    );
    store.replay(&recovery).unwrap();
    let seeded = store.seeded().unwrap();
    assert_eq!(seeded.end(), 2);
    assert!(seeded.text.starts_with(&before));
    assert_eq!(seeded.text.matches("user.message").count(), 1);
    assert!(store.caught_up());
    let prior_effects = lock(&effects.other).len();
    store.replay(&recovery).unwrap();
    assert_eq!(store.seeded().unwrap().end(), 2);
    assert_eq!(
        lock(&effects.other).len(),
        prior_effects,
        "storage replay cannot react"
    );
    let (live, effects) = pump(&store);
    let revised = answer("two", "Final answer");
    assert_eq!(
        store.disposition(&revised).unwrap(),
        NativeDisposition::Revision
    );
    live.revise_committed(
        &revised.frame.as_ref().unwrap().to_string(),
        |line, offset| store.commit_record(&revised, Some(line), offset),
    )
    .unwrap();
    assert_eq!(lock(&effects.events).len(), 1);
    assert!(lock(&effects.other).is_empty());
    assert!(!live.output.turn.load(Ordering::Relaxed));
    assert_eq!(store.seeded().unwrap().end(), 3);
    assert!(store.load().unwrap().starts_with(&seeded.text));
}

#[test]
fn journal_store_repeated_prompts_restore_fifo_receipts_without_global_text_matching() {
    let fixture = Fixture::new();
    let store = fixture.open(None, true);
    store.bind_native("native").unwrap();
    let (live, _) = pump(&store);
    send(&store, &live, "first", "Repeat");
    let first = batch("root-one", vec![user("one", "Repeat")]);
    store.replay(&first).unwrap();
    send(&store, &live, "second", "Repeat");
    drop(live);
    drop(store);
    let store = fixture.open(Some("native"), false);
    let both = batch(
        "root-two",
        vec![user("one", "Repeat"), user("two", "Repeat")],
    );
    store.replay(&both).unwrap();
    assert_eq!(store.load().unwrap().matches("user.message").count(), 2);
    assert!(store.caught_up());
    let writer = lock(&store.inner);
    let one = writer
        .journal
        .lookup("native-user:one", "user-version:one")
        .unwrap()
        .unwrap();
    let two = writer
        .journal
        .lookup("native-user:two", "user-version:two")
        .unwrap()
        .unwrap();
    assert_ne!(one.position, two.position);
}

#[test]
fn journal_store_failed_commit_never_publishes_captures_or_marks_the_sent_input_busy() {
    let fixture = Fixture::new();
    let store = fixture.open(None, true);
    let (live, effects) = pump(&store);
    let submission = store.submission("first", "Task", "Task").unwrap();
    std::fs::write(fixture.root.join("canonical.jsonl"), "foreign tail\n").unwrap();
    let mut sends = 0;
    let mut input = |_: &Value, _: &str| {
        sends += 1;
        Ok(vec![])
    };
    assert!(live
        .capture_committed((), || live.command_committed(
            &json!({"v":1,"type":"message.send","text":"Task"}),
            &mut input,
            |lines, offset| store.commit_submission(&submission, lines, offset)
        ))
        .is_err());
    assert_eq!(sends, 1);
    assert!(!live.output.turn.load(Ordering::Relaxed));
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    assert_eq!(store.load().unwrap(), "foreign tail\n");
}

#[test]
fn journal_store_causal_mismatch_and_unsupported_content_stop_recovery_without_rewriting() {
    let fixture = Fixture::new();
    let store = fixture.open(None, true);
    let (live, _) = pump(&store);
    send(&store, &live, "first", "Expected");
    store.bind_native("native").unwrap();
    let before = store.load().unwrap();
    assert!(store
        .replay(&batch("wrong", vec![user("other", "Different")]))
        .is_err());
    assert_eq!(store.load().unwrap(), before);
    assert!(!store.caught_up());
    let mut unsupported = checkpoint();
    unsupported.checkpoint["kind"] = json!("unsupported_tool");
    assert!(store
        .replay(&batch("unsupported", vec![unsupported]))
        .is_err());
    assert_eq!(store.load().unwrap(), before);
}

#[test]
fn journal_store_resume_recovers_identity_committed_before_the_board_and_rejects_fresh_adoption() {
    use prometeu_core::session::launch::{LaunchRequest, StartMode};
    let fixture = Fixture::new();
    let store = fixture.open(None, true);
    let (live, _) = pump(&store);
    send(&store, &live, "first", "Task");
    let mut request = LaunchRequest {
        session: "app".into(),
        workspace: "workspace".into(),
        worktree: "/worktree".into(),
        settings: prometeu_core::session::launch::Launch {
            agent: crate::state::ProviderId::Cursor,
            ..Default::default()
        },
        mode: StartMode::Resume {
            provider_session: None,
        },
    };
    let before = store.load().unwrap();
    assert!(resume_request(&fixture.root, &request).is_err());
    assert_eq!(store.load().unwrap(), before);
    store.bind_native("native").unwrap();
    let recovered = resume_request(&fixture.root, &request).unwrap();
    assert!(
        matches!(recovered.mode,StartMode::Resume {provider_session:Some(ref id)} if id=="native")
    );
    assert_eq!(store.load().unwrap(), before);
    request.mode = StartMode::Fresh;
    assert!(matches!(
        resume_request(&fixture.root, &request).unwrap().mode,
        StartMode::Fresh
    ));
    assert!(JournalStore::open(
        &fixture.root.join("canonical.jsonl"),
        &journal_path(&fixture.root, "app").unwrap(),
        JournalBinding {
            app_session: "app".into(),
            provider: "cursor".into(),
            native_session: Some("native".into())
        },
        true
    )
    .is_err());
    request.mode = StartMode::Resume {
        provider_session: Some("different".into()),
    };
    assert!(resume_request(&fixture.root, &request).is_err());
    request.mode = StartMode::Resume {
        provider_session: None,
    };
    request.settings.agent = crate::state::ProviderId::Claude;
    assert!(resume_request(&fixture.root, &request).is_err());
}

#[test]
fn journal_store_tool_result_revisions_keep_native_identity_and_do_not_repeat_execution() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let tool = NativeCapture {
        occurrence: "native-answer:step:0:tool".into(),
        version: "step-one".into(),
        checkpoint: json!({"kind":"tool","messageId":"native-answer","index":0,"toolId":"tool-one"}),
        frame: Some(
            json!({"v":1,"type":"assistant.block","at":2,"messageId":"native-answer","index":0,"block":{"kind":"tool","id":"tool-one","name":"shell","input":{"command":"pwd"}}}),
        ),
        input_binding: None,
    };
    let mut result = NativeCapture {
        occurrence: "native-answer:step:0:result".into(),
        version: "step-one".into(),
        checkpoint: json!({"kind":"result","messageId":"native-answer","index":0,"toolId":"tool-one"}),
        frame: Some(
            json!({"v":1,"type":"tool.completed","at":3,"toolId":"tool-one","output":"/work","error":false}),
        ),
        input_binding: None,
    };
    store
        .replay(&batch(
            "first",
            vec![user("first", "Task"), tool.clone(), checkpoint()],
        ))
        .unwrap();
    let before = store.load().unwrap();
    let (live, effects) = pump(&store);
    let current_user = user("second", "Next task");
    live.accept_committed(current_user.frame.as_ref().unwrap(), |line, offset| {
        store.commit_record(&current_user, Some(line), offset)
    })
    .unwrap();
    assert!(live.output.turn.load(Ordering::Relaxed));
    lock(&effects.other).clear();
    lock(&effects.events).clear();
    assert!(matches!(
        store.disposition(&result).unwrap(),
        NativeDisposition::Revision
    ));
    live.revise_committed(
        &result.frame.as_ref().unwrap().to_string(),
        |line, offset| store.commit_record(&result, Some(line), offset),
    )
    .unwrap();
    assert!(lock(&effects.other).is_empty());
    assert!(live.output.turn.load(Ordering::Relaxed));
    lock(&effects.events).clear();
    result.version = "step-two".into();
    result.frame.as_mut().unwrap()["output"] = json!("/work/project");
    assert!(matches!(
        store.disposition(&result).unwrap(),
        NativeDisposition::Revision
    ));
    live.revise_committed(
        &result.frame.as_ref().unwrap().to_string(),
        |line, offset| store.commit_record(&result, Some(line), offset),
    )
    .unwrap();
    assert!(lock(&effects.other).is_empty());
    assert_eq!(lock(&effects.events).len(), 1);
    assert!(live.output.turn.load(Ordering::Relaxed));
    assert!(store.load().unwrap().starts_with(&before));
    assert!(matches!(
        store.disposition(&result).unwrap(),
        NativeDisposition::Duplicate
    ));
    let mut wrong = tool;
    wrong.version = "step-three".into();
    wrong.frame.as_mut().unwrap()["block"]["id"] = json!("other-tool");
    assert!(store.disposition(&wrong).is_err());
    assert!(store
        .disposition(&answer("unproven", "New historical text"))
        .is_err());
}

#[test]
fn journal_store_generic_output_failure_revokes_readiness_and_latches_without_publication() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    assert!(store.caught_up());
    std::fs::write(fixture.root.join("canonical.jsonl"), b"foreign\n").unwrap();
    live.feed(r#"{"v":1,"type":"system.notice","at":1,"level":"info","code":"first"}"#);
    assert!(!store.healthy());
    assert!(!store.caught_up());
    assert!(lock(&effects.events).is_empty());
    assert_eq!(*lock(&effects.other), ["storage_error"]);
    // Even restoration of the old length cannot turn the next occurrence into permission to
    // skip the lost one. Only an explicit process restart can reopen the private journal.
    std::fs::write(fixture.root.join("canonical.jsonl"), b"").unwrap();
    live.feed(r#"{"v":1,"type":"system.notice","at":2,"level":"info","code":"second"}"#);
    assert!(store.load().unwrap().is_empty());
    assert!(lock(&effects.events).is_empty());
    assert_eq!(*lock(&effects.other), ["storage_error", "storage_error"]);
}

#[test]
fn journal_store_completion_commits_source_receipt_before_settling_and_rejects_old_tickets() {
    use prometeu_protocols::terminal_execution::{
        ExecutionFact, ExecutionProjector, TranscriptBarrier,
    };
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let mut execution = ExecutionProjector::new("process").unwrap();
    execution.accepted_input();
    execution.observe_fact(
        ExecutionFact::Working {
            id: None,
            detail: None,
        },
        1,
    );
    let ticket = execution
        .observe_fact(
            ExecutionFact::Finished {
                id: None,
                failed: false,
                detail: None,
            },
            2,
        )
        .completion
        .unwrap();
    let frame = execution
        .preview_completion(&ticket, TranscriptBarrier::CaughtUp, 3)
        .unwrap();
    assert_eq!(execution.pending_completion().as_ref(), Some(&ticket));
    assert!(live
        .feed_committed(&frame.to_string(), |line, offset| {
            let receipt = store.commit_completion("uuid:first-answer", line, offset)?;
            assert!(store.load().unwrap().contains("turn.completed"));
            assert!(lock(&effects.events).is_empty());
            execution.commit_completion(&ticket, TranscriptBarrier::CaughtUp, 3);
            Ok(receipt)
        })
        .unwrap());
    assert!(execution.pending_completion().is_none());
    assert_eq!(store.load().unwrap().lines().count(), 1);
    lock(&effects.events).clear();
    lock(&effects.other).clear();
    assert!(!live
        .feed_committed(&frame.to_string(), |line, offset| store.commit_completion(
            "uuid:first-answer",
            line,
            offset
        ))
        .unwrap());
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    execution.accepted_input();
    assert!(execution
        .preview_completion(&ticket, TranscriptBarrier::CaughtUp, 4)
        .is_none());

    execution.observe_fact(
        ExecutionFact::Working {
            id: None,
            detail: None,
        },
        4,
    );
    let next = execution
        .observe_fact(
            ExecutionFact::Finished {
                id: None,
                failed: false,
                detail: None,
            },
            5,
        )
        .completion
        .unwrap();
    let frame = execution
        .preview_completion(&next, TranscriptBarrier::CaughtUp, 6)
        .unwrap();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(fixture.root.join("canonical.jsonl"))
        .unwrap()
        .write_all(b"foreign\n")
        .unwrap();
    assert!(live
        .feed_committed(&frame.to_string(), |line, offset| {
            let receipt = store.commit_completion("uuid:second-answer", line, offset)?;
            execution.commit_completion(&next, TranscriptBarrier::CaughtUp, 6);
            Ok(receipt)
        })
        .is_err());
    assert_eq!(execution.pending_completion().as_ref(), Some(&next));
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
}

#[test]
fn journal_store_prepared_input_reconciles_only_causal_native_acceptance_after_a_crash() {
    let fixture = Fixture::new();
    let original = "Task\twith\r\noriginal spacing";
    let normalized = crate::native_ui_readiness::normalize_cursor_submission(original);
    {
        let store = fixture.open(Some("native"), true);
        store
            .submission("dispatch-one", original, &normalized)
            .unwrap();
        assert!(!store.caught_up());
        assert!(store.load().unwrap().is_empty());
    }
    let recovered = fixture.open(Some("native"), false);
    assert!(recovered
        .submission("dispatch-one", original, &normalized)
        .is_err());
    assert!(recovered
        .submission("another-id", original, &normalized)
        .is_err());
    recovered.replay(&batch("empty", vec![])).unwrap();
    assert!(!recovered.caught_up());
    assert!(!recovered
        .accepted_submission("dispatch-one", original)
        .unwrap());
    recovered
        .replay(&batch(
            "accepted",
            vec![
                user("native-one", &normalized),
                answer("answer-one", "Done"),
                checkpoint(),
            ],
        ))
        .unwrap();
    assert!(recovered.caught_up());
    assert!(recovered
        .accepted_submission("dispatch-one", original)
        .unwrap());
    assert!(recovered
        .accepted_submission("dispatch-one", "Different input")
        .is_err());
    let history = recovered.load().unwrap();
    let rows = history
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        rows.iter()
            .filter(|frame| frame["type"] == "user.message")
            .count(),
        1
    );
    assert_eq!(rows[0]["content"][0]["text"], original);
    recovered
        .replay(&batch(
            "again",
            vec![
                user("native-one", &normalized),
                answer("answer-one", "Done"),
                checkpoint(),
            ],
        ))
        .unwrap();
    assert_eq!(recovered.load().unwrap(), history);
}

#[test]
fn journal_store_proven_unwritten_input_can_retry_while_an_uncertain_input_cannot() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), true);
    store.submission("dispatch-one", "Task", "Task").unwrap();
    assert!(store.submission("dispatch-one", "Task", "Task").is_err());
    store.cancel_unwritten_submission("dispatch-one").unwrap();
    let next = store.submission("dispatch-one", "Task", "Task").unwrap();
    let (live, _) = pump(&store);
    let mut input = |_: &Value, _: &str| Ok(vec![]);
    live.command_committed(
        &json!({"v":1,"type":"message.send","text":"Task"}),
        &mut input,
        |lines, offset| store.commit_submission(&next, lines, offset),
    )
    .unwrap();
    assert!(store.accepted_submission("dispatch-one", "Task").unwrap());
    assert!(store.cancel_unwritten_submission("dispatch-one").is_err());
}

#[test]
fn journal_store_incremental_batches_preserve_the_native_user_cursor_across_revisions() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    store
        .replay(&batch(
            "baseline",
            vec![user("one", "Repeat"), answer("first", "Done"), checkpoint()],
        ))
        .unwrap();
    let (live, effects) = pump(&store);
    send(&store, &live, "dispatch-two", "Repeat");
    lock(&effects.other).clear();
    lock(&effects.events).clear();
    let next = user("two", "Repeat");
    let mut stream = batch("stream-revision", vec![next.clone()]);
    stream.phase = TranscriptPhase::Live;
    store.begin_stream_batch(&stream).unwrap();
    let events = live
        .accept_committed(next.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(&next, Some(line), offset)
        })
        .unwrap();
    assert!(events.is_empty());
    store.finish_batch(&stream).unwrap();
    assert!(store.caught_up());
    assert_eq!(
        store
            .load()
            .unwrap()
            .lines()
            .filter(|line| line.contains("user.message"))
            .count(),
        2
    );
    assert!(lock(&effects.other).is_empty());
    assert!(lock(&effects.events).is_empty());
}

#[test]
fn cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape() {
    use prometeu_core::board::Status;
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let (mut transport, _) = super::super::tests::fixture();
    let (version, busy) = crate::terminal_agent_controls::test_snapshot(include_str!(
        "../../../terminal_agent_controls/fixtures/cursor-busy.json"
    ));
    let inner = Arc::get_mut(&mut transport).unwrap();
    inner.provider = crate::state::ProviderId::Cursor;
    inner.controls = Some(
        crate::terminal_agent_controls::NativeControls::verified(inner.provider, &version).unwrap(),
    );
    inner.activity = Some(live.output.clone());
    inner.journal = Some(store.clone());
    let capture = |record: &NativeCapture| {
        live.accept_committed(record.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(record, Some(line), offset)
        })
        .unwrap();
        transport.captured_input(&live);
    };

    transport
        .write_user_bytes(b"\x1b[200~A pasted line\r\x1b[201~")
        .unwrap();
    transport.observe_cursor_activity(&busy, 1, &live);
    assert!(
        !transport.busy(),
        "pasted newlines do not submit a native request"
    );
    transport.write_user_bytes(b"\r").unwrap();
    assert!(!transport.busy(), "Enter alone is not activity evidence");
    transport.observe_cursor_activity(&busy, 0, &live);
    assert!(!transport.busy(), "the pre-input busy paint is stale");
    let mut partial = busy.clone();
    partial.pending_escape = "\x1b[".into();
    transport.observe_cursor_activity(&partial, 1, &live);
    assert!(
        !transport.busy(),
        "an incomplete paint cannot establish activity"
    );
    let (_, empty) = crate::terminal_agent_controls::test_snapshot(include_str!(
        "../../../native_ui_readiness/fixtures/cursor-fresh-rest.json"
    ));
    transport.observe_cursor_activity(&empty, 1, &live);
    assert!(
        !transport.busy(),
        "intermediate empty paint is not activity"
    );
    transport.output.feed(b"native busy repaint");
    transport.observe_cursor_activity(&busy, 1, &live);
    assert!(transport.busy());
    assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
    let first_activity: Value =
        serde_json::from_str(lock(&effects.events).last().unwrap()).unwrap();
    assert_eq!(first_activity["state"], "working");
    assert!(
        transport.cursor_completion_input(true).is_none(),
        "old source cannot settle an unbound new native input"
    );
    assert!(
        !store.load().unwrap().contains("user.message"),
        "activity does not invent transcript input"
    );
    transport.observe_cursor_activity(&busy, 2, &live);
    assert_eq!(
        lock(&effects.events).len(),
        1,
        "one start per native submission"
    );

    capture(&user("first", "Inspect a file"));
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-source-1",
        &transport,
        Some("request-1"),
        || transport.cursor_completion_input(true),
    )
    .unwrap());
    assert!(!transport.busy());
    assert!(matches!(*lock(&effects.status), Some(Status::Pronta)));
    transport.observe_cursor_activity(&busy, 2, &live);
    assert!(
        !transport.busy(),
        "old busy output cannot revive a cancelled request"
    );

    // This native retry has no PromptSubmitted hook; its screen and source still prove work.
    transport.write_user_bytes(b"\r").unwrap();
    transport.observe_cursor_activity(&busy, 1, &live);
    assert!(
        !transport.busy(),
        "retry still needs a newer native busy paint"
    );
    transport.output.feed(b"retry busy repaint");
    transport.observe_cursor_activity(&busy, 2, &live);
    assert!(transport.busy());
    assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
    let retry_activity: Value =
        serde_json::from_str(lock(&effects.events).last().unwrap()).unwrap();
    assert_ne!(retry_activity["activityId"], first_activity["activityId"]);
    capture(&user("retry", "Inspect a file"));
    let next = lock(&transport.execution).observe_fact(
        prometeu_protocols::terminal_execution::ExecutionFact::Blocked {
            id: None,
            kind: None,
            detail: None,
        },
        50,
    );
    assert_eq!(
        next.events[0]["activityId"], retry_activity["activityId"],
        "source capture claims the provisional cycle"
    );
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-source-2",
        &transport,
        None,
        || Some(()),
    )
    .unwrap());
    assert!(!transport.busy());
    transport.observe_cursor_activity(&busy, 3, &live);
    assert!(
        !transport.busy(),
        "old busy output cannot revive a successful request"
    );
}

#[test]
fn journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet() {
    use prometeu_core::board::Status;
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let (mut transport, bytes) = super::super::tests::fixture();
    let inner = Arc::get_mut(&mut transport).unwrap();
    inner.provider = crate::state::ProviderId::Cursor;
    inner.activity = Some(live.output.clone());
    inner.journal = Some(store.clone());
    let first = user("first", "Inspect the uploaded file");
    live.accept_committed(first.frame.as_ref().unwrap(), |line, offset| {
        store.commit_record(&first, Some(line), offset)
    })
    .unwrap();
    transport.captured_input(&live);
    let source_answer = |version, text, request| {
        let mut record = answer(version, text);
        record.checkpoint["userMessageId"] = json!("first");
        record.checkpoint["requestId"] = json!(request);
        record
    };
    transport
        .feed_native_output(&live, &store, &source_answer("one", "Reading", "request-1"))
        .unwrap();
    assert!(transport.busy());
    assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
    transport.write_user_bytes(b"\x1b").unwrap();
    assert_eq!(*lock(&bytes), b"\x1b");
    assert!(
        transport.busy(),
        "Escape alone cannot establish cancellation"
    );
    assert!(lock(&transport.input).local_draft.is_some());
    assert!(transport.cursor_completion_input(false).is_none());
    transport
        .begin_submission("next-app-input", "A newer task")
        .unwrap();
    assert!(
        !super::super::commit_cursor_completion(
            &live,
            &store,
            "cursor-source-1",
            &transport,
            Some("request-1"),
            || transport.cursor_completion_input(true),
        )
        .unwrap(),
        "pending app input fences the old request"
    );
    assert!(transport.busy());
    transport.end_submission("next-app-input");

    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-source-1",
        &transport,
        Some("request-1"),
        || transport.cursor_completion_input(true),
    )
    .unwrap());
    assert!(!transport.busy());
    assert!(matches!(*lock(&effects.status), Some(Status::Pronta)));
    assert!(
        lock(&transport.input).local_draft.is_some(),
        "the native draft is preserved"
    );
    lock(&effects.other).clear();
    for version in ["late", "late-revision"] {
        transport
            .feed_native_output(
                &live,
                &store,
                &source_answer(version, "Partial result", "request-1"),
            )
            .unwrap();
        assert!(!transport.busy());
    }
    let mut hook: crate::terminal_agent_hooks::NativeHookEvent = serde_json::from_value(json!({
        "provider":"cursor", "session":"app", "process_nonce":"incarnation", "native_session":"native",
        "kind":"tool_finished", "generation_id":"request-1"
    })).unwrap();
    transport.observe_hook(&hook, &live);
    assert!(!transport.busy());
    assert!(
        lock(&effects.other).is_empty(),
        "late content and hooks have no live effects"
    );
    assert!(store.load().unwrap().contains("Partial result"));
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-source-1",
        &transport,
        None,
        || Some(())
    )
    .unwrap());
    let completions: Vec<Value> = store
        .load()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|frame| frame["type"] == "turn.completed")
        .collect();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0]["outcome"], "interrupted");
    assert!(
        lock(&effects.other).is_empty(),
        "a duplicate success receipt cannot replace cancellation"
    );

    let next = user("next", "Continue with another file");
    live.accept_committed(next.frame.as_ref().unwrap(), |line, offset| {
        store.commit_record(&next, Some(line), offset)
    })
    .unwrap();
    transport.captured_input(&live);
    hook.generation_id = Some("request-2".into());
    transport.observe_hook(&hook, &live);
    assert!(transport.busy(), "new request activity remains live");
    assert!(matches!(*lock(&effects.status), Some(Status::Rodando)));
}

#[test]
fn journal_cursor_completion_commits_before_publication_and_ignores_duplicates_or_stale_composer() {
    use prometeu_protocols::terminal_execution::ExecutionFact;
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let (transport, _) = super::super::tests::fixture();
    let execution = &transport.execution;
    lock(execution).accepted_input();
    lock(execution).observe_fact(
        ExecutionFact::Working {
            id: None,
            detail: None,
        },
        1,
    );
    assert!(!super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-request-1",
        &transport,
        None,
        || None::<()>
    )
    .unwrap());
    assert!(store.load().unwrap().is_empty());
    assert!(lock(&effects.events).is_empty());
    assert!(lock(execution).snapshot().has_active_work());
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-request-1",
        &transport,
        None,
        || Some(())
    )
    .unwrap());
    assert_eq!(store.load().unwrap().lines().count(), 1);
    assert_eq!(
        lock(&effects.events)
            .iter()
            .filter(|line| line.contains("turn.completed"))
            .count(),
        1
    );
    assert!(!lock(execution).snapshot().has_active_work());
    lock(&effects.events).clear();
    lock(&effects.other).clear();
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-request-1",
        &transport,
        None,
        || Some(())
    )
    .unwrap());
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    assert_eq!(store.load().unwrap().lines().count(), 1);
}

#[test]
fn journal_cursor_completion_failure_keeps_execution_active_and_publishes_nothing() {
    use prometeu_protocols::terminal_execution::ExecutionFact;
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let (transport, _) = super::super::tests::fixture();
    let execution = &transport.execution;
    lock(execution).accepted_input();
    lock(execution).observe_fact(
        ExecutionFact::Working {
            id: None,
            detail: None,
        },
        1,
    );
    std::fs::write(fixture.root.join("canonical.jsonl"), "foreign tail\n").unwrap();
    assert!(super::super::commit_cursor_completion(
        &live,
        &store,
        "cursor-request-1",
        &transport,
        None,
        || Some(())
    )
    .is_err());
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    assert!(lock(execution).snapshot().has_active_work());
    assert_eq!(store.load().unwrap(), "foreign tail\n");
}

fn metadata(kind: &str, version: &str, text: &str) -> NativeCapture {
    NativeCapture {
        occurrence: format!("native-answer:metadata:{kind}:0"),
        version: version.into(),
        checkpoint: json!({"kind":"metadata","messageId":"native-answer","index":0,"frameType":kind}),
        frame: Some(json!({"v":1,"type":kind,"at":1,"text":text,"used":12,"window":100})),
        input_binding: None,
    }
}

#[test]
fn sealed_source_rows_keep_live_tool_effects_until_their_causal_user_completes() {
    for completed in [false, true] {
        let fixture = Fixture::new();
        let store = fixture.open(Some("native"), false);
        let (live, effects) = pump(&store);
        let first = user("first", "Task");
        live.accept_committed(first.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(&first, Some(line), offset)
        })
        .unwrap();
        let tool = |index| NativeCapture {
            occurrence: format!("row:block:{index}:tool"),
            version: "sealed-row".into(),
            checkpoint: json!({"kind":"tool","messageId":"row","index":index,"toolId":format!("call-{index}"),"sourceScope":"record","sourceUser":first.occurrence}),
            frame: Some(
                json!({"v":1,"type":"assistant.block","at":2,"messageId":"row","index":index,"block":{"kind":"tool","id":format!("call-{index}"),"name":"Read","input":{"file_path":"fixture"}}}),
            ),
            input_binding: None,
        };
        for index in 0..2 {
            let record = tool(index);
            assert_eq!(
                store.disposition(&record).unwrap(),
                NativeDisposition::Output
            );
            live.feed_committed(
                &record.frame.as_ref().unwrap().to_string(),
                |line, offset| store.commit_record(&record, Some(line), offset),
            )
            .unwrap();
        }
        store.commit_checkpoint(&NativeCapture {
        occurrence:"row:checkpoint".into(),version:"sealed-row".into(),
        checkpoint:json!({"kind":"turn","messageId":"row","status":"completed","sourceScope":"record","sourceUser":first.occurrence}),
        frame:None,input_binding:None,
    }).unwrap();
        let result = |index| NativeCapture {
            occurrence: format!("row:block:{index}:result"),
            version: "result-one".into(),
            checkpoint: json!({"kind":"result","messageId":"row","index":index,"toolId":format!("call-{index}"),"sourceScope":"record","sourceUser":first.occurrence}),
            frame: Some(
                json!({"v":1,"type":"tool.completed","at":3,"toolId":format!("call-{index}"),"output":"Contents","error":false,"background":false}),
            ),
            input_binding: None,
        };
        let current = result(0);
        assert_eq!(
            store.disposition(&current).unwrap(),
            NativeDisposition::Output
        );
        lock(&effects.other).clear();
        live.feed_committed(
            &current.frame.as_ref().unwrap().to_string(),
            |line, offset| store.commit_record(&current, Some(line), offset),
        )
        .unwrap();
        assert!(lock(&effects.other).contains(&"react"));
        let completion = json!({"v":1,"type":"turn.completed","at":4,"outcome":"ok","message":"","durationMs":null,"costUsd":null});
        if completed {
            live.feed_committed(&completion.to_string(), |line, offset| {
                store.commit_completion("native-finish-one", line, offset)
            })
            .unwrap();
        }
        let mut captured = batch("captured-first", vec![]);
        captured.phase = TranscriptPhase::Live;
        store.begin_stream_batch(&captured).unwrap();
        store.finish_batch(&captured).unwrap();
        send(&store, &live, "next-send", "Next task");
        assert_eq!(
            store.disposition(&result(1)).unwrap(),
            NativeDisposition::Revision
        );
        let second = user("second", "Next task");
        live.accept_committed(second.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(&second, Some(line), offset)
        })
        .unwrap();
        // A snapshot replay cursor visiting an old user must not revive its current-turn effects.
        let old = batch("old-snapshot", vec![first.clone()]);
        store.begin_batch(&old).unwrap();
        store.commit_checkpoint(&first).unwrap();
        lock(&effects.other).clear();
        let mut late = result(1);
        assert_eq!(
            store.disposition(&late).unwrap(),
            NativeDisposition::Revision
        );
        live.revise_committed(&late.frame.as_ref().unwrap().to_string(), |line, offset| {
            store.commit_record(&late, Some(line), offset)
        })
        .unwrap();
        assert!(lock(&effects.other).is_empty());
        assert!(live.output.turn.load(Ordering::Relaxed));
        drop(live);
        drop(store);
        let reopened = fixture.open(Some("native"), false);
        late.version = "result-two".into();
        late.frame.as_mut().unwrap()["output"] = json!("Revised contents");
        assert_eq!(
            reopened.disposition(&late).unwrap(),
            NativeDisposition::Revision
        );
        late.checkpoint["sourceUser"] = json!(second.occurrence);
        assert!(reopened.disposition(&late).is_err());
    }
}

#[test]
fn journal_metadata_replay_and_late_revisions_do_not_repeat_capture_or_replace_live_context() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let initial = batch(
        "root-first",
        vec![
            user("one", "Task"),
            metadata("context.updated", "context-1", ""),
            metadata("telemetry.usage", "usage-1", ""),
            metadata("system.summary", "summary-1", "Original summary"),
            checkpoint(),
        ],
    );
    store.replay(&initial).unwrap();
    let saved = store.load().unwrap();
    assert_eq!(saved.lines().count(), 2);
    assert!(!saved.contains("context.updated"));
    assert!(!saved.contains("telemetry.usage"));
    let (live, effects) = pump(&store);
    store.replay(&initial).unwrap();
    assert_eq!(store.load().unwrap(), saved);
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());

    for kind in ["context.updated", "telemetry.usage", "commands.updated"] {
        let revised = metadata(kind, "revised", "Late metadata");
        assert_eq!(
            store.disposition(&revised).unwrap(),
            NativeDisposition::Revision
        );
        store.commit_checkpoint(&revised).unwrap();
        assert_eq!(
            store.disposition(&revised).unwrap(),
            NativeDisposition::Duplicate
        );
    }
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    let summary = metadata("system.summary", "summary-2", "Revised historical summary");
    assert_eq!(
        store.disposition(&summary).unwrap(),
        NativeDisposition::Revision
    );
    live.revise_committed(
        &summary.frame.as_ref().unwrap().to_string(),
        |line, offset| store.commit_record(&summary, Some(line), offset),
    )
    .unwrap();
    assert_eq!(store.load().unwrap().lines().count(), 3);
    assert!(store.load().unwrap().contains("Revised historical summary"));
    assert_eq!(lock(&effects.events).len(), 1);
    assert!(lock(&effects.other).is_empty());
}

#[test]
fn native_compaction_without_a_model_turn_replays_once_and_revises_without_effects() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let capture = |version: &str, after: u64| NativeCapture {
        occurrence: "native:claude:session:boundary:metadata:context.compacted:0".into(),
        version: version.into(),
        checkpoint: json!({"kind":"metadata","messageId":"native:claude:session:boundary","index":0,"frameType":"context.compacted","sourceScope":"record","sourceUser":null}),
        frame: Some(
            json!({"v":1,"type":"context.compacted","at":1,"before":12000,"after":after,"trigger":"manual"}),
        ),
        input_binding: None,
    };
    let first = capture("original", 2000);
    let initial = batch("compact", vec![first.clone()]);
    store.replay(&initial).unwrap();
    let saved = store.load().unwrap();
    assert_eq!(saved.lines().count(), 1);
    assert!(saved.contains("context.compacted"));
    let (live, effects) = pump(&store);
    store.replay(&initial).unwrap();
    assert_eq!(store.load().unwrap(), saved);
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).is_empty());
    let revision = capture("revised", 1900);
    assert_eq!(
        store.disposition(&revision).unwrap(),
        NativeDisposition::Revision
    );
    live.revise_committed(
        &revision.frame.as_ref().unwrap().to_string(),
        |line, offset| store.commit_record(&revision, Some(line), offset),
    )
    .unwrap();
    assert_eq!(store.load().unwrap().lines().count(), 2);
    assert_eq!(lock(&effects.events).len(), 1);
    assert!(lock(&effects.other).is_empty());
    assert_eq!(
        store.disposition(&revision).unwrap(),
        NativeDisposition::Duplicate
    );
    let mut status = capture("status", 1900);
    status.occurrence = "native:claude:session:status:metadata:context.compaction:0".into();
    status.checkpoint["frameType"] = json!("context.compaction");
    status.frame =
        Some(json!({"v":1,"type":"context.compaction","at":1,"state":"stopped","detail":""}));
    assert_eq!(
        store.disposition(&status).unwrap(),
        NativeDisposition::Revision
    );
    store.commit_checkpoint(&status).unwrap();
    assert_eq!(store.load().unwrap().lines().count(), 2);
    assert!(lock(&effects.other).is_empty());
}

#[test]
fn record_scope_without_proven_user_cannot_change_live_work() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let mut unknown = answer("first", "Unattributed native text");
    unknown.checkpoint["sourceScope"] = json!("record");
    unknown.checkpoint["sourceUser"] = Value::Null;
    assert!(store.disposition(&unknown).is_err());
    unknown.checkpoint["sourceUser"] = json!("native-user:missing");
    assert!(store.disposition(&unknown).is_err());
    unknown.checkpoint["sourceUser"] = Value::Null;
    store
        .replay(&batch("recovery", vec![unknown.clone()]))
        .unwrap();
    unknown.version = "revision".into();
    assert_eq!(
        store.disposition(&unknown).unwrap(),
        NativeDisposition::Revision
    );
    let mut notice = metadata("system.notice", "session-start", "Session metadata");
    notice.checkpoint["sourceScope"] = json!("record");
    assert_eq!(
        store.disposition(&notice).unwrap(),
        NativeDisposition::Revision
    );
}

#[test]
fn source_trajectory_binding_survives_restart_and_precedes_duplicate_or_new_step_acceptance() {
    let fixture = Fixture::new();
    let mut captured = user("first", "Task");
    captured.checkpoint["trajectoryId"] = json!("trajectory-one");
    let before = {
        let store = fixture.open(Some("native"), false);
        store
            .replay(&batch("first", vec![captured.clone()]))
            .unwrap();
        store.load().unwrap()
    };
    let store = fixture.open(Some("native"), false);
    assert_eq!(
        store.disposition(&captured).unwrap(),
        NativeDisposition::Duplicate
    );
    captured.checkpoint["trajectoryId"] = json!("trajectory-two");
    assert!(store.disposition(&captured).is_err());
    assert!(store.commit_checkpoint(&captured).is_err());
    captured
        .checkpoint
        .as_object_mut()
        .unwrap()
        .remove("trajectoryId");
    assert!(store.disposition(&captured).is_err());
    let mut next = user("next", "Next task");
    next.checkpoint["trajectoryId"] = json!("trajectory-two");
    assert!(store.disposition(&next).is_err());
    assert!(store.replay(&batch("changed", vec![next])).is_err());
    assert_eq!(store.load().unwrap(), before);
}

#[test]
fn journal_ephemeral_source_receipt_is_required_before_any_live_effect() {
    let fixture = Fixture::new();
    let store = fixture.open(Some("native"), false);
    let (live, effects) = pump(&store);
    let usage = metadata("telemetry.usage", "first", "");
    let frame = usage.frame.as_ref().unwrap().to_string();
    assert!(live
        .feed_observed_committed(&frame, |line, offset| {
            assert!(line.is_none());
            let outcome = store.commit_record(&usage, line, offset)?;
            assert_eq!(store.disposition(&usage)?, NativeDisposition::Duplicate);
            assert!(lock(&effects.other).is_empty());
            Ok(outcome)
        })
        .unwrap());
    assert!(store.load().unwrap().is_empty());
    assert!(lock(&effects.events).is_empty());
    assert!(lock(&effects.other).contains(&"telemetry"));
    lock(&effects.other).clear();
    assert!(!live
        .feed_observed_committed(&frame, |line, offset| store
            .commit_record(&usage, line, offset))
        .unwrap());
    assert!(lock(&effects.other).is_empty());
    let changed = metadata("context.updated", "second", "");
    std::fs::write(fixture.root.join("canonical.jsonl"), "foreign tail\n").unwrap();
    assert!(live
        .feed_observed_committed(
            &changed.frame.as_ref().unwrap().to_string(),
            |line, offset| store.commit_record(&changed, line, offset)
        )
        .is_err());
    assert!(lock(&effects.other).is_empty());
    assert!(lock(&effects.events).is_empty());
}

#[test]
fn journal_binds_claude_nfc_echo_to_nfd_attachment_submission() {
    // macOS hands attachment file names over as NFD; Claude Code records the prompt in NFC.
    let original = "@\"/tmp/Captura de Tela a\u{300}s 15.04.png\"\n\nHelp me";
    let echo = "@\"/tmp/Captura de Tela \u{e0}s 15.04.png\"\n\nHelp me";
    let (transport, _) = super::super::tests::fixture();
    let normalized = transport.begin_submission("attachment", original).unwrap();
    transport.end_submission("attachment");
    let native = crate::terminal_agent_observation::claude_live_user_capture(
        json!({"uuid":"prompt","type":"user","message":{"role":"user","content":echo}}),
        &normalized,
        "attachment",
    );
    assert_eq!(native.input_binding.as_deref(), Some("attachment"));
    let fixture = Fixture::new();
    let store = fixture.open_as("claude", None, true);
    let (live, _) = pump(&store);
    let submission = store
        .submission("attachment", original, &normalized)
        .unwrap();
    let mut input = |_: &Value, _: &str| Ok(vec![]);
    live.capture_committed((), || {
        live.command_committed(
            &json!({"v":1,"type":"message.send","text":original}),
            &mut input,
            |lines, offset| store.commit_submission(&submission, lines, offset),
        )
    })
    .unwrap();
    store.bind_native("session").unwrap();
    assert_eq!(store.disposition(&native).unwrap(), NativeDisposition::User);
    assert!(live
        .accept_committed(native.frame.as_ref().unwrap(), |line, offset| {
            store.commit_record(&native, Some(line), offset)
        })
        .unwrap()
        .is_empty());
    let saved = store.load().unwrap();
    assert_eq!(saved.matches("user.message").count(), 1);
    assert!(
        saved.contains("a\u{300}s 15.04.png"),
        "keeps the original spelling"
    );
}
