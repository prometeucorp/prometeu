use super::*;
use serde_json::json;

#[test]
fn terminal_journal_prepared_inputs_survive_restart_without_authorizing_a_second_write() {
    let fixture = Fixture::new();
    let _log = fixture.log();
    let original = "Task";
    let input = PreparedInput {
        id: "input-one".into(),
        original: original.into(),
        original_hash: format!(
            "{:x}",
            Sha256::digest(
                json!([{"kind":"text","text":original}])
                    .to_string()
                    .as_bytes()
            )
        ),
        normalized_hash: "0".repeat(64),
        after_native_user: None,
        empty_baseline: true,
        at: 1,
    };
    {
        let mut journal = fixture.journal();
        journal.prepare_input(&input).unwrap();
    }
    let mut journal = fixture.journal();
    assert_eq!(journal.prepared_inputs().unwrap(), vec![input.clone()]);
    assert!(journal.prepare_input(&input).is_err());
    let mut other = input.clone();
    other.id = "input-two".into();
    assert!(journal.prepare_input(&other).is_err());
    assert!(fixture.bytes().is_empty());
    journal
        .connection
        .execute(
            "UPDATE prepared_inputs SET value=json_set(value,'$.original','Changed')",
            [],
        )
        .unwrap();
    assert!(journal.prepared_inputs().is_err());
    journal
        .connection
        .execute(
            "UPDATE prepared_inputs SET value=?1",
            ["x".repeat(2 * 1024 * 1024 + 1)],
        )
        .unwrap();
    assert!(journal.prepared_inputs().is_err());
}

#[test]
fn terminal_journal_peek_is_private_scoped_and_does_not_migrate_or_create_a_database() {
    let fixture = Fixture::new();
    assert_eq!(
        CaptureJournal::peek_binding(&fixture.path, "app-session", "cursor").unwrap(),
        None
    );
    assert!(!fixture.path.exists());
    let journal = fixture.journal();
    let expected = fixture.binding.clone();
    assert_eq!(
        CaptureJournal::peek_binding(&fixture.path, "app-session", "cursor").unwrap(),
        Some(expected)
    );
    assert!(matches!(
        CaptureJournal::peek_binding(&fixture.path, "other", "cursor"),
        Err(JournalError::Identity)
    ));
    journal
        .connection
        .execute_batch("PRAGMA user_version=1")
        .unwrap();
    assert!(
        CaptureJournal::peek_binding(&fixture.path, "app-session", "cursor")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        journal
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let link = fixture.root.join("symlink.sqlite");
    std::os::unix::fs::symlink(&fixture.path, &link).unwrap();
    assert!(matches!(
        CaptureJournal::peek_binding(&link, "app-session", "cursor"),
        Err(JournalError::Identity)
    ));
    std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        CaptureJournal::peek_binding(&fixture.path, "app-session", "cursor"),
        Err(JournalError::Identity)
    ));
}

struct Fixture {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    log_path: std::path::PathBuf,
    binding: JournalBinding,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("prometeu-journal-{}", uuid::Uuid::new_v4()));
        crate::paths::ensure_private_dir(&root).unwrap();
        Self {
            path: root.join("capture.sqlite3"),
            log_path: root.join("canonical.jsonl"),
            root,
            binding: JournalBinding {
                app_session: "app-session".into(),
                provider: "cursor".into(),
                native_session: Some("native-session".into()),
            },
        }
    }
    fn journal(&self) -> CaptureJournal {
        CaptureJournal::open(&self.path, &self.binding).unwrap()
    }
    fn log(&self) -> File {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&self.log_path)
            .unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.log_path).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn capture(key: &str, version: &str, position: CanonicalPosition) -> Capture {
    Capture {occurrence:key.into(),version:version.into(),checkpoint:json!({"root":"native-root","record":key}),position,canonical_lines:vec![json!({"v":1,"type":"assistant.block","at":10,"messageId":key,"index":0,"block":{"kind":"text","text":"Preserved answer 🦀"}}).to_string()]}
}
const EMPTY: CanonicalPosition = CanonicalPosition { bytes: 0, lines: 0 };

