use super::*;
use rusqlite::Connection;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Mutex;

// Synthetic SQLite/protobuf fixtures follow the pinned CLI descriptors; no real chat DB is read.
struct Fixture {
    root: PathBuf,
    chats: PathBuf,
    db: Connection,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("prometeu-cursor-observer-{}", uuid::Uuid::new_v4()));
        let chats = root.join("chats");
        let session = chats.join("native-session");
        crate::paths::ensure_private_dir(&session).unwrap();
        let db = Connection::open(session.join("store.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=1; CREATE TABLE blobs (id TEXT PRIMARY KEY,data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY,value TEXT);").unwrap();
        Self { root, chats, db }
    }
    fn put(&self, bytes: &[u8]) -> [u8; 32] {
        let id: [u8; 32] = Sha256::digest(bytes).into();
        self.db
            .execute(
                "INSERT OR IGNORE INTO blobs VALUES (?1,?2)",
                rusqlite::params![cursor_store::BlobId(id).hex(), bytes],
            )
            .unwrap();
        id
    }
    fn turn(&self, id: &str, answer: &str) -> [u8; 32] {
        let user = self.put(&[field(1, b"Repeat"), field(2, id.as_bytes())].concat());
        let step = self.put(&field(1, &field(1, answer.as_bytes())));
        self.turn_steps(id, user, &[step])
    }
    fn turn_steps(&self, id: &str, user: [u8; 32], steps: &[[u8; 32]]) -> [u8; 32] {
        let run = [field(1, format!("run-{id}").as_bytes()), vec![16, 2]].concat();
        let mut body = [
            field(1, &user),
            field(3, format!("request-{id}").as_bytes()),
            field(17, &run),
        ]
        .concat();
        for step in steps {
            body.extend(field(2, step));
        }
        self.put(&field(1, &body))
    }
    fn snapshot(&self, turns: &[[u8; 32]], compact: bool) {
        let mut body = Vec::new();
        for turn in turns {
            body.extend(field(8, turn));
        }
        if compact {
            body.extend(field(13, &[4; 32]));
        }
        let root = self.put(&body);
        let meta =
            json!({"agentId":"native-session","latestRootBlobId":cursor_store::BlobId(root).hex()})
                .to_string()
                .bytes()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
        self.db
            .execute("INSERT OR REPLACE INTO meta VALUES ('0',?1)", [meta])
            .unwrap();
    }
    fn observer(
        &self,
        start: ObservationStart,
    ) -> (
        TerminalAgentObservation,
        Arc<Mutex<Vec<NativeIngestionBatch>>>,
    ) {
        struct TestClock;
        impl Clock for TestClock {
            fn now(&self) -> u64 {
                10
            }
        }
        let mut observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider: ProviderId::Cursor,
                session: "app-session".into(),
                process_nonce: "process".into(),
                expected_native_session: Some("native-session".into()),
                start,
                authorized_roots: vec![self.root.clone()],
            },
            ObservationCallbacks {
                live: Box::new(|_| panic!("store content must use acknowledged ingestion")),
                native_baseline: Box::new(|_| {}),
                warning: Box::new(|_| {}),
                accepted_prompt: Box::new(|_| {
                    panic!("native input must retain its source provenance")
                }),
                observed_hook: Box::new(|_| {}),
            },
            Arc::new(TestClock),
        )
        .unwrap();
        observer
            .configure_cursor_store(CursorObservationConfig {
                chats_root: self.chats.clone(),
                cli_version: "2026.11.13-eacade1".into(),
                staging_root: self.root.join("staging"),
            })
            .unwrap();
        let batches = Arc::new(Mutex::new(Vec::new()));
        let sink = batches.clone();
        observer
            .set_ingestion(Box::new(move |batch| sink.lock().unwrap().push(batch)))
            .unwrap();
        (observer, batches)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn var(mut value: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            break;
        }
    }
    bytes
}
fn field(number: usize, body: &[u8]) -> Vec<u8> {
    [var((number << 3) | 2), var(body.len()), body.to_vec()].concat()
}
fn batch(
    observer: &mut TerminalAgentObservation,
    batches: &Arc<Mutex<Vec<NativeIngestionBatch>>>,
) -> NativeIngestionBatch {
    for _ in 0..32 {
        observer.poll().unwrap();
        if let Some(batch) = batches.lock().unwrap().pop() {
            return batch;
        }
    }
    panic!("fixture did not produce a bounded batch")
}

