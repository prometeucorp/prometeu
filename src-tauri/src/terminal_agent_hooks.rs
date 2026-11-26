//! Private, process-scoped native hooks. Decisions require an explicit canonical response.
use crate::terminal_agent_decisions::{Decisions, Pending, WAIT as DECISION_WAIT};
use crate::{i18n, paths, state::ProviderId};
use prometeu_protocols::catalog::CatalogProcess;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
        Arc, Mutex, OnceLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const INPUT_LIMIT: usize = 1_048_576;
const WIRE_LIMIT: usize = INPUT_LIMIT + 4096;
const QUEUE_LIMIT: usize = 16;
const DEADLINE: Duration = Duration::from_secs(2);

#[cfg(test)]
mod decisions_tests;
const SOCKET_ENV: &str = "PROMETEU_TERMINAL_HOOK_SOCKET";
const TOKEN_ENV: &str = "PROMETEU_TERMINAL_HOOK_TOKEN";
const SESSION_ENV: &str = "PROMETEU_TERMINAL_HOOK_SESSION";
const NONCE_ENV: &str = "PROMETEU_TERMINAL_HOOK_NONCE";
const PROVIDER_ENV: &str = "PROMETEU_TERMINAL_HOOK_PROVIDER";
const EVENT_ENV: &str = "PROMETEU_TERMINAL_HOOK_EVENT";
const EXPECTED_NONCE_ENV: &str = "PROMETEU_TERMINAL_HOOK_EXPECTED_NONCE";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeHookKind {
    SessionStarted,
    PromptSubmitted,
    StopObserved,
    PermissionRequested,
    ToolStarted,
    ToolFinished,
    ToolFailed,
    SessionEnded,
    TurnFailed,
    Notification,
    AssistantResponse,
    ModelInvocationStarted,
    ModelInvocationFinished,
}

/// Content belongs to the native session, not to diagnostics or analytics. None of these fields
/// includes the local bridge credential. SessionStarted is not a terminal-input readiness signal;
/// StopObserved does not prove that another stop hook allowed the turn to finish.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeHookEvent {
    pub provider: ProviderId,
    pub session: String,
    pub process_nonce: String,
    pub native_session: String,
    pub kind: NativeHookKind,
    pub transcript_path: Option<PathBuf>,
    pub prompt: Option<String>,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_input: Option<Value>,
    pub tool_response: Option<Value>,
    pub text: Option<String>,
    pub generation_id: Option<String>,
    pub agent_id: Option<String>,
    pub source: Option<String>,
    pub reason: Option<String>,
    pub notification_type: Option<String>,
    pub stop_hook_active: Option<bool>,
    pub error: Option<Value>,
    pub step_index: Option<u64>,
    pub invocation_number: Option<u64>,
    pub initial_num_steps: Option<u64>,
    pub execution_number: Option<u64>,
    pub fully_idle: Option<bool>,
    pub continuation_active: Option<bool>,
    /// Set only by the authenticated listener before acknowledging a Cursor Stop hook.
    /// Helper payloads cannot supply or replay this process-local witness.
    #[serde(skip)]
    pub cursor_stop_stamp: Option<crate::terminal_agent_observation::CursorStopStamp>,
}

pub type CursorStopCapture = Box<
    dyn FnMut(&NativeHookEvent) -> Option<crate::terminal_agent_observation::CursorStopStamp>
        + Send,
>;
type CursorStopCaptureSlot = Arc<Mutex<Option<CursorStopCapture>>>;
type DecisionsSlot = Arc<Mutex<Option<Arc<Decisions>>>>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    token: String,
    event: NativeHookEvent,
}

#[derive(Clone)]
struct Identity {
    provider: ProviderId,
    session: String,
    nonce: String,
}

/// What a remote hook needs to reach this app's bridge (ADR 0101). It holds the bridge
/// credential, so it never leaves this process.
#[derive(Clone)]
pub(crate) struct RelayTarget {
    socket: String,
    token: String,
    nonce: String,
}

#[cfg(test)]
pub(crate) fn test_relay_target() -> RelayTarget {
    RelayTarget {
        socket: "/tmp/prometeu-test-socket".into(),
        token: "token".into(),
        nonce: "nonce".into(),
    }
}

/// Keep this guard alive for exactly one agent process. Dropping it invalidates old hooks, stops
/// the listener and removes its socket and configuration, including after a failed spawn.
pub struct HookBridge {
    directory: PathBuf,
    token: String,
    identity: Identity,
    native_version: OnceLock<String>,
    cancelled: Arc<AtomicBool>,
    wake: UnixStream,
    worker: Option<JoinHandle<()>>,
    cursor_stop_capture: CursorStopCaptureSlot,
    decisions: DecisionsSlot,
}

impl HookBridge {
    /// Install the canonical consumer before spawning Claude. Other providers keep neutral hooks.
    pub(crate) fn enable_claude_decisions(&self) -> Result<Arc<Decisions>, String> {
        if self.identity.provider != ProviderId::Claude {
            return Err(failure());
        }
        let mut slot = crate::lock::lock(&self.decisions);
        Ok(slot
            .get_or_insert_with(|| Decisions::new(&self.identity.nonce))
            .clone())
    }

    /// Install before spawning the CLI. Captures are read-only and never change hook policy.
    pub fn set_cursor_stop_capture(&self, capture: CursorStopCapture) {
        *crate::lock::lock(&self.cursor_stop_capture) = Some(capture);
    }
    /// Stop delivery before publishing process exit. Already queued observations remain available
    /// to the receiver; after this method returns there is no sender and no newly delivered event.
    pub fn close(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = self.wake.write_all(&[1]);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }

    pub fn start(
        provider: ProviderId,
        session: &str,
        process_nonce: &str,
        expected_native_session: Option<&str>,
    ) -> Result<(Self, Receiver<NativeHookEvent>), String> {
        if !matches!(
            provider,
            ProviderId::Claude | ProviderId::Codex | ProviderId::Cursor | ProviderId::Antigravity
        ) || !valid_id(session)
            || !valid_id(process_nonce)
            || expected_native_session.is_some_and(|id| !valid_id(id))
        {
            return Err(failure());
        }
        // macOS has a short sockaddr_un path limit; the directory is private at creation.
        let directory =
            PathBuf::from("/tmp").join(format!("prometeu-hooks-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| failure())?;
        let result = Self::bind(
            provider,
            session,
            process_nonce,
            expected_native_session,
            &directory,
        );
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&directory);
        }
        result
    }

    fn bind(
        provider: ProviderId,
        session: &str,
        process_nonce: &str,
        expected_native_session: Option<&str>,
        directory: &Path,
    ) -> Result<(Self, Receiver<NativeHookEvent>), String> {
        let socket = directory.join("socket");
        let listener = UnixListener::bind(&socket).map_err(|_| failure())?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
            .map_err(|_| failure())?;
        listener.set_nonblocking(true).map_err(|_| failure())?;
        let (wake, cancellation) = UnixStream::pair().map_err(|_| failure())?;
        wake.set_nonblocking(true).map_err(|_| failure())?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let identity = Identity {
            provider,
            session: session.into(),
            nonce: process_nonce.into(),
        };
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (sender, receiver) = mpsc::sync_channel(QUEUE_LIMIT);
        let cursor_stop_capture: CursorStopCaptureSlot = Arc::new(Mutex::new(None));
        let decisions: DecisionsSlot = Arc::new(Mutex::new(None));
        let decision_slot = decisions.clone();
        let capture = cursor_stop_capture.clone();
        let captured = identity.clone();
        let credential = token.clone();
        let stopped = cancelled.clone();
        let expected = expected_native_session.map(str::to_owned);
        let worker = std::thread::Builder::new()
            .name("terminal-agent-hooks".into())
            .spawn(move || {
                listen(
                    listener,
                    cancellation,
                    stopped,
                    captured,
                    credential,
                    expected,
                    (sender, capture, decision_slot),
                );
            })
            .map_err(|_| failure())?;
        Ok((
            Self {
                directory: directory.into(),
                token,
                identity,
                native_version: OnceLock::new(),
                cancelled,
                wake,
                worker: Some(worker),
                cursor_stop_capture,
                decisions,
            },
            receiver,
        ))
    }

    pub fn native_version(&self) -> Option<&str> {
        self.native_version.get().map(String::as_str)
    }

    /// Add only private invocation configuration. Account, project and selected-plugin hooks keep
    /// their native merge rules. The token is inherited through the process environment, not argv.
    pub fn configure(
        &self,
        command: &mut Command,
        hook_probe: Option<&mut Command>,
    ) -> Result<(), String> {
        let executable = std::env::current_exe().map_err(|_| failure())?;
        self.configure_with_helper(command, hook_probe, &executable)
    }

