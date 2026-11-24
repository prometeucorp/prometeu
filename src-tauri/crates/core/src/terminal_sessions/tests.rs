use super::*;

#[test]
fn sessions_get_dock_safe_ids_and_keep_their_order() {
    let mut board = Board::default();
    let first = create(&mut board, "/Users/me");
    let second = create(&mut board, "/Users/me");
    assert_ne!(first.id, second.id);
    for session in [&first, &second] {
        assert!(session.id.starts_with(PREFIX), "{}", session.id);
        assert!(!session.id.contains(':'), "{}", session.id);
        assert_eq!(session.name, None);
    }
    assert_eq!(board.terminals, [first.clone(), second.clone()]);
    assert_eq!(find(&board, &second.id), Some(&second));
    assert_eq!(find(&board, "missing"), None);
}

#[test]
fn a_blank_name_falls_back_to_the_folder_name() {
    let mut board = Board::default();
    let id = create(&mut board, "/Users/me/code").id;
    let title = |board: &Board| find(board, &id).unwrap().title();
    assert_eq!(title(&board), "code");
    assert!(rename(&mut board, &id, Some("  server  ")));
    assert_eq!(title(&board), "server");
    assert!(!rename(&mut board, &id, Some("server")));
    assert!(rename(&mut board, &id, Some("   ")));
    assert_eq!(find(&board, &id).unwrap().name, None);
    assert_eq!(title(&board), "code");
    assert!(!rename(&mut board, &id, None));
    assert!(!rename(&mut board, "missing", Some("other")));
    let root = TerminalSession {
        id: format!("{PREFIX}root"),
        folder: "/".into(),
        name: None,
    };
    assert_eq!(root.title(), "/");
}

#[test]
fn removing_a_session_keeps_the_others() {
    let mut board = Board::default();
    let first = create(&mut board, "/a");
    let second = create(&mut board, "/b");
    assert!(remove(&mut board, &first.id));
    assert!(!remove(&mut board, &first.id));
    assert_eq!(board.terminals, [second]);
}

/// A board.json written by v0.29.0, before terminal sessions, loads with none and saves back in
/// the same shape except retired workspace stages; a board with sessions keeps them across a save.
#[test]
fn boards_from_v0_29_0_preserve_sessions_without_retired_stages() {
    let mut written: serde_json::Value =
        serde_json::from_str(include_str!("board-v0.29.0.json")).unwrap();
    let mut board: Board = serde_json::from_value(written.clone()).unwrap();
    assert!(board.terminals.is_empty());
    written.as_object_mut().unwrap().remove("stages");
    for workspace in written["workspaces"].as_array_mut().unwrap() {
        workspace.as_object_mut().unwrap().remove("stage");
    }
    assert_eq!(serde_json::to_value(&board).unwrap(), written);

    let session = create(&mut board, "/Users/me");
    rename(&mut board, &session.id, Some("server"));
    let saved = serde_json::to_value(&board).unwrap();
    assert_eq!(
        saved["terminals"],
        serde_json::json!([{"id": session.id, "folder": "/Users/me", "name": "server"}])
    );
    let reloaded: Board = serde_json::from_value(saved).unwrap();
    assert_eq!(reloaded.terminals, board.terminals);
}
