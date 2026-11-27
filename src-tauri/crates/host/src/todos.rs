//! The machine's to-do list (`<root>/todos.json`). Agents on the machine reach it only while a
//! desktop selected this machine to keep the person's list, so they never fill a list that
//! nobody sees. The desktop attachment and the agent socket share one store.
use crate::server::{now, Host};
use prometeu_core::todos::{Status, TodoFile, TodoStore};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const FILE: &str = "todos.json";

pub struct HostFile(pub PathBuf);

impl TodoFile for HostFile {
    fn load(&self) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(&self.0) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, bytes: &[u8]) -> Result<(), String> {
        crate::registry::write_private(&self.0, bytes)
    }
}

pub type Store = TodoStore<HostFile>;

pub fn store(root: &std::path::Path) -> Store {
    TodoStore::new(HostFile(root.join(FILE)))
}

/// Methods that write the list.
pub const CHANGES: &[&str] = &["todos_create", "todos_set_status", "todos_enable"];

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

/// `todos_list`, `todos_create` and `todos_set_status`, for the desktop and for agents. The
/// desktop alone may call `todos_enable`; `attachment` tells which caller this is.
pub fn call(host: &Host, method: &str, params: &Value, attachment: bool) -> Result<Value, String> {
    let here = |enabled: bool| {
        if enabled {
            Ok(())
        } else {
            Err("todos_not_here".to_string())
        }
    };
    let result = match method {
        "todos_list" => {
            let list = host.todos.read()?;
            here(list.enabled)?;
            json!({"items": list.items})
        }
        "todos_create" => {
            let title = text(params, "title")?;
            json!(host.todos.update(|list| {
                here(list.enabled)?;
                list.create(title, now())
            })?)
        }
        "todos_set_status" => {
            let id = text(params, "id")?;
            let status = Status::parse(text(params, "status")?)?;
            json!(host.todos.update(|list| {
                here(list.enabled)?;
                list.set_status(id, status, now())
            })?)
        }
        "todos_enable" if attachment => {
            let enabled = params
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("missing enabled")?;
            host.todos.update(|list| {
                list.enabled = enabled;
                Ok(())
            })?;
            json!({"enabled": enabled})
        }
        other => return Err(format!("unknown method {other}")),
    };
    if CHANGES.contains(&method) {
        host.announce_todos();
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_reach_the_list_only_after_a_desktop_selects_this_machine() {
        let root = std::env::temp_dir().join(format!("host-todos-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let host = crate::server::test_host(&root);
        let create = json!({"title": "From an agent"});
        assert_eq!(
            call(&host, "todos_create", &create, false).unwrap_err(),
            "todos_not_here"
        );
        assert_eq!(
            call(&host, "todos_list", &json!({}), false).unwrap_err(),
            "todos_not_here"
        );
        // Only the desktop attachment can select the machine.
        assert!(call(&host, "todos_enable", &json!({"enabled": true}), false).is_err());
        assert!(!root.join(FILE).exists());
        call(&host, "todos_enable", &json!({"enabled": true}), true).unwrap();
        let todo = call(&host, "todos_create", &create, false).unwrap();
        assert_eq!(todo["status"], "todo");
        let id = todo["id"].as_str().unwrap();
        for (params, error) in [
            (json!({"id": id, "status": "done"}), "invalid_status"),
            (
                json!({"id": "missing", "status": "doing"}),
                "todo_not_found",
            ),
            (json!({"status": "doing"}), "missing id"),
        ] {
            assert_eq!(
                call(&host, "todos_set_status", &params, false).unwrap_err(),
                error
            );
        }
        assert_eq!(
            call(&host, "todos_create", &json!({"title": "one\ntwo"}), false).unwrap_err(),
            "invalid_title"
        );
        call(
            &host,
            "todos_set_status",
            &json!({"id": id, "status": "completed"}),
            true,
        )
        .unwrap();
        // A restarted daemon reads the same list.
        let restarted = crate::server::test_host(&root);
        let items = call(&restarted, "todos_list", &json!({}), false).unwrap()["items"].clone();
        assert_eq!(items[0]["id"], id);
        assert_eq!(items[0]["status"], "completed");
        call(&restarted, "todos_enable", &json!({"enabled": false}), true).unwrap();
        assert_eq!(
            call(&restarted, "todos_list", &json!({}), true).unwrap_err(),
            "todos_not_here"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
