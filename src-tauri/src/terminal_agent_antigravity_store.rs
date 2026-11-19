//! Read one hook-bound AGY conversation from a private SQLite/WAL snapshot.
use crate::terminal_agent_observation::{open_authorized, AuthorizedRoot, ObservationError};
use crate::terminal_agent_sqlite_snapshot::{
    SnapshotError, SnapshotProgress, SnapshotReader, SqliteSnapshot, Stamp,
};
use prometeu_protocols::antigravity_store::{self as protocol, Content, Schema, Source, Step};
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell, collections::BTreeMap, io::Read, os::unix::fs::MetadataExt, path::PathBuf,
};

const CHUNK: usize = 65_536;
const OUTPUT_LIMIT: u64 = 1024 * 1024;
const STEP_LIMIT: u64 = 128 * 1024 * 1024;

pub(crate) struct AntigravityStoreReader {
    reader: SnapshotReader,
    snapshot: Option<SqliteSnapshot>,
    schema: Schema,
    native: String,
    trajectory: Option<String>,
    brain: AuthorizedRoot,
    brain_path: PathBuf,
    revision: String,
    outputs: BTreeMap<PathBuf, Stamp>,
}
impl AntigravityStoreReader {
    pub(crate) fn new(
        root: AuthorizedRoot,
        conversations: PathBuf,
        brain: AuthorizedRoot,
        brain_path: PathBuf,
        native: String,
        version: &str,
        staging: PathBuf,
    ) -> Result<Self, ObservationError> {
        if native.is_empty()
            || native.len() > 256
            || native.contains(['/', '\\'])
            || native.chars().any(char::is_control)
        {
            return Err(ObservationError::Identity);
        }
        let schema = Schema::for_version(version).ok_or(ObservationError::Transcript)?;
        Ok(Self {
            reader: SnapshotReader::new(root, conversations.join(format!("{native}.db")), staging),
            snapshot: None,
            schema,
            native,
            trajectory: None,
            brain,
            brain_path,
            revision: String::new(),
            outputs: BTreeMap::new(),
        })
    }
    pub(crate) fn revision(&self) -> &str {
        &self.revision
    }
    pub(crate) fn trajectory(&self) -> Option<&str> {
        self.trajectory.as_deref()
    }
    pub(crate) fn poll(&mut self) -> Result<SnapshotProgress, ObservationError> {
        let progress = self.reader.poll().map_err(copy_error)?;
        match progress {
            SnapshotProgress::Ready => {
                let snapshot = self.reader.take_snapshot().ok_or(ObservationError::Read)?;
                let connection = snapshot.connection().map_err(copy_error)?;
                let version: i64 = connection
                    .query_row("PRAGMA user_version", [], |r| r.get(0))
                    .map_err(|_| ObservationError::Transcript)?;
                if version != 1 {
                    return Err(ObservationError::Transcript);
                }
                for table in ["trajectory_meta", "steps"] {
                    let kind: Option<String> = connection
                        .query_row(
                            "SELECT type FROM sqlite_schema WHERE name=?1",
                            [table],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(|_| ObservationError::Transcript)?;
                    if kind.as_deref() != Some("table") {
                        return Err(ObservationError::Transcript);
                    }
                }
                for (table, expected) in [
                    (
                        "trajectory_meta",
                        vec![
                            ("trajectory_id", "TEXT", 1),
                            ("cascade_id", "TEXT", 0),
                            ("trajectory_type", "INTEGER", 0),
                            ("source", "INTEGER", 0),
                        ],
                    ),
                    (
                        "steps",
                        vec![
                            ("idx", "INTEGER", 1),
                            ("step_type", "INTEGER", 0),
                            ("status", "INTEGER", 0),
                            ("has_subtrajectory", "numeric", 0),
                            ("metadata", "BLOB", 0),
                            ("error_details", "BLOB", 0),
                            ("permissions", "BLOB", 0),
                            ("task_details", "BLOB", 0),
                            ("render_info", "BLOB", 0),
                            ("step_payload", "BLOB", 0),
                            ("step_format", "INTEGER", 0),
                        ],
                    ),
                ] {
                    let mut query = connection
                        .prepare(&format!("PRAGMA table_xinfo({table})"))
                        .map_err(|_| ObservationError::Transcript)?;
                    let columns = query
                        .query_map([], |row| {
                            Ok((
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(5)?,
                                row.get::<_, i64>(6)?,
                            ))
                        })
                        .map_err(|_| ObservationError::Transcript)?
                        .take(expected.len() + 1)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| ObservationError::Transcript)?;
                    if columns.len() != expected.len()
                        || !columns.iter().zip(expected).all(
                            |(
                                (name, kind, primary, hidden),
                                (expected_name, expected_type, expected_primary),
                            )| {
                                name == expected_name
                                    && kind.eq_ignore_ascii_case(expected_type)
                                    && *primary == expected_primary
                                    && *hidden == 0
                            },
                        )
                    {
                        return Err(ObservationError::Transcript);
                    }
                }
                let mut statement=connection.prepare("SELECT substr(trajectory_id,1,257),substr(cascade_id,1,257) FROM trajectory_meta LIMIT 2").map_err(|_|ObservationError::Transcript)?;
                let identities = statement
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|_| ObservationError::Transcript)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| ObservationError::Transcript)?;
                if identities.len() != 1
                    || identities[0].1 != self.native
                    || !valid_id(&identities[0].0)
                    || self
                        .trajectory
                        .as_ref()
                        .is_some_and(|id| id != &identities[0].0)
                {
                    return Err(ObservationError::Identity);
                }
                self.trajectory = Some(identities[0].0.clone());
                let count: i64 = connection
                    .query_row(
                        "SELECT count(*) FROM (SELECT idx FROM steps LIMIT 65537)",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|_| ObservationError::Transcript)?;
                if count > 65536 {
                    return Err(ObservationError::Limit);
                }
                drop(statement);
                self.outputs.clear();
                self.revision = uuid::Uuid::new_v4().to_string();
                self.snapshot = Some(snapshot);
            }
            SnapshotProgress::Pending => self.snapshot = None,
            SnapshotProgress::Unchanged => {
                for (path, expected) in &self.outputs {
                    let file = open_authorized(std::slice::from_mut(&mut self.brain), path)?
                        .ok_or(ObservationError::Unstable)?;
                    let metadata = file.metadata().map_err(|_| ObservationError::Read)?;
                    if !metadata.is_file()
                        || metadata.nlink() != 1
                        || metadata.uid() != unsafe { libc::geteuid() }
                    {
                        return Err(ObservationError::Path);
                    }
                    if &Stamp::from(&metadata) != expected {
                        self.revision = uuid::Uuid::new_v4().to_string();
                        self.outputs.clear();
                        return Ok(SnapshotProgress::Ready);
                    }
                }
            }
        }
        Ok(progress)
    }
    /// Uses native idx rather than ordinal; compaction cannot renumber an existing occurrence.
    pub(crate) fn next(&mut self, after: Option<u64>) -> Result<Option<Step>, ObservationError> {
        let snapshot = self.snapshot.as_ref().ok_or(ObservationError::Read)?;
        let connection = snapshot.connection().map_err(copy_error)?;
        let after = after
            .map(i64::try_from)
            .transpose()
            .map_err(|_| ObservationError::Limit)?
            .unwrap_or(-1);
        let row:Option<(i64,i64,i64,i64,i64,i64)>=connection.query_row("SELECT idx,step_type,status,step_format,has_subtrajectory,length(step_payload) FROM steps WHERE idx>?1 ORDER BY idx LIMIT 1",[after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|ObservationError::Transcript)?;
        let Some((index, kind, status, format, subtrajectory, length)) = row else {
            return Ok(None);
        };
        if index < 0 || kind < 0 || status < 0 || format != 0 || subtrajectory != 0 || length < 0 {
            return Err(ObservationError::Transcript);
        }
        if length as u64 > STEP_LIMIT {
            return Err(ObservationError::Limit);
        }
        let source = StepSource {
            connection,
            index,
            length: length as u64,
            cache: RefCell::new(None),
        };
        let mut step = protocol::step(
            &source,
            index as u64,
            kind as u64,
            status as u64,
            self.schema,
        )
        .map_err(decode_error)?;
        if let Content::Tool {
            result,
            full_output_uri: Some(uri),
            ..
        } = &mut step.content
        {
            let url = reqwest::Url::parse(uri).map_err(|_| ObservationError::Path)?;
            if url.scheme() != "file"
                || url.host_str().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(ObservationError::Path);
            }
            let path = url.to_file_path().map_err(|_| ObservationError::Path)?;
            let expected = self
                .brain_path
                .join(&self.native)
                .join(".system_generated/steps")
                .join(index.to_string())
                .join("output.txt");
            if path != expected {
                return Err(ObservationError::Path);
            }
            let mut file = open_authorized(std::slice::from_mut(&mut self.brain), &path)?
                .ok_or(ObservationError::Unstable)?;
            let before = file.metadata().map_err(|_| ObservationError::Read)?;
            if !before.is_file()
                || before.nlink() != 1
                || before.uid() != unsafe { libc::geteuid() }
            {
                return Err(ObservationError::Path);
            }
            if before.len() > OUTPUT_LIMIT {
                return Err(ObservationError::Limit);
            }
            let mut bytes = Vec::with_capacity(before.len() as usize);
            file.by_ref()
                .take(OUTPUT_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ObservationError::Read)?;
            if bytes.len() as u64 != before.len()
                || Stamp::from(&before)
                    != Stamp::from(&file.metadata().map_err(|_| ObservationError::Read)?)
            {
                return Err(ObservationError::Unstable);
            }
            self.outputs.insert(path, Stamp::from(&before));
            let text = String::from_utf8(bytes).map_err(|_| ObservationError::Transcript)?;
            step.version = format!(
                "{:x}",
                Sha256::digest([step.version.as_bytes(), text.as_bytes()].concat())
            );
            *result = Some(serde_json::Value::String(text));
            step.complete = true;
        }
        Ok(Some(step))
    }
}
struct StepSource<'a> {
    connection: &'a rusqlite::Connection,
    index: i64,
    length: u64,
    cache: RefCell<Option<(u64, Vec<u8>)>>,
}
impl Source for StepSource<'_> {
    fn len(&self) -> u64 {
        self.length
    }
    fn read_at(&self, mut offset: u64, mut destination: &mut [u8]) -> Result<(), protocol::Error> {
        while !destination.is_empty() {
            let aligned = offset / CHUNK as u64 * CHUNK as u64;
            let mut cache = self.cache.borrow_mut();
            if cache.as_ref().is_none_or(|(at, _)| *at != aligned) {
                let start = i64::try_from(aligned)
                    .ok()
                    .and_then(|n| n.checked_add(1))
                    .ok_or(protocol::Error::Limit)?;
                let bytes: Vec<u8> = self
                    .connection
                    .query_row(
                        "SELECT substr(step_payload,?2,?3) FROM steps WHERE idx=?1",
                        params![self.index, start, CHUNK as i64],
                        |r| r.get(0),
                    )
                    .map_err(|_| protocol::Error::Read)?;
                *cache = Some((aligned, bytes));
            }
            let bytes = &cache.as_ref().ok_or(protocol::Error::Read)?.1;
            let bytes = bytes
                .get((offset - aligned) as usize..)
                .filter(|b| !b.is_empty())
                .ok_or(protocol::Error::Read)?;
            let count = bytes.len().min(destination.len());
            destination[..count].copy_from_slice(&bytes[..count]);
            destination = &mut destination[count..];
            offset += count as u64;
        }
        Ok(())
    }
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}
fn copy_error(error: SnapshotError) -> ObservationError {
    match error {
        SnapshotError::Path => ObservationError::Path,
        SnapshotError::Read => ObservationError::Read,
        SnapshotError::Unstable => ObservationError::Unstable,
        SnapshotError::Schema => ObservationError::Transcript,
        SnapshotError::Identity => ObservationError::Identity,
    }
}
fn decode_error(error: protocol::Error) -> ObservationError {
    match error {
        protocol::Error::Read => ObservationError::Read,
        protocol::Error::Identity => ObservationError::Identity,
        protocol::Error::Limit => ObservationError::Limit,
        protocol::Error::Malformed | protocol::Error::Unsupported => ObservationError::Transcript,
    }
}
#[cfg(test)]
mod tests;
