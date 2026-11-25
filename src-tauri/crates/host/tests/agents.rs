//! Drives the real host the way a bot platform does (ADR 0103): through the token-protected agent
//! socket, with no desktop attached. A stand-in for Claude records its arguments, writes a native
//! transcript, reports hooks through the host's hook command and logs what is typed into it.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_prometeu-host");

struct Machine {
    dir: PathBuf,
    root: PathBuf,
    home: PathBuf,
    host: Child,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.host.kill();
        let _ = self.host.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn sh(cwd: &Path, script: &str) {
    let status = Command::new("sh")
        .args(["-c", script])
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "{script}");
}

impl Machine {
    fn new() -> Self {
        // Short: Unix socket paths are limited to about 100 bytes.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = PathBuf::from("/tmp").join(format!("pa-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Git and Claude report physical paths (macOS's /tmp is a link to /private/tmp).
        let dir = std::fs::canonicalize(&dir).unwrap();
        let root = dir.join("r");
        let home = dir.join("h");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(dir.join("repo")).unwrap();
        sh(
            &dir.join("repo"),
            "git init -q -b main . && echo a > a && git add a && git commit -qm init",
        );
        let fake = dir.join("claude");
        std::fs::write(
            &fake,
            format!(
                r#"#!/bin/sh
# Arguments: --session-id|--resume ID --dangerously-skip-permissions --settings FILE [PROMPT]
id="$2"; prompt="$6"
printf '%s\n' "$@" > "{dir}/args-$id"
slug=$(pwd | sed 's/[^A-Za-z0-9]/-/g')
mkdir -p "$HOME/.claude/projects/$slug"
log="$HOME/.claude/projects/$slug/$id.jsonl"
printf '{{"type":"user","message":{{"content":"%s"}},"timestamp":"t"}}\n' "$prompt" >> "$log"
printf '{{"type":"assistant","message":{{"content":[{{"type":"text","text":"done"}}]}},"timestamp":"t"}}\n' >> "$log"
printf '{{"hook_event_name":"Stop"}}' | '{bin}' hook --root '{root}'
while IFS= read -r line; do printf '%s\n' "$line" >> "{dir}/typed-$id"; done
"#,
                dir = dir.display(),
                bin = BIN,
                root = root.display()
            ),
        )
        .unwrap();
        sh(&dir, "chmod +x claude");
        let setup = Command::new(BIN)
            .args(["agents", "setup", "--root"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(setup.status.success());
        let host = Command::new(BIN)
            .args(["serve", "--root"])
            .arg(&root)
            .env("HOME", &home)
            .env("PROMETEU_HOST_CLAUDE", &fake)
            .env("CLAUDECODE", "1")
            .spawn()
            .unwrap();
        Self {
            dir,
            root,
            home,
            host,
        }
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.root.join("agents/token")).unwrap()
    }

    fn socket(&self) -> PathBuf {
        self.root.join("agents/agents-v1.sock")
    }

    fn connect(&self, token: &str) -> (UnixStream, BufReader<UnixStream>, Value) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match UnixStream::connect(self.socket()) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Err(error) => panic!("agent socket: {error}"),
            }
        };
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        writeln!(
            writer,
            "{}",
            json!({"hello": {"protocol": 1, "token": token, "client": "test-bot"}})
        )
        .unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        (writer, reader, serde_json::from_str(&line).unwrap())
    }

    fn wait_for(&self, what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

struct Agent {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next: u64,
}

impl Agent {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next += 1;
        writeln!(
            self.writer,
            "{}",
            json!({"id": self.next, "method": method, "params": params})
        )
        .unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        if reply["event"] == "error" {
            return Err(format!("event {}", reply["error"].as_str().unwrap()));
        }
        assert_eq!(reply["id"], self.next);
        match reply.get("error") {
            Some(error) => Err(error.as_str().unwrap().to_string()),
            None => Ok(reply["result"].clone()),
        }
    }
}

