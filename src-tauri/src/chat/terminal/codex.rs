//! One native RPC subscription feeds canonical content/control. Bootstrap uses the same native
//! item schema and IDs, so recovery only appends missing receipts and never replays reactions.
use super::*;
use crate::terminal_agent_observation::NativeCapture;
use prometeu_protocols::{codex::Link, terminal_transcript::TranscriptPhase};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) struct Attached {
    state: Mutex<Decoder>,
    active: Mutex<Option<String>>,
    native_idle: AtomicBool,
}
struct Decoder {
    link: Link,
    thread: String,
    messages: HashSet<String>,
    current_turn: Option<String>,
}
struct Decoded {
    frame: Option<Value>,
    record: Option<NativeCapture>,
}

impl Attached {
    pub(super) fn new(link: Link, thread: &str, native_idle: bool) -> Self {
        Self {
            state: Mutex::new(Decoder {
                link,
                thread: thread.into(),
                messages: HashSet::new(),
                current_turn: None,
            }),
            active: Mutex::new(None),
            native_idle: AtomicBool::new(native_idle),
        }
    }

    pub(super) fn native_idle(&self) -> bool {
        self.native_idle.load(Ordering::Acquire) && !self.active()
    }

    pub(super) fn active(&self) -> bool {
        lock(&self.active).is_some()
    }

    pub(super) fn send(&self, command: &Value) -> Result<Vec<Value>, String> {
        lock(&self.state).link.write(command)
    }
    pub(super) fn refresh_usage(&self) -> Result<(), String> {
        lock(&self.state).link.refresh_usage()
    }

    pub(super) fn replay(
        &self,
        history: &[Value],
        journal: &ingestion::JournalStore,
    ) -> Result<(), String> {
        let mut decoder = lock(&self.state);
        let thread = decoder.thread.clone();
        let mut records = Vec::new();
        for turn in history {
            let turn_id = turn["id"].as_str().ok_or_else(error)?;
            decoder.decode(
                &json!({"method":"turn/started","params":{"threadId":thread,"turn":turn}}),
            )?;
            for item in turn["items"].as_array().ok_or_else(error)? {
                for method in ["item/started", "item/completed"] {
                    if method == "item/completed" && !completed_item(item, turn) {
                        if let Some(record) = decoder.partial_text(item)? {
                            records.push(record);
                        }
                        continue;
                    }
                    records.extend(decoder.decode(&json!({"method":method,"params":{"threadId":thread,"turnId":turn_id,"item":item}}))?
                        .into_iter().filter_map(|decoded| decoded.record));
                }
            }
            if matches!(
                turn["status"].as_str(),
                Some("completed" | "failed" | "interrupted")
            ) {
                records.extend(decoder.decode(&json!({"method":"turn/completed","params":{"threadId":thread,"turn":turn}}))?
                    .into_iter().filter_map(|decoded| decoded.record));
            }
        }
        let batch = batch(records, TranscriptPhase::Replay);
        journal.replay(&batch)
    }

    pub(super) fn poll(
        &self,
        events: &mpsc::Receiver<Result<Value, String>>,
        transport: &Transport,
        pump: &super::super::Pump,
        context: &host::ConversationHost,
    ) -> Result<(), String> {
        self.poll_with(
            events,
            transport,
            pump,
            |record| {
                super::super::observe_native_record(
                    &context.app.state::<AppState>(),
                    &context.id,
                    &transport.generation,
                    record,
                )
            },
            || {
                if !context.setup_running() {
                    context.schedule();
                }
            },
        )
    }

