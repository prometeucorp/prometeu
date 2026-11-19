use super::*;

struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> u64 {
        1
    }
}

fn fixture(provider: ProviderId) -> (SourceObservation, Arc<Mutex<Vec<NativeIngestionBatch>>>) {
    let mut source = SourceObservation::new(provider, ObservationStart::Fresh);
    let batches = Arc::new(Mutex::new(Vec::new()));
    let output = batches.clone();
    source.set_callback(Box::new(move |batch| output.lock().unwrap().push(batch)));
    (source, batches)
}
fn record(value: Value, phase: TranscriptPhase) -> TranscriptRecord {
    TranscriptRecord {
        ordinal: 0,
        offset: 0,
        phase,
        value,
        omitted_media: 0,
    }
}
fn text(message: &str) -> Value {
    json!({"v":1,"type":"assistant.block","ts":1,"messageId":"old-synthetic-id","index":0,"block":{"kind":"text","text":message}})
}

#[test]
fn claude_native_compaction_without_a_model_turn_keeps_source_coverage() {
    for start in [ObservationStart::Fresh, ObservationStart::Resume] {
        let (mut source, batches) = fixture(ProviderId::Claude);
        source.phase = if start == ObservationStart::Fresh {
            TranscriptPhase::Live
        } else {
            TranscriptPhase::Replay
        };
        let mut adapter = crate::claude::Adapter::fresh();
        // Native system rows use the same shapes exercised by the Claude adapter tests.
        for (ordinal, value) in [
            json!({"uuid":"compact-start","type":"system","subtype":"status","status":"compacting"}),
            json!({"uuid":"compact-boundary","type":"system","subtype":"compact_boundary","compact_metadata":{"pre_tokens":12000,"post_tokens":2000,"trigger":"manual"}}),
            json!({"uuid":"compact-stop","type":"system","subtype":"status","status":null}),
        ].into_iter().enumerate() {
            let frames = adapter.translate(&value);
            let mut row = record(value, source.phase);
            row.ordinal = ordinal as u64;
            source.observe("session", &row, &frames);
        }
        source.finish_poll(false, false, "compact", false).unwrap();
        let batch = batches.lock().unwrap().pop().unwrap();
        assert!(batch.complete && batch.finished);
        assert_eq!(batch.records.len(), 3);
        for capture in &batch.records {
            assert_eq!(capture.checkpoint["kind"], "metadata");
            assert!(capture.checkpoint["sourceUser"].is_null());
        }
        let compacted = batch.records[1].frame.as_ref().unwrap();
        assert_eq!(compacted["type"], "context.compacted");
        assert_eq!(compacted["before"], 12000);
        assert_eq!(compacted["after"], 2000);
        assert_eq!(compacted["trigger"], "manual");
        source.acknowledge(&batch.id).unwrap();
        source.before_poll().unwrap();
        source.unchanged();
        assert!(source.caught_up() && source.baseline_ready());
    }
}

#[test]
fn native_source_rewrite_keeps_occurrence_changes_version_and_waits_for_durable_ack() {
    let (mut source, batches) = fixture(ProviderId::Antigravity);
    let mut row = record(
        json!({"step_index":1,"type":"PLANNER_RESPONSE","source":"MODEL","status":"RUNNING","content":"Partial"}),
        TranscriptPhase::Live,
    );
    source.observe("session", &row, &[text("Partial")]);
    source.finish_poll(false, false, "before", false).unwrap();
    let first = batches.lock().unwrap().pop().unwrap();
    assert_eq!(first.mode, NativeIngestionMode::Append);
    assert!(!source.caught_up());
    assert!(source.before_poll().unwrap());
    assert_eq!(batches.lock().unwrap().pop().unwrap().id, first.id);
    source.acknowledge(&first.id).unwrap();
    row.value["status"] = json!("DONE");
    row.value["content"] = json!("Final");
    row.phase = TranscriptPhase::Replay;
    source.observe("session", &row, &[text("Final")]);
    source.finish_poll(false, false, "after", true).unwrap();
    let final_batch = batches.lock().unwrap().pop().unwrap();
    assert_eq!(final_batch.phase, TranscriptPhase::Live);
    assert_eq!(final_batch.mode, NativeIngestionMode::Snapshot);
    assert_eq!(
        first.records[0].occurrence,
        final_batch.records[0].occurrence
    );
    assert_ne!(first.records[0].version, final_batch.records[0].version);
    assert_eq!(
        final_batch.records[0].frame.as_ref().unwrap()["block"]["text"],
        "Final"
    );
    assert_eq!(final_batch.records[1].checkpoint["sourceScope"], "record");
    source.acknowledge(&final_batch.id).unwrap();
    source.before_poll().unwrap();
    source.unchanged();
    assert!(source.caught_up());
}

