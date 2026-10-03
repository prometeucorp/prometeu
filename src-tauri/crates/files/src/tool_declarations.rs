//! Existing repository declaration identity and hashing shared by execution hosts.
use prometeu_core::{selection::Tools, tool_resolution::ProjectDeclaration};
use std::path::Path;
/// Read the primary repository's `[tools]` and derive its trust identity. Returns `None` when the
/// repository declares no tools, so there is nothing to trust.
pub fn project_declaration(
    settings: &dyn crate::settings::RepositorySettings,
    worktree: &Path,
    repo: &Path,
) -> Option<ProjectDeclaration> {
    let scripts = settings.read(worktree, repo);
    if scripts.tools == Tools::default() {
        return None;
    }
    Some(ProjectDeclaration {
        repo: repo_identity(repo),
        hash: tools_hash(&scripts.tools),
        file: scripts.file,
        tools: scripts.tools,
    })
}

/// Identify a repository for trust: its `origin` remote URL when one exists, else the clone's
/// absolute path. The URL survives a moved clone; the path covers a repository without a remote.
fn repo_identity(repo: &Path) -> String {
    let origin = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    let origin = origin.trim();
    if !origin.is_empty() {
        return origin.to_string();
    }
    repo.canonicalize()
        .unwrap_or_else(|_| repo.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Hash the declared `[tools]` so a changed declaration re-prompts. Hashing the parsed structure,
/// not the raw TOML, keeps the decision stable across comments, key order and whitespace.
pub fn tools_hash(tools: &Tools) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(tools).unwrap_or_default();
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}