#[test]
fn cursor_observation_ingestion_waits_for_ack_and_binds_repeated_submissions_by_native_occurrence()
{
    let fixture = Fixture::new();
    let first = fixture.turn("first", "Partial");
    let second = fixture.turn("second", "Partial");
    fixture.snapshot(&[first, second], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    observer
        .expect_submission("Repeat", "submission-one")
        .unwrap();
    observer
        .expect_submission("Repeat", "submission-two")
        .unwrap();
    let initial = batch(&mut observer, &batches);
    assert_eq!(initial.phase, TranscriptPhase::Live);
    assert!(initial.complete && initial.finished);
    assert!(!observer.caught_up());
    let users = initial
        .records
        .iter()
        .filter(|record| record.checkpoint["kind"] == "user")
        .collect::<Vec<_>>();
    assert_eq!(
        users
            .iter()
            .map(|record| record.input_binding.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("submission-one"), Some("submission-two")]
    );
    assert_ne!(users[0].occurrence, users[1].occurrence);
    assert_eq!(
        batch(&mut observer, &batches),
        initial,
        "unacknowledged bytes must be retried exactly"
    );
    assert_eq!(
        observer.acknowledge_ingestion("wrong"),
        Err(ObservationError::Identity)
    );
    observer.acknowledge_ingestion(&initial.id).unwrap();
    observer.poll().unwrap();
    assert!(observer.caught_up());

    observer
        .expect_submission("Repeat", "submission-three")
        .unwrap();
    let updated = fixture.turn("first", "Final answer");
    let third = fixture.turn("third", "Partial");
    fixture.snapshot(&[updated, second, third], true);
    let changed = batch(&mut observer, &batches);
    let before = initial
        .records
        .iter()
        .find(|record| record.checkpoint["kind"] == "text")
        .unwrap();
    let after = changed
        .records
        .iter()
        .find(|record| record.occurrence == before.occurrence)
        .unwrap();
    assert_ne!(before.version, after.version);
    assert_eq!(
        before.frame.as_ref().unwrap()["messageId"],
        after.frame.as_ref().unwrap()["messageId"]
    );
    let users = changed
        .records
        .iter()
        .filter(|record| record.checkpoint["kind"] == "user")
        .collect::<Vec<_>>();
    assert_eq!(
        users
            .iter()
            .map(|record| record.input_binding.as_deref())
            .collect::<Vec<_>>(),
        vec![None, None, Some("submission-three")]
    );
    observer.acknowledge_ingestion(&changed.id).unwrap();
    observer.poll().unwrap();
    assert!(observer.caught_up());
}

#[test]
fn cursor_observation_resume_batches_stay_replay_until_final_ack_and_recheck_source_before_catchup()
{
    let fixture = Fixture::new();
    let turns = (0..50)
        .map(|i| fixture.turn(&format!("user-{i}"), "Answer"))
        .collect::<Vec<_>>();
    fixture.snapshot(&turns, false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Resume);
    let mut count = 0;
    loop {
        let next = batch(&mut observer, &batches);
        assert_eq!(next.phase, TranscriptPhase::Replay);
        assert!(!observer.baseline_ready());
        assert!(next.records.len() <= 64);
        count += next.records.len();
        observer.acknowledge_ingestion(&next.id).unwrap();
        if next.finished {
            break;
        }
    }
    assert_eq!(count, 150);
    assert!(observer.baseline_ready());
    assert!(!observer.caught_up());
    let new = fixture.turn("new", "New output");
    fixture.snapshot(&[turns[49], new], true);
    let changed = batch(&mut observer, &batches);
    assert_eq!(changed.phase, TranscriptPhase::Live);
    assert!(!observer.caught_up());
    observer.acknowledge_ingestion(&changed.id).unwrap();
    observer.poll().unwrap();
    assert!(observer.caught_up());
}

#[test]
fn cursor_observation_unsupported_tool_never_claims_coverage_even_if_later_compacted_out() {
    let fixture = Fixture::new();
    let user = fixture.put(&[field(1, b"Task"), field(2, b"user")].concat());
    let tool = fixture.put(&field(
        2,
        &[field(4095, &field(1, b"command")), field(57, b"tool-id")].concat(),
    ));
    let turn = fixture.turn_steps("user", user, &[tool]);
    fixture.snapshot(&[turn], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Resume);
    let next = batch(&mut observer, &batches);
    assert!(next.finished && !next.complete);
    assert!(next.records.iter().all(|record| record
        .frame
        .as_ref()
        .is_none_or(|frame| frame["type"] != "turn.completed")));
    observer.acknowledge_ingestion(&next.id).unwrap();
    observer.poll().unwrap();
    assert!(!observer.caught_up());
    assert!(!observer.baseline_ready());
    fixture.snapshot(&[], true);
    let empty = batch(&mut observer, &batches);
    observer.acknowledge_ingestion(&empty.id).unwrap();
    observer.poll().unwrap();
    assert!(!observer.caught_up());
    assert!(!observer.baseline_ready());
}

#[test]
fn cursor_observation_rejects_a_symlink_in_the_native_session_child() {
    let fixture = Fixture::new();
    fixture.snapshot(&[], false);
    let outside = fixture.root.join("outside");
    std::fs::rename(fixture.chats.join("native-session"), &outside).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.chats.join("native-session")).unwrap();
    let (mut observer, _) = fixture.observer(ObservationStart::Resume);
    assert_eq!(observer.poll(), Err(ObservationError::Path));
    assert!(!observer.caught_up());
}

