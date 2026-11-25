//! Private ingestion journal. Canonical lines remain append-only; recovery completes an exact
//! pending prefix and never publishes, captures telemetry, schedules input or changes old lines.
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

const SCHEMA: i64 = 4;
mod input;
pub(crate) use input::PreparedInput;
const BATCH_LIMIT: usize = 8 * 1024 * 1024;
const LINE_LIMIT: usize = 4096;
const CHECKPOINT_LIMIT: usize = 65_536;
// SQL bounds materialization before Rust decodes even a damaged private journal.
const RECEIPT_COLUMNS: &str = "substr(occurrence,1,1025),substr(version,1,1025),substr(checkpoint,1,65537),start_byte,start_line,byte_len,line_count,substr(hash,1,33)";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct JournalBinding {
    pub app_session: String,
    pub provider: String,
    pub native_session: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JournalError {
    Identity,
    Schema,
    Storage,
    Invalid,
    Conflict,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CanonicalPosition {
    pub bytes: u64,
    pub lines: u64,
}

pub(crate) struct Capture {
    pub occurrence: String,
    pub version: String,
    pub checkpoint: serde_json::Value,
    pub position: CanonicalPosition,
    pub canonical_lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub occurrence: String,
    pub version: String,
    pub position: CanonicalPosition,
    pub byte_len: u64,
    pub line_count: u64,
    pub checkpoint: serde_json::Value,
    hash: Vec<u8>,
}
impl Receipt {
    pub(crate) fn end(&self) -> CanonicalPosition {
        CanonicalPosition {
            bytes: self.position.bytes + self.byte_len,
            lines: self.position.lines + self.line_count,
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AppendResult {
    Appended(Receipt),
    AlreadyCommitted(Receipt),
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Recovery {
    pub receipt: Receipt,
    pub appended_bytes: u64,
}

pub(crate) struct CaptureJournal {
    connection: Connection,
    binding: JournalBinding,
}
struct Intent {
    receipt: Receipt,
    bytes: Vec<u8>,
}

fn read_binding(connection: &Connection) -> Result<JournalBinding, JournalError> {
    let saved: String = connection
        .query_row(
            "SELECT substr(value,1,4097) FROM binding WHERE id=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| JournalError::Schema)?;
    if saved.len() > 4096 {
        return Err(JournalError::Schema);
    }
    serde_json::from_str(&saved).map_err(|_| JournalError::Schema)
}

impl CaptureJournal {
    /// Recover authenticated native identity before provider preparation after a board-write
    /// crash. This opens an existing app-owned database read-only, without migration or creation.
    pub(crate) fn peek_binding(
        path: &Path,
        app_session: &str,
        provider: &str,
    ) -> Result<Option<JournalBinding>, JournalError> {
        if !valid_id(app_session)
            || !matches!(provider, "claude" | "codex" | "antigravity" | "cursor")
        {
            return Err(JournalError::Identity);
        }
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(JournalError::Storage),
        };
        let private = |metadata: &std::fs::Metadata| {
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.permissions().mode() & 0o077 == 0
        };
        if !private(&metadata) {
            return Err(JournalError::Identity);
        }
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(suffix);
            match std::fs::symlink_metadata(sidecar) {
                Ok(metadata) if private(&metadata) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(JournalError::Identity),
            }
        }
        let parent = path
            .parent()
            .ok_or(JournalError::Identity)?
            .canonicalize()
            .map_err(|_| JournalError::Storage)?;
        let path = parent.join(path.file_name().ok_or(JournalError::Identity)?);
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| JournalError::Storage)?;
        connection
            .execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(|_| JournalError::Storage)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| JournalError::Schema)?;
        if !matches!(version, 1 | 2 | 3 | SCHEMA) {
            return Err(JournalError::Schema);
        }
        let saved = read_binding(&connection)?;
        if saved.app_session != app_session
            || saved.provider != provider
            || saved
                .native_session
                .as_deref()
                .is_some_and(|id| !valid_id(id))
            || (version == 1 && saved.native_session.is_none())
        {
            return Err(JournalError::Identity);
        }
        Ok(Some(saved))
    }

    /// One binding and one pending append per journal. The caller holds the same writer gate
    /// for app echoes, native output and notices, and runs recovery before accepting new writes.
    pub(crate) fn open(path: &Path, binding: &JournalBinding) -> Result<Self, JournalError> {
        if !valid_id(&binding.app_session)
            || binding
                .native_session
                .as_deref()
                .is_some_and(|id| !valid_id(id))
            || !matches!(
                binding.provider.as_str(),
                "claude" | "codex" | "antigravity" | "cursor"
            )
        {
            return Err(JournalError::Identity);
        }
        let parent = path.parent().ok_or(JournalError::Invalid)?;
        crate::paths::ensure_private_dir(parent).map_err(|_| JournalError::Storage)?;
        let parent = std::fs::canonicalize(parent).map_err(|_| JournalError::Storage)?;
        let path = parent.join(path.file_name().ok_or(JournalError::Invalid)?);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file.sync_all().map_err(|_| JournalError::Storage)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(JournalError::Storage),
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| JournalError::Storage)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(JournalError::Identity);
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| JournalError::Storage)?;
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| JournalError::Storage)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|_| JournalError::Storage)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| JournalError::Schema)?;
        if !matches!(version, 0 | 1 | 2 | 3 | SCHEMA) {
            return Err(JournalError::Schema);
        }
        let binding_json = serde_json::to_string(binding).map_err(|_| JournalError::Invalid)?;
        if version == 0 {
            let count: i64 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| JournalError::Schema)?;
            if count != 0 {
                return Err(JournalError::Schema);
            }
            let transaction = connection
                .transaction()
                .map_err(|_| JournalError::Storage)?;
            transaction.execute_batch("CREATE TABLE binding (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL); CREATE TABLE captures (occurrence TEXT NOT NULL, version TEXT NOT NULL, checkpoint TEXT NOT NULL, start_byte INTEGER NOT NULL, start_line INTEGER NOT NULL, byte_len INTEGER NOT NULL, line_count INTEGER NOT NULL, hash BLOB NOT NULL, pending_bytes BLOB, committed INTEGER NOT NULL CHECK(committed IN (0,1)), PRIMARY KEY(occurrence,version)); CREATE UNIQUE INDEX one_pending_capture ON captures(committed) WHERE committed=0; PRAGMA user_version=2;").map_err(|_|JournalError::Storage)?;
            transaction
                .execute("INSERT INTO binding VALUES (1,?1)", [binding_json])
                .map_err(|_| JournalError::Storage)?;
            transaction.commit().map_err(|_| JournalError::Storage)?;
        } else {
            let saved = read_binding(&connection)?;
            if (version == 1 && saved.native_session.is_none()) || saved != *binding {
                return Err(JournalError::Identity);
            }
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; PRAGMA trusted_schema=OFF;").map_err(|_|JournalError::Storage)?;
        if version < 3 {
            // The input intent precedes every native write. Existing receipts and public bytes
            // retain their exact identity/ranges across both older journal versions.
            connection
                .execute_batch("BEGIN IMMEDIATE; CREATE TABLE prepared_inputs (id TEXT PRIMARY KEY, value TEXT NOT NULL); PRAGMA user_version=3; COMMIT;")
                .map_err(|_| JournalError::Storage)?;
        }
        if version < 4 {
            connection.execute_batch("BEGIN IMMEDIATE; CREATE INDEX completed_source_user ON captures(json_extract(checkpoint,'$.sourceUser')) WHERE committed=1 AND json_extract(checkpoint,'$.kind')='completion' AND line_count>0; CREATE INDEX source_user_order ON captures(start_line) WHERE committed=1 AND json_extract(checkpoint,'$.kind') IN ('user','submission') AND line_count>0; PRAGMA user_version=4; COMMIT;").map_err(|_| JournalError::Storage)?;
        }
        File::open(&parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| JournalError::Storage)?;
        Ok(Self {
            connection,
            binding: binding.clone(),
        })
    }

    /// Some native CLIs allocate their identity only after the first accepted prompt. Keep its
    /// app echo durable under app/provider identity, then bind the authenticated native identity
    /// exactly once. No placeholder id or later process may rebind that canonical history.
    pub(crate) fn bind_native(&mut self, native_session: &str) -> Result<(), JournalError> {
        if !valid_id(native_session) {
            return Err(JournalError::Identity);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| JournalError::Storage)?;
        let mut saved = read_binding(&transaction)?;
        if saved.app_session != self.binding.app_session || saved.provider != self.binding.provider
        {
            return Err(JournalError::Identity);
        }
        if let Some(identity) = &saved.native_session {
            if identity != native_session {
                return Err(JournalError::Identity);
            }
        } else {
            saved.native_session = Some(native_session.into());
            transaction
                .execute(
                    "UPDATE binding SET value=?1 WHERE id=1",
                    [serde_json::to_string(&saved).map_err(|_| JournalError::Invalid)?],
                )
                .map_err(|_| JournalError::Storage)?;
        }
        transaction.commit().map_err(|_| JournalError::Storage)?;
        self.binding = saved;
        Ok(())
    }

    /// Look up occurrence+version before generating fallback timestamps or public message IDs.
    /// A committed occurrence already owns its original canonical range; it is not replayed.
    pub(crate) fn lookup(
        &self,
        occurrence: &str,
        version: &str,
    ) -> Result<Option<Receipt>, JournalError> {
        if !valid_key(occurrence) || !valid_key(version) {
            return Err(JournalError::Invalid);
        }
        self.connection.query_row(&format!("SELECT {RECEIPT_COLUMNS} FROM captures WHERE occurrence=?1 AND version=?2 AND committed=1"),[occurrence,version],receipt).optional().map_err(|_|JournalError::Storage)
    }

    /// The last committed source version provides the private checkpoint for incremental
    /// text/tool projection. Re-observing an older version never changes this ordering.
    pub(crate) fn latest(&self, occurrence: &str) -> Result<Option<Receipt>, JournalError> {
        if !valid_key(occurrence) {
            return Err(JournalError::Invalid);
        }
        self.connection.query_row(&format!("SELECT {RECEIPT_COLUMNS} FROM captures WHERE occurrence=?1 AND committed=1 ORDER BY rowid DESC LIMIT 1"),[occurrence],receipt).optional().map_err(|_|JournalError::Storage)
    }

    /// A sealed source row is not a turn boundary. Only a committed canonical completion for
    /// this causal native user can suppress current-turn effects from later row revisions.
    pub(crate) fn completed_source_user(&self, user: &str) -> Result<bool, JournalError> {
        if !valid_key(user) {
            return Err(JournalError::Invalid);
        }
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM captures WHERE committed=1 AND json_extract(checkpoint,'$.kind')='completion' AND line_count>0 AND json_extract(checkpoint,'$.sourceUser')=?1)", [user], |row| row.get(0)).map_err(|_| JournalError::Storage)
    }

    /// Native aliases and accepted app inputs retain their public user range. Replay cannot
    /// move an old input forward; a prepared intent without public bytes is not a successor.
    pub(crate) fn source_input_after(
        &self,
        position: CanonicalPosition,
    ) -> Result<bool, JournalError> {
        let line = i64::try_from(position.lines).map_err(|_| JournalError::Invalid)?;
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM captures WHERE committed=1 AND json_extract(checkpoint,'$.kind') IN ('user','submission') AND line_count>0 AND start_line>?1)", [line], |row| row.get(0)).map_err(|_| JournalError::Storage)
    }

    /// Bounded app submissions whose canonical range has not acquired a native input alias.
    /// Checkpoints contain causal watermarks and hashes, never a global prompt-text index.
    pub(crate) fn pending_submissions(&self) -> Result<Vec<Receipt>, JournalError> {
        let sql=format!("SELECT {RECEIPT_COLUMNS} FROM captures AS pending WHERE committed=1 AND json_extract(checkpoint,'$.kind')='submission' AND NOT EXISTS (SELECT 1 FROM captures AS native WHERE native.committed=1 AND native.start_byte=pending.start_byte AND native.byte_len=pending.byte_len AND json_extract(native.checkpoint,'$.kind')='user') ORDER BY start_byte,rowid LIMIT 65");
        let mut query = self
            .connection
            .prepare(&sql)
            .map_err(|_| JournalError::Storage)?;
        let records = query
            .query_map([], receipt)
            .map_err(|_| JournalError::Storage)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| JournalError::Storage)?;
        if records.len() > 64 {
            return Err(JournalError::Invalid);
        }
        Ok(records)
    }

    pub(crate) fn native_session(&self) -> Option<&str> {
        self.binding.native_session.as_deref()
    }

    /// App status events can precede the first prompt. Any other receipt is conversation history.
    pub(crate) fn has_conversation_activity(&self) -> Result<bool, JournalError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM captures WHERE json_extract(checkpoint,'$.kind') IS NOT 'app') OR EXISTS(SELECT 1 FROM prepared_inputs)",
                [],
                |row| row.get(0),
            )
            .map_err(|_| JournalError::Storage)
    }

    pub(crate) fn append(
        &mut self,
        log: &mut File,
        capture: Capture,
    ) -> Result<AppendResult, JournalError> {
        let intent = validate(capture)?;
        if let Some(saved) = self.lookup(&intent.receipt.occurrence, &intent.receipt.version)? {
            if saved.hash != intent.receipt.hash
                || saved.byte_len != intent.receipt.byte_len
                || saved.line_count != intent.receipt.line_count
            {
                return Err(JournalError::Conflict);
            }
            return Ok(AppendResult::AlreadyCommitted(saved));
        }
        self.prepare(log, &intent)?;
        self.finish(log, &intent)?;
        Ok(AppendResult::Appended(intent.receipt))
    }

    /// Recovery is storage-only. The host seeds/reconciles Lines separately, with no Pump feed.
    pub(crate) fn recover(&mut self, log: &mut File) -> Result<Option<Recovery>, JournalError> {
        let intent=self.connection.query_row(&format!("SELECT {RECEIPT_COLUMNS},substr(pending_bytes,1,8388609) FROM captures WHERE committed=0"),[],|row|Ok(Intent{receipt:receipt(row)?,bytes:row.get(8)?})).optional().map_err(|_|JournalError::Storage)?;
        let Some(intent) = intent else {
            return Ok(None);
        };
        if intent.bytes.len() > BATCH_LIMIT
            || intent.bytes.len() as u64 != intent.receipt.byte_len
            || hash(&intent.bytes) != intent.receipt.hash
        {
            return Err(JournalError::Conflict);
        }
        let appended_bytes = self.finish(log, &intent)?;
        Ok(Some(Recovery {
            receipt: intent.receipt,
            appended_bytes,
        }))
    }

    /// Bind a proven native input to an already committed app submission without appending its
    /// echo again. Causal/native identity proof belongs to the caller, never a global text match.
    pub(crate) fn bind_alias(
        &mut self,
        occurrence: &str,
        version: &str,
        checkpoint: serde_json::Value,
        target: &Receipt,
    ) -> Result<Receipt, JournalError> {
        if !valid_key(occurrence) || !valid_key(version) || !valid_checkpoint(&checkpoint) {
            return Err(JournalError::Invalid);
        }
        if self.lookup(&target.occurrence, &target.version)?.as_ref() != Some(target) {
            return Err(JournalError::Conflict);
        }
        let mut alias = target.clone();
        alias.occurrence = occurrence.into();
        alias.version = version.into();
        alias.checkpoint = checkpoint;
        if let Some(saved) = self.lookup(occurrence, version)? {
            return if saved == alias {
                Ok(saved)
            } else {
                Err(JournalError::Conflict)
            };
        }
        insert(&self.connection, &alias, None, true)?;
        Ok(alias)
    }

    fn prepare(&mut self, log: &mut File, intent: &Intent) -> Result<(), JournalError> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT count(*) FROM captures WHERE committed=0",
                [],
                |row| row.get(0),
            )
            .map_err(|_| JournalError::Storage)?;
        if count != 0 {
            return Err(JournalError::Pending);
        }
        let size = log_size(log)?;
        if size != intent.receipt.position.bytes {
            return Err(JournalError::Conflict);
        }
        if size > 0 {
            log.seek(SeekFrom::Start(size - 1))
                .map_err(|_| JournalError::Storage)?;
            let mut last = [0];
            log.read_exact(&mut last)
                .map_err(|_| JournalError::Storage)?;
            if last[0] != b'\n' {
                return Err(JournalError::Conflict);
            }
        }
        insert(
            &self.connection,
            &intent.receipt,
            Some(&intent.bytes),
            false,
        )
    }
    fn finish(&mut self, log: &mut File, intent: &Intent) -> Result<u64, JournalError> {
        let size = log_size(log)?;
        let start = intent.receipt.position.bytes;
        if size < start {
            return Err(JournalError::Conflict);
        }
        let present = (size - start).min(intent.bytes.len() as u64) as usize;
        log.seek(SeekFrom::Start(start))
            .map_err(|_| JournalError::Storage)?;
        let mut offset = 0;
        let mut buffer = [0; 65_536];
        while offset < present {
            let count = (present - offset).min(buffer.len());
            log.read_exact(&mut buffer[..count])
                .map_err(|_| JournalError::Storage)?;
            if buffer[..count] != intent.bytes[offset..offset + count] {
                return Err(JournalError::Conflict);
            }
            offset += count;
        }
        let missing = &intent.bytes[present..];
        if !missing.is_empty() {
            if size != start + present as u64 {
                return Err(JournalError::Conflict);
            }
            log.seek(SeekFrom::End(0))
                .map_err(|_| JournalError::Storage)?;
            log.write_all(missing).map_err(|_| JournalError::Storage)?;
        }
        log.sync_data().map_err(|_| JournalError::Storage)?;
        let updated=self.connection.execute("UPDATE captures SET committed=1,pending_bytes=NULL WHERE occurrence=?1 AND version=?2 AND committed=0",[&intent.receipt.occurrence,&intent.receipt.version]).map_err(|_|JournalError::Storage)?;
        if updated != 1 {
            return Err(JournalError::Conflict);
        }
        Ok(missing.len() as u64)
    }
}