#[test]
fn an_agent_drives_a_workspace_without_a_desktop() {
    let machine = Machine::new();
    let (_, _, refused) = machine.connect("wrong");
    assert_eq!(refused["error"], "unauthorized");
    let (writer, reader, ready) = machine.connect(machine.token().trim());
    assert_eq!(ready["event"], "ready");
    let mut agent = Agent {
        writer,
        reader,
        next: 0,
    };

    let repo = machine.dir.join("repo").display().to_string();
    assert!(agent
        .call("workspace_create", json!({"project": repo, "prompt": "x"}))
        .unwrap_err()
        .starts_with("unknown_project"));
    let project = agent.call("project_add", json!({"path": repo})).unwrap();
    assert_eq!(project["name"], "repo");
    assert_eq!(project["added_by"]["name"], "test-bot");

    let created = agent
        .call(
            "workspace_create",
            json!({"project": "repo", "prompt": "fix the bug"}),
        )
        .unwrap();
    let workspace = created["id"].as_str().unwrap().to_string();
    let conversation = created["conversations"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let branch = created["branch"].as_str().unwrap().to_string();
    let worktree = created["worktree"].as_str().unwrap().to_string();
    assert!(branch.starts_with("prometeu/"), "{branch}");
    assert!(worktree.starts_with(&machine.root.join("worktrees/repo/").display().to_string()));
    assert_eq!(created["base"], "HEAD");
    assert_eq!(created["created_by"]["kind"], "agent");
    assert!(Path::new(&worktree).join("a").exists());

    // Claude would otherwise stop at its folder-trust question.
    let claude: Value =
        serde_json::from_slice(&std::fs::read(machine.home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        claude["projects"][&worktree]["hasTrustDialogAccepted"],
        true
    );

    let args = machine.dir.join(format!("args-{conversation}"));
    machine.wait_for("launch", || args.exists());
    let args = std::fs::read_to_string(&args).unwrap();
    let args: Vec<&str> = args.lines().collect();
    assert_eq!(args[0], "--session-id");
    assert_eq!(args[1], conversation);
    assert_eq!(args[2], "--dangerously-skip-permissions");
    assert_eq!(args[5], "fix the bug");
    let settings: Value = serde_json::from_slice(&std::fs::read(args[4]).unwrap()).unwrap();
    assert!(settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains(" hook --root "));

    // The Stop hook reached the host with no desktop attached.
    machine.wait_for("idle state", || {
        let listed = agent.call("workspace_list", json!({})).unwrap();
        listed[0]["conversations"][0]["state"] == "idle"
    });

    let read = agent
        .call("conversation_read", json!({"id": conversation}))
        .unwrap();
    let texts: Vec<&str> = read["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["fix the bug", "done"]);
    let cursor = read["cursor"].as_u64().unwrap();
    let after = agent
        .call(
            "conversation_read",
            json!({"id": conversation, "cursor": cursor}),
        )
        .unwrap();
    assert_eq!(after["messages"], json!([]));
    let first = agent
        .call(
            "conversation_read",
            json!({"id": conversation, "cursor": 0, "limit": 1}),
        )
        .unwrap();
    assert_eq!(first["messages"].as_array().unwrap().len(), 1);
    assert_eq!(first["more"], true);

    let sent = agent
        .call(
            "conversation_send",
            json!({"id": conversation, "text": "also add a test"}),
        )
        .unwrap();
    assert_eq!(sent["resumed"], false);
    let typed = machine.dir.join(format!("typed-{conversation}"));
    machine.wait_for("typed message", || {
        std::fs::read_to_string(&typed).is_ok_and(|t| t.contains("also add a test"))
    });
    assert!(std::fs::read_to_string(&typed)
        .unwrap()
        .contains("\u{1b}[200~also add a test\u{1b}[201~"));

    // Commits only this branch holds would be lost with it.
    sh(
        Path::new(&worktree),
        "echo c > c && git add c && git commit -qm work",
    );
    assert!(agent
        .call("workspace_close", json!({"id": workspace}))
        .unwrap_err()
        .starts_with("unpushed_commits"));
    // Detached at the base, HEAD holds nothing new, but deleting the branch still would lose it.
    sh(Path::new(&worktree), "git checkout -q --detach HEAD~1");
    assert!(agent
        .call("workspace_close", json!({"id": workspace}))
        .unwrap_err()
        .starts_with("unpushed_commits"));
    std::fs::write(Path::new(&worktree).join("b"), "dirty").unwrap();
    assert!(agent
        .call("workspace_close", json!({"id": workspace}))
        .unwrap_err()
        .starts_with("uncommitted_changes"));
    agent
        .call("workspace_close", json!({"id": workspace, "force": true}))
        .unwrap();
    assert!(!Path::new(&worktree).exists());
    let branches = Command::new("git")
        .args(["-C", &repo, "branch", "--list", &branch])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&branches.stdout).trim().is_empty());
    assert_eq!(agent.call("workspace_list", json!({})).unwrap(), json!([]));
    let all = agent.call("workspace_list", json!({"all": true})).unwrap();
    assert_eq!(all[0]["closed"], true);
    assert_eq!(all[0]["conversations"][0]["state"], "ended");

    // The desktop socket offers the same registry, for adoption into the board.
    let mut desktop = UnixStream::connect(machine.root.join("host-v1.sock")).unwrap();
    writeln!(desktop, "{}", json!({"attach": {"protocol": 1}})).unwrap();
    writeln!(
        desktop,
        "{}",
        json!({"id": 1, "method": "registry_list", "params": {}})
    )
    .unwrap();
    let mut lines = BufReader::new(desktop).lines();
    let reply = loop {
        let frame: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if frame["id"] == 1 {
            break frame;
        }
    };
    assert_eq!(reply["result"]["projects"][0]["path"], repo);
    assert_eq!(reply["result"]["workspaces"][0]["id"], workspace);
}

#[test]
fn rotating_the_token_and_moving_the_socket_cut_off_old_access() {
    let machine = Machine::new();
    let (writer, reader, ready) = machine.connect(machine.token().trim());
    assert_eq!(ready["event"], "ready");
    let mut agent = Agent {
        writer,
        reader,
        next: 0,
    };
    agent.call("projects_list", json!({})).unwrap();
    let rotated = Command::new(BIN)
        .args(["agents", "setup", "--rotate", "--root"])
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(rotated.status.success());
    assert_eq!(
        agent.call("projects_list", json!({})).unwrap_err(),
        "event unauthorized"
    );

    let old = machine.socket();
    let moved = machine.dir.join("moved.sock");
    let setup = Command::new(BIN)
        .args(["agents", "setup", "--socket"])
        .arg(&moved)
        .arg("--root")
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(setup.status.success());
    machine.wait_for("the socket to move", || moved.exists() && !old.exists());
    assert!(UnixStream::connect(&old).is_err());
}

#[test]
fn agents_reach_only_registered_conversations_and_never_resume_prompting_ones() {
    let machine = Machine::new();
    let (writer, reader, _) = machine.connect(machine.token().trim());
    let mut agent = Agent {
        writer,
        reader,
        next: 0,
    };
    assert_eq!(
        agent
            .call("conversation_interrupt", json!({"id": "desktop-terminal"}))
            .unwrap_err(),
        "unknown_conversation"
    );

    // A conversation the person runs with permission prompts, recorded by the desktop.
    let repo = machine.dir.join("repo").display().to_string();
    let mut desktop = UnixStream::connect(machine.root.join("host-v1.sock")).unwrap();
    writeln!(desktop, "{}", json!({"attach": {"protocol": 1}})).unwrap();
    let workspace = json!({
        "id": "w1", "project": repo, "title": "t", "branch": "main", "base": "HEAD",
        "worktree": repo, "created_at": 0, "created_by": {"kind": "desktop"},
        "conversations": [{"id": "c1", "title": "t", "created_at": 0, "permission": "ask"}],
    });
    writeln!(
        desktop,
        "{}",
        json!({"id": 1, "method": "registry_put", "params": {"workspace": workspace}})
    )
    .unwrap();
    let mut lines = BufReader::new(desktop).lines();
    loop {
        let frame: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if frame["id"] == 1 {
            assert_eq!(frame["result"], json!({}));
            break;
        }
    }
    assert!(agent
        .call("conversation_send", json!({"id": "c1", "text": "go"}))
        .unwrap_err()
        .contains("resume it from the desktop"));
    assert!(!machine.dir.join("args-c1").exists());
}

#[test]
fn a_refused_socket_path_is_taken_once_it_is_free() {
    let machine = Machine::new();
    machine.wait_for("agent socket", || machine.socket().exists());
    let blocked = machine.dir.join("blocked.sock");
    std::fs::write(&blocked, "not a socket").unwrap();
    let setup = Command::new(BIN)
        .args(["agents", "setup", "--socket"])
        .arg(&blocked)
        .arg("--root")
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(setup.status.success());
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "not a socket");
    std::fs::remove_file(&blocked).unwrap();
    machine.wait_for("the freed path", || UnixStream::connect(&blocked).is_ok());
}

#[test]
fn the_mcp_server_lists_tools_and_relays_calls() {
    let machine = Machine::new();
    let mut mcp = Command::new(BIN)
        .args(["mcp", "--root"])
        .arg(&machine.root)
        .env("PROMETEU_AGENT_TOKEN", machine.token())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    let mut stdout = BufReader::new(mcp.stdout.take().unwrap());
    let mut ask = |message: Value| -> Value {
        writeln!(stdin, "{message}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    };
    let init = ask(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "prometeu");
    let tools = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert!(tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "create_workspace"));
    machine.wait_for("agent socket", || machine.socket().exists());
    let repo = machine.dir.join("repo").display().to_string();
    let added = ask(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "add_project", "arguments": {"path": repo}}}));
    assert_eq!(added["result"]["isError"], false);
    let listed = ask(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "list_projects", "arguments": {}}}));
    let text = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap()[0]["path"],
        repo
    );
    drop(stdin);
    let _ = mcp.wait();
}
