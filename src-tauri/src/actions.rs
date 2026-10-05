//! The app owns reusable commands and profiles. Each execution stores a resolved copy.
use crate::lock::lock;
#[cfg(test)]
use crate::state::Choice;
use crate::state::{publish, Status, Tab};
use crate::{i18n, session, AppState};
use std::collections::HashSet;
use tauri::{AppHandle, Manager, State};

#[cfg(test)]
use prometeu_core::actions::Action;
pub use prometeu_core::actions::{Catalog, Kind, Permission, Profile, Run};

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        && !matches!(name, "compact" | "context")
}

fn validate_profile(p: &Profile) -> Result<(), String> {
    if p.id.is_empty()
        || p.name.trim().is_empty()
        || p.prompt.trim().is_empty()
        || p.prompt.len() > 100_000
        || p.skills
            .iter()
            .any(|s| s.trim().is_empty() || s.contains('\n'))
    {
        return Err(i18n::t("err.actions.invalid"));
    }
    Ok(())
}

pub fn validate(c: &Catalog) -> Result<(), String> {
    let mut ids = HashSet::new();
    for p in &c.profiles {
        validate_profile(p)?;
        if !ids.insert(&p.id) {
            return Err(i18n::t("err.actions.invalid"));
        }
    }
    let mut names = HashSet::new();
    for a in &c.commands {
        if !valid_name(&a.name)
            || !names.insert(&a.name)
            || a.prompt.len() > 100_000
            || (a.kind == Kind::Prompt && a.prompt.trim().is_empty())
            || (a.kind == Kind::Agent && !a.profile.as_ref().is_some_and(|id| ids.contains(id)))
        {
            return Err(i18n::t("err.actions.invalid"));
        }
    }
    for overrides in c.overrides.values() {
        for (id, p) in overrides {
            validate_profile(p)?;
            if id != &p.id || !ids.contains(id) {
                return Err(i18n::t("err.actions.invalid"));
            }
        }
    }
    if c.pr_action.as_ref().is_some_and(|name| {
        !c.commands
            .iter()
            .any(|a| &a.name == name && a.kind == Kind::Agent)
    }) {
        return Err(i18n::t("err.actions.invalid"));
    }
    Ok(())
}

#[tauri::command]
pub fn actions_save(
    app: AppHandle,
    state: State<AppState>,
    catalog: Catalog,
) -> Result<(), String> {
    validate(&catalog)?;
    crate::catalog::mutate(&app, |doc| doc.actions = Some(catalog.clone()))?;
    lock(&state.board).actions = catalog;
    publish(&app);
    Ok(())
}

pub fn resolve(
    c: &Catalog,
    project: &str,
    id: &str,
    tools: impl FnOnce(crate::state::ProviderId) -> session::ResolvedTools,
) -> Result<Profile, String> {
    let mut p = c
        .overrides
        .get(project)
        .and_then(|map| map.get(id))
        .or_else(|| c.profiles.iter().find(|p| p.id == id))
        .cloned()
        .ok_or_else(|| i18n::t("err.actions.missing"))?;
    // Resolve for the profile's provider, which may differ from the workspace default.
    let resolved = tools(p.choice.agent);
    // A task freezes the tools it starts with, so the caller resolves the layers once and an axis
    // the profile leaves unset inherits that resolved global and workspace selection.
    if p.mcp.is_none() {
        if p.choice.agent == crate::state::ProviderId::Claude
            && resolved.mcp_inherits_base
            && crate::mcp::connectors().is_none()
        {
            return Err(i18n::t("err.mcp.connectors"));
        }
        p.mcp = resolved.mcp.clone();
    }
    if p.plugins.is_none() {
        // Standalone skills ride the plugin pipeline, so the frozen set carries both axes.
        p.plugins = resolved.plugin_packages();
    }
    Ok(p)
}

pub fn instructions(p: &Profile) -> String {
    let mut text = p.prompt.clone();
    if !p.skills.is_empty() {
        text.push_str(&i18n::pick(
            "\n\nUse estas skills; se alguma não estiver disponível, pare e informe: ",
            "\n\nUse these skills; if any is unavailable, stop and report it: ",
        ));
        text.push_str(&p.skills.join(", "));
    }
    text
}

