//! Experimental single-conversation execution host. No desktop or WSL transport dependency.
#![cfg(unix)]
pub mod application;
pub mod dock;
pub mod host;
pub mod lifecycle;
pub mod provider;
pub mod resident;
pub mod resources;
pub mod store;
pub mod terminal;
pub mod workspaces;
pub mod worktrees;
use prometeu_core::{
    conversation::{
        stream::{ConversationEvents, Lines, Snapshot},
        Clock,
    },
    lock::lock,
    process::{ProcessExit, ProcessHandle, ProcessIdentity, ProcessLauncher, ShutdownPolicy},
    session::{
        launch::{LaunchRequest, StartMode},
        output::{ExecutionObservation, SessionOutput, SessionTelemetry},
        provider::{AgentInput, ProviderPreparation},
        pump::{PendingInput, PumpDiagnostics, PumpReactions, SessionPump},
        workers::{ConversationWorkers, WorkerLifecycle},
    },
    tasks::TaskExecutor,
};
use provider::Prepared;
use serde_json::{json, Value};
use std::{
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
use store::SessionStore;

pub trait RuntimeEvents: Send + Sync {
    fn publish(&self, frame: Value) -> Result<(), String>;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}
struct NoTelemetry;
impl SessionTelemetry for NoTelemetry {
    fn observe(&mut self, _: &Value) {}
}
struct Effects {
    store: Arc<dyn SessionStore>,
    events: Arc<dyn RuntimeEvents>,
    generation: String,
    fault: Mutex<Option<String>>,
    work: Mutex<prometeu_core::conversation::work::Work>,
    busy: AtomicBool,
    ready: AtomicBool,
    done: Mutex<bool>,
    exited: Condvar,
}
impl Effects {
    fn failure(&self, error: &str) {
        *lock(&self.fault) = Some(error.into());
        let _ = self
            .events
            .publish(json!({"v":1,"generation":self.generation,"error":error}));
    }
}
impl ConversationEvents for Effects {
    fn emit(&self, text: &str, seq: u64) -> Result<(), String> {
        let frame: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if let Some(identity) = (frame["type"] == "session.identity")
            .then(|| frame["providerSession"].as_str())
            .flatten()
        {
            if let Err(error) = self.store.identity(identity) {
                self.failure(&error);
                return Err(error);
            }
            self.ready.store(true, Ordering::Release);
        }
        self.events
            .publish(json!({"v":1,"generation":self.generation,"seq":seq,"event":frame}))
    }
}
impl ExecutionObservation for Effects {
    fn observe(&self, frame: &Value) {
        if frame["type"] == "session.state" && frame["state"] == "busy" {
            self.busy.store(true, Ordering::Release);
        }
        if prometeu_core::conversation::agent_activity(frame) {
            self.busy.store(true, Ordering::Release);
        }
        if lock(&self.work).observe(frame) {
            self.busy.store(false, Ordering::Release);
        }
    }
    fn publish(&self, _: &Value) {}
}
impl PumpReactions for Effects {
    fn react(&self, _: &Value, _: &AtomicBool) -> bool {
        false
    }
}
impl PendingInput for Effects {
    fn queued(&self) -> bool {
        false
    }
    fn setup_running(&self) -> bool {
        false
    }
    fn schedule(&self) {}
}
impl PumpDiagnostics for Effects {
    fn storage_error(&self, error: &str) {
        self.failure(error);
    }
}
impl WorkerLifecycle for Effects {
    fn initialization_failed(&self, error: &str) {
        self.failure(error);
    }
    fn exited(&self, _: &ProcessIdentity, result: Result<ProcessExit, String>) {
        let _ = self.events.publish(json!({"v":1,"generation":self.generation,"lifecycle":"exited","code":result.as_ref().ok().and_then(|exit| exit.code),"error":result.err()}));
        *lock(&self.done) = true;
        self.exited.notify_all();
    }
}
struct Conversation {
    input: Box<dyn AgentInput>,
    process: ProcessHandle,
    pump: SessionPump<NoTelemetry>,
    effects: Arc<Effects>,
}
impl Conversation {
    fn stop(&mut self) -> Result<(), String> {
        self.pump.output.gone.store(true, Ordering::Release);
        self.input.close();
        ShutdownPolicy::AGENT.shutdown(self.process.control.as_ref());
        let (done, _) = self
            .effects
            .exited
            .wait_timeout_while(lock(&self.effects.done), Duration::from_secs(5), |done| {
                !*done
            })
            .unwrap_or_else(|e| e.into_inner());
        if !*done {
            self.process.control.kill();
            return Err("agent exit has not been observed".into());
        }
        Ok(())
    }
}
impl Drop for Conversation {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct Runtime {
    store: Arc<dyn SessionStore>,
    events: Arc<dyn RuntimeEvents>,
    providers: Arc<dyn ProviderPreparation<Prepared>>,
    launcher: Arc<dyn ProcessLauncher<Command>>,
    tasks: Arc<dyn TaskExecutor>,
    active: Option<Conversation>,
}
impl Runtime {
    pub fn new(
        store: Arc<dyn SessionStore>,
        events: Arc<dyn RuntimeEvents>,
        providers: Arc<dyn ProviderPreparation<Prepared>>,
        launcher: Arc<dyn ProcessLauncher<Command>>,
        tasks: Arc<dyn TaskExecutor>,
    ) -> Self {
        Self {
            store,
            events,
            providers,
            launcher,
            tasks,
            active: None,
        }
    }
    pub fn start(&mut self) -> Result<Value, String> {
        if self.active.is_some() {
            return Err("stop the current conversation before starting again".into());
        }
        let resume = self.store.resume();
        let request = LaunchRequest {
            session: "headless".into(),
            workspace: "headless".into(),
            worktree: self.store.workdir().to_string_lossy().into_owned(),
            settings: prometeu_core::session::launch::Launch {
                agent: prometeu_core::board::ProviderId::Codex,
                ..Default::default()
            },
            mode: StartMode::Resume {
                provider_session: resume.clone(),
            },
        };
        let prepared = self.providers.prepare(&request)?;
        let lines = Arc::new(Mutex::new(Lines::seeded(self.store.as_ref())?));
        let started = self
            .launcher
            .launch(prepared.command)
            .map_err(|e| format!("agent launch failed: {e:?}"))?;
        let generation = uuid::Uuid::new_v4().to_string();
        let effects = Arc::new(Effects {
            store: self.store.clone(),
            events: self.events.clone(),
            generation: generation.clone(),
            fault: Mutex::new(None),
            work: Mutex::new(Default::default()),
            busy: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            done: Mutex::new(false),
            exited: Condvar::new(),
        });
        let output = Arc::new(SessionOutput {
            lines,
            gone: Arc::new(AtomicBool::new(false)),
            turn: AtomicBool::new(false),
            store: self.store.clone(),
            events: effects.clone(),
            execution: effects.clone(),
        });
        let pump = SessionPump::new(
            output,
            NoTelemetry,
            Arc::new(SystemClock),
            effects.clone(),
            effects.clone(),
            effects.clone(),
        );
        let mut protocol = (prepared.connect)(started.input);
        let workers = ConversationWorkers {
            stdout: started.stdout,
            stderr: started.stderr,
            waiter: started.waiter,
            identity: started.handle.identity.clone(),
            translate: protocol.translate,
            stderr_line: prepared.stderr_line,
        };
        workers.start(
            protocol.input.as_mut(),
            pump.clone(),
            effects.clone(),
            self.tasks.as_ref(),
        )?;
        self.active = Some(Conversation {
            input: protocol.input,
            process: started.handle,
            pump,
            effects,
        });
        Ok(json!({"generation":generation,"resuming":resume.is_some()}))
    }
    /// Observe startup without blocking the host's request loop.
    pub fn ready(&self) -> Result<bool, String> {
        let active = self.active.as_ref().ok_or("conversation is stopped")?;
        if let Some(error) = lock(&active.effects.fault).clone() {
            return Err(error);
        }
        if !active.process.control.running() {
            return Err("agent exited before becoming ready".into());
        }
        Ok(active.effects.ready.load(Ordering::Acquire))
    }
    /// Resume through the same context before sending; callers retain explicit session identity.
    pub fn send(&mut self, text: &str) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|a| !a.process.control.running())
        {
            self.stop()?;
        }
        if self.active.is_none() {
            self.start()?;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let active = self.active.as_ref().ok_or("conversation is stopped")?;
            if let Some(error) = lock(&active.effects.fault).clone() {
                return Err(error);
            }
            if !active.process.control.running() {
                return Err("agent exited before becoming ready".into());
            }
            if active.effects.ready.load(Ordering::Acquire) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("agent readiness timed out; message was not sent".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.command(&json!({"v":1,"type":"message.send","text":text}))
    }
    pub fn command(&mut self, frame: &Value) -> Result<(), String> {
        let active = self.active.as_mut().ok_or("conversation is stopped")?;
        if !active.process.control.running() {
            return Err("agent has exited; stop and start to resume".into());
        }
        if let Some(error) = lock(&active.effects.fault).clone() {
            return Err(error);
        }
        if frame["type"] == "message.send" && active.effects.busy.load(Ordering::Acquire) {
            return Err("conversation is busy".into());
        }
        active.pump.command(frame, active.input.as_mut())?;
        Ok(())
    }
    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(active) = &mut self.active {
            active.stop()?;
        }
        self.active = None;
        Ok(())
    }
    pub fn reconfigure(
        &mut self,
        providers: Arc<dyn ProviderPreparation<Prepared>>,
    ) -> Result<(), String> {
        self.stop()?;
        self.providers = providers;
        Ok(())
    }
    pub fn retained(&self) -> bool {
        self.active.is_some()
    }

    pub fn snapshot(&self) -> Result<Value, String> {
        let (generation, snapshot): (Option<&str>, Snapshot) = match &self.active {
            Some(active) => (
                Some(&active.effects.generation),
                lock(&active.pump.output.lines)
                    .snapshot(active.effects.busy.load(Ordering::Acquire), &SystemClock),
            ),
            None => (
                None,
                Lines::seeded(self.store.as_ref())?.snapshot(false, &SystemClock),
            ),
        };
        Ok(
            json!({"generation":generation,"snapshot":snapshot,"providerSession":self.store.resume(),
                "running":self.active.is_some(),
                "ready":self.active.as_ref().is_some_and(|a| a.process.control.running() && a.effects.ready.load(Ordering::Acquire) && lock(&a.effects.fault).is_none()),
                "error":self.active.as_ref().and_then(|a| lock(&a.effects.fault).clone())}),
        )
    }
}

pub mod repository;

pub mod discovery;

pub mod tools;

pub mod mcp;

mod jobs;
mod operations;
