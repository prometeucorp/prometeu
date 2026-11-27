//! Native Codex conversations owned by the remote host. The TUI owns input. A checked RPC
//! connection observes the same engine as the desktop and keeps a compact MCP transcript.
use crate::registry::{Conversation, Provider, Workspace};
use crate::server::{self, Host, Reservation};
use prometeu_core::command::{CommandPolicy, CommandRunner, OutputPolicy};
use prometeu_core::lock::lock;
use prometeu_core::supervision::Observation;
use prometeu_host::codex::{engine_terminal, socket_path, CONNECT_TIMEOUT};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

const HISTORY_LIMIT: usize = 64 * 1_048_576;

fn program() -> String {
    std::env::var("PROMETEU_HOST_CODEX").unwrap_or_else(|_| "codex".into())
}

pub(crate) fn available() -> Result<(), String> {
    for args in [vec!["--version"], vec!["login", "status"]] {
        let result = prometeu_process::command::UnixCommandRunner
            .run(
                Command::new(program()).args(&args),
                &[],
                CommandPolicy {
                    timeout: Duration::from_secs(20),
                    stdout: OutputPolicy::Capture { limit: 4096 },
                    stderr: OutputPolicy::Capture { limit: 4096 },
                },
            )
            .map_err(|_| "codex_unavailable: install Codex 0.169.0 or newer on this machine")?;
        if args[0] == "--version" {
            if !result.success
                || !prometeu_protocols::cli_version::at_least(
                    &String::from_utf8_lossy(&result.stdout),
                    "0.169.0",
                )
            {
                return Err(
                    "codex_unavailable: install Codex 0.169.0 or newer on this machine".into(),
                );
            }
        } else if !result.success {
            return Err("codex_signed_out: run codex login --device-auth on this machine".into());
        }
    }
    Ok(())
}

struct Rpc {
    socket: WebSocket<UnixStream>,
    next: u64,
    pending: VecDeque<Value>,
    pending_bytes: usize,
}

