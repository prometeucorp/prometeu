#[cfg(test)]
use crate::domain::Pr;
use crate::lock::lock;
use crate::selection::{Selection, Tools};
use crate::state::{
    publish, Board, Choice, Project, ProviderId, Repo, Status, Tab, ToolTrust, Workspace,
};
use crate::workspace_tools::{self, Axis};
use crate::{chat, dock, i18n, paths, scripts, AppState};
use prometeu_core::command::{CommandError, CommandPolicy, CommandRunner, OutputPolicy};
pub(crate) use prometeu_core::tool_resolution::ResolvedTools;
use prometeu_core::tool_resolution::{
    approved, axis_provenance, decided, gate_of, resolve_tools, Gate, ProjectDeclaration,
};
#[cfg(test)]
use prometeu_core::tool_resolution::{EffectiveItem, Provenance};
pub use prometeu_core::tool_resolution::{ProjectTools, WorkspaceTools};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager, State};

pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod find;
pub(crate) mod git;
mod launch;

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
    let project = Project {
        id: id.clone(),
        name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo")
            .into(),
        path: id,
    };
    prometeu_core::projects::register(board, project)
}

#[tauri::command(async)]
pub fn remove_project(app: AppHandle, state: State<AppState>, id: String) {
    remove_registered_project(&state.board, &id);
    publish(&app);
}

fn remove_registered_project(board: &std::sync::Mutex<Board>, id: &str) {
    let _sync = crate::catalog::guard();
    prometeu_core::projects::remove(&mut lock(board), id);
}

/// The sidebar lists projects in board order. Projects missing from `ids` (registered while the
/// drag was in flight) keep their relative order after the listed ones; unknown IDs are ignored.
#[tauri::command]
pub fn reorder_projects(app: AppHandle, state: State<AppState>, ids: Vec<String>) {
    {
        let _sync = crate::catalog::guard();
        sort_projects(&mut lock(&state.board).projects, &ids);
    }
    publish(&app);
}

fn sort_projects(projects: &mut [Project], ids: &[String]) {
    prometeu_core::projects::reorder(projects, ids);
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
    crate::workspace_lifecycle::finish(&mut lock(&state.board), &id);
    archive(&app, &state, &id, true);
}

fn archive(app: &AppHandle, state: &State<AppState>, id: &str, archived: bool) {
    let generation = lock(&state.telemetry).generation;
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
    let effects = crate::workspace_lifecycle::archive(&mut lock(&state.board), id, archived);
    // Stop processes outside the board lock: signalling and waiting must not block other sessions.
    stop(state, &effects.stop_tabs);
    publish(app);
    if effects.changed {
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
        let _ = workspace_lifecycle::apply(&mut board, &id, Change::Rename(title.into()));
    }
    publish(&app);
}

/// Pinning moves the workspace to the top without changing its stage.
#[tauri::command]
pub fn pin_workspace(app: AppHandle, state: State<AppState>, id: String, pinned: bool) {
    {
        let mut board = lock(&state.board);
        let _ = workspace_lifecycle::apply(&mut board, &id, Change::Pin(pinned));
    }
    publish(&app);
}

/// Interpret and validate one tool-axis argument: JSON `null` returns the axis to inherit, an
/// object must deserialize as a `Selection` carrying only ids of its own axis.
#[cfg(test)]
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
    prometeu_core::workspace_tools::patch(body, name, kind)
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
    prometeu_core::tool_resolution::record_trust(
        trust,
        declaration,
        hash,
        approved,
        crate::actions::now(),
    )
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
/// no inherited base and its rows stay hub-only. It runs off the main thread because the account
/// connectors may cost a request when the cache is cold (ADR 0063).
#[tauri::command(async)]
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
        let _ = workspace_lifecycle::apply(&mut board, &id, Change::Unread(unread));
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
        let _ = workspace_lifecycle::apply(&mut board, &id, Change::Remove);
        (dead, removed)
    };
    stop(&state, &dead);
    if removed {
        crate::plugins::forget_codex_workspace(&id);
    }
    publish(&app);
}

/* ---------- disk cleanup ---------- */

