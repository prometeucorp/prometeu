//! `prometeu-host mcp`: a stdio MCP server for bot platforms on the machine (ADR 0103). Each tool
//! is one agent-socket method; the host does the work and enforces the token.
use crate::agents::AGENT_PROTOCOL;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

const SERVER_PROTOCOL: &str = "2025-06-18";
/// Versions whose tool calls this server answers identically.
const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// Methods that change nothing, so a request lost with its connection can be sent again.
const READ_ONLY: &[&str] = &[
    "projects_list",
    "workspace_list",
    "workspace_get",
    "conversation_read",
];

/// Where a call failed: before the request left (safe to send again) or after (its outcome is
/// unknown).
enum Failure {
    Before(String),
    After(String),
}

struct Tool {
    name: &'static str,
    method: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "list_projects",
        method: "projects_list",
        description: "List the Git repositories registered on this machine. Workspaces can only be created in these projects.",
        schema: || object(json!({}), &[]),
    },
    Tool {
        name: "add_project",
        method: "project_add",
        description: "Register an existing Git repository on this machine as a project. It also appears in the person's Prometeu app.",
        schema: || object(json!({"path": {"type": "string", "description": "Absolute path of the repository on this machine."}}), &["path"]),
    },
    Tool {
        name: "clone_project",
        method: "project_clone",
        description: "Clone a Git repository onto this machine and register it as a project. Defaults to ~/code/<name>.",
        schema: || object(json!({
            "url": {"type": "string", "description": "Repository URL to clone."},
            "path": {"type": "string", "description": "Optional absolute destination path."},
        }), &["url"]),
    },
    Tool {
        name: "create_workspace",
        method: "workspace_create",
        description: "Create a workspace: a new Git worktree and branch in a project, with a Claude Code conversation that starts working on the prompt right away, without permission prompts. Returns the workspace and its conversation IDs. The person sees it in their Prometeu app.",
        schema: || object(json!({
            "project": {"type": "string", "description": "Project path or name from list_projects."},
            "prompt": {"type": "string", "description": "The task for Claude, as you would type it."},
            "title": {"type": "string", "description": "Optional short workspace title."},
            "branch": {"type": "string", "description": "Optional branch name; an existing local branch is checked out. Defaults to a fresh prometeu/<words> branch."},
            "base": {"type": "string", "description": "Optional base ref for a new branch. Defaults to origin's default branch."},
        }), &["project", "prompt"]),
    },
    Tool {
        name: "list_workspaces",
        method: "workspace_list",
        description: "List workspaces on this machine, including those the person created, with each conversation's state: starting, working, idle (waiting for a message), permission or ended.",
        schema: || object(json!({"all": {"type": "boolean", "description": "Include closed workspaces."}}), &[]),
    },
    Tool {
        name: "get_workspace",
        method: "workspace_get",
        description: "Get one workspace with its conversations and their states.",
        schema: || object(json!({"id": {"type": "string"}}), &["id"]),
    },
    Tool {
        name: "open_conversation",
        method: "conversation_open",
        description: "Start another Claude Code conversation in an existing workspace.",
        schema: || object(json!({
            "workspace": {"type": "string", "description": "Workspace ID."},
            "prompt": {"type": "string", "description": "The first message."},
        }), &["workspace", "prompt"]),
    },
    Tool {
        name: "read_conversation",
        method: "conversation_read",
        description: "Read a conversation's messages and tool calls. Without a cursor, returns the latest messages; pass the returned cursor to get only what came after. Also returns the conversation's state.",
        schema: || object(json!({
            "id": {"type": "string", "description": "Conversation ID."},
            "cursor": {"type": "integer", "description": "Cursor from a previous read."},
            "limit": {"type": "integer", "description": "Maximum messages, 1 to 200. Default 50."},
        }), &["id"]),
    },
    Tool {
        name: "send_message",
        method: "conversation_send",
        description: "Send a message to a conversation, as the person typing it. Claude queues it if it is working. An ended conversation is resumed with this message.",
        schema: || object(json!({
            "id": {"type": "string", "description": "Conversation ID."},
            "text": {"type": "string"},
        }), &["id", "text"]),
    },
    Tool {
        name: "interrupt_conversation",
        method: "conversation_interrupt",
        description: "Interrupt Claude's current turn in a conversation, as pressing Escape.",
        schema: || object(json!({"id": {"type": "string", "description": "Conversation ID."}}), &["id"]),
    },
    Tool {
        name: "close_workspace",
        method: "workspace_close",
        description: "Close a workspace: stop its conversations and delete its worktree and branch. Refuses uncommitted changes unless force is true. Push or merge work you want to keep first.",
        schema: || object(json!({
            "id": {"type": "string", "description": "Workspace ID."},
            "force": {"type": "boolean", "description": "Discard uncommitted changes."},
        }), &["id"]),
    },
];