impl Rpc {
    fn connect(path: &Path, pid: u32, deadline: Instant) -> Result<Self, String> {
        let socket = prometeu_host::codex::client(path, pid, deadline)?;
        let mut client = Self {
            socket,
            next: 0,
            pending: VecDeque::new(),
            pending_bytes: 0,
        };
        client.call("initialize", json!({
            "clientInfo":{"name":"prometeu-native","title":"Prometeu","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }), deadline)?;
        client.send(json!({"method":"initialized","params":{}}))?;
        Ok(client)
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        self.socket
            .send(Message::Text(message.to_string().into()))
            .map_err(|_| "codex_rpc_write_failed".into())
    }

    fn read(&mut self) -> Result<Option<Value>, String> {
        match self.socket.read() {
            Ok(Message::Text(text)) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|_| "codex_rpc_invalid_json".into()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {
                self.socket.flush().map_err(|_| "codex_rpc_closed")?;
                Ok(None)
            }
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            _ => Err("codex_rpc_closed".into()),
        }
    }

    fn call(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, String> {
        self.next += 1;
        let id = self.next;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        while Instant::now() < deadline {
            let Some(frame) = self.read()? else { continue };
            if frame.get("method").is_none() && frame["id"] == id {
                return frame
                    .get("result")
                    .cloned()
                    .ok_or_else(|| format!("codex_rpc_failed: {method}"));
            }
            self.pending_bytes += frame.to_string().len();
            if self.pending_bytes > HISTORY_LIMIT || self.pending.len() >= 1024 {
                return Err("codex_rpc_overflow".into());
            }
            self.pending.push_back(frame);
        }
        Err(format!("codex_rpc_timeout: {method}"))
    }

    fn next(&mut self) -> Result<Option<Value>, String> {
        if let Some(frame) = self.pending.pop_front() {
            self.pending_bytes -= frame.to_string().len();
            Ok(Some(frame))
        } else {
            self.read()
        }
    }
}

fn identity(reply: &Value, cwd: &str, expected: Option<&str>) -> Result<String, String> {
    let thread = reply["thread"]["id"]
        .as_str()
        .filter(|id| prometeu_host::valid_id(id))
        .ok_or("codex_thread_identity_mismatch")?;
    if reply["cwd"].as_str() != Some(cwd) || expected.is_some_and(|id| id != thread) {
        return Err("codex_thread_identity_mismatch".into());
    }
    Ok(thread.into())
}

fn bootstrap(
    client: &mut Rpc,
    workspace: &Workspace,
    conversation: &Conversation,
    deadline: Instant,
) -> Result<Conversation, String> {
    let mut params =
        json!({"cwd":workspace.worktree,"approvalPolicy":"never","sandbox":"danger-full-access"});
    if !conversation.model.is_empty() {
        params["model"] = json!(conversation.model);
    }
    let method = if let Some(thread) = &conversation.thread_id {
        params["threadId"] = json!(thread);
        params["excludeTurns"] = json!(true);
        "thread/resume"
    } else {
        params["historyMode"] = json!("paginated");
        "thread/start"
    };
    let opened = client.call(method, params, deadline)?;
    let thread = identity(
        &opened,
        &workspace.worktree,
        conversation.thread_id.as_deref(),
    )?;
    if conversation.thread_id.is_none() {
        client.call(
            "thread/name/set",
            json!({"threadId":thread,"name":"Prometeu"}),
            deadline,
        )?;
    }
    let model = opened["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("codex_model_missing")?;
    if !conversation.model.is_empty() && model != conversation.model {
        return Err("codex_model_mismatch".into());
    }
    let effort = if conversation.effort.is_empty() {
        opened["reasoningEffort"].clone()
    } else {
        json!(conversation.effort)
    };
    let expected = json!({"threadId":thread,"approvalPolicy":"never",
        "sandboxPolicy":{"type":"dangerFullAccess"}, "effort":effort,
        "collaborationMode":{"mode":"default","settings":{"model":model,"reasoning_effort":effort,"developer_instructions":null}}});
    client.call("thread/settings/update", expected.clone(), deadline)?;
    let verified = client.call(
        "thread/resume",
        json!({"threadId":thread,"excludeTurns":true}),
        deadline,
    )?;
    identity(&verified, &workspace.worktree, Some(&thread))?;
    if verified["approvalPolicy"] != expected["approvalPolicy"]
        || verified["sandbox"] != expected["sandboxPolicy"]
        || verified["reasoningEffort"] != effort
        || verified["collaborationMode"] != expected["collaborationMode"]
    {
        return Err("codex_settings_mismatch".into());
    }
    let mut bound = conversation.clone();
    bound.thread_id = Some(thread);
    bound.model = model.into();
    bound.effort = effort.as_str().unwrap_or_default().into();
    Ok(bound)
}

pub(crate) fn launch(
    host: &Arc<Host>,
    workspace: &Workspace,
    conversation: &Conversation,
    prompt: Option<&str>,
    reserved: &Reservation<'_>,
) -> Result<(), String> {
    available()?;
    let engine = engine_terminal(&conversation.id);
    retire(host, &conversation.id)?;
    let directory = host.root.join("sessions").join(&conversation.id);
    crate::agents::private_dir(&directory)?;
    let socket = socket_path(&host.root.display().to_string(), &conversation.id);
    let opened = server::open(
        host,
        &json!({"id":engine,"workspace":workspace.id,"cwd":workspace.worktree,
        "argv":[program(),"app-server","--listen",format!("unix://{socket}")],"cols":80,"rows":24}),
    )?;
    let pid = opened["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or("codex_engine_pid_missing")?;
    let result = (|| {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut client = Rpc::connect(Path::new(&socket), pid, deadline)?;
        let bound = bootstrap(&mut client, workspace, conversation, deadline)?;
        host.registry.update(|registry| {
            let current = registry
                .workspace_mut(&workspace.id)
                .ok_or("unknown_workspace")?
                .conversations
                .iter_mut()
                .find(|c| c.id == conversation.id)
                .ok_or("unknown_conversation")?;
            if current.thread_id.is_some() && current.thread_id != bound.thread_id {
                return Err("codex_thread_identity_mismatch".into());
            }
            current.thread_id = bound.thread_id.clone();
            current.model = bound.model.clone();
            current.effort = bound.effort.clone();
            Ok(())
        })?;
        let mut journal = Journal::open(host, &conversation.id)?;
        let initial = history(
            &mut client,
            &bound,
            &workspace.worktree,
            &mut journal,
            deadline,
        )?;
        let mut argv = vec![
            program(),
            "--remote".into(),
            format!("unix://{socket}"),
            "resume".into(),
            bound.thread_id.clone().unwrap(),
        ];
        if let Some(prompt) = prompt {
            argv.extend(["--".into(), prompt.into()]);
        }
        server::open_reserved(
            host,
            &json!({"id":conversation.id,"workspace":workspace.id,"cwd":workspace.worktree,
            "argv":argv,"cols":120,"rows":40,"env":{"TERM_PROGRAM":"Prometeu"}}),
            reserved,
        )?;
        watch(
            host,
            workspace.clone(),
            bound,
            pid,
            Some((client, journal, initial)),
        );
        Ok(())
    })();
    if result.is_err() {
        host.end(&engine);
        host.forget(&engine);
    }
    result
}

/// Wait for the observer to drain native history and retire its engine before reuse or removal.
pub(crate) fn retire(host: &Host, id: &str) -> Result<(), String> {
    let deadline = Instant::now() + CONNECT_TIMEOUT + Duration::from_secs(6);
    while lock(&host.codex_watch).contains_key(id) {
        if Instant::now() >= deadline {
            return Err("codex_previous_execution_stopping".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

pub(crate) fn transcript_path(host: &Host, id: &str) -> PathBuf {
    host.root.join("codex").join(format!("{id}.jsonl"))
}

struct Journal {
    file: std::fs::File,
    seen: HashSet<String>,
}

impl Journal {
    fn open(host: &Host, id: &str) -> Result<Self, String> {
        crate::agents::private_dir(&host.root.join("codex"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .mode(0o600)
            .open(transcript_path(host, id))
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        while unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            if Instant::now() >= deadline {
                return Err("codex_journal_busy".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(HISTORY_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > HISTORY_LIMIT {
            return Err("codex_history_limit".into());
        }
        // A crash can leave only the final append incomplete. Replay restores that occurrence.
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        file.set_len(complete as u64).map_err(|e| e.to_string())?;
        let mut seen = HashSet::new();
        for line in bytes[..complete]
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
        {
            let entry: Value = serde_json::from_slice(line).map_err(|_| "codex_journal_invalid")?;
            seen.insert(
                entry["id"]
                    .as_str()
                    .ok_or("codex_journal_invalid")?
                    .to_owned(),
            );
        }
        Ok(Self { file, seen })
    }

    fn item(&mut self, thread: &str, turn: &str, item: &Value, cwd: &str) -> Result<(), String> {
        let item_id = item["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("codex_item_identity_missing")?;
        let id = format!("{thread}:{turn}:{item_id}");
        if self.seen.contains(&id) {
            return Ok(());
        }
        let messages = prometeu_host::transcript::codex_item(item, cwd);
        let entry = json!({"type":"codex","id":id,"messages":messages});
        writeln!(self.file, "{entry}")
            .and_then(|_| self.file.sync_data())
            .map_err(|e| e.to_string())?;
        self.seen.insert(id);
        Ok(())
    }
}

fn history(
    client: &mut Rpc,
    conversation: &Conversation,
    cwd: &str,
    journal: &mut Journal,
    deadline: Instant,
) -> Result<Vec<Value>, String> {
    let thread = conversation
        .thread_id
        .as_deref()
        .ok_or("codex_thread_identity_missing")?;
    let mut cursor = Value::Null;
    let mut seen = HashSet::new();
    let mut bytes = 0;
    let mut turns = Vec::new();
    loop {
        let page = client.call(
            "thread/turns/list",
            json!({"threadId":thread,"cursor":cursor,"limit":16,
            "sortDirection":"asc","itemsView":"full"}),
            deadline,
        )?;
        bytes += page.to_string().len();
        if bytes > HISTORY_LIMIT {
            return Err("codex_history_limit".into());
        }
        for turn in page["data"].as_array().ok_or("codex_history_invalid")? {
            let id = turn["id"].as_str().ok_or("codex_history_invalid")?;
            if !seen.insert(id.to_owned()) || seen.len() > 10_000 {
                return Err("codex_history_invalid".into());
            }
            for item in turn["items"].as_array().ok_or("codex_history_invalid")? {
                if complete(item, turn) {
                    journal.item(thread, id, item, cwd)?;
                }
            }
            turns.push(turn.clone());
        }
        match page.get("nextCursor") {
            Some(Value::Null) => return Ok(turns),
            Some(Value::String(next)) if !next.is_empty() && cursor.as_str() != Some(next) => {
                cursor = json!(next)
            }
            _ => return Err("codex_history_invalid".into()),
        }
    }
}

fn complete(item: &Value, turn: &Value) -> bool {
    if item["type"] == "userMessage" {
        return true;
    }
    match item["status"].as_str() {
        Some("completed" | "failed" | "interrupted" | "declined" | "cancelled") => true,
        Some(_) => false,
        None => {
            item["completedAtMs"].is_u64()
                || matches!(
                    turn["status"].as_str(),
                    Some("completed" | "failed" | "interrupted")
                )
        }
    }
}

pub(crate) fn watch_registered(host: &Arc<Host>) {
    let Ok(registry) = host.registry.read() else {
        return;
    };
    for workspace in registry.workspaces {
        for conversation in &workspace.conversations {
            if conversation.provider != Provider::Codex
                || conversation.thread_id.is_none()
                || !host.is_running(&conversation.id)
            {
                continue;
            }
            if let Some(pid) = host.terminal_pid(&engine_terminal(&conversation.id)) {
                watch(host, workspace.clone(), conversation.clone(), pid, None);
            }
        }
    }
}

fn watch(
    host: &Arc<Host>,
    workspace: Workspace,
    conversation: Conversation,
    pid: u32,
    mut linked: Option<(Rpc, Journal, Vec<Value>)>,
) {
    let mut watches = lock(&host.codex_watch);
    if watches.get(&conversation.id) == Some(&pid) {
        return;
    }
    watches.insert(conversation.id.clone(), pid);
    drop(watches);
    let host = host.clone();
    std::thread::spawn(move || {
        let id = &conversation.id;
        let engine = engine_terminal(id);
        let mut observer = Observer::new(&conversation, &workspace.worktree);
        while host.terminal_pid(&engine) == Some(pid) {
            let result = (|| {
                let (mut client, mut journal, initial) = match linked.take() {
                    Some(pair) => pair,
                    None => {
                        let deadline = Instant::now() + CONNECT_TIMEOUT;
                        let socket = socket_path(&host.root.display().to_string(), id);
                        let mut client = Rpc::connect(Path::new(&socket), pid, deadline)?;
                        let reply = client.call(
                            "thread/resume",
                            json!({"threadId":conversation.thread_id,"excludeTurns":true}),
                            deadline,
                        )?;
                        identity(
                            &reply,
                            &workspace.worktree,
                            conversation.thread_id.as_deref(),
                        )?;
                        let mut journal = Journal::open(&host, id)?;
                        let initial = history(
                            &mut client,
                            &conversation,
                            &workspace.worktree,
                            &mut journal,
                            deadline,
                        )?;
                        observer.settings(&host, pid, &reply["collaborationMode"]["settings"])?;
                        if let Some(state) = state(&reply["thread"]["status"]) {
                            host.codex_activity(id, pid, state, None);
                        }
                        (client, journal, initial)
                    }
                };
                observer.replay(&host, pid, &initial)?;
                observer.reconcile_children(&mut client, &host, pid, &mut journal)?;
                while host.is_running(id) && host.terminal_pid(&engine) == Some(pid) {
                    if let Some(frame) = client.next()? {
                        observer.live(&host, pid, &mut journal, &frame)?;
                    }
                }
                // A TUI exit is not an RPC drain barrier. Read the native snapshot before
                // ending the engine, so ended conversations retain their final messages.
                if host.terminal_pid(&engine) == Some(pid) {
                    let final_turns = history(
                        &mut client,
                        &conversation,
                        &workspace.worktree,
                        &mut journal,
                        Instant::now() + CONNECT_TIMEOUT,
                    )?;
                    observer.replay(&host, pid, &final_turns)?;
                    observer.reconcile_children(&mut client, &host, pid, &mut journal)?;
                }
                Ok::<_, String>(())
            })();
            if !host.is_running(id) {
                break;
            }
            if result.is_err() {
                host.codex_activity(
                    id,
                    pid,
                    "starting",
                    Some("Codex observation unavailable; reconnecting".into()),
                );
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        let _ = host.codex_observe(id, pid, Observation::Exited { success: false });
        // Stop only this engine. A concurrent resume can already own the same terminal name.
        host.end_pid(&engine, pid);
        let mut watches = lock(&host.codex_watch);
        if watches.get(id) == Some(&pid) {
            watches.remove(id);
        }
    });
}

fn state(status: &Value) -> Option<&'static str> {
    match status["type"].as_str() {
        Some("idle") => Some("idle"),
        Some("active") => Some(
            if status["activeFlags"].as_array().is_some_and(|flags| {
                flags
                    .iter()
                    .any(|f| matches!(f.as_str(), Some("waitingOnApproval" | "waitingOnUserInput")))
            }) {
                "permission"
            } else {
                "working"
            },
        ),
        Some("systemError") => Some("idle"),
        _ => None,
    }
}

/// Native turn IDs distinguish recovery from new work. The decoder survives reconnects, and a
/// snapshot seeds its child state without publishing historical execution completions.
struct Observer {
    id: String,
    thread: String,
    cwd: String,
    decoder: prometeu_protocols::codex::Link,
    current: Option<String>,
    ended: HashSet<String>,
    background: HashSet<String>,
    primed: bool,
}

impl Observer {
    fn new(conversation: &Conversation, cwd: &str) -> Self {
        let thread = conversation.thread_id.clone().unwrap_or_default();
        Self {
            id: conversation.id.clone(),
            decoder: Self::decoder(&thread, cwd),
            thread,
            cwd: cwd.into(),
            current: None,
            ended: HashSet::new(),
            background: HashSet::new(),
            primed: false,
        }
    }

    fn decoder(thread: &str, cwd: &str) -> prometeu_protocols::codex::Link {
        prometeu_protocols::codex::Link::attached(
            Box::new(std::io::sink()),
            prometeu_protocols::codex::Start {
                permission: None,
                instructions: String::new(),
                cwd: cwd.into(),
                resume: Some(thread.into()),
                model: String::new(),
                effort: String::new(),
                plugin_ids: Vec::new(),
                plugin_hook_ids: Vec::new(),
            },
            thread.into(),
            "host-supervision".into(),
            |_, english| english.into(),
        )
    }

    fn children(
        decoder: &mut prometeu_protocols::codex::Link,
        frame: &Value,
    ) -> Option<HashSet<String>> {
        decoder
            .on_line(&frame.to_string())
            .into_iter()
            .find(|event| event["type"] == "background.changed")
            .map(|event| {
                event["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|task| task["id"].as_str().map(str::to_owned))
                    .collect()
            })
    }

    fn background(&mut self, host: &Host, pid: u32, next: HashSet<String>) -> Result<(), String> {
        for child in next.difference(&self.background) {
            host.codex_observe(&self.id, pid, Observation::BackgroundStarted(child.clone()))?;
        }
        for child in self.background.difference(&next) {
            host.codex_observe(
                &self.id,
                pid,
                Observation::BackgroundFinished(child.clone()),
            )?;
        }
        self.background = next;
        Ok(())
    }

    fn start(&mut self, host: &Host, pid: u32, turn: &str) -> Result<(), String> {
        if self.current.as_deref() != Some(turn) && !self.ended.contains(turn) {
            host.codex_observe(&self.id, pid, Observation::Started)?;
            self.current = Some(turn.into());
        }
        Ok(())
    }

    fn finish(&mut self, host: &Host, pid: u32, turn: &str, status: &str) -> Result<(), String> {
        if self.current.as_deref() != Some(turn) || self.ended.contains(turn) {
            return Ok(());
        }
        let observation = match status {
            "completed" => Observation::Completed,
            "failed" => Observation::Failed,
            "interrupted" => Observation::Interrupted,
            _ => return Ok(()),
        };
        host.codex_observe(&self.id, pid, observation)?;
        self.ended.insert(turn.into());
        if status != "completed" {
            self.background.clear();
        }
        Ok(())
    }

    fn replay(&mut self, host: &Host, pid: u32, turns: &[Value]) -> Result<(), String> {
        let mut background = self.background.clone();
        let mut snapshots = Vec::new();
        for turn in turns {
            let id = turn["id"].as_str().ok_or("codex_history_invalid")?;
            for item in turn["items"].as_array().ok_or("codex_history_invalid")? {
                if !complete(item, turn) {
                    continue;
                }
                if let Some(children) = Self::children(
                    &mut self.decoder,
                    &json!({"method":"item/completed",
                    "params":{"threadId":self.thread,"turnId":id,"item":item}}),
                ) {
                    background = children;
                }
            }
            let status = turn["status"].as_str().ok_or("codex_history_invalid")?;
            if self.primed && !self.ended.contains(id) {
                snapshots.push((id.to_owned(), status.to_owned(), background.clone()));
            } else if matches!(status, "completed" | "failed" | "interrupted") {
                self.ended.insert(id.into());
            }
        }
        if !self.primed {
            if let Some(turn) = turns.last() {
                let id = turn["id"].as_str().ok_or("codex_history_invalid")?;
                let status = turn["status"].as_str().ok_or("codex_history_invalid")?;
                if status == "inProgress" {
                    self.ended.remove(id);
                    snapshots.push((id.into(), status.into(), background.clone()));
                }
            }
        }
        for (id, status, mut children) in snapshots {
            self.start(host, pid, &id)?;
            children.extend(self.background.iter().cloned());
            self.background(host, pid, children)?;
            self.finish(host, pid, &id, &status)?;
        }
        // A child can finish after the root turn. Reconcile its final native state even when
        // the root completion was already observed before the connection stopped.
        if self.current.is_some() {
            background.extend(self.background.iter().cloned());
            self.background(host, pid, background)?;
        }
        self.primed = true;
        Ok(())
    }

    /// Root history can omit a child's final notification. Query every known child instead of
    /// treating absence from history as completion. Unknown state keeps the execution incomplete.
    fn reconcile_children(
        &mut self,
        client: &mut Rpc,
        host: &Host,
        pid: u32,
        journal: &mut Journal,
    ) -> Result<(), String> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        let children: Vec<String> = self.background.iter().cloned().collect();
        if children.len() > 1024 {
            return Err("codex_child_state_limit".into());
        }
        for child in children {
            let reply = client.call(
                "thread/resume",
                json!({"threadId":child,"excludeTurns":true}),
                deadline,
            )?;
            if reply["thread"]["id"] != child
                || !matches!(
                    reply["thread"]["status"]["type"].as_str(),
                    Some("active" | "idle" | "notLoaded" | "systemError")
                )
            {
                return Err("codex_child_state_unknown".into());
            }
            // These notifications precede the correlated status snapshot.
            while let Some(frame) = client.pending.pop_front() {
                client.pending_bytes -= frame.to_string().len();
                self.live(host, pid, journal, &frame)?;
            }
            if let Some(children) = Self::children(
                &mut self.decoder,
                &json!({
                    "method":"thread/status/changed","params":{"threadId":child,"status":reply["thread"]["status"]}
                }),
            ) {
                self.background(host, pid, children)?;
            }
        }
        Ok(())
    }

    fn settings(&self, host: &Host, pid: u32, settings: &Value) -> Result<(), String> {
        let watches = lock(&host.codex_watch);
        if watches.get(&self.id) != Some(&pid) {
            return Ok(());
        }
        let Some(model) = settings["model"].as_str().filter(|model| !model.is_empty()) else {
            return Ok(());
        };
        let effort = match &settings["reasoning_effort"] {
            Value::String(effort) => effort.as_str(),
            Value::Null => "",
            _ => return Err("codex_settings_mismatch".into()),
        };
        let changed = host.registry.update(|registry| {
            let Some((workspace, _)) = registry.conversation(&self.id) else {
                return Ok(false);
            };
            let workspace = workspace.id.clone();
            let current = registry
                .workspace_mut(&workspace)
                .unwrap()
                .conversations
                .iter_mut()
                .find(|conversation| conversation.id == self.id)
                .unwrap();
            if current.provider != Provider::Codex
                || current.thread_id.as_deref() != Some(&self.thread)
            {
                return Err("codex_thread_identity_mismatch".into());
            }
            let changed = current.model != model || current.effort != effort;
            current.model = model.into();
            current.effort = effort.into();
            Ok(changed)
        })?;
        if changed {
            host.announce_registry();
        }
        Ok(())
    }

    fn live(
        &mut self,
        host: &Host,
        pid: u32,
        journal: &mut Journal,
        frame: &Value,
    ) -> Result<(), String> {
        let params = &frame["params"];
        let root = params["threadId"].as_str() == Some(&self.thread);
        let method = frame["method"].as_str().unwrap_or_default();
        if root && method == "turn/started" {
            self.start(
                host,
                pid,
                params["turn"]["id"]
                    .as_str()
                    .ok_or("codex_turn_identity_missing")?,
            )?;
        }
        if let Some(children) = Self::children(&mut self.decoder, frame) {
            self.background(host, pid, children)?;
        }
        if !root {
            return Ok(());
        }
        if method == "item/completed"
            || (method == "item/started" && params["item"]["type"] == "userMessage")
        {
            let turn = params["turnId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("codex_turn_identity_missing")?;
            journal.item(&self.thread, turn, &params["item"], &self.cwd)?;
        }
        match method {
            "turn/completed" => {
                let turn = params["turn"]["id"]
                    .as_str()
                    .ok_or("codex_turn_identity_missing")?;
                self.start(host, pid, turn)?;
                self.finish(
                    host,
                    pid,
                    turn,
                    params["turn"]["status"].as_str().unwrap_or_default(),
                )?;
            }
            "thread/status/changed" => match state(&params["status"]) {
                Some("permission") => host.codex_observe(&self.id, pid, Observation::Blocked)?,
                Some("working") => host.codex_observe(&self.id, pid, Observation::Working)?,
                Some(state) => host.codex_activity(&self.id, pid, state, None),
                _ => {}
            },
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/tool/requestUserInput" => {
                host.codex_observe(&self.id, pid, Observation::Blocked)?;
            }
            "thread/settings/updated" => self.settings(
                host,
                pid,
                &params["threadSettings"]["collaborationMode"]["settings"],
            )?,
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        host: Arc<Host>,
        conversation: Conversation,
        observer: Observer,
        journal: Journal,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("codex-observe-{}", crate::agents::uuid().unwrap()));
            std::fs::create_dir_all(&root).unwrap();
            let host = server::test_host(&root);
            host.supervision
                .configure(&json!({"action":"setup", "url":"https://cloud.example",
                "token":"test-token", "supervisor":{"id":"supervisor", "name":"Supervisor"}}))
                .unwrap();
            let workspace: Workspace = serde_json::from_value(json!({
                "id":"workspace", "project":"/repo", "title":"Task", "branch":"feature", "base":"HEAD", "worktree":"/repo",
                "created_at":1, "created_by":{"kind":"agent"}, "supervision":{"supervisor_id":"supervisor"},
                "conversations":[{"id":"conversation", "title":"Task", "created_at":1, "permission":"auto",
                    "provider":"codex", "thread_id":"native-thread", "model":"model", "effort":"high"}]
            })).unwrap();
            let conversation = workspace.conversations[0].clone();
            host.supervision.record_workspace(workspace).unwrap();
            lock(&host.codex_watch).insert(conversation.id.clone(), 42);
            let observer = Observer::new(&conversation, "/repo");
            let journal = Journal::open(&host, &conversation.id).unwrap();
            Self {
                host,
                conversation,
                observer,
                journal,
            }
        }

        fn live(&mut self, method: &str, params: Value) {
            self.observer
                .live(
                    &self.host,
                    42,
                    &mut self.journal,
                    &json!({"method":method,"params":params}),
                )
                .unwrap();
        }

        fn root(&mut self, method: &str, turn: &str, status: &str) {
            self.live(
                method,
                json!({"threadId":"native-thread","turn":{"id":turn,"status":status}}),
            );
        }

        fn events(&self, kind: &str) -> usize {
            let registry = serde_json::to_value(self.host.registry.read().unwrap()).unwrap();
            registry["supervision"]["outbox"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["kind"] == kind)
                .count()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.host.root);
        }
    }

    #[test]
    fn completion_waits_for_native_child_work_and_replay_does_not_repeat_it() {
        let mut fixture = Fixture::new();
        fixture.observer.replay(&fixture.host, 42, &[]).unwrap();
        fixture.root("turn/started", "turn-1", "inProgress");
        let child = json!({"type":"subAgentActivity","id":"child-start","kind":"started","agentThreadId":"child"});
        fixture.live(
            "item/completed",
            json!({"threadId":"native-thread","turnId":"turn-1","item":child}),
        );
        fixture.root("turn/completed", "turn-1", "completed");
        assert_eq!(fixture.events("execution.completed"), 0);
        fixture.live(
            "thread/status/changed",
            json!({"threadId":"child","status":{"type":"idle"}}),
        );
        assert_eq!(fixture.events("execution.completed"), 1);
        fixture.root("turn/completed", "turn-1", "completed");
        fixture
            .observer
            .replay(
                &fixture.host,
                42,
                &[json!({"id":"turn-1","status":"completed","items":[child]})],
            )
            .unwrap();
        assert_eq!(fixture.events("execution.completed"), 1);
    }

    #[test]
    fn first_snapshot_skips_history_but_reconnect_recovers_an_unseen_turn() {
        let mut fixture = Fixture::new();
        let old = json!({"id":"old","status":"completed","items":[]});
        fixture
            .observer
            .replay(&fixture.host, 42, std::slice::from_ref(&old))
            .unwrap();
        assert_eq!(fixture.events("execution.completed"), 0);
        let turns = [old, json!({"id":"new","status":"completed","items":[]})];
        fixture.observer.replay(&fixture.host, 42, &turns).unwrap();
        assert_eq!(fixture.events("execution.completed"), 1);
        fixture.observer.replay(&fixture.host, 42, &turns).unwrap();
        assert_eq!(fixture.events("execution.completed"), 1);
    }

    #[test]
    fn failure_interruption_and_old_observers_never_report_success() {
        let mut fixture = Fixture::new();
        fixture.root("turn/started", "failed", "inProgress");
        fixture.root("turn/completed", "failed", "failed");
        fixture.root("turn/completed", "failed", "completed");
        assert_eq!(fixture.events("execution.failed"), 1);
        assert_eq!(fixture.events("execution.completed"), 0);
        fixture.root("turn/started", "interrupted", "inProgress");
        fixture.root("turn/completed", "interrupted", "interrupted");
        fixture.root("turn/completed", "interrupted", "completed");
        assert_eq!(fixture.events("execution.completed"), 0);
        fixture.root("turn/started", "replacement", "inProgress");
        lock(&fixture.host.codex_watch).insert("conversation".into(), 43);
        fixture.root("turn/completed", "replacement", "completed");
        assert_eq!(fixture.events("execution.completed"), 0);
    }

    #[test]
    fn blocking_and_native_settings_updates_use_the_current_engine_only() {
        let mut fixture = Fixture::new();
        fixture.root("turn/started", "turn", "inProgress");
        let request = json!({"threadId":"native-thread", "itemId":"approval", "command":"command"});
        fixture.live("item/commandExecution/requestApproval", request.clone());
        fixture.live("item/commandExecution/requestApproval", request);
        assert_eq!(fixture.events("execution.blocked"), 1);
        fixture
            .observer
            .settings(
                &fixture.host,
                42,
                &json!({"model":"new-model","reasoning_effort":"medium"}),
            )
            .unwrap();
        let saved = fixture.host.registry.read().unwrap();
        let (_, conversation) = saved.conversation(&fixture.conversation.id).unwrap();
        assert_eq!(conversation.model, "new-model");
        assert_eq!(conversation.effort, "medium");
        assert_eq!(conversation.thread_id.as_deref(), Some("native-thread"));
        fixture
            .observer
            .settings(
                &fixture.host,
                41,
                &json!({"model":"stale","reasoning_effort":"low"}),
            )
            .unwrap();
        assert_eq!(
            fixture
                .host
                .registry
                .read()
                .unwrap()
                .conversation(&fixture.conversation.id)
                .unwrap()
                .1
                .model,
            "new-model"
        );
    }
}
