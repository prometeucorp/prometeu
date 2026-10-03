//! Desktop effects for portable session services. No queue or reaction policy lives here.
use super::*;
use prometeu_core::session::host::SessionLifecycle;
use prometeu_core::session::output::{CommandTelemetry, ExecutionObservation, SessionTelemetry};
use prometeu_core::session::reactions::{
    SessionActions, SessionContext, SessionReactions, SessionUsage,
};
use prometeu_core::session::{SessionDiagnostics, SessionPublication, SessionService};

struct Host<'a> {
    app: &'a AppHandle,
    state: State<'a, AppState>,
}
impl<'a> Host<'a> {
    fn new(app: &'a AppHandle) -> Self {
        Self {
            app,
            state: app.state(),
        }
    }
    fn with_sessions<T>(&self, run: impl FnOnce(&SessionService<'_>) -> T) -> T {
        self.state
            .sessions
            .with_service(&self.state.board, self, self, self, run)
    }
}

pub(super) fn with_sessions<T>(app: &AppHandle, run: impl FnOnce(&SessionService<'_>) -> T) -> T {
    Host::new(app).with_sessions(run)
}

impl SessionLifecycle for Host<'_> {
    fn setup_running(&self, session: &str) -> bool {
        super::setup_running(&self.state, session)
    }
    fn stopped(&self, session: &str) {
        crate::embedded_mcp::revoke(session);
        crate::delegation::stopped(&self.state, session);
    }
    fn revive(&self, session: &str) -> Result<(), String> {
        crate::session::revive(self.app, &self.state, session).map(|_| ())
    }
    fn send(&self, session: &str, text: &str, idle_only: bool) -> Result<(), String> {
        super::write_mode(
            &self.state,
            session,
            &json!({"v":1,"type":"message.send","text":text}),
            idle_only,
        )
    }
}
impl SessionPublication for Host<'_> {
    fn publish(&self) {
        crate::state::publish(self.app);
    }
    fn looking(&self) -> Option<String> {
        lock(&self.state.looking).clone()
    }
}
impl SessionDiagnostics for Host<'_> {
    fn account_error(&self, error: &str) {
        let _ = self.app.emit("account-error", error);
    }
    fn pending_error(&self, session: &str, error: &str) {
        eprintln!("Pending input failed for {session}: {error}");
    }
}
impl SessionContext for Host<'_> {
    fn tokens(&self, session: &str) -> Option<u64> {
        let worktree = lock(&self.state.board)
            .workspace_of(session)?
            .worktree
            .clone();
        crate::transcript::context(&paths::transcript(session, Path::new(&worktree)))
    }
}
impl SessionActions for Host<'_> {
    fn completed(&self, session: &str, failed: bool) {
        crate::actions::completed(self.app, session, failed);
    }
}

struct Usage<'a> {
    app: &'a AppHandle,
    profile: &'a accounts::Profile,
}
impl SessionUsage for Usage<'_> {
    fn observe(&self, event: &Value) {
        match event["provider"].as_str() {
            Some("claude") => crate::usage::claude(
                self.app,
                &self.profile.id,
                self.profile.revision,
                &event["usage"],
            ),
            Some("codex") => crate::usage::codex(
                self.app,
                &self.profile.id,
                self.profile.revision,
                &event["usage"],
            ),
            _ => {}
        }
    }
}

fn react(
    app: &AppHandle,
    id: &str,
    frame: &Value,
    ready: &AtomicBool,
    profile: &accounts::Profile,
) -> bool {
    let host = Host::new(app);
    host.with_sessions(|sessions| {
        SessionReactions {
            sessions,
            work: &host.state.sessions.work,
            context: &host,
            actions: &host,
            usage: &Usage { app, profile },
        }
        .react(id, frame, ready)
    })
}