struct Socket {
    path: PathBuf,
    token: String,
    client: String,
    stream: Option<(UnixStream, BufReader<UnixStream>)>,
    next: u64,
}

impl Socket {
    fn connect(&mut self) -> Result<(), String> {
        let stream = UnixStream::connect(&self.path)
            .map_err(|e| format!("Prometeu host unreachable at {}: {e}", self.path.display()))?;
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let hello = json!({"hello": {"protocol": AGENT_PROTOCOL, "token": self.token, "client": self.client}});
        writeln!(writer, "{hello}").map_err(|e| e.to_string())?;
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let ready: Value = serde_json::from_str(&line).map_err(|_| "no answer from the host")?;
        if ready["event"] != "ready" {
            return Err(format!(
                "the host refused the connection: {}",
                ready["error"].as_str().unwrap_or("unknown")
            ));
        }
        self.stream = Some((writer, reader));
        Ok(())
    }

    fn call_once(
        &mut self,
        method: &str,
        params: &Value,
    ) -> Result<Result<Value, String>, Failure> {
        if self.stream.is_none() {
            self.connect().map_err(Failure::Before)?;
        }
        self.next += 1;
        let id = self.next;
        let (writer, reader) = self
            .stream
            .as_mut()
            .ok_or_else(|| Failure::Before("not connected".into()))?;
        let request = json!({"id": id, "method": method, "params": params});
        writeln!(writer, "{request}").map_err(|e| Failure::Before(e.to_string()))?;
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return Err(Failure::After("the host closed the connection".into())),
                Ok(_) => {}
                Err(e) => return Err(Failure::After(e.to_string())),
            }
            let reply: Value =
                serde_json::from_str(&line).map_err(|e| Failure::After(e.to_string()))?;
            if reply["event"] == "error" {
                return Err(Failure::After(format!(
                    "the host refused the request: {}",
                    reply["error"].as_str().unwrap_or("unknown")
                )));
            }
            if reply["id"] != json!(id) {
                continue;
            }
            return Ok(match reply.get("error") {
                Some(error) => Err(error.as_str().unwrap_or("error").to_string()),
                None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
            });
        }
    }

    /// Call the host. A request that never left is sent again on a new connection (the host
    /// restarted); one lost after it left is repeated only when it changes nothing, because the
    /// host may already have done it.
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let failure = match self.call_once(method, params) {
            Ok(result) => return result,
            Err(failure) => failure,
        };
        self.stream = None;
        if let Failure::After(error) = failure {
            if !READ_ONLY.contains(&method) {
                return Err(format!(
                    "{error}. The outcome is unknown: check with list_workspaces or read_conversation before retrying"
                ));
            }
        }
        match self.call_once(method, params) {
            Ok(result) => result,
            Err(Failure::Before(error) | Failure::After(error)) => {
                self.stream = None;
                Err(error)
            }
        }
    }
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn handle(socket: &mut Socket, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(match method {
        "initialize" => {
            let protocol = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| SUPPORTED.contains(v))
                .unwrap_or(SERVER_PROTOCOL);
            reply(
                &id,
                json!({
                    "protocolVersion": protocol,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "prometeu", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": "Use Prometeu on this machine as a person would: create workspaces in registered projects, follow their Claude Code conversations, answer them and close them when done. Everything you create appears in the person's Prometeu app.",
                }),
            )
        }
        "ping" => reply(&id, json!({})),
        "tools/list" => reply(
            &id,
            json!({"tools": TOOLS.iter().map(|t| json!({
                "name": t.name, "description": t.description, "inputSchema": (t.schema)(),
            })).collect::<Vec<_>>()}),
        ),
        "tools/call" => {
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let arguments = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = match TOOLS.iter().find(|t| t.name == name) {
                Some(tool) => socket.call(tool.method, &arguments),
                None => Err(format!("unknown tool {name}")),
            };
            let (text, error) = match result {
                Ok(value) => (
                    serde_json::to_string_pretty(&value).unwrap_or_default(),
                    false,
                ),
                Err(error) => (error, true),
            };
            reply(
                &id,
                json!({"content": [{"type": "text", "text": text}], "isError": error}),
            )
        }
        _ => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("unknown method {method}")}})
        }
    })
}

