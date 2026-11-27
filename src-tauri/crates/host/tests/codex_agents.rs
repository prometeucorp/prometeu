//! Remote MCP exercises the real daemon with a native Codex RPC fixture, without model calls.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const BIN: &str = env!("CARGO_BIN_EXE_prometeu-host");

struct Client {
    child: Child,
    input: std::process::ChildStdin,
    frames: mpsc::Receiver<Value>,
    next: u64,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Client {
    fn new(mut command: Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (sender, frames) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str(&line) {
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        Self {
            child,
            input,
            frames,
            next: 0,
        }
    }

    fn until(&self, mut accepts: impl FnMut(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let frame = self
                .frames
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("host response before deadline");
            if accepts(&frame) {
                return frame;
            }
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0", "id":self.next,"method":method,"params":params})
        )
        .unwrap();
        let result = self.until(|frame| frame["id"] == self.next);
        assert!(result.get("error").is_none(), "{result}");
        result["result"].clone()
    }

    fn tool_result(&mut self, name: &str, params: Value) -> Result<Value, String> {
        let result = self.call("tools/call", json!({"name":name, "arguments":params}));
        let text = result["content"][0]["text"]
            .as_str()
            .expect("MCP text result");
        if result["isError"] == true {
            Err(text.into())
        } else {
            Ok(serde_json::from_str(text).unwrap())
        }
    }

    fn tool(&mut self, name: &str, params: Value) -> Value {
        self.tool_result(name, params).unwrap()
    }
}

struct Machine {
    dir: PathBuf,
    root: PathBuf,
    host: Child,
}

impl Drop for Machine {
    fn drop(&mut self) {
        // Fixture children have no descendants. Reap them even after a failed assertion.
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("pid-") {
                    if let Ok(pid) = std::fs::read_to_string(entry.path())
                        .unwrap_or_default()
                        .parse::<i32>()
                    {
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                        }
                    }
                }
            }
        }
        let _ = self.host.kill();
        let _ = self.host.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Machine {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = PathBuf::from("/tmp").join(format!(
            "pc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let root = dir.join("r");
        let repo = dir.join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "initial",
            ],
        ] {
            std::fs::write(repo.join("README"), "Fixture\n").unwrap();
            assert!(Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success());
        }
        let fake = dir.join("codex");
        let executable = std::env::current_exe().unwrap();
        std::fs::write(
            &fake,
            format!(
                r#"#!/bin/sh
case "$1" in
  --version) echo 'codex-cli 0.172.0'; exit 0 ;;
  login) exit 0 ;;
esac
PROMETEU_TEST_ARGS="{dir}/args-$$"
printf '%s\n' "$@" > "$PROMETEU_TEST_ARGS"
export PROMETEU_TEST_ARGS
exec '{executable}' --ignored --exact fake_codex --nocapture
"#,
                dir = dir.display(),
                executable = executable.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        let setup = Command::new(BIN)
            .args(["agents", "setup", "--json", "--root"])
            .arg(&root)
            .env("HOME", dir.join("home"))
            .output()
            .unwrap();
        assert!(
            setup.status.success(),
            "{}",
            String::from_utf8_lossy(&setup.stderr)
        );
        let host = Command::new(BIN)
            .args(["serve", "--root"])
            .arg(&root)
            .env("HOME", dir.join("home"))
            .env("PROMETEU_TEST_DIR", &dir)
            .env("PROMETEU_HOST_CODEX", &fake)
            .spawn()
            .unwrap();
        let machine = Self { dir, root, host };
        wait("agent socket", || {
            machine.dir.join("r-agents/agents-v1.sock").exists()
        });
        machine
    }

    fn mcp(&self) -> Client {
        let token = std::fs::read_to_string(self.root.join("agents/token")).unwrap();
        let mut command = Command::new(BIN);
        command
            .args(["mcp", "--socket"])
            .arg(self.dir.join("r-agents/agents-v1.sock"))
            .env("PROMETEU_AGENT_TOKEN", token)
            .env("PROMETEU_AGENT_NAME", "codex-test");
        let mut client = Client::new(command);
        client.call("initialize", json!({"protocolVersion":"2025-06-18"}));
        client
    }

    fn desktop(&self) -> Client {
        let mut command = Command::new(BIN);
        command.args(["attach", "--root"]).arg(&self.root);
        let client = Client::new(command);
        let ready = client.until(|frame| frame["event"] == "ready");
        assert_eq!(ready["codex_agents"], true);
        client
    }
}

fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn message_texts(value: &Value) -> Vec<String> {
    value["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["kind"] == "text")
        .map(|m| m["text"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn legacy_registry_conversations_stay_claude() {
    let machine = Machine::new();
    let legacy = json!({"version":2,"projects":[],"workspaces":[{
        "id":"legacy-workspace","project":machine.dir.join("repo"),"title":"Existing task",
        "branch":"main","base":"HEAD","worktree":machine.dir.join("repo"),
        "created_at":1,"created_by":{"kind":"desktop"},
        "conversations":[{"id":"legacy-conversation","title":"Existing conversation",
            "created_at":1,"permission":"auto"}]
    }]});
    std::fs::write(machine.root.join("registry.json"), legacy.to_string()).unwrap();
    let mut mcp = machine.mcp();
    let workspace = mcp.tool("get_workspace", json!({"id":"legacy-workspace"}));
    assert_eq!(workspace["conversations"][0]["provider"], "claude");
    assert!(workspace["conversations"][0]["thread_id"].is_null());
    assert_eq!(workspace["conversations"][0]["state"], "ended");
    assert_eq!(
        std::fs::read_to_string(machine.root.join("registry.json")).unwrap(),
        legacy.to_string()
    );
}

#[test]
fn codex_mcp_survives_reconnect_and_resumes_the_same_native_thread() {
    let machine = Machine::new();
    let mut mcp = machine.mcp();
    let tools = mcp.call("tools/list", json!({}));
    let create = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "create_workspace")
        .unwrap();
    assert_eq!(
        create["inputSchema"]["properties"]["provider"]["enum"],
        json!(["claude", "codex"])
    );
    assert_eq!(
        create["inputSchema"]["properties"]["provider"]["default"],
        "claude"
    );
    let open = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "open_conversation")
        .unwrap();
    assert_eq!(
        open["inputSchema"]["properties"]["provider"]["default"],
        "claude"
    );
    mcp.tool("add_project", json!({"path":machine.dir.join("repo")}));
    let invalid = mcp
        .tool_result(
            "create_workspace",
            json!({"project":"repo", "prompt":"Task", "provider":"cursor"}),
        )
        .unwrap_err();
    assert!(invalid.contains("invalid_provider"), "{invalid}");
    assert!(!machine.root.join("worktrees").exists());
    let workspace = mcp.tool(
        "create_workspace",
        json!({"project":"repo", "prompt":"First task", "provider":"codex"}),
    );
    let id = workspace["id"].as_str().unwrap();
    let conversation = workspace["conversations"][0]["id"].as_str().unwrap();
    let native = workspace["conversations"][0]["thread_id"].as_str().unwrap();
    assert_eq!(workspace["conversations"][0]["provider"], "codex");
    assert_ne!(native, conversation);
    assert_eq!(workspace["conversations"][0]["model"], "fixture-model");
    wait("first reply", || {
        let read = mcp.tool("read_conversation", json!({"id":conversation}));
        message_texts(&read).contains(&"Reply: First task".to_string())
            && read["status"]["state"] == "idle"
    });
    let initial = mcp.tool("read_conversation", json!({"id":conversation,"cursor":0}));
    assert_eq!(message_texts(&initial), ["First task", "Reply: First task"]);
    let cursor = initial["cursor"].clone();

    // A new MCP connection sees the same daemon conversation, without a desktop.
    drop(mcp);
    let mut mcp = machine.mcp();
    assert_eq!(
        mcp.tool("get_workspace", json!({"id":id}))["conversations"][0]["thread_id"],
        native
    );
    mcp.tool(
        "send_message",
        json!({"id":conversation,"text":"Second task"}),
    );
    wait("second reply", || {
        message_texts(&mcp.tool(
            "read_conversation",
            json!({"id":conversation,"cursor":cursor}),
        ))
        .contains(&"Reply: Second task".into())
    });
    let next = mcp.tool(
        "read_conversation",
        json!({"id":conversation,"cursor":cursor}),
    );
    assert_eq!(message_texts(&next), ["Second task", "Reply: Second task"]);

    // Drop the observer's WebSocket. Replayed native items must not appear twice.
    std::fs::write(machine.dir.join(format!("disconnect-{conversation}")), "").unwrap();
    wait("observer reconnect", || {
        std::fs::read_to_string(machine.dir.join(format!("connections-{conversation}")))
            .unwrap_or_default()
            .trim()
            .parse::<u32>()
            .unwrap_or(0)
            >= 2
    });
    wait("recovered status", || {
        mcp.tool("read_conversation", json!({"id":conversation}))["status"]["state"] == "idle"
    });
    assert_eq!(
        message_texts(&mcp.tool("read_conversation", json!({"id":conversation,"cursor":0}))).len(),
        4
    );

    mcp.tool("send_message", json!({"id":conversation,"text":"wait"}));
    wait("working", || {
        mcp.tool("read_conversation", json!({"id":conversation}))["status"]["state"] == "working"
    });
    mcp.tool("interrupt_conversation", json!({"id":conversation}));
    wait("interrupted", || {
        machine
            .dir
            .join(format!("interrupted-{conversation}"))
            .exists()
    });
    wait("idle after interrupt", || {
        mcp.tool("read_conversation", json!({"id":conversation}))["status"]["state"] == "idle"
    });

    let mut desktop = machine.desktop();
    let legacy = desktop.call("registry_list", json!({}));
    assert_eq!(legacy["workspaces"][0]["conversations"], json!([]));
    let modern = desktop.call("registry_list", json!({"codex_agents":true}));
    assert_eq!(
        modern["workspaces"][0]["conversations"][0]["provider"],
        "codex"
    );
    assert_eq!(
        modern["workspaces"][0]["conversations"][0]["thread_id"],
        native
    );
    mcp.tool(
        "send_message",
        json!({"id":conversation,"text":"exit-with-tail"}),
    );
    wait("TUI exit", || {
        mcp.tool("read_conversation", json!({"id":conversation}))["status"]["state"] == "ended"
    });
    wait("engine stopped", || {
        let terminals = desktop.call("terminal_list", json!({}));
        terminals
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["id"] != format!("{conversation}-codex") || t["running"] == false)
    });
    // The final assistant item was stored by the native engine but never broadcast.
    assert!(
        message_texts(&mcp.tool("read_conversation", json!({"id":conversation})))
            .contains(&"Reply: exit-with-tail".into())
    );
    let mut mismatched = modern["workspaces"][0].clone();
    mismatched["conversations"][0]["model"] = json!("requested-model");
    desktop.call("registry_put", json!({"workspace":mismatched}));
    let mismatch = mcp
        .tool_result(
            "send_message",
            json!({"id":conversation,"text":"Do not run with a different model"}),
        )
        .unwrap_err();
    assert!(mismatch.contains("codex_model_mismatch"), "{mismatch}");
    assert_eq!(
        mcp.tool("get_workspace", json!({"id":id}))["conversations"][0]["thread_id"],
        native
    );
    desktop.call("registry_put", json!({"workspace":modern["workspaces"][0]}));
    assert_eq!(
        mcp.tool(
            "send_message",
            json!({"id":conversation,"text":"After restart"})
        )["resumed"],
        true
    );
    wait("resumed reply", || {
        message_texts(&mcp.tool("read_conversation", json!({"id":conversation})))
            .contains(&"Reply: After restart".into())
    });
    let resumed = mcp.tool("get_workspace", json!({"id":id}));
    assert_eq!(resumed["conversations"][0]["thread_id"], native);
    let all = message_texts(&mcp.tool("read_conversation", json!({"id":conversation,"cursor":0})));
    assert_eq!(all.iter().filter(|m| *m == "First task").count(), 1);

    // Sending as soon as the TUI exits must wait for its old observer to drain and retire.
    std::fs::write(machine.dir.join(format!("slow-history-{conversation}")), "").unwrap();
    desktop.call(
        "terminal_signal",
        json!({"id":conversation,"signal":"kill"}),
    );
    wait("TUI stopped before engine", || {
        let terminals = desktop.call("terminal_list", json!({}));
        terminals
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == conversation && t["running"] == false)
    });
    let terminals = desktop.call("terminal_list", json!({}));
    assert!(terminals
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == format!("{conversation}-codex") && t["running"] == true));
    assert_eq!(
        mcp.tool(
            "send_message",
            json!({"id":conversation,"text":"Immediate restart"})
        )["resumed"],
        true
    );
    wait("immediate resumed reply", || {
        message_texts(&mcp.tool("read_conversation", json!({"id":conversation})))
            .contains(&"Reply: Immediate restart".into())
    });
    assert_eq!(
        mcp.tool("get_workspace", json!({"id":id}))["conversations"][0]["thread_id"],
        native
    );

    let another = mcp.tool(
        "open_conversation",
        json!({"workspace":id,"prompt":"Another task","provider":"codex"}),
    );
    assert_eq!(another["provider"], "codex");
    assert_ne!(another["thread_id"], native);
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(machine.root.join("registry.json")).unwrap())
            .unwrap();
    assert_eq!(persisted["version"], 3);
    assert_eq!(
        persisted["workspaces"][0]["conversations"][0]["thread_id"],
        native
    );
    let another_id = another["id"].as_str().unwrap();
    wait("another reply", || {
        message_texts(&mcp.tool("read_conversation", json!({"id":another_id})))
            .contains(&"Reply: Another task".into())
    });
    std::fs::write(machine.dir.join(format!("slow-history-{another_id}")), "").unwrap();
    mcp.tool(
        "send_message",
        json!({"id":another_id,"text":"exit-with-tail"}),
    );
    wait("second TUI exits before close", || {
        mcp.tool("read_conversation", json!({"id":another_id}))["status"]["state"] == "ended"
    });
    mcp.tool("close_workspace", json!({"id":id}));
    assert!(
        message_texts(&mcp.tool("read_conversation", json!({"id":another_id})))
            .contains(&"Reply: exit-with-tail".into())
    );
    assert!(!Path::new(workspace["worktree"].as_str().unwrap()).exists());
    assert_eq!(mcp.tool("get_workspace", json!({"id":id}))["closed"], true);
    let terminals = desktop.call("terminal_list", json!({}));
    assert!(terminals.as_array().unwrap().is_empty(), "{terminals}");
}

