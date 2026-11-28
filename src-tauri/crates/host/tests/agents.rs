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
        Self::with_relative_root(false)
    }

    fn with_relative_root(relative: bool) -> Self {
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
            .args(["agents", "setup", "--json", "--root"])
            .arg(if relative { Path::new("./r") } else { &root })
            .current_dir(&dir)
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(setup.status.success());
        let setup: Value = serde_json::from_slice(&setup.stdout).unwrap();
        // Configuration must be portable before the daemon creates its socket.
        assert!(Path::new(setup["server"]["command"].as_str().unwrap()).is_absolute());
        assert!(Path::new(setup["server"]["args"][2].as_str().unwrap()).is_absolute());
        let host = Command::new(BIN)
            .args(["serve", "--root"])
            .arg(&root)
            .current_dir(&dir)
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
        self.dir.join("r-agents/agents-v1.sock")
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
fn supervision_setup_and_mcp_deliver_real_process_events_without_a_desktop() {
    use std::io::Read;
    use std::process::Stdio;
    let machine = Machine::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (sender, received) = std::sync::mpsc::channel();
    let cloud = std::thread::spawn(move || {
        for index in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut length = 0;
            let mut authorized = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                authorized |= line
                    .trim()
                    .eq_ignore_ascii_case("authorization: Bearer ingest-secret");
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            assert!(authorized);
            let body = if index == 0 {
                assert!(first.starts_with("GET /api/supervision "));
                r#"{"supervisor":{"id":"reviewer","name":"Reviewer"}}"#
            } else {
                assert!(first.starts_with("POST /api/supervision/events "));
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                sender
                    .send(serde_json::from_slice::<Value>(&bytes).unwrap())
                    .unwrap();
                "{}"
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let mut setup = Command::new(BIN)
        .args(["supervision", "setup", "--url", &url, "--root"])
        .arg(&machine.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    setup
        .stdin
        .take()
        .unwrap()
        .write_all(b"ingest-secret\n")
        .unwrap();
    let output = setup.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("ingest-secret"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("ingest-secret"));
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["supervisor"]["id"], "reviewer");
    let (writer, reader, _) = machine.connect(machine.token().trim());
    let mut agent = Agent {
        writer,
        reader,
        next: 0,
    };
    let supervisors = agent.call("supervisors_list", json!({})).unwrap();
    assert_eq!(supervisors[0]["executor_id"], status["executor_id"]);
    agent
        .call("project_add", json!({"path":machine.dir.join("repo")}))
        .unwrap();
    assert!(agent
        .call(
            "workspace_create",
            json!({"project":"repo", "prompt":"do work", "supervisor_id":"unconfigured"})
        )
        .is_err());
    let workspace = agent
        .call(
            "workspace_create",
            json!({"project":"repo", "prompt":"private task", "supervisor_id":"reviewer"}),
        )
        .unwrap();
    let created = received.recv_timeout(Duration::from_secs(10)).unwrap();
    let completed = received.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(created["kind"], "workspace.created");
    assert_eq!(completed["kind"], "execution.completed");
    assert_eq!(completed["workspace_id"], workspace["id"]);
    assert_eq!(
        completed["conversation_id"],
        workspace["conversations"][0]["id"]
    );
    assert!(!completed.to_string().contains("private task"));
    agent
        .call(
            "workspace_pr_link",
            json!({"id":workspace["id"], "repository":"owner/repo", "number":12}),
        )
        .unwrap();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(10)).unwrap()["kind"],
        "workspace.pr_linked"
    );
    agent
        .call("workspace_supervisor_remove", json!({"id":workspace["id"]}))
        .unwrap();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(10)).unwrap()["kind"],
        "workspace.unsupervised"
    );
    assert!(agent
        .call("workspace_get", json!({"id":workspace["id"]}))
        .unwrap()["supervision"]
        .is_null());
    cloud.join().unwrap();
    let reset = Command::new(BIN)
        .args(["supervision", "reset", "--root"])
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(!reset.status.success());
    assert!(String::from_utf8_lossy(&reset.stderr).contains("--discard-pending"));
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
    std::fs::remove_file(Path::new(&worktree).join("b")).unwrap();
    // Renamed away (as after merging it elsewhere), the branch is no longer the workspace's to
    // delete, and HEAD, at the base, holds nothing new: closing needs no force.
    sh(
        Path::new(&worktree),
        &format!("git branch -m {branch} renamed-after-merge"),
    );
    agent
        .call("workspace_close", json!({"id": workspace}))
        .unwrap();
    let kept = Command::new("git")
        .args(["-C", &repo, "branch", "--list", "renamed-after-merge"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&kept.stdout).trim().is_empty());
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

    // Force discards what the checks protect: uncommitted changes and unpushed commits.
    let forced = agent
        .call(
            "workspace_create",
            json!({"project": "repo", "prompt": "throwaway"}),
        )
        .unwrap();
    let forced_tree = forced["worktree"].as_str().unwrap().to_string();
    let forced_branch = forced["branch"].as_str().unwrap().to_string();
    sh(
        Path::new(&forced_tree),
        "echo d > d && git add d && git commit -qm unpushed && echo e > e",
    );
    agent
        .call(
            "workspace_close",
            json!({"id": forced["id"], "force": true}),
        )
        .unwrap();
    assert!(!Path::new(&forced_tree).exists());
    let gone = Command::new("git")
        .args(["-C", &repo, "branch", "--list", &forced_branch])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&gone.stdout).trim().is_empty());

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
    let mut lines = BufReader::new(desktop.try_clone().unwrap()).lines();
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

    // An agent's change reaches the attached desktop at once, so it adopts without reattaching.
    agent.call("project_add", json!({"path": repo})).unwrap();
    loop {
        let frame: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if frame["event"] == "registry" {
            break;
        }
    }

    // The desktop's later record of the workspace keeps what it did not know about.
    let renamed = json!({
        "id": "w1", "project": repo, "title": "renamed", "branch": "main", "base": "HEAD",
        "worktree": repo, "created_at": 0, "created_by": {"kind": "desktop"},
        "conversations": [{"id": "c2", "title": "second", "created_at": 1, "permission": "auto"}],
    });
    writeln!(
        desktop,
        "{}",
        json!({"id": 2, "method": "registry_put", "params": {"workspace": renamed}})
    )
    .unwrap();
    loop {
        let frame: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if frame["id"] == 2 {
            assert_eq!(frame["result"], json!({}));
            break;
        }
    }
    let listed = agent.call("workspace_get", json!({"id": "w1"})).unwrap();
    assert_eq!(listed["title"], "renamed");
    let ids: Vec<&str> = listed["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["c1", "c2"]);

    writeln!(
        desktop,
        "{}",
        json!({"id": 3, "method": "registry_archive", "params": {
            "id": "w1", "archived": true,
        }})
    )
    .unwrap();
    loop {
        let frame: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
        if frame["id"] == 3 {
            assert_eq!(frame["result"], json!({}));
            break;
        }
    }
    assert_eq!(agent.call("workspace_list", json!({})).unwrap(), json!([]));
    assert_eq!(
        agent.call("workspace_list", json!({"all": true})).unwrap()[0]["archived"],
        true
    );
    assert_eq!(
        agent
            .call("conversation_send", json!({"id": "c1", "text": "go"}))
            .unwrap_err(),
        "workspace_archived"
    );
    assert_eq!(
        agent
            .call(
                "conversation_open",
                json!({"workspace": "w1", "prompt": "go"})
            )
            .unwrap_err(),
        "workspace_archived"
    );
}