pub fn run(path: PathBuf) -> Result<(), String> {
    let token = std::env::var("PROMETEU_AGENT_TOKEN")
        .map_err(|_| "PROMETEU_AGENT_TOKEN is not set".to_string())?;
    let client = std::env::var("PROMETEU_AGENT_NAME").unwrap_or_else(|_| "mcp".into());
    let mut socket = Socket {
        path,
        token: token.trim().to_string(),
        client,
        stream: None,
        next: 0,
    };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            let error = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
            writeln!(stdout, "{error}").map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
            continue;
        };
        if let Some(response) = handle(&mut socket, &message) {
            writeln!(stdout, "{response}").map_err(|e| e.to_string())?;
            stdout.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_maps_to_a_distinct_method_with_an_object_schema() {
        let mut methods = std::collections::HashSet::new();
        for tool in TOOLS {
            assert!(methods.insert(tool.method), "{} repeats", tool.method);
            assert_eq!((tool.schema)()["type"], "object");
        }
    }

    /// A host that accepts connections, answers the hello, and then either drops each request
    /// (the first `drops` connections) or answers it. Returns how many requests it received.
    fn flaky_host(path: PathBuf, drops: usize) -> std::sync::mpsc::Receiver<String> {
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let (seen, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (n, stream) in listener.incoming().enumerate() {
                let stream = stream.unwrap();
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                writeln!(writer, "{}", json!({"event": "ready"})).unwrap();
                line.clear();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                seen.send(request["method"].as_str().unwrap().to_string())
                    .unwrap();
                if n >= drops {
                    writeln!(writer, "{}", json!({"id": request["id"], "result": []})).unwrap();
                }
            }
        });
        requests
    }

    fn socket(path: PathBuf) -> Socket {
        Socket {
            path,
            token: "t".into(),
            client: "test".into(),
            stream: None,
            next: 0,
        }
    }

    #[test]
    fn a_lost_reply_repeats_only_requests_that_change_nothing() {
        let dir = std::env::temp_dir().join(format!("mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("write.sock");
        let requests = flaky_host(path.clone(), 1);
        let error = socket(path)
            .call("conversation_send", &json!({"id": "c", "text": "hi"}))
            .unwrap_err();
        assert!(error.contains("outcome is unknown"), "{error}");
        assert_eq!(requests.recv().unwrap(), "conversation_send");
        assert!(requests
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err());

        let path = dir.join("read.sock");
        let requests = flaky_host(path.clone(), 1);
        assert_eq!(
            socket(path).call("workspace_list", &json!({})).unwrap(),
            json!([])
        );
        assert_eq!(requests.iter().take(2).count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_protocol_versions_get_the_servers_own() {
        let mut socket = socket("/nonexistent".into());
        let init = |version: &str| {
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": version}})
        };
        let old = handle(&mut socket, &init("2024-11-05")).unwrap();
        assert_eq!(old["result"]["protocolVersion"], "2024-11-05");
        let future = handle(&mut socket, &init("2099-01-01")).unwrap();
        assert_eq!(future["result"]["protocolVersion"], SERVER_PROTOCOL);
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut socket = Socket {
            path: "/nonexistent".into(),
            token: String::new(),
            client: String::new(),
            stream: None,
            next: 0,
        };
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(handle(&mut socket, &note).is_none());
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "list_projects"}});
        let response = handle(&mut socket, &call).unwrap();
        assert_eq!(response["result"]["isError"], true);
    }
}