fn send(socket: &mut WebSocket<UnixStream>, value: Value) -> bool {
    socket.send(Message::Text(value.to_string().into())).is_ok()
}

fn emit(socket: &mut WebSocket<UnixStream>, method: &str, params: Value) -> bool {
    send(socket, json!({"method":method,"params":params}))
}

#[test]
#[ignore = "subprocess Codex fixture, invoked by the integration test"]
fn fake_codex() {
    let dir = PathBuf::from(std::env::var("PROMETEU_TEST_DIR").unwrap());
    let args_file = std::env::var("PROMETEU_TEST_ARGS").unwrap();
    let args: Vec<String> = std::fs::read_to_string(&args_file)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    std::fs::remove_file(args_file).unwrap();
    let socket = if args[0] == "app-server" {
        &args[2]
    } else {
        &args[1]
    };
    let path = PathBuf::from(socket.strip_prefix("unix://").unwrap());
    let conversation = path
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    let pid = std::process::id();
    std::fs::write(dir.join(format!("pid-{pid}")), pid.to_string()).unwrap();
    if args[0] == "app-server" {
        fake_engine(&dir, &path, conversation);
    } else {
        fake_tui(&dir, conversation, args.get(5).map(String::as_str));
    }
}

fn input(dir: &Path, conversation: &str, event: Value) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("input-{conversation}")))
        .unwrap();
    writeln!(file, "{event}").unwrap();
    file.sync_data().unwrap();
}