#[test]
fn native_source_claude_preserves_explicit_tool_result_relationship_and_source_versions() {
    let (mut source, batches) = fixture(ProviderId::Claude);
    let assistant = record(
        json!({"uuid":"assistant","type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"native-tool","name":"Read","input":{"file_path":"fixture"}}]}}),
        TranscriptPhase::Live,
    );
    source.observe("session", &assistant, &[json!({"v":1,"type":"assistant.block","ts":1,"messageId":"api-id","index":4,"block":{"kind":"tool","id":"native-tool","name":"Read","input":{"file_path":"fixture"}}})]);
    source.observe("session", &record(json!({"uuid":"result","type":"user"}), TranscriptPhase::Live), &[json!({"v":1,"type":"tool.completed","ts":2,"toolId":"native-tool","output":"Contents","error":false,"background":false})]);
    source.finish_poll(false, false, "source", false).unwrap();
    let batch = batches.lock().unwrap().pop().unwrap();
    let tool = batch
        .records
        .iter()
        .find(|r| r.checkpoint["kind"] == "tool")
        .unwrap();
    let result = batch
        .records
        .iter()
        .find(|r| r.checkpoint["kind"] == "result")
        .unwrap();
    assert_eq!(
        result.occurrence.strip_suffix(":result"),
        tool.occurrence.strip_suffix(":tool")
    );
    assert_eq!(result.checkpoint["toolId"], tool.checkpoint["toolId"]);
    assert_eq!(result.checkpoint["messageId"], tool.checkpoint["messageId"]);
    assert!(batch.complete);
}

#[test]
fn claude_source_keeps_the_original_user_for_late_tool_results_and_ancestry() {
    let (mut source, _) = fixture(ProviderId::Claude);
    let user = |id, parent| json!({"uuid":id,"parentUuid":parent,"type":"user"});
    let user_frame =
        json!({"v":1,"type":"user.message","at":1,"content":[{"kind":"text","text":"Task"}]});
    source.observe(
        "session",
        &record(user("first", Value::Null), TranscriptPhase::Live),
        &[user_frame.clone()],
    );
    let mut assistant = record(
        json!({"uuid":"assistant","parentUuid":"first","type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","id":"call","name":"Read","input":{}}]}}),
        TranscriptPhase::Live,
    );
    assistant.ordinal = 1;
    let tool = json!({"v":1,"type":"assistant.block","at":2,"messageId":"api","index":0,"block":{"kind":"tool","id":"call","name":"Read","input":{}}});
    source.observe("session", &assistant, &[tool.clone()]);
    let mut second = record(user("second", json!("assistant")), TranscriptPhase::Live);
    second.ordinal = 2;
    source.observe("session", &second, &[user_frame]);
    let mut late = record(
        json!({"uuid":"late-result","parentUuid":"second","type":"user"}),
        TranscriptPhase::Live,
    );
    late.ordinal = 3;
    source.observe("session", &late, &[json!({"v":1,"type":"tool.completed","at":3,"toolId":"call","output":"Late","error":false,"background":false})]);
    let captured = source.staged.back().unwrap();
    assert_eq!(captured.checkpoint["kind"], "result");
    assert_eq!(
        captured.checkpoint["sourceUser"],
        "native:claude:session:first:user"
    );
    assert_eq!(
        source.current_user.as_deref(),
        Some("native:claude:session:second:user")
    );
    let mut ancestor = record(
        json!({"uuid":"late-assistant","parentUuid":"assistant","type":"assistant","message":{"content":[{"type":"text","text":"Late answer"}]}}),
        TranscriptPhase::Live,
    );
    ancestor.ordinal = 4;
    source.observe("session", &ancestor, &[text("Late answer")]);
    assert_eq!(
        source.staged.back().unwrap().checkpoint["sourceUser"],
        "native:claude:session:first:user"
    );
    assistant.value["uuid"] = json!("different-owner");
    assistant.value["parentUuid"] = json!("second");
    assistant.ordinal = 5;
    source.observe("session", &assistant, &[tool]);
    assert_eq!(source.failure, Some(ObservationError::Identity));
}

