use super::*;
use prometeu_protocols::codex::Start;
use std::path::PathBuf;

fn attached() -> Attached {
    Attached::new(
        Link::attached(
            Box::new(std::io::sink()),
            Start {
                cwd: "/workspace".into(),
                resume: None,
                model: "fixture-model".into(),
                effort: "low".into(),
                plugin_ids: vec![],
                plugin_hook_ids: vec![],
                permission: None,
                instructions: String::new(),
            },
            "thread".into(),
            "consumer-one".into(),
            |_, en| en.into(),
        ),
        "thread",
        true,
    )
}
fn turn() -> Value {
    let proof: Value = serde_json::from_str(include_str!(
        "../../../terminal_agent_codex/fixtures/turn-identity.json"
    ))
    .unwrap();
    json!({"id":proof["rpcUser"]["turnId"],"status":"completed","items":[proof["rpcUser"]["item"],proof["rpcAssistant"]["item"]]})
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn journal() -> (Scratch, Arc<ingestion::JournalStore>) {
    let root =
        std::env::temp_dir().join(format!("prometeu-codex-consumer-{}", uuid::Uuid::new_v4()));
    let store = ingestion::JournalStore::open(
        &root.join("chat.jsonl"),
        &root.join("capture.sqlite3"),
        JournalBinding {
            app_session: "app".into(),
            provider: "codex".into(),
            native_session: Some("thread".into()),
        },
        true,
    )
    .unwrap();
    (Scratch(root), store)
}

#[test]
fn native_history_and_live_items_share_receipts_and_never_duplicate_a_user_or_answer() {
    let (_scratch, journal) = journal();
    let native = attached();
    let turn = turn();
    native.replay(&[], &journal).unwrap();
    let mut decoder = lock(&native.state);
    decoder
        .decode(&json!({"method":"turn/started","params":{"threadId":"thread","turn":turn}}))
        .unwrap();
    for item in turn["items"].as_array().unwrap() {
        for method in ["item/started", "item/completed"] {
            let decoded=decoder.decode(&json!({"method":method,"params":{"threadId":"thread","turnId":turn["id"],"item":item}})).unwrap();
            for record in decoded.into_iter().filter_map(|event| event.record) {
                let batch = batch(vec![record.clone()], TranscriptPhase::Live);
                journal.begin_stream_batch(&batch).unwrap();
                let offset = journal.load().unwrap().lines().count() as u64;
                journal
                    .commit_record(
                        &record,
                        record.frame.as_ref().map(Value::to_string).as_deref(),
                        offset,
                    )
                    .unwrap();
                journal.finish_batch(&batch).unwrap();
            }
        }
    }
    drop(decoder);
    let before = journal.load().unwrap();
    let resumed = attached();
    resumed
        .replay(std::slice::from_ref(&turn), &journal)
        .unwrap();
    let after = journal.load().unwrap();
    let values: Vec<Value> = after
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        values
            .iter()
            .filter(|frame| frame["type"] == "user.message")
            .count(),
        1
    );
    assert_eq!(
        values
            .iter()
            .filter(|frame| frame["type"] == "assistant.block")
            .count(),
        1
    );
    assert_eq!(
        values
            .iter()
            .filter(|frame| frame["type"] == "turn.completed")
            .count(),
        1
    );
    assert!(after.starts_with(&before));
    let replayed = after.clone();
    attached().replay(&[turn], &journal).unwrap();
    assert_eq!(journal.load().unwrap(), replayed);
}

#[test]
fn repeated_native_text_has_distinct_occurrences_and_completion_keeps_native_turn() {
    let native = attached();
    let mut decoder = lock(&native.state);
    let records:Vec<_>=["first","second"].into_iter().map(|id| {
        decoder.decode(&json!({"method":"item/completed","params":{"threadId":"thread","item":{"type":"userMessage","id":id,"clientId":id,"content":[{"type":"text","text":"Repeat"}]}}})).unwrap().pop().unwrap().record.unwrap()
    }).collect();
    assert_ne!(records[0].occurrence, records[1].occurrence);
    assert_eq!(records[0].version, records[1].version);
    assert_eq!(records[0].checkpoint["clientId"], "first");
    decoder.decode(&json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"turn-one"}}})).unwrap();
    let completed=decoder.decode(&json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn-one","status":"interrupted"}}})).unwrap();
    let record = completed
        .into_iter()
        .find_map(|event| event.record)
        .unwrap();
    assert_eq!(record.checkpoint["nativeTurn"], "turn-one");
    assert_eq!(record.frame.unwrap()["outcome"], "interrupted");
}