fn fake_tui(dir: &Path, conversation: &str, prompt: Option<&str>) {
    unsafe {
        let mut attributes: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(0, &mut attributes), 0);
        libc::cfmakeraw(&mut attributes);
        attributes.c_cc[libc::VMIN] = 0;
        attributes.c_cc[libc::VTIME] = 1;
        assert_eq!(libc::tcsetattr(0, libc::TCSANOW, &attributes), 0);
    }
    if let Some(prompt) = prompt {
        input(dir, conversation, json!({"text":prompt}));
    }
    let mut pending = Vec::new();
    loop {
        let mut buffer = [0u8; 4096];
        let count = std::io::stdin().read(&mut buffer).unwrap();
        pending.extend_from_slice(&buffer[..count]);
        if count == 0 && pending == b"\x1b" {
            input(dir, conversation, json!({"interrupt":true}));
            pending.clear();
        }
        while let Some(end) = pending.iter().position(|b| *b == b'\r') {
            let text = String::from_utf8(pending.drain(..=end).collect())
                .unwrap()
                .replace("\x1b[200~", "")
                .replace("\x1b[201~", "")
                .trim_end_matches('\r')
                .to_string();
            input(dir, conversation, json!({"text":text}));
            if text == "exit-with-tail" {
                wait("fixture tail stored", || {
                    dir.join(format!("tail-ready-{conversation}")).exists()
                });
                return;
            }
        }
    }
}

