//! Domain types cross state, services, and IPC. Keep them outside discovery adapters so persisted
//! state does not depend on Tauri command modules.

/// PR data from gh, comparable and printable because Repo exposes those traits in tests.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub is_draft: bool,
    /// OPEN, MERGED, or CLOSED.
    #[serde(default)]
    pub state: String,
    /// A gh transport field used to match responses to workspaces; excluded from board.json and the
    /// frontend contract.
    #[serde(default, skip_serializing)]
    pub head_ref_name: String,
}

impl Pr {
    pub fn merged(&self) -> bool {
        self.state == "MERGED"
    }

    pub fn open(&self) -> bool {
        self.state == "OPEN"
    }
}

/// Stable issue metadata persisted with a workspace.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default, PartialEq)]
pub struct IssueRef {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
}