#[test]
fn claude_source_can_resolve_unknown_ancestry_without_reassigning_known_rows() {
    let (mut source, _) = fixture(ProviderId::Claude);
    let mut assistant = record(
        json!({"uuid":"assistant","parentUuid":"first","type":"assistant","message":{"content":[{"type":"text","text":"Answer"}]}}),
        TranscriptPhase::Replay,
    );
    source.observe("session", &assistant, &[text("Answer")]);
    assert!(source.staged.back().unwrap().checkpoint["sourceUser"].is_null());
    let user_frame =
        json!({"v":1,"type":"user.message","at":1,"content":[{"kind":"text","text":"Task"}]});
    source.observe(
        "session",
        &record(
            json!({"uuid":"first","type":"user"}),
            TranscriptPhase::Replay,
        ),
        &[user_frame.clone()],
    );
    assistant.ordinal = 1;
    source.observe("session", &assistant, &[text("Answer")]);
    assert_eq!(
        source.staged.back().unwrap().checkpoint["sourceUser"],
        "native:claude:session:first:user"
    );
    source.observe(
        "session",
        &record(
            json!({"uuid":"second","type":"user"}),
            TranscriptPhase::Live,
        ),
        &[user_frame],
    );
    assistant.value["parentUuid"] = json!("second");
    source.observe("session", &assistant, &[text("Answer")]);
    assert_eq!(source.failure, Some(ObservationError::Identity));
}