#[test]
fn historical_tool_and_result_replay_use_the_same_native_identity_as_live() {
    let (_scratch, journal) = journal();
    let native = attached();
    let turn = json!({"id":"turn","status":"completed","items":[{"id":"call-one","type":"commandExecution","command":"echo fixture","commandActions":[],"status":"completed","aggregatedOutput":"fixture","exitCode":0}]});
    native.replay(&[turn], &journal).unwrap();
    let lines: Vec<Value> = journal
        .load()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["messageId"], "call-one");
    assert_eq!(lines[0]["block"]["id"], "call-one");
    assert_eq!(lines[1]["toolId"], "call-one");
    assert_eq!(lines[1]["output"], "fixture");
}

#[test]
fn unfinished_history_never_fabricates_tool_or_turn_completion() {
    let (_scratch, journal) = journal();
    let turn = json!({"id":"turn-active","status":"inProgress","items":[{"id":"call-active","type":"commandExecution","command":"sleep 5","commandActions":[],"status":"inProgress","aggregatedOutput":"partial","exitCode":null}]});
    attached().replay(&[turn], &journal).unwrap();
    let lines: Vec<Value> = journal
        .load()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["block"]["kind"], "tool");
    assert!(lines
        .iter()
        .all(|frame| frame["type"] != "tool.completed" && frame["type"] != "turn.completed"));
    assert!(!completed_item(
        &json!({"status":"inProgress"}),
        &json!({"status":"interrupted"})
    ));
}

use prometeu_core::conversation::stream::{ConversationEvents, Lines};
use prometeu_core::session::{
    output::{ExecutionObservation, SessionOutput, SessionTelemetry},
    pump::{PendingInput, PumpDiagnostics, PumpReactions, SessionPump},
};
#[derive(Default)]
struct Effects {
    frames: Mutex<Vec<Value>>,
    reactions: Mutex<Vec<String>>,
    schedules: std::sync::atomic::AtomicUsize,
}
impl ConversationEvents for Effects {
    fn emit(&self, text: &str, _: u64) -> Result<(), String> {
        lock(&self.frames).push(serde_json::from_str(text).unwrap());
        Ok(())
    }
}
impl ExecutionObservation for Effects {
    fn observe(&self, _: &Value) {}
    fn publish(&self, _: &Value) {}
}
impl PumpReactions for Effects {
    fn react(&self, frame: &Value, _: &AtomicBool) -> bool {
        lock(&self.reactions).push(frame["type"].as_str().unwrap_or("").into());
        frame["type"] == "turn.completed"
    }
}
impl PendingInput for Effects {
    fn queued(&self) -> bool {
        true
    }
    fn setup_running(&self) -> bool {
        false
    }
    fn schedule(&self) {
        self.schedules.fetch_add(1, Ordering::Relaxed);
    }
}
impl PumpDiagnostics for Effects {
    fn storage_error(&self, error: &str) {
        panic!("unexpected storage failure: {error}")
    }
}
struct Telemetry;
impl SessionTelemetry for Telemetry {
    fn observe(&mut self, _: &Value) {}
}
type RuntimeFixture = (
    Scratch,
    Arc<ingestion::JournalStore>,
    Arc<Attached>,
    Arc<Transport>,
    SessionPump<Telemetry>,
    Arc<Effects>,
);