pub use prometeu_core::workspace_lifecycle::Cleanable;
use prometeu_core::workspace_lifecycle::{self, has_worktree, Change};

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

    let cleanup = &state.worktree_cleanup;
    std::thread::scope(|scope| {
        let handles: Vec<_> = mine
            .into_iter()
            .map(|ws| scope.spawn(move || cleanup.inspect(&ws)))
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
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
    state.worktree_cleanup.check(&ws, force)?;
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

    state.worktree_cleanup.remove(&ws)?;
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

#[cfg(test)]
use prometeu_git::cleanup::{check, hard};

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

pub use prometeu_core::workspace_draft::Draft;

pub use prometeu_core::session::launch::Launch;

pub(crate) fn project_declaration(worktree: &Path, repo: &Path) -> Option<ProjectDeclaration> {
    prometeu_files::tool_declarations::project_declaration(
        &prometeu_files::settings::NativeSettings,
        worktree,
        repo,
    )
}
#[cfg(test)]
use prometeu_files::tool_declarations::tools_hash;

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
    let project = trusted_project(trust, ws);
    let workspace = ws.tools();
    let (mcp_base, mut mcp_universe) = mcp_base_and_universe(ws, agent);
    if agent == ProviderId::Claude {
        keep_selected_connectors(&mut mcp_universe, [global, &project, &workspace]);
    }
    let plugin_hub: Vec<String> = crate::plugins::load().into_iter().map(|p| p.id).collect();
    resolve_tools(
        global,
        &project,
        &workspace,
        &mcp_base,
        &mcp_universe,
        &plugin_hub,
    )
}

/// Keep explicit connector IDs until strict materialization can verify their configuration.
fn keep_selected_connectors(universe: &mut Vec<String>, layers: [&Tools; 3]) {
    for id in layers
        .into_iter()
        .filter_map(|tools| tools.mcp.as_ref())
        .flat_map(|selection| &selection.add)
    {
        if id.starts_with("claude.ai ") && !universe.contains(id) {
            universe.push(id.clone());
        }
    }
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

/// Default conversations inherit the workspace's model and effort and carry the already resolved
/// tools. Plan mode remains an explicit launcher choice.
pub(crate) fn workspace_launch(workspace: &Workspace, tools: &ResolvedTools) -> Launch {
    Launch {
        agent: workspace.agent,
        model: workspace.model.clone(),
        effort: workspace.effort.clone(),
        plan: false,
        mcp: tools.mcp.clone(),
        mcp_inherits_base: tools.mcp_inherits_base,
        plugins: tools.plugins.clone(),
        skills: tools.skills.clone(),
        ..Default::default()
    }
}

/// Resume with the tab's model override or workspace defaults. Ordinary tabs carry the resolved
/// tools; tasks retain their frozen profile, instructions, and permissions.
pub(crate) fn workspace_launch_of(
    workspace: &Workspace,
    tab: &str,
    tools: &ResolvedTools,
) -> Launch {
    if let Some(run) = workspace
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
    let mut launch = workspace_launch_with(
        workspace,
        workspace
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.choice.clone()),
        tools,
    );
    if launch.agent == ProviderId::Antigravity {
        if let Some(tab) = workspace.tabs.iter().find(|t| t.id == tab) {
            launch.plan = tab.plan;
            launch.permission = tab.permission;
        }
    }
    launch
}

/// Model overrides preserve the resolved tool selection for new and resumed tabs.
fn workspace_launch_with(
    workspace: &Workspace,
    choice: Option<Choice>,
    tools: &ResolvedTools,
) -> Launch {
    choice.map_or_else(
        || workspace_launch(workspace, tools),
        |choice| Launch {
            mcp: tools.mcp.clone(),
            mcp_inherits_base: tools.mcp_inherits_base,
            plugins: tools.plugins.clone(),
            skills: tools.skills.clone(),
            ..Launch::from(choice)
        },
    )
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
            if draft.new_branch == Some(false) {
                existing_branch_source(Path::new(&r.path), &branch, draft.source.as_deref())?;
            }
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
    let preserve_branches = branches_to_preserve(&repos, &branch, draft.worktree, draft.new_branch);

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
        preserve_branches,
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
        if draft.new_branch == Some(false) {
            for r in &repos {
                existing_branch_source(Path::new(&r.path), &branch, draft.source.as_deref())?;
            }
        }
        // If any repository fails, roll back only the worktrees created by this preparation. A
        // partially assembled workspace is not usable.
        let mut feitos: Vec<(PathBuf, PathBuf)> = Vec::new();
        for r in &repos {
            let (clone, dest) = (PathBuf::from(&r.path), PathBuf::from(&r.worktree));
            let base = if draft.new_branch == Some(false) {
                draft.source.as_deref().unwrap_or("")
            } else {
                &r.base
            };
            match add_worktree(state.command_runner.as_ref(), &clone, &branch, base, &dest) {
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
        switch_branch(state.command_runner.as_ref(), &repo, &branch, &draft.base)?;
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
            launch.mcp_inherits_base = tools.mcp_inherits_base;
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
        let launch = workspace_launch_with(&ws, choice.clone(), &tools);
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
        let _ = workspace_lifecycle::apply(
            &mut board,
            &workspace,
            Change::RenameTab {
                tab,
                title: title.into(),
            },
        );
    }
    publish(&app);
}

/// Restart the process through the shared launch workflow and injected desktop effects.
pub fn revive(app: &AppHandle, state: &State<AppState>, tab: &str) -> Result<bool, String> {
    launch::resume(app, state, tab)
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
    launch::start(
        app,
        state,
        &prometeu_core::session::launch::LaunchRequest {
            session: id.clone(),
            workspace: workspace.into(),
            worktree: worktree.to_string_lossy().into_owned(),
            settings: launch.clone(),
            mode: prometeu_core::session::launch::StartMode::Fresh,
        },
    )?;
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
        context_window: None,
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

use prometeu_core::workspace_draft::first_message;

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
fn add_worktree(
    runner: &dyn CommandRunner<Command>,
    repo: &Path,
    branch: &str,
    base: &str,
    dest: &Path,
) -> Result<bool, String> {
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
            if let Err(e) = prepare_base(runner, repo, base) {
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

/// Reusing a branch must not silently create a different branch from the default base.
fn existing_branch_source(repo: &Path, branch: &str, source: Option<&str>) -> Result<(), String> {
    let source = source.unwrap_or("");
    let local = has_commit(repo, &format!("refs/heads/{branch}"));
    if source == branch && local {
        return Ok(());
    }
    if source.ends_with(&format!("/{branch}"))
        && has_commit(repo, &format!("refs/remotes/{source}"))
        && !local
    {
        return Ok(());
    }
    Err(i18n::ta(
        "err.session.branchUnavailable",
        &[("branch", source.to_string())],
    ))
}

fn branches_to_preserve(
    repos: &[Repo],
    branch: &str,
    worktree: bool,
    new_branch: Option<bool>,
) -> Vec<String> {
    repos
        .iter()
        .filter(|r| {
            worktree
                && (new_branch == Some(false)
                    || has_commit(Path::new(&r.path), &format!("refs/heads/{branch}")))
        })
        .map(|r| r.path.clone())
        .collect()
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
fn switch_branch(
    runner: &dyn CommandRunner<Command>,
    repo: &Path,
    branch: &str,
    base: &str,
) -> Result<(), String> {
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
            prepare_base(runner, repo, base)?;
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
    prometeu_core::repository::head_name(&git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]))
}

/// Refresh only the requested remote base. If the network fails, the existing local ref remains
/// usable.
fn prepare_base(
    runner: &dyn CommandRunner<Command>,
    repo: &Path,
    base: &str,
) -> Result<(), String> {
    if let Some((remote, rest)) = base.split_once('/') {
        if has_commit(repo, &format!("refs/remotes/{base}"))
            && git(repo, &["remote"]).lines().any(|name| name == remote)
        {
            let _ = fetch(runner, repo, remote, rest);
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
fn fetch(
    runner: &dyn CommandRunner<Command>,
    repo: &Path,
    remote: &str,
    branch: &str,
) -> Result<(), String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args(["fetch", "--quiet", remote, branch]);
    runner
        .run(
            &mut command,
            &[],
            CommandPolicy {
                timeout: std::time::Duration::from_secs(10),
                stdout: OutputPolicy::Inherit,
                stderr: OutputPolicy::Inherit,
            },
        )
        .map(|_| ())
        .map_err(|error| match error {
            CommandError::Timeout => i18n::t("err.git.fetchSlow"),
            CommandError::Unavailable => i18n::io("git executable not found"),
            CommandError::Io(cause) => i18n::io(cause),
            _ => i18n::io("unexpected fetch output failure"),
        })
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

pub use prometeu_core::repository::Branches;
/// List repository branches using the shared launcher's reference selection policy.
#[tauri::command(async)]
pub fn list_branches(project: String) -> Branches {
    let repo = PathBuf::from(expand(&project));
    prometeu_core::repository::branches(|args| git(&repo, args), repo.join(".git").exists())
}

#[cfg(test)]
mod tests {
    use super::{
        artifacts_of, first_message, linked_pr_text, multi_pr_text, patch_map, pr_text,
        resolve_tools, workspace_launch, workspace_launch_of, workspace_launch_with, Choice, Draft,
        Pr, ProviderId, Repo, RepoPr, ResolvedTools, Tab, Workspace,
    };
    use crate::dock::{is_terminal, multi_setup, quoted};
    use std::path::Path;

    #[test]
    fn project_order_follows_ids_and_keeps_unlisted_projects_last() {
        let mut board = crate::state::Board::default();
        for path in ["/a", "/b", "/c", "/d"] {
            super::register_project(&mut board, Path::new(path));
        }
        let ids = ["/c", "/gone", "/a"].map(String::from);
        super::sort_projects(&mut board.projects, &ids);
        let order: Vec<_> = board.projects.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(order, ["/c", "/a", "/b", "/d"]);
    }

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
        super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "work",
            "origin/main",
            &dest,
        )
        .unwrap();

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
            preserve_branches: Vec::new(),
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
        ws.preserve_branches = vec![ws.repos[0].path.clone()];
        super::check(&ws).unwrap();
        ws.preserve_branches.clear();

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
        let local2 = root.join("clone2");
        run(
            &origin,
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                local2.to_str().unwrap(),
            ],
        );
        let dest2 = root.join("wt2");
        super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local2,
            "work",
            "origin/main",
            &dest2,
        )
        .unwrap();
        ws.repos.push(Repo {
            path: local2.display().to_string(),
            name: "clone-2".into(),
            worktree: dest2.display().to_string(),
            base: String::new(),
            pr: None,
        });
        ws.repos[0].pr = None;
        ws.preserve_branches = vec![local.display().to_string()];
        super::check(&ws).unwrap();
        std::fs::write(dest2.join("d.txt"), "d").unwrap();
        run(&dest2, &["config", "user.email", "t@t"]);
        run(&dest2, &["config", "user.name", "t"]);
        run(&dest2, &["add", "-A"]);
        run(&dest2, &["commit", "-qm", "other repo work"]);
        assert!(super::check(&ws).unwrap_err().contains("unmerged"));
        ws.repos[1].pr = Some(pr(4, "work", "MERGED"));
        super::check(&ws).unwrap();
        std::fs::write(dest2.join("e.txt"), "e").unwrap();
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

    #[test]
    fn linked_pull_prompt_preserves_identity_instead_of_publishing_the_review_branch() {
        let url = "https://github.com/upstream/repo/pull/42";
        let prompt = linked_pr_text(url).unwrap();
        assert!(prompt.contains(&format!("gh pr view {url} --json state,headRefName,headRepository,headRepositoryOwner,baseRefName")));
        assert!(prompt.contains(&format!("gh pr edit {url}")));
        assert!(!prompt.contains("gh pr create"));
        assert!(!prompt.contains("git push -u origin"));
        assert!(linked_pr_text("https://github.com/upstream/repo/issues/42").is_none());
        assert!(linked_pr_text("https://linear.app/team/issue/42").is_none());
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
            preserve_branches: Vec::new(),
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

    #[test]
    fn legacy_workspace_defaults_to_removing_its_branch() {
        let mut value = serde_json::to_value(bare()).unwrap();
        value.as_object_mut().unwrap().remove("preserve_branches");
        let restored: Workspace = serde_json::from_value(value).unwrap();
        assert!(restored.preserve_branches.is_empty());
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
            context_window: None,
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
                    mcp_inherits_base: false,
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
                    workspace_launch_with(&ws, Some(choice), &tools),
                    workspace_launch_of(&ws, "custom", &tools),
                ] {
                    assert_eq!(launch.agent, provider);
                    assert_eq!(launch.model, "tab-model");
                    assert_eq!(launch.effort, "medium");
                    assert_eq!(launch.mcp, selected);
                    assert_eq!(launch.plugins, selected);
                    assert!(!launch.plan);
                }
                for launch in [
                    workspace_launch_with(&ws, None, &tools),
                    workspace_launch_of(&ws, "inherited", &tools),
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
        let launch = workspace_launch(&ws, &tools);
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

    #[test]
    fn explicit_account_connector_survives_replacement_until_materialization() {
        use crate::selection::{Selection, Tools};
        let hub = "hub".to_string();
        let connector = "claude.ai Drive".to_string();
        let global = Tools::default();
        let project = Tools::default();
        let workspace = Tools {
            mcp: Some(Selection::only(vec![hub.clone(), connector.clone()])),
            ..Default::default()
        };
        let mut universe = vec![hub.clone()];
        super::keep_selected_connectors(&mut universe, [&global, &project, &workspace]);
        let resolved = resolve_tools(&global, &project, &workspace, &[], &universe, &[]);
        assert_eq!(resolved.mcp, Some(vec![hub, connector]));
        assert!(!resolved.mcp_inherits_base);
        assert!(
            prometeu_tools::mcp::config_body(&[], resolved.mcp.as_ref().unwrap(), |_| None)
                .is_err()
        );
        let inherited = Tools {
            mcp: Some(Selection::default()),
            ..Default::default()
        };
        assert!(
            resolve_tools(&inherited, &project, &Tools::default(), &[], &[], &[]).mcp_inherits_base
        );
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
                let launch = workspace_launch_with(&ws, Some(choice.clone()), &tools);
                assert_eq!(launch.agent, agent);
                assert_eq!(launch.mcp.as_ref().unwrap(), &expected);
                ws.tabs = vec![tab("mixed", Some(choice))];
                let resumed = workspace_launch_of(&ws, "mixed", &tools);
                assert_eq!(resumed.agent, agent);
                assert_eq!(resumed.mcp.as_ref().unwrap(), &expected);
                // Materialize the exact resolved set at the provider boundary.
                match agent {
                    ProviderId::Claude => {
                        crate::mcp::config_for(
                            "test",
                            launch.mcp.as_deref(),
                            Path::new(&ws.worktree),
                        )
                        .unwrap();
                    }
                    ProviderId::Codex | ProviderId::Antigravity | ProviderId::RetiredGemini => {
                        crate::mcp::codex_config("test", launch.mcp.as_deref()).unwrap();
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
            mcp_inherits_base: false,
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
        let launch = workspace_launch_of(&ws, "task", &ResolvedTools::default());
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

        let inherited = workspace_launch_of(&ws, "inherited", &ResolvedTools::default());
        assert_eq!(
            (inherited.model.as_str(), inherited.effort.as_str()),
            ("opus[1m]", "high")
        );
        assert_eq!(inherited.agent, ProviderId::Claude);

        let explicit_choice = workspace_launch_of(&ws, "explicit", &ResolvedTools::default());
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
            workspace_launch_of(&ws, "missing", &ResolvedTools::default()).model,
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
        let launch = workspace_launch_of(&ws, "open", &ResolvedTools::default());
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
        assert!(branches.local.contains(&"main".to_string()));
        assert!(!branches.local.contains(&"origin/old".to_string()));
        assert!(
            branches.all.contains(&"origin/old".to_string()),
            "{:?}",
            branches.all
        );

        let repos = [&origin, &local].map(|path| Repo {
            path: path.display().to_string(),
            name: String::new(),
            worktree: String::new(),
            base: String::new(),
            pr: None,
        });
        assert_eq!(
            super::branches_to_preserve(&repos, "old", true, Some(true)),
            vec![origin.display().to_string()]
        );
        assert_eq!(
            super::branches_to_preserve(&repos, "old", true, Some(false)),
            repos.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
        );

        let dest = root.join("wt");
        super::existing_branch_source(&local, "old", Some("origin/old")).unwrap();
        assert!(super::existing_branch_source(&local, "missing", Some("origin/missing")).is_err());
        assert!(super::existing_branch_source(&local, "new", Some("origin/old")).is_err());
        assert!(super::existing_branch_source(&local, "main", Some("origin/main")).is_err());
        super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "new",
            "origin/old",
            &dest,
        )
        .unwrap();
        assert_eq!(run(&dest, &["rev-parse", "HEAD"]), old);
        assert_eq!(run(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]), "new");

        // Reject a base that does not exist.
        let error = super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "other",
            "origin/missing",
            &root.join("wt2"),
        );
        assert!(error.unwrap_err().contains("missing"));

        // Reuse a worktree already on the requested branch so repeated creation is safe.
        super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "new",
            "origin/old",
            &dest,
        )
        .unwrap();
        // Reject an existing worktree on the wrong branch.
        let error = super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "other-branch",
            "origin/main",
            &dest,
        )
        .unwrap_err();
        assert!(error.contains("new"), "{error}");

        // Without worktree isolation, move the original clone's HEAD to a branch at the selected
        // base without creating a directory.
        super::switch_branch(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "here",
            "origin/old",
        )
        .unwrap();
        assert_eq!(run(&local, &["rev-parse", "--abbrev-ref", "HEAD"]), "here");
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), old);
        // Selecting the current branch is a no-op.
        super::switch_branch(
            &prometeu_process::command::UnixCommandRunner,
            &local,
            "here",
            "origin/main",
        )
        .unwrap();
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), old);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn synthetic_pull_ref_seeds_worktree_without_fetch_or_clone_changes() {
        struct NoFetch;
        impl prometeu_core::command::CommandRunner<Command> for NoFetch {
            fn run(
                &self,
                request: &mut Command,
                _: &[u8],
                _: prometeu_core::command::CommandPolicy,
            ) -> Result<prometeu_core::command::CommandOutput, prometeu_core::command::CommandError>
            {
                panic!("synthetic remote must not fetch: {request:?}");
            }
        }
        let root = std::env::temp_dir().join(format!("prometeu-pull-{}", uuid::Uuid::new_v4()));
        let clone = root.join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        let run = |dir: &Path, args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        run(&clone, &["init", "-q", "-b", "main"]);
        run(&clone, &["config", "user.email", "test@example.com"]);
        run(&clone, &["config", "user.name", "Test"]);
        run(&clone, &["commit", "--allow-empty", "-qm", "base"]);
        let base = run(&clone, &["rev-parse", "HEAD"]);
        run(&clone, &["update-ref", "refs/remotes/origin/main", &base]);
        run(&clone, &["checkout", "-qb", "existing-work"]);
        std::fs::write(clone.join("review.txt"), "Review this change.\n").unwrap();
        run(&clone, &["add", "review.txt"]);
        run(&clone, &["commit", "-qm", "pull head"]);
        let head = run(&clone, &["rev-parse", "HEAD"]);
        let branch = "github-pr-42-isolated";
        let source = format!("prometeu-pr-42/{branch}");
        run(
            &clone,
            &["update-ref", &format!("refs/remotes/{source}"), &head],
        );
        run(&clone, &["checkout", "-q", "main"]);
        std::fs::write(clone.join("local.txt"), "Keep local changes.\n").unwrap();
        let before = run(&clone, &["status", "--porcelain"]);
        let repo = Repo {
            path: clone.display().to_string(),
            name: "clone".into(),
            worktree: root.join("review").display().to_string(),
            base: "origin/main".into(),
            pr: None,
        };
        super::existing_branch_source(&clone, branch, Some(&source)).unwrap();
        super::add_worktree(&NoFetch, &clone, branch, &source, Path::new(&repo.worktree)).unwrap();
        assert_eq!(run(Path::new(&repo.worktree), &["rev-parse", "HEAD"]), head);
        assert_eq!(
            super::ahead_of(Path::new(&repo.worktree), &repo.base),
            (base.clone(), 1)
        );
        assert_eq!(run(&clone, &["rev-parse", "HEAD"]), base);
        assert_eq!(run(&clone, &["branch", "--show-current"]), "main");
        assert_eq!(run(&clone, &["rev-parse", "existing-work"]), head);
        assert_eq!(run(&clone, &["status", "--porcelain"]), before);
        assert_eq!(
            std::fs::read_to_string(clone.join("local.txt")).unwrap(),
            "Keep local changes.\n"
        );
        std::fs::remove_dir_all(root).unwrap();
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
        assert!(super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &repo,
            "aut-49",
            "",
            &first
        )
        .unwrap());

        // Request the same branch at the multi-repository path.
        let second = root.join("code-rules+autonomous").join("aut-49");
        let error = super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &repo,
            "aut-49",
            "",
            &second.join("code-rules"),
        )
        .unwrap_err();
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
        assert!(!super::add_worktree(
            &prometeu_process::command::UnixCommandRunner,
            &repo,
            "aut-49",
            "",
            &first
        )
        .unwrap());

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
            preserve_branches: Vec::new(),
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
    if let Some(prompt) = workspace_copy(&state, &id)
        .and_then(|workspace| workspace.issue)
        .and_then(|issue| linked_pr_text(&issue.url))
    {
        return Ok(prompt);
    }
    let states: Vec<RepoPr> = repos.iter().map(pr_state).collect();
    Ok(match states.as_slice() {
        [one] => pr_text(one),
        many => multi_pr_text(many),
    })
}