    /// A host may inject its standalone helper executable explicitly, without an environment
    /// override that native children or unrelated launches could accidentally inherit.
    pub(crate) fn configure_with_helper(
        &self,
        command: &mut Command,
        hook_probe: Option<&mut Command>,
        executable: &Path,
    ) -> Result<(), String> {
        let version = prometeu_protocols::catalog::command_output(
            &prometeu_process::query::UnixQueryLauncher,
            Command::new(command.get_program()).arg("--version"),
            Duration::from_secs(5),
        )
        .map_err(|_| i18n::t("err.terminalAgent.version"))?;
        if !supported_version(self.identity.provider, &version) {
            return Err(i18n::t("err.terminalAgent.version"));
        }
        self.native_version
            .set(version.trim().to_owned())
            .map_err(|_| failure())?;
        if self.identity.provider == ProviderId::Codex {
            let helper = format!("{} --terminal-agent-hook", shell_quote(executable)?);
            configure_codex_hooks(command, hook_probe.ok_or_else(failure)?, &helper)?;
        }
        self.configure_with_executable(command, executable)
    }

    /// Configure a CLI that runs on a remote machine (ADR 0101): `helper` is the hook command on
    /// that machine and `socket` is where it reaches the machine's host. Returns the Claude
    /// invocation settings, which the caller writes on the machine and passes with `--settings`.
    /// Only Claude is supported remotely.
    pub(crate) fn configure_remote(
        &self,
        command: &mut Command,
        version: &str,
        helper: &str,
    ) -> Result<Value, String> {
        if self.identity.provider != ProviderId::Claude
            || !supported_version(self.identity.provider, version)
        {
            return Err(i18n::t("err.terminalAgent.version"));
        }
        let _ = self.native_version.set(version.trim().to_owned());
        command
            .env(TOKEN_ENV, &self.token)
            .env(SESSION_ENV, &self.identity.session)
            .env(NONCE_ENV, &self.identity.nonce)
            .env(PROVIDER_ENV, provider_key(self.identity.provider)?);
        Ok(claude_config(helper))
    }

    pub(crate) fn relay_target(&self) -> Result<RelayTarget, String> {
        Ok(RelayTarget {
            socket: self
                .directory
                .join("socket")
                .to_str()
                .ok_or_else(failure)?
                .to_owned(),
            token: self.token.clone(),
            nonce: self.identity.nonce.clone(),
        })
    }

    /// Record the native version of a remote CLI this bridge observes without starting it.
    pub(crate) fn adopt_remote(&self, version: &str) -> Result<(), String> {
        if !supported_version(self.identity.provider, version) {
            return Err(i18n::t("err.terminalAgent.version"));
        }
        let _ = self.native_version.set(version.trim().to_owned());
        Ok(())
    }

    /// Install persistent invocation instructions where native discovery loads them before any
    /// prompt, including text typed directly in the TUI. The bridge owns the files' lifetime.
    pub fn configure_instructions(
        &self,
        _command: &mut Command,
        instructions: &str,
    ) -> Result<(), String> {
        if instructions.is_empty() {
            return Ok(());
        }
        if self.identity.provider == ProviderId::Cursor {
            // This native build loads rules only from its primary workspace. Its supported
            // beforeSubmitPrompt context carrier runs for both fresh and resumed native input.
            // The carrier measures JavaScript UTF-16 length and drops anything over 10,000.
            if instructions.encode_utf16().count() > 10_000 {
                return Err(i18n::t("err.terminalAgent.unsupported"));
            }
            let path = self.directory.join("cursor-instructions.json");
            paths::write_private(
                &path,
                &json!({"additional_context":instructions}).to_string(),
            )
            .map_err(|_| failure())?;
            let config_path = self.directory.join("hooks/hooks.json");
            let mut config: Value = serde_json::from_str(
                &std::fs::read_to_string(&config_path).map_err(|_| failure())?,
            )
            .map_err(|_| failure())?;
            // There is no policy override in this response. Other hooks retain their native
            // blocking/merge behavior, and instruction contents never become shell syntax.
            config["hooks"]["beforeSubmitPrompt"] = json!([{
                "command":format!("/bin/cat {}", shell_quote(&path)?), "timeout":3,
            }]);
            return paths::write_private(&config_path, &config.to_string()).map_err(|_| failure());
        }
        let (path, header) = match self.identity.provider {
            ProviderId::Antigravity => (
                self.directory.join(".agents/rules/prometeu-session-instructions.md"),
                "---\ntrigger: always_on\ndescription: Instructions configured for this Prometeu conversation\n---\n",
            ),
            // Claude and Codex instructions are already native invocation flags, not rule files.
            _ => return Err(failure()),
        };
        let body = format!("{header}{instructions}");
        // Antigravity truncates individual rules at 24,000 bytes. Refuse that loss explicitly.
        if self.identity.provider == ProviderId::Antigravity && body.len() > 24_000 {
            return Err(i18n::t("err.terminalAgent.unsupported"));
        }
        paths::write_private(&path, &body).map_err(|_| failure())?;
        Ok(())
    }

    fn configure_with_executable(
        &self,
        command: &mut Command,
        executable: &Path,
    ) -> Result<(), String> {
        let helper = format!("{} --terminal-agent-hook", shell_quote(executable)?);
        match self.identity.provider {
            ProviderId::Claude => {
                let settings = self.directory.join("settings.json");
                paths::write_private(&settings, &claude_config(&helper).to_string())
                    .map_err(|_| failure())?;
                command.arg("--settings").arg(settings);
            }
            ProviderId::Cursor => {
                let manifest = self.directory.join(".cursor-plugin/plugin.json");
                paths::write_private(&manifest, &json!({
                    "name": "prometeu-native-observer", "version": "1.0.0", "hooks": "./hooks/hooks.json"
                }).to_string()).map_err(|_| failure())?;
                paths::write_private(
                    &self.directory.join("hooks/hooks.json"),
                    &cursor_config(&helper).to_string(),
                )
                .map_err(|_| failure())?;
                command.arg("--plugin-dir").arg(&self.directory);
            }
            ProviderId::Antigravity => {
                paths::write_private(
                    &self.directory.join(".agents/hooks.json"),
                    &antigravity_config(&helper, &self.identity.nonce).to_string(),
                )
                .map_err(|_| failure())?;
                // Added workspace roots participate in native customization discovery without
                // changing HOME or the credential authority. Each process owns a different root.
                command.arg("--add-dir").arg(&self.directory);
            }
            ProviderId::Codex => {}
            _ => return Err(failure()),
        }
        command
            .env(SOCKET_ENV, self.directory.join("socket"))
            .env(TOKEN_ENV, &self.token)
            .env(SESSION_ENV, &self.identity.session)
            .env(NONCE_ENV, &self.identity.nonce)
            .env(PROVIDER_ENV, provider_key(self.identity.provider)?);
        Ok(())
    }
}

impl Drop for HookBridge {
    fn drop(&mut self) {
        self.close();
    }
}

fn listen(
    listener: UnixListener,
    cancellation: UnixStream,
    cancelled: Arc<AtomicBool>,
    identity: Identity,
    token: String,
    mut native_session: Option<String>,
    delivery: (
        SyncSender<NativeHookEvent>,
        CursorStopCaptureSlot,
        DecisionsSlot,
    ),
) {
    let (sender, cursor_stop_capture, decisions) = delivery;
    let mut workers: Vec<JoinHandle<()>> = vec![];
    while !cancelled.load(Ordering::Acquire) {
        let mut ready = [
            libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: cancellation.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // Both descriptors remain owned by this worker throughout the poll.
        if unsafe { libc::poll(ready.as_mut_ptr(), ready.len() as _, -1) } < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if ready[1].revents != 0 || cancelled.load(Ordering::Acquire) {
            break;
        }
        let Ok((mut stream, _)) = listener.accept() else {
            continue;
        };
        let deadline = Instant::now() + DEADLINE;
        let event = read_fd(
            stream.as_raw_fd(),
            WIRE_LIMIT,
            deadline,
            Some(&cancelled),
            true,
        )
        .and_then(|bytes| decode(&bytes, &identity, &token, &mut native_session));
        let Ok(mut event) = event else {
            continue;
        };
        workers.retain(|worker| !worker.is_finished());
        let pending = crate::lock::lock(&decisions).as_ref().and_then(|registry| {
            if event.agent_id.is_none()
                && matches!(
                    event.kind,
                    NativeHookKind::StopObserved
                        | NativeHookKind::SessionEnded
                        | NativeHookKind::TurnFailed
                )
            {
                registry.withdraw();
            }
            registry.open(&event)
        });
        if event.provider == ProviderId::Cursor
            && event.kind == NativeHookKind::StopObserved
            && event.agent_id.is_none()
            && event.reason.as_deref() == Some("completed")
        {
            if let Some(capture) = crate::lock::lock(&cursor_stop_capture).as_mut() {
                event.cursor_stop_stamp = capture(&event);
            }
        }
        // Overflow ends this observation stream. The host sees disconnection instead of receiving
        // a later lifecycle event after an invisible gap. Native CLI execution remains unaffected.
        if cancelled.load(Ordering::Acquire) || sender.try_send(event).is_err() {
            break;
        }
        let _ = stream.set_nonblocking(true);
        if let Some(pending) = pending {
            let stopped = cancelled.clone();
            match std::thread::Builder::new()
                .name("terminal-agent-decision".into())
                .spawn(move || {
                    serve_decision(stream, pending, &stopped);
                }) {
                Ok(worker) => workers.push(worker),
                Err(_) => break,
            }
        } else {
            let _ = write_until(&mut stream, b"ok\n", deadline);
        }
    }
    if let Some(registry) = crate::lock::lock(&decisions).as_ref() {
        registry.close();
    }
    for worker in workers {
        let _ = worker.join();
    }
}

fn serve_decision(mut stream: UnixStream, pending: Pending, stopped: &AtomicBool) {
    let deadline = Instant::now() + DECISION_WAIT;
    while pending.active() && !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
        match pending.receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(delivery) => {
                let accepted = if pending.active() && !stopped.load(Ordering::Acquire) {
                    let deadline = Instant::now() + DEADLINE;
                    let mut bytes = serde_json::to_vec(&delivery.output).unwrap_or_default();
                    bytes.push(b'\n');
                    bytes.len() <= WIRE_LIMIT
                        && write_until(&mut stream, &bytes, deadline).is_ok()
                        && read_fd(stream.as_raw_fd(), 16, deadline, Some(stopped), true)
                            .is_ok_and(|bytes| bytes == b"written\n")
                } else {
                    false
                };
                pending.deliver(delivery, accepted);
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut descriptor = libc::pollfd {
                    fd: stream.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                // A helper sends no bytes until receiving its decision; readability here is EOF
                // or invalid protocol input. Neither may keep an obsolete decision card alive.
                if unsafe { libc::poll(&mut descriptor, 1, 0) } > 0 && descriptor.revents != 0 {
                    break;
                }
            }
        }
    }
    // No decision means native fallback. Timeout, retirement and local completion never allow.
    let _ = write_until(&mut stream, b"ok\n", Instant::now() + DEADLINE);
}

