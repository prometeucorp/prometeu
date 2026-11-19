use super::*;
use std::{cell::RefCell, collections::BTreeMap};

// Synthetic fixtures follow Cursor CLI 2026.11.13-eacade1's generated agent.v1 descriptors
// in index.js and agent-kv/AgentStore + SQLiteBlobStore in 7693.index.js. No private stores.
#[derive(Default)]
struct Blobs {
    values: BTreeMap<BlobId, Vec<u8>>,
    reads: RefCell<Vec<(BlobId, usize)>>,
}
impl Blobs {
    fn put(&mut self, value: Vec<u8>) -> BlobId {
        let id = BlobId(Sha256::digest(&value).into());
        self.values.insert(id, value);
        id
    }
}
impl BlobSource for Blobs {
    fn len(&self, id: BlobId) -> Result<u64, StoreError> {
        Ok(self.values.get(&id).ok_or(StoreError::Missing)?.len() as u64)
    }
    fn read_at(&self, id: BlobId, offset: u64, destination: &mut [u8]) -> Result<(), StoreError> {
        let value = self.values.get(&id).ok_or(StoreError::Missing)?;
        let offset = usize::try_from(offset).map_err(|_| StoreError::Read)?;
        let source = value
            .get(
                offset
                    ..offset
                        .checked_add(destination.len())
                        .ok_or(StoreError::Read)?,
            )
            .ok_or(StoreError::Read)?;
        destination.copy_from_slice(source);
        self.reads.borrow_mut().push((id, destination.len()));
        Ok(())
    }
}
fn var(mut value: u64) -> Vec<u8> {
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
fn field(number: u32, bytes: &[u8]) -> Vec<u8> {
    [
        var((u64::from(number) << 3) | 2),
        var(bytes.len() as u64),
        bytes.to_vec(),
    ]
    .concat()
}
fn integer(number: u32, value: u64) -> Vec<u8> {
    [var(u64::from(number) << 3), var(value)].concat()
}
fn user(store: &mut Blobs, id: &str, text: &str) -> BlobId {
    store.put([field(1, text.as_bytes()), field(2, id.as_bytes())].concat())
}
fn agent(store: &mut Blobs, user: BlobId, request: &str, run: &str, steps: &[BlobId]) -> BlobId {
    let run = [
        field(1, run.as_bytes()),
        integer(2, 2),
        integer(3, 10),
        integer(4, 20),
    ]
    .concat();
    let mut body = [
        field(1, &user.0),
        field(3, request.as_bytes()),
        field(17, &run),
    ]
    .concat();
    for step in steps {
        body.extend(field(2, &step.0));
    }
    store.put(field(1, &body))
}
const SCHEMA: CursorStoreSchema = CursorStoreSchema::Cli20261113;

#[test]
fn cursor_root_run_metadata_retains_identity_and_never_defaults_missing_status_to_completed() {
    let mut store = Blobs::default();
    for (raw, expected) in [
        (0, CursorRunStatus::Unknown),
        (1, CursorRunStatus::Running),
        (2, CursorRunStatus::Completed),
        (3, CursorRunStatus::Failed),
        (4, CursorRunStatus::Interrupted),
    ] {
        let id = store.put(field(
            52,
            &[field(1, b"run-current"), integer(2, raw), integer(4, 20)].concat(),
        ));
        let root = root(&store, id, SCHEMA).unwrap();
        let run = root.last_run.unwrap();
        assert_eq!(run.id, "run-current");
        assert_eq!(run.status, expected);
        assert_eq!(run.finished_at_ms, Some(20));
    }
    let id = store.put(Vec::new());
    assert!(root(&store, id, SCHEMA).unwrap().last_run.is_none());
    let id = store.put(field(
        52,
        &[field(1, b"run-current"), integer(2, 999)].concat(),
    ));
    assert_eq!(root(&store, id, SCHEMA), Err(StoreError::Unsupported));
}

fn tool_step(store: &mut Blobs, kind: u32, id: &str, body: &[u8]) -> BlobId {
    store.put(field(
        2,
        &[field(kind, body), field(57, id.as_bytes())].concat(),
    ))
}

#[test]
fn cursor_tool_ids_preserve_native_newline_separators_without_becoming_path_identities() {
    let mut store = Blobs::default();
    let native = "model-call\nexecution-call";
    let id = tool_step(
        &mut store,
        1,
        native,
        &field(1, &field(4, native.as_bytes())),
    );
    let decoded = tool(&store, &step(&store, id, SCHEMA).unwrap(), SCHEMA).unwrap();
    assert_eq!(decoded.id, native);
    assert!(!valid_id(native));
    assert_eq!(
        serde_json::to_string(&decoded.id).unwrap(),
        "\"model-call\\nexecution-call\""
    );
}

#[test]
fn cursor_tool_projection_keeps_native_identity_and_separate_pending_and_result_revisions() {
    let mut store = Blobs::default();
    let args = [
        field(1, b"printf hello"),
        field(2, b"/tmp/work"),
        field(4, b"call"),
        integer(12, 1),
    ]
    .concat();
    let pending = tool_step(&mut store, 1, "call", &field(1, &args));
    let complete = tool_step(
        &mut store,
        1,
        "call",
        &[field(1, &args), field(2, &field(1, &field(5, b"hello")))].concat(),
    );
    let input = user(&mut store, "user", "Run the command");
    let turn = agent(&mut store, input, "request", "run", &[pending]);
    let root = store.put(field(8, &turn.0));
    let before = projected(&store, root);
    assert!(before.complete);
    let first = before
        .records
        .iter()
        .find(|record| record.checkpoint["kind"] == "tool")
        .unwrap();
    let frame = first.frame.as_ref().unwrap();
    assert_eq!(frame["block"]["name"], "Shell");
    assert_eq!(frame["block"]["id"], "call");
    assert_eq!(frame["block"]["input"]["command"], "printf hello");
    assert!(frame["block"]["input"].get("skip_approval").is_none());
    assert!(!before
        .records
        .iter()
        .any(|record| record.checkpoint["kind"] == "result"));
    let turn = agent(&mut store, input, "request", "run", &[complete]);
    let root = store.put(field(8, &turn.0));
    let after = projected(&store, root);
    let second = after
        .records
        .iter()
        .find(|record| record.checkpoint["kind"] == "tool")
        .unwrap();
    assert_eq!(first.occurrence, second.occurrence);
    assert_ne!(first.version, second.version);
    let result = after
        .records
        .iter()
        .find(|record| record.checkpoint["kind"] == "result")
        .unwrap();
    assert_ne!(first.occurrence, result.occurrence);
    assert_eq!(result.frame.as_ref().unwrap()["toolId"], "call");
    assert_eq!(result.frame.as_ref().unwrap()["output"], "hello");
    assert_eq!(result.frame.as_ref().unwrap()["error"], false);
    assert!(after.complete);
    assert!(after.records.iter().all(|record| record
        .frame
        .as_ref()
        .is_none_or(
            |frame| frame["type"] != "request.opened" && frame["type"] != "turn.completed"
        )));
}

#[test]
fn cursor_tools_preserve_mcp_values_text_and_structured_output_without_policy_metadata() {
    let mut store = Blobs::default();
    // MCP args are protobuf map<string, google.protobuf.Value>; the two fields around it
    // exercise map decoding inside a message rather than in a standalone map envelope.
    let entry = |key: &str, value: &[u8]| [field(1, key.as_bytes()), field(2, value)].concat();
    let args = [
        field(1, b"mcp__documents__search"),
        field(2, &entry("query", &field(3, b"same text"))),
        field(2, &entry("exact", &integer(4, 1))),
        field(3, b"mcp-call"),
        field(9, b"documents"),
    ]
    .concat();
    let content = field(1, &field(1, b"Found a document"));
    let structured = field(
        1,
        &entry(
            "count",
            &[var((2 << 3) | 1), 1_f64.to_le_bytes().to_vec()].concat(),
        ),
    );
    let success = [field(1, &content), field(3, &structured)].concat();
    let id = tool_step(
        &mut store,
        15,
        "mcp-call",
        &[field(1, &args), field(2, &field(1, &success))].concat(),
    );
    let value = tool(&store, &step(&store, id, SCHEMA).unwrap(), SCHEMA).unwrap();
    assert_eq!(value.name, "mcp__documents__search");
    assert_eq!(
        value.input,
        serde_json::json!({"query":"same text","exact":true})
    );
    let result = value.result.unwrap();
    assert_eq!(result.output, "Found a document\n{\"count\":1.0}");
    assert!(!result.error);
}

#[test]
fn cursor_read_blob_and_truncated_tool_recover_original_content_with_integrity_checks() {
    let mut store = Blobs::default();
    let content = store.put(b"Original file text\n".to_vec());
    let original = tool_step(
        &mut store,
        8,
        "read-call",
        &[
            field(1, &field(1, b"file.txt")),
            field(2, &field(1, &field(10, &content.0))),
        ]
        .concat(),
    );
    let truncated = tool_step(&mut store, 34, "read-call", &field(1, &original.0));
    let value = tool(&store, &step(&store, truncated, SCHEMA).unwrap(), SCHEMA).unwrap();
    assert_eq!(value.id, "read-call");
    assert_eq!(value.name, "Read");
    assert_eq!(value.result.unwrap().output, "Original file text\n");
    let conflict = tool_step(&mut store, 34, "other-call", &field(1, &original.0));
    assert_eq!(
        tool(&store, &step(&store, conflict, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Identity)
    );
    store.values.get_mut(&content).unwrap()[0] ^= 1;
    assert_eq!(
        tool(&store, &step(&store, truncated, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Integrity)
    );
}

#[test]
fn cursor_tool_binary_payloads_are_bounded_and_errors_and_background_remain_explicit() {
    let mut store = Blobs::default();
    let binary = store.put(vec![0xff; 6 * 1024 * 1024]);
    let id = tool_step(
        &mut store,
        8,
        "read-binary",
        &field(2, &field(1, &field(9, &binary.0))),
    );
    let value = tool(&store, &step(&store, id, SCHEMA).unwrap(), SCHEMA).unwrap();
    assert_eq!(
        value.result.unwrap().output,
        "{\"data\":{\"bytes\":6291456,\"omitted\":true}}"
    );
    assert!(store.reads.borrow().iter().all(|(_, size)| *size <= BUFFER));
    let result = [
        field(
            2,
            &[integer(3, u64::MAX), field(6, b"command failed")].concat(),
        ),
        integer(102, 1),
    ]
    .concat();
    let id = tool_step(&mut store, 1, "shell-failure", &field(2, &result));
    let value = tool(&store, &step(&store, id, SCHEMA).unwrap(), SCHEMA)
        .unwrap()
        .result
        .unwrap();
    assert!(value.error && value.background);
    assert_eq!(value.output, "command failed");
}

#[test]
fn cursor_tool_unknown_fields_and_conflicting_results_never_claim_complete_coverage() {
    let mut store = Blobs::default();
    let unknown = tool_step(&mut store, 4095, "unknown", &[]);
    assert_eq!(
        tool(&store, &step(&store, unknown, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Unsupported)
    );
    let unknown = tool_step(&mut store, 1, "unknown-field", &field(4095, b"future"));
    assert_eq!(
        tool(&store, &step(&store, unknown, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Unsupported)
    );
    let conflict = tool_step(
        &mut store,
        1,
        "conflicting-result",
        &field(2, &[field(1, &[]), field(2, &[])].concat()),
    );
    assert_eq!(
        tool(&store, &step(&store, conflict, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Malformed)
    );
    let args = [field(1, &vec![b'a'; TEXT_LIMIT]), field(2, b"overflow")].concat();
    let large = tool_step(&mut store, 1, "large", &field(1, &args));
    assert_eq!(
        tool(&store, &step(&store, large, SCHEMA).unwrap(), SCHEMA),
        Err(StoreError::Limit)
    );
}

fn projected(store: &Blobs, id: BlobId) -> projection::CursorProjectionBatch {
    projection::CursorProjection::new("session", root(store, id, SCHEMA).unwrap(), 50)
        .unwrap()
        .next(store, SCHEMA)
        .unwrap()
}

#[test]
fn cursor_projection_updates_native_steps_without_reusing_identity_for_repeated_text_or_new_runs() {
    let mut store = Blobs::default();
    let user_one = user(&mut store, "user-one", "Repeat");
    let user_two = user(&mut store, "user-two", "Repeat");
    let partial = store.put(field(1, &field(1, b"Same answer")));
    let full = store.put(field(1, &field(1, b"Same answer complete")));
    let turn_one = agent(&mut store, user_one, "request-one", "run-one", &[partial]);
    let turn_two = agent(&mut store, user_two, "request-two", "run-two", &[partial]);
    let before = store.put([field(8, &turn_one.0), field(8, &turn_two.0)].concat());
    let first = projected(&store, before);
    assert!(first.finished && first.complete);
    let users = first
        .records
        .iter()
        .filter(|record| record.checkpoint["kind"] == "user")
        .collect::<Vec<_>>();
    assert_ne!(users[0].occurrence, users[1].occurrence);
    assert_eq!(
        users[0].frame.as_ref().unwrap()["content"],
        users[1].frame.as_ref().unwrap()["content"]
    );
    let answers = first
        .records
        .iter()
        .filter(|record| record.checkpoint["kind"] == "text")
        .collect::<Vec<_>>();
    assert_ne!(answers[0].occurrence, answers[1].occurrence);
    let updated = agent(&mut store, user_one, "request-one", "run-one", &[full]);
    // Compaction changes root prompt/summary references, without changing retained native turns.
    let compacted = store.put(
        [
            field(8, &updated.0),
            field(8, &turn_two.0),
            field(1, &[1; 32]),
            field(13, &[2; 32]),
        ]
        .concat(),
    );
    let next = projected(&store, compacted);
    let update = next
        .records
        .iter()
        .find(|record| record.occurrence == answers[0].occurrence)
        .unwrap();
    assert_ne!(update.version, answers[0].version);
    assert_eq!(
        update.frame.as_ref().unwrap()["messageId"],
        answers[0].frame.as_ref().unwrap()["messageId"]
    );
    assert_eq!(
        update.frame.as_ref().unwrap()["index"],
        answers[0].frame.as_ref().unwrap()["index"]
    );
    assert!(next.records.contains(answers[1]));
    let rerun = agent(&mut store, user_one, "request-new", "run-new", &[full]);
    let reroot = store.put(field(8, &rerun.0));
    let next = projected(&store, reroot);
    assert_eq!(
        next.records[0].occurrence, users[0].occurrence,
        "a rerun does not create a second user submission"
    );
    assert_ne!(next.records[1].occurrence, answers[0].occurrence);
    assert!(next.records.iter().all(|record| record
        .frame
        .as_ref()
        .is_none_or(|frame| frame["type"] != "turn.completed")));
}

#[test]
fn cursor_projection_batches_large_histories_and_withholds_coverage_for_unsupported_tools() {
    let mut store = Blobs::default();
    let user = user(&mut store, "user", "Task");
    let answer = store.put(field(1, &field(1, b"Repeated step")));
    let tool = store.put(field(
        2,
        &[field(4095, &field(1, b"command")), field(57, b"tool-id")].concat(),
    ));
    let mut steps = vec![answer; 150];
    steps.push(tool);
    let turn = agent(&mut store, user, "request", "run", &steps);
    let root_id = store.put(field(8, &turn.0));
    let mut projector =
        projection::CursorProjection::new("session", root(&store, root_id, SCHEMA).unwrap(), 0)
            .unwrap();
    let mut records = Vec::new();
    let mut batches = 0;
    loop {
        let batch = projector.next(&store, SCHEMA).unwrap();
        assert!(batch.records.len() <= 64);
        batches += 1;
        records.extend(batch.records);
        if batch.finished {
            assert!(!batch.complete);
            break;
        }
    }
    assert_eq!(batches, 3);
    assert_eq!(records.len(), 153);
    assert_eq!(
        records
            .iter()
            .map(|record| &record.occurrence)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        153
    );
    assert!(records
        .iter()
        .find(|record| record.checkpoint["kind"] == "unsupported_tool")
        .unwrap()
        .frame
        .is_none());
}

#[test]
fn cursor_store_repeated_text_has_distinct_native_identity_and_compaction_preserves_turns() {
    let mut store = Blobs::default();
    let one = user(&mut store, "user-one", "Repeat this");
    let two = user(&mut store, "user-two", "Repeat this");
    let one = agent(&mut store, one, "request-one", "run-one", &[]);
    let two = agent(&mut store, two, "request-two", "run-two", &[]);
    let before = store.put([field(8, &one.0), field(8, &two.0), field(1, &[1; 32])].concat());
    let after = store.put(
        [
            field(13, &[2; 32]),
            field(8, &one.0),
            field(8, &two.0),
            field(1, &[3; 32]),
        ]
        .concat(),
    );
    assert_eq!(
        root(&store, before, SCHEMA).unwrap().turns,
        root(&store, after, SCHEMA).unwrap().turns
    );
    let CursorTurn::Agent {
        key: first,
        user: a,
        ..
    } = turn(&store, one, SCHEMA).unwrap()
    else {
        panic!()
    };
    let CursorTurn::Agent {
        key: second,
        user: b,
        run,
        ..
    } = turn(&store, two, SCHEMA).unwrap()
    else {
        panic!()
    };
    assert_eq!(a.text, b.text);
    assert_ne!(first, second);
    assert_eq!(run.unwrap().status, CursorRunStatus::Completed);
    assert!(!store
        .reads
        .borrow()
        .iter()
        .any(|(id, _)| id.0 == [1; 32] || id.0 == [2; 32] || id.0 == [3; 32]));
}

#[test]
fn cursor_store_new_run_and_mutable_step_keep_separate_identity_and_content_version() {
    let mut store = Blobs::default();
    let user = user(&mut store, "message", "Task");
    let first = store.put(field(1, &field(1, b"Part")));
    let second = store.put(field(1, &field(1, b"Part complete")));
    let a = agent(&mut store, user, "request", "run", &[first]);
    let b = agent(&mut store, user, "request", "run", &[second]);
    let c = agent(&mut store, user, "next-request", "next-run", &[second]);
    let key = |id| match turn(&store, id, SCHEMA).unwrap() {
        CursorTurn::Agent { key, .. } => key,
        _ => panic!(),
    };
    assert_eq!(key(a), key(b));
    assert_ne!(key(a), key(c));
    let a = step(&store, first, SCHEMA).unwrap();
    let b = step(&store, second, SCHEMA).unwrap();
    assert_ne!(a.version, b.version);
    assert_eq!(
        b.content,
        CursorStepContent::Assistant("Part complete".into())
    );
}

#[test]
fn cursor_store_skips_large_binary_context_but_validates_its_hash_and_keeps_user_text() {
    let mut store = Blobs::default();
    let user = store.put(
        [
            field(3, &vec![0xff; 2 * 1024 * 1024]),
            field(1, "Read image 🦀".as_bytes()),
            field(2, b"image-user"),
        ]
        .concat(),
    );
    let turn_id = agent(&mut store, user, "request", "run", &[]);
    let CursorTurn::Agent { user: read, .. } = turn(&store, turn_id, SCHEMA).unwrap() else {
        panic!()
    };
    assert_eq!(read.text, "Read image 🦀");
    assert!(store
        .reads
        .borrow()
        .iter()
        .all(|(_, length)| *length <= BUFFER));
    store.values.get_mut(&user).unwrap()[8] ^= 1;
    assert_eq!(turn(&store, turn_id, SCHEMA), Err(StoreError::Integrity));
}

#[test]
fn cursor_store_tool_identity_is_native_and_opaque_payload_never_becomes_a_request() {
    let mut store = Blobs::default();
    let tool = [
        field(1, &field(1, b"command")),
        field(57, b"native-call"),
        integer(59, 3),
        integer(60, 4),
    ]
    .concat();
    let id = store.put(field(2, &tool));
    let decoded = step(&store, id, SCHEMA).unwrap();
    let CursorStepContent::Tool {
        id: call,
        kind,
        payload,
    } = decoded.content
    else {
        panic!()
    };
    assert_eq!(call.as_deref(), Some("native-call"));
    assert_eq!(kind, 1);
    assert_eq!(payload.blob, id);
    assert_eq!(decoded.completed_at_ms, Some(4));
}

#[test]
fn cursor_store_rejects_foreign_metadata_subagents_unknown_schema_and_malformed_wire() {
    let hex = |value: Value| {
        value
            .to_string()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let value = serde_json::json!({"agentId":"session","latestRootBlobId":"","blobEncryptionKey":"private"});
    assert_eq!(
        metadata(&hex(value.clone()), "session").unwrap(),
        CursorMetadata {
            native_session: "session".into(),
            root: None
        }
    );
    assert_eq!(
        metadata(&hex(value.clone()), "other"),
        Err(StoreError::Identity)
    );
    assert_eq!(
        metadata(
            &hex(serde_json::json!({"subagentInfo":{},"agentId":"session","latestRootBlobId":""})),
            "session"
        ),
        Err(StoreError::Identity)
    );
    assert!(CursorStoreSchema::for_version("future").is_none());
    for bytes in [
        vec![0],
        vec![0xff; 11],
        vec![10, 128],
        vec![10, 4, 1],
        vec![11],
        [field(1, &[]), field(1, &[])].concat(),
    ] {
        let mut store = Blobs::default();
        let id = store.put(bytes);
        assert!(turn(&store, id, SCHEMA).is_err());
    }
    let mut store = Blobs::default();
    let id = store.put(field(1, &field(1, &vec![b'a'; TEXT_LIMIT + 1])));
    assert_eq!(step(&store, id, SCHEMA), Err(StoreError::Limit));
}
