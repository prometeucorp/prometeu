//! Durable delegation identities and restart reconciliation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub state: String,
    pub source: String,
    pub accepted_at: u64,
    pub outcome: Option<String>,
    pub request_hash: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Delegation {
    /// The delegated agent and its conversation share one stable identity in this first version.
    pub id: String,
    pub owner: String,
    pub workspace: String,
    pub task: String,
    /// Caller-supplied key prevents retrying creation from creating a second worktree.
    pub request_key: String,
    pub request_hash: String,
    pub repository_heads: BTreeMap<String, String>,
    #[serde(default)]
    pub permission: Option<crate::actions::Permission>,
    pub executions: Vec<Execution>,
    /// None means no authoritative background signal has been observed in this process lifetime.
    pub background: Option<Vec<Value>>,
    pub requests: Vec<Value>,
}

impl Delegation {
    pub fn reconcile_restart(&mut self, pending: bool) {
        self.background = None;
        self.requests.clear();
        for run in &mut self.executions {
            if run.state == "running" || (run.state == "queued" && !pending) {
                run.state = "stopped".into();
            }
        }
    }
}
