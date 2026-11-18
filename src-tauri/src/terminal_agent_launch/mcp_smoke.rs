//! Opt-in native MCP activation, without submitting a prompt or calling a tool.
use super::*;
use crate::terminal_agent_hooks::smoke::SmokeTerminal;
use prometeu_core::session::launch::Launch;
use prometeu_process::terminal_state::{Client, Key, Size, Stream};
use prometeu_protocols::terminal_status::{
    ProgramStatusEvent, ProgramStatusParser, PROGRAM_STATUS_QUERY_REPLY,
};
use prometeu_tools::{
    mcp::{AcpMcp, CodexMcp, McpFiles, McpMaterializer, McpSources, Server},
    ClaudeTools,
};
use std::{io::Write, os::unix::fs::PermissionsExt, sync::mpsc::RecvTimeoutError, time::Instant};

const CHILD: &str = "PROMETEU_CURSOR_MCP_SMOKE_CHILD";
const TEST: &str =
    "terminal_agent_launch::mcp_smoke::native_cursor_loads_selected_stdio_mcp_without_inference";
const SELECTED: &str = "prometeu-native-mcp-smoke";
const ENV_KEY: &str = "PROMETEU_MCP_SMOKE_VALUE";

#[test]
#[ignore = "opens an installed Cursor TUI and its selected local MCP; no inference or tool call"]
fn native_cursor_loads_selected_stdio_mcp_without_inference() {
    if std::env::var_os(CHILD).is_none() {
        let root = Scratch(PathBuf::from("/tmp").join(format!(
            "prometeu-cursor-mcp-smoke-{}",
            uuid::Uuid::new_v4()
        )));
        paths::ensure_private_dir(&root.0).unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--ignored", "--nocapture"])
            .env(CHILD, "1")
            .env("PROMETEU_ROOT", root.0.join("app"))
            .current_dir(&root.0)
            .status()
            .unwrap();
        assert!(status.success(), "native Cursor MCP smoke failed");
        return;
    }
    crate::install_crypto();
    run().unwrap();
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        if std::env::var_os("PROMETEU_SMOKE_KEEP_PROOF").is_some() {
            println!("private MCP smoke proof: {}", self.0.display());
            return;
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A private stop file reaches only this fixture server, including when a native CLI detaches it.
/// Its own deadline is the final fallback if the test process is externally terminated.
struct ServerShutdown {
    stop: PathBuf,
    receipt: PathBuf,
}
impl Drop for ServerShutdown {
    fn drop(&mut self) {
        let _ = paths::write_private(&self.stop, "stop");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if let Ok(records) = receipts(&self.receipt) {
                if records.is_empty() || server_stopped(&records) {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn run() -> Result<(), String> {
    let root = std::env::current_dir().map_err(|_| "scratch unavailable")?;
    let workspace = root.join("workspace");
    for directory in ["workspace", "config"] {
        paths::ensure_private_dir(&root.join(directory))?;
    }
    accounts::change(|registry| {
        registry.accounts.push(accounts::Account {
            id: "cursor".into(),
            provider: ProviderId::Cursor,
            revision: 0,
            auth_method: Some("external".into()),
            key_suffix: None,
            identity: Default::default(),
        });
        registry.active.insert("cursor".into(), "cursor".into());
        Ok(())
    })?;
    let server = root.join("server.py");
    let receipt = root.join("methods.jsonl");
    let shutdown = root.join("stop");
    let selected_value = "PROMETEU_SELECTED_STDIO_ENVIRONMENT";
    paths::write_private(&server, include_str!("mcp_smoke_server.py"))?;
    let tools = SelectedTools {
        server: Server {
            id: SELECTED.into(),
            config: json!({"command":"/usr/bin/python3", "args":[server,receipt,shutdown], "env":{ENV_KEY:selected_value}}),
            note: String::new(),
        },
    };
    let request = LaunchRequest {
        session: uuid::Uuid::new_v4().to_string(),
        workspace: uuid::Uuid::new_v4().to_string(),
        worktree: workspace.to_string_lossy().into_owned(),
        settings: Launch {
            agent: ProviderId::Cursor,
            permission: Some(Permission::Ask),
            mcp: Some(vec![SELECTED.into()]),
            ..Default::default()
        },
        mode: StartMode::Fresh,
    };
    // This exercises the same selection encoder, native plugin writer and argv builder as startup.
    let mut prepared = prepare(&request, &crate::account_profiles::native(), &tools)?;
    let plugin = paths::root()
        .join("terminal-agents")
        .join(&request.session)
        .join("cursor-mcp");
    for path in [
        plugin.join("mcp.json"),
        plugin.join(".cursor-plugin/plugin.json"),
    ] {
        let metadata = std::fs::metadata(path).map_err(|_| "private MCP artifact unavailable")?;
        if metadata.permissions().mode() & 0o777 != 0o600 {
            return Err("MCP artifact is not private".into());
        }
    }
    if prepared
        .command
        .get_args()
        .any(|argument| argument.to_string_lossy().contains(selected_value))
    {
        return Err("MCP environment leaked into native argv".into());
    }
    // Preserve the CLI's existing authentication. Its writable settings/trust live only here.
    prepared
        .command
        .env("CURSOR_CONFIG_DIR", root.join("config"))
        .env(ENV_KEY, "inherited-value-must-not-win");
    let mut command = portable_pty::CommandBuilder::new(prepared.command.get_program());
    prometeu_process::terminal::own_terminal(&mut command, Some(env!("CARGO_PKG_VERSION")));
    command.args(prepared.command.get_args());
    command.cwd(&workspace);
    for (key, value) in prepared.command.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../packages/terminal-state/dist")
        .join(format!(
            "prometeu-terminal-state-{}-apple-darwin",
            std::env::consts::ARCH
        ));
    let mut emulator = Client::launch(Command::new(helper), Duration::from_secs(3))
        .map_err(|_| "screen helper unavailable")?;
    let mut screen = Stream::create(
        &mut emulator,
        Key {
            session: request.session.clone(),
            generation: uuid::Uuid::new_v4().to_string(),
        },
        Size {
            cols: 120,
            rows: 40,
        },
        200,
    )
    .map_err(|_| "screen helper creation failed")?;
    // Reverse drop order shuts down the PTY before the fixture's independent fallback.
    let fallback = ServerShutdown {
        stop: shutdown,
        receipt: receipt.clone(),
    };
    let mut terminal = SmokeTerminal::open(command)?;
    let control = terminal
        .control
        .as_ref()
        .ok_or("native control unavailable")?
        .clone();
    let mut parser = ProgramStatusParser::default();
    let mut chunk = 0;
    let mut trusted = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut observed = Vec::new();
    while Instant::now() < deadline {
        match terminal.output.recv_timeout(Duration::from_millis(100)) {
            Ok(bytes) => {
                chunk += 1;
                for event in parser.feed(&bytes) {
                    if matches!(event, ProgramStatusEvent::Query) {
                        write(&mut terminal, PROGRAM_STATUS_QUERY_REPLY)?;
                    }
                }
                let applied = screen
                    .write_chunk(&mut emulator, chunk, &bytes)
                    .map_err(|_| "screen write failed")?;
                for reply in applied.replies {
                    write(&mut terminal, &reply)?;
                }
                let snapshot = screen
                    .snapshot(&mut emulator)
                    .map_err(|_| "screen snapshot failed")?
                    .state;
                if !trusted
                    && snapshot
                        .viewport_lines
                        .iter()
                        .any(|line| trust_line(line) == request.worktree)
                    && snapshot.viewport_lines.iter().any(|line| {
                        matches!(
                            trust_line(line),
                            "[a] Trust this workspace" | "▶ [a] Trust this workspace"
                        )
                    })
                {
                    write(&mut terminal, b"a")?;
                    trusted = true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        observed = receipts(&receipt)?;
        if handshake(&observed) {
            break;
        }
    }
    if let Ok(snapshot) = screen.snapshot(&mut emulator) {
        paths::write_private(
            &root.join("screen.json"),
            &serde_json::to_string(&snapshot.state).map_err(|_| "screen encoding failed")?,
        )?;
    }
    terminal.close();
    if control.running() {
        return Err("native PTY was not reaped".into());
    }
    drop(fallback);
    screen
        .dispose(&mut emulator)
        .map_err(|_| "screen disposal failed")?;
    emulator
        .shutdown()
        .map_err(|_| "screen helper shutdown failed")?;
    let stop_deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < stop_deadline {
        if server_stopped(&receipts(&receipt)?) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let final_receipts = receipts(&receipt)?;
    let stopped = server_stopped(&final_receipts);
    if !trusted || !handshake(&observed) || !stopped {
        return Err(format!(
            "native MCP conformance: trusted={trusted} handshake={} server_stopped={stopped}",
            handshake(&observed)
        ));
    }
    if final_receipts
        .iter()
        .any(|entry| entry["method"] == "tools/call" || entry["environment_matches"] == false)
    {
        return Err("unexpected MCP invocation or environment".into());
    }
    println!("Cursor MCP: initialize/initialized/tools-list verified; selected environment verified; no prompt/tool call; PTY/server/helper stopped");
    Ok(())
}

fn write(terminal: &mut SmokeTerminal, bytes: &[u8]) -> Result<(), String> {
    terminal
        .input
        .as_mut()
        .ok_or("native input closed")?
        .write_all(bytes)
        .map_err(|_| "native input failed".into())
}
fn trust_line(line: &str) -> &str {
    line.trim().trim_matches('│').trim()
}
fn server_stopped(records: &[Value]) -> bool {
    let pids: Vec<_> = records
        .iter()
        .filter(|row| row["event"] == "started")
        .filter_map(|row| row["pid"].as_u64().and_then(|pid| i32::try_from(pid).ok()))
        .filter(|pid| *pid > 1)
        .collect();
    !pids.is_empty()
        && pids.iter().all(|pid| {
            // This is a liveness query, never a signal to a potentially recycled PID.
            (unsafe { libc::kill(*pid, 0) }) == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        })
}
fn receipts(path: &Path) -> Result<Vec<Value>, String> {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Ok(metadata) if metadata.len() <= 65536 => {}
        _ => return Err("invalid MCP receipt size".into()),
    }
    let text = std::fs::read_to_string(path).map_err(|_| "MCP receipt unavailable")?;
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}
fn handshake(records: &[Value]) -> bool {
    ["initialize", "notifications/initialized", "tools/list"]
        .iter()
        .all(|method| {
            records
                .iter()
                .any(|row| row["method"] == *method && row["environment_matches"] == true)
        })
}

struct SelectedTools {
    server: Server,
}
impl McpSources for SelectedTools {
    fn claude_servers(&self, _: &str, _: &[String], _: &Path) -> Result<Vec<Server>, String> {
        Err("unexpected Claude tools".into())
    }
    fn codex_servers(&self, _: &str, chosen: &[String]) -> Result<Vec<Server>, String> {
        if chosen != [SELECTED] {
            return Err("MCP selection changed".into());
        }
        Ok(vec![
            self.server.clone(),
            Server {
                id: "unselected".into(),
                config: json!({"command":"must-not-launch-unselected-server"}),
                note: String::new(),
            },
        ])
    }
    fn bearer(&self, _: &str) -> Option<String> {
        None
    }
}
impl McpFiles for SelectedTools {
    fn claude_config(&self, _: &str, _: &str) -> Result<PathBuf, String> {
        Err("unexpected Claude artifact".into())
    }
    fn codex_environment(&self, _: &str, _: &str, _: &str) -> Result<PathBuf, String> {
        Err("unexpected Codex artifact".into())
    }
}
impl StartupTools for SelectedTools {
    fn claude(
        &self,
        _: &str,
        _: &Path,
        _: Option<&[String]>,
        _: Option<&[String]>,
    ) -> Result<ClaudeTools, String> {
        Err("unexpected Claude preparation".into())
    }
    fn codex_plugins(
        &self,
        _: &str,
        _: Option<&[String]>,
        _: &accounts::Profile,
    ) -> Result<CodexPlugins, String> {
        Err("unexpected Codex preparation".into())
    }
    fn codex_mcp(&self, _: &str, _: Option<&[String]>) -> Result<Option<CodexMcp>, String> {
        Err("unexpected Codex MCP".into())
    }
    fn cursor_mcp(
        &self,
        session: &str,
        chosen: Option<&[String]>,
    ) -> Result<Option<AcpMcp>, String> {
        McpMaterializer {
            sources: self,
            files: self,
        }
        .cursor_config(session, chosen)
    }
}