fn receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    let checkpoint: String = row.get(2)?;
    let invalid = || rusqlite::Error::InvalidQuery;
    if checkpoint.len() > CHECKPOINT_LIMIT {
        return Err(invalid());
    }
    let checkpoint = serde_json::from_str(&checkpoint).map_err(|_| invalid())?;
    if !valid_checkpoint(&checkpoint) {
        return Err(invalid());
    }
    let number = |index| u64::try_from(row.get::<_, i64>(index)?).map_err(|_| invalid());
    let receipt = Receipt {
        occurrence: row.get(0)?,
        version: row.get(1)?,
        checkpoint,
        position: CanonicalPosition {
            bytes: number(3)?,
            lines: number(4)?,
        },
        byte_len: number(5)?,
        line_count: number(6)?,
        hash: row.get(7)?,
    };
    if !valid_key(&receipt.occurrence)
        || !valid_key(&receipt.version)
        || receipt.byte_len > BATCH_LIMIT as u64
        || receipt.line_count > LINE_LIMIT as u64
        || receipt.hash.len() != 32
        || receipt
            .position
            .bytes
            .checked_add(receipt.byte_len)
            .is_none()
        || receipt
            .position
            .lines
            .checked_add(receipt.line_count)
            .is_none()
    {
        return Err(invalid());
    }
    Ok(receipt)
}
fn insert(
    connection: &Connection,
    receipt: &Receipt,
    bytes: Option<&[u8]>,
    committed: bool,
) -> Result<(), JournalError> {
    let number = |value| i64::try_from(value).map_err(|_| JournalError::Invalid);
    connection
        .execute(
            "INSERT INTO captures VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            rusqlite::params![
                receipt.occurrence,
                receipt.version,
                receipt.checkpoint.to_string(),
                number(receipt.position.bytes)?,
                number(receipt.position.lines)?,
                number(receipt.byte_len)?,
                number(receipt.line_count)?,
                receipt.hash,
                bytes,
                committed
            ],
        )
        .map_err(|_| JournalError::Storage)?;
    Ok(())
}
fn validate(capture: Capture) -> Result<Intent, JournalError> {
    if !valid_key(&capture.occurrence)
        || !valid_key(&capture.version)
        || !valid_checkpoint(&capture.checkpoint)
        || capture.canonical_lines.len() > LINE_LIMIT
    {
        return Err(JournalError::Invalid);
    }
    let mut bytes = Vec::new();
    for line in &capture.canonical_lines {
        if line.contains(['\r', '\n']) || line.len() >= BATCH_LIMIT.saturating_sub(bytes.len()) {
            return Err(JournalError::Invalid);
        }
        let frame: serde_json::Value =
            serde_json::from_str(line).map_err(|_| JournalError::Invalid)?;
        if frame["v"] != 1
            || !matches!(
                frame["type"].as_str(),
                Some(
                    "user.message"
                        | "assistant.block"
                        | "tool.completed"
                        | "request.opened"
                        | "request.closed"
                        | "turn.completed"
                        | "context.compacted"
                        | "background.changed"
                        | "system.notice"
                        | "system.summary"
                        | "context.reported"
                )
            )
            || frame.get("telemetry").is_some()
            || frame.get("providerDurationMs").is_some()
        {
            return Err(JournalError::Invalid);
        }
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    let receipt = Receipt {
        occurrence: capture.occurrence,
        version: capture.version,
        position: capture.position,
        byte_len: bytes.len() as u64,
        line_count: capture.canonical_lines.len() as u64,
        checkpoint: capture.checkpoint,
        hash: hash(&bytes),
    };
    if receipt
        .position
        .bytes
        .checked_add(receipt.byte_len)
        .is_none()
        || receipt
            .position
            .lines
            .checked_add(receipt.line_count)
            .is_none()
    {
        return Err(JournalError::Invalid);
    }
    Ok(Intent { receipt, bytes })
}
fn log_size(log: &File) -> Result<u64, JournalError> {
    let metadata = log.metadata().map_err(|_| JournalError::Storage)?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(JournalError::Identity);
    }
    Ok(metadata.len())
}
fn hash(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn valid_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
fn valid_checkpoint(value: &serde_json::Value) -> bool {
    value.is_object() && value.to_string().len() <= CHECKPOINT_LIMIT
}

#[cfg(test)]
mod tests;
