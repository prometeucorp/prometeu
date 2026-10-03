//! Desktop adapters for the portable board model and ordered publication service.

use crate::AppState;
pub use prometeu_core::board::{
    split_skills, Board, Choice, Note, Project, ProviderId, Repo, Status, Tab, ToolTrust, Workspace,
};
use prometeu_core::publication::BoardEvents;
use tauri::{AppHandle, Emitter, Manager};

struct DesktopBoardEvents<'a>(&'a AppHandle);

impl BoardEvents for DesktopBoardEvents<'_> {
    fn changed(&self, board: &Board) -> Result<(), String> {
        self.0
            .emit("board", board)
            .map_err(|error| error.to_string())
    }
}

/// Preserve the desktop event name and payload at the transport edge.
pub fn publish(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Err(error) = state.save.publish(&state.board, &DesktopBoardEvents(app)) {
        eprintln!("could not publish board: {error}");
    }
}

pub fn save_now(app: &AppHandle) {
    if let Err(error) = persist_now(app) {
        eprintln!("could not persist board: {error}");
    }
}

pub(crate) fn persist_now(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.save.persist(&state.board)
}
