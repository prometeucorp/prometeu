//! Explicit replies to one authenticated Claude PermissionRequest invocation.
//! The native dialog races this hook; delivery never proves which participant won.
use crate::{
    conversation::{event, now},
    i18n,
    lock::lock,
    state::ProviderId,
    terminal_agent_hooks::{NativeHookEvent, NativeHookKind},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

pub(crate) const WAIT: Duration = Duration::from_secs(300);
const LIMIT: usize = 16;
const DELIVERY_WAIT: Duration = Duration::from_secs(3);

struct Request {
    tool: String,
    input: Value,
    sender: mpsc::SyncSender<Delivery>,
    claimed: bool,
}

#[derive(Default)]
struct State {
    closed: bool,
    requests: HashMap<String, Request>,
    events: VecDeque<Value>,
}

pub(crate) struct Decisions {
    nonce: String,
    state: Mutex<State>,
}

pub(crate) struct Pending {
    id: String,
    registry: Arc<Decisions>,
    pub(crate) receiver: mpsc::Receiver<Delivery>,
}

pub(crate) struct Delivery {
    pub(crate) output: Value,
    acknowledgement: mpsc::SyncSender<bool>,
}

impl Decisions {
    pub(crate) fn new(nonce: &str) -> Arc<Self> {
        Arc::new(Self {
            nonce: nonce.into(),
            state: Mutex::new(State::default()),
        })
    }

    pub(crate) fn open(self: &Arc<Self>, hook: &NativeHookEvent) -> Option<Pending> {
        if hook.provider != ProviderId::Claude
            || hook.kind != NativeHookKind::PermissionRequested
            || hook.process_nonce != self.nonce
        {
            return None;
        }
        let tool = hook.tool_name.as_deref().filter(|name| !name.is_empty())?;
        let input = hook.tool_input.as_ref()?.as_object()?.clone();
        let mut state = lock(&self.state);
        // A stalled consumer must leave subsequent native dialogs alone, without unbounded cards.
        if state.closed || state.requests.len() >= LIMIT || state.events.len() >= LIMIT * 2 {
            return None;
        }
        let id = format!("claude:{}:{}", self.nonce, uuid::Uuid::new_v4());
        let (sender, receiver) = mpsc::sync_channel(1);
        state.requests.insert(
            id.clone(),
            Request {
                tool: tool.into(),
                input: Value::Object(input.clone()),
                sender,
                claimed: false,
            },
        );
        state.events.push_back(event(
            "request.opened",
            now(),
            json!({"requestId":id,"kind":match tool {
                "AskUserQuestion" => "question", "ExitPlanMode" => "plan", _ => "approval"
            },"toolId":null,"tool":tool,"input":input}),
        ));
        Some(Pending {
            id,
            registry: self.clone(),
            receiver,
        })
    }

    pub(crate) fn drain(&self) -> Vec<Value> {
        lock(&self.state).events.drain(..).collect()
    }

    pub(crate) fn respond(&self, command: &Value) -> Result<Vec<Value>, String> {
        if command["type"] != "request.respond" {
            return Err(unavailable());
        }
        let id = command["requestId"].as_str().ok_or_else(unavailable)?;
        let (acknowledgement, received) = mpsc::sync_channel(1);
        {
            let mut state = lock(&self.state);
            let request = state.requests.get_mut(id).ok_or_else(unavailable)?;
            if request.claimed {
                return Err(unavailable());
            }
            let output = response(&request.tool, &request.input, &command["response"])?;
            request.claimed = true;
            request
                .sender
                .try_send(Delivery {
                    output,
                    acknowledgement,
                })
                .map_err(|_| unavailable())?;
        }
        // Only the helper's stdout acknowledgement counts as delivery. The native UI can still
        // have won; request.closed therefore says resolved, never an invented allow/deny result.
        match received.recv_timeout(DELIVERY_WAIT) {
            Ok(true) => Ok(vec![]),
            _ => Err(unavailable()),
        }
    }

    pub(crate) fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        for (id, _) in std::mem::take(&mut state.requests) {
            state.events.push_back(closed(&id, "cancelled"));
        }
    }

    pub(crate) fn withdraw(&self) {
        let mut state = lock(&self.state);
        for (id, _) in std::mem::take(&mut state.requests) {
            state.events.push_back(closed(&id, "cancelled"));
        }
    }

    fn finish(&self, id: &str, delivered: bool) {
        let mut state = lock(&self.state);
        if state.requests.remove(id).is_some() {
            state
                .events
                .push_back(closed(id, if delivered { "resolved" } else { "cancelled" }));
        }
    }
}

impl Pending {
    pub(crate) fn active(&self) -> bool {
        lock(&self.registry.state).requests.contains_key(&self.id)
    }

    pub(crate) fn deliver(&self, delivery: Delivery, delivered: bool) {
        self.registry.finish(&self.id, delivered);
        let _ = delivery.acknowledgement.try_send(delivered);
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.registry.finish(&self.id, false);
    }
}

fn response(tool: &str, input: &Value, response: &Value) -> Result<Value, String> {
    let decision = match response["outcome"].as_str() {
        Some("allow") if tool == "ExitPlanMode" => {
            json!({"behavior":"allow","updatedInput":input})
        }
        Some("allow") if tool != "AskUserQuestion" => json!({"behavior":"allow"}),
        Some("deny") => {
            json!({"behavior":"deny","message":response["message"].as_str().unwrap_or("Denied")})
        }
        Some("answer") if tool == "AskUserQuestion" => {
            let answers = response["answers"].as_object().ok_or_else(unavailable)?;
            let questions = input["questions"].as_array().ok_or_else(unavailable)?;
            if questions.is_empty()
                || answers.len() != questions.len()
                || questions.iter().any(|question| {
                    question["question"]
                        .as_str()
                        .is_none_or(|text| !answers.get(text).is_some_and(Value::is_string))
                })
            {
                return Err(unavailable());
            }
            let mut updated = input.clone();
            updated
                .as_object_mut()
                .ok_or_else(unavailable)?
                .insert("answers".into(), response["answers"].clone());
            json!({"behavior":"allow","updatedInput":updated})
        }
        _ => return Err(unavailable()),
    };
    Ok(json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":decision}}))
}

fn closed(id: &str, outcome: &str) -> Value {
    event(
        "request.closed",
        now(),
        json!({"requestId":id,"outcome":outcome}),
    )
}

fn unavailable() -> String {
    i18n::t("err.terminalAgent.unsupported")
}

#[cfg(test)]
mod smoke;
#[cfg(test)]
mod tests;
