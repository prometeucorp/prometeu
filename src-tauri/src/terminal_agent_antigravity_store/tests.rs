use super::*;
#[test]
#[ignore = "requires an explicit hook-authenticated synthetic smoke; never searches histories"]
fn controlled_antigravity_store_preserves_tool_identity_and_complete_output() {
    let proof = std::env::var_os("PROMETEU_AGY_PROOF").expect("explicit smoke proof");
    let proof: serde_json::Value = serde_json::from_slice(&std::fs::read(proof).unwrap()).unwrap();
    assert_eq!(proof["provider"], "antigravity");
    let native = proof["native_session"].as_str().unwrap();
    let path = PathBuf::from(proof["scoped_store_file"].as_str().unwrap());
    assert_eq!(path.file_name().unwrap(), format!("{native}.db").as_str());
    let root = path.parent().unwrap().to_path_buf();
    let brain = root.parent().unwrap().join("brain");
    let mut reader = AntigravityStoreReader::new(
        AuthorizedRoot::new(root.clone()).unwrap(),
        root,
        AuthorizedRoot::new(brain.clone()).unwrap(),
        brain,
        native.into(),
        proof["version"].as_str().unwrap(),
        PathBuf::from(proof["test_root"].as_str().unwrap()).join("agy-reader-stage"),
    )
    .unwrap();
    loop {
        if reader.poll().unwrap() == SnapshotProgress::Ready {
            break;
        }
    }
    let mut projection =
        protocol::projection::Projection::new(native, reader.trajectory().unwrap()).unwrap();
    let mut after = None;
    let mut tools = Vec::new();
    let mut results = Vec::new();
    let mut captures = Vec::new();
    while let Some(step) = reader.next(after).unwrap() {
        after = Some(step.index);
        assert!(step.complete);
        for capture in projection.push(&step, 1).unwrap() {
            captures.push(crate::terminal_agent_observation::NativeCapture {
                occurrence: capture.occurrence.clone(),
                version: capture.version.clone(),
                checkpoint: capture.checkpoint.clone(),
                frame: capture.frame.clone(),
                input_binding: None,
            });
            if let Some(frame) = capture.frame {
                if frame["block"]["kind"] == "tool" {
                    tools.push(frame);
                } else if frame["type"] == "tool.completed" {
                    results.push(frame);
                }
            }
        }
    }
    assert_eq!(tools.len(), 1);
    assert_eq!(results.len(), 1);
    assert_eq!(tools[0]["block"]["id"], results[0]["toolId"]);
    assert_eq!(
        tools[0]["block"]["input"]["CommandLine"],
        "printf PROMETEU_TERMINAL_TOOL_SMOKE"
    );
    assert!(results[0]["output"]
        .as_str()
        .unwrap()
        .ends_with("PROMETEU_TERMINAL_TOOL_SMOKE"));
    assert_eq!(results[0]["error"], false);
    let output = PathBuf::from(proof["test_root"].as_str().unwrap())
        .join(format!("agy-source-journal-{}", uuid::Uuid::new_v4()));
    let canonical = output.join("canonical.jsonl");
    let journal = crate::chat::terminal::ingestion::JournalStore::open(
        &canonical,
        &crate::chat::terminal::ingestion::journal_path(&output, "smoke").unwrap(),
        crate::terminal_agent_journal::JournalBinding {
            app_session: "smoke".into(),
            provider: "antigravity".into(),
            native_session: Some(native.into()),
        },
        true,
    )
    .unwrap();
    let batch = crate::terminal_agent_observation::NativeIngestionBatch {
        id: "captured-batch".into(),
        phase: prometeu_protocols::terminal_transcript::TranscriptPhase::Replay,
        source_revision: "captured-native-snapshot".into(),
        records: captures,
        finished: true,
        complete: true,
        mode: crate::terminal_agent_observation::NativeIngestionMode::Snapshot,
    };
    journal.replay(&batch).unwrap();
    let first = std::fs::read(&canonical).unwrap();
    journal.replay(&batch).unwrap();
    assert_eq!(first, std::fs::read(&canonical).unwrap());
    assert!(journal.caught_up());
    let lines = String::from_utf8(first).unwrap();
    assert_eq!(
        lines
            .lines()
            .filter(
                |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["type"]
                    == "tool.completed"
            )
            .count(),
        1
    );
    drop(journal);
    std::fs::remove_dir_all(output).unwrap();
}

