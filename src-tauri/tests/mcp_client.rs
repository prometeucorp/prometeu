//! An ordinary external MCP host can register once and reconnect after desktop socket rotation.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_Prometeu"));
    command
        .env("PROMETEU_ROOT", root)
        .env_remove("PROMETEU_MCP_SOCKET")
        .env_remove("PROMETEU_MCP_TOKEN");
    command
}

fn admin(root: &Path, args: &[&str]) -> Value {
    let output = command(root)
        .arg("--prometeu-mcp-client")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn external_stdio_host_registers_discovers_reconnects_and_revokes() {
    let root = std::path::PathBuf::from("/tmp").join(format!("pmcp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("board.json"),
        json!({"projects":[
            {"id":"repo", "name":"Repository", "path":"/source/repo"}
        ]})
        .to_string(),
    )
    .unwrap();
    assert_eq!(admin(&root, &["projects"])["projects"][0]["id"], "repo");
    let invalid = command(&root)
        .args(["--prometeu-mcp-client", "register", "agent", "missing"])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!root.join("mcp-clients").exists());
    let config = admin(&root, &["register", "external agent", "repo"]);
    let server = &config["mcpServers"]["prometeu"];
    let token = server["env"]["PROMETEU_MCP_TOKEN"]
        .as_str()
        .unwrap()
        .to_string();
    let clients = admin(&root, &["list"]);
    assert_eq!(clients["clients"][0]["id"], config["client_id"]);
    assert!(!clients.to_string().contains(&token));
    assert_eq!(server["command"], env!("CARGO_BIN_EXE_Prometeu"));
    assert!(server["env"].get("PROMETEU_MCP_SOCKET").is_none());

    let first = UnixListener::bind(root.join("first")).unwrap();
    let second = UnixListener::bind(root.join("second")).unwrap();
    std::fs::write(
        root.join("mcp-socket"),
        root.join("first").to_str().unwrap(),
    )
    .unwrap();
    let host_root = root.clone();
    let host_token = token.clone();
    let host = std::thread::spawn(move || {
        for (index, listener) in [first, second].into_iter().enumerate() {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "stdio host did not connect");
                std::thread::sleep(Duration::from_millis(10));
            };
            // BSD can inherit the listener's nonblocking flag on accepted sockets.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["token"], host_token);
            assert_eq!(request["name"], "list_projects");
            assert_eq!(request["arguments"], json!({}));
            // Publish the new desktop address before the already-running host makes its next call.
            std::fs::write(
                host_root.join("mcp-socket"),
                host_root.join("second").to_str().unwrap(),
            )
            .unwrap();
            writeln!(stream, "{}", json!({"result":{"connection":index}})).unwrap();
        }
    });
    let mut child = command(&root)
        .arg("--prometeu-mcp")
        .env("PROMETEU_MCP_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"external","version":"1"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_projects","arguments":{}}}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    host.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 4);
    assert!(replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "delegate"));
    assert_eq!(replies[2]["result"]["structuredContent"]["connection"], 0);
    assert_eq!(replies[3]["result"]["structuredContent"]["connection"], 1);
    admin(&root, &["revoke", config["client_id"].as_str().unwrap()]);
    assert_eq!(admin(&root, &["list"])["clients"], json!([]));
    std::fs::remove_dir_all(root).unwrap();
}
