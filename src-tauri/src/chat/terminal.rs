//! Native terminal transport owned by an ordinary conversation, never by the dock registry.
pub(crate) mod binding;
pub(crate) mod codex;
pub(crate) mod ingestion;
mod screen;
use super::{host, Chat};
use crate::{
    conversation, i18n,
    lock::lock,
    paths,
    terminal_agent_hooks::{HookBridge, NativeHookEvent, NativeHookKind},
    terminal_agent_journal::JournalBinding,
    terminal_agent_observation::{
        AntigravityObservationConfig, CursorObservationConfig, CursorStopWitness, NativeBaseline,
        NativeBaselineReason, NativeIngestionBatch, NativeObservation, ObservationCallbacks,
        ObservationConfig, ObservationStart, TerminalAgentObservation,
    },
    AppState,
};
use portable_pty::CommandBuilder;
use prometeu_core::{
    conversation::stream::{CommitOutcome, ConversationInput, TranscriptStore},
    process::{ProcessControl, ProcessExit},
    session::{
        launch::LaunchRequest, provider::AgentInput, pump::PendingInput, workers::WorkerLifecycle,
    },
    terminal::{TerminalControl, TerminalEvents, TerminalOutput, TerminalSize},
};
use prometeu_protocols::terminal_execution::{
    CanonicalDisposition, CompletionTicket, ExecutionFact, ExecutionProjector, TranscriptBarrier,
};
use prometeu_protocols::terminal_status::{
    ProgramStatusEvent, ProgramStatusKind, ProgramStatusParser, ProgramStatusState,
    PROGRAM_STATUS_QUERY_REPLY,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, VecDeque},
    io::{Read, Write},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};

/// The packaged screen helper and native provider adapters are supported on macOS.
pub(crate) const ENABLED: bool = cfg!(target_os = "macos");

/// Native commands can open a modal without producing a user turn. They must not reserve the
/// durable prompt queue, which waits for that turn's native acceptance.
pub(crate) fn validate_message(
    mode: crate::state::ConversationMode,
    text: &str,
) -> Result<(), String> {
    if mode == crate::state::ConversationMode::Terminal
        && text.trim_start().strip_prefix('/').is_some_and(|command| {
            !command
                .split(char::is_whitespace)
                .next()
                .unwrap_or_default()
                .contains('/')
        })
    {
        return Err(i18n::t("err.terminalAgent.nativeCommands"));
    }
    Ok(())
}

struct Input {
    expected_prompts: VecDeque<ExpectedPrompt>,
    submission: Option<Submission>,
    local_draft: Option<u64>,
    composer_seen: bool,
    interrupted: bool,
    cursor_activity: Option<CursorActivity>,
}

struct CursorActivity {
    after: u64,
    started: bool,
}

struct ExpectedPrompt {
    text: String,
    id: Option<String>,
}

struct Submission {
    write_started: bool,
    id: String,
    text: String,
    normalized: String,
}