#[test]
fn a_refused_socket_path_is_taken_once_it_is_free() {
    let machine = Machine::new();
    machine.wait_for("agent socket", || machine.socket().exists());
    let blocked = machine.dir.join("blocked.sock");
    std::fs::write(&blocked, "not a socket").unwrap();
    // Setup refuses a taken path instead of reporting agents as enabled.
    let setup = Command::new(BIN)
        .args(["agents", "setup", "--socket"])
        .arg(&blocked)
        .arg("--root")
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(!setup.status.success());
    assert!(String::from_utf8_lossy(&setup.stderr).contains("is not a socket"));

    // A path taken after setup: the host leaves the file alone, reports why it cannot serve the
    // socket, and takes the path once it is free.
    let config = json!({"socket": blocked.display().to_string()});
    std::fs::write(machine.root.join("agents.json"), config.to_string()).unwrap();
    let status = || {
        let output = Command::new(BIN)
            .args(["agents", "status", "--json", "--root"])
            .arg(&machine.root)
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    machine.wait_for("the bind error", || status()["error"].is_string());
    assert!(status()["error"].as_str().unwrap().contains("not a socket"));
    assert_eq!(status()["serving"], false);
    assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "not a socket");
    std::fs::remove_file(&blocked).unwrap();
    machine.wait_for("the freed path", || UnixStream::connect(&blocked).is_ok());
    machine.wait_for("the error to clear", || status()["error"].is_null());
    assert_eq!(status()["serving"], true);
}

