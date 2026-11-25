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

/// Folder names are trimmed; an empty or absent name takes the projects out of their folder.
/// Unknown IDs are ignored.
pub fn group(projects: &mut [Project], ids: &[String], group: Option<&str>) {
    let group = group.map(str::trim).filter(|g| !g.is_empty());
    for p in projects.iter_mut().filter(|p| ids.contains(&p.id)) {
        p.group = group.map(Into::into);
    }
}

/// Concurrently registered projects keep their relative order after the requested IDs.
pub fn reorder(projects: &mut [Project], ids: &[String]) {
    projects.sort_by_key(|p| ids.iter().position(|id| *id == p.id).unwrap_or(ids.len()));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(id: &str) -> Project {
        serde_json::from_value(serde_json::json!({ "id": id, "name": id, "path": id })).unwrap()
    }
    #[test]
    fn project_folders_persist_trimmed_and_clear_on_empty_names() {
        let mut projects = [project("/project"), project("/another")];
        let ids = ["/project".into(), "/another".into(), "/gone".into()];
        group(&mut projects, &ids, Some("  Work "));
        let groups: Vec<_> = projects.iter().map(|p| p.group.clone()).collect();
        assert_eq!(groups, [Some("Work".into()), Some("Work".into())]);
        group(&mut projects, &["/another".into()], Some(" "));
        let groups: Vec<_> = projects.iter().map(|p| p.group.clone()).collect();
        assert_eq!(groups, [Some("Work".into()), None]);
    }
}