fn var(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 128 });
        if value == 0 {
            return bytes;
        }
    }
}
fn int(number: u64, value: u64) -> Vec<u8> {
    [var(number << 3), var(value)].concat()
}
fn field(number: u64, value: &[u8]) -> Vec<u8> {
    [
        var((number << 3) | 2),
        var(value.len() as u64),
        value.to_vec(),
    ]
    .concat()
}
fn payload(kind: u64, body_field: u64, body: &[u8], call: Option<&[u8]>) -> Vec<u8> {
    let mut metadata = int(3, if kind == 14 { 4 } else { 2 });
    if let Some(call) = call {
        metadata.extend(field(4, call));
    }
    [
        int(1, kind),
        int(4, 3),
        field(5, &metadata),
        field(body_field, body),
    ]
    .concat()
}
struct Fixture {
    root: PathBuf,
    conversations: PathBuf,
    brain: PathBuf,
    connection: rusqlite::Connection,
    output: PathBuf,
    call: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("prometeu-agy-store-{}", uuid::Uuid::new_v4()));
        let conversations = root.join("conversations");
        let brain = root.join("brain");
        crate::paths::ensure_private_dir(&conversations).unwrap();
        crate::paths::ensure_private_dir(&brain).unwrap();
        let connection =
            rusqlite::Connection::open(conversations.join("native-session.db")).unwrap();
        connection.execute_batch("PRAGMA user_version=1; PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE trajectory_meta(trajectory_id TEXT PRIMARY KEY,cascade_id TEXT,trajectory_type INTEGER,source INTEGER); INSERT INTO trajectory_meta VALUES('trajectory','native-session',1,2); CREATE TABLE steps(idx INTEGER PRIMARY KEY,step_type INTEGER NOT NULL DEFAULT 0,status INTEGER NOT NULL DEFAULT 0,has_subtrajectory numeric NOT NULL DEFAULT false,metadata BLOB,error_details BLOB,permissions BLOB,task_details BLOB,render_info BLOB,step_payload BLOB,step_format INTEGER NOT NULL DEFAULT 0);").unwrap();
        let output = brain.join("native-session/.system_generated/steps/2/output.txt");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(&output, "COMPLETE OUTPUT").unwrap();
        let call = [
            field(1, b"native-call"),
            field(2, b"run_command"),
            field(3, br#"{"CommandLine":"printf MARKER"}"#),
        ]
        .concat();
        let fixture = Self {
            root,
            conversations,
            brain,
            connection,
            output,
            call,
        };
        fixture.insert(0, 14, &payload(14, 19, &field(2, b"Repeated prompt"), None));
        fixture.insert(1, 15, &payload(15, 20, &field(7, &fixture.call), None));
        fixture.result(&fixture.output);
        fixture
    }
    fn insert(&self, index: i64, kind: i64, bytes: &[u8]) {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO steps(idx,step_type,status,step_payload)VALUES(?1,?2,3,?3)",
                params![index, kind, bytes],
            )
            .unwrap();
    }
    fn result(&self, path: &std::path::Path) {
        let result = [
            field(1, b"truncated preview"),
            field(
                7,
                reqwest::Url::from_file_path(path)
                    .unwrap()
                    .as_str()
                    .as_bytes(),
            ),
        ]
        .concat();
        self.insert(
            2,
            132,
            &payload(132, 140, &field(2, &result), Some(&self.call)),
        );
    }
    fn reader(&self) -> AntigravityStoreReader {
        AntigravityStoreReader::new(
            AuthorizedRoot::new(self.conversations.clone()).unwrap(),
            self.conversations.clone(),
            AuthorizedRoot::new(self.brain.clone()).unwrap(),
            self.brain.clone(),
            "native-session".into(),
            "1.5.10",
            self.root.join("stage"),
        )
        .unwrap()
    }
    fn stamps(&self) -> Vec<(Vec<u8>, Stamp)> {
        [
            "native-session.db",
            "native-session.db-wal",
            "native-session.db-shm",
        ]
        .into_iter()
        .map(|name| {
            let path = self.conversations.join(name);
            (
                std::fs::read(&path).unwrap(),
                Stamp::from(&std::fs::metadata(path).unwrap()),
            )
        })
        .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn drain(reader: &mut AntigravityStoreReader) -> Result<(), ObservationError> {
    for _ in 0..64 {
        if reader.poll()? == SnapshotProgress::Ready {
            return Ok(());
        }
    }
    panic!("fixture copy did not finish")
}
#[test]
fn agy_native_compaction_checkpoint_survives_sqlite_replay_without_model_completion() {
    use crate::terminal_agent_observation::{
        NativeCapture, NativeIngestionBatch, NativeIngestionMode,
    };
    use prometeu_core::conversation::stream::TranscriptStore;
    use prometeu_protocols::terminal_transcript::TranscriptPhase;
    let fixture = Fixture::new();
    fixture.connection.execute("DELETE FROM steps", []).unwrap();
    fixture.insert(0, 14, &payload(14, 19, &field(2, b"/compact"), None));
    let checkpoint = [
        int(1, 23),
        int(4, 3),
        field(5, &int(3, 5)),
        field(30, &field(5, b"Native retained summary")),
    ]
    .concat();
    fixture.insert(1, 23, &checkpoint);
    let mut reader = fixture.reader();
    drain(&mut reader).unwrap();
    let journal = crate::chat::terminal::ingestion::JournalStore::open(
        &fixture.root.join("canonical.jsonl"),
        &fixture.root.join("journal.sqlite"),
        crate::terminal_agent_journal::JournalBinding {
            app_session: "app".into(),
            provider: "antigravity".into(),
            native_session: Some("native-session".into()),
        },
        false,
    )
    .unwrap();
    let mut prior = None;
    for pass in 0..2 {
        let mut projection =
            protocol::projection::Projection::new("native-session", "trajectory").unwrap();
        let mut records = Vec::new();
        let mut after = None;
        while let Some(step) = reader.next(after).unwrap() {
            after = Some(step.index);
            records.extend(
                projection
                    .push(&step, 1)
                    .unwrap()
                    .into_iter()
                    .map(|capture| NativeCapture {
                        occurrence: capture.occurrence,
                        version: capture.version,
                        checkpoint: capture.checkpoint,
                        frame: capture.frame,
                        input_binding: None,
                    }),
            );
        }
        assert_eq!(records[0].checkpoint["maintenance"], "compact");
        assert_eq!(
            records.last().unwrap().checkpoint["sourceUser"],
            records[0].occurrence
        );
        let batch = NativeIngestionBatch {
            id: format!("pass-{pass}"),
            phase: TranscriptPhase::Replay,
            source_revision: "checkpoint".into(),
            records,
            finished: true,
            complete: true,
            mode: NativeIngestionMode::Snapshot,
        };
        journal.replay(&batch).unwrap();
        assert!(journal.caught_up());
        let canonical = journal.load().unwrap();
        let kinds = canonical
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            ["user.message", "system.summary", "context.compacted"]
        );
        if let Some(prior) = &prior {
            assert_eq!(&canonical, prior);
        }
        prior = Some(canonical);
    }
}
#[test]
fn agy_hidden_system_step_preserves_live_followup_causality_and_quiet_cold_replay() {
    use crate::chat::terminal::ingestion::{JournalStore, NativeDisposition};
    use crate::terminal_agent_observation::{
        NativeCapture, NativeIngestionBatch, NativeIngestionMode,
    };
    use prometeu_core::conversation::stream::TranscriptStore;
    use prometeu_protocols::terminal_transcript::TranscriptPhase;

    fn batch(
        reader: &mut AntigravityStoreReader,
        id: &str,
        phase: TranscriptPhase,
    ) -> NativeIngestionBatch {
        drain(reader).unwrap();
        let mut projection =
            protocol::projection::Projection::new("native-session", "trajectory").unwrap();
        let mut after = None;
        let mut records = Vec::new();
        while let Some(step) = reader.next(after).unwrap() {
            assert!(step.complete);
            after = Some(step.index);
            records.extend(
                projection
                    .push(&step, 1)
                    .unwrap()
                    .into_iter()
                    .map(|capture| NativeCapture {
                        occurrence: capture.occurrence,
                        version: capture.version,
                        checkpoint: capture.checkpoint,
                        frame: capture.frame,
                        input_binding: None,
                    }),
            );
        }
        NativeIngestionBatch {
            id: id.into(),
            phase,
            source_revision: id.into(),
            records,
            finished: true,
            complete: true,
            mode: NativeIngestionMode::Snapshot,
        }
    }

    let fixture = Fixture::new();
    fixture.connection.execute("DELETE FROM steps", []).unwrap();
    let user = payload(14, 19, &field(2, b"Repeated prompt"), None);
    let answer = payload(15, 20, &field(1, b"Repeated answer"), None);
    fixture.insert(0, 14, &user);
    fixture.insert(1, 15, &answer);
    let open = || {
        JournalStore::open(
            &fixture.root.join("canonical.jsonl"),
            &fixture.root.join("journal.sqlite"),
            crate::terminal_agent_journal::JournalBinding {
                app_session: "app".into(),
                provider: "antigravity".into(),
                native_session: Some("native-session".into()),
            },
            false,
        )
        .unwrap()
    };
    let journal = open();
    let mut reader = fixture.reader();
    journal
        .replay(&batch(&mut reader, "baseline", TranscriptPhase::Replay))
        .unwrap();
    assert_eq!(journal.load().unwrap().lines().count(), 2);

    // A resumed invocation inserts an explicitly hidden native system message between the
    // new user and answer. Identical visible text across turns must still retain both turns.
    fixture.insert(2, 14, &user);
    let system = [
        field(1, b"Hidden invocation wrapper"),
        field(2, &int(4, 1)),
        field(
            4,
            &[
                field(1, b"hidden-native-message"),
                field(4, b"Hidden invocation context"),
                int(11, 1),
            ]
            .concat(),
        ),
    ]
    .concat();
    let hidden = [
        int(1, 101),
        int(4, 3),
        field(5, &int(3, 5)),
        field(114, &system),
    ]
    .concat();
    fixture.insert(3, 101, &hidden);
    fixture.insert(4, 15, &answer);
    let live = batch(&mut reader, "followup", TranscriptPhase::Live);
    let hidden = live
        .records
        .iter()
        .find(|record| record.occurrence.ends_with(":step:3:checkpoint"))
        .unwrap();
    assert!(hidden.frame.is_none());
    assert_eq!(hidden.checkpoint["sourceScope"], "record");
    assert_eq!(
        hidden.checkpoint["sourceUser"],
        "native:antigravity:native-session:step:2:user"
    );
    assert_eq!(hidden.checkpoint["trajectoryId"], "trajectory");
    journal.begin_batch(&live).unwrap();
    for record in &live.records {
        let disposition = journal.disposition(record).unwrap();
        if record.occurrence.contains(":step:4:") && record.frame.is_some() {
            assert_eq!(disposition, NativeDisposition::Output);
            assert_eq!(
                record.checkpoint["sourceUser"],
                hidden.checkpoint["sourceUser"]
            );
        }
        match disposition {
            NativeDisposition::Duplicate | NativeDisposition::Checkpoint => {
                journal.commit_checkpoint(record).unwrap();
            }
            NativeDisposition::User | NativeDisposition::Output => {
                let line = record.frame.as_ref().unwrap().to_string();
                let offset = journal.load().unwrap().lines().count() as u64;
                journal.commit_record(record, Some(&line), offset).unwrap();
            }
            other => panic!("unexpected live disposition: {other:?}"),
        }
    }
    journal.finish_batch(&live).unwrap();
    assert!(journal.caught_up());
    let canonical = journal.load().unwrap();
    let kinds: Vec<_> = canonical
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["type"].clone())
        .collect();
    assert_eq!(
        kinds,
        [
            "user.message",
            "assistant.block",
            "user.message",
            "assistant.block"
        ]
    );
    assert!(!canonical.contains("Hidden invocation"));
    drop(journal);
    let cold = open();
    let mut reader = fixture.reader();
    let replay = batch(&mut reader, "cold", TranscriptPhase::Replay);
    for _ in 0..2 {
        cold.replay(&replay).unwrap();
        assert!(cold.caught_up());
        assert_eq!(cold.load().unwrap(), canonical);
    }
}

