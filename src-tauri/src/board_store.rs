//! Existing board.json persistence, composed with an explicit root.
use crate::{lock::lock, paths, recovery};
use prometeu_core::{board::Board, publication::BoardStore};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct FileBoardStore {
    path: PathBuf,
    state: Mutex<State>,
}

/// How the last load fell back, for the notice shown at startup.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoardRecovery {
    /// `backup` when the board came from `board.json.bak`, `reset` when it started empty.
    pub source: &'static str,
    /// Holds byte-for-byte copies of the files that could not be read, once there are any.
    pub folder: Option<String>,
    /// Why the unreadable board.json could not be kept; saving stays blocked while it is set.
    pub error: Option<String>,
}

#[derive(Default)]
struct State {
    recovery: Option<Recovery>,
    /// The backup on disk is missing, readable or kept in the recovery folder, so a save may
    /// replace it. Only this store writes it afterwards, always from a board that parses.
    backup_kept: bool,
}

struct Recovery {
    source: &'static str,
    /// The unreadable board.json, until it is kept. No save may replace it before that.
    pending: Option<Unreadable>,
    kept: bool,
    error: Option<String>,
}

struct Unreadable {
    label: &'static str,
    /// The bytes read at load; `None` when the file itself could not be read, so no copy can
    /// protect it.
    bytes: Option<Vec<u8>>,
    error: String,
}

impl Unreadable {
    fn keep(&self, folder: &Path) -> Result<(), String> {
        match &self.bytes {
            Some(bytes) => recovery::keep(folder, self.label, bytes),
            None => Err(self.error.clone()),
        }
    }
}

impl Recovery {
    /// Copy the unreadable board.json before a save may replace it. A failure blocks the save.
    fn preserve(&mut self, folder: &Path) -> Result<(), String> {
        if let Some(file) = &self.pending {
            if let Err(error) = file.keep(folder) {
                self.error = Some(error.clone());
                return Err(error);
            }
            self.pending = None;
            self.kept = true;
        }
        self.error = None;
        Ok(())
    }
}

/// `Ok(None)` means the file does not exist.
fn read(path: &Path, label: &'static str) -> Result<Option<Board>, Unreadable> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Unreadable {
                label,
                bytes: None,
                error: error.to_string(),
            })
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| Unreadable {
            label,
            bytes: Some(bytes),
            error: error.to_string(),
        })
}

impl FileBoardStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            path: root.join("board.json"),
            state: Mutex::default(),
        }
    }

    pub fn recovery_folder(&self) -> PathBuf {
        recovery::folder(self.path.parent().unwrap_or(Path::new(".")))
    }

    fn backup(&self) -> PathBuf {
        self.path.with_extension("json.bak")
    }

    /// Present when the last load did not come from a readable board.json.
    pub fn recovery(&self) -> Option<BoardRecovery> {
        lock(&self.state)
            .recovery
            .as_ref()
            .map(|recovery| BoardRecovery {
                source: recovery.source,
                folder: recovery
                    .kept
                    .then(|| self.recovery_folder().display().to_string()),
                error: recovery.error.clone(),
            })
    }
}

