//! Source receipts for append transcripts and rewritten native trajectories.
use super::*;
use prometeu_protocols::terminal_transcript::TranscriptRecord;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const RECORDS_PER_BATCH: usize = 64;
const STAGED_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct SourceObservation {
    provider: ProviderId,
    phase: TranscriptPhase,
    staged: VecDeque<NativeCapture>,
    staged_bytes: usize,
    pending: Option<NativeIngestionBatch>,
    callback: Option<Box<dyn FnMut(NativeIngestionBatch) + Send>>,
    tools: HashMap<String, (String, u64, String, Option<String>)>,
    source_users: HashMap<String, Option<String>>,
    current_user: Option<String>,
    known_users: HashSet<String>,
    submissions: VecDeque<(String, String)>,
    submission_bytes: usize,
    revision: String,
    mode: NativeIngestionMode,
    partial: bool,
    complete: bool,
    baseline_ready: bool,
    confirmed: bool,
    failure: Option<ObservationError>,
}

impl SourceObservation {
    pub(super) fn new(provider: ProviderId, start: ObservationStart) -> Self {
        Self {
            provider,
            phase: if start == ObservationStart::Fresh {
                TranscriptPhase::Live
            } else {
                TranscriptPhase::Replay
            },
            staged: VecDeque::new(),
            staged_bytes: 0,
            pending: None,
            callback: None,
            tools: HashMap::new(),
            source_users: HashMap::new(),
            current_user: None,
            known_users: HashSet::new(),
            submissions: VecDeque::new(),
            submission_bytes: 0,
            revision: String::new(),
            mode: NativeIngestionMode::Snapshot,
            partial: false,
            complete: true,
            baseline_ready: start == ObservationStart::Fresh,
            confirmed: false,
            failure: None,
        }
    }

