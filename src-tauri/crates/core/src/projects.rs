//! Project registration does not create or remove workspaces or project files.
use crate::board::{Board, Project};
pub fn register(board: &mut Board, project: Project) -> Project {
    if let Some(existing) = board.projects.iter().find(|p| p.path == project.path) {
        return existing.clone();
    }
    board.projects.push(project.clone());
    project
}
pub fn remove(board: &mut Board, id: &str) {
    board.projects.retain(|p| p.id != id);
}

/// Concurrently registered projects keep their relative order after the requested IDs.
pub fn reorder(projects: &mut [Project], ids: &[String]) {
    projects.sort_by_key(|p| ids.iter().position(|id| *id == p.id).unwrap_or(ids.len()));
}