impl BoardStore for FileBoardStore {
    fn load(&self) -> Board {
        let backup = self.backup();
        let current = match read(&self.path, "board") {
            // The backup is checked before the first save that would replace it.
            Ok(Some(board)) => return board,
            Ok(None) => None,
            Err(file) => {
                eprintln!(
                    "invalid board.json ({}); trying backup {}",
                    file.error,
                    backup.display()
                );
                Some(file)
            }
        };
        let folder = self.recovery_folder();
        let mut state = lock(&self.state);
        let mut kept = false;
        let (board, source) = match read(&backup, "board-backup") {
            Ok(Some(board)) => {
                state.backup_kept = true;
                (board, "backup")
            }
            Ok(None) if current.is_none() => return Board::default(),
            Ok(None) => {
                state.backup_kept = true;
                (Board::default(), "reset")
            }
            Err(file) => {
                eprintln!("invalid board.json backup ({})", file.error);
                // Kept now so the notice can point at it. Without a copy, saves leave it alone.
                state.backup_kept = file.keep(&folder).is_ok();
                kept = state.backup_kept;
                (Board::default(), "reset")
            }
        };
        let mut recovery = Recovery {
            source,
            pending: current,
            kept,
            error: None,
        };
        // A failure is retried before every save, which stays blocked until it succeeds.
        let _ = recovery.preserve(&folder);
        state.recovery = Some(recovery);
        board
    }
    fn save(&self, board: &Board) -> Result<(), String> {
        let mut state = lock(&self.state);
        if let Some(recovery) = state.recovery.as_mut() {
            recovery.preserve(&self.recovery_folder())?;
        }
        if !state.backup_kept {
            state.backup_kept = match read(&self.backup(), "board-backup") {
                Ok(_) => true,
                Err(file) => file.keep(&self.recovery_folder()).is_ok(),
            };
        }
        let path = &self.path;
        let json = serde_json::to_string_pretty(board).map_err(|error| error.to_string())?;
        // Only back up a board that still deserializes, over a backup whose bytes are safe.
        // Replacing the last valid backup with external corruption would remove the recovery path.
        if let Ok(previous) = std::fs::read_to_string(path) {
            if state.backup_kept && serde_json::from_str::<Board>(&previous).is_ok() {
                paths::write_private(&self.backup(), &previous)?;
            }
        }
        paths::write_private(path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn board_json(extra: &str) -> Board {
        let json = format!(
            r#"{{"stages":["Fazendo"],"projects":[],"workspaces":[
                 {{"id":"w","title":"t","repo":"/r","repo_name":"r",
                   "branch":"b","worktree":"/wt","stage":"Fazendo"{extra}}}]}}"#
        );
        serde_json::from_str(&json).expect("board did not deserialize")
    }
    fn temporary_board_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("prometeu-board-{}", uuid::Uuid::new_v4()))
            .join("board.json")
    }
    fn store(current: &Path) -> FileBoardStore {
        FileBoardStore::new(current.parent().unwrap().to_path_buf())
    }
    /// Bytes of every copy in the recovery folder whose name starts with `label-`, oldest first.
    fn copies(store: &FileBoardStore, label: &str) -> Vec<Vec<u8>> {
        let mut names: Vec<_> = std::fs::read_dir(store.recovery_folder())
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path.file_name().unwrap().to_str().unwrap();
                name.strip_prefix(label)
                    .and_then(|rest| rest.strip_prefix('-'))
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
            })
            .collect();
        names.sort();
        names
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect()
    }

    #[test]
    fn corrupt_boards_recover_from_the_last_valid_backup() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, "{truncated").unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = store(&current).load();

        assert_eq!(recovered.workspaces.len(), 1);
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn backup_recovers_a_missing_primary_board() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = store(&current).load();

        assert_eq!(recovered.workspaces.len(), 1);
        assert_eq!(recovered.workspaces[0].id, "w");
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn saving_after_corruption_preserves_the_last_valid_backup() {
        let current = temporary_board_path();
        let store = store(&current);
        let mut board = board_json("");
        board.workspaces[0].title = "Recoverable".into();
        store.save(&board).unwrap();
        board.workspaces[0].title = "Recent".into();
        store.save(&board).unwrap();
        std::fs::write(&current, "{truncated").unwrap();
        board.workspaces[0].title = "Recovered write".into();
        store.save(&board).unwrap();
        std::fs::remove_file(&current).unwrap();
        assert_eq!(store.load().workspaces[0].title, "Recoverable");
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_unreadable_board_is_kept_before_the_backup_replaces_it() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, b"{\"stages\":\xff").unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();
        let store = store(&current);

        let board = store.load();
        store.save(&board).unwrap();
        store.save(&board).unwrap();

        assert_eq!(board.workspaces.len(), 1);
        let recovery = store.recovery().unwrap();
        assert_eq!(recovery.source, "backup");
        assert_eq!(
            recovery.folder.as_deref(),
            Some(store.recovery_folder().to_str().unwrap())
        );
        assert_eq!(recovery.error, None);
        assert_eq!(copies(&store, "board"), vec![b"{\"stages\":\xff".to_vec()]);
        assert!(copies(&store, "board-backup").is_empty());
        assert_eq!(store.load().workspaces.len(), 1);
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn unreadable_board_and_backup_survive_two_saves_byte_for_byte() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        // A board written by a newer version that this one cannot deserialize.
        let newer = br#"{"stages":["Doing"],"workspaces":[{"id":"w","tabs":{"future":true}}]}"#;
        std::fs::write(&current, newer).unwrap();
        std::fs::write(current.with_extension("json.bak"), b"{truncated").unwrap();
        let store = store(&current);

        let mut board = store.load();
        assert!(board.workspaces.is_empty());
        board
            .telemetry_ids
            .0
            .insert("workspace:w".into(), "first".into());
        store.save(&board).unwrap();
        board
            .telemetry_ids
            .0
            .insert("workspace:w".into(), "second".into());
        store.save(&board).unwrap();

        assert_eq!(store.recovery().unwrap().source, "reset");
        assert_eq!(copies(&store, "board"), vec![newer.to_vec()]);
        assert_eq!(copies(&store, "board-backup"), vec![b"{truncated".to_vec()]);
        // Starting again from the same unreadable bytes does not pile up copies.
        std::fs::write(&current, newer).unwrap();
        std::fs::write(current.with_extension("json.bak"), b"{truncated").unwrap();
        let again = FileBoardStore::new(current.parent().unwrap().to_path_buf());
        again.load();
        assert_eq!(copies(&again, "board").len(), 1);
        assert_eq!(copies(&again, "board-backup").len(), 1);
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_failed_copy_blocks_saving_until_the_copy_succeeds() {
        let current = temporary_board_path();
        let root = current.parent().unwrap();
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(&current, "{truncated").unwrap();
        let store = store(&current);
        // A file where the recovery folder belongs makes every copy fail.
        std::fs::write(store.recovery_folder(), "").unwrap();

        let board = store.load();
        let recovery = store.recovery().unwrap();
        assert_eq!(recovery.source, "reset");
        assert_eq!(recovery.folder, None);
        assert!(recovery.error.is_some());
        assert!(store.save(&board).is_err());
        assert_eq!(std::fs::read(&current).unwrap(), b"{truncated");

        std::fs::remove_file(store.recovery_folder()).unwrap();
        store.save(&board).unwrap();
        assert_eq!(store.recovery().unwrap().error, None);
        assert_eq!(copies(&store, "board"), vec![b"{truncated".to_vec()]);
        assert!(store.load().workspaces.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn readable_or_absent_boards_report_no_recovery() {
        let current = temporary_board_path();
        let store = store(&current);
        store.load();
        assert_eq!(store.recovery(), None);
        store.save(&board_json("")).unwrap();
        store.load();
        assert_eq!(store.recovery(), None);
        assert!(!store.recovery_folder().exists());

        // A missing board restored from its backup is reported without copies.
        std::fs::remove_file(&current).unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();
        let restored = FileBoardStore::new(current.parent().unwrap().to_path_buf());
        assert_eq!(restored.load().workspaces.len(), 1);
        assert_eq!(
            restored.recovery(),
            Some(BoardRecovery {
                source: "backup",
                folder: None,
                error: None
            })
        );
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn an_unreadable_backup_is_kept_before_a_save_replaces_it() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, serde_json::to_string(&board_json("")).unwrap()).unwrap();
        let backup = current.with_extension("json.bak");
        std::fs::write(&backup, "{truncated").unwrap();
        let store = store(&current);
        // Without a place for the copy, saves still write the board but leave the backup alone.
        std::fs::write(store.recovery_folder(), "").unwrap();

        let board = store.load();
        assert_eq!(store.recovery(), None);
        store.save(&board).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), b"{truncated");

        std::fs::remove_file(store.recovery_folder()).unwrap();
        store.save(&board).unwrap();
        store.save(&board).unwrap();
        assert_eq!(copies(&store, "board-backup"), vec![b"{truncated".to_vec()]);
        assert_eq!(
            serde_json::from_slice::<Board>(&std::fs::read(&backup).unwrap())
                .unwrap()
                .workspaces
                .len(),
            1
        );
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }
}