/// Lines queued by `diagnose` and not yet written by its writer thread.
static DIAGNOSTICS_PENDING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// One line in the private app log (stderr). Codes and identifiers only, never transcript content.
pub(crate) fn diagnose(scope: &str, code: &str) {
    static LOG: std::sync::OnceLock<Option<mpsc::SyncSender<String>>> = std::sync::OnceLock::new();
    // Callers log right before stopping the agent: a full disk must not panic and a stalled
    // stderr pipe must not block them, so one writer thread drains a bounded queue and
    // lines are dropped once it is full. Without that thread, logging is skipped.
    let log = LOG.get_or_init(|| {
        let (sender, lines) = mpsc::sync_channel::<String>(256);
        std::thread::Builder::new()
            .name("terminal-diagnostics".into())
            .spawn(move || {
                for line in lines {
                    let _ = writeln!(std::io::stderr().lock(), "{line}");
                    DIAGNOSTICS_PENDING.fetch_sub(1, Ordering::SeqCst);
                }
            })
            .ok()
            .map(|_| sender)
    });
    let line = format!("{} {scope} terminal.{code}", conversation::now());
    // Count before sending so the writer can never decrement below zero.
    DIAGNOSTICS_PENDING.fetch_add(1, Ordering::SeqCst);
    if log.as_ref().is_none_or(|log| log.try_send(line).is_err()) {
        DIAGNOSTICS_PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Gives queued diagnostics a short deadline to reach the log before the app exits; a
/// stalled inherited stderr must not hold the quit.
pub(crate) fn flush_diagnostics() {
    wait_drained(&DIAGNOSTICS_PENDING, Duration::from_millis(500));
}

fn wait_drained(pending: &std::sync::atomic::AtomicUsize, limit: Duration) {
    let deadline = Instant::now() + limit;
    while pending.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Every unverifiable-history failure shares one user message; the log keeps the failing step.
pub(crate) fn history(scope: &str, step: &str) -> String {
    diagnose(scope, &format!("history.{step}"));
    i18n::t("err.terminalAgent.history")
}

enum Observation {
    Live(NativeObservation),
    Ingestion(NativeIngestionBatch),
    Baseline {
        revision: String,
        reason: NativeBaselineReason,
        complete: bool,
    },
    Warning(&'static str),
    Prompt(String),
    Hook(Box<NativeHookEvent>),
}

enum ReadEvent {
    Status(Vec<ProgramStatusEvent>),
    Failed(String),
}

/// Callbacks only enqueue local observations. Publication runs after releasing terminal input,
/// since app input holds the transcript capture gate while it writes to that same terminal.
struct Observations {
    native: TerminalAgentObservation,
    events: mpsc::Receiver<Observation>,
    revision: Option<String>,
    identity: Option<String>,
    binding: Option<binding::Binding>,
    statuses: VecDeque<ProgramStatusEvent>,
    completion: Option<(u64, CompletionTicket)>,
    maintenance: Option<(String, bool)>,
}

impl Observations {
    fn new(
        config: ObservationConfig,
        cursor_store: Option<CursorObservationConfig>,
        antigravity_store: Option<AntigravityObservationConfig>,
        native_version: Option<&str>,
    ) -> Result<Self, String> {
        let source_ingestion = cursor_store.is_some()
            || matches!(
                config.provider,
                crate::state::ProviderId::Claude | crate::state::ProviderId::Antigravity
            );
        let (sender, events) = mpsc::channel();
        let live = sender.clone();
        let baseline = sender.clone();
        let warning = sender.clone();
        let prompt = sender.clone();
        let ingestion = sender.clone();
        let session = config.session.clone();
        let mut native = TerminalAgentObservation::new(
            config,
            ObservationCallbacks {
                live: Box::new(move |event| {
                    let _ = live.send(Observation::Live(event));
                }),
                native_baseline: Box::new(
                    move |NativeBaseline {
                              revision,
                              reason,
                              frames,
                              complete,
                          }| {
                        // Historical native projections are deliberately private: publishing them
                        // would repeat notifications and move existing positional comments/cuts.
                        drop(frames);
                        let _ = baseline.send(Observation::Baseline {
                            revision,
                            reason,
                            complete,
                        });
                    },
                ),
                warning: Box::new(move |code| {
                    let _ = warning.send(Observation::Warning(code));
                }),
                accepted_prompt: Box::new(move |text| {
                    let _ = prompt.send(Observation::Prompt(text));
                }),
                observed_hook: Box::new(move |event| {
                    let _ = sender.send(Observation::Hook(Box::new(event)));
                }),
            },
            Arc::new(super::SystemClock),
        )
        .map_err(|_| i18n::t("err.terminalAgent.hooks"))?;
        if let Some(version) = native_version {
            native.configure_completion(version);
        }
        if let Some(config) = cursor_store {
            native
                .configure_cursor_store(config)
                .map_err(|_| history(&session, "cursor_store"))?;
        }
        if let Some(config) = antigravity_store {
            native
                .configure_antigravity_store(config)
                .map_err(|_| history(&session, "antigravity_store"))?;
        }
        if source_ingestion {
            native
                .set_ingestion(Box::new(move |batch| {
                    let _ = ingestion.send(Observation::Ingestion(batch));
                }))
                .map_err(|_| history(&session, "ingestion_callback"))?;
        }
        Ok(Self {
            native,
            events,
            revision: None,
            identity: None,
            binding: None,
            statuses: VecDeque::new(),
            completion: None,
            maintenance: None,
        })
    }

    fn queue_status(&mut self, events: Vec<ProgramStatusEvent>) -> Result<(), String> {
        if events.len() > 4096usize.saturating_sub(self.statuses.len()) {
            return Err(i18n::t("err.terminalAgent.history"));
        }
        self.statuses.extend(events);
        Ok(())
    }

    fn replay_pending(&mut self, journal: &ingestion::JournalStore) -> Result<(), String> {
        for event in self.events.try_iter() {
            match event {
                Observation::Ingestion(batch) => {
                    // Step names only: the launch caller turns them into its history error.
                    journal.replay(&batch).map_err(|_| "replay".to_owned())?;
                    self.native
                        .acknowledge_ingestion(&batch.id)
                        .map_err(|_| "acknowledge".to_owned())?;
                }
                Observation::Baseline { revision, .. } => self.revision = Some(revision),
                Observation::Warning(_) => {}
                _ => return Err("unexpected_event".into()),
            }
        }
        Ok(())
    }

    fn ingest(
        &mut self,
        batch: NativeIngestionBatch,
        transport: &Transport,
        pump: &super::Pump,
        context: &host::ConversationHost,
    ) -> Result<(), String> {
        use ingestion::NativeDisposition;
        let scope = transport.generation.as_str();
        if batch.phase != prometeu_protocols::terminal_transcript::TranscriptPhase::Live {
            return Err(history(scope, "ingest.phase"));
        }
        let journal = transport
            .journal()
            .ok_or_else(|| history(scope, "ingest.journal"))?;
        if batch.mode == crate::terminal_agent_observation::NativeIngestionMode::Append {
            journal.begin_stream_batch(&batch)
        } else {
            journal.begin_batch(&batch)
        }
        .map_err(|_| history(scope, &format!("ingest.begin:{:?}", batch.mode)))?;
        for record in &batch.records {
            // Occurrence and kind are native identifiers; the frame holds content and stays out.
            let step = |name: &str| {
                history(
                    scope,
                    &format!(
                        "ingest.{name}:{}:{}",
                        record.checkpoint["kind"], record.occurrence
                    ),
                )
            };
            match journal
                .disposition(record)
                .map_err(|_| step("disposition"))?
            {
                NativeDisposition::Duplicate | NativeDisposition::Checkpoint => {
                    journal
                        .commit_checkpoint(record)
                        .map_err(|_| step("checkpoint"))?;
                    if record.checkpoint["kind"] == "turn"
                        && record.checkpoint["maintenance"] == "compact"
                    {
                        if let Some((user, complete)) = &mut self.maintenance {
                            if record.checkpoint["sourceUser"] == *user {
                                *complete = true;
                            }
                        }
                    }
                }
                NativeDisposition::User => {
                    let maintenance = transport.provider == crate::state::ProviderId::Antigravity
                        && record.checkpoint["maintenance"] == "compact"
                        && record.input_binding.is_none();
                    if maintenance {
                        let frame = record
                            .frame
                            .as_ref()
                            .ok_or_else(|| step("maintenance_frame"))?;
                        if pump
                            .feed_observed_committed(&frame.to_string(), |line, offset| {
                                journal.commit_record(record, line, offset)
                            })
                            .map_err(|_| step("maintenance"))?
                        {
                            self.maintenance = Some((record.occurrence.clone(), false));
                            self.completion = None;
                            let mut input = lock(&transport.input);
                            input.local_draft = None;
                            input.composer_seen = true;
                            input.interrupted = false;
                            transport.has_submitted.store(true, Ordering::Release);
                            lock(&transport.execution).accepted_maintenance();
                        }
                        continue;
                    }
                    self.maintenance = None;
                    if super::observe_native_record(
                        &context.app.state::<AppState>(),
                        &context.id,
                        &transport.generation,
                        record,
                    )
                    .map_err(|_| step("user"))?
                    {
                        transport.captured_input(pump);
                    }
                }
                NativeDisposition::Revision => {
                    let frame = record
                        .frame
                        .as_ref()
                        .ok_or_else(|| step("revision_frame"))?;
                    if record.checkpoint["kind"] == "metadata"
                        && !prometeu_core::conversation::stream::persistent(frame)
                    {
                        // A changed historical usage/context fact has a receipt, but must not
                        // charge or replace state belonging to the current native request.
                        journal
                            .commit_checkpoint(record)
                            .map_err(|_| step("revision_checkpoint"))?;
                    } else {
                        pump.revise_committed(&frame.to_string(), |line, offset| {
                            journal.commit_record(record, Some(line), offset)
                        })
                        .map_err(|_| step("revision"))?;
                    }
                }
                NativeDisposition::Output => {
                    transport
                        .feed_native_output(pump, &journal, record)
                        .map_err(|_| step("output"))?;
                }
            }
        }
        journal
            .finish_batch(&batch)
            .map_err(|_| history(scope, "ingest.finish"))?;
        self.native
            .acknowledge_ingestion(&batch.id)
            .map_err(|_| history(scope, "ingest.acknowledge"))
    }

    fn poll(
        &mut self,
        transport: &Transport,
        hooks: &mpsc::Receiver<NativeHookEvent>,
        pump: &super::Pump,
        context: &host::ConversationHost,
    ) {
        if transport.gone.load(Ordering::Acquire) {
            return;
        }
        if transport
            .journal()
            .is_some_and(|journal| !journal.healthy())
        {
            transport.abort_submission("", "journal_unhealthy");
            return;
        }
        if transport.codex.is_some() {
            if lock(&transport.input).submission.is_some() {
                return;
            }
            // Codex content, user identity and completion have one ordered RPC owner. Native
            // PSP remains presentation/readiness evidence; hooks cannot republish its rollout.
            for _ in hooks.try_iter() {}
            for event in self.statuses.drain(..) {
                transport.observe(event, context, pump);
            }
            return;
        }
        let statuses = {
            let mut input = lock(&transport.input);
            if input.submission.is_some() {
                // Native acceptance can precede the app's durable echo. Keep its observations
                // staged until that same input transaction commits or retires the transport.
                return;
            }
            for prompt in input.expected_prompts.drain(..) {
                let _ = match prompt.id {
                    Some(id) => self.native.expect_submission(&prompt.text, &id),
                    None => self.native.expect_prompt(&prompt.text),
                };
            }
            // Registration and observation share the writer lock, so a fast hook cannot race
            // an app write's successful return and get captured as a second user submission.
            for event in hooks.try_iter() {
                let _ = self.native.accept_hook(event);
            }
            let statuses = self
                .statuses
                .drain(..)
                .map(|event| {
                    let ordinal = match &event {
                        ProgramStatusEvent::Report(report) => {
                            self.native.observe_program_status(report)
                        }
                        _ => None,
                    };
                    (event, ordinal)
                })
                .collect::<Vec<_>>();
            let _ = self.native.poll();
            statuses
        };
        let was_ready = transport.ready();
        let events = self.events.try_iter().collect::<Vec<_>>();
        for event in events {
            if transport.gone.load(Ordering::Acquire) {
                break;
            }
            match event {
                Observation::Ingestion(batch) => {
                    if self.ingest(batch, transport, pump, context).is_err() {
                        transport.abort_submission("", "ingestion");
                        return;
                    }
                }
                Observation::Baseline {
                    revision,
                    reason,
                    complete,
                } => {
                    if reason == NativeBaselineReason::Rewrite
                        || complete
                        || self.revision.is_none()
                    {
                        self.revision = Some(revision);
                    }
                }
                Observation::Live(observation) => {
                    if self.revision.as_deref() == Some(observation.revision.as_str()) {
                        transport.feed_canonical(pump, observation.frame);
                    }
                }
                Observation::Prompt(text) => {
                    if super::observe_native_input(
                        &context.app.state::<AppState>(),
                        &context.id,
                        &transport.generation,
                        &text,
                    )
                    .is_ok()
                    {
                        transport.captured_input(pump);
                    }
                }
                Observation::Hook(event) if event.agent_id.is_none() => {
                    if transport
                        .journal()
                        .is_some_and(|journal| journal.bind_native(&event.native_session).is_err())
                    {
                        transport
                            .abort_submission("", &format!("identity:{}", event.native_session));
                        return;
                    }
                    if let Some(path) = &event.transcript_path {
                        let binding = binding::Binding::new(
                            &context.id,
                            event.provider,
                            &event.native_session,
                            path,
                        );
                        if self.binding.as_ref() != Some(&binding) {
                            if binding.save(&paths::root()).is_err() {
                                diagnose(&transport.generation, "observation.binding");
                                continue;
                            }
                            self.binding = Some(binding);
                        }
                    }
                    if self.native.native_session() == Some(event.native_session.as_str())
                        && self.identity.as_deref() != Some(event.native_session.as_str())
                    {
                        self.identity = Some(event.native_session.clone());
                        pump.feed(
                            &conversation::event(
                                "session.identity",
                                conversation::now(),
                                json!({"providerSession":event.native_session}),
                            )
                            .to_string(),
                        );
                    }
                    transport.observe_hook(&event, pump);
                }
                Observation::Hook(_) => {}
                Observation::Warning(code) => diagnose(
                    &transport.generation,
                    code.strip_prefix("terminal.").unwrap_or(code),
                ),
            }
            if transport
                .journal()
                .is_some_and(|journal| !journal.healthy())
            {
                transport.abort_submission("", "journal_unhealthy_after_event");
                return;
            }
        }
        for (event, ordinal) in statuses {
            let state = match &event {
                ProgramStatusEvent::Report(report) if report.id.is_none() => Some(report.state),
                _ => None,
            };
            transport.observe(event, context, pump);
            if let Some(state) = state {
                self.completion = if state == ProgramStatusState::Done {
                    let next = ordinal.zip(lock(&transport.execution).pending_completion());
                    match (&self.completion, &next) {
                        (Some((_, previous)), Some((_, current))) if previous == current => {
                            self.completion.clone()
                        }
                        _ => next,
                    }
                } else {
                    None
                };
            }
        }
        transport
            .baseline_ready
            .store(self.native.baseline_ready(), Ordering::Release);
        transport.ingestion_ready.store(
            self.native.caught_up()
                && transport
                    .journal()
                    .is_none_or(|journal| journal.caught_up()),
            Ordering::Release,
        );
        if self
            .maintenance
            .as_ref()
            .is_some_and(|(_, complete)| *complete)
            && transport.ingestion_ready.load(Ordering::Acquire)
        {
            let frames = lock(&transport.execution)
                .settle_maintenance(TranscriptBarrier::CaughtUp, conversation::now());
            if let Some(frames) = frames {
                self.maintenance = None;
                self.completion = None;
                for frame in frames {
                    pump.feed(&frame.to_string());
                }
            }
        }
        if self.native.caught_up()
            && transport
                .journal
                .as_ref()
                .is_some_and(|journal| journal.caught_up())
            && self.complete(transport, pump).is_err()
        {
            transport.abort_submission("", "completion");
            return;
        }
        if !was_ready && transport.ready() && !context.setup_running() {
            context.schedule();
        }
    }

    fn complete(&mut self, transport: &Transport, pump: &super::Pump) -> Result<(), String> {
        if transport.provider == crate::state::ProviderId::Cursor {
            return self.complete_cursor(transport, pump);
        }
        let (ticket, proof, frame) = {
            let input = lock(&transport.input);
            if input.submission.is_some() || transport.gone.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some((ordinal, proof)) = self.native.take_persisted_completion() else {
                return Ok(());
            };
            let Some((expected, ticket)) = self.completion.take() else {
                return Ok(());
            };
            if expected != ordinal {
                return Ok(());
            }
            let Some(frame) = lock(&transport.execution).preview_completion(
                &ticket,
                TranscriptBarrier::CaughtUp,
                conversation::now(),
            ) else {
                return Ok(());
            };
            (ticket, proof, frame)
        };
        let journal = transport
            .journal()
            .ok_or_else(|| i18n::t("err.terminalAgent.history"))?;
        pump.feed_committed(&frame.to_string(), |line, offset| {
            // Input admission orders capture -> input -> execution. Recheck here after acquiring
            // the capture gate, so a new accepted prompt cannot inherit an older source proof.
            let input = lock(&transport.input);
            if input.submission.is_some() || transport.gone.load(Ordering::Acquire) {
                return Ok(CommitOutcome::AlreadyCommitted);
            }
            let mut execution = lock(&transport.execution);
            if execution
                .preview_completion(&ticket, TranscriptBarrier::CaughtUp, conversation::now())
                .is_none()
            {
                return Ok(CommitOutcome::AlreadyCommitted);
            }
            let outcome = journal.commit_completion(&proof.native_source, line, offset)?;
            execution.commit_completion(&ticket, TranscriptBarrier::CaughtUp, conversation::now());
            Ok(outcome)
        })?;
        Ok(())
    }

    fn complete_cursor(&mut self, transport: &Transport, pump: &super::Pump) -> Result<(), String> {
        let proof = match self.native.cursor_completion() {
            Ok(Some(proof)) => proof,
            Ok(None) | Err(crate::terminal_agent_observation::ObservationError::Unstable) => {
                return Ok(())
            }
            Err(_) => return Err(i18n::t("err.terminalAgent.history")),
        };
        let journal = transport
            .journal()
            .ok_or_else(|| i18n::t("err.terminalAgent.history"))?;
        if !journal.caught_up() {
            return Ok(());
        }
        let committed = commit_cursor_completion(
            pump,
            &journal,
            &proof.native_source,
            transport,
            proof.interrupted.then_some(proof.request_id.as_str()),
            || transport.cursor_completion_input(proof.interrupted),
        )?;
        if committed {
            self.native.acknowledge_cursor_completion(&proof.request_id);
        }
        Ok(())
    }
}

fn commit_cursor_completion<T: prometeu_core::session::output::SessionTelemetry, G>(
    pump: &prometeu_core::session::pump::SessionPump<T>,
    journal: &ingestion::JournalStore,
    source: &str,
    transport: &Transport,
    interrupted_request: Option<&str>,
    mut current: impl FnMut() -> Option<G>,
) -> Result<bool, String> {
    let frame = conversation::event(
        "turn.completed",
        conversation::now(),
        json!({"outcome":if interrupted_request.is_some() { "interrupted" } else { "ok" },"message":"","durationMs":null,"costUsd":null}),
    );
    let mut accepted = false;
    let mut observations = Vec::new();
    pump.feed_committed(&frame.to_string(), |line, offset| {
        let Some(_input_gate) = current() else {
            return Ok(CommitOutcome::AlreadyCommitted);
        };
        let outcome = journal.commit_completion(source, line, offset)?;
        accepted = true;
        if let Some(request) = interrupted_request {
            lock(&transport.cursor_interrupted_requests).insert(request.into());
        }
        if outcome == CommitOutcome::Appended {
            let mut execution = lock(&transport.execution);
            if interrupted_request.is_none() {
                observations = execution
                    .observe_fact(
                        ExecutionFact::Finished {
                            id: None,
                            failed: false,
                            detail: None,
                        },
                        conversation::now(),
                    )
                    .events;
            }
            execution.observe_canonical(&frame);
        }
        Ok(outcome)
    })?;
    for event in observations {
        pump.feed(&event.to_string());
    }
    Ok(accepted)
}

pub(crate) struct Transport {
    generation: String,
    provider: crate::state::ProviderId,
    input: Mutex<Input>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    control: Arc<dyn TerminalControl>,
    process: Arc<dyn ProcessControl>,
    pub(super) output: Arc<TerminalOutput>,
    events: Arc<dyn TerminalEvents>,
    closed: Mutex<bool>,
    gone: Arc<AtomicBool>,
    ready: AtomicBool,
    baseline_ready: AtomicBool,
    has_submitted: AtomicBool,
    initial_input_available: AtomicBool,
    ingestion_ready: AtomicBool,
    psp_seen: AtomicBool,
    composer_ready: AtomicBool,
    journal: Option<Arc<ingestion::JournalStore>>,
    screen: Option<screen::Screen>,
    ui_readiness: Option<crate::native_ui_readiness::NativeUiReadiness>,
    controls: Option<crate::terminal_agent_controls::NativeControls>,
    execution: Mutex<ExecutionProjector>,
    cursor_interrupted_requests: Mutex<BTreeSet<String>>,
    codex: Option<Arc<codex::Attached>>,
    decisions: Option<Arc<crate::terminal_agent_decisions::Decisions>>,
    activity: Option<Arc<prometeu_core::session::output::SessionOutput>>,
    published: Arc<(Mutex<bool>, Condvar)>,
    /// Set by a remote terminal while its reader handles bytes replayed after reattachment
    /// (ADR 0101). Their terminal queries were answered, or abandoned, when first produced.
    replaying: Option<Arc<AtomicBool>>,
}

impl Transport {
    fn cursor_completion_input(
        &self,
        interrupted: bool,
    ) -> Option<std::sync::MutexGuard<'_, Input>> {
        let input = lock(&self.input);
        if input.submission.is_some()
            || !input.expected_prompts.is_empty()
            || input.cursor_activity.is_some()
            || self.gone.load(Ordering::Acquire)
        {
            return None;
        }
        // Cancellation restores the native prompt as a draft. Its authenticated request proof
        // settles execution without requiring that draft to be cleared first.
        if interrupted {
            return Some(input);
        }
        if input.local_draft.is_some() {
            return None;
        }
        let (Some(readiness), Some(screen)) = (&self.ui_readiness, &self.screen) else {
            return None;
        };
        // Success additionally needs a current empty composer after its post-hooks source proof.
        screen
            .view()
            .is_ok_and(|snapshot| readiness.ready(&snapshot.state))
            .then_some(input)
    }

    pub(super) fn journal(&self) -> Option<Arc<ingestion::JournalStore>> {
        self.journal.clone()
    }

    pub(crate) fn owns(&self, generation: Option<&str>) -> bool {
        generation == Some(self.generation.as_str())
    }
    pub(super) fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
            && self.codex.as_ref().is_none_or(|codex| codex.native_idle())
            && self.baseline_ready.load(Ordering::Acquire)
            && self
                .journal
                .as_ref()
                .is_none_or(|journal| journal.healthy())
            && (self.journal.is_none()
                || !self.has_submitted.load(Ordering::Acquire)
                || self.ingestion_ready.load(Ordering::Acquire))
            && (self.ui_readiness.is_none() || self.composer_ready.load(Ordering::Acquire))
            && lock(&self.input).local_draft.is_none()
    }

    pub(super) fn busy(&self) -> bool {
        lock(&self.execution).snapshot().has_active_work()
            || self
                .activity
                .as_ref()
                .is_some_and(|activity| activity.turn.load(Ordering::Acquire))
    }

    pub(super) fn begin_submission(&self, id: &str, text: &str) -> Result<String, String> {
        validate_message(crate::state::ConversationMode::Terminal, text)?;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err(i18n::t("err.terminalAgent.input"));
        }
        let mut input = lock(&self.input);
        if input.submission.is_some() || !self.connected() {
            return Err(i18n::t("err.terminalAgent.notReady"));
        }
        let normalized = if self.provider == crate::state::ProviderId::Cursor {
            crate::native_ui_readiness::normalize_cursor_submission(text)
        } else {
            text.to_owned()
        };
        input.submission = Some(Submission {
            write_started: false,
            id: id.into(),
            text: text.into(),
            normalized: normalized.clone(),
        });
        Ok(normalized)
    }

    pub(super) fn submission_write_started(&self, id: &str) -> bool {
        lock(&self.input)
            .submission
            .as_ref()
            .is_none_or(|submission| submission.id != id || submission.write_started)
    }

    pub(super) fn end_submission(&self, id: &str) {
        let mut input = lock(&self.input);
        if input
            .submission
            .as_ref()
            .is_some_and(|submission| submission.id == id)
        {
            input.submission = None;
        }
    }

    pub(super) fn abort_submission(&self, id: &str, reason: &str) {
        // The CLI may already have accepted input whose canonical commit failed. Stop this
        // incarnation instead of resending it or letting unjournaled observations continue.
        diagnose(&self.generation, &format!("abort.{reason}"));
        self.retire();
        self.control.disconnect();
        self.process.kill();
        self.end_submission(id);
    }

    pub(super) fn retire(&self) {
        if let Some(decisions) = &self.decisions {
            decisions.close();
        }
        self.gone.store(true, Ordering::Release);
        self.output.retire();
        // Deliberate invalidation is distinct from late worker output: the current view must
        // stop input even when a model/account change preserves the tab's ready board status.
        self.announce_closed(None);
        let _publication = lock(&self.published.0);
        self.published.1.notify_all();
    }

    pub(super) fn publish(&self) -> Option<String> {
        let first = !std::mem::replace(&mut *lock(&self.published.0), true);
        self.published.1.notify_all();
        first.then(|| self.generation.clone())
    }

    fn await_publication(&self) -> bool {
        let mut published = lock(&self.published.0);
        while !*published && !self.gone.load(Ordering::Acquire) {
            published = self
                .published
                .1
                .wait(published)
                .unwrap_or_else(|error| error.into_inner());
        }
        !self.gone.load(Ordering::Acquire)
    }

    fn announce_closed(&self, code: Option<u32>) {
        let mut closed = lock(&self.closed);
        if !*closed {
            self.events.closed(code);
            *closed = true;
        }
    }

    fn connected(&self) -> bool {
        !self.gone.load(Ordering::Acquire)
            && self.process.running()
            && self.screen.as_ref().is_none_or(screen::Screen::connected)
    }

    pub(crate) fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        self.write_inner(bytes)
    }

    pub(crate) fn write_user_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        let mut input = lock(&self.input);
        let cursor_submission = self.provider == crate::state::ProviderId::Cursor
            && matches!(bytes, b"\r" | b"\n" | b"\r\n")
            && !self.busy();
        let composer_present = input.composer_seen
            || match (&self.ui_readiness, &self.screen) {
                (Some(readiness), Some(screen)) => screen
                    .view()
                    .is_ok_and(|snapshot| readiness.ready_with_native_idle(&snapshot.state)),
                _ => self.ready.load(Ordering::Acquire),
            };
        let edits_composer = composer_present
            && !bytes.is_empty()
            && !matches!(bytes, b"\x1b[I" | b"\x1b[O")
            && !bytes.starts_with(b"\x1b[<")
            && !bytes.starts_with(b"\x1b[M");
        let before = lock(&self.output.buffer).seq;
        if edits_composer {
            // Local Enter may precede its authenticated hook/transcript. Never reopen the
            // startup-only gate from an empty paint in that interval, even if writing fails.
            self.initial_input_available.store(false, Ordering::Release);
        }
        self.write_inner(bytes)?;
        if cursor_submission {
            input.cursor_activity = Some(CursorActivity {
                after: before,
                started: false,
            });
        }
        if edits_composer {
            // A later painted empty composer or accepted native input must release local input
            // ownership. Native idle alone does not mean the person has no draft or open menu.
            input.local_draft = Some(before);
            input.composer_seen = true;
        }
        Ok(())
    }

    fn inspect_readiness(&self, context: &host::ConversationHost, pump: &super::Pump) {
        if self.provider == crate::state::ProviderId::Cursor {
            if let Some(snapshot) = self.screen.as_ref().and_then(|screen| screen.view().ok()) {
                self.observe_cursor_activity(&snapshot.state, snapshot.chunk_seq, pump);
            }
        }
        if self.refresh_readiness() && !context.setup_running() {
            context.schedule();
        }
    }

    fn observe_cursor_activity<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        snapshot: &prometeu_process::terminal_state::Snapshot,
        sequence: u64,
        pump: &prometeu_core::session::pump::SessionPump<T>,
    ) {
        if self.provider != crate::state::ProviderId::Cursor || self.gone.load(Ordering::Acquire) {
            return;
        }
        let frames = {
            let mut input = lock(&self.input);
            let Some(activity) = input.cursor_activity.as_mut() else {
                return;
            };
            if activity.started || sequence <= activity.after {
                return;
            }
            if !self
                .controls
                .as_ref()
                .is_some_and(|controls| controls.observes_working(snapshot))
            {
                // Cursor can clear its composer while submission hooks still run. An empty
                // intermediate paint does not consume the pending start observation.
                return;
            }
            activity.started = true;
            self.ready.store(false, Ordering::Release);
            self.initial_input_available.store(false, Ordering::Release);
            let mut execution = lock(&self.execution);
            execution.observed_input(conversation::now())
        };
        for frame in frames {
            pump.feed(&frame.to_string());
        }
    }

    fn has_native_idle_evidence(&self) -> bool {
        self.psp_seen.load(Ordering::Acquire)
            || self.codex.as_ref().is_some_and(|codex| codex.native_idle())
    }

    fn initial_input_admissible(&self) -> bool {
        self.provider == crate::state::ProviderId::Antigravity
            && self.initial_input_available.load(Ordering::Acquire)
            && !self.has_submitted.load(Ordering::Acquire)
            && !self.psp_seen.load(Ordering::Acquire)
            && !self.busy()
    }

    fn ui_ready(
        &self,
        readiness: &crate::native_ui_readiness::NativeUiReadiness,
        snapshot: &prometeu_process::terminal_state::Snapshot,
    ) -> bool {
        if self.has_native_idle_evidence() {
            readiness.ready_with_native_idle(snapshot)
        } else {
            readiness.ready(snapshot)
                || (self.initial_input_admissible() && readiness.ready_for_initial_input(snapshot))
        }
    }

    fn refresh_readiness(&self) -> bool {
        let (Some(readiness), Some(screen)) = (&self.ui_readiness, &self.screen) else {
            return false;
        };
        if !self.connected() {
            return false;
        }
        let previous = self.ready();
        let Ok(snapshot) = screen.view() else {
            return false;
        };
        let psp = self.psp_seen.load(Ordering::Acquire);
        let candidate = self.ui_ready(readiness, &snapshot.state);
        self.composer_ready.store(candidate, Ordering::Release);
        let mut input = lock(&self.input);
        input.composer_seen |= candidate;
        if input
            .local_draft
            .is_some_and(|after| snapshot.chunk_seq > after)
            && readiness.empty_composer_verified(&snapshot.state)
        {
            input.local_draft = None;
        }
        let ready = candidate && input.local_draft.is_none();
        if !psp {
            self.ready.store(ready, Ordering::Release);
        }
        drop(input);
        ready && !previous && self.ready()
    }

    fn write_inner(&self, bytes: &[u8]) -> Result<(), String> {
        if !self.connected() {
            return Err(i18n::t("err.chat.gone"));
        }
        let mut writer = lock(&self.writer);
        writer
            .as_mut()
            .ok_or_else(|| i18n::t("err.chat.gone"))?
            .write_all(bytes)
            .and_then(|()| writer.as_mut().expect("writer retained").flush())
            .map_err(|error| i18n::io(error.to_string()))
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        if cols == 0 || rows == 0 || !self.connected() {
            return Err(i18n::t("err.pty.gone"));
        }
        if let Some(screen) = &self.screen {
            screen
                .resize(self.control.as_ref(), TerminalSize { cols, rows })
                .map_err(|_| i18n::t("err.terminalAgent.screen"))
        } else {
            self.control
                .resize(TerminalSize { cols, rows })
                .map_err(i18n::io)
        }
    }

    fn output(&self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if self.gone.load(Ordering::Acquire) {
            return Ok(Vec::new());
        }
        let replies = match &self.screen {
            Some(screen) => screen
                .write(bytes)
                .map_err(|_| i18n::t("err.terminalAgent.screen"))?,
            None => Vec::new(),
        };
        self.output.feed(bytes);
        Ok(replies)
    }

    fn feed_canonical<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        pump: &prometeu_core::session::pump::SessionPump<T>,
        frame: Value,
    ) {
        let disposition = lock(&self.execution).observe_canonical(&frame);
        match disposition {
            CanonicalDisposition::Forward => pump.feed(&frame.to_string()),
            CanonicalDisposition::Replace(frame) => pump.feed(&frame.to_string()),
            // An uncorrelated late completion must not close or charge a newer input cycle.
            // Recovery/usage reconciliation uses its native source identity separately.
            CanonicalDisposition::DuplicateCompletion => {}
        }
    }

    fn feed_native_output<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        pump: &prometeu_core::session::pump::SessionPump<T>,
        journal: &ingestion::JournalStore,
        record: &crate::terminal_agent_observation::NativeCapture,
    ) -> Result<(), String> {
        let frame = record
            .frame
            .as_ref()
            .ok_or_else(|| i18n::t("err.terminalAgent.history"))?;
        // The source receipt proves this content belongs to the current native input. Its final
        // persisted rows can trail an explicit root idle report, so retain them without reviving
        // that stopped activity. A new accepted source input opens the next live cycle.
        let root_idle = lock(&self.execution).root_content_idle()
            && record.checkpoint["sourceScope"] == "record"
            && record.checkpoint["sourceUser"].is_string();
        let cursor_interrupted = self.provider == crate::state::ProviderId::Cursor
            && record.checkpoint["requestId"]
                .as_str()
                .is_some_and(|request| lock(&self.cursor_interrupted_requests).contains(request));
        if (root_idle || cursor_interrupted)
            && matches!(
                frame["type"].as_str(),
                Some("assistant.block" | "tool.completed")
            )
        {
            pump.revise_committed(&frame.to_string(), |line, offset| {
                journal.commit_record(record, Some(line), offset)
            })?;
        } else if pump.feed_observed_committed(&frame.to_string(), |line, offset| {
            journal.commit_record(record, line, offset)
        })? {
            // Source records contain content, not lifecycle conclusions. Observe activity only
            // after the canonical occurrence has actually committed and published.
            lock(&self.execution).observe_canonical(frame);
        }
        Ok(())
    }

    fn captured_input<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        pump: &prometeu_core::session::pump::SessionPump<T>,
    ) {
        let frames = {
            let mut input = lock(&self.input);
            input.local_draft = None;
            input.composer_seen = true;
            input.interrupted = false;
            input.cursor_activity = None;
            self.has_submitted.store(true, Ordering::Release);
            lock(&self.execution).captured_input(conversation::now())
        };
        for frame in frames {
            pump.feed(&frame.to_string());
        }
    }

    fn observe_hook<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        hook: &NativeHookEvent,
        pump: &prometeu_core::session::pump::SessionPump<T>,
    ) {
        if hook.agent_id.is_some()
            || self.psp_seen.load(Ordering::Acquire)
            || (self.provider == crate::state::ProviderId::Cursor
                && hook.generation_id.as_deref().is_some_and(|request| {
                    lock(&self.cursor_interrupted_requests).contains(request)
                }))
        {
            // Hook delivery and PTY output have independent queues. Once native PSP is present,
            // a late tool callback must not overwrite a newer native idle/done observation.
            return;
        }
        let fact = match hook.kind {
            NativeHookKind::ModelInvocationStarted
            | NativeHookKind::ToolStarted
            | NativeHookKind::ToolFinished
            | NativeHookKind::ToolFailed => ExecutionFact::Working {
                id: None,
                detail: None,
            },
            NativeHookKind::PermissionRequested => ExecutionFact::Blocked {
                id: None,
                kind: Some(ProgramStatusKind::Permission),
                detail: None,
            },
            _ => return,
        };
        self.initial_input_available.store(false, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        let projected = lock(&self.execution).observe_fact(fact, conversation::now());
        for frame in projected.events {
            pump.feed(&frame.to_string());
        }
    }

    fn drain_decisions(&self, pump: &super::Pump) {
        if let Some(decisions) = &self.decisions {
            for event in decisions.drain() {
                pump.feed(&event.to_string());
                if self.journal().is_some_and(|journal| !journal.healthy()) {
                    self.abort_submission("", "decision_journal_unhealthy");
                    break;
                }
            }
        }
    }

    fn observe(
        &self,
        event: ProgramStatusEvent,
        host: &host::ConversationHost,
        pump: &super::Pump,
    ) {
        // Queries are handled before emulator replies to preserve the CLI discovery barrier.
        let ProgramStatusEvent::Report(report) = event else {
            return;
        };
        if self.observe_report(report, pump) && !host.setup_running() {
            host.schedule();
        }
    }

    fn observe_report<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        report: prometeu_protocols::terminal_status::ProgramStatusReport,
        pump: &prometeu_core::session::pump::SessionPump<T>,
    ) -> bool {
        if self.gone.load(Ordering::Acquire) {
            return false;
        }
        if report.id.is_none() {
            self.psp_seen.store(true, Ordering::Release);
            self.initial_input_available.store(false, Ordering::Release);
        }
        let ready = report.id.is_none()
            && matches!(
                report.state,
                ProgramStatusState::Idle | ProgramStatusState::Done | ProgramStatusState::Error
            );
        let active = report.id.is_none()
            && matches!(
                report.state,
                ProgramStatusState::Working | ProgramStatusState::Blocked
            );
        let projection = lock(&self.execution).observe(report, conversation::now());
        for frame in projection.events {
            pump.feed(&frame.to_string());
        }
        if active {
            self.ready.store(false, Ordering::Release);
        }
        // SessionStart hooks establish identity, not input readiness. An explicit native idle
        // report can release startup input; bytes, silence and a process spawn cannot.
        ready && !self.ready.swap(true, Ordering::AcqRel) && self.ready()
    }

    fn send_message(&self, text: &str) -> Result<(), String> {
        validate_message(crate::state::ConversationMode::Terminal, text)?;
        if !self.ready() {
            return Err(i18n::t("err.terminalAgent.notReady"));
        }
        let mut input = lock(&self.input);
        if input
            .submission
            .as_ref()
            .is_some_and(|submission| submission.text != text)
        {
            return Err(i18n::t("err.terminalAgent.input"));
        }
        let before = if let Some(readiness) = &self.ui_readiness {
            let snapshot = self
                .screen
                .as_ref()
                .and_then(|screen| screen.view().ok())
                .filter(|snapshot| self.ui_ready(readiness, &snapshot.state));
            if input.local_draft.is_some() || snapshot.is_none() {
                if !self.psp_seen.load(Ordering::Acquire) {
                    self.ready.store(false, Ordering::Release);
                }
                return Err(i18n::t("err.terminalAgent.notReady"));
            }
            snapshot
        } else {
            None
        };
        if input.expected_prompts.len() >= 64
            || input
                .expected_prompts
                .iter()
                .map(|prompt| prompt.text.len())
                .sum::<usize>()
                + text.len()
                > 1024 * 1024
        {
            return Err(i18n::t("err.terminalAgent.inputLimit"));
        }
        let mut paste = prompt_bytes(text)?;
        input.local_draft = Some(lock(&self.output.buffer).seq);
        self.initial_input_available.store(false, Ordering::Release);
        self.ready.store(false, Ordering::Release);
        if let Some(submission) = input.submission.as_mut() {
            submission.write_started = true;
        }
        if let Some(readiness) = self.ui_readiness.filter(|ui| ui.requires_paste_ack()) {
            // Cursor's Ink parser can consume a CR adjoining the paste as draft input. Submit
            // once only after the native composer paints this exact, exclusively owned paste.
            paste.pop();
            self.write_inner(&paste)?;
            let after = before.expect("UI admission retained its screen").chunk_seq;
            self.await_paste(&readiness, text, after)?;
            self.write_inner(b"\r")?;
        } else {
            self.write_inner(&paste)?;
        }
        let prompt = match &input.submission {
            Some(submission) => ExpectedPrompt {
                text: submission.normalized.clone(),
                id: Some(submission.id.clone()),
            },
            None => ExpectedPrompt {
                text: text.into(),
                id: None,
            },
        };
        input.expected_prompts.push_back(prompt);
        self.has_submitted.store(true, Ordering::Release);
        self.ingestion_ready.store(false, Ordering::Release);
        input.local_draft = None;
        input.composer_seen = true;
        input.interrupted = false;
        input.cursor_activity = None;
        lock(&self.execution).accepted_input();
        Ok(())
    }

    fn await_paste(
        &self,
        readiness: &crate::native_ui_readiness::NativeUiReadiness,
        text: &str,
        after: u64,
    ) -> Result<(), String> {
        let screen = self.screen.as_ref().expect("native UI owns a screen");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if !self.connected() {
                return Err(i18n::t("err.chat.gone"));
            }
            let snapshot = screen
                .view()
                .map_err(|_| i18n::t("err.terminalAgent.screen"))?;
            if snapshot.chunk_seq > after && readiness.paste_applied(&snapshot.state, text) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                // Keep the unsubmitted draft visible and input reserved from the app queue.
                // A deadline is never permission to send Enter or repeat an uncertain paste.
                return Err(i18n::t("err.terminalAgent.notReady"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn interrupt(&self) -> Result<(), String> {
        let (Some(controls), Some(screen)) = (&self.controls, &self.screen) else {
            return Err(i18n::t("err.terminalAgent.unsupported"));
        };
        let mut input = lock(&self.input);
        if input.interrupted {
            return Err(i18n::t("err.terminalAgent.notReady"));
        }
        let snapshot = screen
            .view()
            .map_err(|_| i18n::t("err.terminalAgent.screen"))?;
        let bytes = controls
            .interrupt(&snapshot.state, self.busy())
            .ok_or_else(|| i18n::t("err.terminalAgent.unsupported"))?;
        self.write_inner(bytes)?;
        input.interrupted = true;
        Ok(())
    }

    fn close(&self) {
        self.retire();
        self.control.disconnect();
        lock(&self.writer).take();
        // Chat owns escalation and the worker owns reaping; do not start a second lifecycle.
    }
}

fn prompt_bytes(text: &str) -> Result<Vec<u8>, String> {
    if text
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(i18n::t("err.terminalAgent.input"));
    }
    let mut bytes = Vec::with_capacity(text.len() + 13);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~\r");
    Ok(bytes)
}

struct NativeInput(Arc<Transport>);
impl ConversationInput for NativeInput {
    fn send(&mut self, command: &Value, _: &str) -> Result<Vec<Value>, String> {
        match command["type"].as_str() {
            Some("message.send") => self.0.send_message(
                command["text"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.terminalAgent.input"))?,
            )?,
            Some("request.respond")
                if matches!(
                    self.0.provider,
                    crate::state::ProviderId::Cursor | crate::state::ProviderId::Antigravity
                ) =>
            {
                return Err(i18n::t("err.terminalAgent.nativeDecisions"));
            }
            Some("turn.interrupt" | "request.respond") if self.0.codex.is_some() => {
                return self
                    .0
                    .codex
                    .as_ref()
                    .expect("checked native engine")
                    .send(command);
            }
            Some("request.respond") if self.0.decisions.is_some() => {
                return self
                    .0
                    .decisions
                    .as_ref()
                    .expect("checked native decision bridge")
                    .respond(command);
            }
            Some("turn.interrupt") => self.0.interrupt()?,
            // A request must remain open unless its real provider decision channel accepts it.
            _ => return Err(i18n::t("err.terminalAgent.unsupported")),
        }
        Ok(vec![])
    }
}
impl AgentInput for NativeInput {
    fn is_connected(&self) -> bool {
        self.0.connected()
    }
    fn close(&mut self) {
        self.0.close();
    }
    fn requires_idle(&self) -> bool {
        true
    }
}

struct Events {
    app: AppHandle,
    session: String,
    generation: String,
    published: Arc<(Mutex<bool>, Condvar)>,
}
impl TerminalEvents for Events {
    fn output(&self, bytes: &[u8], seq: u64) {
        if !*lock(&self.published.0) {
            return;
        }
        let _ = self
            .app
            .emit("pty", (&self.session, bytes, seq, &self.generation));
    }
    fn closed(&self, code: Option<u32>) {
        if !*lock(&self.published.0) {
            return;
        }
        let _ = self
            .app
            .emit("pty-closed", (&self.session, code, &self.generation));
    }
}

/// Copy arguments and explicit environment changes directly, without a shell or interpolation.
fn pty_command(command: &Command) -> CommandBuilder {
    let mut terminal = CommandBuilder::new(command.get_program());
    prometeu_process::terminal::own_terminal(&mut terminal, Some(env!("CARGO_PKG_VERSION")));
    terminal.args(command.get_args());
    if let Some(cwd) = command.get_current_dir() {
        terminal.cwd(cwd);
    }
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => terminal.env(key, value),
            None => terminal.env_remove(key),
        }
    }
    terminal
}

