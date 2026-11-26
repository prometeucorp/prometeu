//! Workspace supervision metadata and execution observations shared by execution hosts.
//! A supervisor is assigned independently of how a workspace was created. Transport,
//! persistence, credentials and provider hooks belong to the host's adapters.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub supervisor_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    #[serde(rename = "workspace.created")]
    WorkspaceCreated,
    #[serde(rename = "workspace.closed")]
    WorkspaceClosed,
    #[serde(rename = "workspace.pr_linked")]
    WorkspacePrLinked,
    #[serde(rename = "workspace.unsupervised")]
    WorkspaceUnsupervised,
    #[serde(rename = "execution.completed")]
    ExecutionCompleted,
    #[serde(rename = "execution.failed")]
    ExecutionFailed,
    #[serde(rename = "execution.blocked")]
    ExecutionBlocked,
}

/// Version one contains identifiers and status metadata only, never prompts or output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub version: u32,
    pub id: String,
    pub supervisor_id: String,
    pub executor_id: String,
    pub workspace_id: String,
    pub kind: Kind,
    pub occurred_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
}

/// Provider adapters normalize observations before they enter the portable rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Started,
    Working,
    Blocked,
    Completed,
    Failed,
    Interrupted,
    BackgroundStarted(String),
    BackgroundFinished(String),
    Exited { success: bool },
}

/// Persist this state with emitted events so replay cannot announce the same end twice.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Execution {
    pub id: Option<String>,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    blocked: bool,
    #[serde(default)]
    main_done: bool,
    #[serde(default)]
    background: BTreeSet<String>,
}

impl Execution {
    /// `new_execution_id` is supplied by the host and used only when an execution starts.
    pub fn observe(&mut self, observation: Observation, new_execution_id: &str) -> Option<Kind> {
        match observation {
            Observation::Started => {
                if !self.active {
                    self.begin(new_execution_id);
                }
                self.main_done = false;
                self.blocked = false;
            }
            Observation::Working => {
                // Child activity after the root's end must not cancel its held completion.
                self.blocked = false;
            }
            Observation::Blocked => {
                if !self.active {
                    self.begin(new_execution_id);
                }
                if !self.blocked {
                    self.blocked = true;
                    return Some(Kind::ExecutionBlocked);
                }
            }
            Observation::Completed if self.active => {
                self.main_done = true;
                return self.settle();
            }
            Observation::Failed if self.active => {
                self.end();
                return Some(Kind::ExecutionFailed);
            }
            Observation::BackgroundStarted(id) if self.active => {
                self.background.insert(id);
            }
            Observation::BackgroundFinished(id) if self.active => {
                self.background.remove(&id);
                return self.settle();
            }
            Observation::Interrupted => self.end(),
            Observation::Exited { .. } if self.active => {
                // A clean process exit alone does not prove the task or known children finished.
                self.end();
                return Some(Kind::ExecutionFailed);
            }
            _ => {}
        }
        None
    }

    fn begin(&mut self, id: &str) {
        self.id = Some(id.to_owned());
        self.active = true;
        self.blocked = false;
        self.main_done = false;
        self.background.clear();
    }

    fn end(&mut self) {
        self.active = false;
        self.blocked = false;
        self.main_done = false;
        self.background.clear();
    }

    fn settle(&mut self) -> Option<Kind> {
        if self.main_done && self.background.is_empty() {
            self.end();
            Some(Kind::ExecutionCompleted)
        } else {
            None
        }
    }

    /// The host's process cannot survive its own restart; an unfinished execution needs review.
    pub fn reconcile_restart(&mut self) -> Option<Kind> {
        if !self.active {
            return None;
        }
        self.end();
        Some(Kind::ExecutionFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_cloud_contract_preserves_metadata_and_additive_fields() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../fixtures/cloud-api.json")).unwrap();
        for key in ["registration", "event"] {
            let value = fixture["supervision"][key].clone();
            let event: Event = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(event.version, 1);
            assert_eq!(serde_json::to_value(&event).unwrap(), value);
            let mut future = value;
            future["future_metadata"] = serde_json::json!("additive");
            assert_eq!(serde_json::from_value::<Event>(future).unwrap(), event);
        }
    }

    #[test]
    fn completion_waits_for_children_and_does_not_rearm_from_late_activity() {
        let mut execution = Execution::default();
        assert_eq!(execution.observe(Observation::Completed, "unused"), None);
        execution.observe(Observation::Started, "run-1");
        execution.observe(Observation::BackgroundStarted("child".into()), "unused");
        execution.observe(Observation::BackgroundStarted("child".into()), "unused");
        assert_eq!(execution.observe(Observation::Completed, "unused"), None);
        execution.observe(Observation::Working, "unused");
        assert_eq!(
            execution.observe(Observation::BackgroundFinished("other".into()), "unused"),
            None
        );
        assert_eq!(
            execution.observe(Observation::BackgroundFinished("child".into()), "unused"),
            Some(Kind::ExecutionCompleted)
        );
        assert_eq!(execution.id.as_deref(), Some("run-1"));
        execution.observe(Observation::Working, "unused");
        assert_eq!(execution.observe(Observation::Completed, "unused"), None);
        assert_eq!(execution.reconcile_restart(), None);
    }

    #[test]
    fn blocking_episodes_and_restart_are_durable_and_idempotent() {
        let mut execution = Execution::default();
        assert_eq!(
            execution.observe(Observation::Blocked, "run-1"),
            Some(Kind::ExecutionBlocked)
        );
        assert_eq!(execution.observe(Observation::Blocked, "unused"), None);
        let persisted = serde_json::to_string(&execution).unwrap();
        let mut execution: Execution = serde_json::from_str(&persisted).unwrap();
        assert_eq!(execution.observe(Observation::Blocked, "unused"), None);
        execution.observe(Observation::Working, "unused");
        assert_eq!(
            execution.observe(Observation::Blocked, "unused"),
            Some(Kind::ExecutionBlocked)
        );
        assert_eq!(execution.reconcile_restart(), Some(Kind::ExecutionFailed));
        assert_eq!(execution.reconcile_restart(), None);
        execution.observe(Observation::Started, "run-2");
        assert_eq!(execution.id.as_deref(), Some("run-2"));
        assert_eq!(
            execution.observe(Observation::Completed, "unused"),
            Some(Kind::ExecutionCompleted)
        );
    }

    #[test]
    fn exits_and_interruptions_never_claim_success() {
        for observation in [
            Observation::Exited { success: true },
            Observation::Exited { success: false },
            Observation::Failed,
        ] {
            let mut execution = Execution::default();
            execution.observe(Observation::Started, "run");
            assert_eq!(
                execution.observe(observation, "unused"),
                Some(Kind::ExecutionFailed)
            );
            assert_eq!(execution.observe(Observation::Completed, "unused"), None);
        }
        let mut execution = Execution::default();
        execution.observe(Observation::Started, "run");
        execution.observe(Observation::Interrupted, "unused");
        assert_eq!(execution.observe(Observation::Completed, "unused"), None);
        assert_eq!(execution.reconcile_restart(), None);
    }
}
