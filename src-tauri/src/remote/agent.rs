//! Terminal agents on a remote machine (ADR 0101). The conversation, its journal, screen and
//! status stay in this app exactly as for a local terminal agent; only the process, its hooks and
//! its native transcript live on the machine. A process that is still running there when this app
//! starts again is reattached instead of resumed.
use crate::chat::terminal::Execution;
use crate::state::ProviderId;
use crate::terminal_agent_hooks::{HookBridge, RelayTarget};
use base64::{engine::general_purpose::STANDARD, Engine};
use prometeu_core::lock::lock;
use prometeu_core::session::launch::{LaunchRequest, StartMode};
use prometeu_core::terminal::{StartedTerminal, TerminalSize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

use super::connection::{Connection, HookSink, Mirror};
use super::terminal::Remote;

/// A remote path's private local mirror: `<root>/remote/<machine>/mirror/<absolute path>`.
pub fn mirror_path(machine: &str, remote: &str) -> PathBuf {
    crate::paths::root()
        .join("remote")
        .join(machine)
        .join("mirror")
        .join(remote.trim_start_matches('/'))
}

pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

struct Relay {
    target: RelayTarget,
    /// The machine's Claude projects directory and its local mirror.
    projects: String,
    mirror: PathBuf,
}

impl Relay {
    /// Point transcript paths at the mirror the observer reads.
    fn rewrite(&self, payload: &[u8]) -> Vec<u8> {
        let Ok(mut value) = serde_json::from_slice::<Value>(payload) else {
            return payload.to_vec();
        };
        if let Some(object) = value.as_object_mut() {
            for key in ["transcript_path", "agent_transcript_path"] {
                let Some(path) = object.get(key).and_then(Value::as_str) else {
                    continue;
                };
                if let Some(rest) = path.strip_prefix(&self.projects) {
                    let local = self.mirror.join(rest.trim_start_matches('/'));
                    object.insert(key.into(), Value::String(local.display().to_string()));
                }
            }
        }
        serde_json::to_vec(&value).unwrap_or_else(|_| payload.to_vec())
    }
}

impl HookSink for Relay {
    fn hook(&self, frame: &Value, resolved: &AtomicBool) -> Vec<u8> {
        let variables: HashMap<String, String> = frame["env"]
            .as_object()
            .map(|env| {
                env.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let Some(payload) = frame["payload"]
            .as_str()
            .and_then(|p| STANDARD.decode(p).ok())
        else {
            return Vec::new();
        };
        crate::terminal_agent_hooks::relay(
            &self.target,
            &variables,
            &self.rewrite(&payload),
            Some(resolved),
        )
        .unwrap_or_default()
    }
}

struct RemoteExecution {
    connection: Arc<Connection>,
    machine: String,
    workspace: String,
    session: String,
    home: String,
    root: String,
    helper: String,
    /// The machine still runs this conversation's process: attach to it instead of starting one.
    running: bool,
    remote: Arc<Remote>,
    limits: super::Limits,
    /// The machine's Codex version when this conversation runs Codex (ADR 0105); Claude
    /// otherwise.
    codex: Option<String>,
}

impl RemoteExecution {
    fn projects(&self) -> String {
        format!("{}/.claude/projects", self.home)
    }
}

impl Execution for RemoteExecution {
    fn observation_roots(
        &self,
        _prepared: &crate::terminal_agent_launch::PreparedTerminalAgent,
    ) -> Result<Vec<PathBuf>, String> {
        if self.codex.is_some() {
            // Codex content comes from its engine's RPC stream; no native file is observed.
            return Ok(vec![mirror_path(
                &self.machine,
                &format!("{}/.codex/sessions", self.home),
            )]);
        }
        Ok(vec![mirror_path(&self.machine, &self.projects())])
    }

    fn configure(
        &self,
        bridge: &HookBridge,
        command: &mut Command,
        _probe: Option<&mut Command>,
    ) -> Result<(), String> {
        if let Some(version) = &self.codex {
            // The engine's RPC stream is the conversation's only source and hooks are not
            // observed for Codex, so none are installed on the machine.
            return bridge.adopt_remote(version);
        }
        let version = self
            .connection
            .exec(&["claude", "--version"], None, Duration::from_secs(20))?
            .ok()
            .map_err(|_| crate::i18n::t("err.remote.claude"))?;
        let cwd = command
            .get_current_dir()
            .and_then(Path::to_str)
            .ok_or_else(|| crate::i18n::t("err.terminalAgent.unsupported"))?
            .to_string();
        // Mirror this working directory's transcripts, including subagent files, before any hook
        // can name them.
        let watch = mirror_transcripts(&self.connection, &self.machine, &self.home, &cwd)?;
        self.remote.mirrored_by(&watch);
        self.connection.on_hook(
            &self.session,
            Arc::new(Relay {
                target: bridge.relay_target()?,
                projects: self.projects(),
                mirror: mirror_path(&self.machine, &self.projects()),
            }),
        );
        if self.running {
            // SessionStart can bind a path before Claude creates the file. Flush before deciding
            // whether this is an existing transcript or a process still awaiting its first prompt.
            self.connection.request_timeout(
                "watch_flush",
                json!({"id": watch}),
                Duration::from_secs(10),
            )?;
            return bridge.adopt_remote(version.trim());
        }
        let settings = bridge.configure_remote(command, version.trim(), &self.helper)?;
        let path = format!("{}/sessions/{}/settings.json", self.root, self.session);
        self.connection
            .write_file(&path, settings.to_string().as_bytes(), 0o600)?;
        command.arg("--settings").arg(&path);
        Ok(())
    }

    fn open(
        &self,
        _app: &AppHandle,
        command: &Command,
        size: TerminalSize,
    ) -> Result<StartedTerminal, String> {
        self.start(command, size)
    }

    fn replaying(&self) -> Option<Arc<AtomicBool>> {
        Some(self.remote.replaying.clone())
    }

    fn adopts_process(&self) -> bool {
        self.running
    }

    fn keeps_process_on_engine_loss(&self) -> bool {
        true
    }
}

impl RemoteExecution {
    /// Open the terminal on the machine, or follow the one it still runs.
    fn start(&self, command: &Command, size: TerminalSize) -> Result<StartedTerminal, String> {
        if !self.running {
            let mut argv = vec![command.get_program().to_string_lossy().to_string()];
            argv.extend(command.get_args().map(|a| a.to_string_lossy().to_string()));
            let mut env = Map::new();
            let mut unset = Vec::new();
            env.insert("TERM_PROGRAM".into(), json!("Prometeu"));
            env.insert(
                "TERM_PROGRAM_VERSION".into(),
                json!(env!("CARGO_PKG_VERSION")),
            );
            for (key, value) in command.get_envs() {
                match value {
                    Some(value) => {
                        env.insert(
                            key.to_string_lossy().to_string(),
                            json!(value.to_string_lossy()),
                        );
                    }
                    None => unset.push(key.to_string_lossy().to_string()),
                }
            }
            let cwd = command
                .get_current_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| self.home.clone());
            let opened = self.connection.request(
                "terminal_open",
                json!({
                    "id": self.session, "workspace": self.workspace, "argv": argv, "cwd": cwd,
                    "env": env, "unset": unset, "cols": size.cols, "rows": size.rows,
                    "limits": self.limits.to_wire(),
                }),
            );
            match opened {
                Ok(_) => self.remote.opened(),
                // Another computer following this machine started the same conversation between
                // this launch's terminal list and its open (ADR 0104). Following that process here
                // could drop what this launch meant to send; trying again adopts it.
                Err(error) if error == "terminal_exists" => {
                    return Err(crate::i18n::t("err.remote.startedElsewhere"))
                }
                Err(error) => return Err(error),
            }
        }
        self.connection
            .subscribe(&self.session, self.remote.clone());
        Ok(self.remote.started())
    }
}

/// Mirror the transcripts of conversations working in `cwd` on the machine; returns the watch.
fn mirror_transcripts(
    connection: &Connection,
    machine: &str,
    home: &str,
    cwd: &str,
) -> Result<String, String> {
    let remote = format!(
        "{home}/.claude/projects/{}",
        prometeu_host::claude_project_slug(cwd)
    );
    let watch = format!("claude-{:016x}", crate::paths::fnv1a(&remote));
    connection.mirror(Mirror {
        id: watch.clone(),
        local: mirror_path(machine, &remote),
        remote,
        depth: 3,
    })?;
    Ok(watch)
}

/// Bind a conversation an agent started to its transcript on the machine, as its first hook
/// would have, so opening it resumes that transcript. An existing binding is kept.
pub fn bind_transcript(machine: &str, home: &str, cwd: &str, session: &str) {
    use crate::chat::terminal::binding::Binding;
    let root = crate::paths::root();
    if Binding::load(&root, session, ProviderId::Claude, session) != Ok(None) {
        return;
    }
    let transcript = mirror_path(
        machine,
        &format!(
            "{home}/.claude/projects/{}/{session}.jsonl",
            prometeu_host::claude_project_slug(cwd)
        ),
    );
    let _ = Binding::new(session, ProviderId::Claude, session, &transcript).save(&root);
}

const REMOTE_PROFILE: &str = "remote:";

/// Read archived history on demand. Failed downloads can be retried by opening the tab again.
pub(crate) fn load_archived_history(app: &AppHandle, session: &str) -> Result<(), String> {
    let state = app.state::<crate::AppState>();
    let workspace = lock(&state.board).workspace_of(session).cloned();
    let Some(workspace) = workspace.filter(|ws| ws.archived && ws.host.is_some()) else {
        return Ok(());
    };
    let gate = state.workspace_lifecycle.workspace(&workspace.id);
    let _lifecycle = gate.write().unwrap_or_else(|error| error.into_inner());
    let workspace = lock(&state.board).workspace_of(session).cloned();
    let Some(workspace) = workspace.filter(|ws| ws.archived && ws.host.is_some()) else {
        return Ok(());
    };
    let provider = workspace
        .tabs
        .iter()
        .find(|tab| tab.id == session)
        .and_then(|tab| tab.choice.as_ref())
        .map_or(workspace.agent, |choice| choice.agent);
    if provider != ProviderId::Claude || lock(&state.sessions.conversations).contains_key(session) {
        return Ok(());
    }
    let machine = workspace.host.as_deref().unwrap();
    let service = app.state::<super::Service>();
    // Cached history remains readable while the machine is disconnected.
    let connection = match service.connected(machine) {
        Ok(connection) => connection,
        Err(_) if crate::paths::chat_log(session).exists() => return Ok(()),
        Err(error) => return Err(error),
    };
    let home = connection
        .home()
        .ok_or_else(|| crate::i18n::t(super::connection::OFFLINE))?;
    let watch = mirror_transcripts(&connection, machine, &home, &workspace.worktree)?;
    connection.request_timeout("watch_flush", json!({"id": watch}), Duration::from_secs(10))?;
    bind_transcript(machine, &home, &workspace.worktree, session);
    let binding = crate::chat::terminal::binding::Binding::load(
        &crate::paths::root(),
        session,
        ProviderId::Claude,
        session,
    )
    .map_err(|_| crate::chat::terminal::history(session, "binding_load"))?
    .ok_or_else(|| crate::chat::terminal::history(session, "binding_missing"))?;
    // A conversation archived before its first prompt has no native transcript.
    if !binding.transcript.exists() {
        return Ok(());
    }
    crate::chat::terminal::import_claude_history(
        &crate::paths::root(),
        &crate::paths::chat_log(session),
        session,
        binding.transcript,
        mirror_path(machine, &format!("{home}/.claude/projects")),
    )
}

/// Whether a conversation's profile stands for a machine's own Claude sign-in.
pub fn is_remote_profile(profile: &prometeu_profiles::Profile) -> bool {
    profile.id.starts_with(REMOTE_PROFILE)
}

/// Start or reattach the terminal agent of a conversation in a remote workspace.
pub fn launch(
    app: &AppHandle,
    request: &LaunchRequest,
    machine: &str,
) -> Result<prometeu_core::session::launch::Launched<crate::chat::Chat>, String> {
    if !crate::agents::capabilities(request.settings.agent).remote_machines {
        return Err(crate::i18n::t("err.remote.provider"));
    }
    let service = app.state::<super::Service>();
    let connection = service.connected(machine)?;
    let home = connection
        .home()
        .ok_or_else(|| crate::i18n::t(super::connection::OFFLINE))?;
    let binary =
        super::install::local_binary().ok_or_else(|| crate::i18n::t("err.remote.noBuild"))?;
    let root = format!("{home}/{}", super::install::remote_root());
    if request.settings.agent == ProviderId::Codex {
        let host_binary = format!("{root}/host/{}/prometeu-host", binary.hash);
        return launch_codex(app, request, machine, connection, home, root, host_binary);
    }
    let listed = connection.request("terminal_list", json!({}))?;
    let terminal = listed
        .as_array()
        .and_then(|list| list.iter().find(|t| t["id"] == json!(request.session)));
    // A running process is adopted; so is one that finished while no desktop was attached, when
    // reattachment recovers it, so its last output and transcript arrive without a new Claude.
    let recovering = terminal.is_some() && lock(&service.recovering).contains(&request.session);
    let running = terminal.is_some_and(|t| t["running"] == json!(true)) || recovering;
    let effective =
        match crate::chat::terminal::ingestion::resume_request(&crate::paths::root(), request) {
            Ok(effective) => effective,
            // A conversation an agent started has no identity here until its first relayed hook
            // binds it (ADR 0103). Its process already runs or ran, so nothing is launched: observe
            // it as a fresh start and the queued hooks bind it, as below for an unbound adoption.
            Err(_)
                if running
                    && matches!(
                        request.mode,
                        StartMode::Resume {
                            provider_session: None
                        }
                    ) =>
            {
                LaunchRequest {
                    session: request.session.clone(),
                    workspace: request.workspace.clone(),
                    worktree: request.worktree.clone(),
                    settings: request.settings.clone(),
                    mode: StartMode::Fresh,
                }
            }
            Err(_) => {
                return Err(crate::chat::terminal::history(
                    &request.session,
                    "resume_request",
                ))
            }
        };
    let cwd = effective.worktree.clone();
    let transcript = mirror_path(
        machine,
        &format!(
            "{home}/.claude/projects/{}/{}.jsonl",
            prometeu_host::claude_project_slug(&cwd),
            effective.session
        ),
    );
    // A conversation resumed on a computer that never mirrored its transcript (one an agent
    // started) needs the file before deciding how to resume and before observation reads it.
    if !running && matches!(effective.mode, StartMode::Resume { .. }) && !transcript.exists() {
        if let Ok(watch) = mirror_transcripts(&connection, machine, &home, &cwd) {
            let _ = connection.request_timeout(
                "watch_flush",
                json!({"id": watch}),
                Duration::from_secs(10),
            );
        }
    }
    let resume = crate::terminal_agent_launch::previous(&effective).or_else(|| {
        (matches!(effective.mode, StartMode::Resume { .. }) && transcript.exists())
            .then(|| effective.session.clone())
    });
    // Claude signs in on the machine (Settings > Remote machines), so this computer's account is
    // neither needed nor used; the launch only reads the provider.
    let profile = prometeu_profiles::Profile {
        id: format!("{REMOTE_PROFILE}{machine}"),
        provider: ProviderId::Claude,
        home: mirror_path(machine, &home),
        managed: false,
        revision: 0,
    };
    let mut command = crate::terminal_agent_launch::claude_command(
        &effective,
        resume.as_deref(),
        prometeu_tools::ClaudeTools {
            mcp_config: None,
            plugin_args: Vec::new(),
        },
    );
    crate::terminal_agent_launch::drop_claude_vars(&mut command);
    // An adopted process resumes observation from its transcript binding. Without one (its first
    // hook never reached this app) nothing was ingested and its first prompt is still pending:
    // observe it as a fresh start, and the queued SessionStart binds it. The mirror is brought up
    // to date first, so a transcript written while this app was away is read from its end.
    let bound = || {
        let Ok(watch) = mirror_transcripts(&connection, machine, &home, &cwd) else {
            return false;
        };
        let _ = connection.request_timeout(
            "watch_flush",
            json!({"id": watch}),
            Duration::from_secs(10),
        );
        crate::chat::terminal::binding::Binding::load(
            &crate::paths::root(),
            &effective.session,
            ProviderId::Claude,
            resume.as_deref().unwrap_or(&effective.session),
        )
        .ok()
        .flatten()
        .is_some()
    };
    let resumed = resume.is_some() || (running && bound());
    let prepared = crate::terminal_agent_launch::PreparedTerminalAgent {
        command,
        engine: None,
        hook_probe: None,
        profile,
        resumed,
        provider_session: Some(resume.unwrap_or_else(|| effective.session.clone())),
        initial_instructions: None,
    };
    let limits = service
        .machine(machine)
        .map(|m| m.limits)
        .unwrap_or_default();
    let execution = RemoteExecution {
        remote: Remote::new(connection.clone(), &effective.session),
        connection,
        machine: machine.into(),
        workspace: effective.workspace.clone(),
        session: effective.session.clone(),
        helper: format!(
            "{} hook --root {}",
            quote(&format!("{root}/host/{}/prometeu-host", binary.hash)),
            quote(&root)
        ),
        home,
        root,
        running,
        limits,
        codex: None,
    };
    Ok(prometeu_core::session::launch::Launched {
        conversation: crate::chat::terminal::launch_on(app, &effective, prepared, &execution)?,
        resumed,
    })
}

/// Start or reattach a Codex conversation in a remote workspace (ADR 0105). Its engine and TUI
/// run on the machine with the machine's own Codex sign-in; nothing from this computer's Codex
/// account is used or sent.
fn launch_codex(
    app: &AppHandle,
    request: &LaunchRequest,
    machine: &str,
    connection: Arc<Connection>,
    home: String,
    root: String,
    host_binary: String,
) -> Result<prometeu_core::session::launch::Launched<crate::chat::Chat>, String> {
    let service = app.state::<super::Service>();
    let name = service
        .machine(machine)
        .map(|m| m.name)
        .unwrap_or_else(|| machine.to_string());
    let listed = connection.request("terminal_list", json!({}))?;
    let running = listed
        .as_array()
        .and_then(|list| list.iter().find(|t| t["id"] == json!(request.session)))
        .is_some_and(|t| t["running"] == json!(true));
    let status = super::codex::status(&connection)?;
    super::publish_codex(app, machine, status.clone());
    // A running conversation is followed whatever its sign-in says now; a new process needs it.
    let problem = if running {
        (!status.installed || !status.supported)
            .then(|| super::codex::problem(&status, &name))
            .flatten()
    } else {
        super::codex::problem(&status, &name)
    };
    if let Some(problem) = problem {
        return Err(problem);
    }
    let version = status.version.unwrap_or_default();
    let effective =
        crate::chat::terminal::ingestion::resume_request(&crate::paths::root(), request)
            .map_err(|_| crate::chat::terminal::history(&request.session, "resume_request"))?;
    let resume = crate::terminal_agent_launch::previous(&effective);
    if running && resume.is_none() {
        // Its engine's thread was never recorded here, so this app cannot follow it.
        return Err(crate::chat::terminal::history(
            &request.session,
            "codex_adopt_identity",
        ));
    }
    let (command, plan) = crate::terminal_agent_launch::remote_codex(&effective);
    let limits = service
        .machine(machine)
        .map(|m| m.limits)
        .unwrap_or_default();
    let remote = Remote::new(connection.clone(), &effective.session);
    let engine = super::codex::RemoteEngine::new(
        app.clone(),
        connection.clone(),
        effective.session.clone(),
        effective.workspace.clone(),
        root.clone(),
        home.clone(),
        host_binary,
        limits.clone(),
        running,
        remote.clone(),
    );
    // Codex signs in on the machine, so this computer's account is neither needed nor used.
    let profile = prometeu_profiles::Profile {
        id: format!("{REMOTE_PROFILE}{machine}"),
        provider: ProviderId::Codex,
        home: mirror_path(machine, &home),
        managed: false,
        revision: 0,
    };
    let resumed = resume.is_some();
    let prepared = crate::terminal_agent_launch::PreparedTerminalAgent {
        command,
        engine: Some(plan.on(Box::new(engine))),
        hook_probe: None,
        profile,
        resumed,
        provider_session: resume,
        initial_instructions: None,
    };
    let execution = RemoteExecution {
        remote,
        connection,
        machine: machine.into(),
        workspace: effective.workspace.clone(),
        session: effective.session.clone(),
        helper: String::new(),
        home,
        root,
        running,
        limits,
        codex: Some(version),
    };
    Ok(prometeu_core::session::launch::Launched {
        conversation: crate::chat::terminal::launch_on(app, &effective, prepared, &execution)?,
        resumed,
    })
}

/// Opt-in, against a real machine with Claude installed: `PROMETEU_REMOTE_TEST_SSH=vps cargo
/// test --bin Prometeu vps_claude -- --ignored --nocapture`. Starts Claude there exactly as a
/// remote conversation does and waits for its first hook to reach a local bridge.
#[cfg(test)]
mod journey {
    use super::*;
    use crate::remote::{Limits, Machine};
    use std::io::Read;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore]
    fn vps_claude() {
        let Ok(target) = std::env::var("PROMETEU_REMOTE_TEST_SSH") else {
            return;
        };
        crate::remote::install::install(&target).expect("install");
        let connection = Connection::new(
            Machine {
                id: "journey".into(),
                name: "journey".into(),
                ssh: target,
                limits: Limits::default(),
            },
            Box::new(|| {}),
            Box::new(|_, _| {}),
        );
        connection.start();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !connection.connected() {
            assert!(Instant::now() < deadline, "not connected");
            std::thread::sleep(Duration::from_millis(100));
        }
        let home = connection.home().unwrap();
        let root = format!("{home}/{}", crate::remote::install::remote_root());
        let hash = &crate::remote::install::local_binary().unwrap().hash;
        let session = uuid::Uuid::new_v4().to_string();
        let cwd = format!("{home}/code/prometeu-remote-demo");
        let execution = RemoteExecution {
            remote: Remote::new(connection.clone(), &session),
            connection: connection.clone(),
            machine: "journey".into(),
            workspace: uuid::Uuid::new_v4().to_string(),
            session: session.clone(),
            helper: format!(
                "{} hook --root {}",
                quote(&format!("{root}/host/{hash}/prometeu-host")),
                quote(&root)
            ),
            home: home.clone(),
            root,
            running: false,
            limits: Limits::default(),
            codex: None,
        };
        let nonce = uuid::Uuid::new_v4().to_string();
        let (bridge, hooks) =
            HookBridge::start(ProviderId::Claude, &session, &nonce, Some(&session)).unwrap();
        let mut command = Command::new("claude");
        command
            .current_dir(&cwd)
            .args(["--session-id", &session, "--permission-mode", "default"]);
        execution
            .configure(&bridge, &mut command, None)
            .expect("configure");
        let started = execution
            .start(
                &command,
                TerminalSize {
                    cols: 100,
                    rows: 30,
                },
            )
            .expect("start");
        let mut output = started.output;
        let (sender, screen) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = output.read(&mut buffer) {
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut bytes = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while bytes.len() < 200 && Instant::now() < deadline {
            if let Ok(chunk) = screen.recv_timeout(Duration::from_millis(200)) {
                bytes.extend(chunk);
            }
        }
        eprintln!("terminal bytes: {}", bytes.len());
        assert!(!bytes.is_empty(), "Claude drew nothing");
        // Claude stops at its native folder-trust and login screens until the person answers them
        // in the terminal view, so run the machine's hook command exactly as Claude would, with
        // the environment the remote process received.
        let mut script = String::new();
        for (key, value) in command.get_envs() {
            if let (Some(key), Some(Some(value))) = (key.to_str(), value.map(|v| v.to_str())) {
                if key.starts_with(prometeu_host::HOOK_ENV_PREFIX) {
                    script.push_str(&format!("export {key}={}\n", quote(value)));
                }
            }
        }
        let transcript = format!(
            "{home}/.claude/projects/{}/{session}.jsonl",
            prometeu_host::claude_project_slug(&cwd)
        );
        let payload = json!({"hook_event_name": "SessionStart", "session_id": session,
            "transcript_path": transcript, "cwd": cwd, "source": "startup"});
        script.push_str(&format!(
            "printf %s {} | {}\n",
            quote(&payload.to_string()),
            execution.helper
        ));
        let ran = connection
            .exec(&["sh", "-c", &script], None, Duration::from_secs(20))
            .expect("hook command");
        assert_eq!(ran.code, Some(0), "{}", ran.stderr);
        let hook = hooks.recv_timeout(Duration::from_secs(10));
        eprintln!(
            "first hook: {:?}",
            hook.as_ref().map(|e| (e.kind, e.transcript_path.clone()))
        );
        started.process.control.kill();
        let _ = connection.request("terminal_forget", json!({"id": session}));
        connection.stop();
        let event = hook.expect("a hook reached the local bridge");
        assert_eq!(event.session, session);
        assert_eq!(
            event.kind,
            crate::terminal_agent_hooks::NativeHookKind::SessionStarted
        );
        let path = event.transcript_path.expect("transcript path");
        assert!(path.starts_with(mirror_path("journey", &format!("{home}/.claude/projects"))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_machine_profiles_skip_the_local_account_check() {
        let profile = |id: &str| prometeu_profiles::Profile {
            id: id.into(),
            provider: ProviderId::Claude,
            home: PathBuf::from("/h"),
            managed: false,
            revision: 0,
        };
        assert!(is_remote_profile(&profile(&format!("{REMOTE_PROFILE}vps"))));
        assert!(!is_remote_profile(&profile("claude-default")));
    }

    #[test]
    fn hook_transcripts_point_at_the_mirror() {
        let relay = Relay {
            target: crate::terminal_agent_hooks::test_relay_target(),
            projects: "/home/me/.claude/projects".into(),
            mirror: PathBuf::from("/mirror/home/me/.claude/projects"),
        };
        let rewritten: Value = serde_json::from_slice(&relay.rewrite(
            br#"{"transcript_path":"/home/me/.claude/projects/-w/s.jsonl","agent_transcript_path":"/elsewhere/a.jsonl","x":1}"#,
        ))
        .unwrap();
        assert_eq!(
            rewritten["transcript_path"],
            "/mirror/home/me/.claude/projects/-w/s.jsonl"
        );
        assert_eq!(rewritten["agent_transcript_path"], "/elsewhere/a.jsonl");
        assert_eq!(rewritten["x"], 1);
    }

    #[test]
    fn mirrors_live_under_the_private_root() {
        let path = mirror_path("vps", "/home/me/.claude/projects/-w");
        assert!(path.ends_with("remote/vps/mirror/home/me/.claude/projects/-w"));
    }
}