    fn poll_with<T: prometeu_core::session::output::SessionTelemetry>(
        &self,
        events: &mpsc::Receiver<Result<Value, String>>,
        transport: &Transport,
        pump: &prometeu_core::session::pump::SessionPump<T>,
        mut accept_user: impl FnMut(&NativeCapture) -> Result<bool, String>,
        mut schedule: impl FnMut(),
    ) -> Result<(), String> {
        // A fast native user item must not precede the durable app echo/receipt it aliases.
        if lock(&transport.input).submission.is_some() {
            return Ok(());
        }
        loop {
            let raw = match events.try_recv() {
                Ok(event) => event?,
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => return Err(error()),
            };
            if raw["method"] == "turn/started"
                && raw["params"]["threadId"] == lock(&self.state).thread
            {
                *lock(&self.active) = Some(
                    raw["params"]["turn"]["id"]
                        .as_str()
                        .ok_or_else(error)?
                        .into(),
                );
                transport.ready.store(false, Ordering::Release);
            }
            if raw["params"]["threadId"] == lock(&self.state).thread {
                match raw["method"].as_str() {
                    Some("thread/status/changed") => self
                        .native_idle
                        .store(raw["params"]["status"]["type"] == "idle", Ordering::Release),
                    Some("turn/started" | "thread/closed") => {
                        self.native_idle.store(false, Ordering::Release)
                    }
                    Some(_) if raw.get("id").is_some() => {
                        self.native_idle.store(false, Ordering::Release)
                    }
                    _ => {}
                }
            }
            let decoded = lock(&self.state).decode(&raw)?;
            let journal = transport.journal().ok_or_else(error)?;
            for mut event in decoded {
                if let Some(record) = event.record.as_mut() {
                    let disposition = journal.disposition(record)?;
                    if record.checkpoint["kind"] == "user"
                        && disposition != ingestion::NativeDisposition::Duplicate
                    {
                        let mut input = lock(&transport.input);
                        if let Some(expected) = input.expected_prompts.pop_front() {
                            let text = user_text(record.frame.as_ref().ok_or_else(error)?);
                            if expected.text != text {
                                return Err(error());
                            }
                            record.input_binding = expected.id;
                        }
                    }
                    let batch = batch(vec![record.clone()], TranscriptPhase::Live);
                    journal.begin_stream_batch(&batch)?;
                    match disposition {
                        ingestion::NativeDisposition::Duplicate
                        | ingestion::NativeDisposition::Checkpoint => {
                            journal.commit_checkpoint(record)?;
                        }
                        ingestion::NativeDisposition::User => {
                            if accept_user(record)? {
                                let mut input = lock(&transport.input);
                                input.local_draft = None;
                                input.composer_seen = true;
                                input.interrupted = false;
                                transport.has_submitted.store(true, Ordering::Release);
                                lock(&transport.execution).accepted_input();
                            }
                        }
                        ingestion::NativeDisposition::Revision => {
                            let frame = record.frame.as_ref().ok_or_else(error)?;
                            pump.revise_committed(&frame.to_string(), |line, offset| {
                                journal.commit_record(record, Some(line), offset)
                            })?;
                        }
                        ingestion::NativeDisposition::Output => {
                            let frame = record.frame.as_ref().ok_or_else(error)?;
                            if pump.feed_committed(&frame.to_string(), |line, offset| {
                                journal.commit_record(record, Some(line), offset)
                            })? {
                                lock(&transport.execution).observe_canonical(frame);
                            }
                        }
                    }
                    journal.finish_batch(&batch)?;
                    transport
                        .ingestion_ready
                        .store(journal.caught_up(), Ordering::Release);
                } else if let Some(frame) = event.frame {
                    transport.feed_canonical(pump, frame);
                }
            }
            if !journal.healthy() {
                return Err(error());
            }
            if raw["method"] == "turn/completed"
                && raw["params"]["threadId"] == lock(&self.state).thread
            {
                let mut active = lock(&self.active);
                if active.as_deref() == raw["params"]["turn"]["id"].as_str() {
                    *active = None;
                    self.native_idle.store(true, Ordering::Release);
                }
                drop(active);
                if transport.ready() {
                    schedule();
                }
            }
        }
    }
}