#[tauri::command(async)]
pub fn action_start(
    app: AppHandle,
    state: State<AppState>,
    workspace: String,
    name: String,
    context: String,
) -> Result<Tab, String> {
    let tab = {
        // Snapshot phase: the checks and the tool resolution (git subprocesses, CLI configuration
        // reads) run on clones so the board mutex stays free for transcript events.
        let (actions, global, trust, a, ws) = {
            let board = lock(&state.board);
            let a = board
                .actions
                .commands
                .iter()
                .find(|a| a.name == name && a.kind == Kind::Agent)
                .cloned()
                .ok_or_else(|| i18n::t("err.actions.missing"))?;
            let Some(ws) = board.workspaces.iter().find(|w| w.id == workspace).cloned() else {
                return Err(i18n::t("err.session.noWorkspace"));
            };
            (
                board.actions.clone(),
                board.tools.clone(),
                board.tool_trust.clone(),
                a,
                ws,
            )
        };
        if ws.cleaned || ws.archived || ws.preparing || ws.failed.is_some() {
            return Err(i18n::t("err.actions.unavailable"));
        }
        if let Some(tab) = ws.tabs.iter().find(|t| {
            t.task
                .as_ref()
                .is_some_and(|r| r.command == name && !r.done)
        }) {
            if !context.trim().is_empty() {
                return Err(i18n::t("err.actions.active"));
            }
            return Ok(tab.clone());
        }
        if ws.tabs.iter().any(|t| {
            matches!(t.status, Status::Rodando | Status::Querendo) || t.pending_prompt.is_some()
        }) {
            return Err(i18n::t("err.actions.busy"));
        }
        let profile = resolve(
            &actions,
            &ws.project,
            a.profile.as_deref().unwrap_or(""),
            |agent| session::resolve_workspace_tools(&global, &trust, &ws, agent),
        )?;
        validate_profile(&profile)?;
        let git_context = format!(
            "{}\n{}",
            ws.title,
            ws.repos
                .iter()
                .map(|r| format!("{}: {} (base: {})", r.name, r.worktree, r.base))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let prompt = [a.prompt.trim(), git_context.as_str(), context.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let tab = Tab {
            plan: false,
            permission: None,
            id: uuid::Uuid::new_v4().to_string(),
            agent_session: None,
            title: profile.name.clone(),
            status: Status::Rodando,
            note: None,
            tokens: None,
            context_tokens: None,
            context_window: None,
            pending_prompt: Some(if prompt.is_empty() {
                profile.prompt.clone()
            } else {
                prompt
            }),
            choice: Some(profile.choice.clone()),
            kickoff: None,
            task: Some(Run {
                command: name.clone(),
                profile,
                paused: false,
                done: false,
                turns: 0,
                checked_at: 0,
                error: None,
            }),
        };
        // Mutation phase: re-validate against the live board, since another action may have
        // started or finished a tab while this one resolved off the lock.
        let mut board = lock(&state.board);
        let ws = board
            .workspace_mut(&workspace)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        if ws.cleaned || ws.archived || ws.preparing || ws.failed.is_some() {
            return Err(i18n::t("err.actions.unavailable"));
        }
        if let Some(existing) = ws.tabs.iter().find(|t| {
            t.task
                .as_ref()
                .is_some_and(|r| r.command == name && !r.done)
        }) {
            if !context.trim().is_empty() {
                return Err(i18n::t("err.actions.active"));
            }
            return Ok(existing.clone());
        }
        if ws.tabs.iter().any(|t| {
            matches!(t.status, Status::Rodando | Status::Querendo) || t.pending_prompt.is_some()
        }) {
            return Err(i18n::t("err.actions.busy"));
        }
        ws.active = Some(tab.id.clone());
        ws.tabs.push(tab.clone());
        tab
    };
    publish(&app);
    if let Err(error) = session::revive(&app, &state, &tab.id) {
        if let Some(tab) = lock(&state.board).tab_mut(&tab.id) {
            tab.status = Status::Desligada;
            tab.note = None;
        }
        fail(&app, &tab.id, error);
    }
    Ok(lock(&state.board).tab_mut(&tab.id).cloned().unwrap_or(tab))
}

fn fail(app: &AppHandle, session: &str, error: String) {
    let state = app.state::<AppState>();
    if let Some(run) = lock(&state.board)
        .tab_mut(session)
        .and_then(|t| t.task.as_mut())
    {
        run.error = Some(error);
    }
    publish(app);
}

pub fn completed(app: &AppHandle, session: &str, failed: bool) {
    let state = app.state::<AppState>();
    if let Some(run) = lock(&state.board)
        .tab_mut(session)
        .and_then(|t| t.task.as_mut())
    {
        if failed {
            run.error = Some(i18n::t("err.actions.turn"));
        } else {
            run.done = true;
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile() -> Profile {
        Profile {
            id: "reviewer".into(),
            name: "Revisor".into(),
            prompt: "Revise".into(),
            choice: Choice::default(),
            mcp: None,
            plugins: None,
            skills: vec![],
            permission: Permission::Ask,
        }
    }

    #[test]
    fn code_review_defaults_migrate_once_and_preserve_user_choices() {
        let mut board: crate::state::Board =
            serde_json::from_value(json!({"stages":[], "workspaces":[]})).unwrap();
        board.revive();
        assert_eq!(board.actions.commands[0].name, "review");
        assert_eq!(board.actions.profiles[0].name, "Code review");
        assert!(validate(&board.actions).is_ok());
        board.actions.profiles[0].choice.model = "opus".into();
        board.revive();
        assert_eq!(board.actions.profiles.len(), 1);
        assert_eq!(board.actions.profiles[0].choice.model, "opus");
        board.actions.commands.clear();
        board.actions.profiles.clear();
        let mut restored: crate::state::Board =
            serde_json::from_str(&serde_json::to_string(&board).unwrap()).unwrap();
        restored.revive();
        assert!(restored.actions.commands.is_empty());
        assert!(restored.actions.profiles.is_empty());
        let mut collision = Catalog {
            commands: vec![Action {
                name: "review".into(),
                description: "Custom".into(),
                kind: Kind::Prompt,
                prompt: "Custom".into(),
                profile: None,
            }],
            ..Default::default()
        };
        collision.initialize_defaults();
        assert_eq!(collision.commands.len(), 1);
        assert_eq!(collision.commands[0].prompt, "Custom");
        assert!(collision.profiles.is_empty());
    }

    #[test]
    fn validates_references_and_names() {
        let mut c = Catalog {
            profiles: vec![profile()],
            commands: vec![Action {
                name: "review".into(),
                description: String::new(),
                kind: Kind::Agent,
                prompt: String::new(),
                profile: Some("reviewer".into()),
            }],
            ..Default::default()
        };
        assert!(validate(&c).is_ok());
        c.commands[0].profile = Some("missing".into());
        assert!(validate(&c).is_err());
        c.commands[0].kind = Kind::Prompt;
        assert!(validate(&c).is_err());
        c.commands[0].prompt = "Explique".into();
        assert!(validate(&c).is_ok());
        c.commands[0].name = "compact".into();
        assert!(validate(&c).is_err());
    }

    #[test]
    fn monitored_profiles_and_tasks_from_earlier_versions_still_load() {
        let profile: Profile = serde_json::from_value(json!({"id":"p","name":"P","prompt":"x","choice":{"agent":"claude","model":"","effort":""},
            "mcp":null,"plugins":null,"skills":[],"permission":"ask","watch":{"interval_seconds":60,"comments":true,"ci":true,"max_turns":10}})).unwrap();
        let run: Run = serde_json::from_value(json!({"command":"review","profile":profile,"paused":true,"done":false,"turns":3,"checked_at":9,
            "error":null,"seen":{"comment":"hash"},"prs":{"repo":1}})).unwrap();
        let written = serde_json::to_value(&run).unwrap();
        assert!(written.get("watch").is_none() && written.get("seen").is_none());
        assert!(written.get("turns").is_some() && written.get("checked_at").is_some());
    }

    #[test]
    fn previous_board_has_no_actions_and_previous_tabs_have_no_task() {
        let board: crate::state::Board =
            serde_json::from_value(json!({"stages":[], "workspaces":[]})).unwrap();
        assert!(board.actions.commands.is_empty());
        let tab: Tab = serde_json::from_value(
            json!({"id":"old", "title":"", "status":"pronta", "note":null, "pending_prompt":null}),
        )
        .unwrap();
        assert!(tab.task.is_none());
    }

    #[test]
    fn snapshot_keeps_configuration_when_catalog_changes() {
        let mut p = profile();
        p.mcp = Some(vec![]);
        p.plugins = Some(vec!["review".into()]);
        p.choice.model = "sonnet".into();
        let run = Run {
            command: "review".into(),
            profile: p.clone(),
            paused: false,
            done: false,
            turns: 0,
            checked_at: 0,
            error: None,
        };
        p.choice.model = "opus".into();
        let restored: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        assert_eq!(restored.profile.choice.model, "sonnet");
        assert_eq!(restored.profile.mcp, Some(vec![]));
    }
}