    pub(super) fn set_callback(&mut self, callback: Box<dyn FnMut(NativeIngestionBatch) + Send>) {
        self.callback = Some(callback);
    }
    pub(super) fn baseline_ready(&self) -> bool {
        self.baseline_ready && self.complete && self.failure.is_none()
    }
    pub(super) fn caught_up(&self) -> bool {
        self.confirmed
            && self.complete
            && !self.partial
            && self.pending.is_none()
            && self.staged.is_empty()
            && self.failure.is_none()
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
    pub(super) fn before_poll(&mut self) -> Result<bool, ObservationError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.pending.is_some() || !self.staged.is_empty() {
            self.dispatch()?;
            return Ok(true);
        }
        self.confirmed = false;
        Ok(false)
    }
    pub(super) fn unchanged(&mut self) {
        self.confirmed = true;
    }
    pub(super) fn finish_poll(
        &mut self,
        partial: bool,
        loss: bool,
        revision: &str,
        replaced: bool,
    ) -> Result<(), ObservationError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        self.partial = partial;
        self.complete &= !loss;
        self.revision = revision.into();
        self.mode = if replaced || self.phase == TranscriptPhase::Replay {
            NativeIngestionMode::Snapshot
        } else {
            NativeIngestionMode::Append
        };
        self.confirmed = false;
        self.dispatch()
    }
    fn dispatch(&mut self) -> Result<(), ObservationError> {
        if let Some(batch) = self.pending.clone() {
            (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
            return Ok(());
        }
        let count = self.staged.len().min(RECORDS_PER_BATCH);
        let records = self.staged.drain(..count).collect::<Vec<_>>();
        self.staged_bytes -= records.iter().map(capture_size).sum::<usize>();
        let batch = NativeIngestionBatch {
            id: uuid::Uuid::new_v4().to_string(),
            phase: self.phase,
            source_revision: self.revision.clone(),
            records,
            finished: self.staged.is_empty() && !self.partial,
            complete: self.complete,
            mode: self.mode,
        };
        self.pending = Some(batch.clone());
        (self.callback.as_mut().ok_or(ObservationError::Identity)?)(batch);
        Ok(())
    }
    pub(super) fn acknowledge(&mut self, id: &str) -> Result<(), ObservationError> {
        let batch = self
            .pending
            .as_ref()
            .filter(|batch| batch.id == id)
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
        }
        self.pending = None;
        self.confirmed = false;
        Ok(())
    }

    pub(super) fn observe(&mut self, native: &str, record: &TranscriptRecord, frames: &[Value]) {
        if self.failure.is_some() {
            return;
        }
        if let Err(error) = self.project(native, record, frames) {
            self.failure = Some(error);
        }
    }
    fn project(
        &mut self,
        native: &str,
        record: &TranscriptRecord,
        frames: &[Value],
    ) -> Result<(), ObservationError> {
        let row = &record.value;
        if record.ordinal == 0 {
            self.current_user = None;
        }
        if self.provider == ProviderId::Claude
            && (row["isSidechain"] == true
                || row["agentId"].is_string()
                || row["prometheusV1Mirror"] == true)
        {
            return Ok(());
        }
        let source = match self.provider {
            ProviderId::Claude => row["uuid"]
                .as_str()
                .filter(|id| valid_native_id(id))
                .map(|id| format!("native:claude:{native}:{id}")),
            ProviderId::Antigravity => row["step_index"]
                .as_u64()
                .map(|index| format!("native:antigravity:{native}:step:{index}")),
            _ => return Err(ObservationError::Identity),
        };
        let Some(message) = source else {
            if frames.is_empty() {
                return Ok(());
            }
            return Err(ObservationError::Identity);
        };
        let version = digest(&row.to_string());
        let parent_user = (self.provider == ProviderId::Claude)
            .then(|| {
                row["parentUuid"].as_str().and_then(|parent| {
                    self.source_users
                        .get(&format!("native:claude:{native}:{parent}"))
                        .cloned()
                        .flatten()
                })
            })
            .flatten();
        let mut source_user = match self.source_users.get(&message) {
            Some(Some(known)) => {
                if parent_user.as_ref().is_some_and(|parent| parent != known)
                    && !frames.iter().any(|frame| frame["type"] == "user.message")
                {
                    return Err(ObservationError::Identity);
                }
                Some(known.clone())
            }
            Some(None) => parent_user,
            None if self.provider == ProviderId::Claude && row["parentUuid"].is_string() => {
                parent_user
            }
            None => self.current_user.clone(),
        };
        if self.provider == ProviderId::Antigravity {
            self.complete &= row["truncated_fields"].as_array().is_none_or(Vec::is_empty);
            if row["type"] == "GENERIC"
                || row["tool_calls"]
                    .as_array()
                    .is_some_and(|calls| !calls.is_empty())
            {
                // Summary JSON omits tool-call identity and its result relationship. Preserve
                // supported content, but never certify complete ingestion of these records.
                self.complete = false;
            }
        }
        let mut block_counts = HashMap::<String, usize>::new();
        let mut metadata_counts = HashMap::<String, u64>::new();
        for original in frames {
            let mut frame = original.clone();
            let mut binding = None;
            let (occurrence, mut checkpoint) = match frame["type"].as_str() {
                Some("user.message") => {
                    if self.provider == ProviderId::Antigravity {
                        let Some(text) =
                            antigravity_user(row["content"].as_str().unwrap_or_default())
                        else {
                            self.complete = false;
                            continue;
                        };
                        frame["content"] = json!([{"kind":"text","text":text}]);
                    }
                    let occurrence = format!("{message}:user");
                    source_user = Some(occurrence.clone());
                    self.current_user = source_user.clone();
                    if self.phase == TranscriptPhase::Live
                        && !self.known_users.contains(&occurrence)
                    {
                        if let Some((expected, id)) = self.submissions.pop_front() {
                            self.submission_bytes -= expected.len();
                            if user_text(&frame) == Some(expected.as_str()) {
                                binding = Some(id);
                            } else {
                                self.submissions.clear();
                                self.submission_bytes = 0;
                            }
                        }
                    }
                    if self.known_users.len() >= PROJECTION_EVENT_LIMIT
                        && !self.known_users.contains(&occurrence)
                    {
                        return Err(ObservationError::Limit);
                    }
                    self.known_users.insert(occurrence.clone());
                    (
                        occurrence,
                        json!({"schema":1,"kind":"user","contentHash":digest(&frame["content"].to_string())}),
                    )
                }
                Some("assistant.block") => {
                    let kind = frame["block"]["kind"]
                        .as_str()
                        .ok_or(ObservationError::Transcript)?
                        .to_owned();
                    if !matches!(kind.as_str(), "text" | "thinking" | "tool") {
                        self.complete = false;
                        continue;
                    }
                    let count = block_counts.entry(kind.clone()).or_default();
                    let index = source_block_index(self.provider, row, &kind, *count)?;
                    *count += 1;
                    frame["messageId"] = json!(message);
                    frame["index"] = json!(index);
                    let occurrence = format!("{message}:block:{index}:{kind}");
                    let mut checkpoint =
                        json!({"schema":1,"kind":kind,"messageId":message,"index":index});
                    if kind == "tool" {
                        let tool = frame["block"]["id"]
                            .as_str()
                            .filter(|id| !id.is_empty())
                            .ok_or(ObservationError::Identity)?;
                        checkpoint["toolId"] = json!(tool);
                        if self.tools.len() >= PROJECTION_EVENT_LIMIT
                            && !self.tools.contains_key(tool)
                        {
                            return Err(ObservationError::Limit);
                        }
                        if let Some((owner, previous_index, previous_occurrence, previous_user)) =
                            self.tools.get(tool)
                        {
                            if owner != &message
                                || previous_index != &index
                                || previous_occurrence != &occurrence
                                || previous_user
                                    .as_ref()
                                    .is_some_and(|user| Some(user) != source_user.as_ref())
                            {
                                return Err(ObservationError::Identity);
                            }
                        }
                        self.tools.insert(
                            tool.into(),
                            (
                                message.clone(),
                                index,
                                occurrence.clone(),
                                source_user.clone(),
                            ),
                        );
                    }
                    (occurrence, checkpoint)
                }
                Some("tool.completed") => {
                    let tool = frame["toolId"].as_str().ok_or(ObservationError::Identity)?;
                    let Some((owner, index, occurrence, user)) = self.tools.get(tool) else {
                        self.complete = false;
                        continue;
                    };
                    let occurrence = occurrence
                        .strip_suffix(":tool")
                        .ok_or(ObservationError::Identity)?;
                    (
                        format!("{occurrence}:result"),
                        json!({"schema":1,"kind":"result","messageId":owner,"index":index,"toolId":tool,"sourceUser":user}),
                    )
                }
                Some(
                    "telemetry.usage" | "context.updated" | "system.summary" | "context.reported"
                    | "system.notice" | "commands.updated" | "context.compaction"
                    | "context.compacted",
                ) => {
                    let kind = frame["type"].as_str().unwrap();
                    let slot = metadata_counts.entry(kind.into()).or_default();
                    let index = *slot;
                    *slot += 1;
                    (
                        format!("{message}:metadata:{kind}:{index}"),
                        json!({"schema":1,"kind":"metadata","messageId":message,"index":index,"frameType":kind}),
                    )
                }
                // Transcript observation never creates a lifecycle or approval decision.
                Some("turn.completed" | "assistant.started") => continue,
                _ => {
                    self.complete = false;
                    continue;
                }
            };
            checkpoint["sourceScope"] = json!("record");
            if checkpoint["kind"] != "result" {
                checkpoint["sourceUser"] = json!(source_user);
            }
            self.push(NativeCapture {
                occurrence,
                version: version.clone(),
                checkpoint,
                frame: Some(frame),
                input_binding: binding,
            })?;
        }
        if self.source_users.len() >= PROJECTION_EVENT_LIMIT
            && !self.source_users.contains_key(&message)
        {
            return Err(ObservationError::Limit);
        }
        self.source_users
            .insert(message.clone(), source_user.clone());
        if (self.provider == ProviderId::Antigravity && row["status"] == "DONE")
            || (self.provider == ProviderId::Claude
                && row["type"] == "assistant"
                && row["message"]["stop_reason"].is_string())
        {
            self.push(NativeCapture {
                occurrence: format!("{message}:checkpoint"), version,
                checkpoint: json!({"schema":1,"kind":"turn","messageId":message,"status":"completed","sourceScope":"record","sourceUser":source_user}),
                frame: None, input_binding: None,
            })?;
        }
        Ok(())
    }
    fn push(&mut self, record: NativeCapture) -> Result<(), ObservationError> {
        let size = capture_size(&record);
        if self.staged.len() >= PROJECTION_EVENT_LIMIT
            || size > STAGED_BYTES.saturating_sub(self.staged_bytes)
        {
            return Err(ObservationError::Limit);
        }
        self.staged_bytes += size;
        self.staged.push_back(record);
        Ok(())
    }
}

