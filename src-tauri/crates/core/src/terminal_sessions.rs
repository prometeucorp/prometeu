//! Terminal sessions belong to the person, not to a workspace or a project (ADR 0092). These rules
//! keep their records on the board; the host checks folders and owns shells and publication.
use crate::board::{Board, TerminalSession};
use std::path::Path;

/// Session ids start here and hold no `:`. Dock keys split `<owner>:<kind>` at the first colon and
/// owners are found by an `"<id>:"` prefix, so a session can never be read as another owner's
/// shell; workspace ids are UUIDs and project ids absolute paths, so neither starts this way.
pub const PREFIX: &str = "terminal-";

/// A session runs exactly one shell, under this dock kind.
pub const SHELL: &str = "terminal";

pub fn is_session(id: &str) -> bool {
    id.strip_prefix(PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains(':'))
}

/// The PTY key of a session's shell.
pub fn key(id: &str) -> String {
    format!("{id}:{SHELL}")
}

/// Add a session for an already checked folder at the end of the list.
pub fn create(board: &mut Board, folder: String) -> TerminalSession {
    let session = TerminalSession {
        id: format!("{PREFIX}{}", uuid::Uuid::new_v4().simple()),
        folder,
        title: String::new(),
        running: false,
    };
    board.terminals.push(session.clone());
    session
}

pub fn find<'a>(board: &'a Board, id: &str) -> Option<&'a TerminalSession> {
    board.terminals.iter().find(|session| session.id == id)
}

/// Keep the trimmed name; an empty one returns the session to its folder name. Answer whether the
/// record changed.
pub fn rename(board: &mut Board, id: &str, title: &str) -> bool {
    let Some(session) = board.terminals.iter_mut().find(|session| session.id == id) else {
        return false;
    };
    let title = title.trim();
    if session.title == title {
        return false;
    }
    session.title = title.to_string();
    true
}

/// Answer whether the record changed.
pub fn set_running(board: &mut Board, id: &str, running: bool) -> bool {
    match board.terminals.iter_mut().find(|session| session.id == id) {
        Some(session) if session.running != running => {
            session.running = running;
            true
        }
        _ => false,
    }
}

pub fn remove(board: &mut Board, id: &str) -> Option<TerminalSession> {
    let at = board
        .terminals
        .iter()
        .position(|session| session.id == id)?;
    Some(board.terminals.remove(at))
}

/// The name the person chose, otherwise the folder's own name; the root folder keeps its path.
pub fn name(session: &TerminalSession) -> String {
    let title = session.title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    Path::new(&session.folder)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.folder.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_distinct_owners_without_a_key_separator() {
        let mut board = Board::default();
        let first = create(&mut board, "/home/person".into());
        let second = create(&mut board, "/home/person".into());

        assert_ne!(first.id, second.id);
        for session in [&first, &second] {
            assert!(is_session(&session.id), "{}", session.id);
            assert!(!session.id.contains(':'));
            // The owner part of its key is the whole id, as `kill_docks` and the resource rows
            // read it.
            assert_eq!(
                key(&session.id).split_once(':'),
                Some((session.id.as_str(), SHELL))
            );
        }
        for other in [
            "4f1c2d8e-8c5b-4f7e-9a1d-2b3c4d5e6f70",
            "/Users/person/dev/terminal-project",
            "terminal-",
            "terminal-abc:terminal",
            "@mesa",
        ] {
            assert!(!is_session(other), "{other}");
        }
        assert_eq!(
            board
                .terminals
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            [first.id.as_str(), second.id.as_str()],
        );
    }

    #[test]
    fn a_session_is_named_after_its_folder_until_the_person_names_it() {
        let mut board = Board::default();
        let session = create(&mut board, "/home/person/dev/api".into());
        assert_eq!(name(&session), "api");

        assert!(rename(&mut board, &session.id, "  server logs "));
        assert_eq!(name(find(&board, &session.id).unwrap()), "server logs");
        assert!(!rename(&mut board, &session.id, "server logs"));

        // Clearing the name falls back to the folder.
        assert!(rename(&mut board, &session.id, "   "));
        assert_eq!(find(&board, &session.id).unwrap().title, "");
        assert_eq!(name(find(&board, &session.id).unwrap()), "api");

        let root = create(&mut board, "/".into());
        assert_eq!(name(&root), "/");
        assert!(!rename(&mut board, "terminal-missing", "x"));
    }

    #[test]
    fn running_state_and_removal_touch_only_the_named_session() {
        let mut board = Board::default();
        let kept = create(&mut board, "/a".into());
        let closed = create(&mut board, "/b".into());

        assert!(set_running(&mut board, &kept.id, true));
        assert!(!set_running(&mut board, &kept.id, true));
        assert!(!set_running(&mut board, "terminal-missing", true));
        assert_eq!(
            remove(&mut board, &closed.id).map(|s| s.folder),
            Some("/b".into())
        );
        assert!(remove(&mut board, &closed.id).is_none());

        assert_eq!(board.terminals.len(), 1);
        assert!(find(&board, &kept.id).unwrap().running);
    }
}
