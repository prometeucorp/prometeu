//! Run workspace shells, setup, and development scripts with their reserved port. Repository
//! configuration owns these commands.

use crate::lock::lock;
use crate::session::cwd_of;
use crate::state::Workspace;
use crate::{chat, i18n, pty, scripts, AppState};
use portable_pty::CommandBuilder;
use std::path::Path;
use tauri::{AppHandle, Manager, State};

/// Validate terminal and terminal-<number> keys before creating PTYs. Numbered UI tabs need
/// distinct keys, but arbitrary names must not create states the frontend cannot render.
pub(crate) fn is_terminal(kind: &str) -> bool {
    kind == "terminal"
        || kind
            .strip_prefix("terminal-")
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Keep setup, run, and auxiliary shells outside agent tabs and board state. One PTY per
/// workspace:type key preserves output and running servers across tab or workspace navigation.
#[tauri::command]
pub fn open_dock(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    kind: String,
    // A name selects scripts.run.<name>; an empty name uses the repository default.
    name: Option<String>,
    cols: u16,
    rows: u16,
) -> Result<String, String> {
    ensure_port(&state, &id);
    let found = workspace_copy(&state, &id);
    let key = format!("{id}:{kind}");

    if kind != "run" && lock(&state.ptys).get(&key).is_some_and(|p| p.alive()) {
        return Ok(key); // Already running; reuse its buffered output.
    }

    // Workspace shells receive the same script environment. A registered project without a
    // workspace can still open a shell using only its directory, without workspace script
    // variables.
    if is_terminal(&kind) {
        let root = cwd_of(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        let shell = crate::platform::shell();
        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(&root);
        cmd.env("TERM", "xterm-256color");
        for (key, value) in found.as_ref().map(script_env).unwrap_or_default() {
            cmd.env(key, value);
        }
        let handle = pty::spawn(&app, &key, cmd, cols, rows, pty::Dock::default())?;
        lock(&state.ptys).insert(key.clone(), handle);
        crate::machine::publish_counts(&app);
        return Ok(key);
    }

    // Setup and Run require a workspace repository.
    let ws = found.ok_or_else(|| i18n::t("err.session.noWorkspace"))?;

    // Setup includes copying files from the clone even when no setup command is configured.
    if kind == "setup" {
        start_setup(&app, &state, &ws, cols, rows)?;
        return Ok(key);
    }

    let found = scripts_of(&ws);
    let run = match kind.as_str() {
        "run" => found.run(name.as_deref()),
        other => return Err(i18n::ta("err.dock.unknown", &[("kind", other.to_string())])),
    }
    .ok_or_else(|| {
        i18n::ta(
            "err.dock.noScript",
            &[
                ("kind", kind.clone()),
                ("file", scripts::FILES[0].to_string()),
            ],
        )
    })?;

    let script = Script {
        kind: &kind,
        command: &run.command,
        name: Some(&run.name),
        header: None,
    };
    start_script(&app, &state, &ws, script, cols, rows)?;
    Ok(key)
}

/// Show copied files before the repository setup command. Either operation can create a Setup tab;
/// copy-only setup runs true so its header and lifecycle remain visible.
pub(crate) fn start_setup(
    app: &AppHandle,
    state: &State<AppState>,
    ws: &Workspace,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    if ws.multi() {
        return start_multi_setup(app, state, ws, cols, rows);
    }
    let found = scripts_of(ws);
    let notes = scripts::hydrate(Path::new(&ws.worktree), Path::new(&ws.repo), &found.copy);
    let report = scripts::report(&notes);
    if found.setup.is_none() && report.is_none() {
        let holes = [
            ("kind", "setup".to_string()),
            ("file", scripts::FILES[0].to_string()),
        ];
        return Err(i18n::ta("err.dock.noScript", &holes));
    }
    let script = Script {
        kind: "setup",
        name: None,
        command: found.setup.as_deref().unwrap_or("true"),
        header: report,
    };
    start_script(app, state, ws, script, cols, rows)
}

/// Run copy and setup steps sequentially for all repositories in one tab. Release the agent's first
/// message only after the last step; stop at the first failure and report its repository and exit
/// code.
fn start_multi_setup(
    app: &AppHandle,
    state: &State<AppState>,
    ws: &Workspace,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let (header, command) = multi_setup(ws).ok_or_else(|| {
        let holes = [
            ("kind", "setup".to_string()),
            ("file", scripts::FILES[0].to_string()),
        ];
        i18n::ta("err.dock.noScript", &holes)
    })?;
    let script = Script {
        kind: "setup",
        name: None,
        command: &command,
        header,
    };
    start_script(app, state, ws, script, cols, rows)
}

/// Build the combined copy header and setup command independently of PTY startup for testing.
/// Return None when no repository needs copying or setup.
pub(crate) fn multi_setup(ws: &Workspace) -> Option<(Option<String>, String)> {
    let mut header = String::new();
    let mut steps: Vec<String> = Vec::new();
    for r in &ws.repos {
        let found = scripts::read_for(Path::new(&r.worktree), Path::new(&r.path));
        let notes = scripts::hydrate(Path::new(&r.worktree), Path::new(&r.path), &found.copy);
        if let Some(report) = scripts::report(&notes) {
            header.push_str(&format!("\x1b[1m{}\x1b[0m\r\n{report}", r.name));
        }
        let Some(setup) = found.setup else { continue };
        // Run each setup command unchanged in its own subshell, worktree, and repository-specific
        // environment.
        let env: String = scripts::env(
            Path::new(&r.worktree),
            Path::new(&r.path),
            &script_name(ws),
            ws.port,
        )
        .into_iter()
        .map(|(k, v)| format!("export {k}={}; ", quoted(&v)))
        .collect();
        steps.push(format!(
            "(printf '\\n\\033[1m── {} ──\\033[0m\\n'; cd {} && {env}{setup})",
            r.name,
            quoted(&r.worktree)
        ));
    }
    if steps.is_empty() && header.is_empty() {
        return None;
    }
    let command = match steps.is_empty() {
        true => "true".to_string(),
        false => steps.join(" && "),
    };
    Some(((!header.is_empty()).then_some(header), command))
}

/// Quote shell values with single quotes, escaping embedded apostrophes.
pub(crate) fn quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Dock startup input: tab key, command, and the header written before process output.
struct Script<'a> {
    kind: &'a str,
    name: Option<&'a str>,
    command: &'a str,
    header: Option<String>,
}

/// Start a repository script unless a process already runs under the same dock key; reuse its
/// output buffer.
fn start_script(
    app: &AppHandle,
    state: &State<AppState>,
    ws: &Workspace,
    script: Script,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    if ws.cleaned {
        return Err(i18n::t("err.session.cleaned"));
    }
    let Script {
        kind,
        name,
        command,
        header,
    } = script;
    let key = format!("{}:{kind}", ws.id);
    // Use a login shell so nvm, rbenv, mise, and similar setup tools see profile exports even when
    // the app starts from Finder.
    let mut cmd = CommandBuilder::new("/bin/sh");
    cmd.args(["-lc", command]);
    // Run the primary repository's script in its worktree, not the multi-repository grouping
    // directory.
    cmd.cwd(&ws.primary().worktree);
    cmd.env("TERM", "xterm-256color");
    for (key, value) in script_env(ws) {
        cmd.env(key, value);
    }
    // Only setup completion releases pending initial agent messages.
    let on_exit: Option<pty::OnExit> = (kind == "setup").then(|| {
        let app = app.clone();
        let id = ws.id.clone();
        Box::new(move |code| release_prompts(&app, &id, code)) as pty::OnExit
    });
    // UI and MCP starts share the check and insertion so a retry cannot spawn a second service.
    let mut ptys = lock(&state.ptys);
    if let Some(active) = ptys.get(&key).filter(|p| p.alive()) {
        if active.script_name.as_deref() != name {
            return Err(i18n::t("err.dock.running"));
        }
        return Ok(());
    }
    let handle = pty::spawn(
        app,
        &key,
        cmd,
        cols,
        rows,
        pty::Dock {
            on_exit,
            header,
            script_name: name.map(str::to_string),
        },
    )?;
    ptys.insert(key, handle);
    drop(ptys);
    crate::machine::publish_counts(app);
    Ok(())
}

/// After setup completes, release pending messages for agents that are already ready. Later agent
/// startup releases the rest. Failed setup adds a warning to the message instead of leaving the
/// agent waiting indefinitely.
fn release_prompts(app: &AppHandle, workspace: &str, code: Option<u32>) {
    let state = app.state::<AppState>();
    let pending: Vec<String> = {
        let board = lock(&state.board);
        board
            .workspaces
            .iter()
            .find(|w| w.id == workspace)
            .map(|w| {
                w.tabs
                    .iter()
                    .filter(|t| t.pending_prompt.is_some())
                    .map(|t| t.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    };
    let waiting: Vec<String> = {
        let ready = lock(&state.ready);
        pending.into_iter().filter(|t| ready.contains(t)).collect()
    };
    let warning = match code {
        Some(0) => None,
        Some(n) => Some(i18n::pick(
            &format!("(O setup deste worktree saiu com código {n} — veja a aba Setup; pode faltar dependência.) "),
            &format!("(This worktree's setup exited with code {n} — see the Setup tab; a dependency may be missing.) "),
        )),
        None => Some(i18n::pick(
            "(O setup deste worktree foi encerrado antes de terminar — veja a aba Setup; pode faltar dependência.) ",
            "(This worktree's setup was stopped before it finished — see the Setup tab; a dependency may be missing.) ",
        )),
    };
    for tab in &waiting {
        chat::send_prompt(app, tab, warning.clone());
    }
}

/// Use worktree scripts, falling back to the original clone; see scripts::read_for.
pub(crate) fn scripts_of(ws: &Workspace) -> scripts::Scripts {
    let main = ws.primary();
    scripts::read_for(Path::new(&main.worktree), Path::new(&main.path))
}

/// Use the workspace directory name as the stable script identity. Renaming a card must not rename
/// containers or databases.
fn script_name(ws: &Workspace) -> String {
    Path::new(&ws.worktree)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| ws.branch.replace('/', "-"))
}

/// Run and archive use the primary repository's environment.
pub(crate) fn script_env(ws: &Workspace) -> Vec<(String, String)> {
    let main = ws.primary();
    scripts::env(
        Path::new(&main.worktree),
        Path::new(&main.path),
        &script_name(ws),
        ws.port,
    )
}

/// Assign a port lazily to older workspaces when their scripts are first requested.
pub(crate) fn ensure_port(state: &State<AppState>, id: &str) -> Option<u16> {
    let mut board = lock(&state.board);
    let ws = board.workspaces.iter().find(|w| w.id == id)?;
    // Replace previously assigned browser-blocked ports. Existing servers retain their old port
    // until restarted; new runs use the replacement.
    if let Some(port) = ws.port.filter(|p| scripts::usable(*p)) {
        return Some(port);
    }
    let worktree = ws.worktree.clone();
    let taken: Vec<u16> = board.workspaces.iter().filter_map(|w| w.port).collect();
    let port = scripts::alloc_port(Path::new(&worktree), &taken)?;
    board.workspace_mut(id)?.port = Some(port);
    if let Err(error) = board.save() {
        eprintln!("não gravei a porta do workspace: {error}");
    }
    Some(port)
}

/// Open Run in the system browser for external inspection. Resolve its port from backend state
/// rather than accepting an arbitrary URL over IPC.
#[tauri::command]
pub fn open_run(state: State<AppState>, id: String) -> Result<(), String> {
    let port = ensure_port(&state, &id).ok_or_else(|| i18n::t("err.session.noPort"))?;
    let url = format!("http://localhost:{port}");
    let ok = crate::platform::opener()
        .arg(&url)
        .status()
        .map_err(i18n::io)?
        .success();
    ok.then_some(())
        .ok_or_else(|| i18n::ta("err.session.openFailed", &[("path", url)]))
}

#[tauri::command]
pub fn close_dock(state: State<AppState>, id: String, kind: String) {
    pty::kill(&state, &format!("{id}:{kind}"));
}

/// Stop every workspace dock when archiving or removing the workspace so hidden servers cannot
/// remain active.
pub(crate) fn kill_docks(state: &State<AppState>, id: &str) {
    let prefix = format!("{id}:");
    lock(&state.ptys).retain(|key, _| !key.starts_with(&prefix));
}

/// Expose repository script declarations and the workspace port so the frontend can render
/// available controls or request configuration.
#[derive(serde::Serialize)]
pub struct ScriptsView {
    #[serde(flatten)]
    pub scripts: scripts::Scripts,
    pub port: Option<u16>,
}

/// Keep stopped docks visible with their output and exit status until replaced. This list also
/// preserves dynamically numbered terminal tabs across workspace navigation.
#[derive(serde::Serialize)]
pub struct DockView {
    pub kind: String,
    pub alive: bool,
}

/// Report dock existence and process status without starting scripts merely because the person
/// opens a log tab.
#[tauri::command]
pub fn dock_state(state: State<AppState>, id: String) -> Vec<DockView> {
    let prefix = format!("{id}:");
    lock(&state.ptys)
        .iter()
        .filter_map(|(key, pty)| {
            Some(DockView {
                kind: key.strip_prefix(&prefix)?.to_string(),
                alive: pty.alive(),
            })
        })
        .collect()
}

#[tauri::command]
pub fn workspace_scripts(state: State<AppState>, id: String) -> ScriptsView {
    let port = ensure_port(&state, &id);
    let scripts = workspace_copy(&state, &id)
        .map(|ws| scripts_of(&ws))
        .unwrap_or_default();
    ScriptsView { scripts, port }
}

/// Create the commented settings example only when no configuration exists. For worktrees
/// inheriting clone settings, copy those settings into the worktree instead, allowing isolated
/// edits. Existing Prometeu or Conductor files are returned without overwriting them.
#[tauri::command]
pub fn create_scripts_file(state: State<AppState>, id: String) -> Result<String, String> {
    let ws = workspace_copy(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let main = ws.primary();
    let root = Path::new(&main.worktree);
    let found = scripts_of(&ws);
    let (rel, text) = match found.file {
        Some(file) if !found.inherited => return Ok(file),
        Some(file) => {
            let text = std::fs::read_to_string(Path::new(&main.path).join(&file))
                .map_err(|e| e.to_string())?;
            (file, text)
        }
        None => (scripts::FILES[0].to_string(), scripts::TEMPLATE.to_string()),
    };
    let path = root.join(&rel);
    let parent = path
        .parent()
        .ok_or_else(|| i18n::t("err.session.badPath"))?;
    std::fs::create_dir_all(parent).map_err(i18n::io)?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(rel)
}

/// Build the configuration-help prompt at the backend, which knows the existing settings file's
/// path.
#[tauri::command]
pub fn scripts_prompt(state: State<AppState>, id: String) -> String {
    let file = workspace_copy(&state, &id)
        .and_then(|ws| scripts_of(&ws).file)
        .unwrap_or_else(|| scripts::FILES[0].to_string());
    scripts::ask_prompt(&file)
}

fn workspace_copy(state: &State<AppState>, id: &str) -> Option<Workspace> {
    lock(&state.board).workspace(id).cloned()
}
