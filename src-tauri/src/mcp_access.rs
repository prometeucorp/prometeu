//! Transport-independent MCP client identity and private, revocable local credentials.

use crate::{paths, state::Board};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Client {
    pub id: String,
    pub conversation: Option<String>,
    pub projects: Vec<String>,
    /// An external client may use the person's to-do list. Only `register --todos` grants it;
    /// credentials from before the field deny it.
    #[serde(default)]
    pub todos: bool,
}

impl Client {
    pub fn conversation(id: &str) -> Self {
        Self {
            id: id.into(),
            conversation: Some(id.into()),
            projects: vec![],
            todos: false,
        }
    }

    pub fn validate(&self, board: &Board) -> Result<(), String> {
        if self
            .conversation
            .as_ref()
            .is_some_and(|id| board.workspace_of(id).is_none())
        {
            return Err("coordinator_unavailable".into());
        }
        Ok(())
    }

    pub fn allows(&self, board: &Board, path: &str) -> bool {
        match &self.conversation {
            Some(id) => board.workspace_of(id).is_some_and(|workspace| {
                workspace.primary().path == path
                    || workspace.repos.iter().any(|repo| repo.path == path)
            }),
            None => self.projects.iter().any(|project| project == path),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Credential {
    version: u8,
    name: String,
    token: String,
    client: Client,
}

fn credential_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    let id = uuid::Uuid::parse_str(id).map_err(|_| "invalid_client_id")?;
    Ok(root.join("mcp-clients").join(format!("{id}.json")))
}

pub fn authenticate(root: &Path, token: &str) -> Result<Client, String> {
    let (id, _) = token.split_once('.').ok_or("unauthorized")?;
    let read = || -> Result<Client, String> {
        let file = credential_path(root, id)?;
        let credential: Credential =
            serde_json::from_str(&std::fs::read_to_string(file).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if credential.version != 1
            || credential.token != token
            || credential.client.id != format!("client:{id}")
            || credential.client.conversation.is_some()
            || (credential.client.projects.is_empty() && !credential.client.todos)
        {
            return Err("unauthorized".into());
        }
        Ok(credential.client)
    };
    read().map_err(|_| "unauthorized".into())
}

fn register(
    root: &Path,
    name: &str,
    projects: Vec<String>,
    todos: bool,
) -> Result<(String, String), String> {
    if name.trim().is_empty() || name.len() > 200 || (projects.is_empty() && !todos) {
        return Err("A client name and at least one project or --todos are required".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let token = format!("{id}.{}", uuid::Uuid::new_v4());
    let credential = Credential {
        version: 1,
        name: name.into(),
        token: token.clone(),
        client: Client {
            id: format!("client:{id}"),
            conversation: None,
            projects,
            todos,
        },
    };
    paths::write_private(
        &credential_path(root, &id)?,
        &serde_json::to_string(&credential).map_err(|e| e.to_string())?,
    )?;
    Ok((id, token))
}

const USAGE: &str = "Usage: --prometeu-mcp-client projects | register [--todos] NAME [PROJECT_ID...] | list | revoke CLIENT_ID";

/// Local administration is deliberately outside the agent-facing tools. Only explicit registration
/// grants access; listing clients never prints their credentials.
pub fn cli(args: &[String]) -> Result<Value, String> {
    let root = paths::root();
    match args.first().map(String::as_str) {
        Some("projects") if args.len() == 1 => Ok(json!({"projects": projects(&root)?})),
        Some("register") if args.len() >= 2 => {
            let todos = args.iter().any(|arg| arg == "--todos");
            let rest: Vec<&String> = args[1..].iter().filter(|arg| *arg != "--todos").collect();
            let Some((name, ids)) = rest.split_first() else {
                return Err(USAGE.into());
            };
            let available = projects(&root)?;
            let scope = ids
                .iter()
                .map(|id| {
                    available
                        .iter()
                        .find(|project| project.id == **id)
                        .map(|project| project.path.clone())
                        .ok_or_else(|| format!("Unknown project: {id}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let root = root.canonicalize().map_err(|e| e.to_string())?;
            let (id, token) = register(&root, name, scope, todos)?;
            Ok(json!({"client_id":id, "mcpServers":{"prometeu":{
                "type":"stdio", "command":executable, "args":["--prometeu-mcp"],
                "env":{"PROMETEU_ROOT":root,"PROMETEU_MCP_TOKEN":token}
            }}}))
        }
        Some("list") if args.len() == 1 => {
            let directory = root.join("mcp-clients");
            if !directory.exists() {
                return Ok(json!({"clients":[]}));
            }
            let mut clients = vec![];
            for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                let credential: Credential = serde_json::from_str(
                    &std::fs::read_to_string(&path).map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                clients.push(json!({"id":path.file_stem().and_then(|id| id.to_str()),"name":credential.name,"projects":credential.client.projects,"todos":credential.client.todos}));
            }
            Ok(json!({"clients":clients}))
        }
        Some("revoke") if args.len() == 2 => {
            std::fs::remove_file(credential_path(&root, &args[1])?).map_err(|e| e.to_string())?;
            Ok(json!({"revoked":args[1]}))
        }
        _ => Err(USAGE.into()),
    }
}

fn projects(root: &Path) -> Result<Vec<crate::state::Project>, String> {
    // Read only the project catalog. Never revive, save or mutate the running app's board.
    let board: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("board.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    serde_json::from_value(board["projects"].clone()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn independent_clients_survive_reload_and_fail_closed_after_revocation() {
        let root = std::env::temp_dir().join(format!("prometeu-access-{}", uuid::Uuid::new_v4()));
        let (id, token) = register(&root, "external agent", vec!["/repo".into()], false).unwrap();
        let (_, other) = register(&root, "other agent", vec!["/other".into()], false).unwrap();
        let board: Board = serde_json::from_value(json!({"stages":[],"workspaces":[]})).unwrap();
        let client = authenticate(&root, &token).unwrap();
        assert_eq!(client.id, format!("client:{id}"));
        assert!(client.validate(&board).is_ok());
        assert!(client.allows(&board, "/repo"));
        assert!(!client.allows(&board, "/other"));
        assert!(authenticate(&root, &format!("{id}.wrong")).is_err());
        assert!(authenticate(&root, "../escape.secret").is_err());
        let path = credential_path(&root, &id).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(authenticate(&root, &token).unwrap().id, client.id);
        paths::write_private(&path, "broken").unwrap();
        assert!(authenticate(&root, &token).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(authenticate(&root, &token).is_err());
        assert!(authenticate(&root, &other).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_todo_grant_is_explicit_and_older_credentials_deny_it() {
        let root = std::env::temp_dir().join(format!("prometeu-access-{}", uuid::Uuid::new_v4()));
        assert!(register(&root, "nothing", vec![], false).is_err());
        let (_, projects) = register(&root, "projects", vec!["/repo".into()], false).unwrap();
        assert!(!authenticate(&root, &projects).unwrap().todos);
        let (_, todos) = register(&root, "todos", vec![], true).unwrap();
        let client = authenticate(&root, &todos).unwrap();
        assert!(client.todos && client.projects.is_empty());
        let board: Board = serde_json::from_value(json!({"stages":[],"workspaces":[]})).unwrap();
        assert!(!client.allows(&board, "/repo"));
        // A version 1 file written before the grant existed.
        let id = uuid::Uuid::new_v4().to_string();
        let token = format!("{id}.secret");
        let legacy = json!({"version":1,"name":"old","token":token,
            "client":{"id":format!("client:{id}"),"conversation":null,"projects":["/repo"]}});
        paths::write_private(&credential_path(&root, &id).unwrap(), &legacy.to_string()).unwrap();
        let old = authenticate(&root, &token).unwrap();
        assert!(!old.todos && old.allows(&board, "/repo"));
        // A file that names neither projects nor the grant gives no access at all.
        let empty = json!({"version":1,"name":"empty","token":token,
            "client":{"id":format!("client:{id}"),"conversation":null,"projects":[]}});
        paths::write_private(&credential_path(&root, &id).unwrap(), &empty.to_string()).unwrap();
        assert!(authenticate(&root, &token).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