#[test]
fn agy_store_reads_committed_wal_and_full_output_without_touching_original_files() {
    let fixture = Fixture::new();
    let before = fixture.stamps();
    let mut reader = fixture.reader();
    drain(&mut reader).unwrap();
    let user = reader.next(None).unwrap().unwrap();
    assert!(matches!(user.content,Content::User{text,..}if text=="Repeated prompt"));
    let tool = reader.next(Some(1)).unwrap().unwrap();
    assert!(tool.complete);
    assert!(matches!(tool.content,Content::Tool{result:Some(value),..}if value=="COMPLETE OUTPUT"));
    assert_eq!(before, fixture.stamps());
    assert_eq!(reader.poll().unwrap(), SnapshotProgress::Unchanged);
    let old_version = tool.version;
    std::fs::write(&fixture.output, "FINAL OUTPUT").unwrap();
    assert_eq!(reader.poll().unwrap(), SnapshotProgress::Ready);
    let updated = reader.next(Some(1)).unwrap().unwrap();
    assert_ne!(old_version, updated.version);
    assert_eq!(reader.poll().unwrap(), SnapshotProgress::Unchanged);
}
#[test]
fn agy_store_rejects_identity_schema_unsupported_format_and_external_output_escape() {
    let fixture = Fixture::new();
    fixture
        .connection
        .execute("UPDATE trajectory_meta SET cascade_id='different'", [])
        .unwrap();
    assert_eq!(fixture.reader().poll(), Err(ObservationError::Identity));
    fixture
        .connection
        .execute("UPDATE trajectory_meta SET cascade_id='native-session'", [])
        .unwrap();
    fixture
        .connection
        .execute("UPDATE steps SET step_format=1 WHERE idx=2", [])
        .unwrap();
    let mut reader = fixture.reader();
    drain(&mut reader).unwrap();
    assert_eq!(reader.next(Some(1)), Err(ObservationError::Transcript));
    fixture.result(&fixture.root.join("outside.txt"));
    drain(&mut reader).unwrap();
    assert_eq!(reader.next(Some(1)), Err(ObservationError::Path));
    fixture
        .connection
        .execute_batch("PRAGMA user_version=2")
        .unwrap();
    assert_eq!(fixture.reader().poll(), Err(ObservationError::Transcript));
}
#[test]
fn agy_store_rejects_symlink_output_and_detects_database_rotation() {
    let fixture = Fixture::new();
    let mut reader = fixture.reader();
    drain(&mut reader).unwrap();
    std::fs::remove_file(&fixture.output).unwrap();
    std::os::unix::fs::symlink(fixture.root.join("secret"), &fixture.output).unwrap();
    assert_eq!(reader.next(Some(1)), Err(ObservationError::Path));
    fixture
        .connection
        .execute(
            "UPDATE trajectory_meta SET trajectory_id='new-trajectory'",
            [],
        )
        .unwrap();
    assert_eq!(reader.poll(), Err(ObservationError::Identity));
}
#[test]
fn agy_observer_fresh_live_resume_quiet_and_receipts_gate_coverage() {
    use crate::terminal_agent_observation::*;
    use prometeu_protocols::terminal_transcript::TranscriptPhase;
    use std::sync::{Arc, Mutex};
    struct Clock;
    impl prometeu_core::conversation::Clock for Clock {
        fn now(&self) -> u64 {
            1
        }
    }
    for start in [ObservationStart::Fresh, ObservationStart::Resume] {
        let fixture = Fixture::new();
        let batches = Arc::new(Mutex::new(Vec::<NativeIngestionBatch>::new()));
        let output = batches.clone();
        let mut observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider: crate::state::ProviderId::Antigravity,
                session: "app-session".into(),
                process_nonce: "process".into(),
                expected_native_session: Some("native-session".into()),
                start,
                authorized_roots: vec![fixture.brain.clone()],
            },
            ObservationCallbacks {
                live: Box::new(|_| panic!("legacy live callback")),
                native_baseline: Box::new(|_| {}),
                warning: Box::new(|_| {}),
                accepted_prompt: Box::new(|_| panic!("legacy prompt callback")),
                observed_hook: Box::new(|_| {}),
            },
            Arc::new(Clock),
        )
        .unwrap();
        observer
            .configure_antigravity_store(AntigravityObservationConfig {
                conversations_root: fixture.conversations.clone(),
                brain_root: fixture.brain.clone(),
                cli_version: "1.5.10".into(),
                staging_root: fixture.root.join("observer-stage"),
            })
            .unwrap();
        observer
            .set_ingestion(Box::new(move |batch| output.lock().unwrap().push(batch)))
            .unwrap();
        if start == ObservationStart::Fresh {
            observer
                .expect_submission("Repeated prompt", "receipt-one")
                .unwrap();
        }
        let mut delivered = Vec::new();
        for _ in 0..32 {
            observer.poll().unwrap();
            let batch = batches.lock().unwrap().pop();
            if let Some(batch) = batch {
                assert!(!observer.caught_up());
                assert_eq!(
                    batch.phase,
                    if start == ObservationStart::Fresh {
                        TranscriptPhase::Live
                    } else {
                        TranscriptPhase::Replay
                    }
                );
                delivered.extend(batch.records);
                observer.acknowledge_ingestion(&batch.id).unwrap();
            }
            if observer.caught_up() {
                break;
            }
        }
        assert!(observer.caught_up());
        assert!(observer.baseline_ready());
        let user = delivered
            .iter()
            .find(|r| r.checkpoint["kind"] == "user")
            .unwrap();
        assert_eq!(
            user.input_binding.as_deref(),
            (start == ObservationStart::Fresh).then_some("receipt-one")
        );
        let tool = delivered
            .iter()
            .find(|r| r.checkpoint["kind"] == "tool")
            .unwrap();
        let result = delivered
            .iter()
            .find(|r| r.checkpoint["kind"] == "result")
            .unwrap();
        assert_eq!(tool.checkpoint["toolId"], result.checkpoint["toolId"]);
        assert_eq!(result.checkpoint["sourceUser"], user.occurrence);
    }
}

