//! Observational native transcripts. Hosts prove file ownership through an authenticated hook,
//! perform bounded reads, and keep replay separate from live delivery and notifications.
use prometeu_core::conversation::event;
use serde_json::{json, Value};
use std::{collections::VecDeque, path::PathBuf};

mod images;
pub use images::InlineImageFormat;

pub const TRANSCRIPT_LINE_LIMIT: usize = 1024 * 1024;
pub const TRANSCRIPT_SNAPSHOT_LIMIT: usize = 32 * 1024 * 1024;
const RECORD_BATCH_LIMIT: usize = 65_536;
const PROMPT_LIMIT: usize = 64;
const PROMPT_BYTES_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptPhase {
    Replay,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptFormat {
    Codex,
    Antigravity,
    Cursor,
}

/// The host must obtain both identity and path from the same authenticated process hook. Neither
/// a filename guess nor a directory scan establishes this binding. No file is opened by this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptBinding {
    pub format: TranscriptFormat,
    pub native_session: String,
    pub transcript_path: PathBuf,
}

/// Errors intentionally contain no native payload, prompt, path or credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptError {
    InvalidBinding,
    IdentityMismatch,
    OffsetMismatch { expected: u64, received: u64 },
    SizeLimit,
    PromptLimit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptRecord {
    pub ordinal: u64,
    pub offset: u64,
    pub phase: TranscriptPhase,
    pub value: Value,
    /// Known inline image bytes were deliberately omitted, with null at their native fields.
    /// Text and metadata remain available. This is private framing metadata, not a V1 event.
    pub omitted_media: usize,
}

#[derive(Default)]
pub struct TailBatch {
    pub records: Vec<TranscriptRecord>,
    pub partial: bool,
    /// At least one oversized, malformed or non-object record was discarded since reset.
    pub loss: bool,
}

/// Incremental JSONL framing usable with the desktop's existing Claude Adapter as well. A host
/// may keep feeding an arbitrarily large file; only the unfinished line is retained here.
#[derive(Default)]
pub struct JsonlTail {
    next_offset: u64,
    line_offset: u64,
    ordinal: u64,
    pending: Vec<u8>,
    pending_phase: Option<TranscriptPhase>,
    oversized: bool,
    loss: bool,
    image_format: Option<InlineImageFormat>,
    images: Option<images::ImageFilter>,
}

impl JsonlTail {
    pub fn with_inline_images(format: InlineImageFormat) -> Self {
        Self {
            image_format: Some(format),
            images: Some(images::ImageFilter::new(format)),
            ..Self::default()
        }
    }