fn decode(
    bytes: &[u8],
    identity: &Identity,
    token: &str,
    native_session: &mut Option<String>,
) -> Result<NativeHookEvent, String> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| failure())?;
    let event = envelope.event;
    if envelope.version != 1
        || !same_token(&envelope.token, token)
        || event.provider != identity.provider
        || event.session != identity.session
        || event.process_nonce != identity.nonce
        || !valid_id(&event.native_session)
        || native_session
            .as_ref()
            .is_some_and(|id| id != &event.native_session)
        || event
            .transcript_path
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
    {
        return Err(failure());
    }
    // Cursor's sessionStart is asynchronous. Any authenticated native hook can prove identity;
    // readiness and prompt admission are independent and must not be inferred from this binding.
    native_session.get_or_insert_with(|| event.native_session.clone());
    Ok(event)
}

/// Entry point before desktop initialization. Antigravity requires a neutral JSON acknowledgement;
/// other providers need no response. Observation failures never change the native policy.
pub fn run() -> Result<(), String> {
    let event = if std::env::var(PROVIDER_ENV).ok().as_deref() == Some("antigravity") {
        Some(environment(EVENT_ENV)?)
    } else {
        None
    };
    let acknowledgement = event
        .as_deref()
        .map(antigravity_acknowledgement)
        .transpose()?;
    let result = run_observer(event.as_deref());
    if let Some(acknowledgement) = acknowledgement {
        io::stdout()
            .write_all(acknowledgement)
            .map_err(|_| failure())?;
    }
    result
}

fn run_observer(antigravity_event: Option<&str>) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    let bytes = read_fd(libc::STDIN_FILENO, INPUT_LIMIT, deadline, None, false)?;
    observe(
        &|key: &str| std::env::var(key).ok(),
        antigravity_event,
        &bytes,
        None,
        &mut |decision: &[u8]| {
            io::stdout().write_all(decision).map_err(|_| failure())?;
            io::stdout().flush().map_err(|_| failure())
        },
    )
}

/// Forward one hook that ran on a remote machine (ADR 0101) to the bridge of the conversation
/// that owns it, as if its helper had run here. `variables` are the hook process's
/// `PROMETEU_TERMINAL_HOOK_*` values; the bridge's own socket, credential and process nonce
/// replace the remote ones, since this app's bridge outlives no app restart while the remote
/// process does. Returns what the remote helper must print: a decision, or nothing. Setting
/// `resolved` while a permission request waits withdraws it: another computer answered it.
pub(crate) fn relay(
    target: &RelayTarget,
    variables: &std::collections::HashMap<String, String>,
    payload: &[u8],
    resolved: Option<&AtomicBool>,
) -> Result<Vec<u8>, String> {
    let environment = |key: &str| match key {
        SOCKET_ENV => Some(target.socket.clone()),
        TOKEN_ENV => Some(target.token.clone()),
        NONCE_ENV => Some(target.nonce.clone()),
        _ => variables.get(key).cloned(),
    };
    let mut decision = Vec::new();
    observe(
        &environment,
        None,
        payload,
        resolved,
        &mut |bytes: &[u8]| {
            decision.extend_from_slice(bytes);
            Ok(())
        },
    )?;
    Ok(decision)
}

type Environment<'a> = &'a dyn Fn(&str) -> Option<String>;

fn observe(
    environment: Environment<'_>,
    antigravity_event: Option<&str>,
    bytes: &[u8],
    withdrawn: Option<&AtomicBool>,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let deadline = Instant::now() + DEADLINE;
    let variable = |key: &str| environment(key).ok_or_else(failure);
    let mut payload: Value = serde_json::from_slice(bytes).map_err(|_| failure())?;
    let provider = match environment(PROVIDER_ENV).as_deref() {
        Some("claude") => ProviderId::Claude,
        Some("codex") => ProviderId::Codex,
        Some("cursor") => ProviderId::Cursor,
        Some("antigravity") => ProviderId::Antigravity,
        _ => return Err(failure()),
    };
    let identity = Identity {
        provider,
        session: variable(SESSION_ENV)?,
        nonce: variable(NONCE_ENV)?,
    };
    if provider == ProviderId::Antigravity {
        // A resumed CLI may retain an older added root. Its command carries its original nonce,
        // so even inherited credentials from the new process cannot reactivate a stale observer.
        if variable(EXPECTED_NONCE_ENV)? != identity.nonce {
            return Err(failure());
        }
        let event = antigravity_event.ok_or_else(failure)?;
        antigravity_acknowledgement(event)?;
        payload
            .as_object_mut()
            .ok_or_else(failure)?
            .insert("hook_event_name".into(), Value::String(event.into()));
    }
    let event = normalize(&identity, &payload)?;
    let permission =
        provider == ProviderId::Claude && event.kind == NativeHookKind::PermissionRequested;
    let envelope = Envelope {
        version: 1,
        token: variable(TOKEN_ENV)?,
        event,
    };
    let socket = PathBuf::from(variable(SOCKET_ENV)?);
    let mut stream = connect(&socket, deadline)?;
    let mut body = serde_json::to_vec(&envelope).map_err(|_| failure())?;
    body.push(b'\n');
    if body.len() > WIRE_LIMIT {
        return Err(failure());
    }
    write_until(&mut stream, &body, deadline)?;
    let deadline = if permission {
        Instant::now() + DECISION_WAIT + DEADLINE
    } else {
        deadline
    };
    // Withdrawing closes this connection, so the bridge drops the decision card it opened.
    let response = read_fd(
        stream.as_raw_fd(),
        if permission { WIRE_LIMIT } else { 16 },
        deadline,
        withdrawn,
        true,
    )?;
    if permission && response != b"ok\n" {
        let value: Value = serde_json::from_slice(&response).map_err(|_| failure())?;
        if value
            .pointer("/hookSpecificOutput/hookEventName")
            .and_then(Value::as_str)
            != Some("PermissionRequest")
        {
            return Err(failure());
        }
        emit(&response)?;
        write_until(&mut stream, b"written\n", Instant::now() + DEADLINE)?;
        return Ok(());
    }
    if response != b"ok\n" {
        return Err(failure());
    }
    Ok(())
}