fn fail_io(transport: &Transport, events: &mpsc::Sender<ReadEvent>, reason: &str, error: String) {
    if !transport.gone.load(Ordering::Acquire) {
        // Logged here because the Failed event is only projected once the tab is published.
        diagnose(&transport.generation, &format!("abort.{reason}"));
        let _ = events.send(ReadEvent::Failed(error));
        transport.control.disconnect();
        transport.process.kill();
    }
}

/// Emulation never waits for canonical capture, publication, or a programmatic paste's
/// acknowledgement. Replies have their own writer so a long input cannot stop output draining.
fn drain_native(
    mut reader: impl Read,
    transport: &Transport,
    events: mpsc::Sender<ReadEvent>,
    replies: mpsc::SyncSender<Vec<Vec<u8>>>,
) {
    let mut parser = ProgramStatusParser::default();
    let mut bytes = [0; 8192];
    loop {
        let count = match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if transport.gone.load(Ordering::Acquire) {
            continue;
        }
        let mut status = parser.feed(&bytes[..count]);
        // Claude completes discovery on DA. Its earlier PSP query must be answered first,
        // even when both queries arrive in one read and the emulator also returns a reply.
        let mut responses = status
            .iter()
            .filter(|event| matches!(event, ProgramStatusEvent::Query))
            .map(|_| PROGRAM_STATUS_QUERY_REPLY.to_vec())
            .collect::<Vec<_>>();
        match transport.output(&bytes[..count]) {
            Ok(native_replies) => responses.extend(native_replies),
            Err(error) => {
                fail_io(transport, &events, "screen_output", error);
                break;
            }
        }
        if transport
            .replaying
            .as_ref()
            .is_some_and(|replaying| replaying.load(Ordering::Acquire))
        {
            responses.clear();
        }
        if !responses.is_empty() && replies.try_send(responses).is_err() {
            // Bounded backpressure must not silently discard a reply or block the PTY reader.
            fail_io(
                transport,
                &events,
                "reply_queue",
                i18n::t("err.terminalAgent.screen"),
            );
            break;
        }
        status.retain(|event| matches!(event, ProgramStatusEvent::Report(_)));
        if !status.is_empty() && events.send(ReadEvent::Status(status)).is_err() {
            break;
        }
    }
}

