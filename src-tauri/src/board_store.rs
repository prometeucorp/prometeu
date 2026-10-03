//! Existing board.json persistence, composed with an explicit root.
use crate::paths;
use prometeu_core::{board::Board, publication::BoardStore};
use std::path::PathBuf;

pub struct FileBoardStore {
    path: PathBuf,
}

impl FileBoardStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            path: root.join("board.json"),
        }
    }
}

impl BoardStore for FileBoardStore {
    fn load(&self) -> Board {
        let current = &self.path;
        let backup = current.with_extension("json.bak");
        let read = |candidate: &std::path::Path| -> Result<Board, String> {
            let text = std::fs::read_to_string(candidate).map_err(|error| error.to_string())?;
            serde_json::from_str(&text).map_err(|error| error.to_string())
        };
        match read(current) {
            Ok(board) => board,
            Err(error) => match read(&backup) {
                Ok(board) => {
                    if current.exists() {
                        eprintln!(
                            "invalid board.json ({error}); trying backup {}",
                            backup.display()
                        );
                    }
                    board
                }
                Err(backup_error) => {
                    if current.exists() || backup.exists() {
                        eprintln!("invalid board.json and backup: {error}; {backup_error}");
                    }
                    Board::default()
                }
            },
        }
    }
    fn save(&self, board: &Board) -> Result<(), String> {
        let path = &self.path;
        let json = serde_json::to_string_pretty(board).map_err(|error| error.to_string())?;
        // Only back up a board that still deserializes. Replacing the last valid backup with
        // external corruption would remove the recovery path.
        if let Ok(previous) = std::fs::read_to_string(path) {
            if serde_json::from_str::<Board>(&previous).is_ok() {
                paths::write_private(&path.with_extension("json.bak"), &previous)?;
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

        let recovered = FileBoardStore {
            path: current.clone(),
        }
        .load();

        assert_eq!(recovered.stages, vec!["Fazendo"]);
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

        let recovered = FileBoardStore {
            path: current.clone(),
        }
        .load();

        assert_eq!(recovered.stages, vec!["Fazendo"]);
        assert_eq!(recovered.workspaces[0].id, "w");
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn injected_roots_keep_boards_and_backups_independent() {
        let first_path = temporary_board_path();
        let second_path = temporary_board_path();
        let first = FileBoardStore {
            path: first_path.clone(),
        };
        let second = FileBoardStore {
            path: second_path.clone(),
        };
        let mut board = board_json("");
        board.workspaces[0].title = "First root".into();
        first.save(&board).unwrap();
        board.workspaces[0].title = "Second root".into();
        second.save(&board).unwrap();
        board.workspaces[0].title = "Updated first root".into();
        first.save(&board).unwrap();

        assert_eq!(first.load().workspaces[0].title, "Updated first root");
        assert_eq!(second.load().workspaces[0].title, "Second root");
        let backup: Board = serde_json::from_str(
            &std::fs::read_to_string(first_path.with_extension("json.bak")).unwrap(),
        )
        .unwrap();
        assert_eq!(backup.workspaces[0].title, "First root");
        assert!(!second_path.with_extension("json.bak").exists());
        std::fs::remove_dir_all(first_path.parent().unwrap()).unwrap();
        std::fs::remove_dir_all(second_path.parent().unwrap()).unwrap();
    }

    #[test]
    fn saving_after_corruption_preserves_the_last_valid_backup() {
        let current = temporary_board_path();
        let store = FileBoardStore {
            path: current.clone(),
        };
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
}