// A review branch is isolated from the PR head; never infer its push destination from its name.
fn linked_pr_text(url: &str) -> Option<String> {
    crate::github_issues::target(url).filter(|(_, kind, _)| kind == "pull")?;
    Some(i18n::pick(
        &format!("Atualize a PR existente {url}. Respeite as instruções de PR do repositório. Consulte `gh pr view {url} --json state,headRefName,headRepository,headRepositoryOwner,baseRefName` antes de alterar qualquer remoto. Esta branch local é isolada: confirme o repositório e a branch de origem da PR e a permissão de escrita antes de enviar commits. Revise as mudanças, execute as verificações adequadas e faça commit do trabalho solicitado. Envie somente para a branch de origem confirmada, sem force push; se a PR estiver fechada, faltar permissão ou houver divergência, pare e explique. Não crie outra PR nem publique a branch local com outro nome. Revise título e descrição com `gh pr view {url}` e atualize com `gh pr edit {url}` somente se necessário."),
        &format!("Update the existing PR {url}. Follow the repository's PR instructions. Inspect `gh pr view {url} --json state,headRefName,headRepository,headRepositoryOwner,baseRefName` before changing any remote. This local branch is isolated: confirm the PR's head repository and branch and write permission before pushing commits. Review changes, run appropriate checks and commit the requested work. Push only to the confirmed head branch without force pushing; if the PR is closed, permission is missing or history diverges, stop and explain. Do not create another PR or publish the local branch under another name. Review the title and description with `gh pr view {url}` and update with `gh pr edit {url}` only if needed."),
    ))
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

#[cfg(test)]
mod fetch_port_tests {
    use super::*;
    use prometeu_core::command::CommandOutput;
    struct Runner(bool);
    impl CommandRunner<Command> for Runner {
        fn run(
            &self,
            request: &mut Command,
            input: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.get_program(), "git");
            assert_eq!(
                request.get_args().collect::<Vec<_>>(),
                ["-C", "/fixture", "fetch", "--quiet", "origin", "main"]
            );
            assert!(input.is_empty());
            assert_eq!(policy.timeout, std::time::Duration::from_secs(10));
            assert_eq!(policy.stdout, OutputPolicy::Inherit);
            match self.0 {
                true => Ok(CommandOutput {
                    success: false,
                    stdout: vec![],
                    stderr: vec![],
                }),
                false => Err(CommandError::Timeout),
            }
        }
    }
    #[test]
    fn preparation_fetch_preserves_nonzero_fallback_and_timeout_code() {
        assert!(fetch(&Runner(true), Path::new("/fixture"), "origin", "main").is_ok());
        assert_eq!(
            fetch(&Runner(false), Path::new("/fixture"), "origin", "main"),
            Err(i18n::t("err.git.fetchSlow"))
        );
    }
}
