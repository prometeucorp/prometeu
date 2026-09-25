use crate::domain::Pr;
use crate::lock::lock;
use crate::selection::{Selection, Tools};
use crate::state::{
    publish, Board, Choice, Project, ProviderId, Repo, Status, Tab, ToolTrust, Workspace,
};
use crate::workspace_tools::{self, Axis};
use crate::{chat, dock, i18n, paths, scripts, AppState};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager, State};

pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod find;
pub(crate) mod git;

#[cfg(test)]
use diff::patch_map;
use diff::{ahead_of, changes_in};

#[tauri::command]
pub fn load_board(state: State<AppState>) -> Board {
    lock(&state.board).clone()
}

/* ---------- projects ---------- */

/// Register folders once so the launcher can reuse them. Non-Git folders support agent sessions but
/// cannot create branches or worktrees.
#[tauri::command(async)]
pub fn add_project(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<Project, String> {
    let path = PathBuf::from(expand(&path));
    let project = register_local_project(&state.board, &path);
    publish(&app);
    Ok(project)
}

fn register_local_project(board: &std::sync::Mutex<Board>, path: &Path) -> Project {
    let _sync = crate::catalog::guard();
    register_project(&mut lock(board), path)
}

/// Both local folders and catalog clones use the same board registration while holding the catalog
/// guard. Acquire that guard before the board lock, so origin checks and cloning stay serialized.
pub(crate) fn register_project(board: &mut Board, path: &Path) -> Project {
    let id = path.display().to_string();
    if let Some(project) = board.projects.iter().find(|p| p.path == id) {
        return project.clone();
    }
    let project = Project {
        id: id.clone(),
        name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo")
            .into(),
        path: id,
    };
    board.projects.push(project.clone());
    project
}

#[tauri::command(async)]
pub fn remove_project(app: AppHandle, state: State<AppState>, id: String) {
    remove_registered_project(&state.board, &id);
    publish(&app);
}

fn remove_registered_project(board: &std::sync::Mutex<Board>, id: &str) {
    let _sync = crate::catalog::guard();
    lock(board).projects.retain(|p| p.id != id);
}

/* ---------- workspaces ---------- */

/// The workspace owns its stage, regardless of whether the menu, header, or drag gesture changes
/// it.
#[tauri::command]
pub fn set_stage(app: AppHandle, state: State<AppState>, id: String, stage: String) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.stage = stage;
        }
    }
    publish(&app);
}

/// Archiving preserves the transcript and initially keeps the worktree and branch. The interface
/// offers their explicit cleanup after this command succeeds. Stop hidden processes so their
/// requests do not wait for an absent reader.
#[tauri::command(async)]
pub fn archive_workspace(app: AppHandle, state: State<AppState>, id: String, archived: bool) {
    archive(&app, &state, &id, archived);
}

/// Finishing moves the workspace to the final stage and archives it. Archiving stops agents, docks,
/// and resources handled by the archive script. The interface then offers worktree cleanup as a
/// separate, confirmed decision.
#[tauri::command(async)]
pub fn finish_workspace(app: AppHandle, state: State<AppState>, id: String) {
    {
        let mut board = lock(&state.board);
        let last = board.stages.last().cloned();
        if let (Some(stage), Some(ws)) = (last, board.workspace_mut(&id)) {
            ws.stage = stage;
        }
    }
    archive(&app, &state, &id, true);
}

fn archive(app: &AppHandle, state: &State<AppState>, id: &str, archived: bool) {
    let generation = lock(&state.telemetry).generation;
    let mut dead: Vec<String> = Vec::new();
    let mut changed = false;
    // Run the archive script before archiving, while its resources still exist. It cleans up
    // containers, databases, and tunnels asynchronously, without a PTY or blocking the window.
    if archived {
        // Stop docks before the archive script removes resources that development servers still
        // use.
        dock::kill_docks(state, id);
        if let Some(ws) = workspace_copy(state, id) {
            if let Some(command) = dock::scripts_of(&ws).archive {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-lc", &command])
                    .current_dir(&ws.primary().worktree);
                for (key, value) in dock::script_env(&ws) {
                    cmd.env(key, value);
                }
                let _ = cmd.spawn();
            }
        }
    }
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(id) {
            changed = ws.archived != archived;
            ws.archived = archived;
            if archived {
                dead = ws.tabs.iter().map(|t| t.id.clone()).collect();
                for tab in &mut ws.tabs {
                    tab.status = Status::Desligada;
                    tab.note = None;
                }
            }
        }
    }
    // Stop processes outside the board lock: signalling and waiting must not block other sessions.
    stop(state, &dead);
    publish(app);
    if changed {
        crate::telemetry::journey(
            app,
            generation,
            id,
            None,
            if archived {
                crate::telemetry::Fact::WorkspaceArchived {}
            } else {
                crate::telemetry::Fact::WorkspaceResumed {}
            },
        );
    }
}

/// Stop these tab processes. Preserve transcripts and worktrees so the next message can resume
/// them.
fn stop(state: &State<AppState>, tabs: &[String]) {
    for tab in tabs {
        chat::kill(state, tab);
        crate::plugins::forget_codex_workspace(tab);
    }
}

/// The initial title comes from the prompt. An empty rename cancels the change rather than erasing
/// the existing title.
#[tauri::command]
pub fn rename_workspace(app: AppHandle, state: State<AppState>, id: String, title: String) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.title = title.to_string();
        }
    }
    publish(&app);
}

/// Pinning moves the workspace to the top without changing its stage.
#[tauri::command]
pub fn pin_workspace(app: AppHandle, state: State<AppState>, id: String, pinned: bool) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.pinned = pinned;
        }
    }
    publish(&app);
}

/// Interpret and validate one tool-axis argument: JSON `null` returns the axis to inherit, an
/// object must deserialize as a `Selection` carrying only ids of its own axis.
fn axis(value: serde_json::Value, kind: Axis) -> Result<Option<Selection>, String> {
    workspace_tools::selection(value, kind).map_err(|error| {
        i18n::t(match error {
            workspace_tools::Invalid::Payload => "err.tools.badPayload",
            workspace_tools::Invalid::Axis => "err.tools.badAxis",
        })
    })
}

/// Read the JSON body before Option deserialization collapses explicit null and an absent key.
/// The outer Option means a supplied axis; the inner Option is its inherit/reset value.
fn axis_patch(
    body: &tauri::ipc::InvokeBody,
    name: &str,
    kind: Axis,
) -> Result<Option<Option<Selection>>, String> {
    let tauri::ipc::InvokeBody::Json(body) = body else {
        return Err(i18n::t("err.tools.badPayload"));
    };
    let body = body
        .as_object()
        .ok_or_else(|| i18n::t("err.tools.badPayload"))?;
    body.get(name)
        .cloned()
        .map(|value| axis(value, kind))
        .transpose()
}

/// Persist one workspace tool axis. The change applies at the next spawn or resume; a running session
/// keeps the set it was born with (ADR 0045), so the picker states that instead of restarting the
/// process, and this command only writes and republishes.
#[tauri::command]
pub fn set_workspace_mcp(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let parsed = axis_patch(request.body(), "mcp", Axis::Mcp)?;
    if let Some(value) = parsed {
        workspace_tools::change(&state.board, &id, Axis::Mcp, value);
    }
    publish(&app);
    Ok(())
}

/// Plugin selection shares the next-spawn application rule. Standalone skills live on their own axis
/// now, so this no longer re-derives them from a combined list.
#[tauri::command]
pub fn set_workspace_plugins(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let parsed = axis_patch(request.body(), "plugins", Axis::Plugins)?;
    if let Some(value) = parsed {
        workspace_tools::change(&state.board, &id, Axis::Plugins, value);
    }
    publish(&app);
    Ok(())
}

/// Standalone-skill selection, its own axis since ADR 0045. Skills still materialize through the
/// plugin-package pipeline; only the state and the picker are separate.
#[tauri::command]
pub fn set_workspace_skills(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let parsed = axis_patch(request.body(), "skills", Axis::Skills)?;
    if let Some(value) = parsed {
        workspace_tools::change(&state.board, &id, Axis::Skills, value);
    }
    publish(&app);
    Ok(())
}

/// Set the global layer of the tool selection, one axis at a time. An absent axis is unchanged; an
/// explicit `null` returns it to inherit. The global layer is a board field, so the result reaches the
/// frontend through the `board` event and the command returns nothing.
#[tauri::command]
pub fn set_tools_global(
    app: AppHandle,
    state: State<AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let mcp = axis_patch(request.body(), "mcp", Axis::Mcp)?;
    let plugins = axis_patch(request.body(), "plugins", Axis::Plugins)?;
    let skills = axis_patch(request.body(), "skills", Axis::Skills)?;
    {
        let mut board = lock(&state.board);
        if let Some(value) = mcp {
            board.tools.mcp = value;
        }
        if let Some(value) = plugins {
            board.tools.plugins = value;
        }
        if let Some(value) = skills {
            board.tools.skills = value;
        }
    }
    publish(&app);
    Ok(())
}

/// The project `[tools]` declaration and its trust state, for the interface (ADR 0045). `hash` and
/// `repo` are empty and `tools` inherits when the primary repository declares nothing.
#[derive(serde::Serialize)]
pub struct ProjectTools {
    /// Repository identity that keys the trust decision: `origin` URL or absolute clone path.
    pub repo: String,
    /// The settings file that declared the layer, if any.
    pub file: Option<String>,
    /// SHA-256 of the declared section; empty when the repository declares nothing.
    pub hash: String,
    /// The declared layer; every axis inherits when nothing is declared.
    pub tools: Tools,
    /// True when a declaration exists whose current hash has no decision, so the interface prompts.
    pub pending: bool,
    /// The stored decision for this repository, if any.
    pub decision: Option<ToolTrust>,
}

/// Resolve a workspace or project id to the worktree and clone of the repository whose `[tools]`
/// governs. A workspace uses its primary repository; a project uses its registered clone for both.
/// Project and workspace ids never overlap (see `cwd_of`).
fn tool_roots(board: &Board, id: &str) -> Option<(PathBuf, PathBuf)> {
    board
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .map(|w| {
            let primary = w.primary();
            (PathBuf::from(primary.worktree), PathBuf::from(primary.path))
        })
        .or_else(|| {
            board
                .projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| (PathBuf::from(&p.path), PathBuf::from(&p.path)))
        })
}

/// Return the `[tools]` declared by the primary repository, the file that declared it, the hash of
/// that section and the stored decision, so the project surface and the trust dialog can render
/// without re-deriving them. `pending` is true only while no decision (approval or rejection)
/// exists for the current hash, so an explicit rejection quiets the prompt until the declaration
/// changes (ADR 0045).
#[tauri::command]
pub fn project_tools(state: State<AppState>, id: String) -> ProjectTools {
    // Snapshot the board references; reading the repository settings runs a git subprocess and
    // parses TOML, which must not hold the board mutex.
    let (roots, trust) = {
        let board = lock(&state.board);
        (tool_roots(&board, &id), board.tool_trust.clone())
    };
    let declaration = roots.and_then(|(worktree, repo)| project_declaration(&worktree, &repo));
    let Some(declaration) = declaration else {
        return ProjectTools {
            repo: String::new(),
            file: None,
            hash: String::new(),
            tools: Tools::default(),
            pending: false,
            decision: None,
        };
    };
    let decision = trust.iter().find(|t| t.repo == declaration.repo).cloned();
    let pending = !decided(&trust, &declaration.repo, &declaration.hash);
    ProjectTools {
        repo: declaration.repo,
        file: declaration.file,
        hash: declaration.hash,
        tools: declaration.tools,
        pending,
        decision,
    }
}

/// Bind the verdict to the declaration displayed by the dialog. Recompute its hash from disk and
/// reject stale dialogs before writing any decision (ADR 0047).
#[tauri::command]
pub fn project_tools_trust(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    hash: String,
    approved: bool,
) -> Result<(), String> {
    let roots = {
        let board = lock(&state.board);
        tool_roots(&board, &id)
    };
    let Some(declaration) =
        roots.and_then(|(worktree, repo)| project_declaration(&worktree, &repo))
    else {
        return Err(i18n::t("err.tools.changed"));
    };
    {
        let mut board = lock(&state.board);
        record_tool_trust(&mut board.tool_trust, declaration, &hash, approved)?;
    }
    publish(&app);
    Ok(())
}

fn record_tool_trust(
    trust: &mut Vec<ToolTrust>,
    declaration: ProjectDeclaration,
    hash: &str,
    approved: bool,
) -> Result<(), String> {
    if declaration.hash != hash {
        return Err(i18n::t("err.tools.changed"));
    }
    let decision = ToolTrust {
        repo: declaration.repo,
        hash: declaration.hash,
        approved,
        at: crate::actions::now(),
    };
    match trust.iter_mut().find(|t| t.repo == decision.repo) {
        Some(existing) => *existing = decision,
        None => trust.push(decision),
    }
    Ok(())
}

/// The effective tool selection of one workspace, per axis, each item labeled with where it came
/// from, so the picker shows the resolved result without reading the three layers (ADR 0045).
#[derive(serde::Serialize)]
pub struct WorkspaceTools {
    pub mcp: Vec<EffectiveItem>,
    pub plugins: Vec<EffectiveItem>,
    pub skills: Vec<EffectiveItem>,
}

/// Resolve the workspace's effective set with provenance. The project layer is read from the
/// primary repository and gated on its stored trust decision: a declaration nobody decided on
/// shows its items as pending, an explicitly rejected one shows them as rejected, and only an
/// approval activates them. On the mcp axis of a Claude conversation the CLI-inherited servers join
/// the universe as the visible base (ADR 0046).
#[tauri::command]
pub fn workspace_tools(
    state: State<AppState>,
    id: String,
    agent: Option<ProviderId>,
) -> Result<WorkspaceTools, String> {
    // Snapshot the board data; hub loads, git and CLI-config reads must not hold the board mutex.
    let (global, trust, ws) = {
        let board = lock(&state.board);
        let Some(ws) = board.workspaces.iter().find(|w| w.id == id).cloned() else {
            return Err(i18n::t("err.session.noWorkspace"));
        };
        (board.tools.clone(), board.tool_trust.clone(), ws)
    };
    let plugin_hub: Vec<String> = crate::plugins::load().into_iter().map(|p| p.id).collect();
    let (mcp_base, mcp_universe) = mcp_base_and_universe(&ws, agent.unwrap_or(ws.agent));
    let primary = ws.primary();
    let declaration = project_declaration(Path::new(&primary.worktree), Path::new(&primary.path));
    let (project, gate) = match &declaration {
        Some(declaration) => (
            declaration.tools.clone(),
            gate_of(&trust, &declaration.repo, &declaration.hash),
        ),
        None => (Tools::default(), Gate::Trusted),
    };
    let workspace = ws.tools();
    Ok(WorkspaceTools {
        mcp: axis_provenance(
            &global.mcp,
            &project.mcp,
            gate,
            &workspace.mcp,
            &mcp_base,
            &mcp_universe,
        ),
        plugins: axis_provenance(
            &global.plugins,
            &project.plugins,
            gate,
            &workspace.plugins,
            &[],
            &plugin_hub,
        ),
        // Standalone skills ride the plugin hub as `skill-<id>`, so the skills axis filters it too.
        skills: axis_provenance(
            &global.skills,
            &project.skills,
            gate,
            &workspace.skills,
            &[],
            &plugin_hub,
        ),
    })
}

/// The CLI-inherited servers of one workspace, absent from the hub, for the composer's picker rows
/// and button gating (ADR 0046). Discovery reads Claude's configuration, so a Codex conversation has
/// no inherited base and its rows stay hub-only.
#[tauri::command]
pub fn mcp_inherited(
    state: State<AppState>,
    id: String,
    agent: Option<ProviderId>,
) -> Vec<crate::mcp::Server> {
    // Discovery reads the CLI configuration files; it must not hold the board mutex.
    let ws = {
        let board = lock(&state.board);
        board.workspaces.iter().find(|w| w.id == id).cloned()
    };
    let Some(ws) = ws else {
        return Vec::new();
    };
    if agent.unwrap_or(ws.agent) != ProviderId::Claude {
        return Vec::new();
    }
    crate::mcp::inherited_missing_hub(Path::new(&ws.worktree))
}