fn runtime(thread: &str) -> RuntimeFixture {
    let root =
        std::env::temp_dir().join(format!("prometeu-codex-runtime-{}", uuid::Uuid::new_v4()));
    let journal = ingestion::JournalStore::open(
        &root.join("chat.jsonl"),
        &root.join("capture.sqlite3"),
        JournalBinding {
            app_session: "runtime".into(),
            provider: "codex".into(),
            native_session: Some(thread.into()),
        },
        true,
    )
    .unwrap();
    let native = Arc::new(Attached::new(
        Link::attached(
            Box::new(std::io::sink()),
            Start {
                cwd: "/workspace".into(),
                resume: None,
                model: "fixture-model".into(),
                effort: "low".into(),
                plugin_ids: vec![],
                plugin_hook_ids: vec![],
                permission: None,
                instructions: String::new(),
            },
            thread.into(),
            "consumer-one".into(),
            |_, en| en.into(),
        ),
        thread,
        true,
    ));
    native.replay(&[], &journal).unwrap();
    let effects = Arc::new(Effects::default());
    let output = Arc::new(SessionOutput {
        lines: Arc::new(Mutex::new(Lines::seeded(journal.as_ref()).unwrap())),
        gone: Arc::new(AtomicBool::new(false)),
        turn: AtomicBool::new(false),
        store: journal.clone(),
        events: effects.clone(),
        execution: effects.clone(),
    });
    let pump = SessionPump::new(
        output.clone(),
        Telemetry,
        Arc::new(crate::conversation::SystemClock),
        effects.clone(),
        effects.clone(),
        effects.clone(),
    )
    .with_commit(journal.clone());
    let (mut transport, _) = crate::chat::terminal::tests::fixture();
    let fields = Arc::get_mut(&mut transport).unwrap();
    fields.provider = crate::state::ProviderId::Codex;
    fields.journal = Some(journal.clone());
    fields.codex = Some(native.clone());
    fields.activity = Some(output);
    fields.ready.store(true, Ordering::Release);
    (Scratch(root), journal, native, transport, pump, effects)
}
fn poll(
    native: &Attached,
    events: &crate::terminal_agent_codex::events::Events,
    transport: &Transport,
    pump: &SessionPump<Telemetry>,
    journal: &ingestion::JournalStore,
    effects: &Effects,
) {
    native
        .poll_with(
            events,
            transport,
            pump,
            |record| {
                let frames = pump
                    .accept_committed(record.frame.as_ref().unwrap(), |line, offset| {
                        journal.commit_record(record, Some(line), offset)
                    })?;
                Ok(!frames.is_empty())
            },
            || effects.schedule(),
        )
        .unwrap();
}

#[test]
fn runtime_consumer_waits_for_app_receipt_then_delivers_once_and_rejects_rpc_text_input() {
    let (_scratch, journal, native, transport, pump, effects) = runtime("thread");
    let turn = turn();
    let prompt = turn["items"][0]["content"][0]["text"].as_str().unwrap();
    transport.begin_submission("app-input", prompt).unwrap();
    let submission = journal.submission("app-input", prompt, prompt).unwrap();
    let (send, events) = crate::terminal_agent_codex::events::channel();
    for raw in [
        json!({"method":"turn/started","params":{"threadId":"thread","turn":turn}}),
        json!({"method":"item/completed","params":{"threadId":"thread","turnId":turn["id"],"item":turn["items"][0]}}),
    ] {
        send.try_send(Ok(raw)).unwrap();
    }
    poll(&native, &events, &transport, &pump, &journal, &effects);
    assert!(lock(&effects.frames).is_empty());
    assert!(!transport.submission_write_started("app-input"));
    let mut input = NativeInput(transport.clone());
    pump.command_committed(
        &json!({"v":1,"type":"message.send","text":prompt}),
        &mut input,
        |lines, offset| journal.commit_submission(&submission, lines, offset),
    )
    .unwrap();
    assert!(transport.submission_write_started("app-input"));
    transport.end_submission("app-input");
    assert!(native
        .send(&json!({"v":1,"type":"message.send","text":"Must stay in the TUI"}))
        .is_err());
    let item = &turn["items"][1];
    for raw in [
        json!({"method":"item/started","params":{"threadId":"thread","turnId":turn["id"],"item":item}}),
        json!({"method":"item/completed","params":{"threadId":"thread","turnId":turn["id"],"item":item}}),
        json!({"method":"turn/completed","params":{"threadId":"thread","turn":turn}}),
    ] {
        send.try_send(Ok(raw)).unwrap();
    }
    poll(&native, &events, &transport, &pump, &journal, &effects);
    let public: Vec<Value> = journal
        .load()
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        public
            .iter()
            .filter(|frame| frame["type"] == "user.message")
            .count(),
        1
    );
    assert_eq!(
        public
            .iter()
            .filter(|frame| frame["type"] == "assistant.block")
            .count(),
        1
    );
    assert_eq!(
        public
            .iter()
            .filter(|frame| frame["type"] == "turn.completed")
            .count(),
        1
    );
    assert_eq!(
        lock(&effects.reactions)
            .iter()
            .filter(|kind| kind.as_str() == "turn.completed")
            .count(),
        1
    );
    assert!(!native.active());
    assert!(journal.caught_up());
    let count = lock(&effects.frames).len();
    attached().replay(&[turn], &journal).unwrap();
    assert_eq!(lock(&effects.frames).len(), count);
}