fn stop(fixture: &Fixture, generation: &str) -> NativeHookEvent {
    serde_json::from_value(json!({"provider":"cursor","session":"app-session","process_nonce":"process","native_session":"native-session","kind":"stop_observed","generation_id":generation,"reason":"completed","transcript_path":fixture.root.join("native-session.jsonl")})).unwrap()
}
fn marker(event: &NativeHookEvent, bytes: &[u8]) {
    std::fs::write(event.transcript_path.as_ref().unwrap(), bytes).unwrap();
}
fn catch_up(
    observer: &mut TerminalAgentObservation,
    batches: &Arc<Mutex<Vec<NativeIngestionBatch>>>,
) {
    for _ in 0..32 {
        observer.poll().unwrap();
        while let Some(batch) = batches.lock().unwrap().pop() {
            observer.acknowledge_ingestion(&batch.id).unwrap();
        }
        if observer.caught_up() {
            return;
        }
    }
    panic!("synthetic source did not catch up");
}

#[test]
fn cursor_completion_requires_post_hook_marker_exact_request_and_durable_source_ack() {
    let fixture = Fixture::new();
    let first = fixture.turn("first", "Answer");
    fixture.snapshot(&[first], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    let mut stop = stop(&fixture, "request-first");
    let mut witness = CursorStopWitness::new(std::slice::from_ref(&fixture.root)).unwrap();
    marker(&stop, b"{\"type\":\"turn_ended\",\"status\":\"success\"}\n");
    stop.cursor_stop_stamp = witness.capture(&stop);
    assert!(stop.cursor_stop_stamp.is_some());
    observer.accept_hook(stop.clone()).unwrap();
    let pending = batch(&mut observer, &batches);
    assert!(observer.cursor_completion().unwrap().is_none());
    observer.acknowledge_ingestion(&pending.id).unwrap();
    observer.poll().unwrap();
    assert!(
        observer.cursor_completion().unwrap().is_none(),
        "old success trailer cannot close this request"
    );
    marker(
        &stop,
        b"{\"role\":\"assistant\",\"message\":{\"content\":[]}}\n",
    );
    assert!(observer.cursor_completion().unwrap().is_none());
    marker(&stop, b"{\"role\":\"assistant\",\"message\":{\"content\":[]}}\n{\"type\":\"turn_ended\",\"status\":\"success\"}");
    assert!(
        observer.cursor_completion().unwrap().is_none(),
        "partial line is not a marker"
    );
    marker(&stop, b"{\"role\":\"assistant\",\"message\":{\"content\":[]}}\n{\"type\":\"turn_ended\",\"status\":\"success\"}\n");
    let proof = observer.cursor_completion().unwrap().unwrap();
    assert_eq!(proof.request_id, "request-first");
    assert!(!proof.interrupted);
    let mut duplicate = stop.clone();
    duplicate.cursor_stop_stamp = witness.capture(&duplicate);
    observer.accept_hook(duplicate).unwrap();
    assert_eq!(
        observer.cursor_completion().unwrap().unwrap().native_source,
        proof.native_source
    );
    observer.acknowledge_cursor_completion(&proof.request_id);
    assert!(observer.cursor_completion().unwrap().is_none());
}

#[test]
fn cursor_completion_does_not_match_repeated_text_or_rearm_from_an_old_stop() {
    let fixture = Fixture::new();
    let second = fixture.turn("second", "Same answer");
    fixture.snapshot(&[second], true);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    let mut event = stop(&fixture, "request-first");
    let mut witness = CursorStopWitness::new(std::slice::from_ref(&fixture.root)).unwrap();
    event.cursor_stop_stamp = witness.capture(&event);
    observer.accept_hook(event.clone()).unwrap();
    marker(
        &event,
        b"{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
    );
    catch_up(&mut observer, &batches);
    assert!(observer.cursor_completion().unwrap().is_none());
    event.generation_id = Some("request-second".into());
    event.cursor_stop_stamp = witness.capture(&event);
    observer.accept_hook(event.clone()).unwrap();
    marker(
        &event,
        b"{\"role\":\"assistant\"}\n{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
    );
    assert!(observer.cursor_completion().unwrap().is_some());
    observer
        .expect_submission("Repeat", "new-submission")
        .unwrap();
    observer.accept_hook(event).unwrap();
    assert!(
        observer.cursor_completion().unwrap().is_none(),
        "an unresolved new input cannot inherit an old marker"
    );
}

#[test]
fn cursor_interruption_requires_durable_request_tail_without_a_success_marker() {
    let fixture = Fixture::new();
    let first = fixture.turn("first", "Partial answer");
    fixture.snapshot(&[first], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    let mut aborted = stop(&fixture, "request-first");
    aborted.reason = Some("aborted".into());
    observer.accept_hook(aborted.clone()).unwrap();
    let pending = batch(&mut observer, &batches);
    assert!(observer.cursor_completion().unwrap().is_none());
    observer.acknowledge_ingestion(&pending.id).unwrap();
    assert!(observer.cursor_completion().unwrap().is_none());
    observer.poll().unwrap();
    let proof = observer.cursor_completion().unwrap().unwrap();
    assert!(proof.interrupted);
    assert_eq!(proof.request_id, "request-first");
    assert!(!aborted.transcript_path.as_ref().unwrap().exists());

    // The native cleanup may still emit completed for this aborted request. Neither that
    // hook nor a stale optional run status upgrades an interruption into a successful reply.
    let mut completed = stop(&fixture, "request-first");
    let mut witness = CursorStopWitness::new(std::slice::from_ref(&fixture.root)).unwrap();
    completed.cursor_stop_stamp = witness.capture(&completed);
    observer.accept_hook(completed).unwrap();
    marker(
        &aborted,
        b"{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
    );
    let after_cleanup = observer.cursor_completion().unwrap().unwrap();
    assert!(after_cleanup.interrupted);
    assert_eq!(after_cleanup.native_source, proof.native_source);

    observer.acknowledge_cursor_completion(&proof.request_id);
    observer.accept_hook(aborted).unwrap();
    assert!(observer.cursor_completion().unwrap().is_none());
}

#[test]
fn cursor_interruption_rejects_other_requests_and_new_app_input() {
    let fixture = Fixture::new();
    fixture.snapshot(&[fixture.turn("first", "Partial answer")], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    catch_up(&mut observer, &batches);
    let mut aborted = stop(&fixture, "request-other");
    aborted.reason = Some("aborted".into());
    observer.accept_hook(aborted.clone()).unwrap();
    assert!(observer.cursor_completion().unwrap().is_none());
    aborted.generation_id = Some("request-first".into());
    observer.accept_hook(aborted.clone()).unwrap();
    assert!(observer.cursor_completion().unwrap().unwrap().interrupted);

    observer
        .expect_submission("Repeat", "new-submission")
        .unwrap();
    observer.accept_hook(aborted).unwrap();
    assert!(observer.cursor_completion().unwrap().is_none());
    fixture.snapshot(&[fixture.turn("second", "Next answer")], false);
    catch_up(&mut observer, &batches);
    assert!(observer.cursor_completion().unwrap().is_none());
}

#[test]
fn cursor_interruption_rejects_late_old_hooks_before_new_prompt_reaches_store() {
    let fixture = Fixture::new();
    fixture.snapshot(&[fixture.turn("first", "Partial answer")], false);
    let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
    catch_up(&mut observer, &batches);
    let mut aborted = stop(&fixture, "request-first");
    aborted.reason = Some("aborted".into());
    observer.accept_hook(aborted.clone()).unwrap();
    assert!(observer.cursor_completion().unwrap().is_some());

    let mut submitted = stop(&fixture, "request-second");
    submitted.kind = NativeHookKind::PromptSubmitted;
    submitted.reason = None;
    submitted.prompt = Some("Repeat".into());
    observer.accept_hook(submitted).unwrap();
    observer.accept_hook(aborted.clone()).unwrap();
    assert!(observer.cursor_completion().unwrap().is_none());
    fixture.snapshot(&[fixture.turn("second", "Next answer")], false);
    catch_up(&mut observer, &batches);
    assert!(observer.cursor_completion().unwrap().is_none());
    aborted.generation_id = Some("request-second".into());
    observer.accept_hook(aborted).unwrap();
    assert!(observer.cursor_completion().unwrap().unwrap().interrupted);
}

#[test]
fn cursor_restored_prompt_retry_settles_without_repeating_before_submit_hook() {
    for interrupted in [false, true] {
        for stop_before_source in [false, true] {
            let fixture = Fixture::new();
            let first = fixture.turn("first", "Partial answer");
            fixture.snapshot(&[first], false);
            let (mut observer, batches) = fixture.observer(ObservationStart::Fresh);
            let mut submitted = stop(&fixture, "request-first");
            submitted.kind = NativeHookKind::PromptSubmitted;
            submitted.reason = None;
            submitted.prompt = Some("Repeat".into());
            observer.accept_hook(submitted).unwrap();
            catch_up(&mut observer, &batches);
            let mut first_stop = stop(&fixture, "request-first");
            first_stop.reason = Some("aborted".into());
            observer.accept_hook(first_stop.clone()).unwrap();
            let first_proof = observer.cursor_completion().unwrap().unwrap();
            observer.acknowledge_cursor_completion(&first_proof.request_id);

            // Model a native retry with a new request ID but no repeated beforeSubmitPrompt.
            // Source identity, not the previous hook, owns that new turn.
            let second = fixture.turn("second", "Retried answer");
            if !stop_before_source {
                fixture.snapshot(&[first, second], false);
                catch_up(&mut observer, &batches);
            }
            let mut retry_stop = stop(&fixture, "request-second");
            if interrupted {
                retry_stop.reason = Some("aborted".into());
            } else {
                let mut witness =
                    CursorStopWitness::new(std::slice::from_ref(&fixture.root)).unwrap();
                retry_stop.cursor_stop_stamp = witness.capture(&retry_stop);
            }
            observer.accept_hook(retry_stop.clone()).unwrap();
            if stop_before_source {
                assert!(observer.cursor_completion().unwrap().is_none());
                fixture.snapshot(&[first, second], false);
                catch_up(&mut observer, &batches);
            }
            marker(
                &retry_stop,
                b"{\"type\":\"turn_ended\",\"status\":\"success\"}\n",
            );
            observer.accept_hook(first_stop).unwrap();
            let proof = observer.cursor_completion().unwrap().unwrap();
            assert_eq!(proof.request_id, "request-second");
            assert_eq!(proof.interrupted, interrupted);
            observer.acknowledge_cursor_completion(&proof.request_id);
            assert!(observer.cursor_completion().unwrap().is_none());
        }
    }
}

#[test]
fn cursor_stop_witness_rejects_unrelated_paths_children_and_links_before_reading() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let mut event = stop(&fixture, "request-first");
    let mut witness = CursorStopWitness::new(std::slice::from_ref(&fixture.root)).unwrap();
    event.agent_id = Some("child".into());
    assert!(witness.capture(&event).is_none());
    event.agent_id = None;
    event.transcript_path = Some(fixture.root.join("other-session.jsonl"));
    assert!(witness.capture(&event).is_none());
    event.transcript_path = Some(fixture.root.join("native-session.jsonl"));
    let target = fixture.root.join("unrelated");
    std::fs::write(&target, b"private fixture").unwrap();
    symlink(&target, event.transcript_path.as_ref().unwrap()).unwrap();
    assert!(witness.capture(&event).is_none());
    std::fs::remove_file(event.transcript_path.as_ref().unwrap()).unwrap();
    std::fs::hard_link(&target, event.transcript_path.as_ref().unwrap()).unwrap();
    assert!(witness.capture(&event).is_none());
}
