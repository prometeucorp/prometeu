//! Canonical event reactions. Vendor usage adaptation and feature effects stay behind ports.
use super::SessionService;
use crate::board::{Note, Status};
use crate::conversation::{agent_activity, work::Work};
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
    fn observe_work(&self, session: &str, event: &Value) -> bool {
        lock(self.work)
            .entry(session.into())
            .or_default()
            .observe(event)
    }

    pub fn react(&self, id: &str, frame: &Value, ready: &AtomicBool) -> bool {
        // A resumed main turn invalidates the terminal previously held by its children.
        if agent_activity(frame) {
            self.observe_work(id, frame);
        }
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
            Some("assistant.block") => {
                let note = match frame["block"]["kind"].as_str() {
                    Some("tool") => Note::Set(activity(&frame["block"])),
                    _ => Note::Keep,
                };
                self.sessions.update(id, Some(Status::Rodando), note, None);
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
            Some("turn.completed") => {
                self.actions.completed(
                    id,
                    frame["outcome"] == "error" || frame["outcome"] == "interrupted",
                );
                let settled = self.observe_work(id, frame);
                let status = match settled {
                    true => Status::Pronta,
                    false => Status::Rodando,
                };
                self.sessions
                    .update(id, Some(status), Note::Clear, self.context.tokens(id));
                return settled;
            }
            Some("background.changed") if self.observe_work(id, frame) => {
                self.sessions.update(
                    id,
                    Some(Status::Pronta),
                    Note::Clear,
                    self.context.tokens(id),
                );
                return true;
            }
            _ => {}
        }
        false
    }
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