#[test]
fn setup_rejects_invalid_identity_and_removed_permission_flags() {
    let root = std::env::temp_dir().join(format!("pa-uid-{}", std::process::id()));
    for bad in [
        &["--client-uid", "nobody"][..],
        &["--client-uid"][..],
        &["--open-home"][..],
    ] {
        let output = Command::new(BIN)
            .args(["agents", "setup"])
            .args(bad)
            .arg("--root")
            .arg(&root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("Token"));
    }
    assert!(!root.join("agents").join("token").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn setup_needs_no_flags_and_the_host_keeps_the_client_current() {
    use std::os::unix::fs::PermissionsExt;
    let machine = Machine::new();
    let agents = machine.dir.join("r-agents");
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
    // New agent directories and tokens are private; existing directory modes are preserved.
    assert_eq!(mode(&agents), 0o700);
    assert_eq!(mode(&machine.root.join("agents")), 0o700);
    let client = agents.join("prometeu-mcp");
    assert_eq!(mode(&client), 0o755);
    assert_eq!(std::fs::read(&client).unwrap(), std::fs::read(BIN).unwrap());
    machine.wait_for("agent socket", || machine.socket().exists());

    // The JSON output is a local MCP entry whose paths stay valid across updates.
    // It authenticates the actual host user, without assuming a deployment environment.
    let output = Command::new(BIN)
        .args(["agents", "setup", "--json", "--root"])
        .arg(&machine.root)
        .env("HOME", &machine.home)
        .output()
        .unwrap();
    assert!(output.status.success());
    let setup: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(setup["token"], machine.token().trim());
    assert!(setup.get("mount").is_none());
    assert!(setup.get("needs_open").is_none());
    assert_eq!(setup["server"]["command"], client.display().to_string());
    assert_eq!(
        setup["server"]["args"],
        json!(["mcp", "--socket", machine.socket().display().to_string()])
    );
    assert_eq!(
        setup["server"]["env"]["PROMETEU_AGENT_TOKEN"],
        machine.token().trim()
    );
    // SAFETY: getuid has no preconditions.
    let uid = unsafe { libc::getuid() }.to_string();
    assert_eq!(setup["server"]["env"]["PROMETEU_AGENT_HOST_UID"], uid);

    // A host started from another build (an update) replaces a stale copy atomically.
    std::fs::write(&client, b"old build").unwrap();
    let mut machine = machine;
    let _ = machine.host.kill();
    let _ = machine.host.wait();
    machine.host = Command::new(BIN)
        .args(["serve", "--root"])
        .arg(&machine.root)
        .env("HOME", &machine.home)
        .spawn()
        .unwrap();
    machine.wait_for("the refreshed client", || {
        std::fs::read(&client).is_ok_and(|b| b == std::fs::read(BIN).unwrap())
            && mode(&client) == 0o755
    });

    // Disabling drops the token, the socket and the client.
    let disabled = Command::new(BIN)
        .args(["agents", "disable", "--root"])
        .arg(&machine.root)
        .output()
        .unwrap();
    assert!(disabled.status.success());
    let status = Command::new(BIN)
        .args(["agents", "status", "--json", "--root"])
        .arg(&machine.root)
        .output()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["enabled"],
        false
    );
    machine.wait_for("the socket to close", || !machine.socket().exists());
    assert!(!client.exists());
}

#[test]
fn the_generated_local_config_lists_tools_and_relays_calls() {
    check_generated_local_config(false);
}

#[test]
fn a_relative_root_generates_an_mcp_config_usable_from_another_directory() {
    check_generated_local_config(true);
}

#[test]
fn setup_rejects_an_oversized_absolute_socket_before_saving() {
    let machine = Machine::new();
    let directory = machine.dir.join("x".repeat(128));
    std::fs::create_dir(&directory).unwrap();
    let root = directory.join("state");
    let output = Command::new(BIN)
        .args(["agents", "setup", "--json", "--root", "./state"])
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid agent socket path"));
    assert!(
        !root.exists(),
        "a rejected configuration must not enable agents"
    );

    // A short explicit socket allows the state directory to stay where the user chose it.
    let socket = machine.dir.join("short/agents-v1.sock");
    let output = Command::new(BIN)
        .args(["agents", "setup", "--json", "--root", "./state", "--socket"])
        .arg(&socket)
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(output.status.success());
    let setup: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(setup["server"]["args"][2], socket.display().to_string());
    assert!(root.join("agents/token").exists());
}

fn check_generated_local_config(relative: bool) {
    let machine = Machine::with_relative_root(relative);
    let output = Command::new(BIN)
        .args(["agents", "setup", "--json", "--root"])
        .arg(if relative {
            Path::new("./r")
        } else {
            &machine.root
        })
        .current_dir(&machine.dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let setup: Value = serde_json::from_slice(&output.stdout).unwrap();
    let server = &setup["server"];
    let mut mcp = Command::new(server["command"].as_str().unwrap())
        .current_dir(&machine.home)
        .args(
            server["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap()),
        )
        .envs(
            server["env"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k, v.as_str().unwrap())),
        )
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

/// The desktop's side of an SSH attachment, as `remote/connection.rs` speaks it.
struct Desktop {
    child: Child,
    stdin: std::process::ChildStdin,
    frames: std::sync::mpsc::Receiver<Value>,
    next: u64,
}

impl Desktop {
    fn attach(machine: &Machine) -> Self {
        Self::attach_as(machine, "mac")
    }

    fn attach_as(machine: &Machine, device: &str) -> Self {
        let mut child = Command::new(BIN)
            .args(["attach", "--root"])
            .arg(&machine.root)
            .args(["--device", device, "--label", device])
            .env("HOME", &machine.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, frames) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let desktop = Self {
            child,
            stdin,
            frames,
            next: 0,
        };
        desktop.until(|f| f["event"] == "ready");
        desktop
    }

    fn until(&self, mut accept: impl FnMut(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frame = self
                .frames
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("frame before deadline");
            if accept(&frame) {
                return frame;
            }
        }
    }

    /// Send without waiting for the reply; returns the request id.
    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"id": self.next, "method": method, "params": params})
        )
        .unwrap();
        self.next
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send(method, params);
        let reply = self.until(|f| f["id"] == id);
        match reply.get("error") {
            Some(error) => Err(error.as_str().unwrap().to_string()),
            None => Ok(reply["result"].clone()),
        }
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The local stdio MCP that `agents setup` prints, run as an MCP client on the machine runs it.
struct HostMcp {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next: u64,
}

impl HostMcp {
    fn start(machine: &Machine) -> Self {
        let output = Command::new(BIN)
            .args(["agents", "setup", "--json", "--root"])
            .arg(&machine.root)
            .current_dir(&machine.dir)
            .env("HOME", &machine.home)
            .output()
            .unwrap();
        assert!(output.status.success());
        let setup: Value = serde_json::from_slice(&output.stdout).unwrap();
        let server = &setup["server"];
        let mut child = Command::new(server["command"].as_str().unwrap())
            .args(
                server["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap()),
            )
            .envs(
                server["env"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k, v.as_str().unwrap())),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut mcp = Self {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
            next: 0,
        };
        mcp.ask(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "t", "version": "1"}}),
        );
        machine.wait_for("agent socket", || machine.socket().exists());
        mcp
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        )
        .unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()["result"].clone()
    }

    /// The parsed result of a tool, or its error text.
    fn tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self.ask("tools/call", json!({"name": name, "arguments": arguments}));
        let text = result["content"][0]["text"].as_str().unwrap().to_string();
        if result["isError"] == true {
            return Err(text);
        }
        Ok(serde_json::from_str(&text).unwrap())
    }
}

