//! Agent clients on the machine (ADR 0103). They use Prometeu as a person would: register
//! projects, open workspaces, read and answer conversations, close workspaces. They reach the host
//! through a second socket that offers only those operations and requires a token, never through
//! the desktop socket, whose `exec` and file methods amount to a shell.
use crate::registry::{self, Conversation, Creator, Project, Workspace};
use crate::server::{now, open, read_line, Host};
use prometeu_core::command::{CommandError, CommandPolicy, CommandRunner, OutputPolicy};
use prometeu_core::lock::lock;
use prometeu_host::{
    claude_hook_settings, claude_project_slug, worktree_dir_name, HOOK_NONCE_ENV,
    HOOK_PROVIDER_ENV, HOOK_SESSION_ENV, HOOK_TOKEN_ENV, LINE_LIMIT,
};
use prometeu_process::command::UnixCommandRunner;
use serde_json::{json, Value};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bumped with incompatible changes to the agent socket; the hello frame carries it.
pub const AGENT_PROTOCOL: u32 = 1;
const CONFIG: &str = "agents.json";
const READ_LIMIT: usize = 200;
const TAIL_BYTES: u64 = 8 * 1_048_576;
/// Serialized messages per reply, well under the socket's line limit.
const REPLY_BUDGET: usize = 4 * 1_048_576;
const GIT: Duration = Duration::from_secs(60);
const CLONE: Duration = Duration::from_secs(600);
/// Variables a Claude process exports to its children. A host started from inside a Claude
/// session must not hand them to the conversations it launches.
const INHERITED_CLAUDE: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_CHILD_NOTES_DIR",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SSE_PORT",
];

pub fn token_path(root: &Path) -> PathBuf {
    root.join("agents").join("token")
}

/// The agent socket's path: `agents.json`'s `socket`, or `<root>/agents/agents-v1.sock`.
pub fn socket_path(root: &Path) -> PathBuf {
    std::fs::read(root.join(CONFIG))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|config| {
            config
                .get("socket")
                .and_then(Value::as_str)
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| {
            root.join("agents")
                .join(format!("agents-v{AGENT_PROTOCOL}.sock"))
        })
}

fn random_bytes(count: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; count];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn uuid() -> Result<String, String> {
    let mut b = random_bytes(16)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}

fn private_dir(path: &Path) -> Result<(), String> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())
}

