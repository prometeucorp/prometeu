//! Experimental conversation client. Platform selection belongs to the composition root.
pub mod application;
mod files;
pub mod terminal;
use application::ApplicationClient;
pub mod workspaces;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use terminal::{OpenedTerminal, TerminalClient, TerminalSnapshot};
use workspaces::{Catalog, WorkspaceClient};

const MAX_FRAME: u64 = 8 * 1024 * 1024;
const MAX_REQUEST: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub distribution: String,
    pub executable: String,
    pub root: String,
    pub workdir: String,
    pub codex: String,
}
impl Target {
    pub fn validate(&self) -> Result<(), String> {
        if self.distribution.trim().is_empty() || self.distribution.starts_with('-') {
            return Err("a WSL distribution is required".into());
        }
        for path in [&self.executable, &self.root, &self.workdir, &self.codex] {
            if !path.starts_with('/') || path.contains(['\0', '\n', '\r']) {
                return Err(
                    "runtime, root, project and Codex paths must be absolute Linux paths".into(),
                );
            }
        }
        if self.distribution.contains(['\0', '\n', '\r']) {
            return Err("invalid WSL distribution".into());
        }
        Ok(())
    }
}

/// The launcher supplies pipes. Tests can run the same Linux host directly.
pub trait RuntimeLauncher: Send + Sync {
    fn launch(&self, target: &Target) -> Result<Child, String>;
}
pub struct WslLauncher;
impl WslLauncher {
    pub fn command(target: &Target) -> Result<Command, String> {
        target.validate()?;
        let mut command = Command::new("wsl.exe");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW for the piped console adapter.
        }
        command.args([
            "--distribution",
            &target.distribution,
            "--exec",
            &target.executable,
            "--root",
            &target.root,
            "--workdir",
            &target.workdir,
            "--codex",
            &target.codex,
        ]);
        Ok(command)
    }
}
impl RuntimeLauncher for WslLauncher {
    fn launch(&self, target: &Target) -> Result<Child, String> {
        Self::command(target)?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())
    }
}

