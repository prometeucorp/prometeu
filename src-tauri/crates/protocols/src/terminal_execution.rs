//! Pure execution observations. Completion needs explicit evidence and a host transcript barrier.
use crate::terminal_status::{
    ProgramStatusKind, ProgramStatusReport, ProgramStatusSnapshot, ProgramStatusState,
    ProgramStatusStore,
};
use prometeu_core::conversation::{agent_activity, event};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct CompletionTicket {
    activity: String,
    serial: u64,
    failed: bool,
    detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptBarrier {
    CaughtUp,
    Incomplete,
}

#[derive(Default)]
pub struct ExecutionProjection {
    pub events: Vec<Value>,
    pub completion: Option<CompletionTicket>,
}

/// Suppressing a duplicate completion suppresses public reactions, not a host's private capture
/// of new provider usage. Replace is used to merge independently observed background snapshots.
pub enum CanonicalDisposition {
    Forward,
    Replace(Value),
    DuplicateCompletion,
}

/// The native adapter supplies only facts its hook contract establishes. A stop callback is not
/// Finished: another provider hook may continue execution. Tool results are not child lifecycle.
pub enum ExecutionFact {
    Working {
        id: Option<String>,
        detail: Option<String>,
    },
    Blocked {
        id: Option<String>,
        kind: Option<ProgramStatusKind>,
        detail: Option<String>,
    },
    Finished {
        id: Option<String>,
        failed: bool,
        detail: Option<String>,
    },
    StopObserved,
}

pub struct ExecutionProjector {
    incarnation: String,
    store: ProgramStatusStore,
    cycle: u64,
    closed: bool,
    root_activity_stopped: bool,
    root_content_idle: bool,
    completed: bool,
    armed: bool,
    first_activity: bool,
    unclaimed_activity: bool,
    maintenance: bool,
    block_serial: u64,
    completion_serial: u64,
    block_ids: std::collections::BTreeMap<Option<String>, String>,
    pending: Option<CompletionTicket>,
    last_execution: Option<Value>,
    last_background: Vec<Value>,
    canonical_background: Vec<Value>,
}

impl ExecutionProjector {
    pub fn new(incarnation: &str) -> Option<Self> {
        if incarnation.is_empty()
            || incarnation.len() > 128
            || !incarnation
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return None;
        }
        Some(Self {
            incarnation: incarnation.into(),
            store: ProgramStatusStore::default(),
            cycle: 0,
            closed: false,
            root_activity_stopped: false,
            root_content_idle: false,
            completed: false,
            armed: false,
            first_activity: true,
            unclaimed_activity: false,
            maintenance: false,
            block_serial: 0,
            completion_serial: 0,
            block_ids: Default::default(),
            pending: None,
            last_execution: None,
            last_background: Vec::new(),
            canonical_background: Vec::new(),
        })
    }

    /// Exactly one accepted user submission, or a distinct native activity generation proven by
    /// the host, opens a new cycle. Repeated status reports alone cannot reopen a completed cycle.
    pub fn accepted_input(&mut self) {
        self.maintenance = false;
        // A hook can report first activity before the same input reaches the native transcript.
        // Claim that still-open cycle instead of invalidating its explicit activity or ticket.
        if self.unclaimed_activity && !self.closed && !self.root_content_idle {
            self.unclaimed_activity = false;
            return;
        }
        self.cycle = self.cycle.saturating_add(1);
        self.closed = false;
        self.root_activity_stopped = false;
        self.root_content_idle = false;
        self.completed = false;
        self.armed = false;
        self.first_activity = false;
        self.unclaimed_activity = false;
        self.pending = None;
        self.last_execution = None;
    }

    /// The host has observed a new native input begin work before its durable source arrives.
    /// The later source capture claims this provisional activity instead of opening it twice.
    pub fn observed_input(&mut self, at: u64) -> Vec<Value> {
        self.accepted_input();
        self.unclaimed_activity = true;
        self.armed = true;
        let mut events = self
            .observe_fact(
                ExecutionFact::Working {
                    id: None,
                    detail: None,
                },
                at,
            )
            .events;
        // A tool hook can have stored Working while the previous cycle was still closed.
        // Opening the new input must publish it even when the native state is unchanged.
        self.publish_execution(at, &mut events);
        events
    }

    /// A native user record can trail both its first activity and its explicit stop. Claim that
    /// provisional cycle and restore the observed state after the host captures the user echo.
    /// A newly submitted app input uses `accepted_input` instead and always opens a closed cycle.
    pub fn captured_input(&mut self, at: u64) -> Vec<Value> {
        if !self.unclaimed_activity || !self.root_content_idle {
            self.accepted_input();
            return Vec::new();
        }
        self.unclaimed_activity = false;
        let uncertain = self
            .last_execution
            .as_ref()
            .is_some_and(|last| last["state"] == "unknown");
        self.last_execution = None;
        let mut events = Vec::new();
        self.publish_state("idle", "root", None, at, &mut events);
        self.publish_execution(at, &mut events);
        if uncertain {
            self.publish_state("unknown", "root", None, at, &mut events);
        }
        events
    }

    /// A source-identified native maintenance command starts activity, not an assistant turn.
    pub fn accepted_maintenance(&mut self) {
        self.accepted_input();
        self.maintenance = true;
        self.armed = false;
        self.pending = None;
    }

    /// The host proves the command's terminal maintenance record is durably caught up. Current
    /// native idle/done evidence settles its observed state without an assistant completion.
    pub fn settle_maintenance(
        &mut self,
        barrier: TranscriptBarrier,
        at: u64,
    ) -> Option<Vec<Value>> {
        let snapshot = self.store.snapshot();
        if !self.maintenance
            || barrier != TranscriptBarrier::CaughtUp
            || snapshot.has_active_work()
            || !self.canonical_background.is_empty()
            || !snapshot.root.as_ref().is_some_and(|root| {
                matches!(
                    root.state,
                    ProgramStatusState::Idle | ProgramStatusState::Done | ProgramStatusState::Error
                )
            })
        {
            return None;
        }
        self.maintenance = false;
        self.closed = true;
        self.completed = true;
        self.armed = false;
        self.pending = None;
        let mut events = Vec::new();
        self.publish_state("idle", "root", snapshot.root.as_ref(), at, &mut events);
        Some(events)
    }

    pub fn snapshot(&self) -> ProgramStatusSnapshot {
        self.store.snapshot()
    }
    pub fn pending_completion(&self) -> Option<CompletionTicket> {
        self.pending.clone()
    }

    /// Explicit active-root -> idle ends observed activity, without proving a completed turn.
    /// Its source-bound content may still arrive after the status and must stay quiet until input.
    pub fn root_activity_stopped(&self) -> bool {
        self.root_activity_stopped
    }

    /// Root idle makes later source content quiet even when a preceding Done still awaits its
    /// independent completion proof. Activity or a new accepted input can make content live again.
    pub fn root_content_idle(&self) -> bool {
        self.root_content_idle
    }

    pub fn observe_fact(&mut self, fact: ExecutionFact, at: u64) -> ExecutionProjection {
        let (id, state, kind, message) = match fact {
            ExecutionFact::Working { id, detail } => {
                (id, ProgramStatusState::Working, None, detail)
            }
            ExecutionFact::Blocked { id, kind, detail } => {
                (id, ProgramStatusState::Blocked, kind, detail)
            }
            ExecutionFact::Finished { id, failed, detail } => (
                id,
                if failed {
                    ProgramStatusState::Error
                } else {
                    ProgramStatusState::Done
                },
                None,
                detail,
            ),
            ExecutionFact::StopObserved => return ExecutionProjection::default(),
        };
        self.observe(
            ProgramStatusReport {
                id,
                state,
                kind,
                message,
                app: None,
                title: None,
                progress: None,
            },
            at,
        )
    }

    pub fn observe(&mut self, report: ProgramStatusReport, at: u64) -> ExecutionProjection {
        let mut output = ExecutionProjection::default();
        let Some(transition) = self.store.apply(report) else {
            return output;
        };
        let root = transition.id.is_none();
        let became_idle = transition
            .previous
            .as_ref()
            .is_some_and(|before| active(before.state))
            && transition
                .current
                .as_ref()
                .is_some_and(|after| after.state == ProgramStatusState::Idle);
        let uncertain_removal =
            transition.removed.iter().any(|record| active(record.state)) || (!root && became_idle);
        if uncertain_removal || (root && transition.current.is_none()) {
            self.invalidate(at, &mut output.events);
        }
        if root && became_idle && !uncertain_removal {
            self.closed = true;
            self.root_activity_stopped = true;
            self.armed = false;
            self.pending = None;
        }
        for removed in &transition.removed {
            self.block_ids.remove(&removed.id);
        }
        if let Some(current) = &transition.current {
            if current.state == ProgramStatusState::Blocked {
                if !self.block_ids.contains_key(&current.id) {
                    self.block_serial = self.block_serial.saturating_add(1);
                    self.block_ids.insert(
                        current.id.clone(),
                        format!("native-block:{}:{}", self.incarnation, self.block_serial),
                    );
                }
            } else {
                self.block_ids.remove(&current.id);
            }
            if root && current.state == ProgramStatusState::Idle && !self.first_activity {
                self.root_content_idle = true;
            }
            if root && active(current.state) && !self.closed {
                if self.first_activity {
                    self.accepted_input();
                    self.unclaimed_activity = true;
                }
                self.root_content_idle = false;
                self.armed = !self.maintenance;
                self.pending = None;
            }
            if root && transition.completed && self.armed && !self.closed && !self.maintenance {
                self.completion_serial = self.completion_serial.saturating_add(1);
                let ticket = CompletionTicket {
                    activity: self.activity(),
                    serial: self.completion_serial,
                    failed: current.state == ProgramStatusState::Error,
                    detail: current.message.clone().unwrap_or_default(),
                };
                self.pending = Some(ticket.clone());
                output.completion = Some(ticket);
            }
        }
        if root && became_idle && !uncertain_removal {
            // Root availability is independent of surviving children. Clear the primary turn
            // before projecting any background activity, without claiming successful completion.
            self.publish_state(
                "idle",
                "root",
                transition.current.as_ref(),
                at,
                &mut output.events,
            );
        }
        self.publish_background(at, &mut output.events);
        if !uncertain_removal && transition.current.is_some() {
            self.publish_execution(at, &mut output.events);
        }
        output
    }

    /// The caller feeds native canonical frames through this gate before Pump. Replay never calls
    /// this method. Usage on a suppressed duplicate can still reach a private telemetry port.
    pub fn observe_canonical(&mut self, frame: &Value) -> CanonicalDisposition {
        match frame["type"].as_str() {
            Some("user.message") => self.accepted_input(),
            Some("turn.completed") => {
                if self.completed {
                    return CanonicalDisposition::DuplicateCompletion;
                }
                if frame["outcome"] == "interrupted" {
                    self.store.release_active();
                    self.canonical_background.clear();
                    self.last_background.clear();
                    self.block_ids.clear();
                } else if self.closed {
                    return CanonicalDisposition::DuplicateCompletion;
                }
                self.closed = true;
                self.completed = true;
                self.armed = false;
                self.first_activity = false;
                self.pending = None;
            }
            Some("background.changed") => {
                let Some(tasks) = frame["tasks"].as_array().filter(|tasks| tasks.len() <= 256)
                else {
                    return CanonicalDisposition::Forward;
                };
                self.canonical_background = tasks.clone();
                let tasks = self.background();
                self.last_background = tasks.clone();
                return CanonicalDisposition::Replace(event(
                    "background.changed",
                    frame["at"].as_u64().unwrap_or_default(),
                    json!({"tasks": tasks}),
                ));
            }
            _ if agent_activity(frame) && self.first_activity => {
                self.accepted_input();
                self.unclaimed_activity = true;
                self.armed = true;
            }
            _ => {}
        }
        CanonicalDisposition::Forward
    }

    pub fn commit_completion(
        &mut self,
        ticket: &CompletionTicket,
        barrier: TranscriptBarrier,
        at: u64,
    ) -> Vec<Value> {
        let Some(frame) = self.preview_completion(ticket, barrier, at) else {
            return Vec::new();
        };
        self.pending = None;
        self.closed = true;
        self.completed = true;
        self.armed = false;
        self.first_activity = false;
        vec![frame]
    }

    /// Prepare public bytes without settling execution. A durable host revalidates the ticket
    /// and calls `commit_completion` only inside its successful canonical commit transaction.
    pub fn preview_completion(
        &self,
        ticket: &CompletionTicket,
        barrier: TranscriptBarrier,
        at: u64,
    ) -> Option<Value> {
        if barrier != TranscriptBarrier::CaughtUp
            || self.closed
            || self.pending.as_ref() != Some(ticket)
        {
            return None;
        }
        Some(event(
            "turn.completed",
            at,
            json!({
                "outcome": if ticket.failed { "error" } else { "ok" }, "message":ticket.detail,
                "durationMs":null, "costUsd":null,
            }),
        ))
    }

    /// Process retirement loses observations; it does not turn them into successful completion.
    pub fn retire(&mut self, at: u64) -> Vec<Value> {
        let mut events = Vec::new();
        self.invalidate(at, &mut events);
        self.store.reset();
        self.canonical_background.clear();
        self.block_ids.clear();
        self.publish_background(at, &mut events);
        events
    }

    fn activity(&self) -> String {
        format!("native-activity:{}:{}", self.incarnation, self.cycle)
    }

    fn invalidate(&mut self, at: u64, events: &mut Vec<Value>) {
        self.closed = true;
        // Losing a child's state cannot undo a root stop already observed. Keep that boundary
        // for late root content while publishing unknown for the missing execution evidence.
        self.first_activity = false;
        self.armed = false;
        self.pending = None;
        self.publish_state("unknown", "root", None, at, events);
    }

    fn background(&self) -> Vec<Value> {
        let mut tasks = self.canonical_background.clone();
        tasks.extend(self.store.snapshot().children.into_iter().filter(|(_, report)| active(report.state)).map(|(id, report)| {
            json!({"id":format!("native-status:{}:{id}", self.incarnation), "description":report.title.or(report.message).unwrap_or_default(), "toolId":null})
        }));
        tasks
    }

    fn publish_background(&mut self, at: u64, events: &mut Vec<Value>) {
        let tasks = self.background();
        if tasks != self.last_background {
            self.last_background = tasks.clone();
            events.push(event("background.changed", at, json!({"tasks":tasks})));
        }
    }

    fn publish_execution(&mut self, at: u64, events: &mut Vec<Value>) {
        let snapshot = self.store.snapshot();
        let blocked = snapshot
            .root
            .iter()
            .chain(snapshot.children.values())
            .find(|record| record.state == ProgramStatusState::Blocked);
        if let Some(blocked) = blocked {
            if blocked.id.is_some() || !self.closed {
                self.publish_state(
                    "blocked",
                    if blocked.id.is_some() {
                        "background"
                    } else {
                        "root"
                    },
                    Some(blocked),
                    at,
                    events,
                );
            }
        } else if snapshot
            .root
            .as_ref()
            .is_some_and(|record| record.state == ProgramStatusState::Working)
            && !self.closed
        {
            self.publish_state("working", "root", snapshot.root.as_ref(), at, events);
        } else if let Some(child) = snapshot
            .children
            .values()
            .find(|record| record.state == ProgramStatusState::Working)
        {
            self.publish_state("working", "background", Some(child), at, events);
        } else if snapshot
            .root
            .as_ref()
            .is_some_and(|record| record.state == ProgramStatusState::Idle)
        {
            self.publish_state("idle", "root", snapshot.root.as_ref(), at, events);
        }
    }

    fn publish_state(
        &mut self,
        state: &str,
        scope: &str,
        record: Option<&ProgramStatusReport>,
        at: u64,
        events: &mut Vec<Value>,
    ) {
        let fields = json!({"activityId":self.activity(), "state":state, "scope":scope,
            "blockId": record.and_then(|record| self.block_ids.get(&record.id)),
            "kind": record.and_then(|record| record.kind).map(|kind| match kind { ProgramStatusKind::Permission => "permission", ProgramStatusKind::Question => "question", ProgramStatusKind::Auth => "auth" }),
            "detail": record.and_then(|record| record.message.as_ref().or(record.title.as_ref())),
        });
        if self.last_execution.as_ref() != Some(&fields) {
            self.last_execution = Some(fields.clone());
            events.push(event("execution.observed", at, fields));
        }
    }
}

fn active(state: ProgramStatusState) -> bool {
    matches!(
        state,
        ProgramStatusState::Working | ProgramStatusState::Blocked
    )
}

#[cfg(test)]
mod tests;