fn answer_queries(
    transport: &Transport,
    replies: mpsc::Receiver<Vec<Vec<u8>>>,
    events: mpsc::Sender<ReadEvent>,
) {
    for batch in replies {
        for reply in batch {
            // Only live emulator writes produce replies. Restoring a screen never writes to
            // the native CLI; retirement also revokes already queued replies at this gate.
            if transport.gone.load(Ordering::Acquire) {
                return;
            }
            if let Err(error) = transport.write_bytes(&reply) {
                fail_io(transport, &events, "reply_write", error);
                return;
            }
        }
    }
}

/// Where a terminal agent's process runs and where its hooks and transcripts are observed.
/// `Local` is this computer; a remote machine provides its own (ADR 0101).
pub(crate) trait Execution {
    fn observation_roots(
        &self,
        prepared: &crate::terminal_agent_launch::PreparedTerminalAgent,
    ) -> Result<Vec<std::path::PathBuf>, String>;
    fn configure(
        &self,
        bridge: &HookBridge,
        command: &mut Command,
        probe: Option<&mut Command>,
    ) -> Result<(), String>;
    fn open(
        &self,
        app: &AppHandle,
        command: &Command,
        size: TerminalSize,
    ) -> Result<prometeu_core::terminal::StartedTerminal, String>;
    fn replaying(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

pub(crate) struct Local;
impl Execution for Local {
    fn observation_roots(
        &self,
        prepared: &crate::terminal_agent_launch::PreparedTerminalAgent,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        prepared.observation_roots()
    }
    fn configure(
        &self,
        bridge: &HookBridge,
        command: &mut Command,
        probe: Option<&mut Command>,
    ) -> Result<(), String> {
        bridge.configure(command, probe)
    }
    fn open(
        &self,
        app: &AppHandle,
        command: &Command,
        size: TerminalSize,
    ) -> Result<prometeu_core::terminal::StartedTerminal, String> {
        app.state::<AppState>()
            .terminal_factory
            .open(pty_command(command), size)
            .map_err(crate::pty::launch_error)
    }
}

pub(crate) fn launch(
    app: &AppHandle,
    request: &LaunchRequest,
    prepared: crate::terminal_agent_launch::PreparedTerminalAgent,
) -> Result<Chat, String> {
    launch_on(app, request, prepared, &Local)
}

pub(crate) fn launch_on(
    app: &AppHandle,
    request: &LaunchRequest,
    prepared: crate::terminal_agent_launch::PreparedTerminalAgent,
    execution: &dyn Execution,
) -> Result<Chat, String> {
    let provider = prepared.profile.provider;
    let authorized_roots = execution.observation_roots(&prepared)?;
    let cursor_store_root = prepared.cursor_store_root();
    let antigravity_store_root = prepared.antigravity_store_root();
    let crate::terminal_agent_launch::PreparedTerminalAgent {
        mut command,
        mut engine,
        mut hook_probe,
        profile,
        resumed,
        mut provider_session,
        initial_instructions,
    } = prepared;
    let generation = uuid::Uuid::new_v4().to_string();
    // Correlates this incarnation's generation-scoped log lines with its tab.
    diagnose(
        &generation,
        &format!("launch:{}:resumed={resumed}", request.session),
    );
    let (mut bridge, hooks) = HookBridge::start(
        profile.provider,
        &request.session,
        &generation,
        provider_session.as_deref(),
    )?;
    if provider == crate::state::ProviderId::Cursor {
        let mut witness = CursorStopWitness::new(&authorized_roots)
            .map_err(|_| history(&request.session, "cursor_witness"))?;
        bridge.set_cursor_stop_capture(Box::new(move |event| witness.capture(event)));
    }
    let decisions = (provider == crate::state::ProviderId::Claude)
        .then(|| bridge.enable_claude_decisions())
        .transpose()?;
    if let Some(engine) = engine.as_mut() {
        execution.configure(&bridge, &mut engine.command, hook_probe.as_mut())?;
    } else {
        execution.configure(&bridge, &mut command, hook_probe.as_mut())?;
    }
    let engine = engine.map(|plan| plan.start(&mut command)).transpose()?;
    if let Some(engine) = &engine {
        provider_session = Some(engine.thread_id().into());
    }
    let engine_events = engine.as_ref().and_then(|engine| engine.take_events());
    let codex = engine.as_ref().map(|engine| {
        Arc::new(codex::Attached::new(
            engine.attached(request),
            engine.thread_id(),
            engine.configuration()["thread"]["status"]["type"] == "idle",
        ))
    });
    let journal = ingestion::JournalStore::open(
        &paths::chat_log(&request.session),
        &ingestion::journal_path(&paths::root(), &request.session)
            .map_err(|_| history(&request.session, "journal_path"))?,
        JournalBinding {
            app_session: request.session.clone(),
            provider: serde_json::to_value(provider)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| history(&request.session, "journal_provider"))?,
            native_session: provider_session.clone(),
        },
        !resumed,
    )
    .map_err(|_| history(&request.session, "journal_open"))?;
    let cursor_store = if provider == crate::state::ProviderId::Cursor {
        Some(CursorObservationConfig {
            chats_root: cursor_store_root
                .ok_or_else(|| history(&request.session, "cursor_root"))?,
            cli_version: bridge
                .native_version()
                .ok_or_else(|| history(&request.session, "cursor_version"))?
                .into(),
            staging_root: paths::root().join("terminal-ingestion").join("snapshots"),
        })
    } else {
        None
    };
    let antigravity_store = if provider == crate::state::ProviderId::Antigravity {
        let conversations_root =
            antigravity_store_root.ok_or_else(|| history(&request.session, "antigravity_root"))?;
        let brain_root = conversations_root
            .parent()
            .ok_or_else(|| history(&request.session, "antigravity_parent"))?
            .join("brain");
        Some(AntigravityObservationConfig {
            conversations_root,
            brain_root,
            cli_version: bridge
                .native_version()
                .ok_or_else(|| history(&request.session, "antigravity_version"))?
                .into(),
            staging_root: paths::root().join("terminal-ingestion").join("snapshots"),
        })
    } else {
        None
    };
    let mut observations = Observations::new(
        ObservationConfig {
            provider: profile.provider,
            session: request.session.clone(),
            process_nonce: generation.clone(),
            expected_native_session: provider_session.clone(),
            start: if resumed {
                ObservationStart::Resume
            } else {
                ObservationStart::Fresh
            },
            authorized_roots,
        },
        cursor_store,
        antigravity_store,
        bridge.native_version(),
    )?;
    if let (Some(codex), Some(engine)) = (&codex, &engine) {
        codex
            .replay(engine.history(), journal.as_ref())
            .inspect_err(|_| diagnose(&request.session, "history.codex_replay"))?;
    }
    if resumed && engine.is_none() {
        let native = provider_session
            .as_deref()
            .ok_or_else(|| history(&request.session, "resume_native"))?;
        let saved =
            binding::Binding::load(&paths::root(), &request.session, profile.provider, native)
                .map_err(|_| history(&request.session, "binding_load"))?;
        if let Some(saved) = saved {
            observations
                .native
                .resume_from(saved.transcript.clone())
                .map_err(|_| history(&request.session, "resume_from"))?;
            observations.binding = Some(saved);
        } else if !matches!(
            provider,
            crate::state::ProviderId::Cursor | crate::state::ProviderId::Antigravity
        ) {
            return Err(history(&request.session, "binding_missing"));
        }
        // Cursor and AGY reauthorize their captured store and exact native metadata identity directly.
        // Its asynchronous SessionStart can precede creation of the derived JSONL binding.
        // Read the baseline before the CLI can append to it. In particular Antigravity resume
        // reports its next hook only after a prompt, so waiting for that hook would deadlock.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match observations.native.poll() {
                Ok(()) | Err(crate::terminal_agent_observation::ObservationError::Unstable) => {}
                Err(error) => {
                    return Err(history(
                        &request.session,
                        &format!("baseline_poll:{error:?}"),
                    ))
                }
            }
            observations
                .replay_pending(journal.as_ref())
                .map_err(|error| history(&request.session, &format!("replay_pending:{error}")))?;
            if observations.native.baseline_ready()
                && observations.native.caught_up()
                && journal.caught_up()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(history(
                    &request.session,
                    &format!(
                        "baseline_timeout:ready={}:native={}:journal={}",
                        observations.native.baseline_ready(),
                        observations.native.caught_up(),
                        journal.caught_up()
                    ),
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    let ui_readiness = bridge.native_version().and_then(|version| {
        crate::native_ui_readiness::NativeUiReadiness::verified(profile.provider, version)
    });
    let controls = bridge.native_version().and_then(|version| {
        crate::terminal_agent_controls::NativeControls::verified(profile.provider, version).ok()
    });
    if let Some(instructions) = initial_instructions {
        bridge.configure_instructions(&mut command, &instructions)?;
    }
    let store: Arc<dyn TranscriptStore> = journal.clone();
    // Recovery must finish before seeding. A native canonical history that cannot be read must
    // never be replaced with an empty buffer and then extended under incorrect line offsets.
    let lines = journal
        .seeded()
        .map_err(|_| history(&request.session, "seed"))?;
    let screen = screen::Screen::launch(
        &request.session,
        &generation,
        TerminalSize { cols: 80, rows: 24 },
    )
    .map_err(|_| i18n::t("err.terminalAgent.screen"))?;
    let state = app.state::<AppState>();
    let started = execution.open(app, &command, TerminalSize { cols: 80, rows: 24 })?;
    let (pump, context) = super::compose_seeded(app, &request.session, profile, store, lines);
    let pump = pump.with_commit(journal.clone());
    let published = Arc::new((Mutex::new(false), Condvar::new()));
    let events = Arc::new(Events {
        app: app.clone(),
        session: request.session.clone(),
        generation: generation.clone(),
        published: published.clone(),
    });
    let transport = Arc::new(Transport {
        generation: generation.clone(),
        provider,
        input: Mutex::new(Input {
            expected_prompts: VecDeque::new(),
            submission: None,
            local_draft: None,
            composer_seen: false,
            interrupted: false,
            cursor_activity: None,
        }),
        writer: Mutex::new(Some(started.input)),
        control: started.control,
        process: started.process.control.clone(),
        output: Arc::new(TerminalOutput::new(events.clone())),
        events,
        closed: Mutex::new(false),
        gone: pump.output.gone.clone(),
        ready: AtomicBool::new(false),
        baseline_ready: AtomicBool::new(codex.is_some() || !resumed),
        has_submitted: AtomicBool::new(resumed),
        initial_input_available: AtomicBool::new(!resumed),
        ingestion_ready: AtomicBool::new(resumed),
        psp_seen: AtomicBool::new(false),
        composer_ready: AtomicBool::new(false),
        journal: Some(journal),
        screen: Some(screen),
        ui_readiness,
        controls,
        execution: Mutex::new(ExecutionProjector::new(&generation).expect("generated incarnation")),
        cursor_interrupted_requests: Mutex::new(BTreeSet::new()),
        codex,
        decisions,
        activity: Some(pump.output.clone()),
        published,
        replaying: execution.replaying(),
    });
    let chat = Chat {
        wire: Box::new(NativeInput(transport.clone())),
        buffer: pump.output.lines.clone(),
        process: started.process,
        pump: pump.clone(),
        context: context.clone(),
        terminal: Some(transport.clone()),
    };
    let (sender, receiver) = mpsc::channel();
    let (reply_sender, reply_receiver) = mpsc::sync_channel(128);
    let reply_transport = transport.clone();
    let reply_events = sender.clone();
    state
        .tasks
        .spawn(Box::new(move || {
            answer_queries(&reply_transport, reply_receiver, reply_events);
        }))
        .map_err(i18n::io)?;
    let reader_transport = transport.clone();
    state
        .tasks
        .spawn(Box::new(move || {
            drain_native(started.output, &reader_transport, sender, reply_sender);
        }))
        .map_err(i18n::io)?;
    let identity = chat.process.identity.clone();
    let mut waiter = started.waiter;
    state
        .tasks
        .spawn(Box::new(move || {
            // Publishing the tab precedes output/identity/exit. A CLI that exits immediately must
            // not lose its closure against a registry entry that has not been installed yet.
            let published = transport.await_publication();
            if published {
                pump.feed(
                    &conversation::event(
                        "session.state",
                        conversation::now(),
                        json!({"state":"starting"}),
                    )
                    .to_string(),
                );
                if let Some(identity) = provider_session {
                    observations.identity = Some(identity.clone());
                    pump.feed(
                        &conversation::event(
                            "session.identity",
                            conversation::now(),
                            json!({"providerSession":identity}),
                        )
                        .to_string(),
                    );
                }
            }
            if published {
                if let Some(codex) = &transport.codex {
                    if codex.refresh_usage().is_err() {
                        transport.abort_submission("", "codex_usage");
                    }
                }
            }
            let mut inspected = Instant::now();
            loop {
                if published {
                    transport.drain_decisions(&pump);
                }
                if let (Some(codex), Some(events)) = (&transport.codex, &engine_events) {
                    if published && codex.poll(events, &transport, &pump, &context).is_err()
                        || engine.as_ref().is_some_and(|engine| !engine.running())
                    {
                        transport.abort_submission("", "codex_poll");
                        break;
                    }
                }
                match receiver.recv_timeout(Duration::from_millis(25)) {
                    Ok(ReadEvent::Status(status_events)) if published => {
                        // Observe persisted content before projecting any status in the same
                        // output batch. The terminal bytes themselves remain provider-owned.
                        if observations.queue_status(status_events).is_err() {
                            transport.abort_submission("", "status_queue");
                            break;
                        }
                        observations.poll(&transport, &hooks, &pump, &context);
                    }
                    Ok(ReadEvent::Failed(error)) if published => {
                        pump.feed(
                            &conversation::event(
                                "system.notice",
                                conversation::now(),
                                json!({"level":"error", "code":"terminal.screen", "detail":error}),
                            )
                            .to_string(),
                        );
                    }
                    Ok(_) => {}
                    Err(RecvTimeoutError::Timeout) if published => {
                        observations.poll(&transport, &hooks, &pump, &context);
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                // This cadence only bounds screen reads. Readiness comes from the current
                // verified composer, never from a duration without output.
                if published && inspected.elapsed() >= Duration::from_millis(200) {
                    transport.inspect_readiness(&context, &pump);
                    inspected = Instant::now();
                }
                if published {
                    transport.drain_decisions(&pump);
                }
            }
            // EOF is not process exit. The sole waiter reaps before either closure notification.
            let exit = waiter.wait();
            drop(waiter);
            if let (Some(codex), Some(events)) = (&transport.codex, &engine_events) {
                if published && !transport.gone.load(Ordering::Acquire) {
                    let _ = codex.poll(events, &transport, &pump, &context);
                }
            }
            if let Some(engine) = &engine {
                engine.stop();
            }
            // Synchronous native hooks may already be queued when the child exits. Retire their
            // sender and publish their final content before lifecycle marks this tab stopped.
            bridge.close();
            if published {
                transport.drain_decisions(&pump);
            }
            if published && !transport.gone.load(Ordering::Acquire) {
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    observations.poll(&transport, &hooks, &pump, &context);
                    if (observations.native.caught_up()
                        && transport
                            .journal()
                            .is_none_or(|journal| journal.caught_up()))
                        || Instant::now() >= deadline
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
            let retired = lock(&transport.execution).retire(conversation::now());
            for frame in retired {
                pump.feed(&frame.to_string());
            }
            let code = exit.as_ref().ok().copied();
            diagnose(&transport.generation, &format!("exit:{code:?}"));
            if let Some(screen) = &transport.screen {
                // TerminalOutput's empty exit chunk also consumes a chunk sequence.
                let _ = screen.write(b"");
                let _ = screen.freeze();
            }
            transport.output.finish(code, b"");
            transport.announce_closed(code);
            context.exited(
                &identity,
                exit.map(|code| ProcessExit {
                    code: i32::try_from(code).ok(),
                }),
            );
        }))
        .map_err(i18n::io)?;
    Ok(chat)
}

pub(crate) fn get(state: &AppState, session: &str) -> Option<Arc<Transport>> {
    lock(&state.sessions.conversations)
        .get(session)
        .and_then(|chat| chat.terminal.clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSnapshot {
    data: Vec<u8>,
    seq: u64,
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    screen: Option<prometeu_process::terminal_state::Snapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    query_replies: Option<&'static str>,
}

#[tauri::command(async)]
pub fn agent_terminal_snapshot(
    state: State<AppState>,
    session: String,
) -> Result<TerminalSnapshot, String> {
    let Some(terminal) = get(&state, &session) else {
        return Ok(TerminalSnapshot {
            data: vec![],
            seq: 0,
            running: false,
            generation: None,
            screen: None,
            query_replies: None,
        });
    };
    if let Some(screen) = &terminal.screen {
        let snapshot = screen
            .snapshot()
            .map_err(|_| i18n::t("err.terminalAgent.screen"))?;
        return Ok(TerminalSnapshot {
            data: Vec::new(),
            seq: snapshot.chunk_seq,
            running: terminal.connected(),
            generation: Some(terminal.generation.clone()),
            screen: Some(snapshot.state),
            query_replies: Some("host"),
        });
    }
    let scroll = lock(&terminal.output.buffer);
    Ok(TerminalSnapshot {
        data: scroll.bytes.clone(),
        seq: scroll.seq,
        running: terminal.connected(),
        generation: Some(terminal.generation.clone()),
        screen: None,
        query_replies: None,
    })
}

// Async: provider preparation runs external CLIs (with multi-second timeouts) and must not block the GUI thread.
#[tauri::command(async)]
pub fn agent_terminal_resume(
    app: AppHandle,
    state: State<AppState>,
    session: String,
) -> Result<(), String> {
    let gate = super::input_gate(&state, &session);
    let _input = lock(&gate);
    if !ENABLED {
        return Err(i18n::t("err.terminalAgent.unsupported"));
    }
    let terminal = lock(&state.board)
        .tab_mut(&session)
        .is_some_and(|tab| tab.conversation_mode == crate::state::ConversationMode::Terminal);
    if !terminal {
        return Err(i18n::t("err.session.noTab"));
    }
    if get(&state, &session).is_some_and(|terminal| terminal.connected()) {
        return Ok(());
    }
    crate::session::revive(&app, &state, &session).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_waits_for_queued_diagnostics_until_the_deadline() {
        let pending = Arc::new(std::sync::atomic::AtomicUsize::new(1));
        let writer = Arc::clone(&pending);
        let started = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            writer.store(0, Ordering::SeqCst);
        });
        wait_drained(&pending, Duration::from_secs(5));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(100),
            "returned before the line was written"
        );
        assert!(
            waited < Duration::from_secs(5),
            "waited past a drained queue"
        );

        // A stalled writer never drains: the quit still proceeds at the deadline.
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let stalled = std::sync::atomic::AtomicUsize::new(1);
            let started = Instant::now();
            wait_drained(&stalled, Duration::from_millis(50));
            let _ = sender.send(started.elapsed());
        });
        let waited = receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("a stalled writer must not hold the wait open");
        assert!(waited >= Duration::from_millis(50));
    }

    struct Memory(Arc<Mutex<Vec<u8>>>);
    impl Write for Memory {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Control;
    impl ProcessControl for Control {
        fn system_id(&self) -> u32 {
            1
        }
        fn running(&self) -> bool {
            true
        }
        fn interrupt(&self) {
            panic!("interrupt must reach native terminal input")
        }
        fn terminate(&self) {}
        fn kill(&self) {}
        fn wait_exit(&self, _: std::time::Duration) -> bool {
            false
        }
    }
    impl TerminalControl for Control {
        fn resize(&self, _: TerminalSize) -> Result<(), String> {
            Ok(())
        }
        fn running(&self) -> bool {
            true
        }
        fn system_id(&self) -> u32 {
            1
        }
        fn close(&self) {
            panic!("Chat owns shutdown")
        }
    }
    impl TerminalEvents for Control {
        fn output(&self, _: &[u8], _: u64) {}
        fn closed(&self, _: Option<u32>) {}
    }
    pub(super) fn fixture() -> (Arc<Transport>, Arc<Mutex<Vec<u8>>>) {
        let bytes = Arc::new(Mutex::new(vec![]));
        (
            Arc::new(Transport {
                generation: "incarnation".into(),
                provider: crate::state::ProviderId::Claude,
                input: Mutex::new(Input {
                    expected_prompts: VecDeque::new(),
                    submission: None,
                    local_draft: None,
                    composer_seen: false,
                    interrupted: false,
                    cursor_activity: None,
                }),
                writer: Mutex::new(Some(Box::new(Memory(bytes.clone())))),
                control: Arc::new(Control),
                process: Arc::new(Control),
                output: Arc::new(TerminalOutput::new(Arc::new(Control))),
                events: Arc::new(Control),
                closed: Mutex::new(false),
                gone: Arc::new(AtomicBool::new(false)),
                ready: AtomicBool::new(false),
                baseline_ready: AtomicBool::new(true),
                has_submitted: AtomicBool::new(false),
                initial_input_available: AtomicBool::new(true),
                ingestion_ready: AtomicBool::new(false),
                psp_seen: AtomicBool::new(false),
                composer_ready: AtomicBool::new(false),
                journal: None,
                screen: None,
                ui_readiness: None,
                controls: None,
                codex: None,
                decisions: None,
                execution: Mutex::new(ExecutionProjector::new("incarnation").unwrap()),
                cursor_interrupted_requests: Mutex::new(BTreeSet::new()),
                activity: None,
                published: Arc::new((Mutex::new(false), Condvar::new())),
                replaying: None,
            }),
            bytes,
        )
    }

    #[test]
    fn initial_input_admission_is_antigravity_only_and_never_restores_after_activity() {
        let (mut transport, _) = fixture();
        assert!(!transport.initial_input_admissible());
        Arc::get_mut(&mut transport).unwrap().provider = crate::state::ProviderId::Antigravity;
        assert!(transport.initial_input_admissible());
        transport.has_submitted.store(true, Ordering::Release);
        assert!(
            !transport.initial_input_admissible(),
            "includes resumed sessions"
        );
        transport.has_submitted.store(false, Ordering::Release);
        transport.psp_seen.store(true, Ordering::Release);
        assert!(!transport.initial_input_admissible());
        transport.psp_seen.store(false, Ordering::Release);
        lock(&transport.execution).observe_fact(
            ExecutionFact::Working {
                id: None,
                detail: None,
            },
            conversation::now(),
        );
        assert!(!transport.initial_input_admissible());
        *lock(&transport.execution) = ExecutionProjector::new("incarnation").unwrap();
        transport
            .initial_input_available
            .store(false, Ordering::Release);
        assert!(
            !transport.initial_input_admissible(),
            "an empty repaint cannot restore the one-use gate"
        );
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn antigravity_initial_composer_admits_one_input_without_releasing_a_later_turn() {
        for local_input in [false, true] {
            let (mut transport, bytes) = fixture();
            let inner = Arc::get_mut(&mut transport).unwrap();
            inner.provider = crate::state::ProviderId::Antigravity;
            inner.ui_readiness = crate::native_ui_readiness::NativeUiReadiness::verified(
                crate::state::ProviderId::Antigravity,
                "1.5.10",
            );
            inner.screen = Some(
                screen::Screen::launch(
                    "agy-initial-input",
                    "generation",
                    TerminalSize { cols: 83, rows: 44 },
                )
                .unwrap(),
            );
            // The owned actual-app capture has this exact composer geometry; other viewport
            // content is irrelevant to input admission and is deliberately not reproduced.
            let rule = "─".repeat(83);
            let empty = format!("\x1b[2J\x1b[H\x1b[?25h\x1b[?2004h\x1b[14;1H{rule}\x1b[15;1H>\x1b[16;1H{rule}\x1b[17;1H? for shortcuts                                             Gemini 3.8 Flash · high\x1b[15;3H");
            transport.output(empty.as_bytes()).unwrap();
            transport
                .output(b"\x1b[17;1H\x1b[2KEnter to confirm\x1b[15;3H")
                .unwrap();
            assert!(!transport.refresh_readiness());
            assert!(!transport.ready());
            transport.output(empty.as_bytes()).unwrap();
            assert!(transport.refresh_readiness());
            assert!(transport.ready());
            assert!(!transport.psp_seen.load(Ordering::Acquire));
            assert!(!transport.busy(), "UI admission creates no activity");
            if local_input {
                transport.write_user_bytes(b"local input\r").unwrap();
                assert_eq!(*lock(&bytes), b"local input\r");
            } else {
                transport.send_message("First synthetic prompt").unwrap();
                assert_eq!(
                    *lock(&bytes),
                    prompt_bytes("First synthetic prompt").unwrap()
                );
            }
            transport.output(empty.as_bytes()).unwrap();
            assert!(!transport.refresh_readiness());
            assert!(
                !transport.ready(),
                "an empty paint after input does not prove native idle"
            );
            assert!(transport.send_message("Must wait for native idle").is_err());
            // Native idle retains its existing authority after the startup-only gate closes.
            transport.psp_seen.store(true, Ordering::Release);
            transport.ready.store(true, Ordering::Release);
            transport.refresh_readiness();
            assert!(transport.ready());
            transport.screen.as_ref().unwrap().freeze().unwrap();
        }
    }

    #[test]
    fn native_input_waits_for_readiness_and_keeps_requests_open() {
        let (transport, bytes) = fixture();
        let mut input = NativeInput(transport.clone());
        let message = json!({"type":"message.send","text":"First prompt"});
        assert!(input.send(&message, "").is_err());
        assert!(lock(&bytes).is_empty());
        transport.ready.store(true, Ordering::Release);
        transport.baseline_ready.store(false, Ordering::Release);
        assert!(input.send(&message, "").is_err());
        assert!(lock(&bytes).is_empty());
        transport.baseline_ready.store(true, Ordering::Release);
        input.send(&message, "").unwrap();
        assert_eq!(*lock(&bytes), prompt_bytes("First prompt").unwrap());
        lock(&bytes).clear();
        assert!(!transport.ready());
        assert!(input
            .send(&json!({"type":"message.send","text":"Follow up"}), "")
            .is_err());
        transport.ready.store(true, Ordering::Release);
        input
            .send(&json!({"type":"message.send","text":"Follow up"}), "")
            .unwrap();
        assert_eq!(*lock(&bytes), prompt_bytes("Follow up").unwrap());
        let before = lock(&bytes).clone();
        assert!(input
            .send(
                &json!({"type":"request.respond","requestId":"native","decision":"allow"}),
                ""
            )
            .is_err());
        assert_eq!(*lock(&bytes), before);
        assert!(input.send(&json!({"type":"turn.interrupt"}), "").is_err());
        assert_eq!(
            *lock(&bytes),
            before,
            "unverified native controls write nothing"
        );
        input.close();
        assert!(!input.is_connected());
        assert!(transport.write_bytes(b"late input").is_err());
    }

    #[test]
    fn native_commands_never_reserve_a_prompt_or_write_programmatic_input() {
        use crate::state::{ConversationMode, ProviderId};
        for provider in [
            ProviderId::Claude,
            ProviderId::Codex,
            ProviderId::Antigravity,
            ProviderId::Cursor,
        ] {
            let (mut transport, bytes) = fixture();
            Arc::get_mut(&mut transport).unwrap().provider = provider;
            transport.ready.store(true, Ordering::Release);
            for command in [
                "/",
                "/compact",
                "  /context details",
                "/plugin:command\nargs",
            ] {
                assert!(validate_message(ConversationMode::Chat, command).is_ok());
                assert_eq!(
                    transport.begin_submission("native-command", command),
                    Err(i18n::t("err.terminalAgent.nativeCommands"))
                );
                assert_eq!(
                    transport.send_message(command),
                    Err(i18n::t("err.terminalAgent.nativeCommands"))
                );
                let input = lock(&transport.input);
                assert!(input.submission.is_none());
                assert!(input.expected_prompts.is_empty());
                assert!(input.local_draft.is_none());
                assert!(transport.ready.load(Ordering::Acquire));
                assert!(lock(&bytes).is_empty());
            }
            // The person can still run native commands directly, without reserving a model turn.
            transport.write_bytes(b"/compact\r").unwrap();
            assert_eq!(*lock(&bytes), b"/compact\r");
        }
        for prompt in [
            "Explain /compact",
            "/Users/me/file",
            "//server/file",
            "@/tmp/file",
            "",
        ] {
            assert!(validate_message(ConversationMode::Terminal, prompt).is_ok());
        }
    }

    #[test]
    fn native_only_decisions_reject_application_responses_without_disabling_native_input() {
        for provider in [
            crate::state::ProviderId::Cursor,
            crate::state::ProviderId::Antigravity,
        ] {
            let (mut transport, bytes) = fixture();
            Arc::get_mut(&mut transport).unwrap().provider = provider;
            let mut input = NativeInput(transport.clone());
            let error = input
            .send(
                &json!({"type":"request.respond","requestId":"stale-card","response":{"outcome":"allow"}}),
                "",
            )
            .unwrap_err();
            assert_eq!(error, i18n::t("err.terminalAgent.nativeDecisions"));
            assert!(lock(&bytes).is_empty());
            transport.write_bytes(b"y").unwrap();
            assert_eq!(*lock(&bytes), b"y");
        }
    }

    #[test]
    fn submission_identity_is_reserved_before_write_and_survives_until_capture_commits() {
        let (mut transport, bytes) = fixture();
        Arc::get_mut(&mut transport).unwrap().provider = crate::state::ProviderId::Cursor;
        transport.ready.store(true, Ordering::Release);
        let original = "A\r\nB\tC";
        assert_eq!(
            transport
                .begin_submission("submission-one", original)
                .unwrap(),
            "A\nB  C"
        );
        assert!(transport
            .begin_submission("submission-two", original)
            .is_err());
        assert!(transport.send_message("unrelated input").is_err());
        assert!(lock(&bytes).is_empty());
        // A native command may have returned to idle without producing a source user record.
        lock(&transport.input).cursor_activity = Some(CursorActivity {
            after: 0,
            started: false,
        });
        transport.send_message(original).unwrap();
        assert_eq!(*lock(&bytes), prompt_bytes(original).unwrap());
        {
            let input = lock(&transport.input);
            assert!(
                input.cursor_activity.is_none(),
                "the app submission owns the new cycle"
            );
            assert_eq!(input.submission.as_ref().unwrap().id, "submission-one");
            let expected = input.expected_prompts.front().unwrap();
            assert_eq!(expected.id.as_deref(), Some("submission-one"));
            assert_eq!(expected.text, "A\nB  C");
        }
        transport.end_submission("unrelated");
        assert!(lock(&transport.input).submission.is_some());
        transport.end_submission("submission-one");
        assert!(lock(&transport.input).submission.is_none());
        assert_eq!(
            *lock(&bytes),
            prompt_bytes(original).unwrap(),
            "ending capture never writes again"
        );
    }

    #[test]
    fn publication_or_retirement_releases_the_worker_and_generations_never_alias() {
        for publish in [true, false] {
            let (transport, _) = fixture();
            assert!(transport.owns(Some("incarnation")));
            assert!(!transport.owns(None));
            assert!(!transport.owns(Some("older")));
            let worker = transport.clone();
            let (sender, receiver) = mpsc::channel();
            let task = std::thread::spawn(move || {
                sender.send(worker.await_publication()).unwrap();
            });
            assert!(receiver.try_recv().is_err());
            if publish {
                assert_eq!(transport.publish().as_deref(), Some("incarnation"));
                assert!(transport.publish().is_none());
            } else {
                transport.retire();
            }
            assert_eq!(
                receiver
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap(),
                publish
            );
            task.join().unwrap();
        }
    }

    #[test]
    fn drain_continues_before_publication_while_capture_and_writer_are_reserved() {
        let (transport, bytes) = fixture();
        let input = lock(&transport.input);
        let writer = lock(&transport.writer);
        let (events, receiver) = mpsc::channel();
        let (replies, reply_receiver) = mpsc::sync_channel(128);
        let query_transport = transport.clone();
        let query_events = events.clone();
        let query_thread = std::thread::spawn(move || {
            answer_queries(&query_transport, reply_receiver, query_events);
        });
        let data = b"\x1b]7501;?\x07\x1b]7501;state=working\x07painted draft";
        let reader_transport = transport.clone();
        let (done, drained) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            drain_native(data.as_slice(), &reader_transport, events, replies);
            done.send(()).unwrap();
        });
        drained.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(lock(&transport.output.buffer).bytes, data);
        assert!(
            matches!(receiver.try_recv().unwrap(), ReadEvent::Status(events) if events.len() == 1)
        );
        assert!(
            lock(&bytes).is_empty(),
            "query waits only for physical input"
        );
        drop(writer);
        query_thread.join().unwrap();
        reader_thread.join().unwrap();
        assert_eq!(*lock(&bytes), PROGRAM_STATUS_QUERY_REPLY);
        drop(input);
        assert!(!*lock(&transport.published.0));
    }

    #[test]
    fn retirement_revokes_queued_native_query_replies() {
        let (transport, bytes) = fixture();
        let (sender, replies) = mpsc::channel();
        sender
            .send(vec![PROGRAM_STATUS_QUERY_REPLY.to_vec()])
            .unwrap();
        drop(sender);
        let (events, _) = mpsc::channel();
        transport.retire();
        answer_queries(&transport, replies, events);
        assert!(lock(&bytes).is_empty());
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn native_discovery_replies_keep_psp_before_the_emulator_da_barrier() {
        let (mut transport, _) = fixture();
        Arc::get_mut(&mut transport).unwrap().screen = Some(
            screen::Screen::launch(
                "query-test",
                "generation",
                TerminalSize { cols: 80, rows: 24 },
            )
            .unwrap(),
        );
        let (events, _) = mpsc::channel();
        let (replies, receiver) = mpsc::sync_channel(128);
        drain_native(
            b"\x1b]7501;?\x07\x1b[c".as_slice(),
            &transport,
            events,
            replies,
        );
        let batch = receiver.recv().unwrap();
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0], PROGRAM_STATUS_QUERY_REPLY);
        assert_eq!(batch[1], b"\x1b[?1;2c");
        transport.screen.as_ref().unwrap().freeze().unwrap();
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn cursor_submission_waits_for_painted_paste_and_retirement_cannot_send_enter() {
        struct Writes(mpsc::Sender<Vec<u8>>);
        impl Write for Writes {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.send(bytes.to_vec()).unwrap();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        for retire in [false, true] {
            let (mut transport, _) = fixture();
            let (writes, received) = mpsc::channel();
            let inner = Arc::get_mut(&mut transport).unwrap();
            inner.writer = Mutex::new(Some(Box::new(Writes(writes))));
            inner.ui_readiness = crate::native_ui_readiness::NativeUiReadiness::verified(
                crate::state::ProviderId::Cursor,
                "2026.11.13-eacade1",
            );
            inner.screen = Some(
                screen::Screen::launch(
                    "paste-test",
                    "generation",
                    TerminalSize { cols: 80, rows: 24 },
                )
                .unwrap(),
            );
            transport.output(b"\x1b[2J\x1b[H\x1b[?25l\x1b[?2004h\x1b[2;3HCursor Agent\x1b[15;3H\x1b[2m\xe2\x86\x92 \x1b[0;7mP\x1b[0;2mlan, search, build anything\x1b[0m\x1b[19;3H/tmp/native-smoke/workspace\x1b[20;1H").unwrap();
            let snapshot = transport.screen.as_ref().unwrap().snapshot().unwrap();
            assert!(transport.ui_readiness.unwrap().ready(&snapshot.state));
            transport.ready.store(true, Ordering::Release);
            transport.composer_ready.store(true, Ordering::Release);
            let sending = transport.clone();
            let thread = std::thread::spawn(move || sending.send_message("First synthetic line."));
            let mut paste = prompt_bytes("First synthetic line.").unwrap();
            paste.pop();
            assert_eq!(
                received.recv_timeout(Duration::from_secs(2)).unwrap(),
                paste
            );
            assert!(
                received.try_recv().is_err(),
                "no Enter before draft acknowledgement"
            );
            if retire {
                transport.retire();
                assert!(thread.join().unwrap().is_err());
                assert!(received.try_recv().is_err());
                assert!(lock(&transport.input).expected_prompts.is_empty());
            } else {
                // The reader can paint while the input/canonical transaction remains reserved.
                transport.output(b"\x1b[15;1H\x1b[2K  \xe2\x86\x92 First synthetic line.\x1b[7m \x1b[0m\x1b[20;1H").unwrap();
                thread.join().unwrap().unwrap();
                assert_eq!(
                    received.recv_timeout(Duration::from_secs(2)).unwrap(),
                    b"\r"
                );
                assert!(received.try_recv().is_err(), "Enter is sent exactly once");
                assert_eq!(lock(&transport.input).expected_prompts.len(), 1);
            }
            transport.screen.as_ref().unwrap().freeze().unwrap();
        }
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn native_local_input_releases_only_after_a_new_verified_empty_composer() {
        let (mut transport, _) = fixture();
        let inner = Arc::get_mut(&mut transport).unwrap();
        inner.ui_readiness = crate::native_ui_readiness::NativeUiReadiness::verified(
            crate::state::ProviderId::Cursor,
            "2026.11.13-eacade1",
        );
        inner.screen = Some(
            screen::Screen::launch(
                "local-input-test",
                "generation",
                TerminalSize { cols: 80, rows: 24 },
            )
            .unwrap(),
        );
        transport.write_user_bytes(b"a").unwrap();
        assert!(
            lock(&transport.input).local_draft.is_none(),
            "startup trust input precedes composer ownership"
        );
        let empty = b"\x1b[2J\x1b[H\x1b[?25l\x1b[?2004h\x1b[2;3HCursor Agent\x1b[15;3H\x1b[2m\xe2\x86\x92 \x1b[0;7mP\x1b[0;2mlan, search, build anything\x1b[0m\x1b[19;3H/tmp/native-smoke/workspace\x1b[20;1H";
        transport.output(empty).unwrap();
        assert!(transport.refresh_readiness());
        transport.psp_seen.store(true, Ordering::Release);
        transport.write_user_bytes(b"x").unwrap();
        assert!(!transport.ready());
        assert!(
            !transport.refresh_readiness(),
            "the pre-input empty snapshot cannot release a draft"
        );
        transport.output(b"\x1b[15;1H\x1b[2K  \xe2\x86\x92 Plan, search, build anything\x1b[7m \x1b[0m\x1b[20;1H").unwrap();
        assert!(
            !transport.refresh_readiness(),
            "literal placeholder text is still a draft"
        );
        transport.output(empty).unwrap();
        assert!(transport.refresh_readiness());
        assert!(transport.ready());
        assert!(lock(&transport.input).local_draft.is_none());
        transport.screen.as_ref().unwrap().freeze().unwrap();
    }

    #[test]
    #[ignore = "requires PROMETEU_TERMINAL_STATE_HELPER pointing to the built SEA helper"]
    fn antigravity_interrupted_repaint_releases_escape_fence_and_admits_followup() {
        let captured: Value = serde_json::from_str(include_str!(
            "../terminal_agent_controls/fixtures/antigravity-1.5.10-interrupted.json"
        ))
        .unwrap();
        let cols = captured["cols"].as_u64().unwrap() as u16;
        let rows = captured["rows"].as_u64().unwrap() as u16;
        let cursor_y = captured["cursor"]["y"].as_u64().unwrap();
        let cursor_x = captured["cursor"]["x"].as_u64().unwrap();
        // Replay the captured native layout through the real emulator. Its empty ASCII
        // composer has default attributes; historical text is outside the input boundary.
        let paint = |busy: bool| {
            let mut bytes = String::from("\x1b[2J\x1b[H\x1b[?25h\x1b[?2004h");
            for (row, line) in captured["lines"].as_array().unwrap().iter().enumerate() {
                let line = if busy && row == cursor_y as usize + 2 {
                    "Esc to interrupt"
                } else {
                    line.as_str().unwrap()
                };
                bytes.push_str(&format!("\x1b[{};1H{line}", row + 1));
            }
            bytes.push_str(&format!("\x1b[{};{}H", cursor_y + 1, cursor_x + 1));
            bytes
        };
        let (mut transport, bytes) = fixture();
        let inner = Arc::get_mut(&mut transport).unwrap();
        inner.provider = crate::state::ProviderId::Antigravity;
        inner.ui_readiness = crate::native_ui_readiness::NativeUiReadiness::verified(
            crate::state::ProviderId::Antigravity,
            "1.5.10",
        );
        inner.screen = Some(
            screen::Screen::launch(
                "agy-interrupted-input",
                "generation",
                TerminalSize { cols, rows },
            )
            .unwrap(),
        );
        inner.has_submitted.store(true, Ordering::Release);
        inner
            .initial_input_available
            .store(false, Ordering::Release);
        inner.psp_seen.store(true, Ordering::Release);
        lock(&inner.input).composer_seen = true;
        transport.output(paint(true).as_bytes()).unwrap();
        transport.write_user_bytes(b"\x1b").unwrap();
        assert!(lock(&transport.input).local_draft.is_some());
        // Native Idle arrives before the new paint. It cannot by itself release local input.
        transport.ready.store(true, Ordering::Release);
        assert!(!transport.refresh_readiness());
        assert!(transport.send_message("Queued follow-up").is_err());
        assert_eq!(*lock(&bytes), b"\x1b");
        transport.output(paint(false).as_bytes()).unwrap();
        assert!(
            transport.refresh_readiness(),
            "new interrupted repaint releases the native input fence"
        );
        assert!(transport.ready());
        assert!(lock(&transport.input).local_draft.is_none());
        assert!(
            !transport.refresh_readiness(),
            "the ready edge is published only once"
        );
        transport.send_message("Queued follow-up").unwrap();
        let mut expected = vec![0x1b];
        expected.extend(prompt_bytes("Queued follow-up").unwrap());
        assert_eq!(*lock(&bytes), expected);
        assert!(
            !transport.ready(),
            "one follow-up consumes native input readiness"
        );
        transport.screen.as_ref().unwrap().freeze().unwrap();
    }

    #[test]
    fn retirement_and_native_exit_publish_only_one_generation_scoped_closure() {
        #[derive(Default)]
        struct Closures(Mutex<Vec<Option<u32>>>);
        impl TerminalEvents for Closures {
            fn output(&self, _: &[u8], _: u64) {}
            fn closed(&self, code: Option<u32>) {
                lock(&self.0).push(code);
            }
        }
        for native_first in [false, true] {
            let (mut transport, _) = fixture();
            let events = Arc::new(Closures::default());
            Arc::get_mut(&mut transport).unwrap().events = events.clone();
            if native_first {
                transport.announce_closed(Some(7));
            }
            transport.retire();
            transport.announce_closed(Some(7));
            transport.close();
            assert_eq!(*lock(&events.0), [native_first.then_some(7)]);
        }
    }

    #[test]
    fn programmatic_prompts_preserve_text_without_injecting_terminal_commands() {
        let text = "Line one\nLine two\tUnicode: λ";
        let bytes = prompt_bytes(text).unwrap();
        assert_eq!(&bytes[..6], b"\x1b[200~");
        assert_eq!(&bytes[6..bytes.len() - 7], text.as_bytes());
        assert_eq!(&bytes[bytes.len() - 7..], b"\x1b[201~\r");
        for text in [
            "bad\x00text",
            "\x1b[201~exit\r",
            "interrupt\x03",
            "\u{009b}2J",
        ] {
            assert!(prompt_bytes(text).is_err());
        }
    }

    #[test]
    fn pty_command_preserves_literal_arguments_cwd_and_explicit_environment() {
        let mut command = Command::new("native-cli");
        command
            .args(["--model", "model with spaces", "`literal $(text)`"])
            .current_dir("/work tree")
            .env("CLAUDE_EXPLICIT_MCP_TOKEN", "private-value")
            .env_remove("CODEX_THREAD_ID");
        let prepared = pty_command(&command);
        assert_eq!(
            prepared
                .get_argv()
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "native-cli",
                "--model",
                "model with spaces",
                "`literal $(text)`"
            ]
        );
        assert_eq!(prepared.get_cwd().unwrap(), "/work tree");
        assert_eq!(
            prepared.get_env("CLAUDE_EXPLICIT_MCP_TOKEN").unwrap(),
            "private-value"
        );
        assert!(prepared.get_env("CODEX_THREAD_ID").is_none());
        assert_eq!(prepared.get_env("TERM_PROGRAM").unwrap(), "Prometeu");
    }
}
