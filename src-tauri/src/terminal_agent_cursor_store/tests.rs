use super::*;
use sha2::{Digest, Sha256};

struct Fixture {
    root: PathBuf,
    session: PathBuf,
    staging: PathBuf,
    connection: Connection,
    blob: BlobId,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("prometeu-cursor-store-{}", uuid::Uuid::new_v4()));
        let session = root.join("native-session");
        crate::paths::ensure_private_dir(&session).unwrap();
        let connection = Connection::open(session.join("store.db")).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; PRAGMA user_version=1; CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB); CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);").unwrap();
        let bytes = b"root fixture bytes";
        let blob = BlobId(Sha256::digest(bytes).into());
        connection
            .execute(
                "INSERT INTO blobs VALUES (?1,?2)",
                rusqlite::params![blob.hex(), bytes.as_slice()],
            )
            .unwrap();
        let mut fixture = Self {
            staging: root.join("staging"),
            root,
            session,
            connection,
            blob,
        };
        fixture.meta("native-session");
        fixture
    }
    fn meta(&mut self, session: &str) {
        let metadata = serde_json::json!({"agentId":session,"latestRootBlobId":self.blob.hex(),"blobEncryptionKey":"private fixture credential"});
        let value = metadata
            .to_string()
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        self.connection
            .execute("INSERT OR REPLACE INTO meta VALUES ('0',?1)", [value])
            .unwrap();
    }
    fn reader(&self) -> CursorStoreReader {
        CursorStoreReader::new(
            self.session.clone(),
            "native-session".into(),
            "2026.11.13-eacade1",
            self.staging.clone(),
        )
        .unwrap()
    }
    fn unchanged(&self) -> Vec<(String, Vec<u8>, Stamp, u32)> {
        ["store.db", "store.db-wal", "store.db-shm"]
            .into_iter()
            .filter_map(|name| {
                let path = self.session.join(name);
                let metadata = std::fs::metadata(&path).ok()?;
                Some((
                    name.into(),
                    std::fs::read(path).unwrap(),
                    Stamp::from(&metadata),
                    metadata.permissions().mode(),
                ))
            })
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn drain(reader: &mut CursorStoreReader) -> Result<(), CursorReadError> {
    for _ in 0..32 {
        if reader.poll()? == CursorReadProgress::Ready {
            return Ok(());
        }
    }
    panic!("bounded synthetic fixture did not finish copying")
}

#[test]
#[ignore = "requires an explicit hook-authenticated synthetic smoke proof; never scans histories"]
fn controlled_cursor_store_smoke_decodes_only_the_authorized_session() {
    let proof_path =
        std::env::var_os("PROMETEU_CURSOR_PROOF").expect("explicit smoke proof required");
    let proof: serde_json::Value =
        serde_json::from_slice(&std::fs::read(proof_path).unwrap()).unwrap();
    assert_eq!(proof["provider"], "cursor");
    let session = proof["native_session"].as_str().unwrap();
    let root = PathBuf::from(
        proof["scoped_store_root"]
            .as_str()
            .expect("explicit scoped root required"),
    );
    let stage = PathBuf::from(proof["test_root"].as_str().unwrap()).join("store-validation");
    let mut reader = CursorStoreReader::new(
        root,
        session.into(),
        proof["version"].as_str().unwrap(),
        stage,
    )
    .unwrap();
    drain(&mut reader).unwrap();
    let snapshot = reader.snapshot().unwrap();
    let root =
        cursor_store::root(snapshot, snapshot.metadata.root.unwrap(), reader.schema()).unwrap();
    if let Some(hooks) = proof["hooks"].as_array() {
        let generation = hooks
            .iter()
            .rev()
            .find(|hook| hook["kind"] == "StopObserved")
            .and_then(|hook| hook["generation_id"].as_str());
        if let Some(generation) = generation {
            let turn =
                cursor_store::turn(snapshot, *root.turns.last().unwrap(), reader.schema()).unwrap();
            let cursor_store::CursorTurn::Agent { key, run, .. } = turn else {
                panic!("expected controlled agent turn")
            };
            assert_eq!(key.request_id.as_deref(), Some(generation));
            assert!(run
                .as_ref()
                .is_none_or(|run| run.status == cursor_store::CursorRunStatus::Completed));
            assert!(root
                .last_run
                .as_ref()
                .is_none_or(|run| run.status == cursor_store::CursorRunStatus::Completed));
            assert_eq!(
                root.last_run.as_ref().map(|run| &run.id),
                run.as_ref().map(|run| &run.id)
            );
            println!("Controlled Cursor hook generation matches native request; turn_run_present={} root_run_present={}", run.is_some(), root.last_run.is_some());
        }
    }
    let mut projection =
        prometeu_protocols::cursor_store::projection::CursorProjection::new(session, root, 1)
            .unwrap();
    let mut users = 0;
    let mut answers = 0;
    let mut tools = Vec::new();
    let mut results = Vec::new();
    loop {
        let batch = projection.next(snapshot, reader.schema()).unwrap();
        for record in &batch.records {
            users += usize::from(record.checkpoint["kind"] == "user");
            answers += usize::from(record.checkpoint["kind"] == "text");
            if record.checkpoint["kind"] == "tool" {
                tools.push(record.frame.clone().unwrap());
            }
            if record.checkpoint["kind"] == "result" {
                results.push(record.frame.clone().unwrap());
            }
        }
        if batch.finished {
            assert!(batch.complete, "unsupported content in controlled smoke");
            break;
        }
    }
    assert!(
        users > 0 && answers > 0,
        "controlled prompt and answer were not found"
    );
    if std::env::var_os("PROMETEU_CURSOR_EXPECT_TOOL").is_some() {
        assert_eq!(tools.len(), 1, "controlled smoke uses exactly one tool");
        assert_eq!(
            results.len(),
            1,
            "controlled tool result was not captured once"
        );
        assert_eq!(tools[0]["block"]["name"], "Shell");
        assert_eq!(
            tools[0]["block"]["input"]["command"],
            "printf PROMETEU_TERMINAL_TOOL_SMOKE"
        );
        assert_eq!(results[0]["toolId"], tools[0]["block"]["id"]);
        assert_eq!(results[0]["error"], false);
        assert_eq!(results[0]["output"], "PROMETEU_TERMINAL_TOOL_SMOKE");
    }
}

#[test]
fn cursor_store_snapshot_reads_committed_wal_without_mutating_original_db_wal_or_shm() {
    let fixture = Fixture::new();
    for name in ["store.db", "store.db-wal", "store.db-shm"] {
        std::fs::set_permissions(
            fixture.session.join(name),
            std::fs::Permissions::from_mode(0o400),
        )
        .unwrap();
    }
    let before = fixture.unchanged();
    let mut reader = fixture.reader();
    drain(&mut reader).unwrap();
    assert_eq!(reader.schema(), CursorStoreSchema::Cli20261113);
    let snapshot = reader.snapshot().unwrap();
    assert_eq!(snapshot.metadata.root, Some(fixture.blob));
    let mut bytes = vec![0; snapshot.len(fixture.blob).unwrap() as usize];
    snapshot.read_at(fixture.blob, 0, &mut bytes).unwrap();
    assert_eq!(bytes, b"root fixture bytes");
    assert_eq!(before, fixture.unchanged());
    assert_eq!(
        std::fs::metadata(&snapshot.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(snapshot.directory.join("store.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(reader.poll().unwrap(), CursorReadProgress::Unchanged);
    drop(reader);
    assert_eq!(std::fs::read_dir(&fixture.staging).unwrap().count(), 0);
}

#[test]
fn cursor_store_copy_budget_and_changed_wal_restart_without_accepting_partial_snapshot() {
    let mut fixture = Fixture::new();
    fixture
        .connection
        .execute(
            "INSERT INTO blobs VALUES ('unreferenced',?1)",
            [vec![7u8; 6 * 1024 * 1024]],
        )
        .unwrap();
    let mut reader = fixture.reader();
    assert_eq!(reader.poll().unwrap(), CursorReadProgress::Pending);
    assert!(reader.snapshot().is_none());
    fixture.meta("native-session");
    assert_eq!(reader.poll(), Err(CursorReadError::Unstable));
    assert!(reader.snapshot().is_none());
    drain(&mut reader).unwrap();
    assert_eq!(
        reader.snapshot().unwrap().metadata.native_session,
        "native-session"
    );
}

#[test]
fn cursor_store_rejects_foreign_identity_future_schema_and_payload_links() {
    let mut fixture = Fixture::new();
    fixture.meta("foreign-session");
    assert_eq!(fixture.reader().poll(), Err(CursorReadError::Identity));
    fixture.meta("native-session");
    fixture
        .connection
        .execute_batch("PRAGMA user_version=2")
        .unwrap();
    assert_eq!(fixture.reader().poll(), Err(CursorReadError::Schema));
    fixture
        .connection
        .execute_batch("PRAGMA user_version=1")
        .unwrap();
    let hard = fixture.root.join("hardlink");
    std::fs::hard_link(fixture.session.join("store.db"), &hard).unwrap();
    assert_eq!(fixture.reader().poll(), Err(CursorReadError::Path));
    std::fs::remove_file(hard).unwrap();
    let wal = fixture.session.join("store.db-wal");
    let moved = fixture.root.join("wal");
    std::fs::rename(&wal, &moved).unwrap();
    std::os::unix::fs::symlink(moved, wal).unwrap();
    assert_eq!(fixture.reader().poll(), Err(CursorReadError::Path));
}

#[test]
fn cursor_store_authorized_root_is_pinned_and_missing_fresh_store_is_pending() {
    let fixture = Fixture::new();
    let parent = fixture.root.join("profile");
    std::fs::create_dir(&parent).unwrap();
    let alias = parent.join("native-session");
    std::os::unix::fs::symlink(&fixture.session, &alias).unwrap();
    let mut reader = CursorStoreReader::new(
        alias.clone(),
        "native-session".into(),
        "2026.11.13-eacade1",
        fixture.staging.clone(),
    )
    .unwrap();
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(fixture.root.join("absent"), &alias).unwrap();
    drain(&mut reader).unwrap();
    assert_eq!(reader.snapshot().unwrap().metadata.root, Some(fixture.blob));
    let mut fresh = CursorStoreReader::new(
        fixture.root.join("fresh"),
        "fresh".into(),
        "2026.11.13-eacade1",
        fixture.staging.clone(),
    )
    .unwrap();
    assert_eq!(fresh.poll().unwrap(), CursorReadProgress::Pending);
}
