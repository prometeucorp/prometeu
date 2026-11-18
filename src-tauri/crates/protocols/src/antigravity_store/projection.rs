//! Stable public anchors from native step indices and explicit call identities.
use super::*;
use prometeu_core::conversation::event;
use prometeu_core::conversation::usage::{Measurement, Usage};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub occurrence: String,
    pub version: String,
    pub checkpoint: Value,
    pub frame: Option<Value>,
}
struct Owner {
    message: String,
    index: u64,
    user: String,
    call: ToolCall,
}
pub struct Projection {
    native: String,
    trajectory: String,
    user: Option<String>,
    tools: HashMap<String, Owner>,
    last: Option<u64>,
    usage: Vec<Usage>,
    models: BTreeSet<String>,
    maintenance: bool,
}
impl Projection {
    pub fn new(native: &str, trajectory: &str) -> Result<Self, Error> {
        if [native, trajectory]
            .into_iter()
            .any(|id| id.is_empty() || id.len() > 256 || id.chars().any(char::is_control))
        {
            return Err(Error::Identity);
        }
        Ok(Self {
            native: native.into(),
            trajectory: trajectory.into(),
            user: None,
            tools: HashMap::new(),
            last: None,
            usage: Vec::new(),
            models: BTreeSet::new(),
            maintenance: false,
        })
    }
    pub fn push(&mut self, step: &Step, at: u64) -> Result<Vec<Capture>, Error> {
        if self.last.is_some_and(|last| step.index <= last) {
            return Err(Error::Identity);
        }
        self.last = Some(step.index);
        let message = format!("native:antigravity:{}:step:{}", self.native, step.index);
        let record_user = match &step.content {
            Content::Tool { call, .. } => self.tools.get(&call.id).map(|owner| owner.user.clone()),
            _ => None,
        };
        let mut out = Vec::new();
        let mut capture = |occurrence: String,
                           mut checkpoint: Value,
                           frame: Option<Value>,
                           user: Option<&str>| {
            checkpoint["schema"] = json!(1);
            checkpoint["sourceScope"] = json!("record");
            checkpoint["trajectoryId"] = json!(self.trajectory);
            if let Some(user) = user {
                checkpoint["sourceUser"] = json!(user);
            }
            out.push(Capture {
                occurrence,
                version: step.version.clone(),
                checkpoint,
                frame,
            });
        };
        match &step.content {
            Content::User { text, .. } => {
                let occurrence = format!("{message}:user");
                self.user = Some(occurrence.clone());
                self.usage.clear();
                self.models.clear();
                self.maintenance = text == "/compact";
                let content = json!([{"kind":"text","text":text}]);
                capture(
                    occurrence,
                    json!({"kind":"user","maintenance":self.maintenance.then_some("compact"),"contentHash":format!("{:x}",Sha256::digest(content.to_string().as_bytes()))}),
                    Some(event("user.message", at, json!({"content":content}))),
                    None,
                );
            }
            Content::Assistant {
                text,
                thinking,
                calls,
                ..
            } => {
                let user = self.user.as_deref().ok_or(Error::Identity)?;
                for (index, kind, text) in [(0, "thinking", thinking), (1, "text", text)] {
                    if !text.is_empty() {
                        capture(
                            format!("{message}:block:{index}:{kind}"),
                            json!({"kind":kind,"messageId":message,"index":index}),
                            Some(event(
                                "assistant.block",
                                at,
                                json!({"messageId":message,"index":index,"block":{"kind":kind,"text":text}}),
                            )),
                            Some(user),
                        );
                    }
                }
                for (offset, call) in calls.iter().enumerate() {
                    if self.tools.len() >= 65_536 || self.tools.contains_key(&call.id) {
                        return Err(Error::Identity);
                    }
                    let index = 2 + offset as u64;
                    self.tools.insert(
                        call.id.clone(),
                        Owner {
                            message: message.clone(),
                            index,
                            user: user.into(),
                            call: call.clone(),
                        },
                    );
                    capture(
                        format!("{message}:block:{index}:tool"),
                        json!({"kind":"tool","messageId":message,"index":index,"toolId":call.id}),
                        Some(event(
                            "assistant.block",
                            at,
                            json!({"messageId":message,"index":index,"block":{"kind":"tool","id":call.id,"name":call.name,"input":call.input}}),
                        )),
                        Some(user),
                    );
                }
            }
            Content::Tool {
                call,
                result,
                error,
                ..
            } => {
                let owner = self.tools.get(&call.id).ok_or(Error::Identity)?;
                if owner.call.name != call.name {
                    return Err(Error::Identity);
                }
                let message = &owner.message;
                let index = owner.index;
                // The execution record may contain arguments edited in the native approval UI.
                if owner.call.input != call.input {
                    capture(
                        format!("{message}:block:{index}:tool"),
                        json!({"kind":"tool","messageId":message,"index":index,"toolId":call.id}),
                        Some(event(
                            "assistant.block",
                            at,
                            json!({"messageId":message,"index":index,"block":{"kind":"tool","id":call.id,"name":call.name,"input":call.input}}),
                        )),
                        Some(&owner.user),
                    );
                }
                if step.status.terminal() {
                    if !step.complete {
                        return Err(Error::Unsupported);
                    }
                    let output = result.as_ref().ok_or(Error::Malformed)?;
                    let output = output
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| output.to_string());
                    capture(
                        format!("{message}:block:{index}:result"),
                        json!({"kind":"result","messageId":message,"index":index,"toolId":call.id}),
                        Some(event(
                            "tool.completed",
                            at,
                            json!({"toolId":call.id,"output":output,"error":error,"background":false}),
                        )),
                        Some(&owner.user),
                    );
                }
            }
            Content::Checkpoint { summary, failed } => {
                if step.status == Status::Done && !failed {
                    if !summary.is_empty() {
                        capture(
                            format!("{message}:metadata:system.summary:0"),
                            json!({"kind":"metadata","messageId":message,"index":0,"frameType":"system.summary"}),
                            Some(event("system.summary", at, json!({"text":summary}))),
                            None,
                        );
                    }
                    capture(
                        format!("{message}:metadata:context.compacted:0"),
                        json!({"kind":"metadata","messageId":message,"index":0,"frameType":"context.compacted"}),
                        Some(event(
                            "context.compacted",
                            at,
                            json!({"before":null,"after":null}),
                        )),
                        None,
                    );
                }
            }
            Content::HiddenSystemMessage => {}
        }
        if let Some(usage) = &step.usage {
            if matches!(step.content, Content::Checkpoint { .. }) {
                capture(
                    format!("{message}:metadata:telemetry.usage:0"),
                    json!({"kind":"metadata","messageId":message,"index":0,"frameType":"telemetry.usage"}),
                    Some(event(
                        "telemetry.usage",
                        at,
                        json!({"measurement":Measurement {
                            usage: usage.clone(), observed_models: step.model.as_ref().map(|model| vec![model.clone()]), ..Default::default()
                        }}),
                    )),
                    None,
                );
            } else {
                if !matches!(step.content, Content::Assistant { .. }) {
                    return Err(Error::Malformed);
                }
                self.usage.push(usage.clone());
                if let Some(model) = &step.model {
                    self.models.insert(model.clone());
                }
                let sum = |field: fn(&Usage) -> Option<u64>| -> Result<Option<u64>, Error> {
                    self.usage
                        .iter()
                        .try_fold(Some(0u64), |sum, usage| match (sum, field(usage)) {
                            (Some(sum), Some(value)) => {
                                sum.checked_add(value).map(Some).ok_or(Error::Limit)
                            }
                            _ => Ok(None),
                        })
                };
                let measurement = Measurement {
                    usage: Usage {
                        input_tokens: sum(|u| u.input_tokens)?,
                        output_tokens: sum(|u| u.output_tokens)?,
                        cache_read_tokens: sum(|u| u.cache_read_tokens)?,
                        cache_write_tokens: sum(|u| u.cache_write_tokens)?,
                        reasoning_tokens: sum(|u| u.reasoning_tokens)?,
                        context_used: usage.input_tokens,
                        peak_context: self.usage.iter().filter_map(|u| u.input_tokens).max(),
                        model_calls: Some(self.usage.len() as u64),
                        ..Default::default()
                    },
                    observed_models: (!self.models.is_empty())
                        .then(|| self.models.iter().cloned().collect()),
                    ..Default::default()
                };
                capture(
                    format!("{message}:metadata:telemetry.usage:0"),
                    json!({"kind":"metadata","messageId":message,"index":0,"frameType":"telemetry.usage"}),
                    Some(event(
                        "telemetry.usage",
                        at,
                        json!({"measurement":measurement}),
                    )),
                    record_user.as_deref().or(self.user.as_deref()),
                );
            }
        }
        if step.status.terminal() {
            capture(
                format!("{message}:checkpoint"),
                json!({"kind":"turn","messageId":message,"status":"completed","sourceScope":"record","maintenance":(self.maintenance && matches!(step.content, Content::Checkpoint { .. })).then_some("compact")}),
                None,
                record_user.as_deref().or(self.user.as_deref()),
            );
        }
        for capture in &mut out {
            if capture.checkpoint["kind"] == "metadata" {
                let measurement = &capture.frame.as_ref().ok_or(Error::Malformed)?["measurement"];
                capture.version = format!(
                    "{:x}",
                    Sha256::digest(
                        [
                            capture.version.as_bytes(),
                            measurement.to_string().as_bytes()
                        ]
                        .concat()
                    )
                );
            }
        }
        Ok(out)
    }
}
/// A private projection of proven protobuf fields for the existing completion fence. It creates
/// neither a hook nor a public event, and never turns an individual DONE step into turn completion.
pub fn completion_record(step: &Step) -> Value {
    let (kind, source, text) = match &step.content {
        Content::User { text, .. } => ("USER_INPUT", "USER_EXPLICIT", Some(text)),
        Content::Assistant { text, .. } => ("PLANNER_RESPONSE", "MODEL", Some(text)),
        Content::Tool { .. } => ("GENERIC", "MODEL", None),
        Content::Checkpoint { .. } => ("CHECKPOINT", "SYSTEM", None),
        Content::HiddenSystemMessage => ("SYSTEM_MESSAGE", "SYSTEM", None),
    };
    let calls = match &step.content {
        Content::Assistant { calls, .. } => calls
            .iter()
            .map(|call| json!({"id":call.id,"name":call.name,"args":call.input}))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    json!({"step_index":step.index,"type":kind,"source":source,"status":if step.status==Status::Done{"DONE"}else{"OTHER"},"content":text,"tool_calls":calls,"truncated_fields":if step.complete{vec![]}else{vec!["source"]}})
}
#[cfg(test)]
mod tests {
    use super::*;
    fn step(index: u64, content: Content) -> Step {
        Step {
            index,
            version: format!("v{index}"),
            status: Status::Done,
            source: 2,
            content,
            complete: true,
            usage: None,
            model: None,
        }
    }
    #[test]
    fn native_compact_checkpoint_is_metadata_with_causal_maintenance_identity() {
        let mut projection = Projection::new("session", "trajectory").unwrap();
        let input = projection
            .push(
                &step(
                    0,
                    Content::User {
                        text: "/compact".into(),
                        client_id: None,
                    },
                ),
                1,
            )
            .unwrap();
        assert_eq!(input[0].checkpoint["maintenance"], "compact");
        assert!(input.last().unwrap().checkpoint["maintenance"].is_null());
        let mut checkpoint = step(
            1,
            Content::Checkpoint {
                summary: "Native summary".into(),
                failed: false,
            },
        );
        checkpoint.usage = Some(Usage {
            input_tokens: Some(1200),
            output_tokens: Some(200),
            ..Default::default()
        });
        let rows = projection.push(&checkpoint, 2).unwrap();
        assert_eq!(rows.last().unwrap().checkpoint["maintenance"], "compact");
        assert_eq!(
            rows.last().unwrap().checkpoint["sourceUser"],
            input[0].occurrence
        );
        let frames = rows
            .iter()
            .filter_map(|row| row.frame.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            frames
                .iter()
                .map(|frame| frame["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["system.summary", "context.compacted", "telemetry.usage"]
        );
        assert!(rows
            .iter()
            .filter(|row| row.frame.is_some())
            .all(|row| row.checkpoint["sourceUser"].is_null()));
        let next = projection
            .push(
                &step(
                    2,
                    Content::User {
                        text: "Task".into(),
                        client_id: None,
                    },
                ),
                3,
            )
            .unwrap();
        assert!(next[0].checkpoint["maintenance"].is_null());
        let mut failed = Projection::new("session", "trajectory").unwrap();
        checkpoint.content = Content::Checkpoint {
            summary: "Partial".into(),
            failed: true,
        };
        checkpoint.usage = None;
        assert!(failed
            .push(&checkpoint, 1)
            .unwrap()
            .iter()
            .all(|row| row.frame.is_none()));
    }
    #[test]
    fn explicit_call_id_links_nonadjacent_result_and_preserves_repeated_text_and_indices() {
        let user = step(
            0,
            Content::User {
                text: "Same".into(),
                client_id: None,
            },
        );
        let call = ToolCall {
            id: "call-1".into(),
            name: "run_command".into(),
            input: json!({"CommandLine":"printf MARKER"}),
        };
        let answer = step(
            1,
            Content::Assistant {
                text: "Same".into(),
                thinking: String::new(),
                calls: vec![call.clone()],
                stop_reason: Some(2),
            },
        );
        let result = step(
            8,
            Content::Tool {
                call,
                result: Some(json!("MARKER")),
                full_output_uri: None,
                error: false,
            },
        );
        let mut p = Projection::new("session", "trajectory").unwrap();
        let u = p.push(&user, 1).unwrap();
        let a = p.push(&answer, 1).unwrap();
        p.push(
            &step(
                7,
                Content::User {
                    text: "New prompt".into(),
                    client_id: None,
                },
            ),
            1,
        )
        .unwrap();
        let r = p.push(&result, 1).unwrap();
        assert!(u
            .iter()
            .chain(&a)
            .chain(&r)
            .all(|capture| capture.checkpoint["sourceScope"] == "record"));
        assert!(r
            .iter()
            .all(|capture| capture.checkpoint["sourceUser"] == u[0].occurrence));
        assert_ne!(u[0].occurrence, a[0].occurrence);
        assert_eq!(r[0].checkpoint["sourceUser"], u[0].occurrence);
        assert_eq!(r[0].checkpoint["messageId"], a[0].checkpoint["messageId"]);
        assert_eq!(r[0].frame.as_ref().unwrap()["toolId"], "call-1");
        assert!(r.iter().all(|c| c
            .frame
            .as_ref()
            .is_none_or(|f| f["type"] != "turn.completed")));
        let mut repeated = answer.clone();
        repeated.index = 10;
        let Content::Assistant { calls, .. } = &mut repeated.content else {
            panic!()
        };
        calls[0].id = "call-2".into();
        let second = p.push(&repeated, 1).unwrap();
        assert_ne!(a[0].occurrence, second[0].occurrence);
        let mut rewrite = Projection::new("session", "trajectory").unwrap();
        rewrite.push(&user, 1).unwrap();
        let mut changed = answer;
        changed.version = "new-version".into();
        let new = rewrite.push(&changed, 1).unwrap();
        assert_eq!(a[0].occurrence, new[0].occurrence);
        assert_ne!(a[0].version, new[0].version);
    }
    #[test]
    fn result_without_explicit_planner_call_is_not_linked_by_position() {
        let mut p = Projection::new("session", "trajectory").unwrap();
        let result = step(
            2,
            Content::Tool {
                call: ToolCall {
                    id: "missing".into(),
                    name: "tool".into(),
                    input: json!({}),
                },
                result: Some(json!("text")),
                full_output_uri: None,
                error: false,
            },
        );
        assert_eq!(p.push(&result, 1), Err(Error::Identity));
    }
}
