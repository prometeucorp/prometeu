//! Terminal sessions: local shells the person opens on their own, outside any workspace or project
//! (ADR 0092). The board keeps each record and whether its shell runs; the single shell runs under
//! `<id>:terminal` through the dock shell path, without workspace variables.

use crate::lock::lock;
use crate::state::{publish, Board};
use crate::{dock, i18n, machine, paths, platform, pty, AppState};
use prometeu_core::board::TerminalSession;
use prometeu_core::terminal_sessions as sessions;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

type Ptys = Mutex<HashMap<String, pty::Pty>>;

/// Without a folder the session starts in the home folder. Creating one registers no project,
/// creates no worktree or branch, reserves no port and starts no agent; opening its view starts
/// the shell.
#[tauri::command]
pub fn create_terminal(
    app: AppHandle,
    state: State<AppState>,
    folder: Option<String>,
) -> Result<TerminalSession, String> {
    let session = create(
        &state.board,
        folder.as_deref().map(Path::new),
        &paths::home(),
    )?;
    publish(&app);
    Ok(session)
}

fn create(
    board: &Mutex<Board>,
    folder: Option<&Path>,
    home: &Path,
) -> Result<TerminalSession, String> {
    let folder = checked(folder.unwrap_or(home))?;
    Ok(sessions::create(
        &mut lock(board),
        folder.display().to_string(),
    ))
}

/// An empty title gives the session back its folder name.
#[tauri::command]
pub fn rename_terminal(app: AppHandle, state: State<AppState>, id: String, title: String) {
    if sessions::rename(&mut lock(&state.board), &id, &title) {
        publish(&app);
    }
}

/// Closing ends the session: its record leaves the board, then its shell gets the shutdown every
/// dock shell gets (hangup, terminate, kill).
#[tauri::command]
pub fn close_terminal(app: AppHandle, state: State<AppState>, id: String) {
    let removed = sessions::remove(&mut lock(&state.board), &id).is_some();
    pty::kill(&state, &sessions::key(&id));
    if removed {
        publish(&app);
    }
    machine::publish_counts(&app);
}

/// Start the session's shell, or reuse the running one with its retained output. A folder that is
/// gone is refused rather than replaced, so a restored session never opens somewhere else.
pub(crate) fn open_shell(
    app: &AppHandle,
    state: &State<AppState>,
    id: &str,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    let key = sessions::key(id);
    let folder = {
        let board = lock(&state.board);
        sessions::find(&board, id).map(|session| PathBuf::from(&session.folder))
    }
    .ok_or_else(|| i18n::t("err.terminal.gone"))?;
    let root = checked(&folder)?;
    {
        // Check and insert under one lock so a repeated open cannot start a second shell.
        let mut ptys = lock(&state.ptys);
        if ptys.get(&key).is_some_and(|pty| pty.alive()) {
            return Ok(key);
        }
        let (exit_app, exit_id) = (app.clone(), id.to_string());
        let on_exit: pty::OnExit = Box::new(move |_| {
            let Some(state) = exit_app.try_state::<AppState>() else {
                return;
            };
            if ended(&state.board, &state.ptys, &exit_id) {
                publish(&exit_app);
            }
        });
        let cmd = dock::shell_command(&platform::shell(), &root, None);
        let dock = pty::Dock {
            on_exit: Some(on_exit),
            ..pty::Dock::default()
        };
        ptys.insert(key.clone(), pty::spawn(app, &key, cmd, cols, rows, dock)?);
    }
    if sessions::set_running(&mut lock(&state.board), id, true) {
        publish(app);
    }
    machine::publish_counts(app);
    Ok(key)
}

/// The webview's folders must be absolute existing directories; the same check guards a restored
/// session whose folder was removed meanwhile.
fn checked(folder: &Path) -> Result<PathBuf, String> {
    if folder.is_absolute() && folder.is_dir() {
        Ok(folder.to_path_buf())
    } else {
        Err(i18n::ta(
            "err.terminal.folder",
            &[("path", folder.display().to_string())],
        ))
    }
}