/// Write `agents.json` and the token, creating the token when missing or when `rotate` asks for a
/// new one. Returns the token and the socket path.
pub fn setup(
    root: &Path,
    socket: Option<&Path>,
    rotate: bool,
) -> Result<(String, PathBuf), String> {
    private_dir(&root.join("agents"))?;
    if let Some(socket) = socket {
        if !socket.is_absolute() {
            return Err("the socket path must be absolute".into());
        }
        let config = json!({"socket": socket.display().to_string()});
        registry::write_private(&root.join(CONFIG), config.to_string().as_bytes())?;
    }
    let path = token_path(root);
    let token = match std::fs::read_to_string(&path) {
        Ok(token) if !rotate && !token.trim().is_empty() => token.trim().to_string(),
        _ => {
            let token = hex(&random_bytes(32)?);
            registry::write_private(&path, token.as_bytes())?;
            token
        }
    };
    Ok((token, socket_path(root)))
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Connections that have not presented a token yet. A socket another user can reach must not let
/// idle connections pile up; each also gets `HELLO_WAIT` to authenticate.
const PENDING: usize = 16;
/// Authenticated agent connections.
const CONNECTIONS: usize = 64;
const HELLO_WAIT: Duration = Duration::from_secs(10);

static PENDING_COUNT: AtomicUsize = AtomicUsize::new(0);
static CONNECTION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Holds one slot of a counter until dropped.
struct Slot(&'static AtomicUsize);
impl Slot {
    fn take(counter: &'static AtomicUsize, limit: usize) -> Option<Self> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < limit).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(counter))
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn is_socket(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket())
}

/// Serve the agent socket for the host's lifetime. The configured path is checked every few
/// seconds, so `prometeu-host agents setup` takes effect without restarting the host and its
/// terminals. When the path changes, the previous listener stops and its socket is removed, so a
/// directory the socket was moved away from no longer reaches the host.
pub fn listen(host: Arc<Host>) {
    std::thread::spawn(move || {
        let mut bound: Option<(PathBuf, Arc<AtomicBool>)> = None;
        let mut refused: Option<PathBuf> = None;
        loop {
            let wanted = socket_path(&host.root);
            let serving = bound.as_ref().is_some_and(|(p, _)| *p == wanted) && is_socket(&wanted);
            // A refused path is retried at every poll, since it may be freed; it is logged once.
            if !serving {
                match bind(&wanted) {
                    Ok(listener) => {
                        eprintln!("prometeu-host agents on {}", wanted.display());
                        if let Some((old, stop)) = bound.take() {
                            retire(&old, &stop, old != wanted);
                        }
                        let stop = Arc::new(AtomicBool::new(false));
                        accept(host.clone(), listener, stop.clone());
                        bound = Some((wanted, stop));
                        refused = None;
                    }
                    Err(error) => {
                        if refused.as_ref() != Some(&wanted) {
                            eprintln!("prometeu-host agents: {}: {error}", wanted.display());
                        }
                        refused = Some(wanted);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

/// Stop a listener: flag it, wake its blocking accept with one connection, and remove its socket
/// when the host moved to another path.
fn retire(path: &Path, stop: &AtomicBool, remove: bool) {
    stop.store(true, Ordering::Release);
    let _ = UnixStream::connect(path);
    if remove && is_socket(path) {
        let _ = std::fs::remove_file(path);
    }
}

fn accept(host: Arc<Host>, listener: UnixListener, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stop.load(Ordering::Acquire) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let Some(pending) = Slot::take(&PENDING_COUNT, PENDING) else {
                continue;
            };
            let host = host.clone();
            std::thread::spawn(move || {
                let _ = serve(&host, stream, pending);
            });
        }
    });
}

/// Bind the agent socket. An existing entry is replaced only when it is a socket nobody answers,
/// such as one a previous host left behind; anything else is refused rather than deleted.
fn bind(path: &Path) -> Result<UnixListener, String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            private_dir(parent)?;
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(meta) if !meta.file_type().is_socket() => {
            return Err("exists and is not a socket; refusing to replace it".into())
        }
        Ok(_) if UnixStream::connect(path).is_ok() => {
            return Err("another process is serving this socket".into())
        }
        Ok(_) => std::fs::remove_file(path).map_err(|e| e.to_string())?,
        Err(_) => {}
    }
    let listener = UnixListener::bind(path).map_err(|e| e.to_string())?;
    // The token, not the file mode, decides who may use the socket: another Unix user or a
    // container reaches it only where the person exposed its directory.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666))
        .map_err(|e| e.to_string())?;
    Ok(listener)
}

/// Whether `offered` is the current token. The file is read every time, so rotating the token
/// cuts off connections that are already open at their next request.
fn authorized(root: &Path, offered: &str) -> bool {
    let token = std::fs::read_to_string(token_path(root)).unwrap_or_default();
    !token.trim().is_empty() && same(offered.as_bytes(), token.trim().as_bytes())
}

fn serve(host: &Arc<Host>, stream: UnixStream, pending: Slot) -> Result<(), String> {
    stream
        .set_read_timeout(Some(HELLO_WAIT))
        .map_err(|e| e.to_string())?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let Some(first) = read_line(&mut reader)? else {
        return Ok(());
    };
    let hello: Value = serde_json::from_str(&first).map_err(|e| e.to_string())?;
    let offered = hello
        .pointer("/hello/token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let protocol = hello.pointer("/hello/protocol").and_then(Value::as_u64);
    if !authorized(&host.root, &offered) {
        let _ = writeln!(
            writer,
            "{}",
            json!({"event": "error", "error": "unauthorized"})
        );
        return Ok(());
    }
    if protocol != Some(AGENT_PROTOCOL as u64) {
        let _ = writeln!(writer, "{}", json!({"event": "error", "error": "protocol"}));
        return Ok(());
    }
    let Some(_connection) = Slot::take(&CONNECTION_COUNT, CONNECTIONS) else {
        let _ = writeln!(writer, "{}", json!({"event": "error", "error": "busy"}));
        return Ok(());
    };
    drop(pending);
    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
    let client = hello
        .pointer("/hello/client")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .map(|c| c.chars().take(64).collect::<String>());
    let creator = Creator {
        kind: "agent".into(),
        name: client,
    };
    writeln!(
        writer,
        "{}",
        json!({"event": "ready", "protocol": AGENT_PROTOCOL, "version": env!("CARGO_PKG_VERSION")})
    )
    .map_err(|e| e.to_string())?;
    while let Some(line) = read_line(&mut reader)? {
        if line.is_empty() {
            continue;
        }
        if !authorized(&host.root, &offered) {
            let _ = writeln!(
                writer,
                "{}",
                json!({"event": "error", "error": "unauthorized"})
            );
            return Ok(());
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                writeln!(
                    writer,
                    "{}",
                    json!({"id": null, "error": error.to_string()})
                )
                .map_err(|e| e.to_string())?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let frame = match dispatch(host, &creator, method, &params) {
            Ok(result) => json!({"id": id, "result": result}),
            Err(error) => json!({"id": id, "error": error}),
        };
        let mut text = frame.to_string();
        if text.len() > LINE_LIMIT {
            text = json!({"id": id, "error": "reply too large"}).to_string();
        }
        writeln!(writer, "{text}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("missing {key}"))
}

fn optional<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn dispatch(
    host: &Arc<Host>,
    creator: &Creator,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    match method {
        "ping" => Ok(json!({})),
        "projects_list" => Ok(json!(host.registry.read()?.projects)),
        "project_add" => project_add(host, creator, text(params, "path")?),
        "project_clone" => project_clone(
            host,
            creator,
            text(params, "url")?,
            optional(params, "path"),
        ),
        "workspace_create" => workspace_create(host, creator, params),
        "workspace_list" => {
            let all = params.get("all").and_then(Value::as_bool) == Some(true);
            let registry = host.registry.read()?;
            Ok(Value::Array(
                registry
                    .workspaces
                    .iter()
                    .filter(|w| all || !w.closed)
                    .map(|w| describe(host, w))
                    .collect(),
            ))
        }
        "workspace_get" => {
            let id = text(params, "id")?;
            let registry = host.registry.read()?;
            let workspace = registry
                .workspaces
                .iter()
                .find(|w| w.id == id)
                .ok_or("unknown_workspace")?;
            Ok(describe(host, workspace))
        }
        "conversation_open" => conversation_open(host, params),
        "conversation_read" => conversation_read(host, params),
        "conversation_send" => conversation_send(host, text(params, "id")?, text(params, "text")?),
        "conversation_interrupt" => {
            let id = text(params, "id")?;
            // Only conversations in the registry: an agent must never reach other terminals.
            find_conversation(host, id)?;
            if !host.is_running(id) {
                return Err("conversation_ended".into());
            }
            // Escape is how a person interrupts Claude's turn in its terminal.
            host.write(id, b"\x1b")?;
            Ok(json!({}))
        }
        "workspace_close" => workspace_close(
            host,
            text(params, "id")?,
            params.get("force").and_then(Value::as_bool) == Some(true),
        ),
        other => Err(format!("unknown method {other}")),
    }
}

/// A workspace with each conversation's observed state.
fn describe(host: &Host, workspace: &Workspace) -> Value {
    let mut value = json!(workspace);
    let conversations: Vec<Value> = workspace
        .conversations
        .iter()
        .map(|c| {
            let activity = host.activity(&c.id);
            let mut value = json!(c);
            value["state"] = json!(activity.state);
            value["state_at"] = json!(activity.at);
            value["notification"] = json!(activity.notification);
            value
        })
        .collect();
    value["conversations"] = Value::Array(conversations);
    value
}

struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Output {
    fn ok(self) -> Result<String, String> {
        match self.success {
            true => Ok(self.stdout.trim().to_string()),
            false => Err(self.stderr.trim().to_string()),
        }
    }
}

/// Run a command without a terminal. The deadline covers reading its output too, and the whole
/// process group is stopped at it, so an SSH helper that keeps a pipe open cannot hold the call.
/// Git never prompts: there is no one to answer.
fn run(argv: &[&str], timeout: Duration) -> Result<Output, String> {
    let mut command = Command::new(argv[0]);
    command.args(&argv[1..]).env("GIT_TERMINAL_PROMPT", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let policy = CommandPolicy {
        timeout,
        stdout: OutputPolicy::Capture {
            limit: 4 * 1_048_576,
        },
        stderr: OutputPolicy::Capture { limit: 1_048_576 },
    };
    match UnixCommandRunner.run(&mut command, &[], policy) {
        Ok(output) => Ok(Output {
            success: output.success,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Err(CommandError::Timeout) => Err(format!("{} timed out", argv.join(" "))),
        Err(error) => Err(format!("{}: {error:?}", argv[0])),
    }
}

fn git(repo: &str, args: &[&str]) -> Result<String, String> {
    let mut argv = vec!["git", "-C", repo];
    argv.extend(args);
    run(&argv, GIT)?.ok()
}

fn home() -> Result<String, String> {
    std::env::var("HOME")
        .ok()
        .filter(|h| h.starts_with('/'))
        .ok_or_else(|| "HOME is not set".to_string())
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn project_add(host: &Host, creator: &Creator, path: &str) -> Result<Value, String> {
    if !path.starts_with('/') {
        return Err("the project path must be absolute".into());
    }
    let top = git(path, &["rev-parse", "--show-toplevel"])
        .map_err(|_| format!("{path} is not a Git repository"))?;
    let origin = git(&top, &["remote", "get-url", "origin"]).ok();
    let project = Project {
        name: basename(&top),
        path: top,
        origin: origin.filter(|o| !o.is_empty()),
        added_at: now(),
        added_by: creator.clone(),
    };
    host.registry
        .update(|registry| Ok(json!(registry.add_project(project))))
}

fn project_clone(
    host: &Host,
    creator: &Creator,
    url: &str,
    path: Option<&str>,
) -> Result<Value, String> {
    if url.starts_with('-') {
        return Err("invalid url".into());
    }
    let path = match path {
        Some(path) if path.starts_with('/') => path.to_string(),
        Some(_) => return Err("the clone path must be absolute".into()),
        None => format!(
            "{}/code/{}",
            home()?,
            basename(url).trim_end_matches(".git")
        ),
    };
    if Path::new(&path).exists() {
        return Err(format!("{path} already exists"));
    }
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    run(&["git", "clone", "--", url, &path], CLONE)?.ok()?;
    project_add(host, creator, &path)
}

/// Serializes this host's edits of `~/.claude.json`. Claude processes also rewrite the file;
/// they cannot be locked out, which is why the entry is written before the folder is used.
static TRUST: Mutex<()> = Mutex::new(());

/// Mark a folder trusted in Claude's own configuration. Claude asks whether to trust every new
/// folder, even without permission prompts, and a conversation nobody watches would wait on that
/// question forever. Only the exact folder counts; a trusted parent does not.
fn trust(folder: &str) -> Result<(), String> {
    let _guard = lock(&TRUST);
    let path = PathBuf::from(home()?).join(".claude.json");
    let mut config = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .map_err(|e| format!("~/.claude.json unreadable: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.to_string()),
    };
    let Some(object) = config.as_object_mut() else {
        return Err("~/.claude.json is not an object".into());
    };
    let projects = object.entry("projects").or_insert_with(|| json!({}));
    let Some(projects) = projects.as_object_mut() else {
        return Err("~/.claude.json projects is not an object".into());
    };
    let entry = projects.entry(folder).or_insert_with(|| json!({}));
    if entry.get("hasTrustDialogAccepted") == Some(&json!(true)) {
        return Ok(());
    }
    entry["hasTrustDialogAccepted"] = json!(true);
    let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    registry::write_private(&path, &bytes)
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn transcript_path(worktree: &str, id: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(home()?)
        .join(".claude")
        .join("projects")
        .join(claude_project_slug(worktree))
        .join(format!("{id}.jsonl")))
}

/// Workspaces being closed. Launches hold `LAUNCHES` while they check this set and open their
/// terminal, and a close registers here under the same lock before it collects the terminals to
/// stop, so no conversation can start in a workspace whose worktree is being removed.
static CLOSING: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LAUNCHES: Mutex<()> = Mutex::new(());

/// Start Claude for a conversation the way the desktop does on a machine (ADR 0101), without
/// permission prompts. `prompt` becomes its first message. A conversation whose transcript exists
/// is resumed instead of started.
fn launch(
    host: &Arc<Host>,
    workspace: &Workspace,
    conversation: &str,
    prompt: Option<&str>,
) -> Result<(), String> {
    let _launching = lock(&LAUNCHES);
    let closed = host
        .registry
        .read()?
        .workspaces
        .iter()
        .any(|w| w.id == workspace.id && w.closed);
    if closed || lock(&CLOSING).contains(&workspace.id) {
        return Err("workspace_closed".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = format!(
        "{} hook --root {}",
        quote(&exe.display().to_string()),
        quote(&host.root.display().to_string())
    );
    let sessions = host.root.join("sessions").join(conversation);
    private_dir(&sessions)?;
    let settings = sessions.join("settings.json");
    registry::write_private(
        &settings,
        claude_hook_settings(&helper).to_string().as_bytes(),
    )?;
    let resume = transcript_path(&workspace.worktree, conversation)?.exists();
    let program = std::env::var("PROMETEU_HOST_CLAUDE").unwrap_or_else(|_| "claude".into());
    let mut argv = vec![
        program,
        if resume { "--resume" } else { "--session-id" }.into(),
        conversation.into(),
        "--dangerously-skip-permissions".into(),
        "--settings".into(),
        settings.display().to_string(),
    ];
    if let Some(prompt) = prompt {
        // A leading dash would read as an option.
        argv.push(match prompt.starts_with('-') {
            true => format!(" {prompt}"),
            false => prompt.to_string(),
        });
    }
    let params = json!({
        "id": conversation,
        "workspace": workspace.id,
        "argv": argv,
        "cwd": workspace.worktree,
        "env": {
            HOOK_SESSION_ENV: conversation,
            HOOK_PROVIDER_ENV: "claude",
            // A desktop that relays these hooks substitutes its own bridge's values.
            HOOK_TOKEN_ENV: "host",
            HOOK_NONCE_ENV: "host",
            "TERM_PROGRAM": "Prometeu",
            "TERM_PROGRAM_VERSION": env!("CARGO_PKG_VERSION"),
        },
        "unset": INHERITED_CLAUDE,
        "cols": 120,
        "rows": 40,
    });
    open(host, &params).map(|_| ())
}

fn title_from(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    match line.char_indices().nth(60) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_string(),
    }
}

fn workspace_create(host: &Arc<Host>, creator: &Creator, params: &Value) -> Result<Value, String> {
    let prompt = text(params, "prompt")?;
    let key = text(params, "project")?;
    let project = host
        .registry
        .read()?
        .project(key)
        .cloned()
        .ok_or_else(|| format!("unknown_project {key}: register it with project_add first"))?;
    let repo = project.path.clone();
    // Best effort: start from what the remote has now, as a person who just pulled would.
    let _ = run(&["git", "-C", &repo, "fetch", "--quiet", "origin"], GIT);
    let taken: Vec<String> = git(
        &repo,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes",
        ],
    )?
    .lines()
    .map(str::to_owned)
    .collect();
    let requested = optional(params, "branch");
    if requested.is_some_and(|b| b.starts_with('-')) {
        return Err("invalid branch".into());
    }
    let branch = match requested {
        Some(branch) => branch.to_string(),
        None => {
            let entropy = random_bytes(64)?;
            let mut index = 0;
            prometeu_host::naming::fresh_branch(&taken, || {
                let value =
                    u64::from_le_bytes(entropy[index..index + 8].try_into().unwrap_or([0; 8]));
                index = (index + 8) % 56;
                value
            })
        }
    };
    let existing = taken.iter().any(|b| b == &branch);
    let base = match optional(params, "base") {
        Some(base) => base.to_string(),
        None => git(
            &repo,
            &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        )
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "HEAD".into()),
    };
    let parent = host.root.join("worktrees").join(&project.name);
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    // Claude names transcript folders after its physical working directory, so the recorded
    // path must not go through a symbolic link.
    let parent = std::fs::canonicalize(&parent).map_err(|e| e.to_string())?;
    let worktree = parent
        .join(worktree_dir_name(&branch))
        .display()
        .to_string();
    if Path::new(&worktree).exists() {
        return Err(format!("{worktree} already exists"));
    }
    // Before Git creates anything, so a failure here leaves nothing behind.
    trust(&worktree)?;
    if existing {
        git(&repo, &["worktree", "add", "--", &worktree, &branch])?;
    } else {
        git(
            &repo,
            &["worktree", "add", "-b", &branch, "--", &worktree, &base],
        )?;
    }
    let conversation = Conversation {
        id: uuid()?,
        title: title_from(prompt),
        created_at: now(),
        permission: "auto".into(),
    };
    let workspace = Workspace {
        id: uuid()?,
        project: repo.clone(),
        title: optional(params, "title")
            .map(str::to_owned)
            .unwrap_or_else(|| title_from(prompt)),
        branch: branch.clone(),
        base,
        worktree: worktree.clone(),
        created_at: now(),
        created_by: creator.clone(),
        closed: false,
        keep_branch: existing,
        conversations: vec![conversation.clone()],
    };
    let recorded = host.registry.update(|registry| {
        registry.put_workspace(workspace.clone());
        Ok(())
    });
    if let Err(error) = recorded {
        // Nothing refers to the checkout: undo it rather than leave an orphan.
        let _ = git(&repo, &["worktree", "remove", "--force", "--", &worktree]);
        if !existing {
            let _ = git(&repo, &["branch", "-D", "--", &branch]);
        }
        return Err(error);
    }
    if let Err(error) = launch(host, &workspace, &conversation.id, Some(prompt)) {
        return Err(format!(
            "workspace {} was created but Claude did not start: {error}. send_message to conversation {} retries; close_workspace removes it",
            workspace.id, conversation.id
        ));
    }
    Ok(describe(host, &workspace))
}

fn conversation_open(host: &Arc<Host>, params: &Value) -> Result<Value, String> {
    let id = text(params, "workspace")?;
    let prompt = text(params, "prompt")?;
    let conversation = Conversation {
        id: uuid()?,
        title: title_from(prompt),
        created_at: now(),
        permission: "auto".into(),
    };
    let workspace = host.registry.update(|registry| {
        let workspace = registry.workspace_mut(id).ok_or("unknown_workspace")?;
        if workspace.closed {
            return Err("workspace_closed".into());
        }
        workspace.conversations.push(conversation.clone());
        Ok(workspace.clone())
    })?;
    if let Err(error) = launch(host, &workspace, &conversation.id, Some(prompt)) {
        let _ = host.registry.update(|registry| {
            if let Some(workspace) = registry.workspace_mut(id) {
                workspace.conversations.retain(|c| c.id != conversation.id);
            }
            Ok(())
        });
        return Err(error);
    }
    let mut value = json!(conversation);
    value["workspace"] = json!(workspace.id);
    Ok(value)
}

fn find_conversation(host: &Host, id: &str) -> Result<(Workspace, Conversation), String> {
    let registry = host.registry.read()?;
    registry
        .conversation(id)
        .map(|(w, c)| (w.clone(), c.clone()))
        .ok_or_else(|| "unknown_conversation".to_string())
}

/// The offset just past the next newline at or after `from`, read in bounded chunks.
fn next_line(file: &mut std::fs::File, from: u64) -> Result<Option<u64>, String> {
    file.seek(SeekFrom::Start(from))
        .map_err(|e| e.to_string())?;
    let mut chunk = vec![0u8; 1_048_576];
    let mut at = from;
    loop {
        let read = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if read == 0 {
            return Ok(None);
        }
        if let Some(position) = chunk[..read].iter().position(|b| *b == b'\n') {
            return Ok(Some(at + position as u64 + 1));
        }
        at += read as u64;
    }
}

/// Messages after `cursor` (a byte offset from a previous read), or the last `limit` messages when
/// no cursor is given. The reply's `cursor` continues from where it stopped.
fn conversation_read(host: &Host, params: &Value) -> Result<Value, String> {
    let id = text(params, "id")?;
    let (workspace, _) = find_conversation(host, id)?;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .map(|l| l.clamp(1, READ_LIMIT as u64) as usize)
        .unwrap_or(50);
    let cursor = params.get("cursor").and_then(Value::as_u64);
    let activity = host.activity(id);
    let state = json!({"state": activity.state, "state_at": activity.at, "notification": activity.notification});
    let path = transcript_path(&workspace.worktree, id)?;
    let mut read = match std::fs::File::open(&path) {
        Ok(mut file) => read_transcript(&mut file, cursor, limit, TAIL_BYTES, REPLY_BUDGET)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            json!({"messages": [], "cursor": 0, "more": false})
        }
        Err(e) => return Err(e.to_string()),
    };
    read["status"] = state;
    Ok(read)
}

/// Read at most `window` bytes of a transcript from `cursor`, or its tail without a cursor.
fn read_transcript(
    file: &mut std::fs::File,
    cursor: Option<u64>,
    limit: usize,
    window: u64,
    budget: usize,
) -> Result<Value, String> {
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let (start, tail) = match cursor {
        Some(cursor) => (cursor.min(size), false),
        None => (size.saturating_sub(window), true),
    };
    file.seek(SeekFrom::Start(start))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    (&mut *file)
        .take(window)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    // A tail read can start inside a line; skip to the next one.
    let mut offset = 0;
    if tail && start > 0 {
        offset = bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |p| p + 1);
    }
    let mut messages: Vec<Value> = Vec::new();
    let mut used = 0;
    let mut consumed = offset;
    let mut more = false;
    while let Some(end) = bytes[consumed..].iter().position(|b| *b == b'\n') {
        if !tail && messages.len() >= limit {
            more = true;
            break;
        }
        let line = &bytes[consumed..consumed + end];
        let found = serde_json::from_slice::<Value>(line)
            .map(|value| prometeu_host::transcript::messages(&value))
            .unwrap_or_default();
        let found_size: usize = found.iter().map(|m| m.to_string().len()).sum();
        // A page always fits in one reply. The next page starts at this record, unless the
        // record alone is too large: then it is cut, and the rest is reported as omitted.
        if !tail && used + found_size > budget && !messages.is_empty() {
            more = true;
            break;
        }
        consumed += end + 1;
        if !tail && found_size > budget {
            let total = found.len();
            for message in found {
                let length = message.to_string().len();
                if used + length > budget {
                    break;
                }
                used += length;
                messages.push(message);
            }
            let omitted = total - messages.len();
            messages.push(json!({"role": "system", "kind": "skipped",
                "text": format!("{omitted} message(s) of one transcript record were omitted")}));
            continue;
        }
        used += found_size;
        messages.extend(found);
    }
    let mut next = start + consumed as u64;
    // A single record longer than the window would otherwise pin the cursor forever: skip it and
    // say so.
    if !tail && consumed == 0 && bytes.len() as u64 == window {
        if let Some(after) = next_line(file, start + window)? {
            messages.push(json!({"role": "system", "kind": "skipped",
                "text": format!("a transcript record of {} bytes was skipped", after - start)}));
            next = after;
        }
    }
    if tail {
        if messages.len() > limit {
            messages.drain(..messages.len() - limit);
        }
        let mut used: usize = messages.iter().map(|m| m.to_string().len()).sum();
        while used > budget && !messages.is_empty() {
            used -= messages.remove(0).to_string().len();
        }
    }
    if next < size && !tail {
        more = true;
    }
    Ok(json!({"messages": messages, "cursor": next, "more": more}))
}

/// Type a message into the conversation, as a person pasting it and pressing Enter. Claude queues
/// it when a turn is running. An ended conversation is resumed with the message as its prompt.
fn conversation_send(host: &Arc<Host>, id: &str, message: &str) -> Result<Value, String> {
    let (workspace, conversation) = find_conversation(host, id)?;
    if workspace.closed || lock(&CLOSING).contains(&workspace.id) {
        return Err("workspace_closed".into());
    }
    if !host.is_running(id) {
        // The host only launches without permission prompts; a conversation the person runs with
        // prompts must be resumed where they can be answered.
        if conversation.permission != "auto" {
            return Err(
                "conversation_ended: it asks for permissions, so resume it from the desktop".into(),
            );
        }
        launch(host, &workspace, id, Some(message))?;
        return Ok(json!({"resumed": true}));
    }
    let clean = message.replace("\x1b[201~", "");
    host.submit(id, format!("\x1b[200~{clean}\x1b[201~").as_bytes())?;
    Ok(json!({"resumed": false}))
}

/// What closing would lose: uncommitted changes, and commits that only the workspace's branch
/// holds (not on the base nor on any remote), which deleting the branch would drop.
fn losses(workspace: &Workspace) -> Result<Option<String>, String> {
    if !Path::new(&workspace.worktree).exists() {
        return Ok(None);
    }
    let status = git(&workspace.worktree, &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Ok(Some(format!("uncommitted_changes\n{status}")));
    }
    if workspace.keep_branch {
        return Ok(None);
    }
    // The base names a ref of the main checkout (`HEAD` there is not the worktree's HEAD).
    let base = git(
        &workspace.project,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", workspace.base),
        ],
    );
    let unique = base
        .and_then(|base| {
            git(
                &workspace.worktree,
                &["rev-list", "--count", "HEAD", "--not", "--remotes", &base],
            )
        })
        .ok()
        .and_then(|count| count.parse::<u64>().ok());
    match unique {
        Some(0) => Ok(None),
        Some(count) => Ok(Some(format!(
            "unpushed_commits: {count} commit(s) exist only on {}; push them or pass force",
            workspace.branch
        ))),
        None => Ok(Some(format!(
            "unpushed_commits: could not compare {} with its base; push it or pass force",
            workspace.branch
        ))),
    }
}

fn workspace_close(host: &Arc<Host>, id: &str, force: bool) -> Result<Value, String> {
    {
        let _launching = lock(&LAUNCHES);
        let mut closing = lock(&CLOSING);
        if closing.iter().any(|c| c == id) {
            return Err("workspace_closing".into());
        }
        closing.push(id.to_string());
    }
    let result = close(host, id, force);
    lock(&CLOSING).retain(|c| c != id);
    result
}

fn close(host: &Arc<Host>, id: &str, force: bool) -> Result<Value, String> {
    // Read after registering as closing: every conversation launched before is recorded here.
    let workspace = host
        .registry
        .read()?
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or("unknown_workspace")?;
    if workspace.closed {
        return Ok(json!({}));
    }
    let separate = workspace.worktree != workspace.project;
    if separate && !force {
        if let Some(loss) = losses(&workspace)? {
            return Err(loss);
        }
    }
    for conversation in &workspace.conversations {
        host.end(&conversation.id);
    }
    if separate && !force {
        // Claude may have written between the first check and its exit.
        if let Some(loss) = losses(&workspace)? {
            return Err(format!(
                "{loss}\nThe conversations were stopped; send_message resumes them."
            ));
        }
    }
    for conversation in &workspace.conversations {
        host.forget(&conversation.id);
    }
    if separate {
        let mut remove = vec!["worktree", "remove"];
        if force {
            remove.push("--force");
        }
        remove.extend(["--", workspace.worktree.as_str()]);
        if let Err(error) = git(&workspace.project, &remove) {
            if Path::new(&workspace.worktree).exists() {
                return Err(error);
            }
            git(&workspace.project, &["worktree", "prune"])?;
        }
        if !workspace.keep_branch {
            let _ = git(
                &workspace.project,
                &["branch", "-D", "--", &workspace.branch],
            );
        }
    }
    host.registry.update(|registry| {
        if let Some(workspace) = registry.workspace_mut(id) {
            workspace.closed = true;
        }
        Ok(())
    })?;
    Ok(json!({}))
}

// Desktop methods on the private socket.

pub fn registry_view(host: &Host) -> Result<Value, String> {
    let registry = host.registry.read()?;
    let mut value = json!(registry);
    value["workspaces"] = Value::Array(
        registry
            .workspaces
            .iter()
            .map(|w| describe(host, w))
            .collect(),
    );
    Ok(value)
}

pub fn registry_put(host: &Host, params: &Value) -> Result<Value, String> {
    let project = params
        .get("project")
        .filter(|p| !p.is_null())
        .map(|p| serde_json::from_value::<Project>(p.clone()))
        .transpose()
        .map_err(|e| e.to_string())?;
    let workspace = params
        .get("workspace")
        .filter(|w| !w.is_null())
        .map(|w| serde_json::from_value::<Workspace>(w.clone()))
        .transpose()
        .map_err(|e| e.to_string())?;
    host.registry.update(|registry| {
        if let Some(project) = project {
            registry.add_project(project);
        }
        if let Some(workspace) = workspace {
            registry.put_workspace(workspace);
        }
        Ok(json!({}))
    })
}

pub fn registry_close(host: &Host, params: &Value) -> Result<Value, String> {
    let id = text(params, "id")?;
    host.registry.update(|registry| {
        if let Some(workspace) = registry.workspace_mut(id) {
            workspace.closed = true;
        }
        Ok(json!({}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"ab"));
        assert!(!same(b"", b"a"));
    }

    #[test]
    fn uuids_are_version_four() {
        let id = uuid().unwrap();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(prometeu_host::valid_id(&id));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn binding_replaces_only_a_stale_socket() {
        let dir = scratch("bind");
        let file = dir.join("taken");
        std::fs::write(&file, "keep").unwrap();
        assert!(bind(&file).is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep");
        let socket = dir.join("s.sock");
        let live = bind(&socket).unwrap();
        assert!(bind(&socket).is_err(), "a served socket is not replaced");
        drop(live);
        assert!(bind(&socket).is_ok(), "a stale socket is replaced");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn slots_are_bounded_and_returned() {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let first = Slot::take(&COUNTER, 1).unwrap();
        assert!(Slot::take(&COUNTER, 1).is_none());
        drop(first);
        assert!(Slot::take(&COUNTER, 1).is_some());
    }

    #[test]
    fn an_oversized_record_is_skipped_instead_of_pinning_the_cursor() {
        let dir = scratch("read");
        let path = dir.join("t.jsonl");
        let small = r#"{"type":"user","message":{"content":"after"}}"#;
        let huge = format!(
            r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
            "x".repeat(300)
        );
        std::fs::write(&path, format!("{huge}\n{small}\n")).unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        let first = read_transcript(&mut file, Some(0), 50, 64, REPLY_BUDGET).unwrap();
        assert_eq!(first["messages"][0]["kind"], "skipped");
        assert_eq!(first["cursor"], huge.len() as u64 + 1);
        let second =
            read_transcript(&mut file, first["cursor"].as_u64(), 50, 64, REPLY_BUDGET).unwrap();
        assert_eq!(second["messages"][0]["text"], "after");
        assert_eq!(second["more"], false);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pages_fit_the_reply_budget_and_cut_records_that_cannot() {
        let dir = scratch("budget");
        let path = dir.join("t.jsonl");
        let text = |t: &str| format!(r#"{{"type":"user","message":{{"content":"{t}"}}}}"#);
        let blocks: Vec<String> = (0..5)
            .map(|i| format!(r#"{{"type":"text","text":"{}{i}"}}"#, "y".repeat(40)))
            .collect();
        let wide = format!(
            r#"{{"type":"assistant","message":{{"content":[{}]}}}}"#,
            blocks.join(",")
        );
        std::fs::write(
            &path,
            format!(
                "{}\n{}\n{wide}\n",
                text(&"a".repeat(40)),
                text(&"b".repeat(40))
            ),
        )
        .unwrap();
        let mut file = std::fs::File::open(&path).unwrap();
        let read = |file: &mut std::fs::File, cursor| {
            read_transcript(file, Some(cursor), 50, 1_048_576, 200).unwrap()
        };
        // Two small records fit; the next page starts at the wide one.
        let first = read(&mut file, 0);
        assert_eq!(first["messages"].as_array().unwrap().len(), 2);
        assert_eq!(first["more"], true);
        // Alone, the wide record does not fit: what fits is kept and the rest reported.
        let second = read(&mut file, first["cursor"].as_u64().unwrap());
        let messages = second["messages"].as_array().unwrap();
        assert_eq!(messages.last().unwrap()["kind"], "skipped");
        assert!(messages.len() < 6);
        assert_eq!(second["more"], false);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(title_from("\n  fix login\nmore"), "fix login");
        assert_eq!(title_from(&"a".repeat(70)).chars().count(), 61);
    }
}