/// Model and effort changes require a process restart; the next message resumes the same transcript
/// with the new settings. Reject provider changes here as well as in the UI because providers
/// cannot resume each other's transcripts.
#[tauri::command]
pub fn set_tab_choice(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    tab: String,
    choice: Choice,
) -> Result<(), String> {
    let generation = lock(&state.telemetry).generation;
    {
        let mut board = lock(&state.board);
        let ws = board
            .workspace_mut(&id)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        ws.retune(&tab, choice)?;
    }
    chat::kill(&state, &tab);
    publish(&app);
    crate::telemetry::journey(
        &app,
        generation,
        &id,
        Some(&tab),
        crate::telemetry::Fact::ProviderSelected {
            scope: crate::telemetry::SelectionScope::Conversation,
        },
    );
    Ok(())
}

/// Allow manual unread marking when the person needs to return to an update later.
#[tauri::command]
pub fn set_unread(app: AppHandle, state: State<AppState>, id: String, unread: bool) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.unread = unread;
        }
    }
    publish(&app);
}

/// Persist sharing intent so reopening the app restores it. The frontend owns relay announcements
/// and stream forwarding.
#[tauri::command]
pub fn set_shared(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    shared: bool,
    audience: Option<Vec<String>>,
    remote_control: bool,
    team: Option<String>,
) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.shared = shared;
            ws.share_team = if shared { team } else { None };
            ws.audience = if shared { audience } else { None };
            ws.remote_control = shared && remote_control;
        }
    }
    publish(&app);
}

/// The visible workspace must not acquire unread status for updates the person is already watching.
#[tauri::command]
pub fn look_at(app: AppHandle, state: State<AppState>, id: Option<String>) {
    *lock(&state.looking) = id.clone();
    let Some(id) = id else { return };
    let had = {
        let mut board = lock(&state.board);
        match board.workspace_mut(&id) {
            Some(ws) => std::mem::replace(&mut ws.unread, false),
            None => false,
        }
    };
    if had {
        publish(&app);
    }
}

/// Remove the workspace from the board while preserving its worktree and branch. Deleting work
/// requires a separate decision.
#[tauri::command]
pub fn remove_workspace(app: AppHandle, state: State<AppState>, id: String) {
    dock::kill_docks(&state, &id);
    let (dead, removed): (Vec<String>, bool) = {
        let mut board = lock(&state.board);
        let removed = board.workspaces.iter().any(|ws| ws.id == id);
        let dead = board
            .workspace_mut(&id)
            .map(|ws| ws.tabs.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default();
        board.workspaces.retain(|w| w.id != id);
        (dead, removed)
    };
    stop(&state, &dead);
    if removed {
        crate::plugins::forget_codex_workspace(&id);
    }
    publish(&app);
}

/* ---------- disk cleanup ---------- */

/// Worktree disk usage and cleanup eligibility. A blocked reason is an error code translated by the
/// frontend.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleanable {
    pub id: String,
    pub title: String,
    pub repo_name: String,
    pub branch: String,
    pub worktree: String,
    /// Disk usage in kilobytes, including large ignored directories such as node_modules and
    /// target.
    pub size_kb: u64,
    pub pr: Option<u64>,
    pub blocked: Option<String>,
}