#[test]
fn terminal_journal_crash_windows_recover_exact_prefix_once_without_rewriting_old_lines() {
    for written in [0, 1, 17, usize::MAX] {
        let fixture = Fixture::new();
        let mut log = fixture.log();
        let notice=json!({"v":1,"type":"system.notice","at":1,"level":"info","code":"fixture","detail":"Keep anchored notice"}).to_string()+"\n";
        log.write_all(notice.as_bytes()).unwrap();
        let position = CanonicalPosition {
            bytes: notice.len() as u64,
            lines: 1,
        };
        let intent = validate(capture("message", "version", position)).unwrap();
        {
            let mut journal = fixture.journal();
            journal.prepare(&mut log, &intent).unwrap();
            let count = written.min(intent.bytes.len());
            log.seek(SeekFrom::End(0)).unwrap();
            log.write_all(&intent.bytes[..count]).unwrap();
            log.sync_data().unwrap();
        }
        let mut journal = fixture.journal();
        let recovered = journal.recover(&mut log).unwrap().unwrap();
        assert_eq!(
            recovered.appended_bytes,
            (intent.bytes.len() - written.min(intent.bytes.len())) as u64
        );
        assert_eq!(recovered.receipt.position, position);
        assert_eq!(
            fixture.bytes(),
            [notice.as_bytes(), intent.bytes.as_slice()].concat()
        );
        assert_eq!(journal.recover(&mut log).unwrap(), None);
        assert_eq!(
            journal
                .append(
                    &mut log,
                    capture("message", "version", recovered.receipt.end())
                )
                .unwrap(),
            AppendResult::AlreadyCommitted(recovered.receipt)
        );
        assert_eq!(
            fixture.bytes(),
            [notice.as_bytes(), intent.bytes.as_slice()].concat()
        );
        let pending: Option<Vec<u8>> = journal
            .connection
            .query_row("SELECT pending_bytes FROM captures", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            pending, None,
            "committed canonical content is not duplicated in the journal"
        );
    }
}