#[test]
fn agy_full_store_completion_requires_the_current_snapshot_final_without_tool_calls() {
    use crate::terminal_agent_observation::*;
    use prometeu_protocols::terminal_status::{ProgramStatusReport, ProgramStatusState};
    use std::sync::{Arc, Mutex};
    struct Clock;
    impl prometeu_core::conversation::Clock for Clock {
        fn now(&self) -> u64 {
            1
        }
    }
    for changed in ["removed", "tool_call", "unchanged"] {
        let fixture = Fixture::new();
        fixture
            .connection
            .execute("DELETE FROM steps WHERE idx=2", [])
            .unwrap();
        fixture.insert(1, 15, &payload(15, 20, &field(1, b"Final answer"), None));
        let batches = Arc::new(Mutex::new(Vec::<NativeIngestionBatch>::new()));
        let output = batches.clone();
        let mut observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider: crate::state::ProviderId::Antigravity,
                session: "app-session".into(),
                process_nonce: "process".into(),
                expected_native_session: Some("native-session".into()),
                start: ObservationStart::Fresh,
                authorized_roots: vec![fixture.brain.clone()],
            },
            ObservationCallbacks {
                live: Box::new(|_| panic!("legacy callback")),
                native_baseline: Box::new(|_| {}),
                warning: Box::new(|_| {}),
                accepted_prompt: Box::new(|_| panic!("legacy callback")),
                observed_hook: Box::new(|_| {}),
            },
            Arc::new(Clock),
        )
        .unwrap();
        assert!(observer.configure_completion("1.5.10"));
        observer
            .configure_antigravity_store(AntigravityObservationConfig {
                conversations_root: fixture.conversations.clone(),
                brain_root: fixture.brain.clone(),
                cli_version: "1.5.10".into(),
                staging_root: fixture.root.join("observation-stage"),
            })
            .unwrap();
        observer
            .set_ingestion(Box::new(move |batch| output.lock().unwrap().push(batch)))
            .unwrap();
        observer
            .expect_submission("Repeated prompt", "submission")
            .unwrap();
        let hook = |kind: &str| {
            serde_json::from_value(serde_json::json!({"provider":"antigravity","session":"app-session","process_nonce":"process","native_session":"native-session","kind":kind,"prompt":"Repeated prompt","initial_num_steps":1,"execution_number":0,"fully_idle":true,"reason":"NO_TOOL_CALL"})).unwrap()
        };
        let report = |state| ProgramStatusReport {
            id: None,
            state,
            kind: None,
            message: None,
            app: None,
            title: None,
            progress: None,
        };
        let drain = |observer: &mut TerminalAgentObservation| {
            for _ in 0..32 {
                observer.poll().unwrap();
                let batch = batches.lock().unwrap().pop();
                if let Some(batch) = batch {
                    observer.acknowledge_ingestion(&batch.id).unwrap();
                }
                if observer.caught_up() {
                    return;
                }
            }
            panic!("fixture observer did not catch up");
        };
        observer
            .accept_hook(hook("model_invocation_started"))
            .unwrap();
        observer.observe_program_status(&report(ProgramStatusState::Working));
        drain(&mut observer);
        assert!(observer.take_persisted_completion().is_none());
        match changed {
            "removed" => {
                fixture
                    .connection
                    .execute("DELETE FROM steps WHERE idx=1", [])
                    .unwrap();
            }
            "tool_call" => fixture.insert(
                1,
                15,
                &payload(
                    15,
                    20,
                    &[field(1, b"Final answer"), field(7, &fixture.call)].concat(),
                    None,
                ),
            ),
            _ => {}
        }
        observer.accept_hook(hook("stop_observed")).unwrap();
        observer.observe_program_status(&report(ProgramStatusState::Done));
        drain(&mut observer);
        assert_eq!(
            observer.take_persisted_completion().is_some(),
            changed == "unchanged",
            "snapshot case {changed}"
        );
    }
}