fn capture_size(record: &NativeCapture) -> usize {
    record.checkpoint.to_string().len()
        + record
            .frame
            .as_ref()
            .map_or(0, |frame| frame.to_string().len())
        + record.occurrence.len()
        + record.version.len()
}
fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn source_block_index(
    provider: ProviderId,
    row: &Value,
    kind: &str,
    ordinal: usize,
) -> Result<u64, ObservationError> {
    if provider == ProviderId::Antigravity {
        return match kind {
            "thinking" if ordinal == 0 => Ok(0),
            "text" if ordinal == 0 => Ok(1),
            "tool" => Ok(2 + ordinal as u64),
            _ => Err(ObservationError::Transcript),
        };
    }
    let raw_kind = if kind == "tool" { "tool_use" } else { kind };
    row["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter(|(_, block)| block["type"] == raw_kind)
        .nth(ordinal)
        .map(|(index, _)| index as u64)
        .ok_or(ObservationError::Transcript)
}

fn antigravity_user(raw: &str) -> Option<&str> {
    let body = raw.strip_prefix("<USER_REQUEST>\n")?;
    let (text, metadata) = body.split_once("\n</USER_REQUEST>")?;
    // The verified formatter appends native metadata after this wrapper. Ambiguous literal
    // wrapper tags cannot be stripped safely and therefore never receive a guessed alias.
    if text.contains("<USER_REQUEST>") || metadata.contains("USER_REQUEST") {
        return None;
    }
    if !metadata.is_empty()
        && !metadata.starts_with("\n<ADDITIONAL_METADATA>\n")
        && !metadata.starts_with("\n<USER_SETTINGS_CHANGE>\n")
    {
        return None;
    }
    Some(text)
}

#[cfg(test)]
mod tests;