/// Scan archived worktrees when the cleanup screen opens, not during board redraws. Exclude
/// sessions in the original clone, and measure independent worktrees in parallel because each git
/// status and du call traverses its own tree.
#[tauri::command(async)]
pub fn cleanup_list(state: State<AppState>) -> Vec<Cleanable> {
    let mine: Vec<Workspace> = lock(&state.board)
        .workspaces
        .iter()
        .filter(|w| has_worktree(w))
        .cloned()
        .collect();

    std::thread::scope(|scope| {
        let handles: Vec<_> = mine
            .into_iter()
            .map(|ws| {
                scope.spawn(move || {
                    let wt = PathBuf::from(&ws.worktree);
                    let pr = ws.prs().next().map(|(_, p)| p.number);
                    Cleanable {
                        size_kb: size_of(&wt),
                        blocked: check(&ws).err(),
                        pr,
                        id: ws.id,
                        title: ws.title,
                        repo_name: ws.repo_name,
                        branch: ws.branch,
                        worktree: ws.worktree,
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// Only archived, uncleaned workspaces with separate worktrees own removable directories. The
/// original clone is never eligible.
fn has_worktree(ws: &Workspace) -> bool {
    ws.archived && !ws.cleaned && ws.worktree != ws.repo
}

/// Permanently remove the worktree and local branch while retaining the card. Force permits
/// explicitly approved loss of uncommitted or unmerged work, but never bypasses archiving or
/// permits deleting the original clone.
#[tauri::command(async)]
pub fn cleanup_worktree(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let ws = workspace_copy(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    if ws.cleaned {
        return Ok(());
    }
    if force {
        hard(&ws)?;
    } else {
        check(&ws)?;
    }
    if ws.multi() {
        validate_multi_root(&ws)?;
    }

    // Stop agents and docks before removing their directories. The archive script already ran
    // during archiving; starting it again would race Git while the worktree disappears.
    dock::kill_docks(&state, &id);
    let dead: Vec<String> = lock(&state.board)
        .workspace_mut(&id)
        .map(|ws| ws.tabs.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default();
    stop(&state, &dead);

    // Remove each repository's worktree through its own clone.
    for r in &ws.repos {
        let repo = PathBuf::from(&r.path);
        let wt = PathBuf::from(&r.worktree);
        if wt.exists() {
            // Force removes ignored files such as node_modules, target, and .env, plus uncommitted
            // changes only when the person explicitly approved losing them.
            let out = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["worktree", "remove", "--force"])
                .arg(&wt)
                .output()
                .map_err(|e| i18n::ta("err.git.spawn", &[("cause", e.to_string())]))?;
            if !out.status.success() {
                return Err(i18n::ta(
                    "err.git",
                    &[
                        ("command", "git worktree remove".into()),
                        (
                            "cause",
                            String::from_utf8_lossy(&out.stderr).trim().to_string(),
                        ),
                    ],
                ));
            }
        }
        // Use -D after checking merge safety, or when force explicitly permits loss. A branch
        // deletion failure can leave a harmless ref after successful worktree cleanup, so it must
        // not turn that cleanup into an error.
        if !ws.branch.is_empty() {
            let _ = git(&repo, &["branch", "-D", &ws.branch]);
        }
        let _ = git(&repo, &["worktree", "prune"]);
    }
    // Remove the application-owned directory that grouped the worktrees after its children are
    // gone.
    if ws.multi() {
        let _ = std::fs::remove_dir_all(&ws.worktree);
    }
    crate::plugins::forget_codex_workspace(&id);

    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.cleaned = true;
            for tab in &mut ws.tabs {
                tab.status = Status::Desligada;
                tab.note = None;
            }
        }
    }
    publish(&app);
    Ok(())
}

/// Return a translated error code when cleanup is unsafe. A missing worktree passes so cleanup can
/// reconcile the board with disk.
fn check(ws: &Workspace) -> Result<(), String> {
    hard(ws)?;
    // Every repository must pass before removing any part of a workspace.
    for r in &ws.repos {
        let wt = PathBuf::from(&r.worktree);
        if !wt.exists() {
            continue;
        }
        let dirty = git(&wt, &["status", "--porcelain"]).lines().count();
        if dirty > 0 {
            return Err(i18n::ta("err.cleanup.dirty", &[("n", dirty.to_string())]));
        }
        if !merged(r.pr.as_ref(), &wt) {
            return Err(i18n::ta(
                "err.cleanup.unmerged",
                &[("branch", ws.branch.clone())],
            ));
        }
    }
    Ok(())
}

/// Force never bypasses archiving or permits deleting the original clone.
fn hard(ws: &Workspace) -> Result<(), String> {
    // Archiving runs the repository's archive script while the worktree exists. Disk cleanup must
    // follow that step.
    if !ws.archived {
        return Err(i18n::t("err.cleanup.notArchived"));
    }
    if ws.worktree == ws.repo {
        return Err(i18n::t("err.cleanup.isRepo"));
    }
    Ok(())
}

/// Only the exact grouping directory produced by create_workspace may be removed directly. Require
/// every worktree to be an immediate child so edited or corrupted board data cannot authorize
/// arbitrary recursive deletion.
fn validate_multi_root(ws: &Workspace) -> Result<(), String> {
    use std::path::Component;

    let normal = |name: &str| {
        let mut components = Path::new(name).components();
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
    };
    if ws.repos.len() < 2 || ws.repos.iter().any(|repo| !normal(&repo.name)) {
        return Err(i18n::t("err.cleanup.badRoot"));
    }
    let names: Vec<String> = ws.repos.iter().map(|repo| repo.name.clone()).collect();
    let expected = paths::multi_dir(&names, &ws.branch);
    // Legacy imports keep worktrees in place. Accept only the exact legacy grouping path, using the
    // same validation as current paths.
    let legacy = paths::prometheus_multi_dir(&names, &ws.branch);
    let root = Path::new(&ws.worktree);
    let children_match = ws.repos.iter().all(|repo| {
        let child = Path::new(&repo.worktree);
        child.parent() == Some(root) && child.file_name() == Some(repo.name.as_ref())
    });
    if (root == expected || root == legacy) && children_match {
        Ok(())
    } else {
        Err(i18n::t("err.cleanup.badRoot"))
    }
}

/// Work is safe when GitHub reports the PR merged or Git reports the branch is an ancestor of its
/// target. The latter also supports merges outside GitHub and machines without gh.
fn merged(pr: Option<&Pr>, wt: &Path) -> bool {
    if pr.is_some_and(|pr| pr.merged()) {
        return true;
    }
    let head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let target = if head.is_empty() {
        "origin/main".to_string()
    } else {
        head
    };
    has_commit(wt, &target) && git_ok(wt, &["merge-base", "--is-ancestor", "HEAD", &target])
}

/// Use the system du command for kilobytes. Unknown size becomes zero because an unavailable
/// estimate must not block cleanup.
fn size_of(wt: &Path) -> u64 {
    if !wt.exists() {
        return 0;
    }
    let out = Command::new("du").arg("-sk").arg(wt).output().ok();
    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
    })
    .unwrap_or(0)
}

/// Keep launcher inputs together so adding an option does not expand the command's parameter list.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    project: String,
    /// Additional repositories receive sibling worktrees on the same branch. This requires
    /// worktrees because the agent needs a common parent directory.
    #[serde(default)]
    extras: Vec<String>,
    /// An empty branch name means no branch should be created.
    branch: String,
    /// The starting ref for a new branch.
    base: String,
    /// Create the branch in a separate worktree when enabled; otherwise switch the original clone.
    worktree: bool,
    title: String,
    stage: String,
    prompt: String,
    inject: Vec<String>,
    /// The Linear issue that opened the launcher, when present.
    #[serde(default)]
    issue: Option<crate::linear::IssueRef>,
    /// The launcher's "Start with" skill as `<package>/<skill>`; empty means none. Older callers
    /// omit it (ADR 0057).
    #[serde(default)]
    kickoff: String,
    /// Model and effort persist on the workspace. Plan mode applies only to the initial
    /// conversation.
    #[serde(flatten)]
    launch: Launch,
}

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

/// The hub IDs a session injects, one axis at a time, after composing the layers. `None` on an axis
/// means no layer declared it, so the provider keeps its own configuration; `Some` is the resolved
/// set to materialize (possibly empty, which injects nothing from the hub).
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedTools {
    pub(crate) mcp: Option<Vec<String>>,
    pub(crate) plugins: Option<Vec<String>>,
    pub(crate) skills: Option<Vec<String>>,
}

/// Resolve one axis. When every layer inherits, the axis stays `None` so the provider's own
/// configuration is preserved; otherwise the composed set — over the CLI-inherited base, when one
/// applies — is what the session injects.
fn resolve_axis(
    global: &Option<Selection>,
    project: &Option<Selection>,
    workspace: &Option<Selection>,
    base: &[String],
    universe: &[String],
) -> Option<Vec<String>> {
    if global.is_none() && project.is_none() && workspace.is_none() {
        None
    } else {
        Some(crate::selection::resolve_with_base(
            base, global, project, workspace, universe,
        ))
    }
}

/// Compose the three layers into the IDs a launch injects, keeping only IDs the universe still has.
/// The hubs, the mcp inherited base (ADR 0046) and the project layer come from the caller so the
/// chain stays testable without disk. The project layer must already be gated on trust before it
/// reaches here (ADR 0045, phase 4); see `trusted_project`.
pub(crate) fn resolve_tools(
    global: &Tools,
    project: &Tools,
    workspace: &Tools,
    mcp_base: &[String],
    mcp_universe: &[String],
    plugin_hub: &[String],
) -> ResolvedTools {
    ResolvedTools {
        mcp: resolve_axis(
            &global.mcp,
            &project.mcp,
            &workspace.mcp,
            mcp_base,
            mcp_universe,
        ),
        plugins: resolve_axis(
            &global.plugins,
            &project.plugins,
            &workspace.plugins,
            &[],
            plugin_hub,
        ),
        // Standalone skills ride the plugin hub as `skill-<id>`, so the skills axis filters too.
        skills: resolve_axis(
            &global.skills,
            &project.skills,
            &workspace.skills,
            &[],
            plugin_hub,
        ),
    }
}

/// The project `[tools]` declaration of a repository, with the identity and hash that key its trust
/// decision (ADR 0045). `project_declaration` returns `None` when the repository declares nothing,
/// so an absent layer needs no approval and simply inherits.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProjectDeclaration {
    /// Repository identity: the `origin` remote URL when one exists, else the clone's absolute path.
    pub repo: String,
    /// SHA-256 of the canonical form of the declared `[tools]` section.
    pub hash: String,
    /// The settings file that declared it, surfaced by the interface.
    pub file: Option<String>,
    /// The declared layer.
    pub tools: Tools,
}

/// Read the primary repository's `[tools]` and derive its trust identity. Returns `None` when the
/// repository declares no tools, so there is nothing to trust.
pub(crate) fn project_declaration(worktree: &Path, repo: &Path) -> Option<ProjectDeclaration> {
    let scripts = crate::scripts::read_for(worktree, repo);
    if scripts.tools == Tools::default() {
        return None;
    }
    Some(ProjectDeclaration {
        repo: repo_identity(repo),
        hash: tools_hash(&scripts.tools),
        file: scripts.file,
        tools: scripts.tools,
    })
}

/// Identify a repository for trust: its `origin` remote URL when one exists, else the clone's
/// absolute path. The URL survives a moved clone; the path covers a repository without a remote.
fn repo_identity(repo: &Path) -> String {
    let origin = git(repo, &["remote", "get-url", "origin"]);
    let origin = origin.trim();
    if !origin.is_empty() {
        return origin.to_string();
    }
    repo.canonicalize()
        .unwrap_or_else(|_| repo.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Hash the declared `[tools]` so a changed declaration re-prompts. Hashing the parsed structure,
/// not the raw TOML, keeps the decision stable across comments, key order and whitespace.
fn tools_hash(tools: &Tools) -> String {
    use sha2::{Digest, Sha256};
    let canonical = serde_json::to_string(tools).unwrap_or_default();
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

/// True when the person approved this exact declaration for this repository.
fn approved(trust: &[ToolTrust], repo: &str, hash: &str) -> bool {
    trust
        .iter()
        .any(|t| t.repo == repo && t.hash == hash && t.approved)
}

/// True when a decision (approval or rejection) exists for this exact declaration; a rejection
/// quiets the prompt until the hash changes (ADR 0045).
fn decided(trust: &[ToolTrust], repo: &str, hash: &str) -> bool {
    trust.iter().any(|t| t.repo == repo && t.hash == hash)
}

/// Whether a declared project layer may activate, for provenance labeling.
#[derive(Clone, Copy, PartialEq)]
enum Gate {
    Trusted,
    /// Nobody decided on the current hash yet, so the interface prompts.
    Pending,
    /// The current hash was explicitly rejected; resolved yet not injected, and no prompt.
    Rejected,
}

fn gate_of(trust: &[ToolTrust], repo: &str, hash: &str) -> Gate {
    match trust.iter().find(|t| t.repo == repo && t.hash == hash) {
        Some(decision) if decision.approved => Gate::Trusted,
        Some(_) => Gate::Rejected,
        None => Gate::Pending,
    }
}

/// The project layer a workspace may inject: the primary repository's declared `[tools]` when its
/// current hash is approved, otherwise nothing. Until approved, project-declared items stay out of
/// the composition entirely, so they never reach a spawn (ADR 0045, "Trust").
fn trusted_project(trust: &[ToolTrust], ws: &Workspace) -> Tools {
    let primary = ws.primary();
    let Some(declaration) =
        project_declaration(Path::new(&primary.worktree), Path::new(&primary.path))
    else {
        return Tools::default();
    };
    match approved(trust, &declaration.repo, &declaration.hash) {
        true => declaration.tools,
        false => Tools::default(),
    }
}

/// Load the hubs and resolve one workspace's tools, gating the project layer on its stored trust
/// decision. Production spawns call this; tests build a `ResolvedTools` directly to stay off disk.
pub(crate) fn resolve_workspace_tools(
    global: &Tools,
    trust: &[ToolTrust],
    ws: &Workspace,
    agent: ProviderId,
) -> ResolvedTools {
    let (mcp_base, mcp_universe) = mcp_base_and_universe(ws, agent);
    let plugin_hub: Vec<String> = crate::plugins::load().into_iter().map(|p| p.id).collect();
    resolve_tools(
        global,
        &trusted_project(trust, ws),
        &ws.tools(),
        &mcp_base,
        &mcp_universe,
        &plugin_hub,
    )
}

/// The mcp axis base and universe of one workspace (ADR 0046): the hub IDs plus the servers the CLI
/// itself loads for the workspace's working directory. Discovery reads Claude's configuration, so a
/// Codex conversation keeps the hub-only universe and today's coexistence behavior. A hub entry wins
/// an ID clash, keeping an imported server Prometeu-managed.
fn mcp_base_and_universe(ws: &Workspace, agent: ProviderId) -> (Vec<String>, Vec<String>) {
    let mut universe: Vec<String> = crate::mcp::available().into_iter().map(|s| s.id).collect();
    if agent != ProviderId::Claude {
        return (Vec::new(), universe);
    }
    let base: Vec<String> = crate::mcp::inherited(Path::new(&ws.worktree))
        .into_iter()
        .map(|s| s.id)
        .filter(|id| !universe.contains(id))
        .collect();
    universe.extend(base.iter().cloned());
    (base, universe)
}

/// Where one effective item came from in the chain, so the picker shows the result without
/// opening each layer (ADR 0045). `Removed` and `Pending` items are listed but not injected.
#[derive(serde::Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// Active because a layer above the workspace selected it.
    Inherited,
    /// Active because the workspace layer added it.
    Added,
    /// Inactive because the workspace layer removed an otherwise-inherited item.
    Removed,
    /// Declared by the project but not yet trusted, so resolved yet not injected.
    Pending,
    /// Declared by the project and explicitly rejected for its current hash; resolved yet not
    /// injected, and the interface stops prompting until the declaration changes.
    Rejected,
    /// Active because the person's CLI configuration loads it, not a Prometeu hub choice; the
    /// visible inherited base of the mcp axis (ADR 0046).
    Cli,
}

/// One item of the axis universe with its provenance.
#[derive(serde::Serialize)]
pub struct EffectiveItem {
    pub id: String,
    pub provenance: Provenance,
}

/// Classify every id of the axis universe for the picker. `project` is the declared layer and
/// `gate` says whether it may activate; while gated, the items it declares appear as `Pending` or
/// `Rejected`. The workspace layer is the person's own action, so an item it adds is active even
/// while the project declaration is still gated. `base` holds the CLI-inherited ids (ADR 0046):
/// an active one is labeled `Cli`, and the workspace can drop it with a removal like any inherited
/// item. Items with no story (off and untouched) are omitted.
fn axis_provenance(
    global: &Option<Selection>,
    project: &Option<Selection>,
    gate: Gate,
    workspace: &Option<Selection>,
    base: &[String],
    universe: &[String],
) -> Vec<EffectiveItem> {
    let gated = (gate == Gate::Trusted).then(|| project.clone()).flatten();
    let effective = crate::selection::resolve_with_base(base, global, &gated, workspace, universe);
    // What the project layer alone would contribute, to label gated items.
    let declared = crate::selection::resolve(&None, project, &None, universe);
    universe
        .iter()
        .filter_map(|id| {
            let on = effective.contains(id);
            let added = workspace.as_ref().is_some_and(|s| s.add.contains(id));
            let removed = workspace.as_ref().is_some_and(|s| s.remove.contains(id));
            let provenance = if on && added {
                Provenance::Added
            } else if on && base.contains(id) {
                Provenance::Cli
            } else if on {
                Provenance::Inherited
            } else if removed {
                Provenance::Removed
            } else if gate != Gate::Trusted && declared.contains(id) {
                match gate {
                    Gate::Rejected => Provenance::Rejected,
                    _ => Provenance::Pending,
                }
            } else {
                return None;
            };
            Some(EffectiveItem {
                id: id.clone(),
                provenance,
            })
        })
        .collect()
}

impl ResolvedTools {
    /// Plugins and standalone skills share the plugin-package pipeline, so a spawn materializes the
    /// two resolved axes together. `None` on both preserves the CLI's own plugins; otherwise the
    /// selected packages are the union, in plugin-then-skill order.
    pub(crate) fn plugin_packages(&self) -> Option<Vec<String>> {
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

impl Workspace {
    /// This workspace's three axes as the workspace layer of the tool selection.
    fn tools(&self) -> Tools {
        Tools {
            mcp: self.mcp.clone(),
            plugins: self.plugins.clone(),
            skills: self.skills.clone(),
        }
    }

    /// Default conversations inherit the workspace's model and effort and carry the already resolved
    /// tools. Plan mode remains an explicit launcher choice.
    pub(crate) fn launch(&self, tools: &ResolvedTools) -> Launch {
        Launch {
            agent: self.agent,
            model: self.model.clone(),
            effort: self.effort.clone(),
            plan: false,
            mcp: tools.mcp.clone(),
            plugins: tools.plugins.clone(),
            skills: tools.skills.clone(),
            ..Default::default()
        }
    }

    /// Resume with the tab's model override or workspace defaults. Ordinary tabs carry the resolved
    /// tools; tasks retain their frozen profile, instructions, and permissions.
    pub(crate) fn launch_of(&self, tab: &str, tools: &ResolvedTools) -> Launch {
        if let Some(run) = self
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.task.as_ref())
        {
            return Launch {
                mcp: run.profile.mcp.clone(),
                plugins: run.profile.plugins.clone(),
                permission: Some(run.profile.permission),
                instructions: crate::actions::instructions(&run.profile),
                config_scope: Some(tab.to_string()),
                ..Launch::from(run.profile.choice.clone())
            };
        }
        let mut launch = self.launch_with(
            self.tabs
                .iter()
                .find(|t| t.id == tab)
                .and_then(|t| t.choice.clone()),
            tools,
        );
        if launch.agent == ProviderId::Antigravity {
            if let Some(tab) = self.tabs.iter().find(|t| t.id == tab) {
                launch.plan = tab.plan;
                launch.permission = tab.permission;
            }
        }
        launch
    }

    /// Model overrides preserve the resolved tool selection for new and resumed tabs.
    fn launch_with(&self, choice: Option<Choice>, tools: &ResolvedTools) -> Launch {
        choice.map_or_else(
            || self.launch(tools),
            |choice| Launch {
                mcp: tools.mcp.clone(),
                plugins: tools.plugins.clone(),
                skills: tools.skills.clone(),
                ..Launch::from(choice)
            },
        )
    }

    /// Persist model and effort overrides on the tab. Choosing the workspace defaults clears the
    /// override so the tab follows later changes. Reject provider changes because providers cannot
    /// resume each other's transcripts.
    pub fn retune(&mut self, tab: &str, choice: Choice) -> Result<(), String> {
        if self.tabs.iter().any(|t| t.id == tab && t.task.is_some()) {
            return Err(i18n::t("err.actions.frozen"));
        }
        // The tab is ordinary, so its provider is the model override or the workspace default.
        let current = self
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.choice.clone())
            .map_or(self.agent, |c| c.agent);
        if current != choice.agent {
            return Err(i18n::t("err.session.otherAgent"));
        }
        let follows = choice.agent == self.agent
            && choice.model == self.model
            && choice.effort == self.effort;
        let tab = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab)
            .ok_or_else(|| i18n::t("err.session.noTab"))?;
        tab.choice = (!follows).then_some(choice);
        Ok(())
    }
}

/// Publish a preparing card before slow Git and filesystem work. Resolve enough metadata to render
/// it, then run preparation on a worker thread and publish each result without blocking the
/// launcher response.
#[tauri::command(async)]
pub fn create_workspace(
    app: AppHandle,
    state: State<AppState>,
    draft: Draft,
    cols: u16,
    rows: u16,
) -> Result<Workspace, String> {
    create_workspace_owned(app, state, draft, cols, rows, None)
}

pub(crate) fn create_workspace_owned(
    app: AppHandle,
    state: State<AppState>,
    draft: Draft,
    cols: u16,
    rows: u16,
    delegation: Option<crate::delegation::Delegation>,
) -> Result<Workspace, String> {
    let generation = lock(&state.telemetry).generation;
    // Reject missing selection before publishing a card or preparing filesystem state.
    crate::accounts::active(draft.launch.agent)?;
    // A kickoff rides the plugin-package pipeline, so it needs the same capability as plugin
    // selection, and it must name a skill that is still installed (ADR 0057).
    if crate::kickoff::resolve(
        &draft.kickoff,
        &crate::skills::load(),
        &crate::plugins::load(),
    )?
    .is_some()
        && !crate::agents::capabilities(draft.launch.agent).workspace_plugin_selection
    {
        return Err(i18n::t("err.kickoff.unsupported"));
    }
    let repo_path = PathBuf::from(expand(&draft.project));
    let repo_name = repo_named(&repo_path)?;

    // Validate additional repositories too. Reject duplicate clones and duplicate directory names
    // that would share a worktree path.
    let mut extras: Vec<(PathBuf, String)> = Vec::new();
    for extra in &draft.extras {
        let path = PathBuf::from(expand(extra));
        let name = repo_named(&path)?;
        if path == repo_path || extras.iter().any(|(p, n)| *p == path || *n == name) {
            return Err(i18n::ta("err.session.dupRepo", &[("name", name)]));
        }
        extras.push((path, name));
    }
    if !extras.is_empty() && !draft.worktree {
        return Err(i18n::t("err.session.extrasNeedWorktree"));
    }

    // Reject branch and worktree requests for non-Git folders at this boundary, even when callers
    // bypass the launcher controls.
    if draft.worktree || !draft.branch.trim().is_empty() {
        for path in std::iter::once(&repo_path).chain(extras.iter().map(|(p, _)| p)) {
            if !path.join(".git").exists() {
                return Err(i18n::ta(
                    "err.session.notGit",
                    &[("path", path.display().to_string())],
                ));
            }
        }
    }

    // An empty branch keeps the clone's current checkout. A separate worktree requires a branch,
    // which also determines its path before creation. Multiple repositories run under a common
    // directory containing their worktrees.
    let (root, branch) = match (draft.worktree, draft.branch.trim().is_empty()) {
        (true, true) => return Err(i18n::t("err.session.worktreeNeedsBranch")),
        (true, false) if extras.is_empty() => (
            paths::worktree_dir(&repo_name, &draft.branch),
            draft.branch.clone(),
        ),
        (true, false) => {
            let names: Vec<String> = std::iter::once(repo_name.clone())
                .chain(extras.iter().map(|(_, n)| n.clone()))
                .collect();
            (
                paths::multi_dir(&names, &draft.branch),
                draft.branch.clone(),
            )
        }
        (false, false) => (repo_path.clone(), draft.branch.clone()),
        // Without a new branch, use the clone's current branch. Non-Git folders have none.
        (false, true) => (
            repo_path.clone(),
            head_branch(&repo_path).unwrap_or_default(),
        ),
    };
    // The launcher selects the primary repository's base. Other repositories use their own default
    // bases, persisted separately for later diff comparisons.
    let repos: Vec<Repo> = match extras.is_empty() {
        true => vec![Repo {
            path: repo_path.display().to_string(),
            name: repo_name.clone(),
            worktree: root.display().to_string(),
            base: draft.base.clone(),
            pr: None,
        }],
        false => std::iter::once((repo_path.clone(), repo_name.clone(), draft.base.clone()))
            .chain(extras.into_iter().map(|(path, name)| {
                let base = delegation
                    .as_ref()
                    .and_then(|d| d.repository_heads.get(&path.display().to_string()))
                    .cloned()
                    .unwrap_or_else(|| default_base(&path));
                (path, name, base)
            }))
            .map(|(path, name, base)| Repo {
                worktree: root.join(&name).display().to_string(),
                path: path.display().to_string(),
                name,
                base,
                pr: None,
            })
            .collect(),
    };

    // Reject branches already checked out elsewhere before publishing a card. Reading worktree
    // metadata is cheap and avoids a preparation failure for a known conflict.
    if draft.worktree {
        for r in &repos {
            branch_free(Path::new(&r.path), &branch, Path::new(&r.worktree))?;
        }
    }

    // Allocate the port before setup and run scripts need it. Release the board lock before binding
    // sockets so allocation does not block publication from other sessions.
    let taken: Vec<u16> = lock(&state.board)
        .workspaces
        .iter()
        .filter_map(|w| w.port)
        .collect();
    let port = scripts::alloc_port(&root, &taken);

    let mut ws = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        title: if draft.title.trim().is_empty() {
            branch.clone()
        } else {
            draft.title.clone()
        },
        issue: draft.issue.clone(),
        project: repo_path.display().to_string(),
        repo: repo_path.display().to_string(),
        repo_name,
        branch,
        worktree: root.display().to_string(),
        repos,
        stage: draft.stage.clone(),
        archived: false,
        pinned: false,
        unread: false,
        pr: None,
        cleaned: false,
        shared: false,
        share_team: None,
        audience: None,
        remote_control: false,
        preparing: true,
        failed: None,
        agent: draft.launch.agent,
        model: draft.launch.model.clone(),
        effort: draft.launch.effort.clone(),
        // The launcher's explicit selections become the workspace layer; `None` keeps inheriting the
        // layers above. Standalone skills are normalized onto their own axis below.
        mcp: draft.launch.mcp.clone().map(Selection::only),
        plugins: draft.launch.plugins.clone().map(Selection::only),
        skills: draft.launch.skills.clone().map(Selection::only),
        port,
        active: None,
        tabs: Vec::new(),
    };
    crate::state::split_skills(&mut ws.plugins, &mut ws.skills);

    // Publish ownership atomically with the workspace, before preparation can start the agent.
    let delegated = delegation.is_some();
    {
        let mut board = lock(&state.board);
        if let Some(mut delegation) = delegation {
            delegation.workspace = ws.id.clone();
            board.delegations.push(delegation);
        }
        board.workspaces.push(ws.clone());
    }
    publish(&app);

    if delegated {
        if let Err(error) = crate::state::persist_now(&app) {
            if let Some(ws) = lock(&state.board).workspace_mut(&ws.id) {
                ws.preparing = false;
                ws.failed = Some(error.clone());
            }
            publish(&app);
            return Err(error);
        }
    }

    // Generate a better title from the full prompt while filesystem preparation runs. Naming does
    // not require a worktree. Workspaces created from issues retain their issue titles.
    if ws.issue.is_none() {
        crate::naming::rename_later(&app, &ws.id, &draft.prompt, &ws.title, &draft.launch);
    }

    crate::telemetry::journey(
        &app,
        generation,
        &ws.id,
        None,
        crate::telemetry::Fact::WorkspaceCreated {
            mode: if draft.worktree {
                crate::telemetry::CreationMode::Worktree
            } else {
                crate::telemetry::CreationMode::Repository
            },
        },
    );
    let (bg, id) = (app.clone(), ws.id.clone());
    std::thread::spawn(move || prepare(&bg, &id, draft, cols, rows, generation));

    Ok(ws)
}

/// Prepare directories, the agent, and setup away from the launcher thread. Preserve failed cards
/// and their errors so the person can resolve the cause.
fn prepare(app: &AppHandle, id: &str, draft: Draft, cols: u16, rows: u16, generation: u64) {
    let Err(err) = build(app, id, &draft, cols, rows, generation) else {
        return;
    };
    let state = app.state::<AppState>();
    {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(id) else {
            return;
        };
        ws.preparing = false;
        ws.failed = Some(err);
        if let Some(d) = board.delegations.iter_mut().find(|d| d.workspace == id) {
            if let Some(run) = d.executions.last_mut() {
                run.state = "completed".into();
                run.outcome = Some("error".into());
            }
        }
    }
    publish(app);
}

fn build(
    app: &AppHandle,
    id: &str,
    draft: &Draft,
    cols: u16,
    rows: u16,
    generation: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (repo, root, branch, repos) = {
        let board = lock(&state.board);
        let ws = board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        (
            PathBuf::from(&ws.repo),
            PathBuf::from(&ws.worktree),
            ws.branch.clone(),
            ws.repos.clone(),
        )
    };

    // Create one worktree per repository on the shared branch, using each repository's saved base.
    // Existing branches retain their own history.
    if draft.worktree {
        // If any repository fails, roll back only the worktrees created by this preparation. A
        // partially assembled workspace is not usable.
        let mut feitos: Vec<(PathBuf, PathBuf)> = Vec::new();
        for r in &repos {
            let (clone, dest) = (PathBuf::from(&r.path), PathBuf::from(&r.worktree));
            match add_worktree(&clone, &branch, &r.base, &dest) {
                Ok(true) => feitos.push((clone, dest)),
                Ok(false) => {}
                Err(e) => {
                    undo_worktrees(&feitos);
                    if repos.len() > 1 {
                        let _ = std::fs::remove_dir(&root);
                    }
                    return Err(e);
                }
            }
        }
        if repos.len() > 1 {
            describe_root(&root, &repos, &branch);
        }
    } else if !draft.branch.trim().is_empty() {
        switch_branch(&repo, &branch, &draft.base)?;
    }

    // The first conversation keeps the launcher's model, instructions, and permissions, but its
    // tool axes resolve through the global and workspace layers like any other spawn. Resolution
    // runs git subprocesses and reads CLI configuration, so it happens off the board mutex.
    let (mut launch, ws) = {
        let (global, trust, ws) = {
            let board = lock(&state.board);
            (
                board.tools.clone(),
                board.tool_trust.clone(),
                board.workspaces.iter().find(|w| w.id == id).cloned(),
            )
        };
        let mut launch = draft.launch.clone();
        if let Some(ws) = &ws {
            let tools = resolve_workspace_tools(&global, &trust, ws, launch.agent);
            launch.mcp = tools.mcp;
            launch.plugins = tools.plugins;
            launch.skills = tools.skills;
        }
        (launch, ws)
    };
    // A kickoff joins only this conversation's resolved set and opens its first message; no
    // selection layer changes (ADR 0057).
    let kickoff = crate::kickoff::resolve(
        &draft.kickoff,
        &crate::skills::load(),
        &crate::plugins::load(),
    )?;
    let opening = kickoff.as_ref().map(|kickoff| {
        let hub: Vec<String> = crate::plugins::load().into_iter().map(|p| p.id).collect();
        crate::kickoff::ensure(&mut launch, &kickoff.id, &hub);
        let artifacts = ws.as_ref().and_then(artifacts_of);
        crate::kickoff::opening_line(kickoff, artifacts.as_deref())
    });
    let delegated_id = lock(&state.board)
        .delegations
        .iter()
        .find(|d| d.workspace == id)
        .map(|d| d.id.clone());
    let mut tab = spawn_tab_with_id(
        app,
        &state,
        id,
        "",
        first_message(opening.as_deref(), &draft.prompt, &draft.inject),
        &launch,
        // The first conversation uses the launch settings already saved on the workspace, so it
        // needs no tab override.
        None,
        delegated_id,
    )?;
    // Remember the kickoff on the tab so a resumed process keeps the method's package.
    tab.kickoff = kickoff.map(|kickoff| kickoff.id);

    // If the workspace was removed during preparation, stop the newly started agent instead of
    // leaving a hidden process.
    let ws = {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(id) else {
            drop(board);
            chat::kill(&state, &tab.id);
            return Ok(());
        };
        ws.preparing = false;
        ws.active = Some(tab.id.clone());
        ws.tabs.push(tab);
        ws.clone()
    };
    publish(app);
    if let Some(tab) = ws.tabs.last() {
        crate::telemetry::journey(
            app,
            generation,
            id,
            Some(&tab.id),
            crate::telemetry::Fact::ConversationCreated {},
        );
        crate::telemetry::journey(
            app,
            generation,
            id,
            Some(&tab.id),
            crate::telemetry::Fact::ProviderSelected {
                scope: crate::telemetry::SelectionScope::Workspace,
            },
        );
    }

    // Copy secrets and other requested ignored files before running setup. Start the agent but hold
    // its first message until setup completes, so tools cannot run against missing dependencies.
    // Setup failures preserve the worktree and remain visible on the Setup tab.
    let _ = dock::start_setup(app, &state, &ws, cols, rows);
    // Hold the prompt while setup runs; otherwise send it immediately.
    if let Some(tab) = ws.tabs.last() {
        chat::ready_now(app, &tab.id);
    }

    Ok(())
}

/* ---------- tabs ---------- */

/// Open another conversation in the same worktree. An explicit choice comes from the new-tab model
/// menu; the shortcut and plain button inherit workspace defaults.
#[tauri::command]
pub fn new_tab(
    app: AppHandle,
    state: State<AppState>,
    workspace: String,
    prompt: String,
    choice: Option<Choice>,
) -> Result<Tab, String> {
    let generation = lock(&state.telemetry).generation;
    let explicit_choice = choice.is_some();
    // Neither path restores plan mode; it belongs to the initial request.
    let (launch, choice) = {
        let (global, trust, ws) = {
            let board = lock(&state.board);
            let Some(ws) = board.workspaces.iter().find(|w| w.id == workspace).cloned() else {
                return Err(i18n::t("err.session.noWorkspace"));
            };
            (board.tools.clone(), board.tool_trust.clone(), ws)
        };
        if ws.cleaned {
            return Err(i18n::t("err.session.cleaned"));
        }
        // Clear choices that match workspace defaults instead of storing duplicate settings.
        let choice =
            choice.filter(|c| c.agent != ws.agent || c.model != ws.model || c.effort != ws.effort);
        let agent = choice.as_ref().map_or(ws.agent, |c| c.agent);
        let tools = resolve_workspace_tools(&global, &trust, &ws, agent);
        let launch = ws.launch_with(choice.clone(), &tools);
        (launch, choice)
    };

    // Without a prompt, keep the title empty so the UI displays the model.
    let title = if prompt.trim().is_empty() {
        String::new()
    } else {
        tab_title(&prompt)
    };
    let pending = (!prompt.trim().is_empty()).then(|| prompt.trim().to_string());
    let tab = spawn_tab(&app, &state, &workspace, &title, pending, &launch, choice)?;

    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.active = Some(tab.id.clone());
            ws.tabs.push(tab.clone());
        }
    }
    publish(&app);
    crate::telemetry::journey(
        &app,
        generation,
        &workspace,
        Some(&tab.id),
        crate::telemetry::Fact::ConversationCreated {},
    );
    if explicit_choice {
        crate::telemetry::journey(
            &app,
            generation,
            &workspace,
            Some(&tab.id),
            crate::telemetry::Fact::ProviderSelected {
                scope: crate::telemetry::SelectionScope::Conversation,
            },
        );
    }
    chat::ready_now(&app, &tab.id);
    Ok(tab)
}

#[tauri::command]
pub fn close_tab(app: AppHandle, state: State<AppState>, workspace: String, tab: String) {
    stop(&state, std::slice::from_ref(&tab));
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.tabs.retain(|t| t.id != tab);
            if ws.active.as_deref() == Some(tab.as_str()) {
                ws.active = ws.tabs.first().map(|t| t.id.clone());
            }
        }
    }
    publish(&app);
}

