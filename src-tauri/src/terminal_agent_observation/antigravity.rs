//! Original AGY SQLite steps replace the lossy summary JSONL as the canonical content source.
use super::*;
use crate::terminal_agent_antigravity_store::AntigravityStoreReader;
use crate::terminal_agent_sqlite_snapshot::SnapshotProgress;
use prometeu_protocols::antigravity_store::{
    projection::{completion_record, Projection},
    Schema,
};
use prometeu_protocols::terminal_transcript::TranscriptRecord;
use std::collections::BTreeSet;

pub struct AntigravityObservationConfig {
    pub conversations_root: PathBuf,
    pub brain_root: PathBuf,
    pub cli_version: String,
    pub staging_root: PathBuf,
}
pub(super) struct AntigravityObservation {
    config: AntigravityObservationConfig,
    roots: Option<(AuthorizedRoot, AuthorizedRoot)>,
    reader: Option<AntigravityStoreReader>,
    projection: Option<Projection>,
    next: Option<u64>,
    pending: Option<NativeIngestionBatch>,
    callback: Option<Box<dyn FnMut(NativeIngestionBatch) + Send>>,
    phase: TranscriptPhase,
    known_users: BTreeSet<String>,
    submissions: VecDeque<(String, String)>,
    submission_bytes: usize,
    processed: bool,
    caught_up: bool,
    baseline_ready: bool,
}
impl AntigravityObservation {
    pub(super) fn new(
        config: AntigravityObservationConfig,
        start: ObservationStart,
    ) -> Result<Self, ObservationError> {
        if !safe_absolute(&config.conversations_root)
            || !safe_absolute(&config.brain_root)
            || !safe_absolute(&config.staging_root)
            || Schema::for_version(&config.cli_version).is_none()
        {
            return Err(ObservationError::Path);
        }
        let roots = Some((
            AuthorizedRoot::new(config.conversations_root.clone())?,
            AuthorizedRoot::new(config.brain_root.clone())?,
        ));
        Ok(Self {
            config,
            roots,
            reader: None,
            projection: None,
            next: None,
            pending: None,
            callback: None,
            phase: if start == ObservationStart::Fresh {
                TranscriptPhase::Live
            } else {
                TranscriptPhase::Replay
            },
            known_users: BTreeSet::new(),
            submissions: VecDeque::new(),
            submission_bytes: 0,
            processed: false,
            caught_up: false,
            baseline_ready: start == ObservationStart::Fresh,
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
        Ok(())
    }
    pub(super) fn acknowledge(&mut self, id: &str) -> Result<(), ObservationError> {
        let batch = self
            .pending
            .as_ref()
            .filter(|b| b.id == id)
            .ok_or(ObservationError::Identity)?;
        for record in &batch.records {
            if record.checkpoint["kind"] == "user" {
                self.known_users.insert(record.occurrence.clone());
            }
        }
        if self.known_users.len() > PROJECTION_EVENT_LIMIT {
            return Err(ObservationError::Limit);
        }
        if batch.finished && batch.complete {
            self.phase = TranscriptPhase::Live;
            self.baseline_ready = true;
            self.processed = true;
            self.projection = None;
        }
        self.pending = None;
        self.caught_up = false;
        Ok(())
    }
    pub(super) fn poll(
        &mut self,
        native: Option<&str>,
        at: u64,
        completion: Option<&CompletionState>,
    ) -> Result<(), ObservationError> {
        self.caught_up = false;
        if let Some(batch) = self.pending.clone() {
            (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
            return Ok(());
        }
        let Some(native) = native else { return Ok(()) };
        if self.reader.is_none() {
            let (root, brain) = self.roots.take().ok_or(ObservationError::Identity)?;
            self.reader = Some(AntigravityStoreReader::new(
                root,
                self.config.conversations_root.clone(),
                brain,
                self.config.brain_root.clone(),
                native.into(),
                &self.config.cli_version,
                self.config.staging_root.clone(),
            )?);
        }
        let reader = self.reader.as_mut().ok_or(ObservationError::Read)?;
        if self.projection.is_none() {
            match reader.poll()? {
                SnapshotProgress::Pending => return Ok(()),
                SnapshotProgress::Unchanged if self.processed => {
                    self.caught_up = true;
                    return Ok(());
                }
                SnapshotProgress::Unchanged | SnapshotProgress::Ready => {}
            }
            if let Some(completion) = completion {
                crate::lock::lock(completion).refresh_snapshot();
            }
            self.projection = Some(
                Projection::new(
                    native,
                    reader.trajectory().ok_or(ObservationError::Identity)?,
                )
                .map_err(|_| ObservationError::Transcript)?,
            );
            self.next = None;
        }
        let mut records = Vec::new();
        let step = reader.next(self.next)?;
        let finished = step.is_none();
        if let Some(step) = step {
            if !step.complete {
                return Err(ObservationError::Transcript);
            }
            self.next = Some(step.index);
            if let Some(completion) = completion {
                crate::lock::lock(completion).record(&TranscriptRecord {
                    ordinal: step.index,
                    offset: step.index,
                    phase: self.phase,
                    value: completion_record(&step),
                    omitted_media: 0,
                });
            }
            for capture in self
                .projection
                .as_mut()
                .ok_or(ObservationError::Identity)?
                .push(&step, at)
                .map_err(|_| ObservationError::Transcript)?
            {
                let mut input_binding = None;
                if self.phase == TranscriptPhase::Live
                    && capture.checkpoint["kind"] == "user"
                    && !self.known_users.contains(&capture.occurrence)
                {
                    if let Some(text) = capture.frame.as_ref().and_then(user_text) {
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
                    occurrence: capture.occurrence,
                    version: capture.version,
                    checkpoint: capture.checkpoint,
                    frame: capture.frame,
                    input_binding,
                });
            }
        }
        let batch = NativeIngestionBatch {
            id: uuid::Uuid::new_v4().to_string(),
            phase: self.phase,
            source_revision: reader.revision().into(),
            records,
            finished,
            complete: true,
            mode: NativeIngestionMode::Snapshot,
        };
        self.pending = Some(batch.clone());
        (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
        Ok(())
    }
}