fn normalize(identity: &Identity, payload: &Value) -> Result<NativeHookEvent, String> {
    let hook = payload["hook_event_name"].as_str().ok_or_else(failure)?;
    let (kind, native_id, output, text) = match identity.provider {
        ProviderId::Claude => {
            let kind = match hook {
                "SessionStart" => NativeHookKind::SessionStarted,
                "UserPromptSubmit" => NativeHookKind::PromptSubmitted,
                "Stop" => NativeHookKind::StopObserved,
                "PermissionRequest" => NativeHookKind::PermissionRequested,
                "PreToolUse" => NativeHookKind::ToolStarted,
                "PostToolUse" => NativeHookKind::ToolFinished,
                "PostToolUseFailure" => NativeHookKind::ToolFailed,
                "SessionEnd" => NativeHookKind::SessionEnded,
                "StopFailure" => NativeHookKind::TurnFailed,
                "Notification" => NativeHookKind::Notification,
                _ => return Err(failure()),
            };
            (
                kind,
                "session_id",
                "tool_response",
                "last_assistant_message",
            )
        }
        ProviderId::Cursor => {
            let kind = match hook {
                "sessionStart" => NativeHookKind::SessionStarted,
                "beforeSubmitPrompt" => NativeHookKind::PromptSubmitted,
                "stop" => NativeHookKind::StopObserved,
                "postToolUse" => NativeHookKind::ToolFinished,
                "postToolUseFailure" => NativeHookKind::ToolFailed,
                "sessionEnd" => NativeHookKind::SessionEnded,
                "afterAgentResponse" => NativeHookKind::AssistantResponse,
                _ => return Err(failure()),
            };
            (kind, "conversation_id", "tool_output", "text")
        }
        ProviderId::Codex => {
            let kind = match hook {
                "SessionStart" => NativeHookKind::SessionStarted,
                "SessionEnd" => NativeHookKind::SessionEnded,
                "UserPromptSubmit" => NativeHookKind::PromptSubmitted,
                "PreToolUse" => NativeHookKind::ToolStarted,
                "PostToolUse" => NativeHookKind::ToolFinished,
                "PermissionRequest" => NativeHookKind::PermissionRequested,
                "Stop" => NativeHookKind::StopObserved,
                _ => return Err(failure()),
            };
            (
                kind,
                "session_id",
                "tool_response",
                "last_assistant_message",
            )
        }
        ProviderId::Antigravity => {
            let kind = match hook {
                "SessionStart" => NativeHookKind::SessionStarted,
                "PreInvocation" => NativeHookKind::ModelInvocationStarted,
                "PostInvocation" => NativeHookKind::ModelInvocationFinished,
                "PostToolUse" if optional_text(payload, "error").is_some_and(|e| !e.is_empty()) => {
                    NativeHookKind::ToolFailed
                }
                "PostToolUse" => NativeHookKind::ToolFinished,
                "Stop" => NativeHookKind::StopObserved,
                _ => return Err(failure()),
            };
            (
                kind,
                "conversationId",
                "tool_response",
                if kind == NativeHookKind::StopObserved {
                    "finalModelOutput"
                } else {
                    "message"
                },
            )
        }
        _ => return Err(failure()),
    };
    let native_session = payload[native_id]
        .as_str()
        .filter(|id| valid_id(id))
        .ok_or_else(failure)?;
    let antigravity = identity.provider == ProviderId::Antigravity;
    let transcript_path = optional_text(
        payload,
        if antigravity {
            "transcriptPath"
        } else {
            "transcript_path"
        },
    )
    .map(PathBuf::from);
    if transcript_path
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(failure());
    }
    Ok(NativeHookEvent {
        provider: identity.provider,
        session: identity.session.clone(),
        process_nonce: identity.nonce.clone(),
        native_session: native_session.into(),
        kind,
        transcript_path,
        prompt: optional_text(
            payload,
            if antigravity {
                "lastUserInput"
            } else {
                "prompt"
            },
        ),
        tool_name: if antigravity {
            optional_text(&payload["toolCall"], "name")
        } else {
            optional_text(payload, "tool_name")
        },
        tool_use_id: optional_text(payload, "tool_use_id"),
        tool_input: if antigravity {
            optional_value(&payload["toolCall"], "args")
        } else {
            optional_value(payload, "tool_input")
        },
        tool_response: optional_value(payload, output),
        text: optional_text(payload, text).or_else(|| {
            (kind != NativeHookKind::StopObserved)
                .then(|| optional_text(payload, "message"))
                .flatten()
        }),
        generation_id: optional_text(
            payload,
            if identity.provider == ProviderId::Codex {
                "turn_id"
            } else {
                "generation_id"
            },
        ),
        agent_id: optional_text(payload, "agent_id"),
        source: optional_text(payload, "source"),
        reason: optional_text(
            payload,
            if antigravity {
                "terminationReason"
            } else {
                "reason"
            },
        )
        .or_else(|| optional_text(payload, "status")),
        notification_type: optional_text(payload, "notification_type"),
        stop_hook_active: payload["stop_hook_active"].as_bool(),
        error: optional_value(payload, "error"),
        step_index: payload["stepIdx"].as_u64(),
        invocation_number: payload["invocationNum"].as_u64(),
        initial_num_steps: payload["initialNumSteps"].as_u64(),
        execution_number: payload["executionNum"].as_u64(),
        fully_idle: payload["fullyIdle"].as_bool(),
        continuation_active: payload["continuationActive"].as_bool(),
        cursor_stop_stamp: None,
    })
}

/// The host launches Claude for agents with the same settings, so either side can observe a
/// conversation the other started (ADR 0103).
fn claude_config(helper: &str) -> Value {
    prometeu_host::claude_hook_settings(helper)
}

fn cursor_config(helper: &str) -> Value {
    // Cursor's preToolUse is a policy hook and cannot safely be observed with an empty response.
    // Prompt submission is recovered from its transcript until a neutral response is verified.
    let hooks = [
        "sessionStart",
        "sessionEnd",
        "postToolUse",
        "postToolUseFailure",
        "afterAgentResponse",
        "stop",
    ]
    .into_iter()
    .map(|name| (name.into(), json!([{"command":helper,"timeout":3}])))
    .collect::<Map<_, _>>();
    json!({"version":1,"hooks":hooks})
}

fn antigravity_config(helper: &str, nonce: &str) -> Value {
    let events = [
        "SessionStart",
        "PreInvocation",
        "PostInvocation",
        "PostToolUse",
        "Stop",
    ]
    .into_iter()
    .map(|event| {
        let command = format!(
            "{PROVIDER_ENV}='antigravity' {EVENT_ENV}={} {EXPECTED_NONCE_ENV}={} {helper}",
            quote_shell_text(event),
            quote_shell_text(nonce)
        );
        let handler = json!({"type":"command", "command":command, "timeout":3});
        let handlers = if event == "PostToolUse" {
            json!([{"matcher":"*", "hooks":[handler]}])
        } else {
            json!([handler])
        };
        (event.into(), handlers)
    })
    .collect::<Map<_, _>>();
    // PreToolUse requires an approval decision, so it is deliberately not observed here.
    json!({"prometeu-native-observer":events})
}

fn antigravity_acknowledgement(event: &str) -> Result<&'static [u8], String> {
    match event {
        "Stop" => Ok(b"{\"decision\":\"\"}\n"),
        "SessionStart" | "PreInvocation" | "PostInvocation" | "PostToolUse" => Ok(b"{}\n"),
        _ => Err(failure()),
    }
}

const CODEX_HOOK_EVENTS: [(&str, &str); 7] = [
    ("SessionStart", "sessionStart"),
    ("SessionEnd", "sessionEnd"),
    ("UserPromptSubmit", "userPromptSubmit"),
    ("PreToolUse", "preToolUse"),
    ("PostToolUse", "postToolUse"),
    ("PermissionRequest", "permissionRequest"),
    ("Stop", "stop"),
];

fn codex_declarations(command: &mut Command, helper: &str) {
    command.args(["--enable", "hooks"]);
    for (event, _) in CODEX_HOOK_EVENTS {
        command.arg("-c").arg(format!(
            "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout=3}}]}}]",
            toml::Value::String(helper.into())
        ));
    }
}

fn codex_trust(command: &mut Command, trusts: &[(String, String)]) {
    // The CLI splits dotted override paths before TOML decoding, including dots inside quotes.
    // Put opaque native keys inside one inline table; hook state still merges across config layers.
    let state = trusts
        .iter()
        .map(|(key, hash)| {
            format!(
                "{}={{enabled=true,trusted_hash={}}}",
                toml::Value::String(key.clone()),
                toml::Value::String(hash.clone())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    command.arg("-c").arg(format!("hooks.state={{{state}}}"));
}

fn configure_codex_hooks(
    command: &mut Command,
    probe: &mut Command,
    helper: &str,
) -> Result<(), String> {
    // Codex discovers each config layer separately, appending hooks from user/project/plugins.
    // These arrays belong only to SessionFlags; trusting one declaration never trusts its layer.
    codex_declarations(probe, helper);
    let first = codex_hooks_list(probe)?;
    let trusts = codex_owned_hook_trust(&first, helper, false)?;
    codex_trust(probe, &trusts);
    let verified = codex_owned_hook_trust(&codex_hooks_list(probe)?, helper, true)?;
    if verified != trusts {
        return Err(failure());
    }
    codex_declarations(command, helper);
    codex_trust(command, &trusts);
    Ok(())
}

fn codex_hooks_list(command: &mut Command) -> Result<Value, String> {
    let cwd = command
        .get_current_dir()
        .and_then(Path::to_str)
        .ok_or_else(failure)?
        .to_owned();
    let mut process = CatalogProcess::spawn(
        &prometeu_process::query::UnixQueryLauncher,
        command,
        Duration::from_secs(20),
    )
    .map_err(|_| failure())?;
    process
        .send(json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"prometeu","title":"Prometeu","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"experimentalApi":true}
        }}))
        .map_err(|_| failure())?;
    process.response(1).map_err(|_| failure())?;
    process
        .send(json!({"method":"initialized","params":{}}))
        .map_err(|_| failure())?;
    process
        .send(json!({"id":2,"method":"hooks/list","params":{"cwds":[cwd]}}))
        .map_err(|_| failure())?;
    process.response(2).map_err(|_| failure())
}