impl Drop for HostMcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_desktop_and_host_mcp_agents_share_one_todo_list() {
    let mut machine = Machine::new();
    let mut mcp = HostMcp::start(&machine);
    let tools = mcp.ask("tools/list", json!({}));
    for name in ["list_todos", "create_todo", "set_todo_status"] {
        assert!(tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == name));
    }
    // Before a desktop selects this machine, agents cannot start a list that nobody sees.
    assert_eq!(
        mcp.tool("create_todo", json!({"title": "Too early"}))
            .unwrap_err(),
        "todos_not_here"
    );
    assert!(!machine.root.join("todos.json").exists());

    let mut desktop = Desktop::attach(&machine);
    desktop
        .call("todos_enable", json!({"enabled": true}))
        .unwrap();
    // An agent cannot select or leave the machine itself.
    let (writer, reader, _) = machine.connect(machine.token().trim());
    let mut agent = Agent {
        writer,
        reader,
        next: 0,
    };
    assert!(agent
        .call("todos_enable", json!({"enabled": false}))
        .is_err());

    // Created on the machine through MCP: the desktop hears it and reads the same item.
    let created = mcp
        .tool("create_todo", json!({"title": "  Review the VM logs "}))
        .unwrap();
    assert_eq!(created["title"], "Review the VM logs");
    assert_eq!(created["status"], "todo");
    let id = created["id"].as_str().unwrap().to_string();
    desktop.until(|f| f["event"] == "todos");
    let listed = desktop.call("todos_list", json!({})).unwrap();
    assert_eq!(listed["items"][0]["id"], id.as_str());

    // A change from the desktop reaches the MCP, and a change from the MCP reaches the desktop.
    desktop
        .call("todos_set_status", json!({"id": id, "status": "doing"}))
        .unwrap();
    assert_eq!(
        mcp.tool("list_todos", json!({})).unwrap()["items"][0]["status"],
        "doing"
    );
    mcp.tool("set_todo_status", json!({"id": id, "status": "completed"}))
        .unwrap();
    desktop.until(|f| f["event"] == "todos");
    assert_eq!(
        desktop.call("todos_list", json!({})).unwrap()["items"][0]["status"],
        "completed"
    );

    // Invalid input changes nothing.
    for (name, arguments, error) in [
        ("create_todo", json!({"title": ""}), "title_required"),
        ("create_todo", json!({"title": "a\nb"}), "invalid_title"),
        (
            "create_todo",
            json!({"title": "x".repeat(201)}),
            "title_too_long",
        ),
        (
            "set_todo_status",
            json!({"id": id, "status": "done"}),
            "invalid_status",
        ),
        (
            "set_todo_status",
            json!({"id": "missing", "status": "todo"}),
            "todo_not_found",
        ),
    ] {
        assert_eq!(mcp.tool(name, arguments).unwrap_err(), error);
    }

    // Agents on separate connections write at the same time; no item is lost.
    let token = machine.token().trim().to_string();
    let writers: Vec<_> = (0..4)
        .map(|n| {
            let (writer, reader, _) = machine.connect(&token);
            std::thread::spawn(move || {
                let mut agent = Agent {
                    writer,
                    reader,
                    next: 0,
                };
                for m in 0..10 {
                    agent
                        .call("todos_create", json!({"title": format!("{n}-{m}")}))
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let items = desktop.call("todos_list", json!({})).unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(items.len(), 41);

    // The list survives a new daemon; a deselected machine stops serving its agents.
    desktop.call("shutdown", json!({"force": true})).unwrap();
    drop(desktop);
    // The old daemon holds the root lock until it exits.
    machine.host.wait().unwrap();
    let mut desktop = Desktop::attach(&machine);
    let again = desktop.call("todos_list", json!({})).unwrap();
    assert_eq!(again["items"].as_array().unwrap().len(), 41);
    assert_eq!(again["items"][0]["status"], "completed");
    desktop
        .call("todos_enable", json!({"enabled": false}))
        .unwrap();
    assert_eq!(
        mcp.tool("list_todos", json!({})).unwrap_err(),
        "todos_not_here"
    );
    let _ = desktop.call("shutdown", json!({"force": true}));
}

#[test]
fn an_update_waits_for_a_desktop_todo_write() {
    let machine = Machine::new();
    let mut writer = Desktop::attach_as(&machine, "writer");
    let mut updater = Desktop::attach_as(&machine, "updater");
    // A FIFO holds the write inside its read of the list until the test feeds it.
    let file = machine.root.join("todos.json");
    sh(&machine.root, "mkfifo todos.json");
    let pending = writer.send("todos_create", json!({"title": "Written during an update"}));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        updater.call("shutdown", json!({})).unwrap_err(),
        "host_busy"
    );
    std::fs::write(&file, r#"{"version":1,"enabled":true,"items":[]}"#).unwrap();
    let done = writer.until(|f| f["id"] == pending);
    assert_eq!(
        done["result"]["title"], "Written during an update",
        "{done}"
    );
    assert!(updater.call("shutdown", json!({})).is_ok());
}