/// Selects persistent attachment at composition; ordinary stdio launch remains available.
pub struct ResidentWslLauncher;
impl RuntimeLauncher for ResidentWslLauncher {
    fn launch(&self, target: &Target) -> Result<Child, String> {
        WslLauncher::command(target)?
            .args(["--transport", "resident"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())
    }
}

/// The shared desktop starts with an empty board, independently of its bootstrap directory.
pub struct ApplicationWslLauncher;
impl RuntimeLauncher for ApplicationWslLauncher {
    fn launch(&self, target: &Target) -> Result<Child, String> {
        WslLauncher::command(target)?
            .args(["--transport", "resident", "--catalog", "application"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())
    }
}

pub trait RuntimeEvents: Send + Sync {
    fn publish(&self, frame: Value);
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Started {
    pub generation: String,
    pub resuming: bool,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub generation: Option<String>,
    pub snapshot: TranscriptSnapshot,
    pub provider_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct TranscriptSnapshot {
    pub text: String,
    pub seq: u64,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Allow,
    Deny {
        message: String,
    },
    Answer {
        answers: std::collections::BTreeMap<String, String>,
    },
}

/// Narrow application operations; request envelopes never escape the transport adapter.
pub trait SessionClient: Send {
    fn start(&mut self) -> Result<Started, String>;
    fn send(&mut self, text: String) -> Result<(), String>;
    fn respond(&mut self, request_id: String, response: Response) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn snapshot(&mut self) -> Result<Snapshot, String>;
    fn shutdown(&mut self) -> Result<(), String>;
}
pub trait AttachmentClient: Send {
    fn connected(&self) -> bool;
    fn disconnect(&mut self) -> Result<(), String>;
}
pub trait RuntimeClient:
    SessionClient + TerminalClient + AttachmentClient + WorkspaceClient + ApplicationClient
{
}
impl<
        T: SessionClient + TerminalClient + AttachmentClient + WorkspaceClient + ApplicationClient,
    > RuntimeClient for T
{
}

pub trait RuntimeConnector: Send + Sync {
    fn connect(
        &self,
        target: &Target,
        events: Arc<dyn RuntimeEvents>,
    ) -> Result<Box<dyn RuntimeClient>, String>;
}
pub struct StdioConnector {
    pub launcher: Arc<dyn RuntimeLauncher>,
}
impl RuntimeConnector for StdioConnector {
    fn connect(
        &self,
        target: &Target,
        events: Arc<dyn RuntimeEvents>,
    ) -> Result<Box<dyn RuntimeClient>, String> {
        target.validate()?;
        let delivery_for = || {
            Arc::new(AttachmentEvents {
                active: AtomicBool::new(false),
                output: events.clone(),
            })
        };
        let mut delivery = delivery_for();
        let mut client = Client::open(self.launcher.launch(target)?, delivery.clone())?;
        if client
            .retirement
            .as_ref()
            .is_some_and(|path| path != &target.executable)
            && client.request(json!({"method":"retire"}))?["retired"] == true
        {
            // The old host has atomically admitted retirement without live execution.
            // Retry attachment only; no application request is repeated.
            client.disconnect()?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let next_delivery = delivery_for();
                match self
                    .launcher
                    .launch(target)
                    .and_then(|child| Client::open(child, next_delivery.clone()))
                {
                    Ok(next) => {
                        client = next;
                        delivery = next_delivery;
                        break;
                    }
                    Err(error) if Instant::now() >= deadline => return Err(error),
                    Err(_) => std::thread::sleep(Duration::from_millis(100)),
                }
            }
        }
        delivery.active.store(true, Ordering::Release);
        Ok(Box::new(client))
    }
}

// Handshake, retirement and failed replacement attempts are not live attachments.
struct AttachmentEvents {
    active: AtomicBool,
    output: Arc<dyn RuntimeEvents>,
}
impl RuntimeEvents for AttachmentEvents {
    fn publish(&self, frame: Value) {
        if self.active.load(Ordering::Acquire) {
            self.output.publish(frame);
        }
    }
}
#[derive(Default, Deserialize)]
struct Ready {
    #[serde(default)]
    capabilities: Vec<String>,
    executable: Option<String>,
}

type Reply = Result<Value, String>;
type Pending = Option<(u64, mpsc::SyncSender<Reply>)>;
struct Shared {
    pending: Mutex<Pending>,
    failure: Mutex<Option<String>>,
    events: Arc<dyn RuntimeEvents>,
}
impl Shared {
    fn fail(&self, error: String) {
        let mut failure = self.failure.lock().unwrap();
        if failure.is_some() {
            return;
        }
        *failure = Some(error.clone());
        if let Some((_, reply)) = self.pending.lock().unwrap().take() {
            let _ = reply.send(Err(error.clone()));
        }
        drop(failure);
        self.events
            .publish(json!({"v":1,"lifecycle":"disconnected","error":error}));
    }
}
struct Client {
    child: Child,
    input: Option<ChildStdin>,
    shared: Arc<Shared>,
    diagnostics: Arc<Mutex<String>>,
    next_id: u64,
    terminal_supported: bool,
    workspaces_supported: bool,
    worktrees_supported: bool,
    application_supported: bool,
    operations_supported: bool,
    initialization_supported: bool,
    retirement: Option<String>,
}
impl Client {
    fn open(mut child: Child, events: Arc<dyn RuntimeEvents>) -> Result<Self, String> {
        let input = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            failure: Mutex::new(None),
            events,
        });
        let diagnostics = Arc::new(Mutex::new(String::new()));
        let mut client = Self {
            child,
            input,
            shared: shared.clone(),
            diagnostics: diagnostics.clone(),
            next_id: 0,
            terminal_supported: false,
            workspaces_supported: false,
            worktrees_supported: false,
            application_supported: false,
            operations_supported: false,
            initialization_supported: false,
            retirement: None,
        };
        let stdout = stdout.ok_or("launcher did not provide stdout")?;
        let stderr = stderr.ok_or("launcher did not provide stderr")?;
        if client.input.is_none() {
            return Err("launcher did not provide stdin".into());
        }
        let (diagnostics_done, drained) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut stderr = stderr;
            let mut bytes = [0; 1024];
            while let Ok(n) = stderr.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let mut text = diagnostics.lock().unwrap();
                text.push_str(&String::from_utf8_lossy(&bytes[..n]));
                // Keep a bounded, UTF-8-safe tail for bootstrap failures.
                while text.len() > 4096 {
                    text.remove(0);
                }
            }
            let _ = diagnostics_done.send(());
        });
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // Retain the sender until failure publication, so an early EOF cannot
            // disconnect the handshake channel before its reason is available.
            let result = read_frames(stdout, &shared, &ready_tx);
            shared.fail(
                result
                    .err()
                    .unwrap_or_else(|| "runtime connection closed".into()),
            );
        });
        match ready_rx.recv_timeout(TIMEOUT) {
            Ok(Ok(ready)) => {
                let capabilities = ready.capabilities;
                client.retirement = ready
                    .executable
                    .filter(|_| capabilities.iter().any(|c| c == "retire.v1"));
                client.terminal_supported = capabilities.iter().any(|c| c == "terminal.v1");
                client.initialization_supported = capabilities
                    .iter()
                    .any(|c| c == "application.initialization.v1");
                client.operations_supported = capabilities
                    .iter()
                    .any(|c| c == "application.operations.v1");
                client.application_supported = capabilities.iter().any(|c| c == "application.v1");
                client.worktrees_supported = capabilities.iter().any(|c| c == "worktrees.v1");
                client.workspaces_supported = capabilities.iter().any(|c| c == "workspaces.v1");
                Ok(client)
            }
            result => {
                client.close();
                // Process exit and the stderr reader complete independently. Bound
                // this wait in case a descendant retained the diagnostic pipe.
                let _ = drained.recv_timeout(Duration::from_secs(1));
                let reason = match result {
                    Ok(Err(e)) => e,
                    _ => client
                        .shared
                        .failure
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or("runtime handshake timed out".into()),
                };
                Err(format!("{reason}\n{}", client.diagnostics.lock().unwrap()))
            }
        }
    }
    fn request(&mut self, action: Value) -> Reply {
        self.next_id += 1;
        let id = self.next_id;
        let line = format!("{}\n", json!({"v":1,"id":id,"action":action}));
        if line.len() > MAX_REQUEST {
            return Err("request exceeds 1 MiB".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let failure = self.shared.failure.lock().unwrap();
            if let Some(error) = &*failure {
                return Err(error.clone());
            }
            *self.shared.pending.lock().unwrap() = Some((id, tx));
        }
        let result = self
            .input
            .as_mut()
            .ok_or("runtime connection closed".to_owned())
            .and_then(|input| {
                input
                    .write_all(line.as_bytes())
                    .and_then(|()| input.flush())
                    .map_err(|e| e.to_string())
            });
        if let Err(error) = result {
            self.shared.fail(error.clone());
            self.close();
            return Err(error);
        }
        match rx.recv_timeout(TIMEOUT) {
            Ok(reply) => reply,
            Err(_) => {
                let error = "runtime reply timed out; outcome unknown; reconnect and inspect history before retrying".to_owned();
                self.shared.fail(error.clone());
                self.close();
                Err(error)
            }
        }
    }
    fn close(&mut self) {
        // EOF closes the selected transport: disposable hosts stop; resident proxies detach.
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl AttachmentClient for Client {
    fn connected(&self) -> bool {
        self.input.is_some() && self.shared.failure.lock().unwrap().is_none()
    }

    fn disconnect(&mut self) -> Result<(), String> {
        self.close();
        Ok(())
    }
}
impl SessionClient for Client {
    fn start(&mut self) -> Result<Started, String> {
        serde_json::from_value(self.request(json!({"method":"start"}))?).map_err(|e| e.to_string())
    }
    fn send(&mut self, text: String) -> Result<(), String> {
        self.request(json!({"method":"command","frame":{"v":1,"type":"message.send","text":text}}))
            .map(|_| ())
    }
    fn respond(&mut self, request_id: String, response: Response) -> Result<(), String> {
        self.request(json!({"method":"command","frame":{"v":1,"type":"request.respond","requestId":request_id,"response":response}})).map(|_| ())
    }
    fn stop(&mut self) -> Result<(), String> {
        self.request(json!({"method":"stop"})).map(|_| ())
    }
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        serde_json::from_value(self.request(json!({"method":"snapshot"}))?)
            .map_err(|e| e.to_string())
    }
    fn shutdown(&mut self) -> Result<(), String> {
        self.request(json!({"method":"shutdown"}))?;
        self.close();
        Ok(())
    }
}
impl ApplicationClient for Client {
    fn initialization_supported(&self) -> bool {
        self.initialization_supported
    }
    fn operations_supported(&self) -> bool {
        self.operations_supported
    }
    fn application(&mut self, command: String, args: Value) -> Result<Value, String> {
        if !self.application_supported {
            return Err("application_unsupported".into());
        }
        self.request(json!({"method":"application","command":command,"args":args}))
    }
}
impl WorkspaceClient for Client {
    fn workspace_worktree(
        &mut self,
        request: workspaces::WorktreeRequest,
    ) -> Result<Catalog, String> {
        if !self.worktrees_supported {
            return Err("workspace_worktree_unsupported".into());
        }
        serde_json::from_value(
            self.request(json!({"method":"workspace_worktree","request":request}))?,
        )
        .map_err(|e| e.to_string())
    }
    fn workspace_list(&mut self) -> Result<Catalog, String> {
        if !self.workspaces_supported {
            return Err("workspace_unsupported".into());
        }
        serde_json::from_value(self.request(json!({"method":"workspace_list"}))?)
            .map_err(|e| e.to_string())
    }
    fn workspace_create(&mut self, title: String, path: String) -> Result<Catalog, String> {
        serde_json::from_value(
            self.request(json!({"method":"workspace_create","title":title,"path":path}))?,
        )
        .map_err(|e| e.to_string())
    }
    fn workspace_select(&mut self, id: String) -> Result<Catalog, String> {
        serde_json::from_value(self.request(json!({"method":"workspace_select","id":id}))?)
            .map_err(|e| e.to_string())
    }
    fn workspace_stage(&mut self, id: String, stage: String) -> Result<Catalog, String> {
        serde_json::from_value(
            self.request(json!({"method":"workspace_stage","id":id,"stage":stage}))?,
        )
        .map_err(|e| e.to_string())
    }
}
impl Client {
    fn terminal_request(&mut self, action: Value) -> Reply {
        if !self.terminal_supported {
            return Err("runtime does not support terminal.v1; rebuild the Linux runtime".into());
        }
        self.request(action)
    }
}
impl TerminalClient for Client {
    fn terminal_current(&mut self) -> Result<Option<TerminalSnapshot>, String> {
        serde_json::from_value(self.terminal_request(json!({"method":"terminal_current"}))?)
            .map_err(|e| e.to_string())
    }
    fn terminal_supported(&self) -> bool {
        self.terminal_supported
    }
    fn terminal_open(&mut self, cols: u16, rows: u16) -> Result<OpenedTerminal, String> {
        serde_json::from_value(
            self.terminal_request(json!({"method":"terminal_open","cols":cols,"rows":rows}))?,
        )
        .map_err(|e| e.to_string())
    }
    fn terminal_write(&mut self, id: String, data: Vec<u8>) -> Result<(), String> {
        if data.len() > 4096 {
            return Err("terminal input exceeds 4 KiB".into());
        }
        self.terminal_request(json!({"method":"terminal_write","id":id,"data":data}))
            .map(|_| ())
    }
    fn terminal_resize(&mut self, id: String, cols: u16, rows: u16) -> Result<(), String> {
        self.terminal_request(json!({"method":"terminal_resize","id":id,"cols":cols,"rows":rows}))
            .map(|_| ())
    }
    fn terminal_snapshot(&mut self, id: String) -> Result<TerminalSnapshot, String> {
        serde_json::from_value(
            self.terminal_request(json!({"method":"terminal_snapshot","id":id}))?,
        )
        .map_err(|e| e.to_string())
    }
    fn terminal_acknowledge(&mut self, id: String, seq: u64) -> Result<(), String> {
        self.terminal_request(json!({"method":"terminal_acknowledge","id":id,"seq":seq}))
            .map(|_| ())
    }
    fn terminal_close(&mut self, id: String) -> Result<(), String> {
        self.terminal_request(json!({"method":"terminal_close","id":id}))
            .map(|_| ())
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.close();
    }
}

fn read_frames(
    stdout: impl Read,
    shared: &Shared,
    ready: &mpsc::SyncSender<Result<Ready, String>>,
) -> Result<(), String> {
    let mut reader = BufReader::new(stdout);
    let hello = read_frame(&mut reader)?;
    if hello["v"] != 1 || hello["lifecycle"] != "ready" || hello["provider"] != "codex" {
        let error = hello["error"]
            .as_str()
            .unwrap_or("incompatible runtime handshake")
            .to_owned();
        let _ = ready.send(Err(error.clone()));
        return Err(error);
    }
    let handshake = serde_json::from_value::<Ready>(hello).map_err(|e| e.to_string())?;
    let _ = ready.send(Ok(handshake));
    loop {
        let frame = read_frame(&mut reader)?;
        if frame["v"] != 1 {
            return Err("incompatible runtime frame".into());
        }
        if let Some(id) = frame["id"].as_u64() {
            let mut pending = shared.pending.lock().unwrap();
            if pending.as_ref().map(|p| p.0) != Some(id) {
                return Err("unexpected runtime reply".into());
            }
            let (_, reply) = pending.take().unwrap();
            let result = match (frame.get("result"), frame["error"].as_str()) {
                (Some(value), None) => Ok(value.clone()),
                (None, Some(error)) => Err(error.to_owned()),
                _ => return Err("invalid runtime reply".into()),
            };
            let _ = reply.send(result);
        } else if frame.get("id").is_some() {
            return Err("runtime rejected the request envelope".into());
        } else {
            shared.events.publish(frame);
        }
    }
}
fn read_frame(reader: &mut impl BufRead) -> Reply {
    let mut line = String::new();
    let n = reader
        .take(MAX_FRAME + 1)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("runtime connection closed".into());
    }
    if n as u64 > MAX_FRAME || !line.ends_with('\n') {
        return Err("invalid or oversized runtime frame".into());
    }
    serde_json::from_str(&line).map_err(|e| format!("invalid runtime JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wsl_arguments_preserve_linux_paths_and_never_use_a_shell() {
        let target = Target {
            distribution: "Ubuntu Test".into(),
            executable: "/home/test/runtime binary".into(),
            root: "/tmp/a ' $(echo bad)".into(),
            workdir: "/home/test/ação".into(),
            codex: "/home/test/bin/codex".into(),
        };
        let command = WslLauncher::command(&target).unwrap();
        assert_eq!(command.get_program(), "wsl.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![
                "--distribution",
                "Ubuntu Test",
                "--exec",
                "/home/test/runtime binary",
                "--root",
                "/tmp/a ' $(echo bad)",
                "--workdir",
                "/home/test/ação",
                "--codex",
                "/home/test/bin/codex"
            ]
        );
        let mut invalid = target;
        invalid.workdir = "C:\\project".into();
        assert!(WslLauncher::command(&invalid).is_err());
    }
    #[test]
    fn interleaved_events_do_not_consume_the_pending_reply() {
        struct Events(Mutex<Vec<Value>>);
        impl RuntimeEvents for Events {
            fn publish(&self, frame: Value) {
                self.0.lock().unwrap().push(frame);
            }
        }
        let events = Arc::new(Events(Mutex::new(Vec::new())));
        let (tx, rx) = mpsc::sync_channel(1);
        let shared = Shared {
            pending: Mutex::new(Some((7, tx))),
            failure: Mutex::new(None),
            events: events.clone(),
        };
        let (ready, ready_rx) = mpsc::sync_channel(1);
        let frames = [
            json!({"v":1,"lifecycle":"ready","provider":"codex"}),
            json!({"v":1,"generation":"epoch","seq":1,"event":{"type":"session.identity"}}),
            json!({"v":1,"id":7,"result":{"accepted":true}}),
        ];
        let wire = frames
            .iter()
            .map(|frame| format!("{frame}\n"))
            .collect::<String>();
        assert!(read_frames(wire.as_bytes(), &shared, &ready).is_err());
        assert!(ready_rx.recv().unwrap().unwrap().capabilities.is_empty());
        assert_eq!(rx.recv().unwrap().unwrap()["accepted"], true);
        assert_eq!(events.0.lock().unwrap().len(), 1);
        assert!(shared.pending.lock().unwrap().is_none());
    }
    #[test]
    fn incompatible_handshake_and_unexpected_reply_fail_closed() {
        struct Ignore;
        impl RuntimeEvents for Ignore {
            fn publish(&self, _: Value) {}
        }
        for frames in [
            vec![json!({"v":2,"lifecycle":"ready","provider":"codex"})],
            vec![
                json!({"v":1,"lifecycle":"ready","provider":"codex"}),
                json!({"v":1,"id":999,"result":{}}),
            ],
        ] {
            let shared = Shared {
                pending: Mutex::new(None),
                failure: Mutex::new(None),
                events: Arc::new(Ignore),
            };
            let (ready, _) = mpsc::sync_channel(1);
            let wire = frames
                .iter()
                .map(|frame| format!("{frame}\n"))
                .collect::<String>();
            assert!(read_frames(wire.as_bytes(), &shared, &ready).is_err());
        }
    }
    #[test]
    fn terminal_capability_is_explicit_and_unknown_capabilities_remain_compatible() {
        struct Ignore;
        impl RuntimeEvents for Ignore {
            fn publish(&self, _: Value) {}
        }
        let shared = Shared {
            pending: Mutex::new(None),
            failure: Mutex::new(None),
            events: Arc::new(Ignore),
        };
        let (ready, rx) = mpsc::sync_channel(1);
        let hello = format!(
            "{}\n",
            json!({"v":1,"lifecycle":"ready","provider":"codex","capabilities":["terminal.v1","future"]})
        );
        assert!(read_frames(hello.as_bytes(), &shared, &ready).is_err());
        assert_eq!(
            rx.recv().unwrap().unwrap().capabilities,
            ["terminal.v1", "future"]
        );
    }
    #[test]
    fn framing_rejects_truncation_invalid_utf8_and_oversized_data() {
        for bytes in [
            b"{}".to_vec(),
            vec![255, 10],
            vec![b'x'; MAX_FRAME as usize + 1],
        ] {
            assert!(read_frame(&mut &bytes[..]).is_err());
        }
        assert_eq!(read_frame(&mut &b"{\"v\":1}\n"[..]).unwrap()["v"], 1);
    }
}

pub mod bootstrap;
pub mod paths;
pub mod wsl_command;

pub mod mcp;
pub mod oauth_error;

mod operations;
