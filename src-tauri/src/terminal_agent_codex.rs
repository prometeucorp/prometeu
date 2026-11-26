//! One private Codex app-server owns the native TUI's exact thread and session settings.
//! The remote TUI keeps interactive decisions; observing a request never answers it.
use crate::i18n;
use prometeu_core::{actions::Permission, lock::lock, session::launch::LaunchRequest};
use serde_json::{json, Value};
use std::{
    collections::{HashSet, VecDeque},
    io::{self, Write},
    os::unix::{fs::DirBuilderExt, process::CommandExt},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

mod wire;
use wire::{Client, Error};
pub(crate) mod events;
mod lifecycle;
use events::Events;

/// Tauri exits without necessarily dropping AppState or conversation worker references.
/// Stop and reap every owned engine before the event loop returns to process exit.
pub fn shutdown() {
    lifecycle::shutdown();
}

const START_TIMEOUT: Duration = Duration::from_secs(20);
const QUEUE_LIMIT: usize = 64;
const QUEUE_BYTES: usize = 8 * 1024 * 1024;
/// Version the ignored native smoke tests were last verified on; launch only enforces a minimum.
#[cfg(test)]
pub const VERIFIED_VERSION: &str = "codex-cli 0.169.0";

/// Prepared configuration, not a running process. HookBridge must configure `command` first.
pub struct NativeEnginePlan {
    pub command: Command,
    settings: Settings,
}

#[derive(Clone)]
struct Settings {
    cwd: String,
    resume: Option<String>,
    model: String,
    effort: String,
    instructions: String,
    ask: bool,
    plan: bool,
}

impl NativeEnginePlan {
    pub fn new(command: Command, request: &LaunchRequest, resume: Option<String>) -> Self {
        Self {
            command,
            settings: Settings {
                cwd: request.worktree.clone(),
                resume,
                model: request.settings.model.trim().into(),
                effort: crate::agents::effort(&request.settings.effort)
                    .trim()
                    .into(),
                instructions: request.settings.instructions.clone(),
                ask: request.settings.permission == Some(Permission::Ask),
                plan: request.settings.plan,
            },
        }
    }

    /// Starts and verifies the engine before the caller spawns the TUI or admits input.
    /// Failure owns cleanup, including a server that started but never opened its socket.
    pub fn start(self, tui: &mut Command) -> Result<NativeEngine, String> {
        self.start_inner(tui).map_err(|error| {
            // Error categories carry no provider payload, command arguments or account data.
            eprintln!("Native Codex engine startup failed: {error:?}");
            failure()
        })
    }

    fn start_inner(self, tui: &mut Command) -> Result<NativeEngine, Error> {
        self.start_bounded(tui, START_TIMEOUT)
    }

    fn start_bounded(
        mut self,
        tui: &mut Command,
        timeout: Duration,
    ) -> Result<NativeEngine, Error> {
        let _admission = lifecycle::admit()?;
        let deadline = Instant::now() + timeout;
        let incarnation = uuid::Uuid::new_v4().to_string();
        let directory =
            PathBuf::from("/tmp").join(format!("prometeu-codex-{}", uuid::Uuid::new_v4().simple()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let mut owner = Owner {
            directory,
            child: None,
        };
        let socket = owner.directory.join("rpc");
        self.command
            .args(["--listen", &format!("unix://{}", socket.display())]);
        self.command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        owner.child = Some(self.command.spawn()?);
        let pid = owner.child.as_ref().ok_or(Error::Closed)?.id();
        let mut client = Client::connect(&socket, pid, deadline, || owner.exited())?;
        client.call("initialize", json!({
            "clientInfo": {"name":"prometeu-native", "title":"Prometeu", "version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }), deadline)?;
        client.send(json!({"method":"initialized", "params":{}}))?;
        let (thread, expected) = self.settings.open(&mut client, deadline)?;
        client.call("thread/settings/update", expected.clone(), deadline)?;
        // A no-op update emits no settings notification. Live resume is the native atomic
        // settings/read + subscribe operation, and its correlated reply verifies both cases.
        let verified = client.call(
            "thread/resume",
            json!({"threadId":thread,"excludeTurns":true}),
            deadline,
        )?;
        client.verify_settings(&thread, &expected, &verified)?;
        let history = read_history(&mut client, &thread, deadline)?;

        // Remote resume rejects permission flags. All semantic settings belong to the engine;
        // the TUI receives only the captured account/environment, cwd and exact native identity.
        tui.args([
            "--remote",
            &format!("unix://{}", socket.display()),
            "resume",
            &thread,
        ]);
        let (send, commands) = mpsc::sync_channel(32);
        let (events, receive) = events::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let stop = stopped.clone();
        let alive = running.clone();
        let native = thread.clone();
        let worker = std::thread::Builder::new()
            .name("codex-native-engine".into())
            .spawn(move || {
                let result = serve(&mut client, &native, &commands, &events, &stop, &mut owner);
                if let Err(error) = result {
                    let _ = events.try_send(Err(error.message()));
                }
                // Close the socket before retiring the owned server and its private directory.
                drop(client);
                drop(owner);
                alive.store(false, Ordering::Release);
            })?;
        let shutdown = lifecycle::EngineShutdown::register(stopped.clone(), worker);
        Ok(NativeEngine {
            thread,
            incarnation,
            configuration: verified,
            history,
            commands: send,
            events: Mutex::new(Some(receive)),
            stopped,
            running,
            shutdown,
        })
    }
}

/// The native paginated history carries exactly the same item IDs and item schema as the live
/// connection. Read oldest first before releasing the TUI, without inspecting another session.
fn read_history(client: &mut Client, thread: &str, deadline: Instant) -> Result<Vec<Value>, Error> {
    let mut history = Vec::new();
    let mut cursor = Value::Null;
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0usize;
    loop {
        let page = client.call(
            "thread/turns/list",
            json!({"threadId":thread,"cursor":cursor,
            "limit":16,"sortDirection":"asc","itemsView":"full"}),
            deadline,
        )?;
        bytes = bytes
            .checked_add(page.to_string().len())
            .ok_or(Error::Protocol)?;
        let turns = page["data"].as_array().ok_or(Error::Protocol)?;
        if bytes > 64 * 1024 * 1024 || history.len() + turns.len() > 10_000 {
            return Err(Error::Protocol);
        }
        for turn in turns {
            let id = turn["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or(Error::Protocol)?;
            if !seen.insert(id.to_owned()) || !turn["items"].is_array() {
                return Err(Error::Protocol);
            }
            history.push(turn.clone());
        }
        match page.get("nextCursor") {
            Some(Value::Null) => break,
            Some(Value::String(next)) if !next.is_empty() && cursor.as_str() != Some(next) => {
                cursor = json!(next)
            }
            _ => return Err(Error::Protocol),
        }
    }
    Ok(history)
}

impl Settings {
    fn open(&self, client: &mut Client, deadline: Instant) -> Result<(String, Value), Error> {
        let policy = if self.ask { "untrusted" } else { "never" };
        let mut params =
            json!({"cwd":self.cwd,"approvalPolicy":policy,"sandbox":"danger-full-access"});
        if !self.model.is_empty() {
            params["model"] = self.model.clone().into();
        }
        if !self.instructions.is_empty() {
            params["developerInstructions"] = self.instructions.clone().into();
        }
        let method = if let Some(resume) = &self.resume {
            params["threadId"] = resume.clone().into();
            params["excludeTurns"] = true.into();
            "thread/resume"
        } else {
            params["historyMode"] = "paginated".into();
            "thread/start"
        };
        let opened = client.call(method, params, deadline)?;
        let thread = opened["thread"]["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or(Error::Identity)?
            .to_owned();
        if self.resume.as_ref().is_some_and(|id| id != &thread)
            || opened["cwd"].as_str() != Some(self.cwd.as_str())
        {
            return Err(Error::Identity);
        }
        if self.resume.is_none() {
            // Codex does not persist an empty fresh thread until a turn or metadata mutation.
            // Naming it is a native no-inference operation and makes remote resume possible.
            client.call(
                "thread/name/set",
                json!({"threadId":thread,"name":"Prometeu"}),
                deadline,
            )?;
        }
        let model = if self.model.is_empty() {
            opened["model"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(Error::Protocol)?
        } else {
            &self.model
        };
        let effort = if self.effort.is_empty() {
            opened["reasoningEffort"].clone()
        } else {
            Value::String(self.effort.clone())
        };
        let expected = json!({"threadId":thread,"approvalPolicy":policy,
            "sandboxPolicy":{"type":"dangerFullAccess"}, "effort":effort,
            "collaborationMode":{"mode":if self.plan {"plan"} else {"default"},
                "settings":{"model":model,"reasoning_effort":effort,"developer_instructions":null}}});
        Ok((thread, expected))
    }
}

pub struct NativeEngine {
    thread: String,
    incarnation: String,
    configuration: Value,
    history: Vec<Value>,
    commands: mpsc::SyncSender<Control>,
    events: Mutex<Option<Events>>,
    stopped: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    shutdown: Arc<lifecycle::EngineShutdown>,
}

enum Control {
    Send {
        frame: Value,
        deadline: Instant,
        done: mpsc::Sender<Result<(), Error>>,
    },
}

impl NativeEngine {
    pub fn thread_id(&self) -> &str {
        &self.thread
    }
    pub fn history(&self) -> &[Value] {
        &self.history
    }
    pub fn configuration(&self) -> &Value {
        &self.configuration
    }
    pub fn attached(&self, request: &LaunchRequest) -> prometeu_protocols::codex::Link {
        use prometeu_core::session::launch::StartMode;
        let start = prometeu_protocols::codex::Start {
            cwd: request.worktree.clone(),
            resume: match &request.mode {
                StartMode::Fresh => None,
                StartMode::Resume { provider_session } => provider_session.clone(),
            },
            model: self.configuration["model"].as_str().unwrap_or("").into(),
            effort: self.configuration["reasoningEffort"]
                .as_str()
                .unwrap_or("")
                .into(),
            plugin_ids: vec![],
            plugin_hook_ids: vec![],
            permission: request.settings.permission,
            instructions: String::new(),
        };
        prometeu_protocols::codex::Link::attached(
            self.input(),
            start,
            self.thread.clone(),
            self.incarnation.clone(),
            i18n::pick,
        )
    }
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    /// One consumer must drain this bounded stream; loss is fatal, never silent truncation.
    pub(crate) fn take_events(&self) -> Option<Events> {
        lock(&self.events).take()
    }
    /// The attached adapter may query usage, interrupt this thread, or answer a pending native
    /// request. This port cannot initialize another client/thread or start a model turn.
    pub fn input(&self) -> Box<dyn Write + Send> {
        Box::new(Input {
            commands: self.commands.clone(),
            running: self.running.clone(),
            stopped: self.stopped.clone(),
            pending: Vec::new(),
        })
    }
    /// Only an explicit application decision may answer a still-pending request for this thread.
    /// The native TUI may win the same decision first; a resolved/stale ID is rejected.
    pub fn respond(&self, id: Value, result: Value) -> Result<(), String> {
        send(
            &self.commands,
            &self.running,
            &self.stopped,
            json!({"id":id,"result":result}),
        )
        .map_err(|_| failure())
    }
    pub fn stop(&self) {
        self.shutdown.stop();
    }
}
impl Drop for NativeEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

fn send(
    commands: &mpsc::SyncSender<Control>,
    running: &AtomicBool,
    stopped: &AtomicBool,
    frame: Value,
) -> Result<(), Error> {
    if !running.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
        return Err(Error::Closed);
    }
    let (done, wait) = mpsc::channel();
    commands
        .try_send(Control::Send {
            frame,
            deadline: Instant::now() + Duration::from_secs(5),
            done,
        })
        .map_err(|_| Error::Limit)?;
    wait.recv_timeout(Duration::from_secs(5))
        .map_err(|_| Error::Timeout)?
}

struct Input {
    commands: mpsc::SyncSender<Control>,
    running: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    pending: Vec<u8>,
}
impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.pending.len().saturating_add(bytes.len()) > 1024 * 1024 {
            return Err(io::Error::other(failure()));
        }
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<_> = self.pending.drain(..=end).collect();
            let frame = serde_json::from_slice(&line).map_err(|_| io::Error::other(failure()))?;
            send(&self.commands, &self.running, &self.stopped, frame)
                .map_err(|_| io::Error::other(failure()))?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(failure()))
        }
    }
}

fn serve(
    client: &mut Client,
    thread: &str,
    commands: &mpsc::Receiver<Control>,
    events: &events::Sender,
    stop: &AtomicBool,
    owner: &mut Owner,
) -> Result<(), Error> {
    let mut pending = VecDeque::<Value>::new();
    let mut requests = HashSet::<String>::new();
    while !stop.load(Ordering::Acquire) {
        if owner.exited()? {
            return Err(Error::Closed);
        }
        // Drain native frames before controls so an already delivered resolution wins the race.
        if let Some(value) = client.next()? {
            if value.get("method").is_none() {
                requests.remove(&value["id"].to_string());
            }
            if value.get("method").is_some()
                && value.get("id").is_some()
                && value["params"]["threadId"].as_str() == Some(thread)
            {
                if pending.len() == QUEUE_LIMIT {
                    return Err(Error::Limit);
                }
                pending.push_back(value["id"].clone());
            }
            if value["method"] == "serverRequest/resolved" {
                pending.retain(|id| id != &value["params"]["requestId"]);
            }
            events.try_send(Ok(value)).map_err(|_| Error::Limit)?;
        }
        while let Ok(Control::Send {
            frame,
            deadline,
            done,
        }) = commands.try_recv()
        {
            let id = &frame["id"];
            let method = frame["method"].as_str();
            let permitted_request = matches!(method, Some("account/rateLimits/read"))
                || (method == Some("turn/interrupt")
                    && frame["params"]["threadId"] == thread
                    && frame["params"]["turnId"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty()));
            let permitted_response =
                method.is_none() && frame.get("result").is_some() && pending.contains(id);
            let valid_id = id.is_string() || id.is_u64();
            let response = if Instant::now() >= deadline
                || !valid_id
                || !(permitted_request || permitted_response)
                || (permitted_request
                    && (requests.len() >= 32 || requests.contains(&id.to_string())))
            {
                Err(Error::Conflict)
            } else {
                if permitted_response {
                    pending.retain(|request| request != id);
                } else {
                    requests.insert(id.to_string());
                }
                client.send(frame)
            };
            let failed = response.is_err() && !matches!(response, Err(Error::Conflict));
            let _ = done.send(response);
            if failed {
                return Err(Error::Closed);
            }
        }
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}
fn failure() -> String {
    i18n::t("err.terminalAgent.engine")
}

/// Single mutable owner keeps signaling and reaping serialized; a reaped PID is never signaled.
struct Owner {
    directory: PathBuf,
    child: Option<Child>,
}
impl Owner {
    fn exited(&mut self) -> Result<bool, Error> {
        if let Some(child) = self.child.as_mut() {
            if child.try_wait()?.is_none() {
                return Ok(false);
            }
            self.child = None;
        }
        Ok(true)
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().ok().flatten().is_none() {
                // The dedicated group leader is still owned and unreaped.
                unsafe {
                    libc::killpg(child.id() as libc::pid_t, libc::SIGTERM);
                }
                let deadline = Instant::now() + Duration::from_secs(1);
                let mut exited = false;
                while Instant::now() < deadline {
                    if child.try_wait().ok().flatten().is_some() {
                        exited = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                if !exited {
                    unsafe {
                        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
                    }
                }
            }
            let _ = child.wait();
        }
        wire::remove_owned_endpoint(&self.directory.join("rpc"));
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod smoke;
#[cfg(test)]
mod tests;