fn fake_engine(dir: &Path, path: &Path, conversation: &str) {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let history = dir.join(format!("history-{conversation}"));
    let mut state: Value = std::fs::read(&history).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(||
        json!({"thread":format!("00000000-0000-4000-8000-{:012}", std::process::id()),"turns":[],"processed":0}));
    let cwd = std::env::current_dir().unwrap().display().to_string();
    let mut settings = json!({"model":"fixture-model","effort":"medium",
        "collaborationMode":{"mode":"default","settings":{"model":"fixture-model","reasoning_effort":"medium","developer_instructions":null}}});
    let mut connections = 0;
    for stream in listener.incoming() {
        let stream = stream.unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let Ok(mut socket) = tungstenite::accept(stream) else {
            continue;
        };
        connections += 1;
        std::fs::write(
            dir.join(format!("connections-{conversation}")),
            connections.to_string(),
        )
        .unwrap();
        loop {
            if dir.join(format!("disconnect-{conversation}")).exists() {
                std::fs::remove_file(dir.join(format!("disconnect-{conversation}"))).unwrap();
                let _ = socket.close(None);
                break;
            }
            match socket.read() {
                Ok(Message::Text(text)) => {
                    let request: Value = serde_json::from_str(&text).unwrap();
                    if request.get("id").is_none() {
                        continue;
                    }
                    let result = match request["method"].as_str().unwrap() {
                        "initialize" | "thread/name/set" => json!({}),
                        "thread/settings/update" => {
                            settings = request["params"].clone();
                            json!({})
                        }
                        "thread/start" | "thread/resume" => {
                            if let Some(requested) = request["params"]["threadId"].as_str() {
                                assert_eq!(requested, state["thread"]);
                            }
                            let status = if state["turns"]
                                .as_array()
                                .unwrap()
                                .last()
                                .is_some_and(|t| t["status"] == "inProgress")
                            {
                                "active"
                            } else {
                                "idle"
                            };
                            json!({"thread":{"id":state["thread"],"status":{"type":status,"activeFlags":[]}},
                                "cwd":cwd,"model":"fixture-model","reasoningEffort":settings["effort"],
                                "approvalPolicy":"never","sandbox":{"type":"dangerFullAccess"},"collaborationMode":settings["collaborationMode"]})
                        }
                        "thread/turns/list" => {
                            let slow = dir.join(format!("slow-history-{conversation}"));
                            if slow.exists() {
                                std::fs::remove_file(slow).unwrap();
                                std::thread::sleep(Duration::from_millis(700));
                            }
                            json!({"data":state["turns"],"nextCursor":null})
                        }
                        other => panic!("unexpected native method {other}"),
                    };
                    if !send(&mut socket, json!({"id":request["id"],"result":result})) {
                        break;
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
                _ => {}
            }
            let events = std::fs::read_to_string(dir.join(format!("input-{conversation}")))
                .unwrap_or_default();
            let pending: Vec<Value> = events
                .lines()
                .skip(state["processed"].as_u64().unwrap() as usize)
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            for event in pending {
                state["processed"] = json!(state["processed"].as_u64().unwrap() + 1);
                let thread = state["thread"].clone();
                if event["interrupt"] == true {
                    if let Some(turn) = state["turns"].as_array_mut().unwrap().last_mut() {
                        turn["status"] = json!("interrupted");
                        emit(
                            &mut socket,
                            "turn/completed",
                            json!({"threadId":thread,"turn":turn}),
                        );
                    }
                    std::fs::write(dir.join(format!("interrupted-{conversation}")), "").unwrap();
                } else {
                    let text = event["text"].as_str().unwrap();
                    let turn_id = format!("turn-{}", state["turns"].as_array().unwrap().len());
                    let user = json!({"id":format!("user-{turn_id}"),"type":"userMessage","content":[{"type":"text","text":text}],"completedAtMs":1});
                    let mut turn =
                        json!({"id":turn_id,"status":"inProgress","items":[user.clone()]});
                    emit(
                        &mut socket,
                        "turn/started",
                        json!({"threadId":thread,"turn":turn}),
                    );
                    emit(
                        &mut socket,
                        "item/completed",
                        json!({"threadId":thread,"turnId":turn_id,"item":user}),
                    );
                    if text != "wait" {
                        let assistant = json!({"id":format!("assistant-{turn_id}"),"type":"agentMessage","text":format!("Reply: {text}"),"completedAtMs":2});
                        turn["items"]
                            .as_array_mut()
                            .unwrap()
                            .push(assistant.clone());
                        turn["status"] = json!("completed");
                        if text != "exit-with-tail" {
                            emit(
                                &mut socket,
                                "item/completed",
                                json!({"threadId":thread,"turnId":turn_id,"item":assistant}),
                            );
                            emit(
                                &mut socket,
                                "turn/completed",
                                json!({"threadId":thread,"turn":turn}),
                            );
                        }
                    }
                    state["turns"].as_array_mut().unwrap().push(turn);
                }
                std::fs::write(&history, state.to_string()).unwrap();
                if event["text"] == "exit-with-tail" {
                    std::fs::write(dir.join(format!("tail-ready-{conversation}")), "").unwrap();
                }
            }
        }
    }
}
