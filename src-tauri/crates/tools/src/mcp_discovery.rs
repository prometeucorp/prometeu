//! Read-only import discovery. Paths are supplied by the host, never derived from desktop globals.
use crate::mcp::{slug, Server};
use serde_json::Value;
use std::path::Path;
pub fn found(path: &Path, known: &[Server]) -> Vec<Server> {
    distinct(
        from_config(path).into_iter().chain(from_repositories(path)),
        known,
    )
}
pub fn distinct(candidates: impl IntoIterator<Item = Server>, known: &[Server]) -> Vec<Server> {
    let mut found: Vec<Server> = Vec::new();
    for server in candidates {
        // Deduplicate identical named configurations across registered entries and discovered
        // projects.
        if known.iter().any(|s| s.id == server.id)
            || found
                .iter()
                .any(|s| s.id == server.id && s.config == server.config)
        {
            continue;
        }
        // Keep different configurations with the same original name by assigning distinct import
        // names.
        let clash = found.iter().any(|s| s.id == server.id);
        let id = match (clash, server.note.trim()) {
            (true, origin) if !origin.is_empty() => format!("{}-{}", server.id, slug(origin)),
            (true, _) => format!("{}-2", server.id),
            _ => server.id.clone(),
        };
        found.push(Server { id, ..server });
    }
    found
}
fn from_config(path: &Path) -> Vec<Server> {
    let Some(root) = read_json(path) else {
        return vec![];
    };
    let mut out = servers_in(&root, "");
    if let Some(projects) = root.get("projects").and_then(Value::as_object) {
        for (path, project) in projects {
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            out.extend(servers_in(project, &name));
        }
    }
    out
}
fn from_repositories(path: &Path) -> Vec<Server> {
    let Some(root) = read_json(path) else {
        return vec![];
    };
    let Some(projects) = root.get("projects").and_then(Value::as_object) else {
        return vec![];
    };
    let mut out = Vec::new();
    for path in projects.keys() {
        let Some(file) = read_json(&Path::new(path).join(".mcp.json")) else {
            continue;
        };
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        out.extend(servers_in(&file, &name));
    }
    out
}
pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}
/// Extract mcpServers from a user, project, or repository configuration object.
pub fn servers_in(value: &Value, origin: &str) -> Vec<Server> {
    value
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|servers| {
            servers
                .iter()
                .filter(|(_, config)| config.is_object())
                .map(|(id, config)| Server {
                    id: id.clone(),
                    config: config.clone(),
                    note: origin.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}