pub(super) struct DelegationObservation {
    pub app: AppHandle,
    pub id: String,
}
impl ExecutionObservation for DelegationObservation {
    fn observe(&self, event: &Value) {
        crate::delegation::observe(&self.app, &self.id, event);
    }
    fn publish(&self, event: &Value) {
        crate::delegation::publish_observation(&self.app, &self.id, event);
    }
}

pub(crate) struct TelemetryCapture {
    capture: crate::telemetry::Capture,
    app: AppHandle,
}
impl TelemetryCapture {
    pub fn new(app: AppHandle) -> Self {
        Self {
            capture: Default::default(),
            app,
        }
    }
    pub fn initialized(&self) -> bool {
        self.capture.initialized()
    }
}
impl SessionTelemetry for TelemetryCapture {
    fn observe(&mut self, event: &Value) {
        self.capture
            .observe(&mut lock(&self.app.state::<AppState>().telemetry), event);
    }
}
impl CommandTelemetry for TelemetryCapture {
    type Prepared = (
        Option<(crate::telemetry::Scope, Option<String>)>,
        Vec<crate::telemetry::Event>,
    );
    fn generation(&self) -> u64 {
        lock(&self.app.state::<AppState>().telemetry).generation
    }
    fn accepted(&mut self, (scope, relations): Self::Prepared, generation: u64, events: &[Value]) {
        let state = self.app.state::<AppState>();
        let mut telemetry = lock(&state.telemetry);
        if generation != telemetry.generation {
            return;
        }
        if let Some((scope, model)) = scope {
            self.capture.accepted(&mut telemetry, scope, model);
            for event in &relations {
                telemetry.capture(generation, event);
            }
        }
        for event in events {
            self.capture.observe(&mut telemetry, event);
        }
    }
}

/// Captured native context stays outside the portable pump and its clone/capture machinery.
pub(super) struct ConversationHost {
    pub tasks: Arc<dyn prometeu_core::tasks::TaskExecutor>,
    pub app: AppHandle,
    pub id: String,
    pub profile: accounts::Profile,
}
impl ConversationHost {
    pub fn busy(&self) -> bool {
        self.app.state::<AppState>().sessions.busy(&self.id)
    }
}
impl prometeu_core::session::pump::PumpReactions for ConversationHost {
    fn react(&self, frame: &Value, ready: &AtomicBool) -> bool {
        react(&self.app, &self.id, frame, ready, &self.profile)
    }
}
impl prometeu_core::session::pump::PendingInput for ConversationHost {
    fn queued(&self) -> bool {
        lock(&self.app.state::<AppState>().board)
            .tab_mut(&self.id)
            .is_some_and(|tab| tab.pending_prompt.is_some())
    }
    fn setup_running(&self) -> bool {
        super::setup_running(&self.app.state::<AppState>(), &self.id)
    }
    fn schedule(&self) {
        let (app, id) = (self.app.clone(), self.id.clone());
        if let Err(error) = self
            .tasks
            .spawn(Box::new(move || super::send_prompt(&app, &id, None)))
        {
            eprintln!("Could not schedule pending input for {}: {error}", self.id);
        }
    }
}
impl prometeu_core::session::pump::PumpDiagnostics for ConversationHost {
    fn storage_error(&self, error: &str) {
        eprintln!("Could not save transcript for {}: {error}", self.id);
    }
}

impl prometeu_core::session::workers::WorkerLifecycle for ConversationHost {
    fn initialization_failed(&self, error: &str) {
        eprintln!("Could not initialize agent for {}: {error}", self.id);
    }
    fn exited(
        &self,
        identity: &ProcessIdentity,
        _result: Result<prometeu_core::process::ProcessExit, String>,
    ) {
        let state = self.app.state::<AppState>();
        state.sessions.exited(&self.id, identity, || {
            crate::delegation::stopped(&state, &self.id);
            super::update(
                &self.app,
                &self.id,
                Some(Status::Desligada),
                Note::Clear,
                None,
            );
            let _ = self.app.emit("chat-closed", &self.id);
        });
    }
}
