//! Isolated development-runtime storage; never adopts a desktop root.
use prometeu_core::{conversation::stream::TranscriptStore, lock::lock};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    v: u32,
    workdir: PathBuf,
    provider: String,
    provider_session: Option<String>,
}
/// A host-owned conversation identity and canonical transcript, independent of its process.
pub trait SessionStore: TranscriptStore {
    fn identity(&self, identity: &str) -> Result<(), String>;
    fn resume(&self) -> Option<String>;
    fn workdir(&self) -> PathBuf;
}
pub struct Store {
    root: PathBuf,
    metadata: Mutex<Metadata>,
    _lease: File,
}
impl Store {
    pub fn open(root: &Path, workdir: &Path) -> Result<Self, String> {
        let workdir = match workdir.canonicalize() {
            Ok(path) if path.is_dir() => path,
            Ok(_) => return Err("workdir must be a directory".into()),
            // A cleaned worktree keeps its transcript. Only existing metadata may bind a
            // missing directory; the identity comparison below still prevents adoption.
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && workdir.is_absolute()
                    && root.join("runtime.json").is_file() =>
            {
                workdir.to_owned()
            }
            Err(e) => return Err(e.to_string()),
        };
        std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if !root.join("runtime.json").exists() {
            for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
                if entry.map_err(|e| e.to_string())?.file_name() != "runtime.lock" {
                    return Err("use an empty directory for the headless runtime; desktop roots cannot be adopted".into());
                }
            }
        }
        let lease = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(root.join("runtime.lock"))
            .map_err(|e| e.to_string())?;
        // SAFETY: the open descriptor stays owned by Store for the runtime lifetime.
        if unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("runtime root is already in use".into());
        }
        let path = root.join("runtime.json");
        let metadata: Metadata = match std::fs::read_to_string(&path) {
            Ok(body) => {
                serde_json::from_str(&body).map_err(|e| format!("invalid runtime metadata: {e}"))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
                    if entry.map_err(|e| e.to_string())?.file_name() != "runtime.lock" {
                        return Err("use an empty directory for the headless runtime; desktop roots cannot be adopted".into());
                    }
                }
                Metadata {
                    v: 1,
                    workdir: workdir.clone(),
                    provider: "codex".into(),
                    provider_session: None,
                }
            }
            Err(e) => return Err(e.to_string()),
        };
        if metadata.v != 1 || metadata.provider != "codex" || metadata.workdir != workdir {
            return Err("runtime root version, provider or workdir does not match".into());
        }
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        lease
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        let store = Self {
            root,
            metadata: Mutex::new(metadata),
            _lease: lease,
        };
        store.save(&lock(&store.metadata))?;
        Ok(store)
    }
    fn save(&self, metadata: &Metadata) -> Result<(), String> {
        let temporary = self.root.join("runtime.json.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, metadata).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::rename(temporary, self.root.join("runtime.json")).map_err(|e| e.to_string())?;
        File::open(&self.root)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())
    }
    pub fn identity(&self, identity: &str) -> Result<(), String> {
        let mut metadata = lock(&self.metadata);
        let mut next = metadata.clone();
        next.provider_session = Some(identity.into());
        self.save(&next)?;
        *metadata = next;
        Ok(())
    }
    pub fn resume(&self) -> Option<String> {
        lock(&self.metadata).provider_session.clone()
    }
    pub fn workdir(&self) -> PathBuf {
        lock(&self.metadata).workdir.clone()
    }
}
impl TranscriptStore for Store {
    fn load(&self) -> Result<String, String> {
        match std::fs::read_to_string(self.root.join("transcript.jsonl")) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.to_string()),
        }
    }
    fn append(&self, line: &str) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.root.join("transcript.jsonl"))
            .map_err(|e| e.to_string())?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        writeln!(file, "{line}")
            .and_then(|()| file.sync_data())
            .map_err(|e| e.to_string())
    }
}

impl SessionStore for Store {
    fn identity(&self, identity: &str) -> Result<(), String> {
        Store::identity(self, identity)
    }
    fn resume(&self) -> Option<String> {
        Store::resume(self)
    }
    fn workdir(&self) -> PathBuf {
        Store::workdir(self)
    }
}