fn codex_owned_hook_trust(
    response: &Value,
    helper: &str,
    require_trusted: bool,
) -> Result<Vec<(String, String)>, String> {
    let mut owned = std::collections::BTreeMap::new();
    for hook in response["data"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
    {
        if hook["source"] != "sessionFlags" || hook["command"] != helper {
            continue;
        }
        let event = hook["eventName"].as_str().ok_or_else(failure)?;
        if !CODEX_HOOK_EVENTS.iter().any(|(_, wire)| *wire == event)
            || hook["sourcePath"] != "/<session-flags>/config.toml"
            || hook["handlerType"] != "command"
            || hook["async"].as_bool().unwrap_or(false)
            || hook["isManaged"] != false
            || !hook["pluginId"].is_null()
            || !hook["environmentId"].is_null()
            || !hook["matcher"].is_null()
            || hook["timeoutSec"] != 3
            || (require_trusted && (hook["enabled"] != true || hook["trustStatus"] != "trusted"))
        {
            return Err(failure());
        }
        let key = hook["key"]
            .as_str()
            .filter(|key| valid_id(key))
            .ok_or_else(failure)?;
        let hash = hook["currentHash"]
            .as_str()
            .filter(|hash| valid_id(hash))
            .ok_or_else(failure)?;
        if owned
            .insert(event.to_owned(), (key.to_owned(), hash.to_owned()))
            .is_some()
        {
            return Err(failure());
        }
    }
    if owned.len() != CODEX_HOOK_EVENTS.len() {
        return Err(failure());
    }
    let values = owned.into_values().collect::<Vec<_>>();
    if values
        .iter()
        .map(|(key, _)| key)
        .collect::<std::collections::HashSet<_>>()
        .len()
        != values.len()
    {
        return Err(failure());
    }
    Ok(values)
}

fn optional_text(value: &Value, field: &str) -> Option<String> {
    value[field].as_str().map(str::to_owned)
}
fn optional_value(value: &Value, field: &str) -> Option<Value> {
    value.get(field).filter(|value| !value.is_null()).cloned()
}
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)
}
fn environment(key: &str) -> Result<String, String> {
    std::env::var(key)
        .ok()
        .filter(|value| valid_id(value))
        .ok_or_else(failure)
}
fn provider_key(provider: ProviderId) -> Result<&'static str, String> {
    match provider {
        ProviderId::Claude => Ok("claude"),
        ProviderId::Codex => Ok("codex"),
        ProviderId::Cursor => Ok("cursor"),
        ProviderId::Antigravity => Ok("antigravity"),
        _ => Err(failure()),
    }
}

fn supported_version(provider: ProviderId, raw: &str) -> bool {
    let (at_least, min): (fn(&str, &str) -> bool, _) = match provider {
        ProviderId::Claude => (prometeu_protocols::cli_version::at_least, "2.1.326"),
        ProviderId::Antigravity => (prometeu_protocols::cli_version::at_least, "1.5.9"),
        ProviderId::Codex => (prometeu_protocols::cli_version::at_least, "0.169.0"),
        ProviderId::Cursor => (
            prometeu_protocols::cli_version::build_date_at_least,
            "2026.11.13",
        ),
        _ => return false,
    };
    at_least(raw, min)
}
fn failure() -> String {
    i18n::t("err.terminalAgent.hooks")
}
fn same_token(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}
fn shell_quote(path: &Path) -> Result<String, String> {
    let path = path.to_str().ok_or_else(failure)?;
    Ok(quote_shell_text(path))
}
fn quote_shell_text(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn read_fd(
    fd: RawFd,
    limit: usize,
    deadline: Instant,
    cancelled: Option<&AtomicBool>,
    frame: bool,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(failure());
        }
        wait_fd(fd, libc::POLLIN, deadline, cancelled)?;
        // The caller owns fd, and buffer is writable for the requested length.
        let read = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read < 0 {
            if matches!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            return Err(failure());
        }
        if read == 0 {
            return Ok(bytes);
        }
        let chunk = &buffer[..read as usize];
        let count = if frame {
            chunk
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(chunk.len(), |index| index + 1)
        } else {
            chunk.len()
        };
        if bytes.len() + count > limit {
            return Err(failure());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if frame && bytes.last() == Some(&b'\n') {
            return Ok(bytes);
        }
    }
}

fn wait_fd(
    fd: RawFd,
    events: i16,
    deadline: Instant,
    cancelled: Option<&AtomicBool>,
) -> Result<(), String> {
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(failure());
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(failure)?;
        let millis = remaining.as_millis().clamp(1, 100) as i32;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // Poll never outlives the borrowed descriptor.
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if result < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(failure());
        }
        if descriptor.revents & (events | libc::POLLHUP) != 0 {
            return Ok(());
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(failure());
        }
    }
}

fn write_until(stream: &mut UnixStream, mut bytes: &[u8], deadline: Instant) -> Result<(), String> {
    while !bytes.is_empty() {
        wait_fd(stream.as_raw_fd(), libc::POLLOUT, deadline, None)?;
        match stream.write(bytes) {
            Ok(0) => return Err(failure()),
            Ok(size) => bytes = &bytes[size..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => return Err(failure()),
        }
    }
    Ok(())
}

fn connect(path: &Path, deadline: Instant) -> Result<UnixStream, String> {
    let bytes = path.as_os_str().as_bytes();
    // sockaddr_un is initialized entirely before being passed to connect.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(failure());
    }
    address.sun_family = libc::AF_UNIX as _;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as _;
    }
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as _;
    }
    // The returned descriptor is immediately owned, including every error path below.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(failure());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    stream.set_nonblocking(true).map_err(|_| failure())?;
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(failure());
    }
    let connected = unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&address) as _,
        )
    };
    if connected < 0 {
        if io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(failure());
        }
        wait_fd(fd, libc::POLLOUT, deadline, None)?;
        let mut error: libc::c_int = 0;
        let mut length = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut length,
            )
        } < 0
            || error != 0
        {
            return Err(failure());
        }
    }
    Ok(stream)
}

