use super::*;
use crate::terminal_agent_cursor_store::{CursorReadError, CursorReadProgress, CursorStoreReader};
use prometeu_protocols::cursor_store::{
    self,
    projection::{CursorCapture, CursorProjection},
    CursorRoot, CursorStoreSchema, StoreError,
};
use std::collections::BTreeSet;

/// The caller supplies the captured account's `<config>/chats/<original-cwd-hash>` directory.
/// Native identity chooses one child only, and is never obtained by scanning this directory.
pub struct CursorObservationConfig {
    pub chats_root: PathBuf,
    pub cli_version: String,
    pub staging_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeCapture {
    pub occurrence: String,
    pub version: String,
    pub checkpoint: Value,
    pub frame: Option<Value>,
    /// Proven FIFO match to an app submission receipt, never a text-derived identity.
    pub input_binding: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeIngestionBatch {
    pub id: String,
    pub phase: TranscriptPhase,
    pub source_revision: String,
    pub records: Vec<NativeCapture>,
    pub finished: bool,
    pub complete: bool,
    pub mode: NativeIngestionMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeIngestionMode {
    Snapshot,
    Append,
}

pub(super) struct CursorObservation {
    config: CursorObservationConfig,
    root: Option<AuthorizedRoot>,
    schema: CursorStoreSchema,
    reader: Option<CursorStoreReader>,
    projection: Option<CursorProjection>,
    pending: Option<NativeIngestionBatch>,
    callback: Option<Box<dyn FnMut(NativeIngestionBatch) + Send>>,
    phase: TranscriptPhase,
    processed: bool,
    coverage: bool,
    coverage_lost: bool,
    caught_up: bool,
    baseline_ready: bool,
    known_users: BTreeSet<String>,
    submissions: VecDeque<(String, String)>,
    submission_bytes: usize,
    pub(super) tail_turn: Option<cursor_store::projection::CursorTailTurn>,
    pub(super) stops: VecDeque<cursor_completion::CursorStop>,
    pub(super) prompt_generation: Option<String>,
    pub(super) completed_request: Option<String>,
}

impl CursorObservation {
    pub(super) fn new(
        config: CursorObservationConfig,
        start: ObservationStart,
    ) -> Result<Self, ObservationError> {
        if !safe_absolute(&config.chats_root) || !safe_absolute(&config.staging_root) {
            return Err(ObservationError::Path);
        }
        let schema = CursorStoreSchema::for_version(&config.cli_version)
            .ok_or(ObservationError::Transcript)?;
        let root = AuthorizedRoot::new(config.chats_root.clone())?;
        Ok(Self {
            config,
            root: Some(root),
            schema,
            reader: None,
            projection: None,
            pending: None,
            callback: None,
            phase: if start == ObservationStart::Fresh {
                TranscriptPhase::Live
            } else {
                TranscriptPhase::Replay
            },
            processed: false,
            coverage: false,
            coverage_lost: false,
            caught_up: false,
            baseline_ready: start == ObservationStart::Fresh,
            known_users: BTreeSet::new(),
            submissions: VecDeque::new(),
            submission_bytes: 0,
            tail_turn: None,
            stops: VecDeque::new(),
            prompt_generation: None,
            completed_request: None,
        })
    }
    pub(super) fn set_callback(&mut self, callback: Box<dyn FnMut(NativeIngestionBatch) + Send>) {
        self.callback = Some(callback);
    }
    pub(super) fn baseline_ready(&self) -> bool {
        self.baseline_ready
    }
    pub(super) fn caught_up(&self) -> bool {
        self.caught_up && self.pending.is_none()
    }

    pub(super) fn expect_submission(
        &mut self,
        text: &str,
        id: &str,
    ) -> Result<(), ObservationError> {
        if !valid_id(id)
            || self.submissions.len() >= PROMPT_COUNT_LIMIT
            || text.len() > PROMPT_BYTES_LIMIT.saturating_sub(self.submission_bytes)
        {
            return Err(ObservationError::Limit);
        }
        self.submission_bytes += text.len();
        self.submissions.push_back((text.into(), id.into()));
        self.stops.clear();
        Ok(())
    }

    pub(super) fn acknowledge(&mut self, id: &str) -> Result<(), ObservationError> {
        let Some(batch) = self.pending.as_ref().filter(|batch| batch.id == id) else {
            return Err(ObservationError::Identity);
        };
        let users = batch
            .records
            .iter()
            .filter(|record| record.checkpoint["kind"] == "user")
            .map(|record| record.occurrence.clone())
            .collect::<BTreeSet<_>>();
        let added = users
            .iter()
            .filter(|id| !self.known_users.contains(*id))
            .count();
        if added > PROJECTION_EVENT_LIMIT.saturating_sub(self.known_users.len()) {
            return Err(ObservationError::Limit);
        }
        self.known_users.extend(users);
        let batch = self.pending.take().ok_or(ObservationError::Identity)?;
        self.coverage_lost |= !batch.complete;
        if batch.finished {
            self.tail_turn = self
                .projection
                .as_ref()
                .and_then(CursorProjection::tail_turn);
            // beforeSubmitPrompt fences only an input that has not reached the durable source.
            // Cursor's restored-prompt retry creates a new request without invoking that hook.
            if self.prompt_generation.is_some()
                && self.prompt_generation.as_deref()
                    == self
                        .tail_turn
                        .as_ref()
                        .and_then(|tail| tail.key.request_id.as_deref())
            {
                self.prompt_generation = None;
            }
            self.projection = None;
            self.processed = true;
            self.coverage = batch.complete && !self.coverage_lost;
            self.baseline_ready |= self.coverage;
            self.phase = TranscriptPhase::Live;
        }
        // Confirm the source is still unchanged on the next poll before releasing completion.
        self.caught_up = false;
        Ok(())
    }

    pub(super) fn completion_ready(&self) -> bool {
        self.caught_up() && self.submissions.is_empty() && self.phase == TranscriptPhase::Live
    }

    pub(super) fn poll(&mut self, native: Option<&str>, at: u64) -> Result<(), ObservationError> {
        self.caught_up = false;
        if self.callback.is_none() {
            return Err(ObservationError::Identity);
        }
        if let Some(batch) = self.pending.clone() {
            (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
            return Ok(());
        }
        let Some(native) = native else { return Ok(()) };
        if !valid_native_id(native) {
            return Err(ObservationError::Identity);
        }
        if self.reader.is_none() {
            self.reader = Some(
                CursorStoreReader::from_authorized_root(
                    self.root.take().ok_or(ObservationError::Identity)?,
                    self.config.chats_root.join(native),
                    native.into(),
                    self.schema,
                    self.config.staging_root.clone(),
                )
                .map_err(read_error)?,
            );
        }
        if self.projection.is_none() {
            let reader = self.reader.as_mut().ok_or(ObservationError::Identity)?;
            match reader.poll().map_err(read_error)? {
                CursorReadProgress::Pending => return Ok(()),
                CursorReadProgress::Unchanged if self.processed => {
                    self.caught_up = self.coverage;
                    return Ok(());
                }
                CursorReadProgress::Unchanged | CursorReadProgress::Ready => {}
            }
            let snapshot = reader.snapshot().ok_or(ObservationError::Read)?;
            let root = match snapshot.metadata.root {
                Some(id) => cursor_store::root(snapshot, id, self.schema).map_err(decode_error)?,
                None => CursorRoot {
                    version: cursor_store::BlobId([0; 32]),
                    turns: Vec::new(),
                    last_run: None,
                },
            };
            self.projection = Some(CursorProjection::new(native, root, at).map_err(decode_error)?);
        }
        let projection = self.projection.as_mut().ok_or(ObservationError::Identity)?;
        let source_revision = projection.revision();
        let snapshot = self
            .reader
            .as_ref()
            .and_then(CursorStoreReader::snapshot)
            .ok_or(ObservationError::Read)?;
        let projected = projection
            .next(
                snapshot,
                self.reader.as_ref().ok_or(ObservationError::Read)?.schema(),
            )
            .map_err(decode_error)?;
        let mut new_users = BTreeSet::new();
        let mut records = Vec::with_capacity(projected.records.len());
        for CursorCapture {
            occurrence,
            version,
            checkpoint,
            frame,
        } in projected.records
        {
            let mut input_binding = None;
            if self.phase == TranscriptPhase::Live
                && checkpoint["kind"] == "user"
                && !self.known_users.contains(&occurrence)
                && new_users.insert(occurrence.clone())
            {
                if let Some(text) = frame.as_ref().and_then(user_text) {
                    if let Some((expected, id)) = self.submissions.pop_front() {
                        self.submission_bytes -= expected.len();
                        if expected == text {
                            input_binding = Some(id);
                        } else {
                            self.submissions.clear();
                            self.submission_bytes = 0;
                        }
                    }
                }
            }
            records.push(NativeCapture {
                occurrence,
                version,
                checkpoint,
                frame,
                input_binding,
            });
        }
        let batch = NativeIngestionBatch {
            mode: NativeIngestionMode::Snapshot,
            id: uuid::Uuid::new_v4().to_string(),
            phase: self.phase,
            source_revision,
            records,
            finished: projected.finished,
            complete: projected.complete && !self.coverage_lost,
        };
        self.pending = Some(batch.clone());
        (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
        Ok(())
    }
}

#[cfg(test)]
mod tests;

fn read_error(error: CursorReadError) -> ObservationError {
    match error {
        CursorReadError::Path => ObservationError::Path,
        CursorReadError::Read => ObservationError::Read,
        CursorReadError::Unstable => ObservationError::Unstable,
        CursorReadError::Schema => ObservationError::Transcript,
        CursorReadError::Identity => ObservationError::Identity,
    }
}
fn decode_error(error: StoreError) -> ObservationError {
    match error {
        StoreError::Identity => ObservationError::Identity,
        StoreError::Limit => ObservationError::Limit,
        StoreError::Read | StoreError::Missing => ObservationError::Read,
        StoreError::Malformed | StoreError::Integrity | StoreError::Unsupported => {
            ObservationError::Transcript
        }
    }
}