#[test]
fn native_source_user_alias_is_fifo_and_native_occurrence_scoped() {
    let (mut source, batches) = fixture(ProviderId::Antigravity);
    source.expect_submission("Same", "first-send").unwrap();
    source.expect_submission("Same", "second-send").unwrap();
    for index in [0, 2] {
        source.observe("session", &record(json!({"step_index":index,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nSame\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nSynthetic time.\n</ADDITIONAL_METADATA>"}), TranscriptPhase::Live), &[json!({"v":1,"type":"user.message","ts":1,"content":[{"kind":"text","text":"native envelope"}]})]);
    }
    source.finish_poll(false, false, "source", false).unwrap();
    let batch = batches.lock().unwrap().pop().unwrap();
    let users = batch
        .records
        .iter()
        .filter(|row| row.checkpoint["kind"] == "user")
        .collect::<Vec<_>>();
    assert_eq!(users[0].input_binding.as_deref(), Some("first-send"));
    assert_eq!(users[1].input_binding.as_deref(), Some("second-send"));
    assert_ne!(users[0].occurrence, users[1].occurrence);
    assert_eq!(
        users[0].frame.as_ref().unwrap()["content"][0]["text"],
        "Same"
    );
}

#[test]
fn native_source_missing_tool_identity_and_ambiguous_user_wrapper_are_explicitly_incomplete() {
    let (mut source, batches) = fixture(ProviderId::Antigravity);
    source.observe("session", &record(json!({"step_index":1,"type":"GENERIC","source":"TOOL","status":"DONE","content":"Output without call ID"}), TranscriptPhase::Live), &[]);
    source.finish_poll(false, false, "source", false).unwrap();
    let batch = batches.lock().unwrap().pop().unwrap();
    assert!(!batch.complete);
    assert_eq!(
        antigravity_user("<USER_REQUEST>\nLiteral\n</USER_REQUEST>\nother input\n</USER_REQUEST>"),
        None
    );
    assert_eq!(
        antigravity_user("<USER_REQUEST>\nSão Paulo 🐣\n</USER_REQUEST>"),
        Some("São Paulo 🐣")
    );
}

#[test]
#[ignore = "reads only an explicitly supplied authenticated proof from this harness; no inference"]
fn controlled_native_source_proof_replays_into_private_journal_without_duplicate_content() {
    use prometeu_core::conversation::stream::TranscriptStore;
    use std::os::unix::fs::PermissionsExt;
    let proof_path = PathBuf::from(
        std::env::var_os("PROMETEU_SMOKE_SOURCE_PROOF").expect("provide the exact smoke proof"),
    );
    let root = proof_path.parent().unwrap();
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("prometeu-native-smoke-"));
    assert_eq!(
        std::fs::metadata(&proof_path).unwrap().permissions().mode() & 0o077,
        0
    );
    let proof: Value = serde_json::from_slice(&std::fs::read(&proof_path).unwrap()).unwrap();
    let provider = match proof["provider"].as_str().unwrap() {
        "claude" => ProviderId::Claude,
        "antigravity" => ProviderId::Antigravity,
        _ => panic!("unsupported proof"),
    };
    let native = proof["native_session"].as_str().unwrap();
    let transcript = PathBuf::from(proof["transcript"].as_str().unwrap());
    assert!(session_path(provider, &transcript, native));
    let directory = root.join(format!("source-replay-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let journal = crate::chat::terminal::ingestion::JournalStore::open(
        &directory.join("canonical.jsonl"),
        &directory.join("journal.sqlite"),
        crate::terminal_agent_journal::JournalBinding {
            app_session: "controlled-source-replay".into(),
            provider: proof["provider"].as_str().unwrap().into(),
            native_session: Some(native.into()),
        },
        false,
    )
    .unwrap();
    let mut saved = String::new();
    for pass in 0..2 {
        let batches = Arc::new(Mutex::new(VecDeque::new()));
        let output = batches.clone();
        let mut observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider,
                session: "controlled-source-replay".into(),
                process_nonce: format!("pass-{pass}"),
                expected_native_session: Some(native.into()),
                start: ObservationStart::Resume,
                authorized_roots: vec![transcript.parent().unwrap().into()],
            },
            ObservationCallbacks {
                live: Box::new(|_| panic!("source replay entered legacy live publication")),
                native_baseline: Box::new(|_| {}),
                accepted_prompt: Box::new(|_| panic!("source replay captured input")),
                observed_hook: Box::new(|_| {}),
                warning: Box::new(|_| {}),
            },
            Arc::new(TestClock),
        )
        .unwrap();
        observer
            .set_ingestion(Box::new(move |batch| {
                output.lock().unwrap().push_back(batch)
            }))
            .unwrap();
        observer.resume_from(transcript.clone()).unwrap();
        for _ in 0..256 {
            observer.poll().unwrap();
            while let Some(batch) = batches.lock().unwrap().pop_front() {
                assert!(batch.complete);
                assert_eq!(batch.phase, TranscriptPhase::Replay);
                journal.replay(&batch).unwrap();
                observer.acknowledge_ingestion(&batch.id).unwrap();
            }
            if observer.baseline_ready() && observer.caught_up() {
                break;
            }
        }
        assert!(observer.baseline_ready() && observer.caught_up());
        let content = journal.load().unwrap();
        assert!(content.contains("PROMETEU_TERMINAL_SMOKE"));
        if pass == 0 {
            saved = content;
        } else {
            assert_eq!(saved, content);
        }
    }
    println!(
        "controlled source replay and cold duplicate pass committed {} canonical lines",
        saved.lines().count()
    );
}