fn completed_item(item: &Value, turn: &Value) -> bool {
    match item["status"].as_str() {
        Some("inProgress" | "pending" | "running") => false,
        Some("completed" | "failed" | "interrupted" | "declined" | "cancelled") => true,
        Some(_) => false,
        None => {
            item["completedAtMs"].is_u64()
                || matches!(
                    turn["status"].as_str(),
                    Some("completed" | "failed" | "interrupted")
                )
        }
    }
}

fn batch(records: Vec<NativeCapture>, phase: TranscriptPhase) -> NativeIngestionBatch {
    let id = uuid::Uuid::new_v4().to_string();
    NativeIngestionBatch {
        mode: if phase == TranscriptPhase::Live {
            crate::terminal_agent_observation::NativeIngestionMode::Append
        } else {
            crate::terminal_agent_observation::NativeIngestionMode::Snapshot
        },
        id: id.clone(),
        phase,
        source_revision: format!("codex:{id}"),
        records,
        finished: true,
        complete: true,
    }
}
fn error() -> String {
    i18n::t("err.terminalAgent.history")
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn user_text(frame: &Value) -> String {
    frame["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
fn user_frame(item: &Value) -> Result<Value, String> {
    let content = item["content"]
        .as_array()
        .ok_or_else(error)?
        .iter()
        .map(|part| match part["type"].as_str() {
            Some("text") => part["text"]
                .as_str()
                .map(|text| json!({"kind":"text","text":text}))
                .ok_or_else(error),
            Some("image" | "localImage") => {
                let name = part["path"]
                    .as_str()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .and_then(|name| name.to_str())
                    .unwrap_or("image");
                let media = part["image_url"]
                    .as_str()
                    .and_then(|url| url.strip_prefix("data:"))
                    .and_then(|url| url.split_once(';'))
                    .map(|(media, _)| media)
                    .unwrap_or("application/octet-stream");
                Ok(json!({"kind":"image","name":name,"mediaType":media}))
            }
            _ => Err(error()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(conversation::event(
        "user.message",
        item["completedAtMs"]
            .as_u64()
            .unwrap_or_else(conversation::now),
        json!({"content":content}),
    ))
}
impl Decoder {
    fn partial_text(&self, item: &Value) -> Result<Option<NativeCapture>, String> {
        let (kind, text) = match item["type"].as_str() {
            Some("agentMessage" | "plan") => {
                ("text", item["text"].as_str().unwrap_or("").to_owned())
            }
            Some("reasoning") => (
                "thinking",
                item["summary"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            _ => return Ok(None),
        };
        if text.is_empty() {
            return Ok(None);
        }
        let message = item["id"].as_str().ok_or_else(error)?;
        let frame = conversation::event(
            "assistant.block",
            item["startedAtMs"].as_u64().unwrap_or(0),
            json!({"messageId":message,"index":0,"block":{"kind":kind,"text":text}}),
        );
        let mut semantic = frame.clone();
        semantic.as_object_mut().ok_or_else(error)?.remove("at");
        Ok(Some(NativeCapture {
            occurrence: format!(
                "codex:item:{}:{kind}",
                digest(&format!("{}:{message}:0", self.thread))
            ),
            version: digest(&semantic.to_string()),
            checkpoint: json!({"kind":kind,"messageId":message,"index":0,"toolId":null}),
            frame: Some(frame),
            input_binding: None,
        }))
    }
    fn decode(&mut self, raw: &Value) -> Result<Vec<Decoded>, String> {
        if raw["params"]["threadId"]
            .as_str()
            .is_some_and(|id| id != self.thread)
        {
            // Link still consumes known subagent lifecycle, but never root content from a peer.
            return Ok(self
                .link
                .on_line(&raw.to_string())
                .into_iter()
                .map(|frame| Decoded {
                    frame: Some(frame),
                    record: None,
                })
                .collect());
        }
        let method = raw["method"].as_str().unwrap_or("");
        if (method.starts_with("item/")
            || method.starts_with("turn/")
            || method == "serverRequest/resolved")
            && raw["params"]["threadId"].as_str() != Some(self.thread.as_str())
        {
            return Err(error());
        }

        let item = &raw["params"]["item"];
        if method == "turn/started" {
            self.current_turn = Some(
                raw["params"]["turn"]["id"]
                    .as_str()
                    .ok_or_else(error)?
                    .into(),
            );
            self.messages.clear();
        }
        if method == "turn/completed"
            && self.current_turn.as_deref() != raw["params"]["turn"]["id"].as_str()
        {
            return Ok(Vec::new());
        }
        if method == "item/completed" && item["type"] == "userMessage" {
            let id = item["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(error)?;
            let frame = user_frame(item)?;
            let content_hash = digest(&frame["content"].to_string());
            let occurrence = format!("codex:user:{}", digest(&format!("{}:{id}", self.thread)));
            let record = NativeCapture {
                occurrence,
                version: content_hash.clone(),
                checkpoint: json!({"kind":"user","nativeMessageId":id,"clientId":item["clientId"],"contentHash":content_hash}),
                frame: Some(frame),
                input_binding: None,
            };
            return Ok(vec![Decoded {
                frame: None,
                record: Some(record),
            }]);
        }
        let frames = self.link.on_line(&raw.to_string());
        let mut output = Vec::new();
        for frame in frames {
            let kind = frame["type"].as_str().unwrap_or("");
            let durable = kind == "turn.completed"
                || kind == "tool.completed"
                || kind == "assistant.block"
                    && (method == "item/completed" || frame["block"]["kind"] == "tool");
            if !durable {
                output.push(Decoded {
                    frame: Some(frame),
                    record: None,
                });
                continue;
            }
            let (occurrence, checkpoint, version) = if kind == "turn.completed" {
                let turn = raw["params"]["turn"]["id"].as_str().ok_or_else(error)?;
                let status = raw["params"]["turn"]["status"].as_str().ok_or_else(error)?;
                (
                    format!(
                        "codex:completion:{}",
                        digest(&format!("{}:{turn}", self.thread))
                    ),
                    json!({"kind":"completion","nativeTurn":turn}),
                    status.to_owned(),
                )
            } else {
                let message = if kind == "tool.completed" {
                    frame["toolId"].as_str()
                } else {
                    frame["messageId"].as_str()
                }
                .ok_or_else(error)?;
                self.messages.insert(message.into());
                let index = frame["index"].as_u64().unwrap_or(0);
                let content_kind = if kind == "tool.completed" {
                    "result"
                } else {
                    frame["block"]["kind"].as_str().ok_or_else(error)?
                };
                let tool = if content_kind == "result" {
                    frame["toolId"].clone()
                } else {
                    frame["block"]["id"].clone()
                };
                let occurrence = format!(
                    "codex:item:{}:{content_kind}",
                    digest(&format!("{}:{message}:{index}", self.thread))
                );
                let mut semantic = frame.clone();
                semantic.as_object_mut().ok_or_else(error)?.remove("at");
                (
                    occurrence,
                    json!({"kind":content_kind,"messageId":message,"index":index,"toolId":tool}),
                    digest(&semantic.to_string()),
                )
            };
            output.push(Decoded {
                frame: None,
                record: Some(NativeCapture {
                    occurrence,
                    version,
                    checkpoint,
                    frame: Some(frame),
                    input_binding: None,
                }),
            });
        }
        if method == "turn/completed" {
            self.current_turn = None;
            for message in self.messages.drain() {
                let status = raw["params"]["turn"]["status"].as_str().ok_or_else(error)?;
                output.push(Decoded {
                    frame: None,
                    record: Some(NativeCapture {
                        occurrence: format!("{message}:checkpoint"),
                        version: status.into(),
                        checkpoint: json!({"kind":"turn","messageId":message,"status":status}),
                        frame: None,
                        input_binding: None,
                    }),
                });
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::real_proof;
