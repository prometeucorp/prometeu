//! Copy one explicitly authorized SQLite database and its WAL before querying. SQLite
//! never opens the original path, WAL or shared-memory file; snapshot reads cannot mutate them.
use crate::terminal_agent_observation::{open_authorized, AuthorizedRoot};
use rusqlite::{Connection, OpenFlags};
use std::{
    fs::{File, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const COPY_BUDGET: u64 = 4 * 1024 * 1024;
const CHUNK: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotError {
    Path,
    Read,
    Unstable,
    Schema,
    Identity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SnapshotProgress {
    Pending,
    Unchanged,
    /// A consistent private database snapshot is ready; callers must still decode its referenced
    /// records successfully before declaring transcript coverage or committing completion.
    Ready,
}

pub(crate) struct SnapshotReader {
    root: AuthorizedRoot,
    path: PathBuf,
    staging_root: PathBuf,
    stage: Option<CopyStage>,
    snapshot: Option<SqliteSnapshot>,
    stamp: Option<SourceStamp>,
}

impl SnapshotReader {
    pub(crate) fn new(root: AuthorizedRoot, path: PathBuf, staging_root: PathBuf) -> Self {
        Self {
            root,
            path,
            staging_root,
            stage: None,
            snapshot: None,
            stamp: None,
        }
    }
    pub(crate) fn take_snapshot(&mut self) -> Option<SqliteSnapshot> {
        self.snapshot.take()
    }

    /// At most 4 MiB of source payload is copied per poll. Changes during any part of a copy
    /// discard the staging snapshot and return Unstable; callers bound retries and never infer EOF.
    pub(crate) fn poll(&mut self) -> Result<SnapshotProgress, SnapshotError> {
        if self.stage.is_none() {
            let Some(source) = self.open_source()? else {
                self.snapshot = None;
                self.stamp = None;
                return Ok(SnapshotProgress::Pending);
            };
            if self.stamp.as_ref() == Some(&source.stamp()) {
                return Ok(SnapshotProgress::Unchanged);
            }
            self.snapshot = None;
            self.stage = Some(CopyStage::new(source, &self.staging_root)?);
        }
        let copied = self.stage.as_mut().ok_or(SnapshotError::Read)?.copy();
        match copied {
            Ok(false) => return Ok(SnapshotProgress::Pending),
            Err(error) => {
                self.stage = None;
                return Err(error);
            }
            Ok(true) => {}
        }
        let stage = self.stage.take().ok_or(SnapshotError::Read)?;
        let current = self.open_source()?.ok_or(SnapshotError::Unstable)?;
        if stage.source.stamp() != current.stamp() || !stage.source.unchanged()? {
            return Err(SnapshotError::Unstable);
        }
        let stamp = current.stamp();
        let snapshot = stage.finish()?;
        self.snapshot = Some(snapshot);
        self.stamp = Some(stamp);
        Ok(SnapshotProgress::Ready)
    }

    fn open_source(&mut self) -> Result<Option<Source>, SnapshotError> {
        let roots = std::slice::from_mut(&mut self.root);
        let db = open_authorized(roots, &self.path).map_err(|_| SnapshotError::Path)?;
        let Some(db) = db else {
            return Ok(None);
        };
        let db = Original::new(db)?;
        let mut wal_path = self.path.as_os_str().to_os_string();
        wal_path.push("-wal");
        let wal_path = PathBuf::from(wal_path);
        let wal = open_authorized(roots, &wal_path)
            .map_err(|_| SnapshotError::Path)?
            .map(Original::new)
            .transpose()?;
        Ok(Some(Source { db, wal }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    modified: i64,
    modified_ns: i64,
    changed: i64,
    changed_ns: i64,
}
impl From<&Metadata> for Stamp {
    fn from(value: &Metadata) -> Self {
        Self {
            dev: value.dev(),
            ino: value.ino(),
            len: value.len(),
            modified: value.mtime(),
            modified_ns: value.mtime_nsec(),
            changed: value.ctime(),
            changed_ns: value.ctime_nsec(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceStamp {
    db: Stamp,
    wal: Option<Stamp>,
}
struct Original {
    file: File,
    stamp: Stamp,
}
impl Original {
    fn new(file: File) -> Result<Self, SnapshotError> {
        let metadata = file.metadata().map_err(|_| SnapshotError::Read)?;
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(SnapshotError::Path);
        }
        Ok(Self {
            file,
            stamp: Stamp::from(&metadata),
        })
    }
    fn unchanged(&self) -> Result<bool, SnapshotError> {
        Ok(self.stamp == Stamp::from(&self.file.metadata().map_err(|_| SnapshotError::Read)?))
    }
}
struct Source {
    db: Original,
    wal: Option<Original>,
}
impl Source {
    fn stamp(&self) -> SourceStamp {
        SourceStamp {
            db: self.db.stamp.clone(),
            wal: self.wal.as_ref().map(|wal| wal.stamp.clone()),
        }
    }
    fn unchanged(&self) -> Result<bool, SnapshotError> {
        Ok(self.db.unchanged()?
            && self
                .wal
                .as_ref()
                .map(Original::unchanged)
                .transpose()?
                .unwrap_or(true))
    }
}

struct CopyStage {
    source: Source,
    directory: Option<PathBuf>,
    db: File,
    wal: Option<File>,
    db_offset: u64,
    wal_offset: u64,
}
impl CopyStage {
    fn new(source: Source, root: &Path) -> Result<Self, SnapshotError> {
        crate::paths::ensure_private_dir(root).map_err(|_| SnapshotError::Read)?;
        let directory = root.join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir(&directory).map_err(|_| SnapshotError::Read)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| SnapshotError::Read)?;
        // Platform temp roots may themselves be aliases (for example /var on macOS). SQLite's
        // NOFOLLOW applies to that path too; resolve only our newly created private directory.
        let directory = std::fs::canonicalize(directory).map_err(|_| SnapshotError::Read)?;
        let file = |name| {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(directory.join(name))
                .map_err(|_| SnapshotError::Read)
        };
        let outputs = (|| {
            let db = file("store.db")?;
            let wal = source
                .wal
                .as_ref()
                .map(|_| file("store.db-wal"))
                .transpose()?;
            Ok((db, wal))
        })();
        let (db, wal) = match outputs {
            Ok(outputs) => outputs,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(error);
            }
        };
        Ok(Self {
            source,
            directory: Some(directory),
            db,
            wal,
            db_offset: 0,
            wal_offset: 0,
        })
    }
    fn copy(&mut self) -> Result<bool, SnapshotError> {
        if !self.source.unchanged()? {
            return Err(SnapshotError::Unstable);
        }
        let mut budget = COPY_BUDGET;
        let mut buffer = vec![0; CHUNK];
        copy_part(
            &mut self.source.db,
            &mut self.db,
            &mut self.db_offset,
            &mut budget,
            &mut buffer,
        )?;
        if let (Some(source), Some(target)) = (&mut self.source.wal, &mut self.wal) {
            copy_part(
                source,
                target,
                &mut self.wal_offset,
                &mut budget,
                &mut buffer,
            )?;
        }
        Ok(self.db_offset == self.source.db.stamp.len
            && self
                .source
                .wal
                .as_ref()
                .is_none_or(|wal| self.wal_offset == wal.stamp.len))
    }
    fn finish(mut self) -> Result<SqliteSnapshot, SnapshotError> {
        self.db.flush().map_err(|_| SnapshotError::Read)?;
        if let Some(wal) = &mut self.wal {
            wal.flush().map_err(|_| SnapshotError::Read)?;
        }
        let directory = self.directory.as_ref().ok_or(SnapshotError::Read)?;
        let connection = Connection::open_with_flags(
            directory.join("store.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| SnapshotError::Read)?;
        connection
            .execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF; BEGIN DEFERRED;")
            .map_err(|_| SnapshotError::Read)?;
        Ok(SqliteSnapshot {
            connection: Some(connection),
            directory: self.directory.take().ok_or(SnapshotError::Read)?,
        })
    }
}
impl Drop for CopyStage {
    fn drop(&mut self) {
        if let Some(directory) = &self.directory {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}
fn copy_part(
    source: &mut Original,
    target: &mut File,
    offset: &mut u64,
    budget: &mut u64,
    buffer: &mut [u8],
) -> Result<(), SnapshotError> {
    while *offset < source.stamp.len && *budget > 0 {
        let count = (source.stamp.len - *offset)
            .min(*budget)
            .min(buffer.len() as u64) as usize;
        source
            .file
            .read_exact(&mut buffer[..count])
            .map_err(|_| SnapshotError::Unstable)?;
        target
            .write_all(&buffer[..count])
            .map_err(|_| SnapshotError::Read)?;
        *offset += count as u64;
        *budget -= count as u64;
    }
    Ok(())
}

pub(crate) struct SqliteSnapshot {
    connection: Option<Connection>,
    pub(crate) directory: PathBuf,
}
impl SqliteSnapshot {
    pub(crate) fn connection(&self) -> Result<&Connection, SnapshotError> {
        self.connection.as_ref().ok_or(SnapshotError::Read)
    }
}
impl Drop for SqliteSnapshot {
    fn drop(&mut self) {
        self.connection.take();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