#[tauri::command]
pub fn focus_tab(app: AppHandle, state: State<AppState>, workspace: String, tab: String) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.active = Some(tab);
        }
    }
    publish(&app);
}

/// Initial tab titles come from the prompt, or remain empty so the UI shows the model. An empty
/// rename cancels rather than erasing an existing title.
#[tauri::command]
pub fn rename_tab(
    app: AppHandle,
    state: State<AppState>,
    workspace: String,
    tab: String,
    title: String,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    {
        let mut board = lock(&state.board);
        if let Some(t) = board
            .workspace_mut(&workspace)
            .and_then(|ws| ws.tabs.iter_mut().find(|t| t.id == tab))
        {
            t.title = title.to_string();
        }
    }
    publish(&app);
}

/// Restart the process when chat_send receives a message for a stopped tab.
pub fn revive(app: &AppHandle, state: &State<AppState>, tab: &str) -> Result<bool, String> {
    let (workspace, worktree, mut launch, cleaned, agent_session, kickoff_lost) = {
        // Snapshot under the lock; tool resolution runs git subprocesses and reads CLI
        // configuration, which must not block board events.
        let (global, trust, snapshot) = {
            let board = lock(&state.board);
            let snapshot = board.workspace_of(tab).map(|w| {
                let previous = w
                    .tabs
                    .iter()
                    .find(|t| t.id == tab)
                    .and_then(|t| t.agent_session.clone());
                (w.clone(), previous)
            });
            (board.tools.clone(), board.tool_trust.clone(), snapshot)
        };
        let Some((ws, previous)) = snapshot else {
            return Err(i18n::t("err.session.noTab"));
        };
        let agent = ws.launch_of(tab, &ResolvedTools::default()).agent;
        let tools = resolve_workspace_tools(&global, &trust, &ws, agent);
        let mut launch = ws.launch_of(tab, &tools);
        // A conversation started from a skill keeps that skill's package across resumes, validated
        // like at creation; a skill no longer installed never blocks the resume (ADR 0057).
        let kickoff_lost = ws
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.kickoff.as_deref())
            .and_then(|kickoff| {
                crate::kickoff::resume(
                    &mut launch,
                    kickoff,
                    &crate::skills::load(),
                    &crate::plugins::load(),
                )
            });
        (
            ws.id.clone(),
            PathBuf::from(&ws.worktree),
            launch,
            ws.cleaned,
            previous,
            kickoff_lost,
        )
    };
    if let Some(delegation) = lock(&state.board).delegations.iter().find(|d| d.id == tab) {
        launch.permission = delegation.permission;
    }
    if cleaned {
        return Err(i18n::t("err.session.cleaned"));
    }
    if !worktree.exists() {
        return Err(i18n::ta(
            "err.session.noWorktree",
            &[("path", worktree.display().to_string())],
        ));
    }

    // Remove the previous Chat handle even when its process has already exited.
    chat::kill(state, tab);

    // Claude conversations without a transcript must restart with their existing ID. Codex instead
    // resumes only when its previously returned thread identity is known.
    let (resume, handle) = match launch.agent {
        ProviderId::RetiredGemini => return Err(i18n::t("err.provider.retired")),
        ProviderId::Antigravity => (
            agent_session.is_some(),
            crate::antigravity::spawn(app, tab, &workspace, &worktree, agent_session, &launch)?,
        ),
        ProviderId::Codex => (
            agent_session.is_some(),
            crate::codex::spawn(app, tab, &workspace, &worktree, agent_session, &launch)?,
        ),
        ProviderId::Claude => {
            let resume = paths::transcript(tab, &worktree).exists();
            (
                resume,
                crate::claude::spawn(app, tab, &worktree, resume, &launch)?,
            )
        }
    };
    if let Some(notice) = kickoff_lost {
        handle.warn("kickoff.missing", &notice);
    }
    lock(&state.chats).insert(tab.to_string(), handle);
    {
        let mut board = lock(&state.board);
        if let Some(t) = board.tab_mut(tab) {
            t.status = Status::Pronta;
            t.note = None;
        }
    }
    publish(app);
    chat::ready_now(app, tab);
    Ok(resume)
}