pub(crate) fn real_proof(thread: &str, live: &[Value], history: &[Value]) -> Value {
    let (_scratch, journal, native, transport, pump, effects) = runtime(thread);
    let (send, events) = crate::terminal_agent_codex::events::channel();
    for raw in live {
        send.try_send(Ok(raw.clone())).unwrap();
        poll(&native, &events, &transport, &pump, &journal, &effects);
    }
    let before = journal.load().unwrap();
    let fresh = Attached::new(
        Link::attached(
            Box::new(std::io::sink()),
            Start {
                cwd: "/workspace".into(),
                resume: Some(thread.into()),
                model: "fixture-model".into(),
                effort: "low".into(),
                plugin_ids: vec![],
                plugin_hook_ids: vec![],
                permission: None,
                instructions: String::new(),
            },
            thread.into(),
            "consumer-one".into(),
            |_, en| en.into(),
        ),
        thread,
        true,
    );
    fresh.replay(history, &journal).unwrap();
    assert_eq!(journal.load().unwrap(), before);
    let frames: Vec<Value> = before
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let users = frames
        .iter()
        .filter(|frame| frame["type"] == "user.message")
        .count();
    let completions = frames
        .iter()
        .filter(|frame| frame["type"] == "turn.completed")
        .count();
    assert_eq!((users, completions), (1, 1));
    json!({"nativeUserCount":users,"completionCount":completions,"coldReplayUnchanged":true,"canonicalConsumerDelivered":true})
}

#[test]
fn only_typed_native_idle_or_committed_current_completion_releases_input_authority() {
    let (_scratch, journal, native, transport, pump, effects) = runtime("thread");
    let (send, events) = crate::terminal_agent_codex::events::channel();
    for status in [
        json!({"type":"active","activeFlags":["waitingOnApproval"]}),
        json!({"type":"unknown"}),
    ] {
        send.try_send(Ok(json!({"method":"thread/status/changed","params":{"threadId":"thread","status":status}}))).unwrap();
        poll(&native, &events, &transport, &pump, &journal, &effects);
        assert!(!native.native_idle());
        assert!(!transport.ready());
    }
    send.try_send(Ok(json!({"method":"thread/status/changed","params":{"threadId":"thread","status":{"type":"idle"}}}))).unwrap();
    poll(&native, &events, &transport, &pump, &journal, &effects);
    assert!(native.native_idle());
    send.try_send(Ok(
        json!({"method":"turn/started","params":{"threadId":"thread","turn":{"id":"current"}}}),
    ))
    .unwrap();
    send.try_send(Ok(json!({"method":"thread/status/changed","params":{"threadId":"thread","status":{"type":"idle"}}}))).unwrap();
    poll(&native, &events, &transport, &pump, &journal, &effects);
    assert!(!native.native_idle());
    send.try_send(Ok(json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"older","status":"completed"}}}))).unwrap();
    poll(&native, &events, &transport, &pump, &journal, &effects);
    assert!(!native.native_idle());
    send.try_send(Ok(json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"current","status":"completed"}}}))).unwrap();
    poll(&native, &events, &transport, &pump, &journal, &effects);
    assert!(native.native_idle());
    assert_eq!(
        journal
            .load()
            .unwrap()
            .lines()
            .filter(|line| serde_json::from_str::<Value>(line).unwrap()["type"] == "turn.completed")
            .count(),
        1
    );
}
