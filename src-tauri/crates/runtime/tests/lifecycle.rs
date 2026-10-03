#![cfg(unix)]
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
struct Client {
    child: Child,
    input: ChildStdin,
    events: Receiver<Value>,
}
impl Client {
    fn open(root: &Path, workdir: &Path, executable: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_prometeu-runtime"))
            .args([
                "--root",
                root.to_str().unwrap(),
                "--workdir",
                workdir.to_str().unwrap(),
                "--codex",
                executable.to_str().unwrap(),
            ])
            .env("CODEX_HOME", root.parent().unwrap().join("provider-home"))
            .env(
                "CLAUDE_CONFIG_DIR",
                root.parent().unwrap().join("claude-home"),
            )
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if send.send(serde_json::from_str(&line).unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            events,
        };
        client.until(|v| v["lifecycle"] == "ready");
        client
    }
    fn write(&mut self, id: u64, action: Value) {
        writeln!(self.input, "{}", json!({"v":1,"id":id,"action":action})).unwrap();
        self.input.flush().unwrap();
    }
    fn until(&mut self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let value = self
                .events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("runtime output timed out");
            if predicate(&value) {
                return value;
            }
        }
    }
    fn request(&mut self, id: u64, action: Value) -> Value {
        self.write(id, action);
        self.until(|v| v["id"] == id)
    }
    fn message(&mut self, id: u64, text: &str) {
        self.write(
            id,
            json!({"method":"command","frame":{"v":1,"type":"message.send","text":text}}),
        );
        let mut accepted = false;
        let mut completed = false;
        while !accepted || !completed {
            let value = self.until(|_| true);
            if value["id"] == id {
                assert_eq!(value["result"]["accepted"], true);
                accepted = true;
            }
            if value["event"]["type"] == "turn.completed" {
                completed = true;
            }
        }
    }
    fn shutdown(&mut self) {
        assert_eq!(
            self.request(99, json!({"method":"shutdown"}))["result"]["stopped"],
            true
        );
        assert!(self.child.wait().unwrap().success());
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Fixture {
    root: PathBuf,
    workdir: PathBuf,
    executable: PathBuf,
}
fn fixture_git(path: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(path)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!("prometeu-runtime-{}", uuid::Uuid::new_v4()));
        let workdir = base.join("work");
        std::fs::create_dir_all(&workdir).unwrap();
        let executable = base.join("codex");
        std::fs::write(&executable, include_str!("fake_codex.py")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root: base.join("runtime"),
            workdir,
            executable,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.root.parent().unwrap()).unwrap();
    }
}
#[test]
fn executable_launches_messages_stops_and_resumes_after_runtime_restart() {
    let fixture = Fixture::new();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    client.write(1, json!({"method":"start"}));
    client.until(|v| v["event"]["type"] == "session.identity");
    client.message(2, "first message");
    assert!(client.request(3, json!({"method":"start"}))["error"].is_string());
    assert_eq!(
        client.request(4, json!({"method":"stop"}))["result"]["stopped"],
        true
    );
    assert!(client.request(
        5,
        json!({"method":"command","frame":{"v":1,"type":"message.send","text":"rejected"}})
    )["error"]
        .is_string());
    client.shutdown();
    drop(client);
    std::fs::set_permissions(
        fixture.root.join("runtime.lock"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    let mut resumed = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let snapshot = resumed.request(6, json!({"method":"snapshot"}));
    assert_eq!(snapshot["result"]["providerSession"], "fixture-thread");
    assert!(snapshot["result"]["snapshot"]["text"]
        .as_str()
        .unwrap()
        .contains("first message"));
    resumed.write(7, json!({"method":"start"}));
    resumed.until(|v| v["event"]["type"] == "session.identity");
    resumed.message(8, "second message");
    resumed.shutdown();
    assert_eq!(
        std::fs::metadata(fixture.root.join("runtime.lock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let transcript = std::fs::read_to_string(fixture.root.join("transcript.jsonl")).unwrap();
    assert!(transcript.contains("second message"));
    assert!(!transcript.contains("rejected"));
    assert_eq!(
        std::fs::metadata(fixture.root.join("runtime.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(fixture.root.join("transcript.jsonl"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn rejects_concurrent_owners_and_foreign_roots() {
    let fixture = Fixture::new();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let output = Command::new(env!("CARGO_BIN_EXE_prometeu-runtime"))
        .args([
            "--root",
            fixture.root.to_str().unwrap(),
            "--workdir",
            fixture.workdir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already in use"));
    client.shutdown();
    let foreign = fixture.root.parent().unwrap().join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("board.json"), "keep").unwrap();
    assert!(prometeu_runtime::store::Store::open(&foreign, &fixture.workdir).is_err());
    assert!(!foreign.join("runtime.lock").exists());
    assert_eq!(
        std::fs::read_to_string(foreign.join("board.json")).unwrap(),
        "keep"
    );
}

#[test]
fn busy_input_is_rejected_and_stop_reaps_a_group_with_inherited_output() {
    let fixture = Fixture::new();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    client.write(1, json!({"method":"start"}));
    client.until(|v| v["event"]["type"] == "session.identity");
    client.write(
        2,
        json!({"method":"command","frame":{"v":1,"type":"message.send","text":"hold"}}),
    );
    client.until(|v| v["event"]["block"]["text"] == "child-started");
    assert!(client.request(
        3,
        json!({"method":"command","frame":{"v":1,"type":"message.send","text":"must not queue"}})
    )["error"]
        .is_string());
    let before = std::time::Instant::now();
    assert_eq!(
        client.request(4, json!({"method":"stop"}))["result"]["stopped"],
        true
    );
    assert!(before.elapsed() < Duration::from_secs(10));
    client.shutdown();
    assert!(
        !std::fs::read_to_string(fixture.root.join("transcript.jsonl"))
            .unwrap()
            .contains("must not queue")
    );
}

#[test]
fn protocol_errors_leave_the_runtime_available_and_cannot_start_a_process() {
    let fixture = Fixture::new();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    writeln!(client.input, "not-json").unwrap();
    client.input.flush().unwrap();
    assert!(client.until(|v| v["id"].is_null() && v["error"].is_string())["error"].is_string());
    writeln!(
        client.input,
        "{}",
        json!({"v":2,"id":7,"action":{"method":"start"}})
    )
    .unwrap();
    client.input.flush().unwrap();
    assert_eq!(
        client.until(|v| v["id"] == 7)["error"],
        "unsupported protocol version"
    );
    let snapshot = client.request(8, json!({"method":"snapshot"}));
    assert!(snapshot["result"]["providerSession"].is_null());
    client.shutdown();
}

impl Client {
    fn app_result(&mut self, command: &str, args: Value) -> Value {
        self.request(
            80,
            json!({"method":"application","command":command,"args":args}),
        )
    }
    fn app(&mut self, command: &str, args: Value) -> Value {
        let reply = self.app_result(command, args);
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }
    fn conversation(&mut self, session: &str, text: &str) {
        self.app("chat_send", json!({"session":session,"text":text}));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let snapshot = self.app("chat_snapshot", json!({"session":session}));
            let transcript = snapshot["text"].as_str().unwrap();
            if transcript
                .rsplit_once(text)
                .is_some_and(|(_, tail)| tail.contains("turn.completed"))
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{snapshot}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn application_lifecycle_preserves_transcripts_and_stops_only_owned_execution() {
    let fixture = Fixture::new();
    let script = include_str!("fake_codex.py").replace("import json", "import pathlib\nimport json")
        .replace("    if method == \"account/read\":", "    if method in ('thread/start', 'thread/resume', 'turn/start'):\n        pathlib.Path('launches.jsonl').open('a').write(json.dumps(request) + '\\n')\n    if method == \"account/read\":");
    std::fs::write(&fixture.executable, script).unwrap();
    std::fs::create_dir_all(fixture.workdir.join(".prometeu")).unwrap();
    std::fs::write(
        fixture.workdir.join(".prometeu/settings.toml"),
        "[scripts]\narchive = 'printf \"%s\" \"$PROMETEU_WORKSPACE_NAME\" > archived.txt'\n",
    )
    .unwrap();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    client.conversation("primary", "Original conversation");
    let created = client.request(
        2,
        json!({"method":"workspace_create","title":"Sibling","path":fixture.workdir}),
    );
    let sibling = created["result"]["board"]["workspaces"][1]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client.conversation(&sibling, "Sibling conversation");
    let tab = client.app(
        "new_tab",
        json!({"workspace":"primary","prompt":"Second tab"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client.app(
        "open_dock",
        json!({"id":"primary","kind":"terminal","cols":80,"rows":24}),
    );
    client.app(
        "open_dock",
        json!({"id":sibling,"kind":"terminal","cols":80,"rows":24}),
    );
    for (command, args) in [
        (
            "rename_workspace",
            json!({"id":"primary","title":"  Renamed  "}),
        ),
        (
            "rename_tab",
            json!({"workspace":"primary","tab":tab,"title":"  Notes  "}),
        ),
        ("pin_workspace", json!({"id":"primary","pinned":true})),
        ("set_unread", json!({"id":"primary","unread":true})),
    ] {
        client.app(command, args);
    }
    let invalid = client.app_result("set_tab_choice", json!({"id":"primary","tab":"primary","choice":{"agent":"claude","model":"other","effort":"high"}}));
    assert!(invalid["error"]
        .as_str()
        .unwrap()
        .contains("err.session.otherAgent"));
    client.app("set_tab_choice", json!({"id":"primary","tab":"primary","choice":{"agent":"codex","model":"fixture","effort":"high"}}));
    let board = client.app("load_board", Value::Null);
    assert_eq!(board["workspaces"][0]["tabs"][0]["status"], "desligada");
    assert_ne!(board["workspaces"][1]["tabs"][0]["status"], "desligada");
    client.conversation("primary", "After model change");
    let launches = std::fs::read_to_string(fixture.workdir.join("launches.jsonl")).unwrap();
    let launches: Vec<Value> = launches
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let last = launches
        .iter()
        .rev()
        .find(|v| v["method"] == "thread/resume")
        .unwrap();
    assert_eq!(last["method"], "thread/resume");
    assert_eq!(last["params"]["model"], "fixture");
    assert_eq!(launches.last().unwrap()["params"]["effort"], "high");
    client.app("finish_workspace", json!({"id":"primary"}));
    let board = client.app("load_board", Value::Null);
    let ws = &board["workspaces"][0];
    assert_eq!(ws["title"], "Renamed");
    assert_eq!(ws["tabs"][1]["title"], "Notes");
    assert_eq!(ws["archived"], true);
    assert_eq!(ws["pinned"], true);
    assert_eq!(ws["unread"], true);
    assert_eq!(
        ws["stage"],
        *board["stages"].as_array().unwrap().last().unwrap()
    );
    assert!(ws["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|t| t["status"] == "desligada"));
    assert_eq!(
        client.app("dock_state", json!({"id":"primary"}))[0]["alive"],
        false
    );
    assert_eq!(
        client.app("dock_state", json!({"id":sibling}))[0]["alive"],
        true
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !fixture.workdir.join("archived.txt").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(fixture.workdir.join("archived.txt")).unwrap(),
        "work"
    );
    assert!(
        client.app_result("cleanup_worktree", json!({"id":"primary","force":true}))["error"]
            .as_str()
            .unwrap()
            .contains("err.cleanup.isRepo")
    );
    client.shutdown();
    drop(client);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let board = client.app("load_board", Value::Null);
    assert_eq!(board["workspaces"][0]["title"], "Renamed");
    assert_eq!(
        board["workspaces"][0]["tabs"][0]["choice"]["effort"],
        "high"
    );
    let history = client.app("chat_snapshot", json!({"session":"primary"}));
    assert!(history["text"]
        .as_str()
        .unwrap()
        .contains("Original conversation"));
    assert!(history["text"]
        .as_str()
        .unwrap()
        .contains("After model change"));
    client.app(
        "archive_workspace",
        json!({"id":"primary","archived":false}),
    );
    client.conversation("primary", "After unarchive");
    let launches = std::fs::read_to_string(fixture.workdir.join("launches.jsonl")).unwrap();
    let last: Value = serde_json::from_str(launches.lines().last().unwrap()).unwrap();
    assert_eq!(last["params"]["effort"], "high");
    client.app("remove_workspace", json!({"id":"primary"}));
    assert!(fixture.workdir.is_dir());
    assert!(fixture.root.join("transcript.jsonl").is_file());
    assert!(
        client.app_result("chat_send", json!({"session":"primary","text":"Removed"}))["error"]
            .is_string()
    );
    assert_eq!(
        client.app("load_board", Value::Null)["workspaces"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    client.shutdown();
}

#[test]
fn worktree_cleanup_requires_archive_and_confirmation_and_retains_history_after_restart() {
    let fixture = Fixture::new();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&fixture.workdir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Lifecycle test"]);
    git(&["config", "user.email", "test@example.test"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["config", "core.hooksPath", "/dev/null"]);
    git(&["commit", "--allow-empty", "-qm", "initial"]);
    git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let catalog = client.request(2, json!({"method":"workspace_worktree","request":{"title":"Disposable","path":fixture.workdir,"branch":"cleanup-test","base":"HEAD"}}));
    let ws = &catalog["result"]["board"]["workspaces"][1];
    let id = ws["id"].as_str().unwrap().to_owned();
    let worktree = PathBuf::from(ws["worktree"].as_str().unwrap());
    client.conversation(&id, "Retained after cleanup");
    assert!(
        client.app_result("cleanup_worktree", json!({"id":id,"force":true}))["error"]
            .as_str()
            .unwrap()
            .contains("err.cleanup.notArchived")
    );
    client.app("archive_workspace", json!({"id":id,"archived":true}));
    std::fs::write(worktree.join("unsaved.txt"), "Local changes").unwrap();
    assert!(client.app("cleanup_list", Value::Null)[0]["blocked"]
        .as_str()
        .unwrap()
        .contains("err.cleanup.dirty"));
    assert!(
        client.app_result("cleanup_worktree", json!({"id":id,"force":false}))["error"]
            .as_str()
            .unwrap()
            .contains("err.cleanup.dirty")
    );
    assert!(worktree.join("unsaved.txt").exists());
    client.app("cleanup_worktree", json!({"id":id,"force":true}));
    client.app("cleanup_worktree", json!({"id":id,"force":true}));
    assert!(!worktree.exists());
    assert!(fixture.workdir.join(".git").exists());
    assert!(!git(&["branch", "--list", "cleanup-test"]).contains("cleanup-test"));
    assert!(client
        .app("cleanup_list", Value::Null)
        .as_array()
        .unwrap()
        .is_empty());
    let empty = client.request(3, json!({"method":"workspace_worktree","request":{"title":"Never started","path":fixture.workdir,"branch":"empty-cleanup","base":"HEAD"}}));
    let empty_id = empty["result"]["board"]["workspaces"][2]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    client.app("archive_workspace", json!({"id":empty_id,"archived":true}));
    client.app("cleanup_worktree", json!({"id":empty_id,"force":false}));
    client.shutdown();
    drop(client);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let empty = client.app("chat_snapshot", json!({"session":empty_id}));
    assert!(empty["text"]
        .as_str()
        .unwrap()
        .lines()
        .all(|line| serde_json::from_str::<Value>(line).unwrap()["type"] == "session.state"));
    let snapshot = client.app("chat_snapshot", json!({"session":id}));
    assert!(snapshot["text"]
        .as_str()
        .unwrap()
        .contains("Retained after cleanup"));
    assert!(
        client.app_result("chat_send", json!({"session":id,"text":"Cannot execute"}))["error"]
            .as_str()
            .unwrap()
            .contains("err.session.cleaned")
    );
    assert!(client.app_result(
        "open_dock",
        json!({"id":id,"kind":"terminal","cols":80,"rows":24})
    )["error"]
        .as_str()
        .unwrap()
        .contains("err.session.cleaned"));
    client.shutdown();
}

#[test]
fn workspace_tools_apply_on_spawn_preserve_live_sessions_and_gate_repository_declarations() {
    let fixture = Fixture::new();
    let script = include_str!("fake_codex.py")
        .replace("import json", include_str!("tool_probe.py"))
        .replace(
            "        if method == \"thread/resume\":",
            "        record_tools(request)\n        if method == \"thread/resume\":",
        );
    std::fs::write(&fixture.executable, script).unwrap();
    let home = fixture.root.parent().unwrap().join("provider-home");
    std::fs::create_dir_all(&home).unwrap();
    let original = "[plugins.\"personal@example\"]\nenabled = true\n";
    std::fs::write(home.join("config.toml"), original).unwrap();
    std::fs::create_dir_all(fixture.workdir.join(".prometeu")).unwrap();
    let declaration = fixture.workdir.join(".prometeu/settings.toml");
    std::fs::write(
        &declaration,
        "[tools]\nmcp = { add = ['project'] }\nskills = { add = ['skill-review'] }\n",
    )
    .unwrap();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    client.app("skill_save", json!({"skill":{"id":"review","description":"Review a diff","content":"Read every changed line."}}));
    for (id, config) in [
        (
            "global",
            json!({"url":"https://example.invalid/global", "headers":{"Authorization":"Bearer private-test-token"}}),
        ),
        (
            "project",
            json!({"command":"/bin/echo", "args":["project-tool"]}),
        ),
        (
            "local",
            json!({"command":"/bin/echo", "env":{"LOCAL_SECRET":"private-local-token"}}),
        ),
    ] {
        client.app("mcp_save", json!({"server":{"id":id,"config":config}}));
    }
    assert!(
        client.app_result("mcp_save", json!({"server":{"id":"prometeu","config":{}}}))["error"]
            .as_str()
            .unwrap()
            .contains("err.mcp.builtin")
    );
    client.app(
        "set_tools_global",
        json!({"mcp":{"base":"none","add":["global"]}}),
    );
    client.app(
        "set_workspace_mcp",
        json!({"id":"primary","mcp":{"add":["local"]}}),
    );
    client.app("set_workspace_mcp", json!({"id":"primary"}));
    let effective = client.app("workspace_tools", json!({"id":"primary","agent":"codex"}));
    assert!(effective["mcp"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"] == "project" && s["provenance"] == "pending"));
    let pending = client.app("project_tools", json!({"id":"primary"}));
    assert_eq!(pending["pending"], true);
    client.conversation("primary", "Before approval");
    let launches = || -> Vec<Value> {
        std::fs::read_to_string(fixture.workdir.join("tool-launches.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    assert_eq!(launches().len(), 1);
    let first = &launches()[0];
    assert!(first["mcp"].get("global").is_some());
    assert!(first["mcp"].get("local").is_some());
    assert!(first["mcp"].get("project").is_none());
    assert_eq!(first["home"], home.to_str().unwrap());
    assert!(!first["args"].to_string().contains("private-test-token"));
    assert!(!first["args"].to_string().contains("private-local-token"));
    let environment = fixture.root.join("mcp/primary.local.env");
    assert!(std::fs::read_to_string(&environment)
        .unwrap()
        .contains("private-local-token"));
    assert_eq!(
        std::fs::metadata(&environment)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let before = client.app("load_board", Value::Null);
    assert!(client.app_result(
        "set_tools_global",
        json!({"mcp":null,"skills":{"add":["wrong-axis"]}})
    )["error"]
        .as_str()
        .unwrap()
        .contains("err.tools.badAxis"));
    assert_eq!(
        client.app("load_board", Value::Null)["tools"],
        before["tools"]
    );
    std::fs::write(
        &declaration,
        "[tools]\nmcp = { add = ['project', 'local'] }\nskills = { add = ['skill-review'] }\n",
    )
    .unwrap();
    assert!(client.app_result(
        "project_tools_trust",
        json!({"id":"primary","hash":pending["hash"],"approved":true})
    )["error"]
        .as_str()
        .unwrap()
        .contains("err.tools.changed"));
    let current = client.app("project_tools", json!({"id":"primary"}));
    client.app(
        "project_tools_trust",
        json!({"id":"primary","hash":current["hash"],"approved":true}),
    );
    client.app("set_workspace_mcp", json!({"id":"primary","mcp":null}));
    // A registered local plugin uses the same pipeline as a generated standalone skill.
    let package = fixture.root.parent().unwrap().join("local-plugin");
    std::fs::create_dir_all(package.join(".claude-plugin")).unwrap();
    std::fs::write(
        package.join(".claude-plugin/plugin.json"),
        r#"{"name":"local-review","version":"1.0.0"}"#,
    )
    .unwrap();
    fixture_git(&package, &["init", "-q"]);
    fixture_git(&package, &["add", "."]);
    fixture_git(&package, &["commit", "-qm", "initial plugin"]);
    assert_eq!(
        client.app(
            "plugin_install",
            json!({"source":format!("file://{}",package.display())})
        )["saved"],
        true
    );
    client.app(
        "set_workspace_plugins",
        json!({"id":"primary","plugins":{"add":["local-review"]}}),
    );
    client.conversation("primary", "Keep the running process");
    assert_eq!(launches().len(), 1);
    let stop = |client: &mut Client| {
        client.app("set_tab_choice", json!({"id":"primary","tab":"primary","choice":{"agent":"codex","model":"fixture","effort":"high"}}));
    };
    stop(&mut client);
    client.conversation("primary", "After approval");
    let records = launches();
    let resumed = records.last().unwrap();
    assert_eq!(resumed["method"], "thread/resume");
    assert!(resumed["mcp"].get("project").is_some());
    assert!(resumed["plugins"]
        .as_array()
        .unwrap()
        .contains(&json!("skill-review@prometeu")));
    assert!(resumed["plugins"]
        .as_array()
        .unwrap()
        .contains(&json!("local-review@prometeu")));
    assert!(resumed["plugins"]
        .as_array()
        .unwrap()
        .contains(&json!("personal@example")));
    let derived = PathBuf::from(resumed["home"].as_str().unwrap());
    assert!(derived.starts_with(&fixture.root));
    assert_eq!(
        std::fs::metadata(derived.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::read_to_string(home.join("config.toml")).unwrap(),
        original
    );
    client.app(
        "set_workspace_skills",
        json!({"id":"primary","skills":{"base":"none","add":[]}}),
    );
    client.app(
        "set_workspace_plugins",
        json!({"id":"primary","plugins":{"base":"none","add":[]}}),
    );
    client.app(
        "set_workspace_mcp",
        json!({"id":"primary","mcp":{"base":"none","add":[]}}),
    );
    client.conversation("primary", "Not restarted by deselection");
    assert_eq!(launches().len(), 2);
    stop(&mut client);
    client.conversation("primary", "Explicit empty selections");
    let records = launches();
    assert_eq!(records.last().unwrap()["mcp"], json!({}));
    assert_eq!(
        records.last().unwrap()["plugins"],
        json!(["personal@example"])
    );
    client.app(
        "set_workspace_skills",
        json!({"id":"primary","skills":null}),
    );
    stop(&mut client);
    std::fs::write(fixture.root.parent().unwrap().join("fail-install"), "fail").unwrap();
    let rejected = client.app_result(
        "chat_send",
        json!({"session":"primary","text":"Must not send"}),
    );
    assert!(rejected["error"]
        .as_str()
        .unwrap()
        .contains("fixture installation refused"));
    assert_eq!(launches().len(), 3);
    std::fs::remove_file(fixture.root.parent().unwrap().join("fail-install")).unwrap();
    client.shutdown();
    drop(client);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    assert_eq!(
        client.app("project_tools", json!({"id":"primary"}))["pending"],
        false
    );
    client.conversation("primary", "Restored selection");
    assert!(launches().last().unwrap()["plugins"]
        .as_array()
        .unwrap()
        .contains(&json!("skill-review@prometeu")));
    let snapshot = client.app("chat_snapshot", json!({"session":"primary"}));
    assert!(snapshot["text"]
        .as_str()
        .unwrap()
        .contains("Before approval"));
    assert!(!snapshot["text"].as_str().unwrap().contains("Must not send"));
    client.shutdown();
}

#[test]
fn plugin_import_updates_and_removal_keep_catalog_choices_and_user_files() {
    let fixture = Fixture::new();
    let source = fixture.root.parent().unwrap().join("marketplace");
    let manifest = |root: &Path, name: &str, description: &str| {
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            json!({"name":name,"description":description,"version":"1.0.0"}).to_string(),
        )
        .unwrap();
    };
    manifest(&source.join("plugins/alpha"), "alpha", "Original");
    manifest(&source.join("plugins/beta"), "beta", "Original");
    let private = fixture.root.parent().unwrap().join("private");
    manifest(&private, "personal", "Private");
    std::os::unix::fs::symlink(&private, source.join("plugins/escape")).unwrap();
    std::fs::create_dir_all(source.join(".agents/plugins")).unwrap();
    std::fs::write(source.join(".agents/plugins/marketplace.json"), json!({"plugins":[{"source":"./plugins/alpha"},{"source":{"source":"local","path":"./plugins/beta"}},{"source":"./plugins/escape"},{"source":"../../private"}]}).to_string()).unwrap();
    fixture_git(&source, &["init", "-q"]);
    fixture_git(&source, &["add", "."]);
    fixture_git(&source, &["commit", "-qm", "initial"]);
    let url = format!("file://{}", source.display());
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let found = client.app("plugin_install", json!({"source":url}));
    assert_eq!(found["saved"], false);
    assert_eq!(found["plugins"].as_array().unwrap().len(), 2);
    assert_eq!(client.app("plugin_hub", Value::Null), json!([]));
    client.app("plugin_scrap", json!({"dir":private}));
    assert!(private.exists());
    client.app("plugin_scrap", json!({"dir":found["dir"]}));
    assert!(!Path::new(found["dir"].as_str().unwrap()).exists());
    let found = client.app("plugin_install", json!({"source":url}));
    let alpha = found["plugins"][0].clone();
    assert!(
        client.app_result("plugin_save", json!({"plugin":alpha,"revision":1}))["error"]
            .as_str()
            .unwrap()
            .contains("err.catalog.disconnected")
    );
    assert_eq!(client.app("plugin_hub", Value::Null), json!([]));
    client.app("plugin_save", json!({"plugin":alpha}));
    client.app("plugin_scrap", json!({"dir":found["dir"]}));
    assert!(Path::new(alpha["source"].as_str().unwrap()).exists());
    assert!(
        client.app_result("plugin_install", json!({"source":url}))["error"]
            .as_str()
            .unwrap()
            .contains("err.plugin.exists")
    );
    manifest(&source.join("plugins/alpha"), "alpha", "Updated");
    fixture_git(&source, &["add", "."]);
    fixture_git(&source, &["commit", "-qm", "update"]);
    assert_eq!(
        client.app("plugin_update", json!({"id":"alpha"}))[0]["note"],
        "Updated"
    );
    let checkout = PathBuf::from(found["dir"].as_str().unwrap());
    std::fs::write(checkout.join("local.txt"), "Local work").unwrap();
    fixture_git(&checkout, &["add", "."]);
    fixture_git(&checkout, &["commit", "-qm", "local change"]);
    manifest(&source.join("plugins/alpha"), "alpha", "Diverged");
    fixture_git(&source, &["add", "."]);
    fixture_git(&source, &["commit", "-qm", "remote change"]);
    assert!(
        client.app_result("plugin_update", json!({"id":"alpha"}))["error"]
            .as_str()
            .unwrap()
            .contains("err.plugin.pull")
    );
    assert_eq!(client.app("plugin_hub", Value::Null)[0]["note"], "Updated");
    assert_eq!(
        std::fs::read_to_string(checkout.join("local.txt")).unwrap(),
        "Local work"
    );
    let personal = client.app("plugin_look", json!({"source":private}));
    assert_eq!(personal["made"], false);
    client.app("plugin_save", json!({"plugin":personal}));
    client.shutdown();
    drop(client);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    assert_eq!(
        client
            .app("plugin_hub", Value::Null)
            .as_array()
            .unwrap()
            .len(),
        2
    );
    client.app("plugin_remove", json!({"id":"personal"}));
    assert!(private.exists());
    client.app("plugin_remove", json!({"id":"alpha"}));
    assert!(!Path::new(alpha["source"].as_str().unwrap()).exists());
    assert!(checkout.join("plugins/beta").exists());
    client.app("plugin_scrap", json!({"dir":checkout}));
    assert!(!checkout.exists());
    client.shutdown();
}

impl prometeu_bridge::application::ApplicationClient for Client {
    fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
        let response = self.app_result(&command, args);
        match response.get("error").and_then(Value::as_str) {
            Some(error) => Err(error.into()),
            None => Ok(response["result"].clone()),
        }
    }
}
struct OAuthServer {
    child: Child,
    url: String,
    directory: PathBuf,
}
impl OAuthServer {
    fn start(directory: PathBuf) -> Self {
        let mut child = Command::new("python3")
            .args([
                "-u",
                "-c",
                include_str!("../../../../scripts/fixtures/mcp-oauth.py"),
            ])
            .arg(&directory)
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut url = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut url)
            .unwrap();
        Self {
            child,
            url: url.trim().into(),
            directory,
        }
    }
    fn server(&self) -> Value {
        json!({"id":"fixture-auth","config":{"url":format!("{}/mcp",self.url)},"note":"Fixture"})
    }
    fn stats(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join("stats.json")).unwrap()).unwrap()
    }
}
impl Drop for OAuthServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct FixtureConsent;
impl prometeu_oauth::Consent for FixtureConsent {
    fn authorize(&self, request: &prometeu_oauth::ConsentRequest) -> Result<String, String> {
        use std::io::Read;
        let address = request.authorize.strip_prefix("http://").unwrap();
        let (address, path) = address.split_once('/').unwrap();
        let mut socket = std::net::TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            socket,
            "GET /{path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut result = String::new();
        socket.read_to_string(&mut result).unwrap();
        assert!(result.starts_with("HTTP/1.0 302"));
        assert!(result.contains(prometeu_oauth::MCP_REDIRECT));
        Ok(request.state.clone())
    }
}
fn mcp(client: &mut Client, command: &str, args: Value) -> Value {
    prometeu_bridge::mcp::request(client, &FixtureConsent, command, args).unwrap()
}
#[test]
fn mcp_oauth_uses_shared_pkce_private_tokens_refresh_and_restart_logout() {
    let fixture = Fixture::new();
    let server = OAuthServer::start(fixture.root.parent().unwrap().join("oauth"));
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    assert_eq!(mcp(&mut client, "mcp_logins", json!({})), json!([]));
    let args = json!({"server":server.server()});
    let check = mcp(&mut client, "mcp_check", args.clone());
    assert_eq!(check["probe"]["auth"], true);
    assert!(check["steps"]
        .as_array()
        .unwrap()
        .iter()
        .all(|step| step["ok"] == true));
    assert_eq!(mcp(&mut client, "mcp_login", args.clone()), Value::Null);
    assert_eq!(
        mcp(&mut client, "mcp_logins", json!({})),
        json!(["fixture-auth"])
    );
    let auth_path = fixture.root.join("mcp-auth.json");
    assert_eq!(
        std::fs::metadata(&auth_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let check = mcp(&mut client, "mcp_check", args.clone());
    assert_eq!(check["probe"]["ok"], true);
    assert_eq!(check["probe"]["tools"], 1);
    let saved: Value = serde_json::from_slice(&std::fs::read(&auth_path).unwrap()).unwrap();
    assert_eq!(saved["fixture-auth"]["access_token"], "fixture-renewed");
    assert_eq!(saved["fixture-auth"]["refresh_token"], "fixture-refresh");
    assert_eq!(server.stats()["refreshes"], 1);
    // The original startup encoder receives the refreshed credential through its source port.
    client.app("mcp_save", args.clone());
    let resources = prometeu_runtime::resources::native(&fixture.root);
    let tools = prometeu_runtime::tools::native(&fixture.root, &fixture.executable, &resources);
    let encoded = tools
        .codex_mcp("test", Some(&["fixture-auth".into()]))
        .unwrap()
        .unwrap();
    assert!(!encoded.0.contains("fixture-renewed"));
    assert!(encoded
        .1
        .iter()
        .any(|(_, value)| value == "Bearer fixture-renewed"));
    // A second authorization reuses registration and still requires a fresh PKCE verifier/code.
    mcp(&mut client, "mcp_login", args.clone());
    assert_eq!(server.stats()["registrations"], 1);
    assert_eq!(server.stats()["exchanges"], 2);
    client.shutdown();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    assert_eq!(
        mcp(&mut client, "mcp_logins", json!({})),
        json!(["fixture-auth"])
    );
    assert_eq!(
        mcp(&mut client, "mcp_check", args.clone())["probe"]["ok"],
        true
    );
    mcp(&mut client, "mcp_logout", json!({"id":"fixture-auth"}));
    assert_eq!(mcp(&mut client, "mcp_logins", json!({})), json!([]));
    assert_eq!(mcp(&mut client, "mcp_check", args)["probe"]["auth"], true);
    client.shutdown();
    assert_eq!(std::fs::read_to_string(&auth_path).unwrap().trim(), "{}");
}
#[test]
fn deferred_mcp_inspection_keeps_application_requests_responsive() {
    let fixture = Fixture::new();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let gate = fixture.workdir.join("release-mcp");
    let script = r#"import json,pathlib,sys,time
while not pathlib.Path(sys.argv[1]).exists(): time.sleep(.01)
for line in sys.stdin:
    req=json.loads(line)
    if req.get('id') == 1: print(json.dumps({'id':1,'result':{'serverInfo':{'name':'fixture'}}}),flush=True)
    if req.get('id') == 2: print(json.dumps({'id':2,'result':{'tools':[{'name':'fixture'}]}}),flush=True)
"#;
    let started=client.app("mcp_operation_start",json!({"operation":{"kind":"check","server":{"id":"slow","note":"","config":{"command":"python3","args":["-c",script,gate]}}}}));
    let now = std::time::Instant::now();
    let board = client.app("load_board", Value::Null);
    assert!(now.elapsed() < Duration::from_secs(3));
    assert!(board["workspaces"].is_array());
    assert_eq!(
        client.app("mcp_operation_poll", started.clone())["done"],
        false
    );
    std::fs::write(gate, "").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let result = client.app("mcp_operation_poll", started.clone());
        if result["done"] == true {
            assert_eq!(result["result"]["probe"]["tools"], 1);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    client.shutdown();
}

#[test]
fn mcp_discovery_reuses_user_and_repository_configuration_without_registering_it() {
    let fixture = Fixture::new();
    let home = fixture.root.parent().unwrap().join("claude-home");
    std::fs::create_dir_all(&home).unwrap();
    let local = json!({"command":"fixture-mcp"});
    let root = json!({"mcpServers":{"user":local},"projects":{fixture.workdir.to_str().unwrap():{"mcpServers":{"project":local}}}});
    std::fs::write(home.join(".claude.json"), root.to_string()).unwrap();
    std::fs::write(
        fixture.workdir.join(".mcp.json"),
        json!({"mcpServers":{"repository":local,"user":local}}).to_string(),
    )
    .unwrap();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let found = client.app("mcp_found", Value::Null);
    assert_eq!(found.as_array().unwrap().len(), 3);
    assert_eq!(client.app("mcp_hub", Value::Null), json!([]));
    client.app("mcp_save", json!({"server":found[0]}));
    assert_eq!(
        client
            .app("mcp_found", Value::Null)
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(home.join(".claude.json")).unwrap(),
        root.to_string()
    );
    client.shutdown();
}

#[test]
fn deferred_discovery_keeps_requests_responsive_and_preserves_catalog_errors() {
    let fixture = Fixture::new();
    let script = include_str!("fake_codex.py").replace("import json", "import pathlib\nimport json").replace(
        "    params = request.get(\"params\", {})",
        "    params = request.get(\"params\", {})\n    if method in ('account/read', 'model/list') and pathlib.Path('hold-discovery').exists():\n        pathlib.Path(method.replace('/', '-') + '-entered').touch()\n        while pathlib.Path('hold-discovery').exists(): time.sleep(.01)",
    );
    std::fs::write(&fixture.executable, script).unwrap();
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    std::fs::write(fixture.workdir.join("hold-discovery"), "").unwrap();
    let models = client.app(
        "application_operation_start",
        json!({"command":"agent_models","args":{"agent":"codex"}}),
    );
    let accounts = client.app(
        "application_operation_start",
        json!({"command":"accounts","args":null}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !(fixture.workdir.join("model-list-entered").exists()
        && fixture.workdir.join("account-read-entered").exists())
    {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let now = std::time::Instant::now();
    assert!(client.app("load_board", Value::Null)["workspaces"].is_array());
    assert!(now.elapsed() < Duration::from_secs(3));
    for job in [&models, &accounts] {
        assert_eq!(
            client.app("application_operation_poll", job.clone())["done"],
            false
        );
    }
    std::fs::remove_file(fixture.workdir.join("hold-discovery")).unwrap();
    for (job, field) in [(models, "models"), (accounts, "accounts")] {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let reply = client.app("application_operation_poll", job.clone());
            if reply["done"] == true {
                assert!(!reply["result"][field].as_array().unwrap().is_empty());
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let job = client.app(
        "application_operation_start",
        json!({"command":"agent_models","args":{"agent":"claude"}}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let reply = client.app_result("application_operation_poll", job.clone());
        if let Some(error) = reply["error"].as_str() {
            assert!(error.starts_with("application-error:"));
            assert!(error.contains("unavailable"));
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    client.shutdown();
}

#[test]
fn deferred_setup_prepares_off_loop_and_preserves_existing_files_and_single_execution() {
    use std::os::fd::FromRawFd;
    let fixture = Fixture::new();
    fixture_git(&fixture.workdir, &["init", "-b", "main"]);
    std::fs::write(fixture.workdir.join("tracked"), "original").unwrap();
    fixture_git(&fixture.workdir, &["add", "tracked"]);
    fixture_git(&fixture.workdir, &["commit", "-m", "initial"]);
    let mut client = Client::open(&fixture.root, &fixture.workdir, &fixture.executable);
    let catalog = client.request(50, json!({"method":"workspace_worktree","request":{"title":"Prepared","path":fixture.workdir,"branch":"feature/setup","base":"main"}}));
    assert!(catalog["error"].is_null(), "{catalog}");
    let workspace = &catalog["result"]["board"]["workspaces"][1];
    let id = workspace["id"].as_str().unwrap();
    let checkout = Path::new(workspace["worktree"].as_str().unwrap());
    std::fs::write(checkout.join("tracked"), "keep local edit").unwrap();
    std::fs::write(fixture.workdir.join(".env"), "PRIVATE_TEST=value").unwrap();
    let declarations = fixture.workdir.join(".prometeu/settings.toml");
    std::fs::create_dir(declarations.parent().unwrap()).unwrap();
    let path = std::ffi::CString::new(declarations.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let args = json!({"id":id,"kind":"setup","cols":80,"rows":24});
    let job = client.app(
        "application_operation_start",
        json!({"command":"open_dock","args":args}),
    );
    // A FIFO holds native settings preparation without relying on a slow disk or sleeps.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut writer = loop {
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd >= 0 {
            break unsafe { std::fs::File::from_raw_fd(fd) };
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    let now = std::time::Instant::now();
    assert!(client.app("load_board", Value::Null)["workspaces"].is_array());
    assert!(now.elapsed() < Duration::from_secs(3));
    assert_eq!(
        client.app("application_operation_poll", job.clone())["done"],
        false
    );
    client.app(
        "open_dock",
        json!({"id":id,"kind":"terminal","cols":80,"rows":24}),
    );
    for (command, args) in [
        (
            "application_operation_start",
            json!({"command":"open_dock","args":args}),
        ),
        ("open_dock", args.clone()),
        ("close_dock", json!({"id":id,"kind":"setup"})),
        ("remove_workspace", json!({"id":id})),
    ] {
        assert!(client.app_result(command, args)["error"]
            .as_str()
            .unwrap()
            .contains("err.windows.operationBusy"));
    }
    let body = "[scripts]\nsetup = \"echo once >> setup-count; while [ ! -f setup-release ]; do sleep .01; done\"\n[worktree]\ncopy = [\".env\", \"tracked\"]\n";
    std::fs::remove_file(&declarations).unwrap();
    std::fs::write(&declarations, body).unwrap();
    writer.write_all(body.as_bytes()).unwrap();
    drop(writer);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if client.app("application_operation_poll", job.clone())["done"] == true {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(checkout.join(".env")).unwrap(),
        "PRIVATE_TEST=value"
    );
    assert_eq!(
        std::fs::read_to_string(checkout.join("tracked")).unwrap(),
        "keep local edit"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !checkout.join("setup-count").exists() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let reused = client.app(
        "application_operation_start",
        json!({"command":"open_dock","args":args}),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if client.app("application_operation_poll", reused.clone())["done"] == true {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read_to_string(checkout.join("setup-count")).unwrap(),
        "once\n"
    );
    std::fs::write(checkout.join("setup-release"), "").unwrap();
    client.shutdown();
}
