//! Bounded canonical projection from one authenticated immutable Cursor store snapshot.
//! Source occurrence and blob version are private: public V1 frames retain their own identity.
use super::*;
use prometeu_core::conversation::event;
use serde_json::json;
use std::collections::VecDeque;

const RECORD_LIMIT: usize = 64;
const BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct CursorCapture {
    pub occurrence: String,
    pub version: String,
    pub checkpoint: Value,
    /// A checkpoint may have no public content. It never needs a fabricated V1 event.
    pub frame: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorProjectionBatch {
    pub records: Vec<CursorCapture>,
    /// Every referenced turn and step in this snapshot has been visited.
    pub finished: bool,
    /// Every visited content kind is supported; false never authorizes completion.
    pub complete: bool,
}

/// Identity of the last fully decoded agent turn. This is not a completion signal: some pinned
/// CLI paths omit run records, and native Stop hooks still run after the last checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorTailTurn {
    pub key: CursorTurnKey,
    pub version: BlobId,
    pub run: Option<CursorRun>,
    pub root_run: Option<CursorRun>,
}

pub struct CursorProjection {
    native_session: String,
    root: CursorRoot,
    turn_index: usize,
    active: Option<ActiveTurn>,
    deferred: VecDeque<CursorCapture>,
    at: u64,
    complete: bool,
    tail_turn: Option<CursorTailTurn>,
}

struct ActiveTurn {
    version: BlobId,
    key: CursorTurnKey,
    steps: Vec<BlobId>,
    step_index: usize,
    message: String,
    run: Option<CursorRun>,
}

impl CursorProjection {
    pub fn new(native_session: &str, root: CursorRoot, at: u64) -> Result<Self, StoreError> {
        if !valid_id(native_session) {
            return Err(StoreError::Identity);
        }
        if root.turns.len() > REFERENCE_LIMIT {
            return Err(StoreError::Limit);
        }
        Ok(Self {
            native_session: native_session.into(),
            root,
            turn_index: 0,
            active: None,
            deferred: VecDeque::new(),
            at,
            complete: true,
            tail_turn: None,
        })
    }

    pub fn revision(&self) -> String {
        self.root.version.hex()
    }

    pub fn tail_turn(&self) -> Option<CursorTailTurn> {
        (self.complete
            && self.active.is_none()
            && self.turn_index == self.root.turns.len()
            && self.deferred.is_empty())
        .then(|| self.tail_turn.clone())
        .flatten()
    }

    /// Payload memory is bounded independently of total history. The source must remain the
    /// same copied snapshot until `finished`; a host acknowledges each returned batch durably.
    pub fn next(
        &mut self,
        source: &dyn BlobSource,
        schema: CursorStoreSchema,
    ) -> Result<CursorProjectionBatch, StoreError> {
        let mut records = Vec::new();
        let mut bytes = 0;
        while records.len() < RECORD_LIMIT {
            let record = match self.deferred.pop_front() {
                Some(record) => Some(record),
                None => self.record(source, schema)?,
            };
            let Some(record) = record else { break };
            let size = record.checkpoint.to_string().len()
                + record
                    .frame
                    .as_ref()
                    .map_or(0, |frame| frame.to_string().len());
            if size > BATCH_BYTES {
                return Err(StoreError::Limit);
            }
            if size > BATCH_BYTES - bytes {
                self.deferred.push_front(record);
                break;
            }
            bytes += size;
            records.push(record);
        }
        Ok(CursorProjectionBatch {
            records,
            finished: self.turn_index == self.root.turns.len()
                && self.active.is_none()
                && self.deferred.is_empty(),
            complete: self.complete,
        })
    }

