//! Cursor schema and identity checks over a private, consistent SQLite snapshot.
use crate::terminal_agent_observation::AuthorizedRoot;
#[cfg(test)]
use crate::terminal_agent_sqlite_snapshot::Stamp;
pub(crate) use crate::terminal_agent_sqlite_snapshot::{
    SnapshotError as CursorReadError, SnapshotProgress as CursorReadProgress,
};
use crate::terminal_agent_sqlite_snapshot::{SnapshotReader, SqliteSnapshot};
use prometeu_protocols::cursor_store::{
    self, BlobId, BlobSource, CursorMetadata, CursorStoreSchema, StoreError,
};
#[cfg(test)]
use rusqlite::Connection;
use rusqlite::OptionalExtension;
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::{cell::RefCell, path::PathBuf};
const CHUNK: usize = 65_536;
const METADATA_LIMIT: usize = 131_072;

pub(crate) struct CursorStoreReader {
    reader: SnapshotReader,
    session: String,
    schema: CursorStoreSchema,
    snapshot: Option<CursorStoreSnapshot>,
}
impl CursorStoreReader {
    /// The caller derives this exact session directory from its captured Cursor config directory,
    /// original cwd and authenticated native session. This method never searches for a store.
    #[cfg(test)]
    pub(crate) fn new(
        session_root: PathBuf,
        session: String,
        version: &str,
        staging_root: PathBuf,
    ) -> Result<Self, CursorReadError> {
        let schema = CursorStoreSchema::for_version(version).ok_or(CursorReadError::Schema)?;
        if session.is_empty()
            || session.len() > 256
            || session.contains(['/', '\\'])
            || session.chars().any(char::is_control)
            || session_root.file_name().and_then(|name| name.to_str()) != Some(session.as_str())
        {
            return Err(CursorReadError::Identity);
        }
        let root = AuthorizedRoot::new(session_root.clone()).map_err(|_| CursorReadError::Path)?;
        Self::from_authorized_root(root, session_root, session, schema, staging_root)
    }

    pub(crate) fn from_authorized_root(
        root: AuthorizedRoot,
        session_root: PathBuf,
        session: String,
        schema: CursorStoreSchema,
        staging_root: PathBuf,
    ) -> Result<Self, CursorReadError> {
        Ok(Self {
            reader: SnapshotReader::new(root, session_root.join("store.db"), staging_root),
            session,
            schema,
            snapshot: None,
        })
    }
    pub(crate) fn schema(&self) -> CursorStoreSchema {
        self.schema
    }
    pub(crate) fn snapshot(&self) -> Option<&CursorStoreSnapshot> {
        self.snapshot.as_ref()
    }
    pub(crate) fn poll(&mut self) -> Result<CursorReadProgress, CursorReadError> {
        let progress = self.reader.poll()?;
        match progress {
            CursorReadProgress::Ready => {
                let store = self.reader.take_snapshot().ok_or(CursorReadError::Read)?;
                self.snapshot = Some(CursorStoreSnapshot::new(store, &self.session)?);
            }
            CursorReadProgress::Pending => self.snapshot = None,
            CursorReadProgress::Unchanged => {}
        }
        Ok(progress)
    }
}

pub(crate) struct CursorStoreSnapshot {
    store: SqliteSnapshot,
    #[cfg(test)]
    directory: PathBuf,
    pub(crate) metadata: CursorMetadata,
    cache: RefCell<Option<(BlobId, u64, Vec<u8>)>>,
}
impl CursorStoreSnapshot {
    fn new(store: SqliteSnapshot, session: &str) -> Result<Self, CursorReadError> {
        let connection = store.connection()?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| CursorReadError::Schema)?;
        if version != 1 {
            return Err(CursorReadError::Schema);
        }
        for table in ["meta", "blobs"] {
            let kind: Option<String> = connection
                .query_row(
                    "SELECT type FROM sqlite_schema WHERE name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| CursorReadError::Schema)?;
            if kind.as_deref() != Some("table") {
                return Err(CursorReadError::Schema);
            }
            let query = match table {
                "meta" => "PRAGMA table_xinfo(meta)",
                _ => "PRAGMA table_xinfo(blobs)",
            };
            let mut statement = connection
                .prepare(query)
                .map_err(|_| CursorReadError::Schema)?;
            let columns = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                })
                .map_err(|_| CursorReadError::Schema)?
                .take(3)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| CursorReadError::Schema)?;
            let expected = match table {
                "meta" => [("key", "TEXT", 1, 0), ("value", "TEXT", 0, 0)],
                _ => [("id", "TEXT", 1, 0), ("data", "BLOB", 0, 0)],
            };
            if columns.len() != 2
                || !columns.iter().zip(expected).all(
                    |(
                        (name, kind, primary, hidden),
                        (wanted_name, wanted_kind, wanted_primary, wanted_hidden),
                    )| {
                        name == wanted_name
                            && kind.eq_ignore_ascii_case(wanted_kind)
                            && *primary == wanted_primary
                            && *hidden == wanted_hidden
                    },
                )
            {
                return Err(CursorReadError::Schema);
            }
        }
        let (length, value): (i64, String) = connection
            .query_row(
                "SELECT length(value), substr(value, 1, ?1) FROM meta WHERE key = '0'",
                [(METADATA_LIMIT + 1) as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| CursorReadError::Schema)?;
        if length < 0 || length > METADATA_LIMIT as i64 {
            return Err(CursorReadError::Schema);
        }
        let metadata =
            cursor_store::metadata(&value, session).map_err(|_| CursorReadError::Identity)?;
        Ok(Self {
            #[cfg(test)]
            directory: store.directory.clone(),
            store,
            metadata,
            cache: RefCell::new(None),
        })
    }
}
impl BlobSource for CursorStoreSnapshot {
    fn len(&self, id: BlobId) -> Result<u64, StoreError> {
        let length: i64 = self
            .store
            .connection()
            .map_err(|_| StoreError::Read)?
            .query_row(
                "SELECT length(data) FROM blobs WHERE id = ?1",
                [id.hex()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| StoreError::Read)?
            .ok_or(StoreError::Missing)?;
        u64::try_from(length).map_err(|_| StoreError::Read)
    }
    fn read_at(
        &self,
        id: BlobId,
        mut offset: u64,
        mut destination: &mut [u8],
    ) -> Result<(), StoreError> {
        while !destination.is_empty() {
            let aligned = offset / CHUNK as u64 * CHUNK as u64;
            let mut cache = self.cache.borrow_mut();
            if !cache
                .as_ref()
                .is_some_and(|(key, start, _)| *key == id && *start == aligned)
            {
                let start = i64::try_from(aligned)
                    .ok()
                    .and_then(|offset| offset.checked_add(1))
                    .ok_or(StoreError::Limit)?;
                let bytes: Vec<u8> = self
                    .store
                    .connection()
                    .map_err(|_| StoreError::Read)?
                    .query_row(
                        "SELECT substr(data, ?2, ?3) FROM blobs WHERE id = ?1",
                        rusqlite::params![id.hex(), start, CHUNK as i64],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|_| StoreError::Read)?
                    .ok_or(StoreError::Missing)?;
                *cache = Some((id, aligned, bytes));
            }
            let (_, _, bytes) = cache.as_ref().ok_or(StoreError::Read)?;
            let start = (offset - aligned) as usize;
            let available = bytes
                .get(start..)
                .filter(|bytes| !bytes.is_empty())
                .ok_or(StoreError::Read)?;
            let count = available.len().min(destination.len());
            destination[..count].copy_from_slice(&available[..count]);
            destination = &mut destination[count..];
            offset += count as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