/// The shell ended. Only an end the shell reached on its own (`exit`, a crash, an outside signal)
/// ends the session: closing it, quitting the app and starting a replacement take the PTY out of
/// the map first, so the record stays. Answer whether the board changed.
fn ended(board: &Mutex<Board>, ptys: &Ptys, id: &str) -> bool {
    let key = sessions::key(id);
    let (gone, running) = {
        let mut ptys = lock(ptys);
        match ptys.get(&key).map(|pty| pty.alive()) {
            Some(false) => (ptys.remove(&key), false),
            Some(true) => (None, true),
            None => (None, false),
        }
    };
    // Drop the finished terminal outside the map's lock.
    let on_its_own = gone.is_some();
    drop(gone);
    let mut board = lock(board);
    if on_its_own {
        sessions::remove(&mut board, id).is_some()
    } else {
        sessions::set_running(&mut board, id, running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometeu_core::terminal::{TerminalControl, TerminalEvents, TerminalOutput, TerminalSize};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct Shell(AtomicBool);
    impl TerminalControl for Shell {
        fn resize(&self, _: TerminalSize) -> Result<(), String> {
            Ok(())
        }
        fn running(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
        fn system_id(&self) -> u32 {
            0
        }
        fn close(&self) {}
    }
    struct Quiet;
    impl TerminalEvents for Quiet {
        fn output(&self, _: &[u8], _: u64) {}
        fn closed(&self, _: Option<u32>) {}
    }
    fn shell(running: bool) -> pty::Pty {
        pty::Pty::new(
            Box::new(std::io::sink()),
            Arc::new(Shell(AtomicBool::new(running))),
            Arc::new(TerminalOutput::new(Arc::new(Quiet))),
            None,
        )
    }

    /// A fresh folder under the system temporary directory, removed when dropped.
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("prometeu-terminal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn running_session(board: &Mutex<Board>, folder: &Path) -> TerminalSession {
        let session = create(board, Some(folder), folder).unwrap();
        assert!(sessions::set_running(&mut lock(board), &session.id, true));
        session
    }

    #[test]
    fn sessions_start_home_or_in_an_existing_folder_and_never_elsewhere() {
        let (home, picked) = (Folder::new(), Folder::new());
        let file = picked.0.join("notes.txt");
        std::fs::write(&file, "").unwrap();
        let board = Mutex::new(Board::default());

        let at_home = create(&board, None, &home.0).unwrap();
        assert_eq!(at_home.folder, home.0.display().to_string());
        let chosen = create(&board, Some(&picked.0), &home.0).unwrap();
        assert_eq!(chosen.folder, picked.0.display().to_string());

        for refused in [
            file.as_path(),
            &picked.0.join("missing"),
            Path::new("relative/folder"),
        ] {
            let error = create(&board, Some(refused), &home.0).unwrap_err();
            assert!(error.contains("err.terminal.folder"), "{error}");
        }
        // A refused folder adds nothing; no project or workspace appears either.
        let board = lock(&board);
        assert_eq!(board.terminals.len(), 2);
        assert!(board.projects.is_empty() && board.workspaces.is_empty());
    }

    #[test]
    fn a_shell_ending_on_its_own_ends_its_session() {
        let folder = Folder::new();
        let board = Mutex::new(Board::default());
        let session = running_session(&board, &folder.0);
        let ptys: Ptys = Mutex::new(HashMap::from([(sessions::key(&session.id), shell(false))]));

        assert!(ended(&board, &ptys, &session.id));
        assert!(lock(&board).terminals.is_empty());
        assert!(lock(&ptys).is_empty());
    }

    #[test]
    fn closing_quitting_or_replacing_the_shell_keeps_the_session() {
        let folder = Folder::new();
        let board = Mutex::new(Board::default());

        // Stopped through its dock key, or by the app ending: no PTY is left under the key.
        let stopped = running_session(&board, &folder.0);
        let ptys: Ptys = Mutex::new(HashMap::new());
        assert!(ended(&board, &ptys, &stopped.id));
        assert!(!sessions::find(&lock(&board), &stopped.id).unwrap().running);

        // A newer shell already took the key: the old one's end changes nothing.
        let replaced = running_session(&board, &folder.0);
        let ptys: Ptys = Mutex::new(HashMap::from([(sessions::key(&replaced.id), shell(true))]));
        assert!(!ended(&board, &ptys, &replaced.id));
        assert!(sessions::find(&lock(&board), &replaced.id).unwrap().running);
        assert_eq!(lock(&ptys).len(), 1);

        // Closing removed the record first; the late end finds nothing to change.
        assert!(sessions::remove(&mut lock(&board), &stopped.id).is_some());
        assert!(!ended(&board, &Mutex::new(HashMap::new()), &stopped.id));
        assert_eq!(lock(&board).terminals.len(), 1);
    }
}
