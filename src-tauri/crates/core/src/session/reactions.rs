//! Canonical event reactions. Vendor usage adaptation and feature effects stay behind ports.
use super::SessionService;
use crate::board::{Note, Status};
use crate::conversation::work::Work;
use crate::error::{code, with_args};
use crate::lock::lock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

pub trait SessionContext {
    fn tokens(&self, session: &str) -> Option<u64>;
}
pub trait SessionActions {
    fn completed(&self, session: &str, failed: bool);
}
pub trait SessionUsage {
    fn observe(&self, event: &Value);
}

pub struct SessionReactions<'a> {
    pub sessions: &'a SessionService<'a>,
    pub work: &'a Mutex<HashMap<String, Work>>,
    pub context: &'a dyn SessionContext,
    pub actions: &'a dyn SessionActions,
    pub usage: &'a dyn SessionUsage,
}

impl SessionReactions<'_> {
    pub fn react(&self, id: &str, frame: &Value, ready: &AtomicBool) -> bool {
        let (settled, status) = {
            let mut work = lock(self.work);
            let work = work.entry(id.into()).or_default();
            let waited = work.waiting();
            let settled = work.observe(frame);
            (settled, next_status(frame, settled, waited, work))
        };
        match frame["type"].as_str() {
            Some("session.state") if frame["state"] == "starting" => {
                lock(self.work).remove(id);
            }
            Some("commands.updated") if !ready.swap(true, Ordering::Relaxed) => {
                self.sessions.ready_now(id)
            }
            Some("usage.updated") => self.usage.observe(frame),
            Some("context.updated") => {
                {
                    let mut board = lock(self.sessions.board);
                    if let Some(tab) = board.tab_mut(id) {
                        tab.context_window = frame["window"].as_u64().filter(|window| *window > 0);
                        if frame["used"].as_u64() == Some(0) {
                            tab.context_tokens = Some(0);
                        }
                    }
                }
                self.sessions
                    .update(id, None, Note::Keep, frame["used"].as_u64())
            }
            Some("session.identity") => {
                if let Some(identity) = frame["providerSession"].as_str() {
                    self.sessions.remember_session(id, identity);
                }
            }
            // Any accepted message moves an interrupted conversation on, Continue included.
            Some("user.message") => {
                let resumed = lock(self.sessions.board)
                    .tab_mut(id)
                    .is_some_and(|tab| std::mem::take(&mut tab.interrupted));
                if resumed {
                    self.sessions.update(id, None, Note::Keep, None);
                }
            }
            Some("execution.observed") => {
                let note = match (status, frame["state"].as_str()) {
                    (Some(Status::Querendo), Some(state)) if state != "blocked" => Note::Keep,
                    _ => match frame["detail"].as_str().filter(|detail| !detail.is_empty()) {
                        Some(detail) => Note::Set(detail.into()),
                        None => Note::Clear,
                    },
                };
                if matches!(frame["state"].as_str(), Some("idle" | "unknown")) {
                    self.sessions.observe_state(id, status, note);
                } else {
                    self.sessions.update(id, status, note, None);
                }
            }
            // Output while a request is open leaves the tab waiting and the request's note.
            Some("assistant.block") if status == Some(Status::Rodando) => {
                let note = match frame["block"]["kind"].as_str() {
                    Some("tool") => Note::Set(activity(&frame["block"])),
                    _ => Note::Keep,
                };
                self.sessions.update(id, status, note, None);
            }
            Some("request.opened") => {
                let note = match frame["kind"].as_str() {
                    Some("question") => frame["input"]["questions"][0]["question"]
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| code("note.question")),
                    Some("plan") => code("note.plan"),
                    _ => match frame["tool"].as_str() {
                        Some(tool) => with_args("note.permission", &[("tool", tool.into())]),
                        None => code("note.permissionAny"),
                    },
                };
                self.sessions
                    .update(id, Some(Status::Querendo), Note::Set(note), None);
            }
            Some("request.closed") if status == Some(Status::Rodando) => {
                self.sessions.update(id, status, Note::Clear, None);
            }
            Some("turn.completed") => {
                self.actions.completed(
                    id,
                    frame["outcome"] == "error" || frame["outcome"] == "interrupted",
                );
                let note = match status {
                    Some(Status::Querendo) => Note::Keep,
                    _ => Note::Clear,
                };
                self.sessions
                    .update(id, status, note, self.context.tokens(id));
                return settled;
            }
            Some("background.changed") if settled => {
                self.sessions
                    .update(id, status, Note::Clear, self.context.tokens(id));
                return true;
            }
            _ => {}
        }
        false
    }
}

/// The status a canonical event moves its tab to, after `work` observed it; `None` leaves the
/// status as it is. `waited` tells whether a request was open before the event. While one is open
/// the tab waits for its person, whatever else the agent streams, and the answer to the last one
/// lets the agent run again. The desktop reactions derive status here.
pub fn next_status(event: &Value, settled: bool, waited: bool, work: &Work) -> Option<Status> {
    let moved = match event["type"].as_str() {
        Some("execution.observed") => match event["state"].as_str() {
            Some("working") => Status::Rodando,
            Some("blocked") => Status::Querendo,
            Some("idle") if !work.busy() => Status::Pronta,
            Some("idle" | "unknown") => Status::Rodando,
            _ => return None,
        },
        Some("user.message" | "assistant.started" | "assistant.block") => Status::Rodando,
        Some("request.opened") => Status::Querendo,
        Some("request.closed") if waited => Status::Rodando,
        Some("turn.completed") if settled => Status::Pronta,
        Some("turn.completed") => Status::Rodando,
        Some("background.changed") if settled => Status::Pronta,
        _ => return None,
    };
    Some(match work.waiting() {
        true => Status::Querendo,
        false => moved,
    })
}

/// Build a compact activity label such as `Bash cd /Users/...`.
pub fn activity(block: &Value) -> String {
    let tool = block["name"].as_str().unwrap_or("");
    let input = &block["input"];
    let detail = [
        "command",
        "file_path",
        "pattern",
        "path",
        "prompt",
        "url",
        "query",
        "description",
    ]
    .iter()
    .find_map(|k| input[k].as_str())
    .unwrap_or("");
    let detail: String = match detail.chars().count() > 70 {
        true => detail.chars().take(69).collect::<String>() + "…",
        false => detail.to_string(),
    };
    format!("{tool} {detail}").trim().to_string()
}