    pub fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub fn feed(
        &mut self,
        offset: u64,
        bytes: &[u8],
        phase: TranscriptPhase,
    ) -> Result<TailBatch, TranscriptError> {
        if offset != self.next_offset {
            return Err(TranscriptError::OffsetMismatch {
                expected: self.next_offset,
                received: offset,
            });
        }
        if bytes.len() > TRANSCRIPT_SNAPSHOT_LIMIT {
            return Err(TranscriptError::SizeLimit);
        }
        let next = offset
            .checked_add(bytes.len() as u64)
            .ok_or(TranscriptError::SizeLimit)?;
        let mut batch = TailBatch::default();
        for (index, &byte) in bytes.iter().enumerate() {
            if self.pending_phase.is_none() {
                self.pending_phase = Some(phase);
            }
            if phase == TranscriptPhase::Replay {
                self.pending_phase = Some(TranscriptPhase::Replay);
            }
            if byte == b'\n' {
                if batch.records.len() == RECORD_BATCH_LIMIT {
                    self.loss = true;
                } else if !self.oversized && !self.pending.trim_ascii().is_empty() {
                    let value = serde_json::from_slice::<Value>(&self.pending)
                        .ok()
                        .filter(Value::is_object);
                    if let Some(mut value) = value {
                        let omitted = self
                            .images
                            .as_ref()
                            .map_or(Ok(0), |filter| filter.finish(&mut value));
                        if let Ok(omitted_media) = omitted {
                            if value.to_string().len() <= TRANSCRIPT_LINE_LIMIT {
                                batch.records.push(TranscriptRecord {
                                    ordinal: self.ordinal,
                                    offset: self.line_offset,
                                    phase: self.pending_phase.unwrap_or(phase),
                                    value,
                                    omitted_media,
                                });
                            } else {
                                self.loss = true;
                            }
                        } else {
                            self.loss = true;
                        }
                    } else {
                        self.loss = true;
                    }
                }
                self.pending.clear();
                self.pending_phase = None;
                self.oversized = false;
                self.images = self.image_format.map(images::ImageFilter::new);
                self.ordinal += 1;
                self.line_offset = offset + index as u64 + 1;
            } else if !self.oversized {
                let valid = if let Some(filter) = &mut self.images {
                    filter.feed(byte, &mut self.pending).is_ok()
                } else {
                    self.pending.push(byte);
                    true
                };
                if !valid || self.pending.len() > TRANSCRIPT_LINE_LIMIT {
                    self.pending.clear();
                    self.oversized = true;
                    self.loss = true;
                }
            }
        }
        self.next_offset = next;
        batch.partial = self.pending_phase.is_some();
        batch.loss = self.loss;
        Ok(batch)
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct TranscriptBatch {
    pub replay: Vec<Value>,
    pub live: Vec<Value>,
    /// Replace this observer's previous native projection; never append a rewritten snapshot.
    /// Application-owned echoes and events are a separate host concern.
    pub replace: bool,
    pub partial: bool,
    pub loss: bool,
}

pub struct NativeTranscript {
    binding: TranscriptBinding,
    tail: JsonlTail,
    translator: Translator,
    // Keeping a bounded byte prefix allows exact rewrite detection without text hashes or guessing
    // from file size. Once this ceiling is exceeded, only append mode remains available.
    snapshot_bytes: Option<Vec<u8>>,
    has_snapshot: bool,
    pending_prompts: VecDeque<String>,
    pending_prompt_bytes: usize,
    record_observer: Option<RecordObserver>,
    projection_observer: Option<ProjectionObserver>,
}

type RecordObserver = Box<dyn FnMut(&TranscriptRecord) + Send>;
type ProjectionObserver = Box<dyn FnMut(&TranscriptRecord, &[Value]) + Send>;

fn native_tail(format: TranscriptFormat) -> JsonlTail {
    match format {
        TranscriptFormat::Codex => JsonlTail::with_inline_images(InlineImageFormat::Codex),
        _ => JsonlTail::default(),
    }
}

impl NativeTranscript {
    pub fn new(binding: TranscriptBinding) -> Result<Self, TranscriptError> {
        if binding.native_session.is_empty()
            || binding.native_session.len() > 256
            || binding.native_session.chars().any(char::is_control)
            || !binding.transcript_path.is_absolute()
        {
            return Err(TranscriptError::InvalidBinding);
        }
        Ok(Self {
            translator: Translator::new(binding.format, binding.native_session.clone()),
            tail: native_tail(binding.format),
            binding,
            snapshot_bytes: Some(Vec::new()),
            has_snapshot: false,
            pending_prompts: VecDeque::new(),
            pending_prompt_bytes: 0,
            record_observer: None,
            projection_observer: None,
        })
    }

    pub fn binding(&self) -> &TranscriptBinding {
        &self.binding
    }

    /// An optional native adapter can retain source provenance before canonical conversion.
    /// The observer never receives a record that failed this transcript's identity validation.
    pub fn set_record_observer(&mut self, observer: RecordObserver) {
        self.record_observer = Some(observer);
    }

    /// Preserve source identity with its translated frames before replay filtering. Rewrites
    /// keep the callback, allowing a durable consumer to reconcile occurrences independently.
    pub fn set_projection_observer(&mut self, observer: ProjectionObserver) {
        self.projection_observer = Some(observer);
    }

    pub fn next_offset(&self) -> u64 {
        self.tail.next_offset()
    }

    /// Register only an accepted Prometeu send, under the same ordering lock as its local echo.
    /// One exact next native user message consumes one expectation. Repeated real prompts survive.
    pub fn expect_prompt(&mut self, text: &str) -> Result<(), TranscriptError> {
        if self.pending_prompts.len() == PROMPT_LIMIT
            || text.len() > PROMPT_BYTES_LIMIT.saturating_sub(self.pending_prompt_bytes)
        {
            return Err(TranscriptError::PromptLimit);
        }
        self.pending_prompts.push_back(text.into());
        self.pending_prompt_bytes += text.len();
        Ok(())
    }

    pub fn append(
        &mut self,
        offset: u64,
        bytes: &[u8],
        phase: TranscriptPhase,
        at: u64,
    ) -> Result<TranscriptBatch, TranscriptError> {
        if self.translator.identity_failed {
            return Err(TranscriptError::IdentityMismatch);
        }
        let tail = self.tail.feed(offset, bytes, phase)?;
        if let Some(snapshot) = self.snapshot_bytes.as_mut() {
            if bytes.len() <= TRANSCRIPT_SNAPSHOT_LIMIT.saturating_sub(snapshot.len()) {
                snapshot.extend_from_slice(bytes);
            } else {
                self.snapshot_bytes = None;
            }
        }
        self.convert(tail, at)
    }

    /// A first snapshot is replay, even if the caller requests live delivery. An unchanged prefix
    /// permits appending new records. Any rewrite/truncation rebuilds the complete native projection
    /// as replay. Cursor and Antigravity must use this path because their writers can rewrite files.
    pub fn snapshot(
        &mut self,
        bytes: &[u8],
        phase: TranscriptPhase,
        at: u64,
    ) -> Result<TranscriptBatch, TranscriptError> {
        if self.translator.identity_failed {
            return Err(TranscriptError::IdentityMismatch);
        }
        if bytes.len() > TRANSCRIPT_SNAPSHOT_LIMIT {
            return Err(TranscriptError::SizeLimit);
        }
        let first = !self.has_snapshot && self.tail.next_offset() == 0;
        let prefix = self
            .snapshot_bytes
            .as_deref()
            .filter(|prefix| bytes.starts_with(prefix));
        if let Some(prefix) = prefix {
            let offset = prefix.len();
            self.has_snapshot = true;
            return self.append(
                offset as u64,
                &bytes[offset..],
                if first {
                    TranscriptPhase::Replay
                } else {
                    phase
                },
                at,
            );
        }
        self.tail = native_tail(self.binding.format);
        self.translator = Translator::new(self.binding.format, self.binding.native_session.clone());
        self.snapshot_bytes = Some(Vec::new());
        self.has_snapshot = true;
        // Rewrites can move or remove user rows. Expectations from the old stream must not hide
        // a later unrelated prompt with the same text.
        self.pending_prompts.clear();
        self.pending_prompt_bytes = 0;
        let mut batch = self.append(0, bytes, TranscriptPhase::Replay, at)?;
        batch.replace = true;
        Ok(batch)
    }

    fn convert(&mut self, tail: TailBatch, at: u64) -> Result<TranscriptBatch, TranscriptError> {
        let mut batch = TranscriptBatch {
            partial: tail.partial,
            loss: tail.loss || self.translator.loss,
            ..Default::default()
        };
        for record in tail.records {
            let events = self.translator.translate(&record, at)?;
            if let Some(observer) = self.record_observer.as_mut() {
                observer(&record);
            }
            if let Some(observer) = self.projection_observer.as_mut() {
                observer(&record, &events);
            }
            batch.loss |= self.translator.loss;
            for frame in events {
                if frame["type"] == "turn.completed" && (batch.partial || batch.loss) {
                    continue;
                }
                if record.phase == TranscriptPhase::Live {
                    let user_message = frame["type"] == "user.message";
                    let echoed = user_message
                        && self.pending_prompts.front().is_some_and(|prompt| {
                            frame["content"].as_array().is_some_and(|content| {
                                content.len() == 1
                                    && content[0]["kind"] == "text"
                                    && content[0]["text"].as_str() == Some(prompt.as_str())
                            })
                        });
                    if user_message {
                        if let Some(prompt) = self.pending_prompts.pop_front() {
                            self.pending_prompt_bytes -= prompt.len();
                        }
                        // Without a native client id, a mismatch cannot safely be attributed to
                        // an older send later. Prefer a duplicate echo to hiding a person's input.
                        if !echoed {
                            self.pending_prompts.clear();
                            self.pending_prompt_bytes = 0;
                        }
                    }
                    if !echoed {
                        batch.live.push(frame);
                    }
                } else {
                    batch.replay.push(frame);
                }
            }
        }
        // A later malformed/truncated record in the same batch also invalidates completion.
        if batch.loss {
            batch.live.retain(|frame| frame["type"] != "turn.completed");
            batch
                .replay
                .retain(|frame| frame["type"] != "turn.completed");
        }
        Ok(batch)
    }
}

struct Translator {
    format: TranscriptFormat,
    session: String,
    active_turn: Option<String>,
    loss: bool,
    identity_failed: bool,
}

impl Translator {
    fn new(format: TranscriptFormat, session: String) -> Self {
        Self {
            format,
            session,
            active_turn: None,
            loss: false,
            identity_failed: false,
        }
    }

    fn translate(
        &mut self,
        record: &TranscriptRecord,
        at: u64,
    ) -> Result<Vec<Value>, TranscriptError> {
        if self.identity_failed {
            return Err(TranscriptError::IdentityMismatch);
        }
        if self.format == TranscriptFormat::Codex && record.value["type"] == "session_meta" {
            if record.value["payload"]["id"].as_str() != Some(self.session.as_str()) {
                self.identity_failed = true;
                return Err(TranscriptError::IdentityMismatch);
            }
            return Ok(Vec::new());
        }
        let id = format!("native:{}:{}", self.session, record.ordinal);
        Ok(match self.format {
            TranscriptFormat::Codex => self.codex(record, &id, at),
            TranscriptFormat::Antigravity => self.antigravity(record, &id, at),
            TranscriptFormat::Cursor => self.cursor(record, &id, at),
        })
    }

    fn codex(&mut self, record: &TranscriptRecord, id: &str, at: u64) -> Vec<Value> {
        let row = &record.value;
        let payload = &row["payload"];
        match (row["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("task_started" | "turn_started")) => {
                let Some(turn) = payload["turn_id"].as_str().filter(|turn| !turn.is_empty()) else {
                    return vec![];
                };
                if record.phase == TranscriptPhase::Live {
                    self.active_turn = Some(turn.into());
                }
                vec![event("assistant.started", at, json!({"messageId": turn}))]
            }
            (Some("event_msg"), Some("task_complete" | "turn_complete")) => {
                let Some(turn) = payload["turn_id"].as_str() else {
                    return vec![];
                };
                let failed = payload["error"].is_object();
                self.complete(
                    record.phase,
                    Some(turn),
                    if failed { "error" } else { "ok" },
                    payload["error"]["message"].as_str().unwrap_or(""),
                    payload["duration_ms"].as_u64(),
                    at,
                )
            }
            (Some("event_msg"), Some("turn_aborted")) => self.complete(
                record.phase,
                payload["turn_id"].as_str(),
                "interrupted",
                "",
                None,
                at,
            ),
            (Some("event_msg"), Some("user_message")) => payload["message"]
                .as_str()
                .map(|text| vec![user(text, at)])
                .unwrap_or_default(),
            (Some("event_msg"), Some("exec_command_end")) => {
                let (Some(call), Some(output), Some(exit)) = (
                    payload["call_id"].as_str(),
                    payload["aggregated_output"].as_str(),
                    payload["exit_code"].as_i64(),
                ) else {
                    return vec![];
                };
                vec![tool_output(call, output, exit != 0, at)]
            }
            (Some("event_msg"), Some("patch_apply_end")) => {
                let (Some(call), Some(success)) =
                    (payload["call_id"].as_str(), payload["success"].as_bool())
                else {
                    return vec![];
                };
                let output = [payload["stdout"].as_str(), payload["stderr"].as_str()]
                    .into_iter()
                    .flatten()
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                vec![tool_output(call, &output, !success, at)]
            }
            (Some("event_msg"), Some("token_count")) => {
                let Some(used) = payload["info"]["last_token_usage"]["input_tokens"]
                    .as_u64()
                    .filter(|used| *used > 0)
                else {
                    return vec![];
                };
                vec![event(
                    "context.updated",
                    at,
                    json!({"used":used,"window":payload["info"]["model_context_window"].as_u64()}),
                )]
            }
            (Some("response_item"), Some("message")) if payload["role"] == "assistant" => {
                let message = payload["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .unwrap_or(id);
                let blocks = payload["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|part| {
                        (part["type"] == "output_text")
                            .then(|| part["text"].as_str())
                            .flatten()
                            .map(|text| json!({"kind":"text","text":text}))
                    })
                    .collect();
                self.blocks(record.phase, message, blocks, at)
            }
            (Some("response_item"), Some("function_call" | "custom_tool_call")) => {
                let (Some(call), Some(name)) =
                    (payload["call_id"].as_str(), payload["name"].as_str())
                else {
                    return vec![];
                };
                let input = payload["arguments"].as_str().and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({"input":payload["input"].as_str().or_else(||payload["arguments"].as_str()).unwrap_or("")}));
                self.blocks(
                    record.phase,
                    id,
                    vec![json!({"kind":"tool","id":call,"name":name,"input":input})],
                    at,
                )
            }
            // agent_message duplicates the response_item message; user response_items can contain
            // developer context, so only the explicit user_message event supplies the user echo.
            _ => vec![],
        }
    }

    fn cursor(&mut self, record: &TranscriptRecord, id: &str, at: u64) -> Vec<Value> {
        let row = &record.value;
        if row["type"] == "turn_ended" {
            let outcome = match row["status"].as_str() {
                Some("success") => "ok",
                Some("error") => "error",
                Some("aborted") => "interrupted",
                _ => return vec![],
            };
            return self.complete(
                record.phase,
                None,
                outcome,
                row["error"].as_str().unwrap_or(""),
                None,
                at,
            );
        }
        let content = row["message"]["content"].as_array();
        match row["role"].as_str() {
            Some("user") => {
                let text = content
                    .into_iter()
                    .flatten()
                    .filter_map(|part| {
                        (part["type"] == "text")
                            .then(|| part["text"].as_str())
                            .flatten()
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                if text.is_empty() {
                    vec![]
                } else {
                    vec![user(&text, at)]
                }
            }
            Some("assistant") => {
                let blocks = content.into_iter().flatten().enumerate().filter_map(|(index, part)| {
                    match part["type"].as_str() {
                        Some("text") => part["text"].as_str().map(|text| json!({"kind":"text","text":text})),
                        Some("tool_use") => part["name"].as_str().filter(|_|part["input"].is_object()).map(|name| {
                            json!({"kind":"tool","id":format!("{id}:tool:{index}"),"name":name,"input":part["input"]})
                        }),
                        _ => None,
                    }
                }).collect();
                self.blocks(record.phase, id, blocks, at)
            }
            _ => vec![],
        }
    }

    fn antigravity(&mut self, record: &TranscriptRecord, fallback: &str, at: u64) -> Vec<Value> {
        let row = &record.value;
        let truncated = |field: &str| {
            row["truncated_fields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|value| value == field))
        };
        if row["truncated_fields"]
            .as_array()
            .is_some_and(|fields| !fields.is_empty())
        {
            self.loss = true;
        }
        let id = row["step_index"]
            .as_u64()
            .map(|step| format!("native:{}:step:{step}", self.session))
            .unwrap_or_else(|| fallback.into());
        match (row["source"].as_str(), row["type"].as_str()) {
            (Some("USER_EXPLICIT"), Some("USER_INPUT")) if !truncated("content") => row["content"]
                .as_str()
                .map(|text| vec![user(text, at)])
                .unwrap_or_default(),
            (Some("MODEL"), Some("PLANNER_RESPONSE")) => {
                let mut blocks = Vec::new();
                for (field, kind) in [("thinking", "thinking"), ("content", "text")] {
                    if !truncated(field) {
                        if let Some(text) = row[field].as_str().filter(|text| !text.is_empty()) {
                            blocks.push(json!({"kind":kind,"text":text}));
                        }
                    }
                }
                if !truncated("tool_calls") {
                    for (index, call) in row["tool_calls"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        if let Some(name) =
                            call["name"].as_str().filter(|_| call["args"].is_object())
                        {
                            blocks.push(json!({"kind":"tool","id":format!("{id}:tool:{index}"),"name":name,"input":call["args"]}));
                        }
                    }
                }
                self.blocks(record.phase, &id, blocks, at)
            }
            // DONE is a step status, not a turn outcome. GENERIC tool output has no trustworthy
            // call identity in this format; correlating it by adjacency would invent evidence.
            _ => vec![],
        }
    }

    fn blocks(
        &mut self,
        phase: TranscriptPhase,
        id: &str,
        blocks: Vec<Value>,
        at: u64,
    ) -> Vec<Value> {
        if phase == TranscriptPhase::Live && !blocks.is_empty() && self.active_turn.is_none() {
            self.active_turn = Some(id.into());
        }
        blocks
            .into_iter()
            .enumerate()
            .map(|(index, block)| {
                event(
                    "assistant.block",
                    at,
                    json!({"messageId":id,"index":index,"block":block}),
                )
            })
            .collect()
    }

    fn complete(
        &mut self,
        phase: TranscriptPhase,
        turn: Option<&str>,
        outcome: &str,
        message: &str,
        duration: Option<u64>,
        at: u64,
    ) -> Vec<Value> {
        if phase != TranscriptPhase::Live
            || self.loss
            || self.active_turn.is_none()
            || turn.is_some_and(|turn| self.active_turn.as_deref() != Some(turn))
        {
            return vec![];
        }
        let message_id = self.active_turn.take();
        vec![event(
            "turn.completed",
            at,
            json!({"outcome":outcome,"message":message,"durationMs":duration,"costUsd":null,"messageId":message_id}),
        )]
    }
}

fn user(text: &str, at: u64) -> Value {
    event(
        "user.message",
        at,
        json!({"content":[{"kind":"text","text":text}]}),
    )
}

fn tool_output(id: &str, output: &str, error: bool, at: u64) -> Value {
    event(
        "tool.completed",
        at,
        json!({"toolId":id,"output":output,"error":error,"background":false}),
    )
}

#[cfg(test)]
mod tests;