fn spawn_tab(
    app: &AppHandle,
    state: &State<AppState>,
    workspace: &str,
    title: &str,
    pending_prompt: Option<String>,
    launch: &Launch,
    choice: Option<Choice>,
) -> Result<Tab, String> {
    spawn_tab_with_id(
        app,
        state,
        workspace,
        title,
        pending_prompt,
        launch,
        choice,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_tab_with_id(
    app: &AppHandle,
    state: &State<AppState>,
    workspace: &str,
    title: &str,
    pending_prompt: Option<String>,
    launch: &Launch,
    choice: Option<Choice>,
    id: Option<String>,
) -> Result<Tab, String> {
    let worktree = lock(&state.board)
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace)
        .map(|workspace| PathBuf::from(&workspace.worktree))
        .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // The selected provider determines which CLI runs; the tab identity stays the same.
    let handle = match launch.agent {
        ProviderId::Antigravity => {
            crate::antigravity::spawn(app, &id, workspace, &worktree, None, launch)?
        }
        ProviderId::RetiredGemini => return Err(i18n::t("err.provider.retired")),
        ProviderId::Codex => crate::codex::spawn(app, &id, workspace, &worktree, None, launch)?,
        ProviderId::Claude => crate::claude::spawn(app, &id, &worktree, false, launch)?,
    };
    lock(&state.chats).insert(id.clone(), handle);
    // The caller publishes the tab before chat::ready_now releases its pending prompt.
    Ok(Tab {
        plan: launch.agent == ProviderId::Antigravity && launch.plan,
        permission: launch.permission,
        task: None,
        id,
        agent_session: None,
        title: title.to_string(),
        status: Status::Pronta,
        note: None,
        pending_prompt,
        tokens: None,
        context_tokens: None,
        choice,
        kickoff: None,
    })
}

/* ---------- plumbing ---------- */

/// The declared `[method] artifacts` path of the primary repository, relative to the agent's
/// working directory: the path itself for one repository, or under the primary repository's folder
/// when several share a parent directory.
fn artifacts_of(ws: &Workspace) -> Option<String> {
    let primary = ws.primary();
    let declared = crate::scripts::read_for(Path::new(&primary.worktree), Path::new(&primary.path))
        .artifacts?;
    let inside = Path::new(&primary.worktree)
        .strip_prefix(&ws.worktree)
        .unwrap_or(Path::new(""));
    Some(inside.join(declared).to_string_lossy().into_owned())
}

/// Open with the kickoff line when there is one, then attach initial context through @path
/// mentions supported by the agent, then the person's prompt.
fn first_message(opening: Option<&str>, prompt: &str, inject: &[String]) -> Option<String> {
    let mentions = inject
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("@{}", p.trim()))
        .collect::<Vec<_>>()
        .join(" ");

    let parts: Vec<String> = [
        opening.unwrap_or_default().to_string(),
        mentions,
        prompt.trim().to_string(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Derive a short tab title from the prompt's first line; multiple tabs share limited horizontal
/// space.
fn tab_title(prompt: &str) -> String {
    let line = prompt.trim().lines().next().unwrap_or("").trim();
    match line.chars().count() > 34 {
        true => line.chars().take(33).collect::<String>() + "…",
        false => line.to_string(),
    }
}

/// Use base only for new branches; existing branches retain their history. Return whether this call
/// created the worktree, because adopted directories must survive rollback of sibling repositories.
fn add_worktree(repo: &Path, branch: &str, base: &str, dest: &Path) -> Result<bool, String> {
    // Reuse existing directories only when they contain the requested branch.
    if dest.exists() {
        return match head_branch(dest) {
            Some(head) if head == branch => Ok(false),
            Some(head) => Err(i18n::ta(
                "err.session.worktreeElsewhere",
                &[
                    ("path", dest.display().to_string()),
                    ("head", head),
                    ("branch", branch.to_string()),
                ],
            )),
            None => Err(i18n::ta(
                "err.session.worktreeDetached",
                &[("path", dest.display().to_string())],
            )),
        };
    }
    branch_free(repo, branch, dest)?;

    let parent = dest
        .parent()
        .ok_or_else(|| i18n::t("err.session.noParent"))?;
    // Let Git create the worktree directory and roll back newly created parent directories on
    // failure, avoiding empty orphan paths.
    let abertas = open_dirs(parent)?;

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("worktree").arg("add");
    if has_commit(repo, &format!("refs/heads/{branch}")) {
        cmd.arg(dest).arg(branch);
    } else {
        cmd.arg("-b").arg(branch).arg(dest);
        if !base.is_empty() {
            if let Err(e) = prepare_base(repo, base) {
                close_dirs(&abertas);
                return Err(e);
            }
            cmd.arg(base);
        }
    }

    let out = cmd.output().map_err(|e| {
        close_dirs(&abertas);
        i18n::ta("err.git.spawn", &[("cause", e.to_string())])
    })?;
    if !out.status.success() {
        close_dirs(&abertas);
        return Err(i18n::ta(
            "err.git",
            &[
                ("command", "git worktree add".into()),
                (
                    "cause",
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ),
            ],
        ));
    }
    Ok(true)
}

/// Find where this branch is checked out, including the original clone and its worktrees.
fn worktree_of_branch(repo: &Path, branch: &str) -> Option<PathBuf> {
    let want = format!("branch refs/heads/{branch}");
    let listed = git(repo, &["worktree", "list", "--porcelain"]);
    let mut at = None;
    for line in listed.lines() {
        match line.strip_prefix("worktree ") {
            Some(path) => at = Some(PathBuf::from(path)),
            None if line == want => return at,
            None => {}
        }
    }
    None
}

/// Git permits a branch in only one checkout. Workspaces from the same issue can request the same
/// branch at different paths; report the occupying path so the person can resolve the conflict.
fn branch_free(repo: &Path, branch: &str, dest: &Path) -> Result<(), String> {
    match worktree_of_branch(repo, branch) {
        Some(at) if !same_path(&at, dest) => Err(i18n::ta(
            "err.session.branchBusy",
            &[
                ("name", repo_label(repo)),
                ("branch", branch.to_string()),
                ("path", at.display().to_string()),
            ],
        )),
        _ => Ok(()),
    }
}

/// Canonicalize paths because Git resolves worktree paths, while HOME may contain aliases such as
/// /var and /private/var.
fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

/// Create missing parent directories and return them from outermost to innermost for close_dirs
/// rollback.
fn open_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut novas = Vec::new();
    let mut at = Some(dir);
    while let Some(p) = at.filter(|p| !p.exists()) {
        novas.push(p.to_path_buf());
        at = p.parent();
    }
    std::fs::create_dir_all(dir).map_err(i18n::io)?;
    novas.reverse();
    Ok(novas)
}

/// Remove only empty directories. Preserve any content created there after preparation began.
fn close_dirs(dirs: &[PathBuf]) {
    for dir in dirs.iter().rev() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// Use the clone directory name in repository errors.
fn repo_label(repo: &Path) -> String {
    repo.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Remove only worktrees created by this preparation. Retain branches for retry, including branches
/// that existed before the attempt.
fn undo_worktrees(feitos: &[(PathBuf, PathBuf)]) {
    for (repo, dest) in feitos.iter().rev() {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "remove", "--force"])
            .arg(dest)
            .output();
        let _ = git(repo, &["worktree", "prune"]);
    }
}

/// Without worktree isolation, create or select the branch in the original clone. Uncommitted
/// changes move with the checkout when Git permits it; otherwise report Git's error.
fn switch_branch(repo: &Path, branch: &str, base: &str) -> Result<(), String> {
    if head_branch(repo).as_deref() == Some(branch) {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("switch");
    if has_commit(repo, &format!("refs/heads/{branch}")) {
        cmd.arg(branch);
    } else {
        cmd.arg("-c").arg(branch);
        if !base.is_empty() {
            prepare_base(repo, base)?;
            cmd.arg(base);
        }
    }

    let out = cmd
        .output()
        .map_err(|e| i18n::ta("err.git.spawn", &[("cause", e.to_string())]))?;
    if !out.status.success() {
        return Err(i18n::ta(
            "err.git",
            &[
                ("command", "git switch".into()),
                (
                    "cause",
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ),
            ],
        ));
    }
    Ok(())
}

fn head_branch(repo: &Path) -> Option<String> {
    let name = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();
    (!name.is_empty() && name != "HEAD").then_some(name)
}

/// Refresh only the requested remote base. If the network fails, the existing local ref remains
/// usable.
fn prepare_base(repo: &Path, base: &str) -> Result<(), String> {
    if let Some((remote, rest)) = base.split_once('/') {
        if has_commit(repo, &format!("refs/remotes/{base}")) {
            let _ = fetch(repo, remote, rest);
        }
    }
    match has_commit(repo, base) {
        true => Ok(()),
        false => Err(i18n::ta(
            "err.session.noBase",
            &[
                ("base", base.to_string()),
                ("path", repo.display().to_string()),
            ],
        )),
    }
}

/// Bound git fetch duration so a stalled network cannot stall workspace preparation.
fn fetch(repo: &Path, remote: &str, branch: &str) -> Result<(), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["fetch", "--quiet", remote, branch])
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(i18n::io)?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Err(e) => return Err(i18n::io(e)),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                return Err(i18n::t("err.git.fetchSlow"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

/// Require a ref that resolves to a commit; verify alone accepts objects that worktree add cannot
/// use.
fn has_commit(repo: &Path, reference: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{reference}^{{commit}}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Use the registered folder's name on disk.
fn repo_named(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| i18n::t("err.session.badPath"))
}

/// Write the same repository map to CLAUDE.md and AGENTS.md for multi-repository workspaces. Never
/// overwrite files the person may have written.
fn describe_root(root: &Path, repos: &[Repo], branch: &str) {
    let list: String = repos
        .iter()
        .map(|r| {
            format!(
                "- `{}/` — {} `{}`\n",
                r.name,
                i18n::pick("clone em", "clone at"),
                r.path
            )
        })
        .collect();
    let text = i18n::pick(
        &format!(
            "# Workspace com {} repositórios\n\nEsta pasta reúne um worktree por repositório, todos na branch `{branch}`:\n\n{list}\nCada um é um repositório git independente: commits, `git status` e PRs são por pasta. Leia o `CLAUDE.md` ou `AGENTS.md` de cada um antes de mexer nele.\n",
            repos.len()
        ),
        &format!(
            "# Workspace with {} repositories\n\nThis folder holds one worktree per repository, all on branch `{branch}`:\n\n{list}\nEach one is an independent git repository: commits, `git status` and PRs are per folder. Read each one's `CLAUDE.md` or `AGENTS.md` before working on it.\n",
            repos.len()
        ),
    );
    for name in ["CLAUDE.md", "AGENTS.md"] {
        let file = root.join(name);
        if !file.exists() {
            let _ = std::fs::write(&file, &text);
        }
    }
}

/// Choose the clone's origin/HEAD, then conventional default names, then its current branch. The
/// launcher uses the same default.
fn default_base(repo: &Path) -> String {
    list_branches(repo.display().to_string()).default
}

/// List repository branches for the launcher, with recently updated branches first.
#[derive(serde::Serialize)]
pub struct Branches {
    pub all: Vec<String>,
    pub default: String,
    /// Distinguish a non-Git folder from a newly initialized repository with no refs, so the
    /// launcher can enable valid controls.
    pub git: bool,
}

#[tauri::command(async)]
pub fn list_branches(project: String) -> Branches {
    let repo = PathBuf::from(expand(&project));
    let git_repo = repo.join(".git").exists();
    let refs = |pattern: &str| -> Vec<String> {
        git(
            &repo,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:short)",
                pattern,
            ],
        )
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty() && !r.ends_with("/HEAD"))
        .map(str::to_string)
        .collect()
    };
    let locals = refs("refs/heads");
    let remotes = refs("refs/remotes");

    // Prefer the saved origin/HEAD, then conventional names, then the current branch.
    let head = git(
        &repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .trim()
    .to_string();
    let default = [head, "origin/main".to_string(), "origin/master".to_string()]
        .into_iter()
        .find(|r| !r.is_empty() && remotes.contains(r))
        .or_else(|| {
            let head = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])
                .trim()
                .to_string();
            (!head.is_empty() && head != "HEAD").then_some(head)
        })
        .or_else(|| locals.first().cloned())
        .unwrap_or_default();

    // Put the selected base first, followed by local and remote branches.
    let mut all: Vec<String> = Vec::new();
    for name in [default.clone()].into_iter().chain(locals).chain(remotes) {
        if !name.is_empty() && !all.contains(&name) {
            all.push(name);
        }
    }
    Branches {
        all,
        default,
        git: git_repo,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        artifacts_of, first_message, multi_pr_text, patch_map, pr_text, resolve_tools, Choice,
        Draft, Pr, ProviderId, Repo, RepoPr, ResolvedTools, Tab, Workspace,
    };
    use crate::dock::{is_terminal, multi_setup, quoted};
    use std::path::Path;

    #[test]
    fn local_project_changes_wait_for_catalog_installation_without_locking_the_board() {
        use std::sync::{mpsc, Mutex};
        use std::time::Duration;

        let mut initial = crate::state::Board::default();
        super::register_project(&mut initial, Path::new("/old"));
        let board = Mutex::new(initial);
        let sync = crate::catalog::guard();
        let (started, ready) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        std::thread::scope(|scope| {
            let add = scope.spawn(|| {
                started.send(()).unwrap();
                super::register_local_project(&board, Path::new("/new"));
                finished.send(()).unwrap();
            });
            let remove = scope.spawn(|| {
                started.send(()).unwrap();
                super::remove_registered_project(&board, "/old");
                finished.send(()).unwrap();
            });
            ready.recv().unwrap();
            ready.recv().unwrap();
            let blocked = matches!(
                done.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
            let during = board.try_lock().ok().map(|b| {
                b.projects
                    .iter()
                    .map(|p| p.path.clone())
                    .collect::<Vec<_>>()
            });
            drop(sync);
            add.join().unwrap();
            remove.join().unwrap();
            assert!(
                blocked,
                "local project mutations must wait for the catalog guard"
            );
            assert_eq!(during, Some(vec!["/old".to_string()]));
        });
        let board = board.into_inner().unwrap();
        assert_eq!(board.projects.len(), 1);
        assert_eq!(board.projects[0].path, "/new");
    }

    fn pr(number: u64, branch: &str, state: &str) -> Pr {
        Pr {
            number,
            title: format!("PR {number}"),
            is_draft: false,
            state: state.into(),
            head_ref_name: branch.into(),
        }
    }

    /// Verify cleanup against real Git repositories: require archiving, committed changes, and work
    /// already merged into the target unless force explicitly permits loss.
    #[test]
    fn check_allows_only_clean_workspaces_that_were_entered() {
        let root = std::env::temp_dir().join(format!("prometeu-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (origin, local) = (root.join("origin"), root.join("clone"));
        std::fs::create_dir_all(&origin).unwrap();

        let run = |dir: &std::path::Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                // Disable commit signing so tests do not depend on the runner's GPG configuration.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["config", "user.email", "t@t"]);
        run(&origin, &["config", "user.name", "t"]);
        std::fs::write(origin.join("a.txt"), "a").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "a"]);

        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&local)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let dest = root.join("wt");
        super::add_worktree(&local, "work", "origin/main", &dest).unwrap();

        let mut ws = super::Workspace {
            id: "w".into(),
            title: "work".into(),
            project: local.display().to_string(),
            repo: local.display().to_string(),
            repo_name: "clone".into(),
            branch: "work".into(),
            worktree: dest.display().to_string(),
            repos: vec![Repo {
                path: local.display().to_string(),
                name: "clone".into(),
                worktree: dest.display().to_string(),
                base: String::new(),
                pr: None,
            }],
            stage: "Feito".into(),
            agent: ProviderId::Claude,
            archived: true,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            share_team: None,
            audience: None,
            remote_control: false,
            preparing: false,
            mcp: None,
            plugins: None,
            skills: None,
            failed: None,
            model: String::new(),
            effort: String::new(),
            port: None,
            issue: None,
            tabs: Vec::new(),
            active: None,
        };

        // A branch with no new commits is already contained in its target.
        super::check(&ws).unwrap();

        // An unmerged commit prevents cleanup.
        std::fs::write(dest.join("b.txt"), "b").unwrap();
        run(&dest, &["config", "user.email", "t@t"]);
        run(&dest, &["config", "user.name", "t"]);
        run(&dest, &["add", "-A"]);
        run(&dest, &["commit", "-qm", "b"]);
        assert!(super::check(&ws).unwrap_err().contains("unmerged"));

        // GitHub's merged PR status also permits cleanup after a squash merge, which does not
        // preserve branch ancestry.
        ws.repos[0].pr = Some(pr(3, "work", "MERGED"));
        super::check(&ws).unwrap();

        // Uncommitted changes prevent cleanup.
        std::fs::write(dest.join("c.txt"), "c").unwrap();
        assert!(super::check(&ws).unwrap_err().contains("dirty"));
        // Force permits the explicitly approved loss of uncommitted changes.
        super::hard(&ws).unwrap();
        std::fs::remove_file(dest.join("c.txt")).unwrap();
        super::check(&ws).unwrap();

        // Force never bypasses the requirement to archive first.
        ws.archived = false;
        assert!(super::check(&ws).unwrap_err().contains("notArchived"));
        assert!(super::hard(&ws).unwrap_err().contains("notArchived"));
        ws.archived = true;

        // Force never permits deleting the original clone.
        ws.worktree = ws.repo.clone();
        assert!(super::hard(&ws).unwrap_err().contains("isRepo"));

        // An unmerged commit in any additional repository blocks cleanup of the entire workspace.
        ws.worktree = dest.display().to_string();
        let dest2 = root.join("wt2");
        super::add_worktree(&local, "work-2", "origin/main", &dest2).unwrap();
        ws.repos.push(Repo {
            path: local.display().to_string(),
            name: "clone-2".into(),
            worktree: dest2.display().to_string(),
            base: String::new(),
            pr: None,
        });
        super::check(&ws).unwrap();
        std::fs::write(dest2.join("d.txt"), "d").unwrap();
        assert!(super::check(&ws).unwrap_err().contains("dirty"));

        // Recursive removal requires the exact application-owned grouping path with its worktrees
        // as immediate children.
        let names: Vec<String> = ws.repos.iter().map(|repo| repo.name.clone()).collect();
        let multi = super::paths::multi_dir(&names, &ws.branch);
        ws.worktree = multi.display().to_string();
        for repo in &mut ws.repos {
            repo.worktree = multi.join(&repo.name).display().to_string();
        }
        super::validate_multi_root(&ws).unwrap();

        // Legacy imports retain worktree locations. Only the exact calculated legacy root is
        // accepted.
        let legacy = super::paths::prometheus_multi_dir(&names, &ws.branch);
        ws.worktree = legacy.display().to_string();
        for repo in &mut ws.repos {
            repo.worktree = legacy.join(&repo.name).display().to_string();
        }
        super::validate_multi_root(&ws).unwrap();
        ws.worktree = legacy.join("neighbor").display().to_string();
        assert!(super::validate_multi_root(&ws)
            .unwrap_err()
            .contains("badRoot"));

        ws.worktree = super::paths::home().display().to_string();
        assert!(super::validate_multi_root(&ws)
            .unwrap_err()
            .contains("badRoot"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn repo_pr(
        name: &str,
        branch: Option<&str>,
        dirty: usize,
        ahead: u32,
        target: &str,
        upstream: bool,
        open: Option<u64>,
    ) -> RepoPr {
        RepoPr {
            name: name.into(),
            branch: branch.map(str::to_string),
            dirty,
            ahead,
            target: target.into(),
            upstream,
            open,
        }
    }

    /// The PR prompt must name the branch, strip the remote from --base, and describe uncommitted
    /// changes.
    #[test]
    fn pr_text_describes_state_and_steps() {
        let t = pr_text(&repo_pr(
            "app",
            Some("my/update"),
            3,
            2,
            "origin/main",
            false,
            None,
        ));
        assert!(t.contains("Há 3 arquivos"));
        assert!(t.contains("git push -u origin HEAD:my/update"));
        assert!(t.contains("gh pr create --base main"));
        assert!(t.contains("Ainda não há branch upstream."));

        let clean = pr_text(&repo_pr("app", None, 0, 0, "origin/master", true, None));
        assert!(clean.contains("limpo"));
        assert!(clean.contains("HEAD solto"));
        assert!(clean.contains("--base master"));
        assert!(clean.contains("A branch já tem upstream."));
    }

    /// An existing PR requests an update to its number instead of another PR.
    #[test]
    fn pr_text_requests_updates_for_open_pull_requests() {
        let t = pr_text(&repo_pr(
            "app",
            Some("my/update"),
            1,
            1,
            "origin/main",
            true,
            Some(42),
        ));
        assert!(t.contains("Quero atualizar o PR #42"));
        assert!(t.contains("gh pr view 42"));
        assert!(t.contains("gh pr edit 42"));
        assert!(!t.contains("gh pr create"));
        // Both creation and updates require committing and pushing the work.
        assert!(t.contains("git push -u origin HEAD:my/update"));
    }

    /// For multiple repositories, update existing PRs, skip unchanged repositories, and create the
    /// remaining PRs with cross-links.
    #[test]
    fn multi_pr_text_lists_each_repository() {
        let t = multi_pr_text(&[
            repo_pr("backend", Some("feat/x"), 2, 4, "origin/main", true, None),
            repo_pr(
                "portal",
                Some("feat/x"),
                0,
                1,
                "origin/develop",
                true,
                Some(17),
            ),
            repo_pr("dash", Some("feat/x"), 0, 0, "origin/main", false, None),
        ]);
        assert!(t.contains("reúne 3 repositórios"));
        assert!(t.contains("`backend/` — branch `feat/x`, 4 commits além de `origin/main`, 2 arquivos fora de commit. sem PR ainda."));
        assert!(t.contains("`portal/` — branch `feat/x`, 1 commit além de `origin/develop`, nada fora de commit. PR #17 aberto"));
        assert!(t.contains("`dash/` — branch `feat/x`, nenhum commit além de `origin/main`, nada fora de commit, sem upstream. nada a entregar: fica sem PR."));
        assert!(t.contains("linkar os outros PRs"));
    }
    use std::process::Command;

    /// A minimal workspace for tests that do not need disk access.
    fn bare() -> Workspace {
        Workspace {
            id: "w".into(),
            title: "w".into(),
            project: String::new(),
            repo: String::new(),
            repo_name: String::new(),
            branch: String::new(),
            worktree: String::new(),
            repos: Vec::new(),
            stage: String::new(),
            archived: false,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            share_team: None,
            audience: None,
            remote_control: false,
            preparing: false,
            mcp: None,
            plugins: None,
            skills: None,
            failed: None,
            agent: ProviderId::Claude,
            model: String::new(),
            effort: String::new(),
            port: None,
            issue: None,
            tabs: Vec::new(),
            active: None,
        }
    }

    /// A minimal tab with an optional provider and model choice.
    fn tab(id: &str, choice: Option<Choice>) -> Tab {
        Tab {
            plan: false,
            permission: None,
            task: None,
            id: id.into(),
            agent_session: None,
            title: id.into(),
            status: crate::state::Status::Desligada,
            note: None,
            pending_prompt: None,
            tokens: None,
            context_tokens: None,
            choice,
            kickoff: None,
        }
    }

    /// Launcher drafts sent before ADR 0057, including the delegation payload, carry no kickoff and
    /// still deserialize as a conversation without one.
    #[test]
    fn drafts_without_a_kickoff_still_deserialize() {
        let old: Draft = serde_json::from_value(serde_json::json!({
            "project": "/r", "branch": "b", "base": "main", "worktree": true, "title": "t",
            "stage": "s", "prompt": "p", "inject": [], "agent": "codex",
        }))
        .unwrap();
        assert_eq!(old.kickoff, "");
        let new: Draft = serde_json::from_value(serde_json::json!({
            "project": "/r", "branch": "b", "base": "main", "worktree": true, "title": "t",
            "stage": "s", "prompt": "p", "inject": [], "kickoff": "sdd-kit/specify",
        }))
        .unwrap();
        assert_eq!(new.kickoff, "sdd-kit/specify");
    }

    /// The kickoff line opens the first message, ahead of attachments and the person's prompt.
    #[test]
    fn first_message_opens_with_the_kickoff_line() {
        assert_eq!(
            first_message(
                Some("Use the \"specify\" skill."),
                "  Build login  ",
                &["a.md".into()]
            ),
            Some("Use the \"specify\" skill.\n\n@a.md\n\nBuild login".into())
        );
        assert_eq!(
            first_message(Some("Use the \"specify\" skill."), "", &[]),
            Some("Use the \"specify\" skill.".into())
        );
        assert_eq!(
            first_message(None, "Build login", &[]),
            Some("Build login".into())
        );
        assert_eq!(first_message(None, " ", &[]), None);
    }

    /// The declared artifact path is relative to the agent's working directory, which is the
    /// primary repository's parent when several repositories share it; nothing is declared, nothing
    /// is named.
    #[test]
    fn artifact_path_follows_the_primary_repository() {
        let root =
            std::env::temp_dir().join(format!("prometeu-artifacts-{}", uuid::Uuid::new_v4()));
        let clone = root.join("app");
        std::fs::create_dir_all(clone.join(".prometeu")).unwrap();
        std::fs::write(
            clone.join(".prometeu/settings.toml"),
            "[method]\nartifacts = \"docs/specs\"\n",
        )
        .unwrap();
        let parent = root.join("wt");
        let primary = Repo {
            path: clone.display().to_string(),
            name: "app".into(),
            worktree: parent.join("app").display().to_string(),
            base: String::new(),
            pr: None,
        };
        let mut multi = bare();
        multi.worktree = parent.display().to_string();
        multi.repos = vec![primary.clone()];
        assert_eq!(artifacts_of(&multi).as_deref(), Some("app/docs/specs"));

        let mut single = bare();
        single.worktree = primary.worktree.clone();
        single.repos = vec![primary];
        assert_eq!(artifacts_of(&single).as_deref(), Some("docs/specs"));

        std::fs::remove_file(clone.join(".prometeu/settings.toml")).unwrap();
        assert_eq!(artifacts_of(&single), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transcript_routing_uses_tab_and_frozen_task_providers() {
        for (workspace_provider, tab_provider) in [
            (ProviderId::Claude, ProviderId::RetiredGemini),
            (ProviderId::RetiredGemini, ProviderId::Antigravity),
            (ProviderId::Claude, ProviderId::Codex),
            (ProviderId::Codex, ProviderId::Claude),
        ] {
            let mut ws = bare();
            ws.agent = workspace_provider;
            ws.worktree = "/tmp/prometeu-transcript-routing".into();
            let choice = Choice {
                agent: tab_provider,
                ..Default::default()
            };
            let mut task = tab("task", None);
            task.task = Some(
                serde_json::from_value(serde_json::json!({
                    "command": "review", "profile": {
                        "id": "review", "name": "Review", "prompt": "Review",
                        "choice": choice, "mcp": null, "plugins": null,
                        "skills": [], "permission": "ask", "watch": null
                    }, "paused": false, "done": false, "turns": 0,
                    "checked_at": 0, "error": null
                }))
                .unwrap(),
            );
            ws.tabs = vec![tab("custom", Some(choice)), tab("inherited", None), task];
            for (id, provider) in [
                ("custom", tab_provider),
                ("task", tab_provider),
                ("inherited", workspace_provider),
            ] {
                let expected = match provider {
                    ProviderId::Claude => crate::paths::transcript(id, Path::new(&ws.worktree)),
                    ProviderId::Codex | ProviderId::Antigravity | ProviderId::RetiredGemini => {
                        crate::paths::chat_log(id)
                    }
                };
                assert_eq!(crate::chat::transcript_of(&ws, id), expected, "tab {id}");
            }
        }
    }

    #[test]
    fn new_and_resumed_tabs_preserve_resolved_tools_with_model_overrides() {
        for selected in [None, Some(vec![]), Some(vec!["selected".to_string()])] {
            for provider in [ProviderId::Claude, ProviderId::Codex] {
                let mut ws = bare();
                ws.model = "workspace-model".into();
                ws.effort = "high".into();
                // The caller resolves the layers; a tab carries that result whatever its model.
                let tools = ResolvedTools {
                    mcp: selected.clone(),
                    plugins: selected.clone(),
                    skills: None,
                };
                let choice = Choice {
                    agent: provider,
                    model: "tab-model".into(),
                    effort: "medium".into(),
                };
                ws.tabs = vec![tab("custom", Some(choice.clone())), tab("inherited", None)];
                for launch in [
                    ws.launch_with(Some(choice), &tools),
                    ws.launch_of("custom", &tools),
                ] {
                    assert_eq!(launch.agent, provider);
                    assert_eq!(launch.model, "tab-model");
                    assert_eq!(launch.effort, "medium");
                    assert_eq!(launch.mcp, selected);
                    assert_eq!(launch.plugins, selected);
                    assert!(!launch.plan);
                }
                for launch in [
                    ws.launch_with(None, &tools),
                    ws.launch_of("inherited", &tools),
                ] {
                    assert_eq!(launch.model, "workspace-model");
                    assert_eq!(launch.effort, "high");
                    assert_eq!(launch.mcp, selected);
                    assert_eq!(launch.plugins, selected);
                }
            }
        }
    }

    /// A legacy board stored standalone skills inside `plugins`. After migration the skill moves to
    /// its own axis, yet a spawn still materializes both through the plugin pipeline, so an old
    /// workspace keeps the tools it had. See ADR 0045 and docs/contracts/persistence.md.
    #[test]
    fn legacy_tools_with_skills_migrate_and_still_materialize() {
        use crate::selection::{Selection, Tools};
        use crate::state::split_skills;

        let mut ws = bare();
        // The pre-migration selection: one plugin and one standalone skill mixed on the plugin axis.
        ws.plugins = Some(Selection::only(vec![
            "reviewer".into(),
            "skill-review".into(),
        ]));
        split_skills(&mut ws.plugins, &mut ws.skills);
        assert_eq!(ws.plugins, Some(Selection::only(vec!["reviewer".into()])));
        assert_eq!(
            ws.skills,
            Some(Selection::only(vec!["skill-review".into()]))
        );

        // Both ride the plugin hub, so resolution keeps them and a launch materializes the union in
        // plugin-then-skill order, exactly as the old single-axis selection did.
        let hub = vec!["reviewer".to_string(), "skill-review".to_string()];
        let tools = resolve_tools(
            &Tools::default(),
            &Tools::default(),
            &ws.tools(),
            &[],
            &[],
            &hub,
        );
        let launch = ws.launch(&tools);
        assert_eq!(launch.plugins, Some(vec!["reviewer".into()]));
        assert_eq!(launch.skills, Some(vec!["skill-review".into()]));
        assert_eq!(
            launch.plugin_packages(),
            Some(vec!["reviewer".into(), "skill-review".into()])
        );
    }

    /// A project `[tools]` declaration is gated on trust: it composes only when the person approved
    /// the current hash, a changed declaration re-gates until approved again, and a rejection never
    /// activates it (ADR 0045, phase 4).
    #[test]
    fn project_tools_require_approval_and_invalidate_it_when_hash_changes() {
        use super::{project_declaration, tools_hash, trusted_project};
        use crate::selection::{Selection, Tools};
        use crate::state::ToolTrust;

        let root = std::env::temp_dir().join(format!("prometeu-trust-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".prometeu")).unwrap();
        std::fs::write(
            root.join(".prometeu/settings.toml"),
            "[tools]\nplugins = { base = \"none\", add = [\"reviewer\"] }\n",
        )
        .unwrap();

        let mut ws = bare();
        ws.repo = root.display().to_string();
        ws.worktree = root.display().to_string();
        ws.repos.push(Repo {
            path: root.display().to_string(),
            name: "trust".into(),
            worktree: root.display().to_string(),
            base: String::new(),
            pr: None,
        });

        let hub = vec!["reviewer".to_string()];
        let declaration = project_declaration(&root, &root).expect("declaration");
        assert_eq!(
            declaration.tools.plugins,
            Some(Selection::only(vec!["reviewer".into()]))
        );

        // Unapproved: the project layer contributes nothing, so no plugin is injected.
        assert_eq!(trusted_project(&[], &ws), Tools::default());
        let gated = resolve_tools(
            &Tools::default(),
            &Tools::default(),
            &ws.tools(),
            &[],
            &[],
            &hub,
        );
        assert_eq!(gated.plugins, None);

        // Approved for the current hash: the declaration composes and the plugin is injected.
        let trust = vec![ToolTrust {
            repo: declaration.repo.clone(),
            hash: declaration.hash.clone(),
            approved: true,
            at: 0,
        }];
        let allowed = trusted_project(&trust, &ws);
        assert_eq!(
            allowed.plugins,
            Some(Selection::only(vec!["reviewer".into()]))
        );
        let resolved = resolve_tools(&Tools::default(), &allowed, &ws.tools(), &[], &[], &hub);
        assert_eq!(resolved.plugins, Some(vec!["reviewer".into()]));

        // A changed declaration re-gates: the stored hash no longer matches, so approval lapses.
        std::fs::write(
            root.join(".prometeu/settings.toml"),
            "[tools]\nplugins = { base = \"none\", add = [\"reviewer\", \"other\"] }\n",
        )
        .unwrap();
        let changed = project_declaration(&root, &root).expect("declaration");
        assert_ne!(changed.hash, declaration.hash);
        let mut decisions = trust.clone();
        for verdict in [true, false] {
            let current = project_declaration(&root, &root).unwrap();
            let error =
                super::record_tool_trust(&mut decisions, current, &declaration.hash, verdict)
                    .unwrap_err();
            assert!(error.contains("err.tools.changed"));
            assert_eq!(decisions, trust);
        }
        let current_hash = changed.hash.clone();
        super::record_tool_trust(&mut decisions, changed, &current_hash, true).unwrap();
        assert_eq!(
            trusted_project(&decisions, &ws)
                .plugins
                .as_ref()
                .unwrap()
                .add,
            ["reviewer", "other"]
        );

        assert_eq!(trusted_project(&trust, &ws), Tools::default());

        // A recorded rejection does not activate the layer either.
        let rejected = vec![ToolTrust {
            repo: declaration.repo.clone(),
            hash: declaration.hash.clone(),
            approved: false,
            at: 0,
        }];
        assert_eq!(trusted_project(&rejected, &ws), Tools::default());

        // The hash is stable for the same declaration.
        assert_eq!(tools_hash(&declaration.tools), declaration.hash);

        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Exercise the native IPC body, where Option<Value> used to lose explicit nulls.
    #[test]
    fn tool_axis_ipc_preserves_absent_null_and_replacement() {
        use super::{axis_patch, Axis};
        use crate::selection::Selection;
        use tauri::ipc::InvokeBody;
        for (name, kind) in [
            ("mcp", Axis::Mcp),
            ("plugins", Axis::Plugins),
            ("skills", Axis::Skills),
        ] {
            let absent = InvokeBody::Json(serde_json::json!({"id":"workspace"}));
            assert_eq!(axis_patch(&absent, name, kind).unwrap(), None);
            let reset = InvokeBody::Json(serde_json::json!({"id":"workspace",name:null}));
            assert_eq!(axis_patch(&reset, name, kind).unwrap(), Some(None));
            let empty =
                InvokeBody::Json(serde_json::json!({name:{"base":"none","add":[],"remove":[]}}));
            assert_eq!(
                axis_patch(&empty, name, kind).unwrap(),
                Some(Some(Selection::only(vec![])))
            );
            for value in [
                serde_json::json!([]),
                serde_json::json!(false),
                serde_json::json!({"add":1}),
            ] {
                let bad = InvokeBody::Json(serde_json::json!({name:value}));
                assert!(axis_patch(&bad, name, kind)
                    .unwrap_err()
                    .contains("err.tools.badPayload"));
            }
        }
        assert!(axis_patch(&InvokeBody::Raw(vec![]), "mcp", Axis::Mcp).is_err());
    }

    /// Real files and both adapters' universes, isolated from the user's configuration in a child.
    #[test]
    fn tool_resolution_uses_the_tab_provider_and_configured_claude_home() {
        use super::resolve_workspace_tools;
        use crate::selection::{Base, Selection, Tools};
        let marker = "PROMETEU_TOOL_PROVIDER_TEST_CHILD";
        if std::env::var_os(marker).is_none() {
            let root =
                std::env::temp_dir().join(format!("prometeu-tools-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "session::tests::tool_resolution_uses_the_tab_provider_and_configured_claude_home", "--nocapture"])
                .env(marker, "1")
                .env("PROMETEU_ROOT", &root)
                .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
                .output().unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let home = crate::claude::user_home();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{"mcpServers":{"cli-only":{"command":"cli-test"}}}"#,
        )
        .unwrap();
        crate::mcp::store(&[crate::mcp::Server {
            id: "hub".into(),
            config: serde_json::json!({"command":"hub-test"}),
            note: String::new(),
        }])
        .unwrap();
        let mut ws = bare();
        ws.worktree = crate::paths::root().join("repo").display().to_string();
        std::fs::create_dir_all(&ws.worktree).unwrap();
        let global = Tools {
            mcp: Some(Selection {
                base: Base::Inherit,
                add: vec!["hub".into()],
                remove: vec![],
            }),
            ..Tools::default()
        };
        for workspace_agent in [ProviderId::Claude, ProviderId::Codex] {
            ws.agent = workspace_agent;
            for agent in [ProviderId::Claude, ProviderId::Codex] {
                let tools = resolve_workspace_tools(&global, &[], &ws, agent);
                let expected = if agent == ProviderId::Claude {
                    vec!["cli-only".to_string(), "hub".into()]
                } else {
                    vec!["hub".to_string()]
                };
                assert_eq!(tools.mcp.as_ref().unwrap(), &expected);
                let choice = Choice {
                    agent,
                    ..Choice::default()
                };
                let launch = ws.launch_with(Some(choice.clone()), &tools);
                assert_eq!(launch.agent, agent);
                assert_eq!(launch.mcp.as_ref().unwrap(), &expected);
                ws.tabs = vec![tab("mixed", Some(choice))];
                let resumed = ws.launch_of("mixed", &tools);
                assert_eq!(resumed.agent, agent);
                assert_eq!(resumed.mcp.as_ref().unwrap(), &expected);
                // Materialize the exact resolved set at the provider boundary.
                match agent {
                    ProviderId::Claude => {
                        crate::mcp::config_for(
                            "test",
                            launch.mcp.as_ref(),
                            Path::new(&ws.worktree),
                        )
                        .unwrap();
                    }
                    ProviderId::Codex | ProviderId::Antigravity | ProviderId::RetiredGemini => {
                        crate::mcp::codex_config("test", launch.mcp.as_ref()).unwrap();
                    }
                }
            }
        }
    }

    /// The setters' payload gate: `null` means inherit, a well-formed `Selection` passes, a
    /// malformed payload errors instead of silently resetting the axis, and ids belonging to the
    /// other plugin-pipeline axis are refused (ADR 0045).
    #[test]
    fn tool_axis_payload_is_validated_before_persistence() {
        use super::{axis, Axis};
        use crate::selection::{Base, Selection};

        assert_eq!(axis(serde_json::Value::Null, Axis::Mcp).unwrap(), None);

        let ok = serde_json::json!({ "base": "inherit", "add": ["notion"], "remove": [] });
        assert_eq!(
            axis(ok, Axis::Mcp).unwrap(),
            Some(Selection {
                base: Base::Inherit,
                add: vec!["notion".into()],
                remove: vec![]
            })
        );

        let bad = axis(serde_json::json!({ "base": "maybe" }), Axis::Mcp);
        assert!(bad.unwrap_err().contains("err.tools.badPayload"));

        // Standalone skills ride `skill-<id>` and belong to the skills axis only.
        let misplaced = axis(
            serde_json::json!({ "add": ["skill-reviewer"] }),
            Axis::Plugins,
        );
        assert!(misplaced.unwrap_err().contains("err.tools.badAxis"));
        let misplaced = axis(serde_json::json!({ "remove": ["reviewer"] }), Axis::Skills);
        assert!(misplaced.unwrap_err().contains("err.tools.badAxis"));

        assert!(axis(
            serde_json::json!({ "add": ["skill-reviewer"] }),
            Axis::Skills
        )
        .unwrap()
        .is_some());
        assert!(
            axis(serde_json::json!({ "add": ["reviewer"] }), Axis::Plugins)
                .unwrap()
                .is_some()
        );
    }

    /// The picker's provenance: a global item is inherited, a workspace add is added, removing an
    /// inherited item is removed, and an untrusted project item is pending until trusted (ADR 0045).
    #[test]
    fn provenance_classifies_each_hub_item() {
        use super::{axis_provenance, Gate, Provenance};
        use crate::selection::{Base, Selection};

        let hub = vec!["g".into(), "a".into(), "r".into(), "p".into(), "off".into()];
        let global = Some(Selection::only(vec!["g".into(), "r".into()]));
        // The project declares `p`; it stays pending until trusted.
        let project = Some(Selection {
            base: Base::Inherit,
            add: vec!["p".into()],
            remove: vec![],
        });
        // The workspace adds `a` and removes the inherited `r`.
        let workspace = Some(Selection {
            base: Base::Inherit,
            add: vec!["a".into()],
            remove: vec!["r".into()],
        });

        let items = axis_provenance(&global, &project, Gate::Pending, &workspace, &[], &hub);
        let prov = |id: &str| items.iter().find(|i| i.id == id).map(|i| i.provenance);
        assert_eq!(prov("g"), Some(Provenance::Inherited));
        assert_eq!(prov("a"), Some(Provenance::Added));
        assert_eq!(prov("r"), Some(Provenance::Removed));
        assert_eq!(prov("p"), Some(Provenance::Pending));
        // An off, untouched hub item carries no story and is omitted.
        assert_eq!(prov("off"), None);

        // Once trusted, the project item activates and inherits into the effective set.
        let trusted_items =
            axis_provenance(&global, &project, Gate::Trusted, &workspace, &[], &hub);
        assert_eq!(
            trusted_items
                .iter()
                .find(|i| i.id == "p")
                .map(|i| i.provenance),
            Some(Provenance::Inherited)
        );

        // An explicitly rejected declaration keeps its items visible, labeled rejected, so the
        // picker shows the state without prompting again.
        let rejected_items =
            axis_provenance(&global, &project, Gate::Rejected, &workspace, &[], &hub);
        assert_eq!(
            rejected_items
                .iter()
                .find(|i| i.id == "p")
                .map(|i| i.provenance),
            Some(Provenance::Rejected)
        );
    }

    /// The CLI-inherited base is visible without any layer action: an active base id is labeled
    /// `Cli`, the workspace may remove it, and an explicit add wins over the base label (ADR 0046).
    #[test]
    fn provenance_classifies_inherited_cli_configuration() {
        use super::{axis_provenance, Gate, Provenance};
        use crate::selection::{Base, Selection};

        let universe = vec!["hub-a".into(), "cli-on".into(), "cli-off".into()];
        let base = vec!["cli-on".into(), "cli-off".into()];
        let delta = |add: &[&str], remove: &[&str]| Selection {
            base: Base::Inherit,
            add: add.iter().map(|id| id.to_string()).collect(),
            remove: remove.iter().map(|id| id.to_string()).collect(),
        };
        let prov = |items: &[super::EffectiveItem], id: &str| {
            items.iter().find(|i| i.id == id).map(|i| i.provenance)
        };

        // With no declared layer, every base id is active and labeled as inherited from the CLI;
        // an untouched hub id stays omitted.
        let items = axis_provenance(&None, &None, Gate::Trusted, &None, &base, &universe);
        assert_eq!(prov(&items, "cli-on"), Some(Provenance::Cli));
        assert_eq!(prov(&items, "cli-off"), Some(Provenance::Cli));
        assert_eq!(prov(&items, "hub-a"), None);

        // A workspace removal drops one inherited server without relisting the rest.
        let items = axis_provenance(
            &None,
            &None,
            Gate::Trusted,
            &Some(delta(&[], &["cli-off"])),
            &base,
            &universe,
        );
        assert_eq!(prov(&items, "cli-on"), Some(Provenance::Cli));
        assert_eq!(prov(&items, "cli-off"), Some(Provenance::Removed));

        // An explicit workspace add over a base id reads as the person's own action.
        let items = axis_provenance(
            &None,
            &None,
            Gate::Trusted,
            &Some(delta(&["cli-on", "hub-a"], &[])),
            &base,
            &universe,
        );
        assert_eq!(prov(&items, "cli-on"), Some(Provenance::Added));
        assert_eq!(prov(&items, "hub-a"), Some(Provenance::Added));

        // A global replacement clears the base, and a base id no layer mentions is omitted.
        let items = axis_provenance(
            &Some(Selection::only(vec!["hub-a".into()])),
            &None,
            Gate::Trusted,
            &None,
            &base,
            &universe,
        );
        assert_eq!(prov(&items, "hub-a"), Some(Provenance::Inherited));
        assert_eq!(prov(&items, "cli-on"), None);
    }

    #[test]
    fn task_launch_uses_project_override_and_freezes_tools_and_model() {
        use crate::actions::{Catalog, Permission, Profile};
        use crate::selection::Selection;
        let mut ws = bare();
        ws.agent = ProviderId::Codex;
        ws.project = "project".into();
        ws.mcp = Some(Selection::only(vec!["original".into()]));
        let base = Profile {
            id: "review".into(),
            name: "Reviewer".into(),
            prompt: "Review".into(),
            choice: Choice {
                model: "sonnet".into(),
                ..Default::default()
            },
            mcp: None,
            plugins: Some(vec![]),
            skills: vec!["review".into()],
            permission: Permission::Ask,
            watch: None,
        };
        let mut catalog = Catalog {
            profiles: vec![base.clone()],
            ..Default::default()
        };
        let mut customized = base;
        customized.choice.model = "opus".into();
        catalog
            .overrides
            .entry("project".into())
            .or_default()
            .insert("review".into(), customized);
        // The profile leaves MCP unset, so it freezes the resolved layer the caller supplies.
        let resolved = ResolvedTools {
            mcp: Some(vec!["original".into()]),
            plugins: None,
            skills: None,
        };
        let profile = crate::actions::resolve(&catalog, &ws.project, "review", |agent| {
            assert_eq!(agent, ProviderId::Claude);
            resolved
        })
        .unwrap();
        let mut task = tab("task", None);
        task.task = Some(
            serde_json::from_value(serde_json::json!({
                "command":"review", "profile":profile, "paused":false, "done":false,
                "turns":0, "checked_at":0, "error":null
            }))
            .unwrap(),
        );
        ws.tabs.push(task);
        // Changing the workspace afterwards must not reach the frozen task.
        ws.mcp = Some(Selection::only(vec!["changed".into()]));
        ws.model = "haiku".into();
        let launch = ws.launch_of("task", &ResolvedTools::default());
        assert_eq!(launch.model, "opus");
        assert_eq!(launch.mcp, Some(vec!["original".into()]));
        assert_eq!(launch.plugins, Some(vec![]));
        assert_eq!(launch.config_scope.as_deref(), Some("task"));
        assert!(launch.instructions.contains("review"));
        assert!(ws.retune("task", Choice::default()).is_err());
    }

    /// Resume tabs with their own model choices. Tabs without overrides follow workspace defaults,
    /// including older persisted tabs.
    #[test]
    fn resuming_a_tab_preserves_its_original_model() {
        let mut ws = bare();
        ws.agent = ProviderId::Claude;
        ws.model = "opus[1m]".into();
        ws.effort = "high".into();
        ws.tabs = vec![
            tab("inherited", None),
            tab(
                "explicit",
                Some(Choice {
                    agent: ProviderId::Codex,
                    model: "gpt-5.6-sol".into(),
                    effort: "ultracode".into(),
                }),
            ),
        ];

        let inherited = ws.launch_of("inherited", &ResolvedTools::default());
        assert_eq!(
            (inherited.model.as_str(), inherited.effort.as_str()),
            ("opus[1m]", "high")
        );
        assert_eq!(inherited.agent, ProviderId::Claude);

        let explicit_choice = ws.launch_of("explicit", &ResolvedTools::default());
        assert_eq!(explicit_choice.agent, ProviderId::Codex);
        assert_eq!(
            (
                explicit_choice.model.as_str(),
                explicit_choice.effort.as_str()
            ),
            ("gpt-5.6-sol", "ultracode")
        );
        // Resuming never restores initial plan mode.
        assert!(!explicit_choice.plan);

        // A tab removed while the request was in flight falls back to workspace defaults.
        assert_eq!(
            ws.launch_of("missing", &ResolvedTools::default()).model,
            "opus[1m]"
        );
    }

    /// Retuning persists a tab override; choosing workspace defaults clears it. Reject switching
    /// providers within a transcript.
    #[test]
    fn changing_a_conversation_model_persists_it_on_the_tab() {
        let mut ws = bare();
        ws.model = "opus[1m]".into();
        ws.effort = "high".into();
        ws.tabs = vec![tab("open", None)];

        let choice = |model: &str, effort: &str| Choice {
            agent: ProviderId::Claude,
            model: model.into(),
            effort: effort.into(),
        };

        ws.retune("open", choice("sonnet", "medium")).unwrap();
        let launch = ws.launch_of("open", &ResolvedTools::default());
        assert_eq!(
            (launch.model.as_str(), launch.effort.as_str()),
            ("sonnet", "medium")
        );
        // Sibling tabs following workspace defaults remain unchanged.
        assert_eq!(ws.model, "opus[1m]");

        // Return to inherited settings instead of freezing a copy of today's defaults.
        ws.retune("open", choice("opus[1m]", "high")).unwrap();
        assert!(ws.tabs[0].choice.is_none());

        // The provider cannot change during a conversation.
        let gpt = Choice {
            agent: ProviderId::Codex,
            model: "gpt-5.6-sol".into(),
            effort: "high".into(),
        };
        assert!(ws.retune("open", gpt).is_err());
        assert!(ws.retune("missing", choice("sonnet", "high")).is_err());
    }

    /// Parse modified and deleted files, including paths containing spaces, from git diff HEAD.
    const DIFF: &str = "\
diff --git a/src/main.ts b/src/main.ts
index 1c1c1c1..2d2d2d2 100644
--- a/src/main.ts
+++ b/src/main.ts
@@ -12,3 +12,4 @@ const $ = (id: string) => document.getElementById(id)!;
 let state: Board;
-let openWs = null;
+let openWs: string | null = null;
+let sidePane = \"files\";
diff --git a/src/old.ts b/src/old.ts
deleted file mode 100644
index 3e3e3e3..0000000
--- a/src/old.ts
+++ /dev/null
@@ -1,2 +0,0 @@
-const gone = true;
-export default gone;
diff --git a/docs/with spaces.md b/docs/with spaces.md
--- a/docs/with spaces.md
+++ b/docs/with spaces.md
@@ -1 +1 @@
-before
+after
";

    /// Every text file with counted changes must have a renderable patch. Binary files have zero
    /// line counts and no text patch.
    #[test]
    fn file_listing_and_patch_refer_to_the_same_file() {
        let wt = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        for change in super::changes_in(wt) {
            if change.added + change.removed == 0 {
                continue;
            }
            assert!(
                change.patch.contains("@@"),
                "{} has {}+/{}- without a hunk",
                change.path,
                change.added,
                change.removed
            );
        }
    }

    /// Cleanup counts branch commits against the base, independently of local changes.
    #[test]
    fn cleanup_counts_commits_against_the_base() {
        let root = std::env::temp_dir().join(format!("prometeu-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                // Disable commit signing so tests do not depend on the runner's GPG configuration.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("gone.txt"), "x\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);
        run(&["checkout", "-qb", "feat"]);
        std::fs::write(root.join("a.txt"), "a\nb\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        std::fs::write(root.join("z.txt"), "z\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "feat"]);
        // Include both a modified tracked file and an untracked file.
        std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
        std::fs::write(root.join("new.txt"), "n\n").unwrap();

        let (base, ahead) = super::ahead_of(&root, "main");
        assert_eq!(base, super::git(&root, &["rev-parse", "main"]).trim());
        assert_eq!(ahead, 1);
        assert_eq!(super::ahead_of(&root, "nonexistent"), ("HEAD".into(), 0));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Use a local bare remote to prove new branches start from the selected base and respect the
    /// clone's origin/HEAD default.
    #[test]
    fn new_branches_start_from_the_selected_base() {
        let root = std::env::temp_dir().join(format!("prometeu-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (origin, local) = (root.join("origin"), root.join("clone"));
        std::fs::create_dir_all(&origin).unwrap();

        let run = |dir: &std::path::Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                // Disable commit signing so tests do not depend on the runner's GPG configuration.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["config", "user.email", "t@t"]);
        run(&origin, &["config", "user.name", "t"]);
        std::fs::write(origin.join("a.txt"), "a").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "a"]);
        run(&origin, &["checkout", "-qb", "old"]);
        std::fs::write(origin.join("b.txt"), "b").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "b"]);
        let old = run(&origin, &["rev-parse", "HEAD"]);
        run(&origin, &["checkout", "-q", "main"]);

        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&local)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let branches = super::list_branches(local.display().to_string());
        assert_eq!(branches.default, "origin/main");
        assert_eq!(branches.all.first().unwrap(), "origin/main");
        assert!(
            branches.all.contains(&"origin/old".to_string()),
            "{:?}",
            branches.all
        );

        let dest = root.join("wt");
        super::add_worktree(&local, "new", "origin/old", &dest).unwrap();
        assert_eq!(run(&dest, &["rev-parse", "HEAD"]), old);
        assert_eq!(run(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]), "new");

        // Reject a base that does not exist.
        let error = super::add_worktree(&local, "other", "origin/missing", &root.join("wt2"));
        assert!(error.unwrap_err().contains("missing"));

        // Reuse a worktree already on the requested branch so repeated creation is safe.
        super::add_worktree(&local, "new", "origin/old", &dest).unwrap();
        // Reject an existing worktree on the wrong branch.
        let error = super::add_worktree(&local, "other-branch", "origin/main", &dest).unwrap_err();
        assert!(error.contains("new"), "{error}");

        // Without worktree isolation, move the original clone's HEAD to a branch at the selected
        // base without creating a directory.
        super::switch_branch(&local, "here", "origin/old").unwrap();
        assert_eq!(run(&local, &["rev-parse", "--abbrev-ref", "HEAD"]), "here");
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), old);
        // Selecting the current branch is a no-op.
        super::switch_branch(&local, "here", "origin/main").unwrap();
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), old);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// When the same branch is requested in another directory, report its existing checkout and
    /// leave no empty directories behind. Multi-repository workspace names can produce different
    /// paths for the same issue branch.
    #[test]
    fn branches_open_in_another_directory_are_rejected_without_leaving_a_directory() {
        let root = std::env::temp_dir().join(format!("prometeu-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("code-rules");
        std::fs::create_dir_all(&repo).unwrap();

        let run = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.email", "t@t"]);
        run(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-qm", "a"]);

        // Create the single-repository workspace first.
        let first = root.join("code-rules").join("aut-49");
        assert!(super::add_worktree(&repo, "aut-49", "", &first).unwrap());

        // Request the same branch at the multi-repository path.
        let second = root.join("code-rules+autonomous").join("aut-49");
        let error =
            super::add_worktree(&repo, "aut-49", "", &second.join("code-rules")).unwrap_err();
        let location = first.canonicalize().unwrap().display().to_string();
        assert!(
            error.contains("aut-49") && error.contains(&location),
            "{error}"
        );
        assert!(
            !second.exists(),
            "attempt directory remains: {}",
            second.display()
        );
        assert!(!root.join("code-rules+autonomous").exists());

        // Adopt an existing worktree on the requested branch, and preserve it during rollback of a
        // sibling failure.
        assert!(!super::add_worktree(&repo, "aut-49", "", &first).unwrap());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Quote each setup subshell's path so spaces and apostrophes cannot split arguments.
    #[test]
    fn quoted_handles_spaces_and_apostrophes() {
        assert_eq!(quoted("/a b"), "'/a b'");
        assert_eq!(quoted("/d'x"), "'/d'\\''x'");
    }

    /// Run the combined setup script in a real shell to verify sequential execution in each
    /// repository with its own environment.
    #[test]
    fn multi_repository_setup_runs_in_each_repository_directory() {
        let root = std::env::temp_dir().join(format!("prometeu-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |name: &str, setup: &str| {
            let repo = root.join("clones").join(name);
            let wt = root.join("ws").join(name);
            std::fs::create_dir_all(repo.join(".prometeu")).unwrap();
            std::fs::create_dir_all(&wt).unwrap();
            // Use a TOML literal string because the command contains double quotes.
            std::fs::write(
                repo.join(".prometeu/settings.toml"),
                format!("[scripts]\nsetup = '{setup}'\n"),
            )
            .unwrap();
            Repo {
                path: repo.display().to_string(),
                name: name.into(),
                worktree: wt.display().to_string(),
                base: String::new(),
                pr: None,
            }
        };
        let repos = vec![
            mk("back end", "echo \"$PROMETEU_WORKSPACE_PATH\" > output.txt"),
            mk("front", "echo \"$PROMETEU_ROOT_PATH:$PORT\" > output.txt"),
        ];
        let ws = super::Workspace {
            id: "w".into(),
            title: "t".into(),
            project: repos[0].path.clone(),
            repo: repos[0].path.clone(),
            repo_name: repos[0].name.clone(),
            branch: "b".into(),
            worktree: root.join("ws").display().to_string(),
            repos: repos.clone(),
            stage: "Fazendo".into(),
            agent: ProviderId::Claude,
            archived: false,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            share_team: None,
            audience: None,
            remote_control: false,
            preparing: false,
            mcp: None,
            plugins: None,
            skills: None,
            failed: None,
            model: String::new(),
            effort: String::new(),
            port: Some(3100),
            issue: None,
            tabs: Vec::new(),
            active: None,
        };

        let (header, command) = multi_setup(&ws).unwrap();
        // Without copy declarations, omit the copy header.
        assert!(header.is_none());
        let out = Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let read =
            |r: &Repo| std::fs::read_to_string(Path::new(&r.worktree).join("output.txt")).unwrap();
        assert_eq!(read(&repos[0]).trim(), repos[0].worktree);
        assert_eq!(read(&repos[1]).trim(), format!("{}:3100", repos[1].path));

        // One setup declaration still creates a tab; no declarations create none.
        std::fs::remove_file(Path::new(&repos[1].path).join(".prometeu/settings.toml")).unwrap();
        assert!(multi_setup(&ws).unwrap().1.contains("back end"));
        std::fs::remove_file(Path::new(&repos[0].path).join(".prometeu/settings.toml")).unwrap();
        assert!(multi_setup(&ws).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn splits_one_patch_per_file() {
        let map = patch_map(DIFF);
        assert_eq!(map.len(), 3);

        let main = &map["src/main.ts"].body;
        assert!(main.starts_with("@@ -12,3 +12,4 @@ const $"), "{main}");
        assert!(main.contains("+let openWs: string | null = null;"));
        // Exclude index and path headers from the rendered patch.
        assert!(!main.contains("index 1c1c1c1"));
        assert!(!main.contains("--- a/src/main.ts"));
        assert!(!map["src/main.ts"].new && !map["src/main.ts"].deleted);

        // Deleted files use /dev/null as the destination, so recover their path from the source
        // header and mark deleted file mode.
        assert!(map["src/old.ts"].body.contains("-export default gone;"));
        assert!(map["src/old.ts"].deleted);
        assert_eq!(map["docs/with spaces.md"].body.lines().count(), 3);
    }

    /// Only terminal and terminal-<number> may open a PTY. Reject arbitrary keys before they create
    /// shell processes.
    #[test]
    fn only_numbered_terminal_labels_become_shells() {
        assert!(is_terminal("terminal"));
        assert!(is_terminal("terminal-2"));
        assert!(is_terminal("terminal-10"));
        assert!(!is_terminal("terminal-"));
        assert!(!is_terminal("terminal-2x"));
        assert!(!is_terminal("terminalextra"));
        assert!(!is_terminal("setup"));
        assert!(!is_terminal("run"));
    }
}

/// Run read-only Git predicates and use their exit status as the result.
fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn expand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => paths::home().join(rest).display().to_string(),
        None => p.to_string(),
    }
}

/// Build the Open PR prompt from Git state at the backend. The agent commits, pushes, and opens the
/// PR; a repository PR skill takes precedence over these instructions.
#[tauri::command(async)]
pub fn pr_prompt(state: State<AppState>, id: String) -> Result<String, String> {
    let repos = repos_of(&state, &id);
    if repos.is_empty() {
        return Err(i18n::t("err.session.noWorkspace"));
    }
    let states: Vec<RepoPr> = repos.iter().map(pr_state).collect();
    Ok(match states.as_slice() {
        [one] => pr_text(one),
        many => multi_pr_text(many),
    })
}

/// Repository state needed for a PR: branch, uncommitted changes, commits beyond the base, and an
/// existing PR.
struct RepoPr {
    name: String,
    branch: Option<String>,
    dirty: usize,
    ahead: u32,
    target: String,
    upstream: bool,
    open: Option<u64>,
}

fn pr_state(r: &Repo) -> RepoPr {
    let wt = Path::new(&r.worktree);
    let branch = head_branch(wt);
    let dirty = changes_in(wt).len();
    // Use the remote default branch, falling back to conventional names when origin/HEAD is
    // unavailable.
    let head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let target = if head.is_empty() {
        "origin/main".to_string()
    } else {
        head
    };
    let upstream = !git(wt, &["rev-parse", "--abbrev-ref", "@{upstream}"])
        .trim()
        .is_empty();
    // For an existing PR, request a push and a description review instead of creating a duplicate.
    let open = branch
        .as_deref()
        .and_then(|b| crate::github::pr_for_branch(wt, b))
        .filter(|pr| pr.open())
        .map(|pr| pr.number);
    let (_, ahead) = ahead_of(wt, &r.base);
    RepoPr {
        name: r.name.clone(),
        branch,
        dirty,
        ahead,
        target,
        upstream,
        open,
    }
}

fn pr_text(s: &RepoPr) -> String {
    let estado = match s.dirty {
        0 => "O worktree está limpo — nada fora de commit.".to_string(),
        1 => "Há 1 arquivo com mudanças fora de commit.".to_string(),
        n => format!("Há {n} arquivos com mudanças fora de commit."),
    };
    let onde = match &s.branch {
        Some(b) => format!("A branch atual é `{b}`"),
        None => "O worktree está em HEAD solto — crie uma branch antes de commitar".to_string(),
    };
    let target = &s.target;
    let up = if s.upstream {
        "A branch já tem upstream."
    } else {
        "Ainda não há branch upstream."
    };
    let base = target.split_once('/').map_or(target.as_str(), |(_, b)| b);
    let push = match &s.branch {
        Some(b) => format!("git push -u origin HEAD:{b}"),
        None => "git push -u origin HEAD:<nome-da-branch>".to_string(),
    };
    let (abertura, fim) = match s.open {
        Some(n) => (
            format!("Quero atualizar o PR #{n} desta branch."),
            format!(
                "5. Revise o diff inteiro da branch contra `{target}` e confira com `gh pr view {n}` se o título e a \
                 descrição ainda cobrem tudo.\n6. Se não cobrirem mais, atualize com `gh pr edit {n}`. Título com \
                 menos de 80 caracteres; descrição com até cinco frases, cobrindo todas as mudanças da branch — não \
                 só as desta conversa."
            ),
        ),
        None => (
            "Quero abrir um PR deste worktree.".to_string(),
            format!(
                "5. Revise o diff inteiro da branch contra `{target}` antes de escrever o PR.\n6. Crie o PR com \
                 `gh pr create --base {base}`. Título com menos de 80 caracteres; descrição com até cinco frases, \
                 cobrindo todas as mudanças da branch — não só as desta conversa."
            ),
        ),
    };
    format!(
        r#"{abertura}

{estado} {onde}; o alvo é `{target}`. {up}

Siga estes passos:

1. Se este repositório tiver uma skill ou comando de abrir PR (ex.: /open-pr), invoque-a agora — as instruções dela têm precedência sobre as daqui.
2. Revise o que está fora de commit com `git status` e `git diff`.
3. Commite seguindo as convenções de commit do repositório.
4. Empurre com `{push}`.
{fim}

Se algum passo falhar, pare e me pergunte."#
    )
}

/// Create a PR for each repository with changes, on the shared branch, and cross-link their
/// descriptions for review.
fn multi_pr_text(states: &[RepoPr]) -> String {
    let lista: String = states
        .iter()
        .map(|s| {
            let branch = match &s.branch {
                Some(b) => format!("branch `{b}`"),
                None => "HEAD solto (crie a branch antes de commitar)".to_string(),
            };
            let commits = match s.ahead {
                0 => format!("nenhum commit além de `{}`", s.target),
                1 => format!("1 commit além de `{}`", s.target),
                n => format!("{n} commits além de `{}`", s.target),
            };
            let dirty = match s.dirty {
                0 => "nada fora de commit".to_string(),
                1 => "1 arquivo fora de commit".to_string(),
                n => format!("{n} arquivos fora de commit"),
            };
            let pr = match s.open {
                Some(n) => format!("PR #{n} aberto — atualize, não crie outro"),
                None if s.ahead == 0 && s.dirty == 0 => "nada a entregar: fica sem PR".to_string(),
                None => "sem PR ainda".to_string(),
            };
            let up = if s.upstream { "" } else { ", sem upstream" };
            format!(
                "- `{}/` — {branch}, {commits}, {dirty}{up}. {pr}.\n",
                s.name
            )
        })
        .collect();
    format!(
        r#"Quero abrir os PRs deste workspace. Ele reúne {n} repositórios na mesma branch, cada um com histórico próprio — e cada um leva o seu PR:

{lista}
Siga estes passos em cada repositório que tem o que entregar, entrando na pasta dele antes de cada comando de git ou gh:

1. Se o repositório tiver uma skill ou comando de abrir PR (ex.: /open-pr), invoque-a nele — as instruções dela têm precedência sobre as daqui.
2. Revise o que está fora de commit com `git status` e `git diff`, e commite seguindo as convenções daquele repositório.
3. Empurre com `git push -u origin HEAD:<branch>`.
4. Revise o diff inteiro da branch contra o alvo daquele repositório antes de escrever.
5. Sem PR: crie com `gh pr create --base <alvo sem o origin/>`. Com PR aberto: confira com `gh pr view <n>` se o título e a descrição ainda cobrem tudo, e atualize com `gh pr edit <n>` se não cobrirem. Título com menos de 80 caracteres; descrição com até cinco frases, cobrindo todas as mudanças da branch naquele repositório — não só as desta conversa.
6. Com todos abertos, edite a descrição de cada um para linkar os outros PRs deste workspace ("Parte de: <url>"): quem revisa um precisa achar o resto.

Repositório sem commit além da base e sem mudança fora de commit não ganha PR.
Se algum passo falhar, pare e me pergunte."#,
        n = states.len()
    )
}

/// Return the primary worktree used for diff, branch, and PR operations. Cleaned workspaces return
/// no path.
fn workspace_copy(state: &State<AppState>, id: &str) -> Option<Workspace> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .cloned()
}

fn worktree_of(state: &State<AppState>, id: &str) -> Option<PathBuf> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| PathBuf::from(w.primary().worktree))
}

/// Return remaining workspace repositories in their saved order, primary first. Cleaned workspaces
/// return an empty list.
fn repos_of(state: &State<AppState>, id: &str) -> Vec<Repo> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| w.repos.clone())
        .unwrap_or_default()
}

/// Resolve the agent's working directory or the common parent of multiple worktrees. Project IDs
/// resolve to their registered clone, allowing file access without a workspace. Project and
/// workspace IDs never overlap.
pub(crate) fn cwd_of(state: &State<AppState>, id: &str) -> Option<PathBuf> {
    let board = lock(&state.board);
    board
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| PathBuf::from(&w.worktree))
        .or_else(|| {
            board
                .projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| PathBuf::from(&p.path))
        })
}
