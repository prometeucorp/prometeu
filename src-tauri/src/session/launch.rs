//! Desktop preparation and lifecycle composition for the portable launch service.
use super::*;
use prometeu_core::session::launch::{
    ConversationLauncher, LaunchEffects, LaunchRequest, LaunchService, Launched, PreparedResume,
    ResumePreparation, ResumeSnapshot, StartMode,
};

struct Desktop<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
}
impl Desktop<'_> {
    fn service(&self) -> LaunchService<'_, chat::Chat> {
        LaunchService {
            board: &self.state.board,
            host: &self.state.sessions,
            preparation: self,
            launcher: self,
            effects: self,
        }
    }
}

pub(super) fn resume(app: &AppHandle, state: &AppState, session: &str) -> Result<bool, String> {
    Desktop { app, state }.service().resume(session)
}
pub(super) fn start(
    app: &AppHandle,
    state: &AppState,
    request: &LaunchRequest,
) -> Result<(), String> {
    Desktop { app, state }
        .service()
        .start(request, None)
        .map(|_| ())
}

impl ResumePreparation for Desktop<'_> {
    fn prepare(&self, session: &str, snapshot: ResumeSnapshot) -> Result<PreparedResume, String> {
        let ws = snapshot.workspace;
        let tab = ws.tabs.iter().find(|tab| tab.id == session);
        let previous = tab.and_then(|tab| tab.agent_session.clone());
        let agent = workspace_launch_of(&ws, session, &ResolvedTools::default()).agent;
        let tools = resolve_workspace_tools(&snapshot.tools, &snapshot.trust, &ws, agent);
        let mut launch = workspace_launch_of(&ws, session, &tools);
        let warning = tab
            .and_then(|tab| tab.kickoff.as_deref())
            .and_then(|kickoff| {
                crate::kickoff::resume(
                    &mut launch,
                    kickoff,
                    &crate::skills::load(),
                    &crate::plugins::load(),
                )
            });
        if ws.cleaned {
            return Err(i18n::t("err.session.cleaned"));
        }
        if !Path::new(&ws.worktree).exists() {
            return Err(i18n::ta("err.session.noWorktree", &[("path", ws.worktree)]));
        }
        Ok(PreparedResume {
            request: LaunchRequest {
                session: session.into(),
                workspace: ws.id,
                worktree: ws.worktree,
                settings: launch,
                mode: StartMode::Resume {
                    provider_session: previous,
                },
            },
            warning,
        })
    }
}

impl LaunchEffects<chat::Chat> for Desktop<'_> {
    fn revoke(&self, session: &str) {
        crate::embedded_mcp::revoke(session);
    }
    fn stopped(&self, session: &str) {
        crate::delegation::stopped(self.state, session);
    }
    fn warning(&self, conversation: &chat::Chat, detail: &str) {
        conversation.warn("kickoff.missing", detail);
    }
    fn publish(&self) {
        publish(self.app);
    }
    fn ready(&self, session: &str) {
        chat::ready_now(self.app, session);
    }
}

impl ConversationLauncher<chat::Chat> for Desktop<'_> {
    fn launch(&self, request: &LaunchRequest) -> Result<Launched<chat::Chat>, String> {
        let prepared = self.state.providers.prepare(request)?;
        let resumed = prepared.resumed;
        Ok(Launched {
            conversation: chat::launch(self.app, &request.session, prepared)?,
            resumed,
        })
    }
}
