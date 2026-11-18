//! Process-scoped, read-only native observations. Hosts schedule polling and own all publication.
use crate::{
    claude,
    state::ProviderId,
    terminal_agent_hooks::{NativeHookEvent, NativeHookKind},
};
use prometeu_core::conversation::Clock;
use prometeu_protocols::terminal_transcript::{
    InlineImageFormat, JsonlTail, NativeTranscript, TailBatch, TranscriptBatch, TranscriptBinding,
    TranscriptFormat, TranscriptPhase, TRANSCRIPT_SNAPSHOT_LIMIT,
};
use serde_json::Value;
use std::{
    collections::VecDeque,
    ffi::CString,
    fs::{File, Metadata},
    io::{self, Read, Seek, SeekFrom},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

const PROMPT_COUNT_LIMIT: usize = 64;
const PROMPT_BYTES_LIMIT: usize = 1024 * 1024;
const PROJECTION_EVENT_LIMIT: usize = 65_536;
const APPEND_POLL_LIMIT: u64 = 4 * 1024 * 1024;

mod antigravity;
mod completion;
pub use antigravity::AntigravityObservationConfig;
mod cursor;
mod cursor_completion;
mod source;
pub use cursor::{
    CursorObservationConfig, NativeCapture, NativeIngestionBatch, NativeIngestionMode,
};
pub use cursor_completion::{CursorStopStamp, CursorStopWitness};
type CompletionState = Arc<Mutex<completion::CompletionObservation>>;
type SourceState = Arc<Mutex<source::SourceObservation>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationStart {
    /// The caller proved this is a new native session and constructs the observer before input.
    Fresh,
    /// Native history exists. The caller waits for baseline_ready before sending kickoff input.
    Resume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBaselineReason {
    Fresh,
    Resume,
    Rewrite,
}

/// Private provenance accompanies callbacks; it is never inserted into the public V1 envelope.
pub struct NativeObservation {
    pub revision: String,
    pub frame: Value,
}

/// This replaces only the host's native cache, never its append-only canonical transcript.
pub struct NativeBaseline {
    pub revision: String,
    pub reason: NativeBaselineReason,
    pub frames: Vec<Value>,
    pub complete: bool,
}

pub struct ObservationConfig {
    pub provider: ProviderId,
    pub session: String,
    pub process_nonce: String,
    pub expected_native_session: Option<String>,
    pub start: ObservationStart,
    /// Explicit native storage roots for the captured account. No root is inferred from HOME.
    /// A fresh session may not have created its root yet. Intentional profile-root symlinks are
    /// pinned when available; symlinks supplied below that root by a hook are never followed.
    pub authorized_roots: Vec<PathBuf>,
}

pub struct ObservationCallbacks {
    pub live: Box<dyn FnMut(NativeObservation) + Send>,
    pub native_baseline: Box<dyn FnMut(NativeBaseline) + Send>,
    pub warning: Box<dyn FnMut(&'static str) + Send>,
    /// Capture an already accepted native prompt. This callback must never write it to the PTY.
    pub accepted_prompt: Box<dyn FnMut(String) + Send>,
    /// Native facts remain available without assigning readiness, approval policy or completion.
    pub observed_hook: Box<dyn FnMut(NativeHookEvent) + Send>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationError {
    Identity,
    Path,
    Read,
    Unstable,
    Limit,
    Transcript,
}

impl ObservationError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Identity => "terminal.observation.identity",
            Self::Path => "terminal.observation.path",
            Self::Read => "terminal.observation.read",
            Self::Unstable => "terminal.observation.unstable",
            Self::Limit => "terminal.observation.limit",
            Self::Transcript => "terminal.observation.loss",
        }
    }
}

pub struct TerminalAgentObservation {
    config: ObservationConfig,
    roots: Vec<AuthorizedRoot>,
    callbacks: ObservationCallbacks,
    clock: Arc<dyn Clock>,
    native_session: Option<String>,
    transcript: Option<PathBuf>,
    reader: Option<ProjectionReader>,
    stamp: Option<FileStamp>,
    projection: Vec<Value>,
    projection_bytes: usize,
    // Separate acknowledgements allow a hook and the transcript to arrive in either order.
    injected_hooks: PromptQueue,
    transcript_echoes: PromptQueue,
    late_hooks: PromptQueue,
    last_warning: Option<&'static str>,
    replace_pending: bool,
    identity_failed: bool,
    content_failed: Option<ObservationError>,
    revision: String,
    baseline_reason: NativeBaselineReason,
    baseline_ready: bool,
    baseline_pending: bool,
    append_offset: u64,
    append_started: bool,
    append_replay_until: Option<u64>,
    partial: bool,
    last_poll_failed: bool,
    file_present: bool,
    cursor_store: Option<cursor::CursorObservation>,
    antigravity_store: Option<antigravity::AntigravityObservation>,
    completion: Option<CompletionState>,
    source: Option<SourceState>,
}

impl TerminalAgentObservation {
    pub fn new(
        config: ObservationConfig,
        mut callbacks: ObservationCallbacks,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ObservationError> {
        if !valid_id(&config.session)
            || !valid_id(&config.process_nonce)
            || config
                .expected_native_session
                .as_deref()
                .is_some_and(|id| !valid_native_id(id))
            || config.authorized_roots.is_empty()
            || config
                .authorized_roots
                .iter()
                .any(|root| !safe_absolute(root))
            || config.provider == ProviderId::RetiredGemini
        {
            return Err(ObservationError::Identity);
        }
        let roots = config
            .authorized_roots
            .iter()
            .map(|path| AuthorizedRoot::new(path.clone()))
            .collect::<Result<Vec<_>, ObservationError>>()?;
        let fresh = config.start == ObservationStart::Fresh;
        let revision = uuid::Uuid::new_v4().to_string();
        let baseline_reason = if fresh {
            NativeBaselineReason::Fresh
        } else {
            NativeBaselineReason::Resume
        };
        // Establish the empty baseline before the host can send a prompt. A fast first answer
        // may already be on disk when its first authenticated hook reaches the observer.
        if fresh {
            (callbacks.native_baseline)(NativeBaseline {
                revision: revision.clone(),
                reason: baseline_reason,
                frames: Vec::new(),
                complete: true,
            });
        }
        Ok(Self {
            native_session: config.expected_native_session.clone(),
            config,
            roots,
            callbacks,
            clock,
            transcript: None,
            reader: None,
            stamp: None,
            projection: Vec::new(),
            projection_bytes: 0,
            injected_hooks: PromptQueue::default(),
            transcript_echoes: PromptQueue::default(),
            late_hooks: PromptQueue::default(),
            last_warning: None,
            replace_pending: false,
            identity_failed: false,
            content_failed: None,
            revision,
            baseline_reason,
            baseline_ready: fresh,
            baseline_pending: !fresh,
            append_offset: 0,
            append_started: false,
            append_replay_until: None,
            partial: false,
            last_poll_failed: false,
            file_present: false,
            cursor_store: None,
            antigravity_store: None,
            completion: None,
            source: None,
        })
    }

    pub fn native_session(&self) -> Option<&str> {
        self.native_session.as_deref()
    }

    /// Enable only the persistence fence audited for this exact native build. Unsupported builds
    /// keep their ordinary observations but cannot produce this successful-completion proof.
    pub fn configure_completion(&mut self, version: &str) -> bool {
        if self.completion.is_some() || self.file_present {
            return false;
        }
        let Some(completion) =
            completion::CompletionObservation::new(&self.config, version, &self.revision)
        else {
            return false;
        };
        let state = Arc::new(Mutex::new(completion));
        if let Some(reader) = self.reader.as_mut() {
            reader.set_completion(state.clone());
        }
        self.completion = Some(state);
        true
    }

    pub fn observe_program_status(
        &mut self,
        report: &prometeu_protocols::terminal_status::ProgramStatusReport,
    ) -> Option<u64> {
        self.completion
            .as_ref()
            .and_then(|completion| crate::lock::lock(completion).program_status(report))
    }

    /// The host must combine this with its current execution ticket while input remains reserved.
    pub fn take_persisted_completion(
        &mut self,
    ) -> Option<(u64, crate::native_completion_fence::PersistedCompletion)> {
        self.completion
            .as_ref()
            .and_then(|completion| crate::lock::lock(completion).take())
    }

    /// Opt into original Cursor source identity before polling or accepting hooks. The caller
    /// captures the account/cwd storage root and explicitly supplies the supported CLI version.
    pub fn configure_cursor_store(
        &mut self,
        config: CursorObservationConfig,
    ) -> Result<(), ObservationError> {
        if self.config.provider != ProviderId::Cursor
            || self.cursor_store.is_some()
            || self.reader.is_some()
            || self.file_present
        {
            return self.fail(ObservationError::Identity);
        }
        self.cursor_store = Some(cursor::CursorObservation::new(config, self.config.start)?);
        Ok(())
    }

    pub fn configure_antigravity_store(
        &mut self,
        config: AntigravityObservationConfig,
    ) -> Result<(), ObservationError> {
        if self.config.provider != ProviderId::Antigravity
            || self.antigravity_store.is_some()
            || self.reader.is_some()
            || self.source.is_some()
            || self.file_present
        {
            return self.fail(ObservationError::Identity);
        }
        self.antigravity_store = Some(antigravity::AntigravityObservation::new(
            config,
            self.config.start,
        )?);
        Ok(())
    }

    pub fn set_ingestion(
        &mut self,
        callback: Box<dyn FnMut(NativeIngestionBatch) + Send>,
    ) -> Result<(), ObservationError> {
        if let Some(cursor) = self.cursor_store.as_mut() {
            cursor.set_callback(callback);
        } else if let Some(store) = self.antigravity_store.as_mut() {
            store.set_callback(callback);
        } else {
            if !matches!(
                self.config.provider,
                ProviderId::Claude | ProviderId::Antigravity
            ) || self.source.is_some()
                || self.file_present
            {
                return Err(ObservationError::Identity);
            }
            let mut source =
                source::SourceObservation::new(self.config.provider, self.config.start);
            source.set_callback(callback);
            let state = Arc::new(Mutex::new(source));
            if let Some(reader) = self.reader.as_mut() {
                reader.set_source(state.clone());
            }
            self.source = Some(state);
        }
        Ok(())
    }

    /// A batch may be delivered again until every content/checkpoint receipt is durably stored.
    /// This acknowledgment has no publication, readiness or user-input side effects.
    pub fn acknowledge_ingestion(&mut self, id: &str) -> Result<(), ObservationError> {
        if let Some(store) = self.antigravity_store.as_mut() {
            return store.acknowledge(id);
        }
        if let Some(source) = self.source.as_ref() {
            return crate::lock::lock(source).acknowledge(id);
        }
        self.cursor_store
            .as_mut()
            .ok_or(ObservationError::Identity)?
            .acknowledge(id)
    }

    pub fn expect_submission(
        &mut self,
        text: &str,
        submission_id: &str,
    ) -> Result<(), ObservationError> {
        if let Some(cursor) = self.cursor_store.as_mut() {
            cursor.expect_submission(text, submission_id)
        } else {
            if let Some(store) = self.antigravity_store.as_mut() {
                store.expect_submission(text, submission_id)?;
            }
            if let Some(source) = self.source.as_ref() {
                crate::lock::lock(source).expect_submission(text, submission_id)?;
            }
            self.expect_prompt(text)
        }
    }

    /// Restore only a previously authenticated private binding for this exact account/session.
    /// The host owns that persistence trust boundary. Poll opens it through the same secure path
    /// as a current hook; restoration does not fabricate a hook or establish TUI readiness.
    pub fn resume_from(&mut self, path: PathBuf) -> Result<(), ObservationError> {
        if self.config.start != ObservationStart::Resume
            || self.identity_failed
            || self.content_failed.is_some()
        {
            return self.fail(ObservationError::Identity);
        }
        let Some(native) = self.config.expected_native_session.as_deref() else {
            return self.fail(ObservationError::Identity);
        };
        if self.native_session.as_deref() != Some(native) {
            return self.fail(ObservationError::Identity);
        }
        if !self.allowed_path(&path, native) {
            return self.fail(ObservationError::Path);
        }
        if let Some(current) = &self.transcript {
            return if current == &path {
                Ok(())
            } else {
                self.fail(ObservationError::Identity)
            };
        }
        if self.cursor_store.is_none() && self.antigravity_store.is_none() {
            self.reader = Some(self.new_reader(native, &path)?);
        }
        self.transcript = Some(path);
        Ok(())
    }

    /// This is only a transcript bootstrap barrier, never evidence that the TUI accepts input.
    pub fn baseline_ready(&self) -> bool {
        if let Some(store) = &self.antigravity_store {
            return store.baseline_ready()
                && self.content_failed.is_none()
                && !self.identity_failed;
        }
        if let Some(source) = self.source.as_ref() {
            return crate::lock::lock(source).baseline_ready()
                && self.content_failed.is_none()
                && !self.identity_failed;
        }
        self.cursor_store.as_ref().map_or(
            self.baseline_ready,
            cursor::CursorObservation::baseline_ready,
        ) && self.content_failed.is_none()
            && !self.identity_failed
    }

    /// The last successful read consumed the known file through a complete JSONL record.
    /// A fresh empty baseline alone does not establish this EOF-draining condition.
    pub fn caught_up(&self) -> bool {
        if let Some(store) = &self.antigravity_store {
            return store.caught_up()
                && !self.last_poll_failed
                && self.content_failed.is_none()
                && !self.identity_failed;
        }
        if let Some(cursor) = &self.cursor_store {
            return cursor.caught_up()
                && !self.last_poll_failed
                && self.content_failed.is_none()
                && !self.identity_failed;
        }
        !self.last_poll_failed
            && self
                .source
                .as_ref()
                .is_none_or(|source| crate::lock::lock(source).caught_up())
            && self.file_present
            && self.content_failed.is_none()
            && !self.identity_failed
            && !self.partial
            && self.stamp.as_ref().is_some_and(|stamp| {
                !matches!(self.config.provider, ProviderId::Claude | ProviderId::Codex)
                    || self.append_offset >= stamp.len
            })
    }

    /// Register only after the host accepted an injected prompt and captured its canonical echo.
    pub fn expect_prompt(&mut self, text: &str) -> Result<(), ObservationError> {
        if self.cursor_store.is_some() {
            return self.fail(ObservationError::Identity);
        }
        if self.source.is_some() || self.antigravity_store.is_some() {
            if let Some(completion) = &self.completion {
                crate::lock::lock(completion).expect_submission(text);
            }
            return Ok(());
        }
        if (has_prompt_hooks(self.config.provider) && !self.injected_hooks.can_push(text))
            || !self.transcript_echoes.can_push(text)
        {
            return self.fail(ObservationError::Limit);
        }
        if has_prompt_hooks(self.config.provider) {
            self.injected_hooks.push(text);
        }
        self.transcript_echoes.push(text);
        if let Some(completion) = &self.completion {
            crate::lock::lock(completion).expect_submission(text);
        }
        Ok(())
    }

    pub fn accept_hook(&mut self, event: NativeHookEvent) -> Result<(), ObservationError> {
        if self.identity_failed
            || event.provider != self.config.provider
            || event.session != self.config.session
            || event.process_nonce != self.config.process_nonce
            || !valid_native_id(&event.native_session)
            || self
                .native_session
                .as_deref()
                .is_some_and(|id| id != event.native_session)
        {
            return self.fail(ObservationError::Identity);
        }
        // A subagent may share its parent's session id while supplying a different transcript.
        // Keep the observation without allowing that child to bind or publish root content.
        if event.agent_id.is_some() {
            (self.callbacks.observed_hook)(event);
            return Ok(());
        }
        if let Some(path) = event.transcript_path.as_deref() {
            if !self.allowed_path(path, &event.native_session) {
                return self.fail(ObservationError::Path);
            }
        }
        self.native_session
            .get_or_insert_with(|| event.native_session.clone());
        if let Some(path) = &event.transcript_path {
            if self.transcript.as_ref() != Some(path) {
                if self.cursor_store.is_some() || self.antigravity_store.is_some() {
                    self.transcript = Some(path.clone());
                } else {
                    self.replace_pending = self.transcript.is_some();
                    let mut reader = self.new_reader(&event.native_session, path)?;
                    if self.transcript.is_none() && self.config.start == ObservationStart::Fresh {
                        reader.snapshot(&[], self.clock.now())?;
                    }
                    self.reader = Some(reader);
                    self.transcript = Some(path.clone());
                    self.stamp = None;
                    self.append_offset = 0;
                    self.append_started = false;
                    self.append_replay_until = None;
                }
            }
        }
        if event.kind == NativeHookKind::PromptSubmitted
            && self.cursor_store.is_none()
            && self.source.is_none()
            && self.antigravity_store.is_none()
        {
            if let Some(text) = event.prompt.as_deref() {
                let injected =
                    self.injected_hooks.matches(text) && self.injected_hooks.consume(text);
                let already_read =
                    !injected && self.late_hooks.matches(text) && self.late_hooks.consume(text);
                if !injected && !already_read {
                    self.injected_hooks.clear();
                    self.late_hooks.clear();
                    if !self.transcript_echoes.can_push(text) {
                        return self.fail(ObservationError::Limit);
                    }
                    self.transcript_echoes.push(text);
                    (self.callbacks.accepted_prompt)(text.to_owned());
                }
            }
        }
        if let Some(completion) = &self.completion {
            crate::lock::lock(completion).hook(&event);
        }
        self.cursor_hook(&event);
        (self.callbacks.observed_hook)(event);
        Ok(())
    }

    /// Polling creates no thread, never writes a file and never follows other conversations.
    /// An absent fresh transcript is normal. Unstable snapshots are retried by the next host poll.
    pub fn poll(&mut self) -> Result<(), ObservationError> {
        let completion_poll = self
            .completion
            .as_ref()
            .map(|completion| crate::lock::lock(completion).start_poll());
        self.file_present = false;
        let result = self.poll_inner();
        self.last_poll_failed = result.is_err();
        if let Err(error) = result {
            if matches!(
                error,
                ObservationError::Identity | ObservationError::Limit | ObservationError::Transcript
            ) {
                self.content_failed = Some(error);
            }
            self.warn(error.code());
        }
        if let (Some(completion), Some(token)) = (&self.completion, completion_poll) {
            let mut completion = crate::lock::lock(completion);
            if self.content_failed.is_some() || self.identity_failed {
                completion.fail();
            }
            completion.finish_poll(
                token,
                result.is_ok() && self.caught_up(),
                self.baseline_ready(),
            );
        }
        result
    }

    fn poll_inner(&mut self) -> Result<(), ObservationError> {
        if let Some(error) = self.content_failed {
            return Err(error);
        }
        if self.identity_failed {
            return Err(ObservationError::Identity);
        }
        if let Some(store) = self.antigravity_store.as_mut() {
            return store.poll(
                self.native_session.as_deref(),
                self.clock.now(),
                self.completion.as_ref(),
            );
        }
        if let Some(cursor) = self.cursor_store.as_mut() {
            return cursor.poll(self.native_session.as_deref(), self.clock.now());
        }
        if let Some(source) = self.source.as_ref() {
            if crate::lock::lock(source).before_poll()? {
                return Ok(());
            }
        }
        let Some(path) = self.transcript.as_deref() else {
            return Ok(());
        };
        let Some(mut file) = open_authorized(&mut self.roots, path)? else {
            return Ok(());
        };
        let before = file.metadata().map_err(|_| ObservationError::Read)?;
        if !before.is_file() || before.nlink() != 1 || before.uid() != unsafe { libc::geteuid() } {
            return Err(ObservationError::Path);
        }
        self.file_present = true;
        let before = FileStamp::from(&before);
        let incremental = matches!(self.config.provider, ProviderId::Claude | ProviderId::Codex);
        if self.stamp.as_ref() == Some(&before)
            && (!incremental || self.append_offset >= before.len)
        {
            if let Some(source) = self.source.as_ref() {
                crate::lock::lock(source).unchanged();
            }
            return Ok(());
        }
        if incremental {
            return self.poll_append(file, before);
        }
        if before.len > TRANSCRIPT_SNAPSHOT_LIMIT as u64 {
            return Err(ObservationError::Limit);
        }
        let mut bytes = Vec::with_capacity(before.len as usize);
        file.by_ref()
            .take(before.len + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ObservationError::Read)?;
        let after = FileStamp::from(&file.metadata().map_err(|_| ObservationError::Read)?);
        if before != after || bytes.len() as u64 != before.len {
            return Err(ObservationError::Unstable);
        }
        let rotated = self
            .stamp
            .as_ref()
            .is_some_and(|old| old.dev != after.dev || old.ino != after.ino);
        if rotated {
            self.reader = Some(
                self.new_reader(
                    self.native_session
                        .as_deref()
                        .ok_or(ObservationError::Identity)?,
                    path,
                )?,
            );
            self.replace_pending = true;
        }
        if let Some(completion) = &self.completion {
            crate::lock::lock(completion).refresh_snapshot();
        }
        let result = self
            .reader
            .as_mut()
            .ok_or(ObservationError::Identity)?
            .snapshot(&bytes, self.clock.now());
        let mut batch = match result {
            Ok(batch) => batch,
            Err(ObservationError::Identity) => {
                self.identity_failed = true;
                return Err(ObservationError::Identity);
            }
            Err(error) => return Err(error),
        };
        batch.replace |= self.replace_pending;
        self.apply(batch)?;
        self.stamp = Some(after);
        self.replace_pending = false;
        Ok(())
    }

    fn poll_append(&mut self, mut file: File, before: FileStamp) -> Result<(), ObservationError> {
        let replaced = self.append_offset > before.len
            || self.append_replay_until.is_some_and(|end| end > before.len)
            || self.stamp.as_ref().is_some_and(|old| {
                old.dev != before.dev
                    || old.ino != before.ino
                    || before.len < old.len
                    || (before.len == old.len && old != &before)
            });
        if replaced {
            self.reader = Some(
                self.new_reader(
                    self.native_session
                        .as_deref()
                        .ok_or(ObservationError::Identity)?,
                    self.transcript
                        .as_deref()
                        .ok_or(ObservationError::Identity)?,
                )?,
            );
            self.append_offset = 0;
            self.append_started = false;
            self.append_replay_until = None;
            self.replace_pending = true;
        }
        if !self.append_started {
            if self.config.start == ObservationStart::Resume || self.replace_pending {
                self.append_replay_until = Some(before.len);
            }
            self.append_started = true;
        }
        let phase = if self.append_replay_until.is_some() {
            TranscriptPhase::Replay
        } else {
            TranscriptPhase::Live
        };
        let end = self.append_replay_until.unwrap_or(before.len);
        let count = end
            .saturating_sub(self.append_offset)
            .min(APPEND_POLL_LIMIT);
        file.seek(SeekFrom::Start(self.append_offset))
            .map_err(|_| ObservationError::Read)?;
        let mut bytes = Vec::with_capacity(count as usize);
        file.by_ref()
            .take(count)
            .read_to_end(&mut bytes)
            .map_err(|_| ObservationError::Read)?;
        let after = FileStamp::from(&file.metadata().map_err(|_| ObservationError::Read)?);
        if before != after || bytes.len() as u64 != count {
            return Err(ObservationError::Unstable);
        }
        let mut batch = self
            .reader
            .as_mut()
            .ok_or(ObservationError::Identity)?
            .append(self.append_offset, &bytes, phase, self.clock.now())?;
        let next = self.append_offset + count;
        if self.append_replay_until.is_some_and(|end| next >= end) {
            self.append_replay_until = None;
        }
        // A complete JSON record is not necessarily a complete historical bootstrap. The host
        // must not send kickoff input between bounded reads of an existing append-only file.
        batch.partial |= self.append_replay_until.is_some();
        batch.replace |= self.replace_pending;
        self.apply(batch)?;
        self.append_offset = next;
        self.stamp = Some(after);
        self.replace_pending = false;
        Ok(())
    }

    fn apply(&mut self, mut batch: TranscriptBatch) -> Result<(), ObservationError> {
        self.partial = batch.partial;
        if batch.replace {
            self.revision = uuid::Uuid::new_v4().to_string();
            if let Some(completion) = &self.completion {
                crate::lock::lock(completion).source_replaced(&self.revision);
            }
            self.baseline_reason = NativeBaselineReason::Rewrite;
            self.baseline_ready = false;
            self.baseline_pending = true;
            self.projection.clear();
            self.projection_bytes = 0;
            self.injected_hooks.clear();
            self.transcript_echoes.clear();
            self.late_hooks.clear();
            self.warn("terminal.observation.rewrite");
        }
        if batch.loss {
            if let Some(completion) = &self.completion {
                crate::lock::lock(completion).fail();
            }
            self.warn(ObservationError::Transcript.code());
        }
        if let Some(source) = self.source.as_ref() {
            crate::lock::lock(source).finish_poll(
                batch.partial,
                batch.loss,
                &self.revision,
                batch.replace,
            )?;
            return Ok(());
        }
        let session = self
            .native_session
            .as_deref()
            .ok_or(ObservationError::Identity)?;
        for frame in batch.replay.iter_mut().chain(batch.live.iter_mut()) {
            scope_synthetic_ids(frame, session, &self.revision);
        }
        let replay = self.baseline_pending || !batch.replay.is_empty();
        let historical = batch
            .replay
            .iter()
            .filter_map(public_projection)
            .collect::<Vec<_>>();
        let mut staged = historical;
        let historical_count = staged.len();
        staged.extend(batch.live.iter().filter_map(public_projection));
        let added = staged
            .iter()
            .map(|frame| frame.to_string().len() + 1)
            .sum::<usize>();
        if staged.len() > PROJECTION_EVENT_LIMIT.saturating_sub(self.projection.len())
            || added > TRANSCRIPT_SNAPSHOT_LIMIT.saturating_sub(self.projection_bytes)
        {
            return Err(ObservationError::Limit);
        }
        self.projection_bytes += added;
        let current = staged.split_off(historical_count);
        self.projection.extend(staged);
        if replay {
            let complete = !batch.partial && !batch.loss;
            self.baseline_ready = complete;
            self.baseline_pending = !complete;
            (self.callbacks.native_baseline)(NativeBaseline {
                revision: self.revision.clone(),
                reason: self.baseline_reason,
                frames: self.projection.clone(),
                complete,
            });
        }
        self.projection.extend(current);
        for frame in batch.live {
            if let Some(text) = user_text(&frame) {
                if !self.transcript_echoes.consume(text) {
                    if has_prompt_hooks(self.config.provider) {
                        if !self.late_hooks.can_push(text) {
                            return self.fail(ObservationError::Limit);
                        }
                        self.late_hooks.push(text);
                    }
                    (self.callbacks.accepted_prompt)(text.to_owned());
                }
            } else {
                (self.callbacks.live)(NativeObservation {
                    revision: self.revision.clone(),
                    frame,
                });
            }
        }
        Ok(())
    }

    fn allowed_path(&self, path: &Path, session: &str) -> bool {
        safe_absolute(path)
            && self.config.authorized_roots.iter().any(|root| {
                path.strip_prefix(root)
                    .is_ok_and(|relative| !relative.as_os_str().is_empty())
            })
            && session_path(self.config.provider, path, session)
    }

    fn new_reader(&self, native: &str, path: &Path) -> Result<ProjectionReader, ObservationError> {
        let mut reader = ProjectionReader::new(self.config.provider, native, path)?;
        if let Some(completion) = &self.completion {
            reader.set_completion(completion.clone());
        }
        if let Some(source) = &self.source {
            reader.set_source(source.clone());
        }
        Ok(reader)
    }

    fn warn(&mut self, code: &'static str) {
        if self.last_warning != Some(code) {
            self.last_warning = Some(code);
            (self.callbacks.warning)(code);
        }
    }

    fn fail<T>(&mut self, error: ObservationError) -> Result<T, ObservationError> {
        if matches!(
            error,
            ObservationError::Identity
                | ObservationError::Path
                | ObservationError::Limit
                | ObservationError::Transcript
        ) {
            if let Some(completion) = &self.completion {
                crate::lock::lock(completion).fail();
            }
        }
        self.warn(error.code());
        Err(error)
    }
}

enum ProjectionReader {
    Claude {
        native: String,
        adapter: Box<claude::Adapter>,
        tail: Box<JsonlTail>,
        completion: Option<CompletionState>,
        source: Option<SourceState>,
    },
    Native(Box<NativeTranscript>),
}

impl ProjectionReader {
    fn new(provider: ProviderId, native: &str, path: &Path) -> Result<Self, ObservationError> {
        let format = match provider {
            ProviderId::Claude => {
                return Ok(Self::Claude {
                    native: native.into(),
                    adapter: Box::default(),
                    tail: Box::new(JsonlTail::with_inline_images(InlineImageFormat::Claude)),
                    completion: None,
                    source: None,
                })
            }
            ProviderId::Codex => TranscriptFormat::Codex,
            ProviderId::Antigravity => TranscriptFormat::Antigravity,
            ProviderId::Cursor => TranscriptFormat::Cursor,
            ProviderId::RetiredGemini => return Err(ObservationError::Identity),
        };
        NativeTranscript::new(TranscriptBinding {
            format,
            native_session: native.into(),
            transcript_path: path.into(),
        })
        .map(|reader| Self::Native(Box::new(reader)))
        .map_err(|_| ObservationError::Identity)
    }

    fn set_completion(&mut self, state: CompletionState) {
        match self {
            Self::Claude { completion, .. } => *completion = Some(state),
            Self::Native(reader) => reader.set_record_observer(Box::new(move |record| {
                crate::lock::lock(&state).record(record);
            })),
        }
    }

    fn set_source(&mut self, state: SourceState) {
        match self {
            Self::Claude { source, .. } => *source = Some(state),
            Self::Native(reader) => {
                let native = reader.binding().native_session.clone();
                reader.set_projection_observer(Box::new(move |record, frames| {
                    crate::lock::lock(&state).observe(&native, record, frames);
                }));
            }
        }
    }

    fn snapshot(&mut self, input: &[u8], at: u64) -> Result<TranscriptBatch, ObservationError> {
        match self {
            Self::Native(reader) => reader
                .snapshot(input, TranscriptPhase::Live, at)
                .map_err(transcript_error),
            Self::Claude { .. } => self.append(0, input, TranscriptPhase::Replay, at),
        }
    }

    fn append(
        &mut self,
        offset: u64,
        input: &[u8],
        phase: TranscriptPhase,
        at: u64,
    ) -> Result<TranscriptBatch, ObservationError> {
        match self {
            Self::Native(reader) => reader
                .append(offset, input, phase, at)
                .map_err(transcript_error),
            Self::Claude {
                native,
                adapter,
                tail,
                completion,
                source,
            } => {
                let batch = tail.feed(offset, input, phase).map_err(transcript_error)?;
                claude_batch(
                    native,
                    adapter,
                    batch,
                    at,
                    completion.as_ref(),
                    source.as_ref(),
                )
            }
        }
    }
}

fn transcript_error(
    error: prometeu_protocols::terminal_transcript::TranscriptError,
) -> ObservationError {
    match error {
        prometeu_protocols::terminal_transcript::TranscriptError::IdentityMismatch => {
            ObservationError::Identity
        }
        prometeu_protocols::terminal_transcript::TranscriptError::SizeLimit => {
            ObservationError::Limit
        }
        _ => ObservationError::Transcript,
    }
}

fn claude_batch(
    native: &str,
    adapter: &mut claude::Adapter,
    batch: TailBatch,
    at: u64,
    completion: Option<&CompletionState>,
    source: Option<&SourceState>,
) -> Result<TranscriptBatch, ObservationError> {
    let mut output = TranscriptBatch {
        partial: batch.partial,
        loss: batch.loss,
        ..Default::default()
    };
    for record in batch.records {
        if record.value["sessionId"]
            .as_str()
            .is_some_and(|id| id != native)
        {
            return Err(ObservationError::Identity);
        }
        // Native transcripts cannot inject pre-canonicalized application events.
        if record.value.get("v").is_some() {
            continue;
        }
        if let Some(completion) = completion {
            crate::lock::lock(completion).record(&record);
        }
        let mut value = record.value.clone();
        if record.omitted_media != 0 {
            // The legacy adapter prints a placeholder for every image item. Its bytes are
            // intentionally unavailable here; retain the person's exact text for prompt binding
            // instead of presenting that placeholder as another accepted part of their input.
            if let Some(content) = value["message"]["content"].as_array_mut() {
                content
                    .retain(|part| !(part["type"] == "image" && part["source"]["data"].is_null()));
            }
        }
        value["ts"] = serde_json::json!(at);
        let frames = adapter.translate(&value);
        if let Some(source) = source {
            crate::lock::lock(source).observe(native, &record, &frames);
        }
        for frame in frames {
            if frame["type"] == "turn.completed" && (output.partial || output.loss) {
                continue;
            }
            if record.phase == TranscriptPhase::Replay {
                output.replay.push(frame);
            } else {
                output.live.push(frame);
            }
        }
    }
    Ok(output)
}

#[derive(Default)]
struct PromptQueue {
    values: VecDeque<String>,
    bytes: usize,
}
impl PromptQueue {
    fn matches(&self, text: &str) -> bool {
        self.values.front().is_some_and(|expected| expected == text)
    }
    fn can_push(&self, text: &str) -> bool {
        self.values.len() < PROMPT_COUNT_LIMIT
            && text.len() <= PROMPT_BYTES_LIMIT.saturating_sub(self.bytes)
    }
    fn push(&mut self, text: &str) {
        self.bytes += text.len();
        self.values.push_back(text.into());
    }
    fn consume(&mut self, text: &str) -> bool {
        let Some(expected) = self.values.pop_front() else {
            return false;
        };
        self.bytes -= expected.len();
        if expected == text {
            true
        } else {
            self.clear();
            false
        }
    }
    fn clear(&mut self) {
        self.values.clear();
        self.bytes = 0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    modified: i64,
    modified_ns: i64,
    changed: i64,
    changed_ns: i64,
}
impl From<&Metadata> for FileStamp {
    fn from(value: &Metadata) -> Self {
        Self {
            dev: value.dev(),
            ino: value.ino(),
            len: value.len(),
            modified: value.mtime(),
            modified_ns: value.mtime_nsec(),
            changed: value.ctime(),
            changed_ns: value.ctime_nsec(),
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
fn has_prompt_hooks(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Claude | ProviderId::Codex)
}
fn valid_native_id(value: &str) -> bool {
    valid_id(value) && !value.contains(['/', '\\']) && value != "." && value != ".."
}
fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().as_bytes().len() <= 4096
        && !path.as_os_str().as_bytes().contains(&0)
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn session_path(provider: ProviderId, path: &Path, session: &str) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    match provider {
        ProviderId::Claude | ProviderId::Cursor => name == format!("{session}.jsonl"),
        ProviderId::Codex => {
            name == format!("{session}.jsonl")
                || (name.starts_with("rollout-") && name.ends_with(&format!("-{session}.jsonl")))
        }
        ProviderId::Antigravity => ["transcript.jsonl", "transcript_full.jsonl"]
            .into_iter()
            .any(|file| {
                path.ends_with(Path::new(session).join(".system_generated/logs").join(file))
            }),
        ProviderId::RetiredGemini => false,
    }
}

pub(crate) struct AuthorizedRoot {
    path: PathBuf,
    directory: Option<File>,
}

impl AuthorizedRoot {
    pub(crate) fn new(path: PathBuf) -> Result<Self, ObservationError> {
        if !safe_absolute(&path) {
            return Err(ObservationError::Path);
        }
        let directory = pin_root(&path)?;
        Ok(Self { path, directory })
    }
}

fn pin_root(path: &Path) -> Result<Option<File>, ObservationError> {
    let canonical = match std::fs::canonicalize(path) {
        Ok(root) => root,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ObservationError::Path),
    };
    let fd = unsafe {
        libc::open(
            c"/".as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return open_failure();
    }
    let mut current = unsafe { File::from_raw_fd(fd) };
    // Canonicalization authorizes intentional profile-root symlinks once. Resolve its result
    // without following any newly introduced ancestor symlink, then retain that directory fd.
    for part in canonical.components() {
        let Component::Normal(name) = part else {
            continue;
        };
        let name = CString::new(name.as_bytes()).map_err(|_| ObservationError::Path)?;
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return open_failure();
        }
        current = unsafe { File::from_raw_fd(fd) };
    }
    Ok(Some(current))
}

pub(crate) fn open_authorized(
    roots: &mut [AuthorizedRoot],
    path: &Path,
) -> Result<Option<File>, ObservationError> {
    for root in roots {
        let Ok(relative) = path.strip_prefix(&root.path) else {
            continue;
        };
        if root.directory.is_none() {
            root.directory = pin_root(&root.path)?;
        }
        let Some(directory) = &root.directory else {
            continue;
        };
        let mut current: Option<File> = None;
        let parts = relative.components().collect::<Vec<_>>();
        if parts.is_empty() {
            return Err(ObservationError::Path);
        }
        for (index, part) in parts.iter().enumerate() {
            let Component::Normal(name) = part else {
                return Err(ObservationError::Path);
            };
            let name = CString::new(name.as_bytes()).map_err(|_| ObservationError::Path)?;
            let flags = libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if index + 1 < parts.len() {
                    libc::O_DIRECTORY
                } else {
                    0
                };
            let parent = current.as_ref().unwrap_or(directory).as_raw_fd();
            let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
            if fd < 0 {
                return open_failure();
            }
            current = Some(unsafe { File::from_raw_fd(fd) });
        }
        return Ok(current);
    }
    Ok(None)
}

fn open_failure() -> Result<Option<File>, ObservationError> {
    match io::Error::last_os_error() {
        error if error.kind() == io::ErrorKind::NotFound => Ok(None),
        error if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
            Err(ObservationError::Path)
        }
        _ => Err(ObservationError::Read),
    }
}

fn public_projection(frame: &Value) -> Option<Value> {
    if frame["type"] == "telemetry.usage" {
        return None;
    }
    let mut frame = frame.clone();
    if let Some(fields) = frame.as_object_mut() {
        fields.remove("telemetry");
        fields.remove("providerDurationMs");
    }
    Some(frame)
}

fn scope_synthetic_ids(frame: &mut Value, session: &str, revision: &str) {
    let prefix = format!("native:{session}:");
    let scoped = |value: &mut Value| {
        if let Some(suffix) = value.as_str().and_then(|id| id.strip_prefix(&prefix)) {
            *value = Value::String(format!("native:{revision}:{suffix}"));
        }
    };
    if let Some(id) = frame.get_mut("messageId") {
        scoped(id);
    }
    if let Some(id) = frame.get_mut("toolId") {
        scoped(id);
    }
    if let Some(id) = frame.get_mut("block").and_then(|block| block.get_mut("id")) {
        scoped(id);
    }
}

fn user_text(frame: &Value) -> Option<&str> {
    if frame["type"] != "user.message" {
        return None;
    }
    let content = frame["content"].as_array()?;
    if content.len() != 1 || content[0]["kind"] != "text" {
        return None;
    }
    content[0]["text"].as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs::OpenOptions, io::Write, os::unix::fs::symlink, sync::Mutex};

    #[derive(Default)]
    struct Seen {
        live: Vec<Value>,
        live_revisions: Vec<String>,
        replay: Vec<Vec<Value>>,
        baselines: Vec<(String, NativeBaselineReason, bool)>,
        prompts: Vec<String>,
        hooks: Vec<NativeHookKind>,
        warnings: Vec<&'static str>,
    }
    struct TestClock;
    impl Clock for TestClock {
        fn now(&self) -> u64 {
            42
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("prometeu-observation-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self) -> PathBuf {
            self.0.join("native-session.jsonl")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn observer(
        provider: ProviderId,
        root: PathBuf,
    ) -> (TerminalAgentObservation, Arc<Mutex<Seen>>) {
        observer_start(provider, root, ObservationStart::Resume)
    }

    fn observer_start(
        provider: ProviderId,
        root: PathBuf,
        start: ObservationStart,
    ) -> (TerminalAgentObservation, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (live, replay, prompt, hook, warning) = (
            seen.clone(),
            seen.clone(),
            seen.clone(),
            seen.clone(),
            seen.clone(),
        );
        let observer = TerminalAgentObservation::new(
            ObservationConfig {
                provider,
                session: "logical-tab".into(),
                process_nonce: "incarnation-1".into(),
                expected_native_session: Some("native-session".into()),
                start,
                authorized_roots: vec![root],
            },
            ObservationCallbacks {
                live: Box::new(move |observation| {
                    let mut seen = live.lock().unwrap();
                    seen.live.push(observation.frame);
                    seen.live_revisions.push(observation.revision);
                }),
                native_baseline: Box::new(move |baseline| {
                    let mut seen = replay.lock().unwrap();
                    seen.replay.push(baseline.frames);
                    seen.baselines
                        .push((baseline.revision, baseline.reason, baseline.complete));
                }),
                accepted_prompt: Box::new(move |text| prompt.lock().unwrap().prompts.push(text)),
                observed_hook: Box::new(move |event| hook.lock().unwrap().hooks.push(event.kind)),
                warning: Box::new(move |code| warning.lock().unwrap().warnings.push(code)),
            },
            Arc::new(TestClock),
        )
        .unwrap();
        (observer, seen)
    }

    fn hook(
        provider: ProviderId,
        kind: NativeHookKind,
        path: &Path,
        prompt: Option<&str>,
    ) -> NativeHookEvent {
        NativeHookEvent {
            provider,
            session: "logical-tab".into(),
            process_nonce: "incarnation-1".into(),
            native_session: "native-session".into(),
            kind,
            transcript_path: Some(path.into()),
            prompt: prompt.map(str::to_owned),
            tool_name: None,
            tool_use_id: None,
            tool_input: None,
            tool_response: None,
            text: None,
            generation_id: None,
            agent_id: None,
            source: None,
            reason: None,
            notification_type: None,
            stop_hook_active: None,
            error: None,
            step_index: None,
            initial_num_steps: None,
            invocation_number: None,
            execution_number: None,
            fully_idle: None,
            continuation_active: None,
            cursor_stop_stamp: None,
        }
    }
    fn claude_user(text: &str) -> Value {
        json!({"type":"user","sessionId":"native-session","message":{"role":"user","content":text}})
    }
    fn claude_answer(id: &str, text: &str) -> Value {
        json!({"type":"assistant","sessionId":"native-session","message":{"id":id,"role":"assistant","content":[{"type":"text","text":text}]}})
    }
    fn cursor(role: &str, text: &str) -> Value {
        json!({"role":role,"message":{"content":[{"type":"text","text":text}]}})
    }
    fn lines(values: &[Value]) -> String {
        values.iter().map(|value| format!("{value}\n")).collect()
    }
    fn append(path: &Path, values: &[Value]) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(lines(values).as_bytes())
            .unwrap();
    }

    fn completion_status(
        state: prometeu_protocols::terminal_status::ProgramStatusState,
    ) -> prometeu_protocols::terminal_status::ProgramStatusReport {
        prometeu_protocols::terminal_status::ProgramStatusReport {
            id: None,
            state,
            kind: None,
            message: None,
            app: None,
            title: None,
            progress: None,
        }
    }

    fn completion_user(id: &str, text: &str) -> Value {
        let mut row = claude_user(text);
        row["uuid"] = json!(id);
        row
    }

    fn completion_answer(id: &str, text: &str) -> Value {
        let mut row = claude_answer(id, text);
        row["uuid"] = json!(format!("uuid-{id}"));
        row["message"]["stop_reason"] = json!("end_turn");
        row
    }

    #[test]
    fn terminal_completion_waits_for_current_raw_user_boundary_and_exact_done_report() {
        use prometeu_protocols::terminal_status::ProgramStatusState::{Done, Working};
        let fixture = Fixture::new();
        std::fs::write(
            fixture.file(),
            lines(&[
                completion_user("old-user", "Repeat"),
                completion_answer("old-answer", "Same response"),
            ]),
        )
        .unwrap();
        let (mut observer, _) = observer(ProviderId::Claude, fixture.0.clone());
        assert!(observer.configure_completion("2.1.326 (Claude Code)"));
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        observer
            .expect_submission("Repeat", "submission-new")
            .unwrap();
        // This is an earlier response that was not yet read at the time of the new submission.
        append(
            &fixture.file(),
            &[completion_answer("unread-old-answer", "Same response")],
        );
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Repeat"),
            ))
            .unwrap();
        observer.observe_program_status(&completion_status(Working));
        let mut stopped = hook(
            ProviderId::Claude,
            NativeHookKind::StopObserved,
            &fixture.file(),
            None,
        );
        stopped.text = Some("Same response".into());
        observer.accept_hook(stopped).unwrap();
        let done = observer
            .observe_program_status(&completion_status(Done))
            .unwrap();
        // A repeated root Done does not create another coordinator ticket.
        assert!(
            observer
                .observe_program_status(&completion_status(Done))
                .unwrap()
                > done
        );
        observer.poll().unwrap();
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_none());
        append(
            &fixture.file(),
            &[
                completion_user("new-user", "Repeat"),
                completion_answer("new-answer", "Same response"),
            ],
        );
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_none());
        observer.poll().unwrap();
        let (report, proof) = observer.take_persisted_completion().unwrap();
        assert_eq!(report, done);
        assert_eq!(proof.native_source, "uuid-new-answer");
        assert!(observer.take_persisted_completion().is_none());
    }

    #[test]
    fn terminal_completion_new_input_invalidates_a_persisted_old_ticket() {
        use prometeu_protocols::terminal_status::ProgramStatusState::{Done, Working};
        let fixture = Fixture::new();
        std::fs::write(fixture.file(), "").unwrap();
        let (mut observer, _) = observer_start(
            ProviderId::Claude,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        assert!(observer.configure_completion("2.1.326 (Claude Code)"));
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.expect_submission("First", "first").unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("First"),
            ))
            .unwrap();
        observer.observe_program_status(&completion_status(Working));
        let mut stopped = hook(
            ProviderId::Claude,
            NativeHookKind::StopObserved,
            &fixture.file(),
            None,
        );
        stopped.text = Some("Same response".into());
        observer.accept_hook(stopped).unwrap();
        observer.observe_program_status(&completion_status(Done));
        append(
            &fixture.file(),
            &[
                completion_user("first-user", "First"),
                completion_answer("first-answer", "Same response"),
            ],
        );
        observer.poll().unwrap();
        observer.poll().unwrap();
        observer.expect_submission("Second", "second").unwrap();
        assert!(observer.take_persisted_completion().is_none());
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_none());
    }

    #[test]
    fn terminal_completion_antigravity_waits_for_final_step_rewrite_after_native_stop() {
        use prometeu_protocols::terminal_status::ProgramStatusState::{Done, Working};
        let fixture = Fixture::new();
        let path = fixture
            .0
            .join("native-session/.system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let user = json!({"step_index":5,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"Repeat"});
        let mut answer = json!({"step_index":6,"source":"MODEL","type":"PLANNER_RESPONSE","status":"RUNNING","content":"Same response","tool_calls":[],"truncated_fields":[]});
        std::fs::write(&path, lines(&[user.clone(), answer.clone()])).unwrap();
        let (mut observer, _) = observer_start(
            ProviderId::Antigravity,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        assert!(observer.configure_completion("1.5.10"));
        observer
            .expect_submission("Repeat", "native-input")
            .unwrap();
        let mut invocation = hook(
            ProviderId::Antigravity,
            NativeHookKind::ModelInvocationStarted,
            &path,
            Some("Repeat"),
        );
        invocation.initial_num_steps = Some(6);
        invocation.invocation_number = Some(0);
        observer.accept_hook(invocation).unwrap();
        observer.observe_program_status(&completion_status(Working));
        let mut stopped = hook(
            ProviderId::Antigravity,
            NativeHookKind::StopObserved,
            &path,
            Some("Repeat"),
        );
        stopped.text = Some("Same response".into());
        stopped.reason = Some("NO_TOOL_CALL".into());
        stopped.fully_idle = Some(true);
        stopped.execution_number = Some(0);
        observer.accept_hook(stopped).unwrap();
        let done = observer
            .observe_program_status(&completion_status(Done))
            .unwrap();
        observer.poll().unwrap();
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_none());
        answer["status"] = json!("DONE");
        std::fs::write(&path, lines(&[user, answer])).unwrap();
        observer.poll().unwrap();
        let (report, proof) = observer.take_persisted_completion().unwrap();
        assert_eq!(report, done);
        assert_eq!(proof.native_source, "step:6");
    }

    #[test]
    fn terminal_observation_antigravity_accepts_only_native_transcript_names_for_exact_session() {
        for file in ["transcript.jsonl", "transcript_full.jsonl"] {
            assert!(session_path(
                ProviderId::Antigravity,
                &Path::new("/native/brain/session")
                    .join(".system_generated/logs")
                    .join(file),
                "session",
            ));
        }
        for path in [
            "/native/brain/other/.system_generated/logs/transcript_full.jsonl",
            "/native/brain/session/transcript_full.jsonl",
            "/native/brain/session/.system_generated/logs/unrelated.jsonl",
            "/native/brain/session/.system_generated/logs/transcript_full.jsonl.extra",
        ] {
            assert!(!session_path(
                ProviderId::Antigravity,
                Path::new(path),
                "session"
            ));
        }
    }

    #[test]
    fn terminal_completion_antigravity_binds_rendered_user_to_exact_hook_step_in_either_order() {
        use prometeu_protocols::terminal_status::ProgramStatusState::{Done, Working};
        for hook_first in [false, true] {
            let fixture = Fixture::new();
            let path = fixture
                .0
                .join("native-session/.system_generated/logs/transcript_full.jsonl");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let user = json!({"step_index":5,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nRepeat\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nSynthetic native context.\n</ADDITIONAL_METADATA>"});
            let answer = json!({"step_index":6,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Same response"});
            std::fs::write(&path, lines(&[user, answer])).unwrap();
            let (mut observer, _) = observer_start(
                ProviderId::Antigravity,
                fixture.0.clone(),
                ObservationStart::Fresh,
            );
            assert!(observer.configure_completion("1.5.10"));
            observer.expect_submission("Repeat", "input").unwrap();
            observer
                .accept_hook(hook(
                    ProviderId::Antigravity,
                    NativeHookKind::SessionStarted,
                    &path,
                    Some("Repeat"),
                ))
                .unwrap();
            observer.observe_program_status(&completion_status(Working));
            let mut invocation = hook(
                ProviderId::Antigravity,
                NativeHookKind::ModelInvocationStarted,
                &path,
                Some("Repeat"),
            );
            invocation.initial_num_steps = Some(6);
            if !hook_first {
                observer.poll().unwrap();
                assert!(observer.take_persisted_completion().is_none());
            }
            observer.accept_hook(invocation).unwrap();
            let mut stopped = hook(
                ProviderId::Antigravity,
                NativeHookKind::StopObserved,
                &path,
                Some("Repeat"),
            );
            stopped.reason = Some("NO_TOOL_CALL".into());
            stopped.execution_number = Some(0);
            stopped.fully_idle = Some(true);
            stopped.text = None;
            observer.accept_hook(stopped).unwrap();
            let done = observer
                .observe_program_status(&completion_status(Done))
                .unwrap();
            observer.poll().unwrap();
            observer.poll().unwrap();
            let (ordinal, proof) = observer.take_persisted_completion().unwrap();
            assert_eq!(ordinal, done);
            assert_eq!(proof.native_source, "step:6");
        }
    }

    #[test]
    fn native_source_observer_preserves_live_rewrite_and_cold_resume_with_stable_step_receipts() {
        let fixture = Fixture::new();
        let path = fixture
            .0
            .join("native-session/.system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let user = json!({"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nPrompt\n</USER_REQUEST>"});
        let mut answer = json!({"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"RUNNING","content":"Partial"});
        std::fs::write(&path, lines(&[user.clone(), answer.clone()])).unwrap();
        let batches = Arc::new(Mutex::new(Vec::new()));
        let output = batches.clone();
        let (mut observer, legacy) = observer_start(
            ProviderId::Antigravity,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        observer
            .set_ingestion(Box::new(move |batch| output.lock().unwrap().push(batch)))
            .unwrap();
        observer.expect_submission("Prompt", "submission").unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Antigravity,
                NativeHookKind::SessionStarted,
                &path,
                Some("Prompt"),
            ))
            .unwrap();
        observer.poll().unwrap();
        let first = batches.lock().unwrap().pop().unwrap();
        assert_eq!(first.phase, TranscriptPhase::Live);
        let first_answer = first
            .records
            .iter()
            .find(|row| row.checkpoint["kind"] == "text")
            .unwrap()
            .clone();
        assert!(!observer.caught_up());
        assert!(legacy.lock().unwrap().live.is_empty());
        observer.acknowledge_ingestion(&first.id).unwrap();
        observer.poll().unwrap();
        assert!(observer.caught_up());
        answer["status"] = json!("DONE");
        answer["content"] = json!("Final");
        std::fs::write(&path, lines(&[user, answer])).unwrap();
        observer.poll().unwrap();
        let final_batch = batches.lock().unwrap().pop().unwrap();
        let final_answer = final_batch
            .records
            .iter()
            .find(|row| row.checkpoint["kind"] == "text")
            .unwrap()
            .clone();
        assert_eq!(final_batch.mode, NativeIngestionMode::Snapshot);
        assert_eq!(final_batch.phase, TranscriptPhase::Live);
        assert_eq!(final_answer.occurrence, first_answer.occurrence);
        assert_ne!(final_answer.version, first_answer.version);
        assert_eq!(
            final_answer.frame.as_ref().unwrap()["block"]["text"],
            "Final"
        );
        observer.acknowledge_ingestion(&final_batch.id).unwrap();
        let output = batches.clone();
        let (mut resumed, _) = observer_start(
            ProviderId::Antigravity,
            fixture.0.clone(),
            ObservationStart::Resume,
        );
        resumed
            .set_ingestion(Box::new(move |batch| output.lock().unwrap().push(batch)))
            .unwrap();
        resumed.resume_from(path).unwrap();
        resumed.poll().unwrap();
        assert!(!resumed.baseline_ready());
        let replay = batches.lock().unwrap().pop().unwrap();
        assert_eq!(replay.phase, TranscriptPhase::Replay);
        let replayed_answer = replay
            .records
            .iter()
            .find(|row| row.checkpoint["kind"] == "text")
            .unwrap();
        assert_eq!(replayed_answer.occurrence, final_answer.occurrence);
        assert_eq!(replayed_answer.version, final_answer.version);
        resumed.acknowledge_ingestion(&replay.id).unwrap();
        resumed.poll().unwrap();
        assert!(resumed.baseline_ready());
        assert!(resumed.caught_up());
    }

    #[test]
    fn native_source_observer_completion_waits_until_source_batch_is_durably_acknowledged() {
        use prometeu_protocols::terminal_status::ProgramStatusState::{Done, Working};
        let fixture = Fixture::new();
        let path = fixture
            .0
            .join("native-session/.system_generated/logs/transcript_full.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, lines(&[
            json!({"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","content":"<USER_REQUEST>\nPrompt\n</USER_REQUEST>"}),
            json!({"step_index":1,"source":"MODEL","type":"PLANNER_RESPONSE","status":"DONE","content":"Answer"}),
        ])).unwrap();
        let batches = Arc::new(Mutex::new(Vec::new()));
        let output = batches.clone();
        let (mut observer, _) = observer_start(
            ProviderId::Antigravity,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        observer.configure_completion("1.5.10");
        observer
            .set_ingestion(Box::new(move |batch| output.lock().unwrap().push(batch)))
            .unwrap();
        observer.expect_submission("Prompt", "submission").unwrap();
        let mut invocation = hook(
            ProviderId::Antigravity,
            NativeHookKind::ModelInvocationStarted,
            &path,
            Some("Prompt"),
        );
        invocation.initial_num_steps = Some(1);
        observer.accept_hook(invocation).unwrap();
        observer.observe_program_status(&completion_status(Working));
        let mut stop = hook(
            ProviderId::Antigravity,
            NativeHookKind::StopObserved,
            &path,
            Some("Prompt"),
        );
        stop.execution_number = Some(0);
        stop.fully_idle = Some(true);
        stop.reason = Some("NO_TOOL_CALL".into());
        observer.accept_hook(stop).unwrap();
        observer.observe_program_status(&completion_status(Done));
        observer.poll().unwrap();
        let batch = batches.lock().unwrap().pop().unwrap();
        assert!(observer.take_persisted_completion().is_none());
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_none());
        observer.acknowledge_ingestion(&batch.id).unwrap();
        observer.poll().unwrap();
        assert!(observer.take_persisted_completion().is_some());
    }

    #[test]
    fn terminal_observation_fresh_first_response_is_live_even_before_the_first_hook() {
        let fixture = Fixture::new();
        let (mut observer, seen) = observer_start(
            ProviderId::Cursor,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        assert!(observer.baseline_ready());
        assert!(seen.lock().unwrap().replay[0].is_empty());
        assert_eq!(
            seen.lock().unwrap().baselines[0].1,
            NativeBaselineReason::Fresh
        );
        observer.expect_prompt("Fast input").unwrap();
        std::fs::write(
            fixture.file(),
            lines(&[
                cursor("user", "Fast input"),
                cursor("assistant", "Fast answer"),
                json!({"type":"turn_ended","status":"success"}),
            ]),
        )
        .unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen.prompts.is_empty());
        assert_eq!(seen.replay.len(), 1);
        assert!(seen
            .live
            .iter()
            .any(|event| event["block"]["text"] == "Fast answer"));
        assert_eq!(
            seen.live
                .iter()
                .filter(|event| event["type"] == "turn.completed")
                .count(),
            1
        );
        assert!(seen
            .live_revisions
            .iter()
            .all(|revision| revision == &seen.baselines[0].0));
    }

    #[test]
    fn terminal_observation_rewrite_has_new_ids_and_does_not_dedup_repeated_text() {
        let fixture = Fixture::new();
        let (mut observer, seen) = observer_start(
            ProviderId::Cursor,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        std::fs::write(
            fixture.file(),
            lines(&[cursor("user", "Repeat"), cursor("assistant", "Same answer")]),
        )
        .unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        let first = seen.lock().unwrap().live[0]["messageId"].clone();
        std::fs::write(fixture.file(), "").unwrap();
        observer.poll().unwrap();
        append(
            &fixture.file(),
            &[cursor("user", "Repeat"), cursor("assistant", "Same answer")],
        );
        observer.poll().unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.prompts, vec!["Repeat", "Repeat"]);
        assert_eq!(seen.live.len(), 2);
        assert_ne!(seen.live[1]["messageId"], first);
        assert_ne!(seen.live_revisions[0], seen.live_revisions[1]);
        assert_eq!(
            seen.baselines.last().unwrap().1,
            NativeBaselineReason::Rewrite
        );
    }

    #[test]
    fn terminal_observation_claude_replay_and_hooks_never_claim_readiness_or_completion() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.file(),
            lines(&[
                claude_user("Old prompt"),
                claude_answer("old", "Old answer"),
            ]),
        )
        .unwrap();
        let (mut observer, seen) = observer(ProviderId::Claude, fixture.0.clone());
        assert!(!observer.baseline_ready());
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        assert!(observer.baseline_ready());
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::StopObserved,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen.live.is_empty());
        assert!(seen.prompts.is_empty());
        assert_eq!(seen.replay.len(), 1);
        assert!(seen.replay[0]
            .iter()
            .any(|frame| frame["block"]["text"] == "Old answer"));
        assert_eq!(
            seen.hooks,
            vec![NativeHookKind::SessionStarted, NativeHookKind::StopObserved]
        );
    }

    #[test]
    fn terminal_observation_prompts_are_accepted_once_for_either_hook_transcript_order() {
        let fixture = Fixture::new();
        std::fs::write(fixture.file(), "").unwrap();
        let (mut observer, seen) = observer(ProviderId::Claude, fixture.0.clone());
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        observer.expect_prompt("Injected").unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Injected"),
            ))
            .unwrap();
        append(
            &fixture.file(),
            &[
                claude_user("Injected"),
                claude_answer("reply", "Real answer"),
            ],
        );
        observer.poll().unwrap();
        assert!(seen.lock().unwrap().prompts.is_empty());

        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Typed"),
            ))
            .unwrap();
        append(&fixture.file(), &[claude_user("Typed")]);
        observer.poll().unwrap();
        append(&fixture.file(), &[claude_user("Late hook")]);
        observer.poll().unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Late hook"),
            ))
            .unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Typed"),
            ))
            .unwrap();
        append(&fixture.file(), &[claude_user("Typed")]);
        observer.poll().unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.prompts, vec!["Typed", "Late hook", "Typed"]);
        assert_eq!(
            seen.live
                .iter()
                .filter(|frame| frame["block"]["text"] == "Real answer")
                .count(),
            1
        );
        assert!(seen
            .live
            .iter()
            .all(|frame| frame["type"] != "user.message"));
    }

    #[test]
    fn terminal_observation_cursor_rewrites_and_truncation_replace_only_replay() {
        let fixture = Fixture::new();
        std::fs::write(fixture.file(), "").unwrap();
        let (mut observer, seen) = observer(ProviderId::Cursor, fixture.0.clone());
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        append(
            &fixture.file(),
            &[
                cursor("user", "Native prompt"),
                cursor("assistant", "First answer"),
            ],
        );
        observer.poll().unwrap();
        assert_eq!(seen.lock().unwrap().prompts, vec!["Native prompt"]);
        let live = seen.lock().unwrap().live.len();
        std::fs::write(
            fixture.file(),
            lines(&[
                cursor("user", "Different history"),
                cursor("assistant", "Replacement"),
            ]),
        )
        .unwrap();
        observer.poll().unwrap();
        assert_eq!(seen.lock().unwrap().live.len(), live);
        assert!(seen
            .lock()
            .unwrap()
            .replay
            .last()
            .unwrap()
            .iter()
            .any(|frame| frame["block"]["text"] == "Replacement"));
        std::fs::write(fixture.file(), "").unwrap();
        observer.poll().unwrap();
        assert!(seen.lock().unwrap().replay.last().unwrap().is_empty());
        assert_eq!(seen.lock().unwrap().live.len(), live);
        assert!(seen
            .lock()
            .unwrap()
            .warnings
            .contains(&"terminal.observation.rewrite"));
    }

    #[test]
    fn terminal_observation_rejects_stale_hooks_other_sessions_and_traversal_before_reading() {
        let fixture = Fixture::new();
        let (mut observer, seen) = observer(ProviderId::Claude, fixture.0.clone());
        let valid = hook(
            ProviderId::Claude,
            NativeHookKind::SessionStarted,
            &fixture.file(),
            None,
        );
        let mut stale = valid.clone();
        stale.process_nonce = "old-incarnation".into();
        assert_eq!(observer.accept_hook(stale), Err(ObservationError::Identity));
        let mut wrong = valid.clone();
        wrong.native_session = "other-native-session".into();
        assert_eq!(observer.accept_hook(wrong), Err(ObservationError::Identity));
        let mut wrong = valid.clone();
        wrong.transcript_path = Some(fixture.0.join("other-native-session.jsonl"));
        assert_eq!(observer.accept_hook(wrong), Err(ObservationError::Path));
        let mut wrong = valid;
        wrong.transcript_path = Some(fixture.0.join("../native-session.jsonl"));
        assert_eq!(observer.accept_hook(wrong), Err(ObservationError::Path));
        assert!(seen.lock().unwrap().hooks.is_empty());
        assert!(seen.lock().unwrap().live.is_empty());
    }

    #[test]
    fn terminal_observation_subagents_cannot_bind_root_content_or_capture_root_prompts() {
        let fixture = Fixture::new();
        let (mut observer, seen) = observer(ProviderId::Codex, fixture.0.clone());
        observer.native_session = None;
        let mut child = hook(
            ProviderId::Codex,
            NativeHookKind::PromptSubmitted,
            &fixture.0.join("child-session.jsonl"),
            Some("Child task"),
        );
        child.agent_id = Some("worker-1".into());
        observer.accept_hook(child).unwrap();
        assert_eq!(observer.native_session(), None);
        assert!(observer.transcript.is_none());
        assert!(seen.lock().unwrap().prompts.is_empty());
        assert_eq!(
            seen.lock().unwrap().hooks,
            vec![NativeHookKind::PromptSubmitted]
        );

        // Codex's authenticated root UserPromptSubmit and rollout user_message are two
        // observations of one accepted injected prompt, not a second user action.
        std::fs::write(fixture.file(), "").unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Codex,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        assert_eq!(observer.native_session(), Some("native-session"));
        observer.expect_prompt("Injected root task").unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Codex,
                NativeHookKind::PromptSubmitted,
                &fixture.file(),
                Some("Injected root task"),
            ))
            .unwrap();
        append(
            &fixture.file(),
            &[json!({
                "type":"event_msg", "payload":{"type":"user_message", "message":"Injected root task"}
            })],
        );
        observer.poll().unwrap();
        assert!(seen.lock().unwrap().prompts.is_empty());
    }

    #[test]
    fn terminal_observation_does_not_follow_payload_symlinks_or_hardlinks() {
        let fixture = Fixture::new();
        let outside = Fixture::new();
        std::fs::write(outside.file(), lines(&[claude_user("Other fixture")])).unwrap();
        symlink(&outside.0, fixture.0.join("linked")).unwrap();
        let (mut observer, seen) = observer(ProviderId::Claude, fixture.0.clone());
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.0.join("linked/native-session.jsonl"),
                None,
            ))
            .unwrap();
        assert_eq!(observer.poll(), Err(ObservationError::Path));
        assert!(seen.lock().unwrap().replay.is_empty());
        std::fs::hard_link(outside.file(), fixture.file()).unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        assert_eq!(observer.poll(), Err(ObservationError::Path));
        assert!(seen.lock().unwrap().replay.is_empty());
    }

    #[test]
    fn terminal_observation_trusted_root_symlink_is_pinned_before_retargeting() {
        let fixture = Fixture::new();
        let original = Fixture::new();
        let replacement = Fixture::new();
        std::fs::write(
            original.file(),
            lines(&[cursor("assistant", "Original root")]),
        )
        .unwrap();
        std::fs::write(
            replacement.file(),
            lines(&[cursor("assistant", "Unapproved replacement")]),
        )
        .unwrap();
        let root = fixture.0.join("profile-transcripts");
        symlink(&original.0, &root).unwrap();
        let (mut observer, seen) = observer(ProviderId::Cursor, root.clone());
        std::fs::remove_file(&root).unwrap();
        symlink(&replacement.0, &root).unwrap();
        observer
            .resume_from(root.join("native-session.jsonl"))
            .unwrap();
        observer.poll().unwrap();
        assert_eq!(
            seen.lock().unwrap().replay[0][0]["block"]["text"],
            "Original root"
        );
    }

    #[test]
    fn terminal_observation_accepts_an_explicit_missing_root_then_reads_only_that_root() {
        let fixture = Fixture::new();
        let root = fixture.0.join("future");
        let path = root.join("native-session.jsonl");
        let (mut observer, seen) = observer(ProviderId::Cursor, root.clone());
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &path,
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        assert!(seen.lock().unwrap().warnings.is_empty());
        std::fs::create_dir(&root).unwrap();
        std::fs::write(&path, lines(&[cursor("assistant", "Created later")])).unwrap();
        observer.poll().unwrap();
        assert!(seen.lock().unwrap().live.is_empty());
        assert_eq!(
            seen.lock().unwrap().replay[0][0]["block"]["text"],
            "Created later"
        );
    }

    #[test]
    fn terminal_observation_wrong_native_identity_and_oversize_never_publish_content() {
        let fixture = Fixture::new();
        let mut wrong = claude_user("Wrong conversation fixture");
        wrong["sessionId"] = json!("different-session");
        std::fs::write(fixture.file(), lines(&[wrong])).unwrap();
        let (mut observer, seen) = observer(ProviderId::Claude, fixture.0.clone());
        observer
            .accept_hook(hook(
                ProviderId::Claude,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        assert_eq!(observer.poll(), Err(ObservationError::Identity));
        std::fs::write(
            fixture.file(),
            lines(&[claude_answer("valid", "Must remain closed")]),
        )
        .unwrap();
        assert_eq!(observer.poll(), Err(ObservationError::Identity));
        assert!(seen.lock().unwrap().replay.is_empty());
        assert!(seen.lock().unwrap().live.is_empty());

        let (mut observer, seen) = observer_for_size(&fixture);
        File::create(fixture.file())
            .unwrap()
            .set_len(TRANSCRIPT_SNAPSHOT_LIMIT as u64 + 1)
            .unwrap();
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        assert_eq!(observer.poll(), Err(ObservationError::Limit));
        assert_eq!(
            seen.lock().unwrap().warnings,
            vec!["terminal.observation.limit"]
        );
        assert!(seen.lock().unwrap().replay.is_empty());
    }
    fn observer_for_size(fixture: &Fixture) -> (TerminalAgentObservation, Arc<Mutex<Seen>>) {
        observer(ProviderId::Cursor, fixture.0.clone())
    }

    #[test]
    fn terminal_observation_trusted_resume_binding_bootstraps_without_fabricating_hooks() {
        let fixture = Fixture::new();
        std::fs::write(
            fixture.file(),
            lines(&[cursor("assistant", "Prior answer")]),
        )
        .unwrap();
        let (mut resumed, seen) = observer(ProviderId::Cursor, fixture.0.clone());
        assert_eq!(
            resumed.resume_from(fixture.0.join("other-session.jsonl")),
            Err(ObservationError::Path)
        );
        resumed.resume_from(fixture.file()).unwrap();
        resumed.resume_from(fixture.file()).unwrap();
        assert!(!resumed.baseline_ready() && !resumed.caught_up());
        resumed.poll().unwrap();
        assert!(resumed.baseline_ready() && resumed.caught_up());
        let seen = seen.lock().unwrap();
        assert!(seen.hooks.is_empty() && seen.live.is_empty());
        assert_eq!(seen.replay[0][0]["block"]["text"], "Prior answer");
        drop(seen);
        let (mut fresh, _) = observer_start(
            ProviderId::Cursor,
            fixture.0.clone(),
            ObservationStart::Fresh,
        );
        assert_eq!(
            fresh.resume_from(fixture.file()),
            Err(ObservationError::Identity)
        );
        let (mut unbound, _) = observer(ProviderId::Cursor, fixture.0.clone());
        unbound.config.expected_native_session = None;
        assert_eq!(
            unbound.resume_from(fixture.file()),
            Err(ObservationError::Identity)
        );
    }

    #[test]
    fn terminal_observation_large_append_only_histories_bootstrap_in_bounded_reads() {
        for provider in [ProviderId::Claude, ProviderId::Codex] {
            let fixture = Fixture::new();
            let answer = |id: &str, text: &str| match provider {
                ProviderId::Claude => claude_answer(id, text),
                _ => {
                    json!({"type":"response_item","payload":{"type":"message","role":"assistant","id":id,"content":[{"type":"output_text","text":text}]}})
                }
            };
            let mut file = File::create(fixture.file()).unwrap();
            file.write_all(lines(&[answer("old", "Historical answer")]).as_bytes())
                .unwrap();
            let padding = format!(
                "{}\n",
                json!({"type":"ignored_fixture","padding":"x".repeat(4096)})
            );
            for _ in 0..(TRANSCRIPT_SNAPSHOT_LIMIT / padding.len() + 2) {
                file.write_all(padding.as_bytes()).unwrap();
            }
            let size = file.metadata().unwrap().len();
            assert!(size > TRANSCRIPT_SNAPSHOT_LIMIT as u64);
            drop(file);
            let (mut observer, seen) = observer(provider, fixture.0.clone());
            observer
                .accept_hook(hook(
                    provider,
                    NativeHookKind::SessionStarted,
                    &fixture.file(),
                    None,
                ))
                .unwrap();
            observer.poll().unwrap();
            assert_eq!(observer.append_offset, APPEND_POLL_LIMIT);
            assert!(!observer.baseline_ready());
            for _ in 0..16 {
                if observer.baseline_ready() {
                    break;
                }
                let previous = observer.append_offset;
                observer.poll().unwrap();
                assert!(observer.append_offset - previous <= APPEND_POLL_LIMIT);
            }
            assert!(observer.baseline_ready());
            assert_eq!(observer.append_offset, size);
            assert!(seen.lock().unwrap().live.is_empty());
            assert!(seen.lock().unwrap().warnings.is_empty());
            append(&fixture.file(), &[answer("new", "New answer")]);
            observer.poll().unwrap();
            let seen = seen.lock().unwrap();
            assert_eq!(
                seen.live
                    .iter()
                    .filter(|frame| frame["block"]["text"] == "New answer")
                    .count(),
                1
            );
            assert!(seen
                .live
                .iter()
                .all(|frame| frame["block"]["text"] != "Historical answer"));
        }
    }

    #[test]
    fn terminal_observation_large_inline_images_preserve_fresh_input_and_resume_quietly() {
        for provider in [ProviderId::Claude, ProviderId::Codex] {
            for start in [ObservationStart::Fresh, ObservationStart::Resume] {
                let fixture = Fixture::new();
                let (mut observer, seen) = observer_start(provider, fixture.0.clone(), start);
                assert!(!observer.caught_up());
                let data = "A".repeat(6 * 1024 * 1024);
                let rows = match provider {
                    ProviderId::Claude => vec![
                        json!({"type":"user","sessionId":"native-session","message":{"role":"user","content":[
                            {"type":"text","text":"Describe this image"},
                            {"type":"image","source":{"type":"base64","media_type":"image/png","data":data}}
                        ]}}),
                        claude_answer("answer", "Visible answer"),
                    ],
                    _ => vec![
                        json!({"type":"event_msg","payload":{"type":"user_message","message":"Describe this image","images":[format!("data:image/png;base64,{data}")]}}),
                        json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-1"}}),
                        json!({"type":"response_item","payload":{"type":"message","role":"assistant","id":"answer","content":[{"type":"output_text","text":"Visible answer"}]}}),
                        json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}),
                    ],
                };
                std::fs::write(fixture.file(), lines(&rows)).unwrap();
                observer
                    .accept_hook(hook(
                        provider,
                        NativeHookKind::SessionStarted,
                        &fixture.file(),
                        None,
                    ))
                    .unwrap();
                observer.poll().unwrap();
                assert!(!observer.caught_up());
                if start == ObservationStart::Resume {
                    assert!(!observer.baseline_ready());
                }
                observer.poll().unwrap();
                assert!(observer.caught_up());
                assert!(observer.baseline_ready());
                let seen = seen.lock().unwrap();
                assert!(seen.warnings.is_empty());
                if start == ObservationStart::Fresh {
                    assert_eq!(seen.prompts, vec!["Describe this image"]);
                    assert!(seen
                        .live
                        .iter()
                        .any(|frame| frame["block"]["text"] == "Visible answer"));
                    if provider == ProviderId::Codex {
                        assert_eq!(
                            seen.live
                                .iter()
                                .filter(|frame| frame["type"] == "turn.completed")
                                .count(),
                            1
                        );
                    }
                } else {
                    assert!(seen.prompts.is_empty() && seen.live.is_empty());
                    let baseline = seen.replay.last().unwrap();
                    assert!(baseline
                        .iter()
                        .any(|frame| frame["content"][0]["text"] == "Describe this image"));
                    assert!(baseline
                        .iter()
                        .any(|frame| frame["block"]["text"] == "Visible answer"));
                }
            }
        }
    }

    #[test]
    fn terminal_observation_partial_replay_never_reaches_the_live_callback() {
        let fixture = Fixture::new();
        let initial = lines(&[cursor("assistant", "Old partial")]);
        let split = initial.len() / 2;
        std::fs::write(fixture.file(), &initial.as_bytes()[..split]).unwrap();
        let (mut observer, seen) = observer(ProviderId::Cursor, fixture.0.clone());
        observer
            .accept_hook(hook(
                ProviderId::Cursor,
                NativeHookKind::SessionStarted,
                &fixture.file(),
                None,
            ))
            .unwrap();
        observer.poll().unwrap();
        assert!(!observer.baseline_ready());
        OpenOptions::new()
            .append(true)
            .open(fixture.file())
            .unwrap()
            .write_all(&initial.as_bytes()[split..])
            .unwrap();
        append(&fixture.file(), &[cursor("assistant", "New live")]);
        observer.poll().unwrap();
        assert!(observer.baseline_ready());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.replay.last().unwrap().len(), 1);
        assert_eq!(
            seen.replay.last().unwrap()[0]["block"]["text"],
            "Old partial"
        );
        assert_eq!(seen.live.len(), 1);
        assert_eq!(seen.live[0]["block"]["text"], "New live");
    }
}