#[cfg(test)]
#[path = "terminal_agent_hooks/smoke.rs"]
pub(crate) mod smoke;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::Shutdown, process::Stdio, sync::mpsc::TryRecvError};

    #[test]
    fn claude_waits_for_decisions_as_long_as_this_app_does() {
        assert_eq!(DECISION_WAIT.as_secs(), prometeu_host::DECISION_WAIT_SECS);
        let settings = claude_config("helper");
        let permission = &settings["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(permission["command"], "helper");
        assert_eq!(permission["timeout"], DECISION_WAIT.as_secs() + 5);
    }

    fn identity() -> Identity {
        Identity {
            provider: ProviderId::Claude,
            session: "logical-tab".into(),
            nonce: "incarnation-1".into(),
        }
    }

    fn payload(hook: &str) -> Value {
        json!({"hook_event_name":hook,"session_id":"native-session","transcript_path":"/tmp/native/history.jsonl"})
    }

    fn envelope(bridge: &HookBridge, payload: Value) -> Vec<u8> {
        let event = normalize(&bridge.identity, &payload).unwrap();
        let mut bytes = serde_json::to_vec(&Envelope {
            version: 1,
            token: bridge.token.clone(),
            event,
        })
        .unwrap();
        bytes.push(b'\n');
        bytes
    }

    fn deliver(bridge: &HookBridge, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut stream = connect(&bridge.directory.join("socket"), deadline)?;
        write_until(&mut stream, bytes, deadline)?;
        read_fd(stream.as_raw_fd(), 16, deadline, None, true)
    }

    #[test]
    fn claude_observations_preserve_content_and_do_not_claim_readiness_or_completion() {
        let mut input = payload("UserPromptSubmit");
        input["prompt"] = json!("Exact\nuser input with `$(literal)` and ünicode.");
        let prompt = normalize(&identity(), &input).unwrap();
        assert_eq!(prompt.kind, NativeHookKind::PromptSubmitted);
        assert_eq!(prompt.prompt.as_deref(), input["prompt"].as_str());
        assert_eq!(
            prompt.transcript_path.as_deref(),
            Some(Path::new("/tmp/native/history.jsonl"))
        );
        assert_eq!(prompt.native_session, "native-session");
        assert_eq!(prompt.session, "logical-tab");
        assert_eq!(prompt.process_nonce, "incarnation-1");
        let mut permission = payload("PermissionRequest");
        permission["tool_name"] = json!("Bash");
        permission["tool_input"] = json!({"command":"npm test", "description":"Run tests"});
        let permission = normalize(&identity(), &permission).unwrap();
        assert_eq!(permission.kind, NativeHookKind::PermissionRequested);
        assert_eq!(
            permission.tool_input,
            Some(json!({"command":"npm test", "description":"Run tests"}))
        );
        assert!(permission.tool_use_id.is_none());
        assert_eq!(
            normalize(&identity(), &payload("SessionStart"))
                .unwrap()
                .kind,
            NativeHookKind::SessionStarted
        );
        assert_eq!(
            normalize(&identity(), &payload("Stop")).unwrap().kind,
            NativeHookKind::StopObserved
        );
        assert!(normalize(&identity(), &payload("UnknownFutureEvent")).is_err());
    }

    #[test]
    fn cursor_uses_conversation_identity_and_keeps_native_tool_results() {
        let identity = Identity {
            provider: ProviderId::Cursor,
            ..identity()
        };
        let event = normalize(&identity, &json!({
            "hook_event_name":"postToolUse", "conversation_id":"cursor-conversation",
            "session_id":"different-process-id", "generation_id":"turn-generation",
            "transcript_path":"/tmp/cursor/transcript.jsonl", "tool_name":"Shell", "tool_use_id":"tool-1",
            "tool_input":{"command":"pwd"}, "tool_output":{"stdout":"/tmp/project\n"}
        })).unwrap();
        assert_eq!(event.native_session, "cursor-conversation");
        assert_eq!(event.generation_id.as_deref(), Some("turn-generation"));
        assert_eq!(event.kind, NativeHookKind::ToolFinished);
        assert_eq!(
            event.tool_response,
            Some(json!({"stdout":"/tmp/project\n"}))
        );
        assert!(normalize(
            &identity,
            &json!({"hook_event_name":"sessionStart","session_id":"ambiguous"})
        )
        .is_err());
    }

    fn codex_inventory(helper: &str) -> Value {
        // HookMetadata and event names follow rust-v0.169.0 HooksListResponse.json.
        let hooks = CODEX_HOOK_EVENTS.iter().enumerate().map(|(index, (_, event))| json!({
            "handlerType":"command", "command":helper, "async":false,
            "eventName":event, "source":"sessionFlags", "sourcePath":"/<session-flags>/config.toml",
            "key":format!("/<session-flags>/config.toml:fixture_event_{index}:0:0"),
            "currentHash":format!("sha256:fixture-hash-{index}"), "enabled":true,
            "trustStatus":"trusted", "isManaged":false, "pluginId":null,
            "environmentId":null, "matcher":null, "timeoutSec":3, "displayOrder":index
        })).collect::<Vec<_>>();
        json!({"data":[{"cwd":"/tmp/project", "hooks":hooks,"errors":[],"warnings":[]}]})
    }

    #[test]
    fn codex_trust_is_limited_to_exact_owned_session_declarations_and_current_hashes() {
        let helper = "'/tmp/Prometeu' --terminal-agent-hook";
        let mut inventory = codex_inventory(helper);
        for source in ["user", "project", "plugin"] {
            let mut other = inventory["data"][0]["hooks"][0].clone();
            other["source"] = json!(source);
            other["key"] = json!(format!("unrelated-{source}"));
            inventory["data"][0]["hooks"]
                .as_array_mut()
                .unwrap()
                .push(other);
        }
        let mut other = inventory["data"][0]["hooks"][0].clone();
        other["command"] = json!("other-session-command");
        inventory["data"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(other);
        let trusts = codex_owned_hook_trust(&inventory, helper, true).unwrap();
        assert_eq!(trusts.len(), CODEX_HOOK_EVENTS.len());
        assert!(trusts.iter().all(
            |(key, hash)| key.starts_with("/<session-flags>/config.toml:")
                && hash.starts_with("sha256:fixture-hash-")
        ));
        for (field, value) in [
            ("sourcePath", json!("/tmp/other/config.toml")),
            ("currentHash", Value::Null),
            ("key", Value::Null),
            ("pluginId", json!("another@plugin")),
            ("handlerType", json!("mcpTool")),
            ("async", json!(true)),
            ("isManaged", json!(true)),
            ("eventName", json!("unknownEvent")),
            ("timeoutSec", json!(30)),
        ] {
            let mut changed = codex_inventory(helper);
            changed["data"][0]["hooks"][0][field] = value;
            assert!(
                codex_owned_hook_trust(&changed, helper, false).is_err(),
                "invalid {field}"
            );
        }
        let mut disabled = codex_inventory(helper);
        disabled["data"][0]["hooks"][0]["enabled"] = json!(false);
        disabled["data"][0]["hooks"][0]["trustStatus"] = json!("modified");
        assert!(codex_owned_hook_trust(&disabled, helper, false).is_ok());
        assert!(codex_owned_hook_trust(&disabled, helper, true).is_err());
        let mut duplicate = codex_inventory(helper);
        let repeated = duplicate["data"][0]["hooks"][0].clone();
        duplicate["data"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(repeated);
        assert!(codex_owned_hook_trust(&duplicate, helper, false).is_err());
        let mut missing = codex_inventory(helper);
        missing["data"][0]["hooks"].as_array_mut().unwrap().pop();
        assert!(codex_owned_hook_trust(&missing, helper, false).is_err());
    }

    #[test]
    fn codex_session_config_adds_event_arrays_and_exact_trust_keys_without_replacing_other_layers()
    {
        let helper = "'/tmp/App with \"quotes\" and ünicode' --terminal-agent-hook";
        let mut command = Command::new("codex");
        command.args([
            "--no-daemon",
            "-c",
            "developer_instructions=\"Keep existing settings\"",
        ]);
        codex_declarations(&mut command, helper);
        let trusts = codex_owned_hook_trust(&codex_inventory(helper), helper, false).unwrap();
        codex_trust(&mut command, &trusts);
        let args = command
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(args.contains(&"developer_instructions=\"Keep existing settings\""));
        assert!(!args
            .iter()
            .any(|arg| arg.starts_with("hooks=") || arg.contains("bypass")));
        assert_eq!(
            args.iter()
                .filter(|arg| arg.starts_with("hooks.state="))
                .count(),
            1
        );
        assert!(!args.iter().any(|arg| arg.starts_with("hooks.state.")));
        let configs = args
            .windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| toml::from_str::<toml::Value>(pair[1]).unwrap())
            .collect::<Vec<_>>();
        for (event, _) in CODEX_HOOK_EVENTS {
            let config = configs
                .iter()
                .find(|config| {
                    config
                        .get("hooks")
                        .and_then(|hooks| hooks.get(event))
                        .is_some()
                })
                .unwrap();
            assert_eq!(
                config["hooks"][event][0]["hooks"][0]["command"].as_str(),
                Some(helper)
            );
        }
        for (key, hash) in trusts {
            let config = configs
                .iter()
                .find(|config| {
                    config
                        .get("hooks")
                        .and_then(|hooks| hooks.get("state"))
                        .and_then(|state| state.get(&key))
                        .is_some()
                })
                .unwrap();
            assert_eq!(
                config["hooks"]["state"][&key]["trusted_hash"].as_str(),
                Some(hash.as_str())
            );
            assert_eq!(
                config["hooks"]["state"][&key]["enabled"].as_bool(),
                Some(true)
            );
        }
    }

    #[test]
    fn codex_hook_payloads_keep_session_turn_transcript_and_subagent_identity() {
        let identity = Identity {
            provider: ProviderId::Codex,
            ..identity()
        };
        let mut input = json!({
            "hook_event_name":"SessionStart", "session_id":"native-session",
            "transcript_path":"/tmp/codex/rollout-native-session.jsonl", "source":"resume",
            "model":"gpt-example", "permission_mode":"default", "cwd":"/tmp/project"
        });
        let started = normalize(&identity, &input).unwrap();
        assert_eq!(started.kind, NativeHookKind::SessionStarted);
        assert_eq!(started.source.as_deref(), Some("resume"));
        input["hook_event_name"] = json!("UserPromptSubmit");
        input["turn_id"] = json!("native-turn");
        input["prompt"] = json!("Exact user input");
        input["agent_id"] = json!("child-agent");
        let submitted = normalize(&identity, &input).unwrap();
        assert_eq!(submitted.native_session, "native-session");
        assert_eq!(submitted.generation_id.as_deref(), Some("native-turn"));
        assert_eq!(submitted.agent_id.as_deref(), Some("child-agent"));
        assert_eq!(submitted.prompt.as_deref(), Some("Exact user input"));
        input["hook_event_name"] = json!("Stop");
        input["stop_hook_active"] = json!(true);
        input["last_assistant_message"] = json!("Native assistant response");
        let stopped = normalize(&identity, &input).unwrap();
        assert_eq!(stopped.kind, NativeHookKind::StopObserved);
        assert_eq!(stopped.stop_hook_active, Some(true));
        assert_eq!(stopped.text.as_deref(), Some("Native assistant response"));
        input["transcript_path"] = Value::Null;
        assert!(normalize(&identity, &input)
            .unwrap()
            .transcript_path
            .is_none());
    }

    #[test]
    fn antigravity_preserves_native_identity_without_treating_model_calls_as_user_submissions() {
        let identity = Identity {
            provider: ProviderId::Antigravity,
            ..identity()
        };
        let mut input = json!({
            "hook_event_name":"PreInvocation", "conversationId":"native-agy-session",
            "transcriptPath":"/tmp/agy/transcript.jsonl", "lastUserInput":"Exact\nuser prompt",
            "invocationNum":4, "initialNumSteps":8
        });
        let event = normalize(&identity, &input).unwrap();
        assert_eq!(event.kind, NativeHookKind::ModelInvocationStarted);
        assert_eq!(event.native_session, "native-agy-session");
        assert_eq!(event.prompt.as_deref(), Some("Exact\nuser prompt"));
        assert_eq!(event.invocation_number, Some(4));
        assert_eq!(event.initial_num_steps, Some(8));
        assert_eq!(
            event.transcript_path.as_deref(),
            Some(Path::new("/tmp/agy/transcript.jsonl"))
        );
        input["hook_event_name"] = json!("PostToolUse");
        input["toolCall"] = json!({"name":"run_command", "args":{"CommandLine":"pwd"}});
        input["stepIdx"] = json!(9);
        input["error"] = json!("Command failed");
        let failed = normalize(&identity, &input).unwrap();
        assert_eq!(failed.kind, NativeHookKind::ToolFailed);
        assert_eq!(failed.tool_name.as_deref(), Some("run_command"));
        assert_eq!(failed.tool_input, Some(json!({"CommandLine":"pwd"})));
        assert_eq!(failed.step_index, Some(9));
        input["hook_event_name"] = json!("Stop");
        input["executionNum"] = json!(2);
        input["fullyIdle"] = json!(false);
        input["terminationReason"] = json!("model_stop");
        input["finalModelOutput"] = json!("Exact final\nmodel output");
        input["continuationActive"] = json!(true);
        let stopped = normalize(&identity, &input).unwrap();
        assert_eq!(stopped.kind, NativeHookKind::StopObserved);
        assert_eq!(stopped.fully_idle, Some(false));
        assert_eq!(stopped.execution_number, Some(2));
        assert_eq!(stopped.reason.as_deref(), Some("model_stop"));
        assert_eq!(stopped.text.as_deref(), Some("Exact final\nmodel output"));
        assert_eq!(stopped.continuation_active, Some(true));
        input["finalModelOutput"] = Value::Null;
        input["message"] = json!("Unrelated metadata is not final model output");
        assert!(normalize(&identity, &input).unwrap().text.is_none());
        input["hook_event_name"] = json!("PreToolUse");
        assert!(normalize(&identity, &input).is_err());
        assert!(antigravity_acknowledgement("PreToolUse").is_err());
    }

    #[test]
    fn native_instruction_rules_are_private_always_active_and_owned_by_the_incarnation() {
        let (provider, relative, trigger) = (
            ProviderId::Antigravity,
            ".agents/rules/prometeu-session-instructions.md",
            "trigger: always_on",
        );
        let (mut bridge, _events) =
            HookBridge::start(provider, "tab", "incarnation", None).unwrap();
        let mut command = Command::new("native");
        bridge
            .configure_with_executable(&mut command, Path::new("/app/Prometeu"))
            .unwrap();
        let instructions = "Preserve this instruction exactly.\n---\n${VARIABLE} `literal text`";
        bridge
            .configure_instructions(&mut command, instructions)
            .unwrap();
        let path = bridge.directory.join(relative);
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with(&format!("---\n{trigger}\n")));
        assert!(body.ends_with(instructions));
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        bridge.close();
        assert!(!path.exists());
    }

    #[test]
    fn cursor_instructions_use_native_prompt_context_without_policy_or_shell_interpolation() {
        let (mut bridge, _) =
            HookBridge::start(ProviderId::Cursor, "tab", "incarnation", None).unwrap();
        let mut command = Command::new("cursor-agent");
        bridge
            .configure_with_executable(&mut command, Path::new("/app/Prometeu"))
            .unwrap();
        let instructions =
            "Keep `literal $(text)` and ${VARIABLE}.\nPreserve 'quotes' and \"quotes\".";
        bridge
            .configure_instructions(&mut command, instructions)
            .unwrap();
        let config: Value = serde_json::from_str(
            &std::fs::read_to_string(bridge.directory.join("hooks/hooks.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["hooks"]["sessionStart"].as_array().unwrap().len(), 1);
        assert!(config["hooks"].get("preToolUse").is_none());
        let hook = config["hooks"]["beforeSubmitPrompt"][0]["command"]
            .as_str()
            .unwrap();
        let output = Command::new("/bin/sh").args(["-c", hook]).output().unwrap();
        assert!(output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response, json!({"additional_context":instructions}));
        let path = bridge.directory.join("cursor-instructions.json");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(bridge
            .configure_instructions(&mut command, &"🦀".repeat(5_001))
            .is_err());
        assert!(bridge
            .configure_instructions(&mut command, &"🦀".repeat(5_000))
            .is_ok());
        bridge.close();
        assert!(!path.exists());
    }

    #[test]
    fn antigravity_config_uses_private_added_root_and_does_not_change_the_account_home() {
        let (mut first, events) = HookBridge::start(
            ProviderId::Antigravity,
            "logical-tab",
            "incarnation-1",
            None,
        )
        .unwrap();
        let (second, _) = HookBridge::start(
            ProviderId::Antigravity,
            "logical-tab",
            "incarnation-2",
            None,
        )
        .unwrap();
        assert_ne!(first.directory, second.directory);
        let mut command = Command::new("agy");
        command.env("HOME", "/original/account-home");
        first
            .configure_with_executable(&mut command, Path::new("/tmp/App's directory/Prometeu"))
            .unwrap();
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "--add-dir".to_owned(),
                first.directory.to_string_lossy().into_owned()
            ]
        );
        assert!(command
            .get_envs()
            .any(|(key, value)| key == "HOME" && value == Some("/original/account-home".as_ref())));
        let file = first.directory.join(".agents/hooks.json");
        let config: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let bundle = &config["prometeu-native-observer"];
        assert!(bundle.get("PreToolUse").is_none());
        assert_eq!(bundle["PostToolUse"][0]["matcher"], "*");
        let helper = bundle["SessionStart"][0]["command"].as_str().unwrap();
        assert!(helper.contains("PROMETEU_TERMINAL_HOOK_EXPECTED_NONCE='incarnation-1'"));
        assert!(helper.contains("PROMETEU_TERMINAL_HOOK_EVENT='SessionStart'"));
        assert!(helper.ends_with("'/tmp/App'\"'\"'s directory/Prometeu' --terminal-agent-hook"));
        assert!(!helper.contains(&first.token));
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        first.close();
        first.close();
        assert!(!first.directory.exists());
        assert!(second.directory.exists());
        assert!(matches!(events.try_recv(), Err(TryRecvError::Disconnected)));
    }

    #[test]
    fn antigravity_helper_acknowledges_without_policy_changes_and_rejects_retained_old_roots() {
        const CHILD: &str = "PROMETEU_AGY_HOOK_HELPER_TEST";
        if let Ok(mode) = std::env::var(CHILD) {
            let result = run();
            assert_eq!(
                result.is_ok(),
                !matches!(mode.as_str(), "stale" | "detached")
            );
            return;
        }
        let (bridge, events) = HookBridge::start(
            ProviderId::Antigravity,
            "logical-tab",
            "incarnation-2",
            Some("native-agy-session"),
        )
        .unwrap();
        let mut configured = Command::new("agy");
        bridge
            .configure_with_executable(&mut configured, &std::env::current_exe().unwrap())
            .unwrap();
        for (mode, event, expected_nonce, acknowledgement) in [
            ("start", "SessionStart", "incarnation-2", "{}\n"),
            ("stop", "Stop", "incarnation-2", "{\"decision\":\"\"}\n"),
            ("stale", "Stop", "incarnation-1", "{\"decision\":\"\"}\n"),
            ("detached", "Stop", "incarnation-2", "{\"decision\":\"\"}\n"),
        ] {
            let mut helper = Command::new(std::env::current_exe().unwrap());
            helper.args(["--exact", "terminal_agent_hooks::tests::antigravity_helper_acknowledges_without_policy_changes_and_rejects_retained_old_roots", "--nocapture"])
                .env(CHILD,mode).env(EVENT_ENV,event).env(EXPECTED_NONCE_ENV,expected_nonce)
                .envs(configured.get_envs().filter_map(|(key,value)| value.map(|value| (key,value))))
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            if mode == "detached" {
                helper
                    .env_remove(NONCE_ENV)
                    .env_remove(TOKEN_ENV)
                    .env_remove(SOCKET_ENV);
            }
            let mut child = helper.spawn().unwrap();
            child.stdin.take().unwrap().write_all(json!({
                "conversationId":"native-agy-session", "transcriptPath":"/tmp/agy/transcript.jsonl",
                "lastUserInput":"Private prompt content", "fullyIdle":true
            }).to_string().as_bytes()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "helper process failed");
            assert!(String::from_utf8_lossy(&output.stdout).contains(acknowledgement));
            for captured in [&output.stdout, &output.stderr] {
                let text = String::from_utf8_lossy(captured);
                assert!(!text.contains(&bridge.token));
                assert!(!text.contains("Private prompt content"));
            }
            if matches!(mode, "stale" | "detached") {
                assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
            } else {
                let delivered = events.recv_timeout(Duration::from_secs(1)).unwrap();
                assert_eq!(delivered.native_session, "native-agy-session");
                assert_eq!(delivered.prompt.as_deref(), Some("Private prompt content"));
            }
        }
    }

    #[test]
    fn listener_rejects_stale_credentials_identity_and_nonce_before_delivery() {
        let (bridge, events) = HookBridge::start(
            ProviderId::Claude,
            "logical-tab",
            "incarnation-1",
            Some("native-session"),
        )
        .unwrap();
        let good: Value =
            serde_json::from_slice(&envelope(&bridge, payload("SessionStart"))).unwrap();
        for pointer in [
            "/token",
            "/event/process_nonce",
            "/event/native_session",
            "/event/session",
        ] {
            let mut stale = good.clone();
            *stale.pointer_mut(pointer).unwrap() = json!("stale");
            let rejected = deliver(&bridge, format!("{stale}\n").as_bytes());
            assert!(rejected.is_err() || rejected.unwrap().is_empty());
            assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
        }
        assert_eq!(
            deliver(&bridge, &envelope(&bridge, payload("SessionStart"))).unwrap(),
            b"ok\n"
        );
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(event.native_session, "native-session");
        let serialized = serde_json::to_string(&event).unwrap();
        assert!(!serialized.contains(&bridge.token));
        assert!(!serialized.contains("\"token\""));
    }

    #[test]
    fn first_authenticated_hook_binds_identity_without_requiring_a_synchronous_start_hook() {
        let (bridge, events) =
            HookBridge::start(ProviderId::Claude, "logical-tab", "incarnation-1", None).unwrap();
        assert_eq!(
            deliver(&bridge, &envelope(&bridge, payload("UserPromptSubmit"))).unwrap(),
            b"ok\n"
        );
        assert_eq!(
            events
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .native_session,
            "native-session"
        );
        let mut other = payload("SessionStart");
        other["session_id"] = json!("another-session");
        assert!(deliver(&bridge, &envelope(&bridge, other))
            .unwrap()
            .is_empty());
        assert!(matches!(events.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn bridge_configuration_is_private_quoted_and_removed_on_drop() {
        for provider in [ProviderId::Claude, ProviderId::Cursor] {
            let (bridge, events) =
                HookBridge::start(provider, "logical-tab", "incarnation-1", None).unwrap();
            let directory = bridge.directory.clone();
            let mut command = Command::new("native-cli");
            bridge
                .configure_with_executable(&mut command, Path::new("/tmp/App's directory/Prometeu"))
                .unwrap();
            let file = directory.join(if provider == ProviderId::Claude {
                "settings.json"
            } else {
                "hooks/hooks.json"
            });
            let body = std::fs::read_to_string(&file).unwrap();
            let config: Value = serde_json::from_str(&body).unwrap();
            let hook = if provider == ProviderId::Claude {
                assert_eq!(config["env"]["CLAUDE_CODE_DISABLE_TERMINAL_TITLE"], "0");
                assert_eq!(config["env"]["CLAUDE_CODE_EAGER_FLUSH"], "1");
                &config["hooks"]["SessionStart"][0]["hooks"][0]
            } else {
                &config["hooks"]["sessionStart"][0]
            };
            assert_eq!(
                hook["command"],
                "'/tmp/App'\"'\"'s directory/Prometeu' --terminal-agent-hook"
            );
            assert!(!body.contains(&bridge.token));
            assert!(!command
                .get_args()
                .any(|arg| arg.to_string_lossy().contains(&bridge.token)));
            assert!(command
                .get_envs()
                .any(|(key, value)| key == TOKEN_ENV && value == Some(bridge.token.as_ref())));
            assert_eq!(
                std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for path in [&file, &directory.join("socket")] {
                assert_eq!(
                    std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            if provider == ProviderId::Cursor {
                assert!(config["hooks"].get("preToolUse").is_none());
                assert!(config["hooks"].get("beforeSubmitPrompt").is_none());
            }
            let started = Instant::now();
            drop(bridge);
            assert!(started.elapsed() < Duration::from_secs(1));
            assert!(!directory.exists());
            assert!(matches!(events.try_recv(), Err(TryRecvError::Disconnected)));
        }
    }

    #[test]
    fn overflow_disconnects_observation_without_delivering_events_after_a_gap() {
        let (bridge, events) =
            HookBridge::start(ProviderId::Claude, "logical-tab", "incarnation-1", None).unwrap();
        let bytes = envelope(&bridge, payload("Stop"));
        for _ in 0..QUEUE_LIMIT {
            assert_eq!(deliver(&bridge, &bytes).unwrap(), b"ok\n");
        }
        assert!(deliver(&bridge, &bytes).unwrap().is_empty());
        for _ in 0..QUEUE_LIMIT {
            events.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        // The worker drops the sender after leaving its loop, so wait for the disconnect.
        assert!(matches!(
            events.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        ));
    }

    #[test]
    fn input_is_bounded_and_a_partial_frame_cannot_extend_its_deadline() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        writer.write_all(b"0123456789").unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        assert!(read_fd(
            reader.as_raw_fd(),
            8,
            Instant::now() + Duration::from_secs(1),
            None,
            false
        )
        .is_err());
        let (_writer, reader) = UnixStream::pair().unwrap();
        let start = Instant::now();
        assert!(read_fd(
            reader.as_raw_fd(),
            INPUT_LIMIT,
            start + Duration::from_millis(30),
            None,
            false
        )
        .is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn helper_forwards_one_bounded_native_event_without_printing_content_or_credentials() {
        const CHILD: &str = "PROMETEU_HOOK_HELPER_TEST";
        if std::env::var_os(CHILD).is_some() {
            run().unwrap();
            return;
        }
        let (bridge, events) = HookBridge::start(
            ProviderId::Claude,
            "logical-tab",
            "incarnation-1",
            Some("native-session"),
        )
        .unwrap();
        let mut configured = Command::new("claude");
        bridge
            .configure_with_executable(&mut configured, &std::env::current_exe().unwrap())
            .unwrap();
        let mut helper = Command::new(std::env::current_exe().unwrap());
        helper.args(["--exact", "terminal_agent_hooks::tests::helper_forwards_one_bounded_native_event_without_printing_content_or_credentials", "--nocapture"])
            .env(CHILD,"1")
            .envs(configured.get_envs().filter_map(|(key,value)| value.map(|value| (key,value))))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = helper.spawn().unwrap();
        let mut native = payload("UserPromptSubmit");
        native["prompt"] = json!("private prompt only sent over the socket");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(native.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "helper process failed");
        for captured in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(captured);
            assert!(!text.contains(&bridge.token));
            assert!(!text.contains("private prompt only sent over the socket"));
        }
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            event.prompt.as_deref(),
            Some("private prompt only sent over the socket")
        );
    }

    #[test]
    fn hook_installation_requires_the_verified_native_cli_versions() {
        let _: fn(&HookBridge, &mut Command, Option<&mut Command>) -> Result<(), String> =
            HookBridge::configure;
        assert!(supported_version(
            ProviderId::Claude,
            "2.1.326 (Claude Code)"
        ));
        assert!(!supported_version(
            ProviderId::Claude,
            "2.1.325 (Claude Code)"
        ));
        assert!(supported_version(ProviderId::Cursor, "2026.11.13-eacade1"));
        assert!(!supported_version(ProviderId::Cursor, "2026.11.12-example"));
        assert!(!supported_version(ProviderId::Cursor, "unrecognized"));
        assert!(supported_version(ProviderId::Codex, "codex-cli 0.169.0"));
        assert!(supported_version(ProviderId::Codex, "codex-cli 0.171.0"));
        assert!(!supported_version(ProviderId::Codex, "codex-cli 0.168.0"));
        assert!(!supported_version(
            ProviderId::Codex,
            "codex-cli 0.169.0-alpha.1"
        ));
        assert!(supported_version(ProviderId::Antigravity, "1.5.9"));
        assert!(!supported_version(ProviderId::Antigravity, "1.5.8"));
    }

    #[test]
    fn cursor_stop_captures_authorized_metadata_before_ack_without_accepting_a_wire_witness() {
        use crate::terminal_agent_observation::CursorStopWitness;
        let (bridge, events) =
            HookBridge::start(ProviderId::Cursor, "app", "process", Some("native")).unwrap();
        let path = bridge.directory.join("native.jsonl");
        std::fs::write(&path, b"{\"role\":\"assistant\"}\n").unwrap();
        let mut witness = CursorStopWitness::new(std::slice::from_ref(&bridge.directory)).unwrap();
        let captured = Arc::new(AtomicBool::new(false));
        let callback = captured.clone();
        bridge.set_cursor_stop_capture(Box::new(move |event| {
            let stamp = witness.capture(event);
            callback.store(stamp.is_some(), Ordering::Release);
            stamp
        }));
        let payload = json!({"hook_event_name":"stop","conversation_id":"native","generation_id":"request","status":"completed","transcript_path":path});
        let bytes = envelope(&bridge, payload);
        assert!(!String::from_utf8_lossy(&bytes).contains("cursor_stop_stamp"));
        assert_eq!(deliver(&bridge, &bytes).unwrap(), b"ok\n");
        assert!(
            captured.load(Ordering::Acquire),
            "the CLI must not resume its Stop chain before capture"
        );
        let event = events.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(event.cursor_stop_stamp.is_some());
        let serialized = serde_json::to_value(&event).unwrap();
        assert!(serialized.get("cursor_stop_stamp").is_none());
        assert!(serde_json::from_value::<NativeHookEvent>(serialized)
            .unwrap()
            .cursor_stop_stamp
            .is_none());
    }
}
