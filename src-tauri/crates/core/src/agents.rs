//! Provider-neutral discovery contracts shared by application hosts.
use crate::{
    accounts::{Account, Identity},
    board::ProviderId,
};
/// A model as exposed to the launcher.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub label: String,
    /// Supported effort levels prevent the launcher from offering values the CLI rejects.
    pub efforts: Vec<String>,
    #[serde(default)]
    pub additional: bool,
}

/// Provider-independent features exposed to the app. Serde maps contract field names to camelCase.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub initial_plan_mode: bool,
    pub workspace_mcp_selection: bool,
    pub workspace_plugin_selection: bool,
    pub resume: bool,
    pub compact: bool,
    pub context_report: bool,
    pub usage_tokens: bool,
    pub usage_cost: bool,
    pub context_window: bool,
    pub approvals: bool,
    pub user_questions: bool,
    pub attachments: bool,
}

/// Return every supported provider even when its installation is unavailable.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDescriptor {
    pub id: ProviderId,
    pub label: String,
    pub installed: bool,
    pub models: Vec<Model>,
    pub capabilities: AgentCapabilities,
    pub auth_methods: Vec<AuthMethod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    pub account_notice: Option<String>,
}

#[derive(serde::Serialize)]
pub struct AuthMethod {
    pub id: String,
    pub kind: String,
    pub label: String,
}

#[derive(serde::Serialize)]
pub struct Agents {
    pub providers: Vec<AgentDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CatalogError {
    pub code: String,
}
impl CatalogError {
    pub fn new(kind: &str) -> Self {
        Self {
            code: format!("err.modelsCatalog.{kind}"),
        }
    }
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalog {
    pub models: Vec<Model>,
    pub fetched_at: u64,
}

pub trait ProviderDiscovery: Send + Sync {
    fn descriptor(&self) -> AgentDescriptor;
    fn models(&self, account: &Account) -> Result<ModelCatalog, CatalogError>;
    fn identity(&self, account: &Account) -> Result<Identity, String>;
}
