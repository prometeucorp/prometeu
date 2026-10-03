//! Typed workspace operations, independent of desktop and WSL process details.
pub use prometeu_core::workspaces::{Catalog, WorktreeRequest};
pub trait WorkspaceClient: Send {
    fn workspace_worktree(&mut self, request: WorktreeRequest) -> Result<Catalog, String>;
    fn workspace_list(&mut self) -> Result<Catalog, String>;
    fn workspace_create(&mut self, title: String, path: String) -> Result<Catalog, String>;
    fn workspace_select(&mut self, id: String) -> Result<Catalog, String>;
    fn workspace_stage(&mut self, id: String, stage: String) -> Result<Catalog, String>;
}
