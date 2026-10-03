//! Session launch settings and restart coordination, with native preparation and spawning injected.
use super::host::{HostedConversation, SessionHost};
use crate::actions::Permission;
use crate::board::{Board, Choice, ProviderId, Status, ToolTrust, Workspace};
use crate::error::code;
use crate::lock::lock;
use crate::selection::Tools;
use std::sync::Mutex;

/// Provider-neutral launch settings shared by the launcher and persisted workspace defaults.
#[derive(serde::Deserialize, Clone, Default)]
pub struct Launch {
    #[serde(default)]
    pub permission: Option<crate::actions::Permission>,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub config_scope: Option<String>,
    /// The provider comes from the selected model's catalog entry.
    #[serde(default)]
    pub agent: ProviderId,
    /// An empty model lets the provider choose its default.
    #[serde(default)]
    pub model: String,
    /// An empty effort lets the provider choose its default.
    #[serde(default)]
    pub effort: String,
    /// Start the initial conversation in plan mode, requiring plan approval before execution.
    #[serde(default)]
    pub plan: bool,
    /// MCP server IDs from the hub. None preserves the CLI's own configuration; see Workspace::mcp
    /// and mcp.rs.
    #[serde(default)]
    pub mcp: Option<Vec<String>>,
    /// Whether a declared MCP selection still inherits the CLI base.
    #[serde(skip)]
    pub mcp_inherits_base: bool,
    /// Plugin IDs from the hub. None preserves the CLI's own configuration; see Workspace::plugins
    /// and plugins.rs.
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
    /// Standalone-skill hub IDs (`skill-<id>`). They share the plugin-package pipeline, so the
    /// adapters materialize them together with `plugins`. None preserves the CLI's own configuration.
    #[serde(default)]
    pub skills: Option<Vec<String>>,
}

/// Convert a persisted tab choice into launch settings. Plan mode belongs to the initial request
/// and is never restored from the tab choice.
impl From<Choice> for Launch {
    fn from(c: Choice) -> Self {
        Launch {
            agent: c.agent,
            model: c.model,
            effort: c.effort,
            plan: false,
            mcp: None,
            plugins: None,
            skills: None,
            ..Default::default()
        }
    }
}

impl Launch {
    /// Plugins and standalone skills share the plugin-package pipeline, so a spawn materializes the
    /// two resolved axes together. `None` on both preserves the CLI's own plugins; otherwise the
    /// selected packages are the union, in plugin-then-skill order.
    pub fn plugin_packages(&self) -> Option<Vec<String>> {
        match (&self.plugins, &self.skills) {
            (None, None) => None,
            (plugins, skills) => {
                let mut merged = plugins.clone().unwrap_or_default();
                if let Some(skills) = skills {
                    merged.extend(skills.iter().cloned());
                }
                Some(merged)
            }
        }
    }
}

/// Local application requests; paths are interpreted only by the execution adapter.
#[derive(Clone)]
pub enum StartMode {
    Fresh,
    Resume { provider_session: Option<String> },
}

pub struct LaunchRequest {
    pub session: String,
    pub workspace: String,
    pub worktree: String,
    pub settings: Launch,
    pub mode: StartMode,
}

pub struct Launched<C> {
    pub conversation: C,
    /// Whether the adapter actually selected an existing provider transcript/session.
    pub resumed: bool,
}

pub trait ConversationLauncher<C> {
    fn launch(&self, request: &LaunchRequest) -> Result<Launched<C>, String>;
}

pub struct ResumeSnapshot {
    pub workspace: Workspace,
    pub tools: Tools,
    pub trust: Vec<ToolTrust>,
    pub delegation_permission: Option<Option<Permission>>,
}

pub struct PreparedResume {
    pub request: LaunchRequest,
    pub warning: Option<String>,
}

pub trait ResumePreparation {
    /// Resolve tools, kickoff, account/configuration inputs and validate native paths outside locks.
    fn prepare(&self, session: &str, snapshot: ResumeSnapshot) -> Result<PreparedResume, String>;
}

pub trait LaunchEffects<C> {
    fn revoke(&self, session: &str);
    fn stopped(&self, session: &str);
    fn warning(&self, conversation: &C, detail: &str);
    fn publish(&self);
    fn ready(&self, session: &str);
}

pub struct LaunchService<'a, C> {
    pub board: &'a Mutex<Board>,
    pub host: &'a SessionHost<C>,
    pub preparation: &'a dyn ResumePreparation,
    pub launcher: &'a dyn ConversationLauncher<C>,
    pub effects: &'a dyn LaunchEffects<C>,
}

impl<C: HostedConversation> LaunchService<'_, C> {
    /// New-tab callers publish their tab before signaling readiness. This method installs only.
    pub fn start(&self, request: &LaunchRequest, warning: Option<&str>) -> Result<bool, String> {
        let started = self.launcher.launch(request)?;
        if let Some(warning) = warning {
            self.effects.warning(&started.conversation, warning);
        }
        lock(&self.host.conversations).insert(request.session.clone(), started.conversation);
        Ok(started.resumed)
    }

    /// Callers retain their existing admission serialization. No native work runs under board locks.
    pub fn resume(&self, session: &str) -> Result<bool, String> {
        let snapshot = {
            let board = lock(self.board);
            ResumeSnapshot {
                workspace: board
                    .workspace_of(session)
                    .cloned()
                    .ok_or_else(|| code("err.session.noTab"))?,
                tools: board.tools.clone(),
                trust: board.tool_trust.clone(),
                delegation_permission: board
                    .delegations
                    .iter()
                    .find(|d| d.id == session)
                    .map(|d| d.permission),
            }
        };
        let permission = snapshot.delegation_permission;
        let mut prepared = self.preparation.prepare(session, snapshot)?;
        if let Some(permission) = permission {
            prepared.request.settings.permission = permission;
        }
        self.effects.revoke(session);
        self.host.remove(session);
        self.effects.stopped(session);
        let resumed = self.start(&prepared.request, prepared.warning.as_deref())?;
        {
            let mut board = lock(self.board);
            if let Some(tab) = board.tab_mut(session) {
                tab.status = Status::Pronta;
                tab.note = None;
            }
        }
        self.effects.publish();
        self.effects.ready(session);
        Ok(resumed)
    }
}

#[cfg(test)]
mod tests;
