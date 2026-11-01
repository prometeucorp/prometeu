use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(20);

struct Process(Child);

impl Process {
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "search process did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Client {
    process: Process,
    input: Option<ChildStdin>,
    replies: Receiver<Result<Value, String>>,
    next_id: u64,
}

impl Client {
    fn open(executable: &str, root: &Path, database: &Path) -> Self {
        let mut process = Process(
            Command::new(executable)
                .arg("--prometeu-search-mcp")
                .arg(root)
                .arg(database)
                .env(
                    "PROMETEU_ROOT",
                    database.parent().unwrap().join("host-state"),
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let input = process.0.stdin.take().unwrap();
        let output = process.0.stdout.take().unwrap();
        let (send, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let reply = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if send.send(reply).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            process,
            input: Some(input),
            replies,
            next_id: 0,
        };
        let reply = client.request(
            "initialize",
            json!({
                "protocolVersion":"2025-06-18", "capabilities":{},
                "clientInfo":{"name":"bundled-search-test", "version":"1"}
            }),
        );
        assert_eq!(reply["result"]["serverInfo"]["name"], "prometeu-search");
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        client
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let input = self.input.as_mut().unwrap();
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0", "id":self.next_id, "method":method, "params":params})
        )
        .unwrap();
        input.flush().unwrap();
        let reply = self
            .replies
            .recv_timeout(TIMEOUT)
            .expect("search response timed out or process closed stdout")
            .expect("search stdout must contain only JSON-RPC");
        assert_eq!(reply["id"], self.next_id);
        reply
    }

    fn search(&mut self, query: &str) -> Value {
        let reply = self.request(
            "tools/call",
            json!({"name":"code_search", "arguments":{"query":query, "max_bytes":1024}}),
        );
        assert_eq!(reply["result"]["isError"], false, "{reply}");
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.len() <= 1024,
            "result exceeded its requested byte budget"
        );
        serde_json::from_str(text).unwrap()
    }

    fn close(mut self) {
        drop(self.input.take());
        assert!(
            self.process.wait().success(),
            "stdin EOF must stop search cleanly"
        );
    }
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("bundled search {}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) {
    let mut process = Process(
        Command::new("git")
            .current_dir(root)
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
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    assert!(process.wait().success(), "Git fixture preparation failed");
}

pub fn worktree_search_contract(executable: &str) {
    let temp = Fixture::new();
    let first = temp.0.join("first worktree");
    let second = temp.0.join("second worktree");
    std::fs::create_dir(&first).unwrap();
    git(&first, &["init", "--quiet"]);
    std::fs::write(first.join("main.rs"), "pub fn initial() {}\n").unwrap();
    git(&first, &["add", "main.rs"]);
    git(&first, &["commit", "--quiet", "-m", "Initial source"]);
    git(
        &first,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            second.to_str().unwrap(),
        ],
    );
    std::fs::write(first.join("main.rs"), "pub fn orchid() {}\n").unwrap();
    std::fs::write(second.join("main.rs"), "pub fn cactus() {}\n").unwrap();

    let first_database = temp.0.join("first cache/index.sqlite");
    let second_database = temp.0.join("second cache/index.sqlite");
    let mut owner = Client::open(executable, &first, &first_database);
    let mut other = Client::open(executable, &second, &second_database);
    let tools = owner.request("tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["code_search", "code_explore", "code_search_status"]);
    assert_eq!(owner.search("orchid")["results"][0]["name"], "orchid");
    assert!(owner.search("cactus")["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(other.search("cactus")["results"][0]["name"], "cactus");
    assert!(other.search("orchid")["results"]
        .as_array()
        .unwrap()
        .is_empty());

    for arguments in [
        json!({"query":"orchid", "root":second}),
        json!({"query":"orchid", "path":"../second worktree/main.rs"}),
    ] {
        let reply = owner.request(
            "tools/call",
            json!({"name":"code_explore", "arguments":arguments}),
        );
        assert_eq!(
            reply["error"]["code"], -32602,
            "tool arguments cannot change roots"
        );
    }

    let mut survivor = Client::open(executable, &first, &first_database);
    assert_eq!(survivor.search("orchid")["results"][0]["name"], "orchid");
    owner.close();
    std::fs::write(first.join("main.rs"), "pub fn replacement() {}\n").unwrap();
    assert_eq!(
        survivor.search("replacement")["results"][0]["name"],
        "replacement"
    );
    assert!(survivor.search("orchid")["results"]
        .as_array()
        .unwrap()
        .is_empty());
    std::fs::rename(first.join("main.rs"), first.join("renamed.rs")).unwrap();
    assert_eq!(
        survivor.search("replacement")["results"][0]["path"],
        "renamed.rs"
    );
    std::fs::remove_file(first.join("renamed.rs")).unwrap();
    assert!(survivor.search("replacement")["results"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(other.search("cactus")["results"][0]["name"], "cactus");
    survivor.close();
    other.close();
    assert!(first_database.is_file() && second_database.is_file());
    assert!(!first_database.parent().unwrap().join("host-state").exists());
    assert!(!second_database
        .parent()
        .unwrap()
        .join("host-state")
        .exists());
}