    fn record(
        &mut self,
        source: &dyn BlobSource,
        schema: CursorStoreSchema,
    ) -> Result<Option<CursorCapture>, StoreError> {
        if let Some(active) = self.active.as_mut() {
            if let Some(id) = active.steps.get(active.step_index).copied() {
                let index = active.step_index;
                let step = step(source, id, schema)?;
                active.step_index += 1;
                if matches!(step.content, CursorStepContent::Tool { .. }) {
                    match tool(source, &step, schema) {
                        Ok(tool) => {
                            let occurrence = format!("{}:step:{index}", active.message);
                            let mut checkpoint = json!({"schema":1,"kind":"tool","messageId":active.message,"index":index,"userMessageId":active.key.user_message_id,"requestId":active.key.request_id,"runId":active.key.run_id,"toolId":tool.id});
                            let frame = event(
                                "assistant.block",
                                step.started_at_ms.unwrap_or(self.at),
                                json!({"messageId":active.message,"index":index,"block":{"kind":"tool","id":tool.id,"name":tool.name,"input":tool.input}}),
                            );
                            let record = CursorCapture {
                                occurrence: format!("{occurrence}:tool"),
                                version: id.hex(),
                                checkpoint: checkpoint.clone(),
                                frame: Some(frame),
                            };
                            if let Some(result) = tool.result {
                                checkpoint["kind"] = json!("result");
                                self.deferred.push_back(CursorCapture {
                                    occurrence: format!("{occurrence}:result"), version: id.hex(), checkpoint,
                                    frame: Some(event("tool.completed", step.completed_at_ms.unwrap_or(self.at), json!({"toolId":tool.id,"output":result.output,"error":result.error,"background":result.background}))),
                                });
                            }
                            return Ok(Some(record));
                        }
                        Err(StoreError::Unsupported) => {}
                        Err(error) => return Err(error),
                    }
                }
                let (kind, block) = match step.content {
                    CursorStepContent::Assistant(text) => {
                        ("text", Some(json!({"kind":"text","text":text})))
                    }
                    CursorStepContent::Thinking(text) => {
                        ("thinking", Some(json!({"kind":"thinking","text":text})))
                    }
                    CursorStepContent::Tool { .. } => {
                        self.complete = false;
                        ("unsupported_tool", None)
                    }
                };
                let occurrence = format!("{}:step:{index}", active.message);
                return Ok(Some(CursorCapture {
                    occurrence,
                    version: id.hex(),
                    checkpoint: json!({"schema":1,"kind":kind,"messageId":active.message,"index":index,"userMessageId":active.key.user_message_id,"requestId":active.key.request_id,"runId":active.key.run_id}),
                    frame: block.map(|block| {
                        event(
                            "assistant.block",
                            step.started_at_ms.unwrap_or(self.at),
                            json!({"messageId":active.message,"index":index,"block":block}),
                        )
                    }),
                }));
            }
            let active = self.active.take().ok_or(StoreError::Malformed)?;
            if self.turn_index == self.root.turns.len() {
                self.tail_turn = Some(CursorTailTurn {
                    key: active.key.clone(),
                    version: active.version,
                    run: active.run.clone(),
                    root_run: self.root.last_run.clone(),
                });
            }
            return Ok(Some(CursorCapture {
                occurrence: format!("{}:checkpoint", active.message),
                version: active.version.hex(),
                checkpoint: json!({"schema":1,"kind":"turn","messageId":active.message,"steps":active.steps.len(),"userMessageId":active.key.user_message_id,"requestId":active.key.request_id,"runId":active.key.run_id,"status":active.run.map(|run| match run.status { CursorRunStatus::Unknown=>"unknown", CursorRunStatus::Running=>"running", CursorRunStatus::Completed=>"completed", CursorRunStatus::Failed=>"failed", CursorRunStatus::Interrupted=>"interrupted" })}),
                frame: None,
            }));
        }
        let Some(id) = self.root.turns.get(self.turn_index).copied() else {
            return Ok(None);
        };
        let turn = turn(source, id, schema)?;
        self.turn_index += 1;
        match turn {
            CursorTurn::Shell { version, .. } => {
                self.complete = false;
                Ok(Some(CursorCapture {
                    occurrence: format!("cursor:shell:{}", version.hex()),
                    version: version.hex(),
                    checkpoint: json!({"schema":1,"kind":"unsupported_shell"}),
                    frame: None,
                }))
            }
            CursorTurn::Agent {
                version,
                key,
                user,
                steps,
                run,
            } => {
                let message = identity(
                    "turn",
                    &self.native_session,
                    &json!([key.user_message_id, key.request_id, key.run_id]),
                );
                self.active = Some(ActiveTurn {
                    version,
                    key,
                    steps,
                    step_index: 0,
                    message,
                    run,
                });
                let human = !user.simulated && user.sent_by_agent.is_none();
                if !human {
                    self.complete = false;
                }
                let content = json!([{"kind":"text","text":user.text}]);
                Ok(Some(CursorCapture {
                    occurrence: identity("user", &self.native_session, &json!(user.message_id)),
                    version: user.version.hex(),
                    checkpoint: json!({"schema":1,"kind":if human {"user"} else {"unsupported_synthetic_user"},"nativeMessageId":user.message_id,"contentHash":digest(content.to_string().as_bytes())}),
                    frame: human.then(|| {
                        event(
                            "user.message",
                            user.started_at_ms.unwrap_or(self.at),
                            json!({"content":content}),
                        )
                    }),
                }))
            }
        }
    }
}

fn identity(kind: &str, session: &str, key: &Value) -> String {
    format!(
        "cursor:{kind}:{}",
        digest(json!([session, key]).to_string().as_bytes())
    )
}
fn digest(bytes: &[u8]) -> String {
    BlobId(Sha256::digest(bytes).into()).hex()
}