#[test]
fn terminal_journal_conflicts_preserve_both_log_and_intent_and_block_later_appends() {
    let fixture = Fixture::new();
    let mut log = fixture.log();
    let mut journal = fixture.journal();
    let intent = validate(capture("first", "v1", EMPTY)).unwrap();
    journal.prepare(&mut log, &intent).unwrap();
    assert_eq!(
        journal.append(&mut log, capture("second", "v1", EMPTY)),
        Err(JournalError::Pending)
    );
    log.write_all(b"foreign tail").unwrap();
    let before = fixture.bytes();
    assert_eq!(journal.recover(&mut log), Err(JournalError::Conflict));
    assert_eq!(fixture.bytes(), before);
    let pending: i64 = journal
        .connection
        .query_row(
            "SELECT count(*) FROM captures WHERE committed=0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
}

#[test]
fn terminal_journal_aliases_bind_repeated_prompt_echoes_to_distinct_occurrences() {
    let fixture = Fixture::new();
    let mut log = fixture.log();
    let mut journal = fixture.journal();
    let line =
        json!({"v":1,"type":"user.message","at":10,"content":[{"kind":"text","text":"Repeat"}]})
            .to_string();
    let mut position = EMPTY;
    for ordinal in 0..2 {
        let mut input = capture(&format!("submission-{ordinal}"), "accepted", position);
        input.canonical_lines = vec![line.clone()];
        let AppendResult::Appended(receipt) = journal.append(&mut log, input).unwrap() else {
            panic!()
        };
        position = receipt.end();
        let checkpoint = json!({"native_message_id":format!("native-{ordinal}")});
        let alias = journal
            .bind_alias(
                &format!("native-{ordinal}"),
                "blob-version",
                checkpoint.clone(),
                &receipt,
            )
            .unwrap();
        assert_eq!(alias.position, receipt.position);
        assert_eq!(
            journal
                .bind_alias(
                    &format!("native-{ordinal}"),
                    "blob-version",
                    checkpoint,
                    &receipt
                )
                .unwrap(),
            alias
        );
        assert_eq!(
            journal
                .lookup(&format!("native-{ordinal}"), "blob-version")
                .unwrap(),
            Some(alias)
        );
    }
    assert_eq!(fixture.bytes(), format!("{line}\n{line}\n").as_bytes());
    assert_eq!(position.lines, 2);
}

#[test]
fn terminal_journal_versions_append_and_retry_conflicts_do_not_silently_replace_content() {
    let fixture = Fixture::new();
    let mut log = fixture.log();
    let mut journal = fixture.journal();
    let AppendResult::Appended(first) = journal
        .append(&mut log, capture("step", "blob-one", EMPTY))
        .unwrap()
    else {
        panic!()
    };
    let mut changed = capture("step", "blob-one", first.end());
    changed.canonical_lines[0] =
        changed.canonical_lines[0].replace("Preserved answer", "Changed answer");
    assert_eq!(
        journal.append(&mut log, changed),
        Err(JournalError::Conflict)
    );
    let mut next = capture("step", "blob-two", first.end());
    next.canonical_lines[0] =
        next.canonical_lines[0].replace("Preserved answer", "Completed answer");
    let AppendResult::Appended(second) = journal.append(&mut log, next).unwrap() else {
        panic!()
    };
    assert_eq!(second.position, first.end());
    assert_eq!(second.end().lines, 2);
    assert_eq!(journal.latest("step").unwrap(), Some(second));
    journal
        .append(&mut log, capture("step", "blob-one", first.end()))
        .unwrap();
    assert_eq!(journal.latest("step").unwrap().unwrap().version, "blob-two");
    let bytes = fixture.bytes();
    assert!(String::from_utf8(bytes)
        .unwrap()
        .contains("Preserved answer"));
}

#[test]
fn terminal_journal_recovery_preserves_complete_tail_and_rejects_damaged_intent() {
    let fixture = Fixture::new();
    let mut log = fixture.log();
    let mut journal = fixture.journal();
    let intent = validate(capture("message", "version", EMPTY)).unwrap();
    journal.prepare(&mut log, &intent).unwrap();
    log.write_all(&intent.bytes).unwrap();
    let notice = b"{\"v\":1,\"type\":\"system.notice\"}\n";
    log.write_all(notice).unwrap();
    assert_eq!(
        journal.recover(&mut log).unwrap().unwrap().appended_bytes,
        0
    );
    assert_eq!(fixture.bytes(), [intent.bytes.as_slice(), notice].concat());

    let next = CanonicalPosition {
        bytes: fixture.bytes().len() as u64,
        lines: 2,
    };
    let intent = validate(capture("next", "version", next)).unwrap();
    journal.prepare(&mut log, &intent).unwrap();
    journal
        .connection
        .execute(
            "UPDATE captures SET pending_bytes=zeroblob(?1) WHERE committed=0",
            [BATCH_LIMIT as i64 + 100],
        )
        .unwrap();
    let before = fixture.bytes();
    assert_eq!(journal.recover(&mut log), Err(JournalError::Conflict));
    assert_eq!(fixture.bytes(), before);
    journal
        .connection
        .execute(
            "UPDATE captures SET checkpoint=?1 WHERE committed=0",
            ["a".repeat(CHECKPOINT_LIMIT + 1)],
        )
        .unwrap();
    assert_eq!(journal.recover(&mut log), Err(JournalError::Storage));
    assert_eq!(fixture.bytes(), before);
}

#[test]
fn terminal_journal_checks_binding_versions_limits_and_never_persists_ephemeral_or_private_frames()
{
    let fixture = Fixture::new();
    let mut log = fixture.log();
    let mut journal = fixture.journal();
    let mut other = fixture.binding.clone();
    other.native_session = Some("other".into());
    assert!(matches!(
        CaptureJournal::open(&fixture.path, &other),
        Err(JournalError::Identity)
    ));
    for frame in [
        json!({"v":1,"type":"execution.observed"}),
        json!({"v":1,"type":"turn.completed","telemetry":{}}),
        json!({"v":2,"type":"user.message"}),
    ] {
        let mut invalid = capture("record", "v", EMPTY);
        invalid.canonical_lines = vec![frame.to_string()];
        assert_eq!(
            journal.append(&mut log, invalid),
            Err(JournalError::Invalid)
        );
    }
    let mut invalid = capture("record", "v", EMPTY);
    invalid.checkpoint = json!({"large":"a".repeat(CHECKPOINT_LIMIT)});
    assert_eq!(
        journal.append(&mut log, invalid),
        Err(JournalError::Invalid)
    );
    assert!(fixture.bytes().is_empty());
    assert_eq!(
        std::fs::metadata(&fixture.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    journal
        .connection
        .execute_batch("PRAGMA user_version=5")
        .unwrap();
    assert!(matches!(
        CaptureJournal::open(&fixture.path, &fixture.binding),
        Err(JournalError::Schema)
    ));
    assert_eq!(
        journal
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        5
    );
}

#[test]
fn terminal_journal_provisional_echo_survives_crash_and_native_identity_binds_once() {
    let mut fixture = Fixture::new();
    fixture.binding.native_session = None;
    let mut log = fixture.log();
    let intent = validate(capture("submission-one", "accepted", EMPTY)).unwrap();
    {
        let mut journal = fixture.journal();
        journal.prepare(&mut log, &intent).unwrap();
        log.write_all(&intent.bytes[..17]).unwrap();
    }
    let mut journal = fixture.journal();
    let recovered = journal.recover(&mut log).unwrap().unwrap();
    let before = fixture.bytes();
    assert_eq!(before, intent.bytes);
    journal.bind_native("native-discovered").unwrap();
    journal.bind_native("native-discovered").unwrap();
    assert_eq!(
        journal.bind_native("foreign-session"),
        Err(JournalError::Identity)
    );
    assert_eq!(journal.bind_native(""), Err(JournalError::Identity));
    assert_eq!(fixture.bytes(), before);
    assert_eq!(
        journal.lookup("submission-one", "accepted").unwrap(),
        Some(recovered.receipt)
    );
    drop(journal);
    assert!(
        matches!(
            CaptureJournal::open(&fixture.path, &fixture.binding),
            Err(JournalError::Identity)
        ),
        "reopening cannot remove the native binding"
    );
    fixture.binding.native_session = Some("native-discovered".into());
    let reopened = fixture.journal();
    assert_eq!(reopened.binding, fixture.binding);
    assert_eq!(
        reopened
            .lookup("submission-one", "accepted")
            .unwrap()
            .unwrap()
            .position,
        EMPTY
    );
}

#[test]
fn terminal_journal_older_schema_migrations_preserve_receipts_bytes_and_identity() {
    for version in [1, 2, 3] {
        let fixture = Fixture::new();
        let mut log = fixture.log();
        let receipt = {
            let mut journal = fixture.journal();
            let AppendResult::Appended(receipt) = journal
                .append(&mut log, capture("step", "version", EMPTY))
                .unwrap()
            else {
                panic!()
            };
            journal
                .connection
                .execute_batch(&format!(
                    "DROP INDEX completed_source_user; DROP INDEX source_user_order; {} PRAGMA user_version={version}",
                    if version < 3 {
                        "DROP TABLE prepared_inputs;"
                    } else {
                        ""
                    }
                ))
                .unwrap();
            receipt
        };
        let before = fixture.bytes();
        let mut other = fixture.binding.clone();
        other.native_session = Some("foreign-session".into());
        assert!(matches!(
            CaptureJournal::open(&fixture.path, &other),
            Err(JournalError::Identity)
        ));
        let journal = fixture.journal();
        assert_eq!(
            journal
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            SCHEMA
        );
        assert_eq!(journal.lookup("step", "version").unwrap(), Some(receipt));
        assert_eq!(fixture.bytes(), before);
        assert_eq!(journal.binding, fixture.binding);
    }
}

#[test]
fn native_user_completion_requires_durable_public_bytes_and_survives_recovery() {
    let fixture = Fixture::new();
    let mut journal = fixture.journal();
    let mut log = fixture.log();
    let mut completed = capture("completion", "once", EMPTY);
    completed.checkpoint = json!({"kind":"completion","sourceUser":"native-user:first"});
    completed.canonical_lines = vec![json!({"v":1,"type":"turn.completed","at":1,"outcome":"ok","message":"","durationMs":null,"costUsd":null}).to_string()];
    let intent = validate(completed).unwrap();
    journal.prepare(&mut log, &intent).unwrap();
    assert!(!journal.completed_source_user("native-user:first").unwrap());
    drop(journal);
    let mut journal = fixture.journal();
    journal.recover(&mut log).unwrap();
    assert!(journal.completed_source_user("native-user:first").unwrap());
    assert!(!journal.completed_source_user("native-user:second").unwrap());
    let position = intent.receipt.end();
    journal.append(&mut log, Capture {
        occurrence:"row:checkpoint".into(), version:"sealed".into(),
        checkpoint:json!({"kind":"turn","status":"completed","sourceScope":"record","sourceUser":"native-user:second"}),
        position, canonical_lines:vec![],
    }).unwrap();
    assert!(!journal.completed_source_user("native-user:second").unwrap());
}
