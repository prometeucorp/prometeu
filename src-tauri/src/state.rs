//! Projects register repositories. Workspaces group branches and worktrees. Tabs hold agent
//! sessions that share those files. A conversation can close without deleting its worktree; another
//! tab can continue using the same files.

use crate::lock::lock;
use crate::selection::{Base, Selection, Tools};
use crate::{paths, AppState};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Persisted agent runtime identity. Legacy empty strings deserialize as Claude and save as
/// `"claude"`. Unknown values use the default so newer boards remain readable by older app
/// versions.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    #[default]
    Claude,
    Codex,
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "codex" => Self::Codex,
            "" | "claude" => Self::Claude,
            _ => Self::default(),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The agent is working.
    Rodando,
    /// The process is alive and waiting for input.
    Pronta,
    /// The agent needs an answer before continuing.
    Querendo,
    /// The process is stopped; its transcript remains available for resume. Tabs start here when
    /// the app reopens. `serde(other)` accepts legacy values and must remain last, so urgency
    /// belongs in `rank`.
    #[serde(other)]
    Desligada,
}

impl Status {
    /// A workspace displays its most urgent tab so a pending question remains visible.
    pub fn rank(self) -> u8 {
        match self {
            Status::Querendo => 3,
            Status::Rodando => 2,
            Status::Pronta => 1,
            Status::Desligada => 0,
        }
    }
}

/// An explicit activity update. Clearing a finished tool differs from preserving the current
/// activity.
pub enum Note {
    /// Clear the activity when work stops.
    Clear,
    Set(String),
    /// Preserve the current tool activity across unrelated events.
    Keep,
}

/// Provider, model and effort for a conversation. A missing tab override inherits workspace
/// defaults; an empty model explicitly selects the CLI default.
#[derive(Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Choice {
    #[serde(default)]
    pub agent: ProviderId,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Tab {
    #[serde(default)]
    pub task: Option<crate::actions::Run>,
    /// The local session ID, also used by Claude for its transcript.
    pub id: String,
    /// The provider's session ID when it differs from the local ID. Codex supplies the thread ID
    /// used by `thread/resume`. Absent for Claude or a Codex tab whose thread has not opened yet.
    #[serde(default)]
    pub agent_session: Option<String>,
    pub title: String,
    pub status: Status,
    pub note: Option<String>,
    pub pending_prompt: Option<String>,
    /// Cumulative token estimate. Context after compaction adds to the previous total.
    #[serde(default)]
    pub tokens: Option<u64>,
    /// Last observed context size. Only growth is added; a decrease starts a new segment after
    /// compaction.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    /// An optional model override selected when opening or retuning a tab. `None` inherits the
    /// workspace; selecting its model again removes the override. Resume preserves the tab's
    /// choice.
    #[serde(default)]
    pub choice: Option<Choice>,
}

impl Tab {
    pub fn observe_tokens(&mut self, current: u64) {
        if current == 0 {
            return;
        }
        // Legacy `tokens` stored the context size. Treat it as both total and cursor to avoid
        // counting it twice.
        let previous = self.context_tokens.or(self.tokens).unwrap_or(0);
        let added = match current >= previous {
            true => current - previous,
            false => current,
        };
        self.tokens = Some(self.tokens.unwrap_or(0).saturating_add(added));
        self.context_tokens = Some(current);
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// A repository's source clone and working copy. Multi-repository workspaces keep one adjacent
/// worktree per repository on the same branch.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Repo {
    /// The clone registered as a project.
    pub path: String,
    /// The clone directory name, used in the UI and for its worktree directory.
    pub name: String,
    /// The worktree path, or the clone itself when the workspace uses it directly.
    pub worktree: String,
    /// The base used for commits and diffs. The launcher chooses the primary repository's base;
    /// other repositories use their own defaults. Legacy empty values are resolved and saved by the
    /// diff path.
    #[serde(default)]
    pub base: String,
    /// The last PR result from `gh` for this repository and branch. The board caches it for badges
    /// and controls. `None` covers both an unchecked branch and one without a PR.
    #[serde(default)]
    pub pr: Option<crate::domain::Pr>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Workspace {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
    /// Legacy primary repository fields remain for board compatibility. The first `repos` entry is
    /// authoritative.
    pub repo: String,
    pub repo_name: String,
    pub branch: String,
    /// The agent's working directory: a worktree or clone for one repository, or the parent
    /// containing all worktrees for multiple repositories. File navigation and dock shells start
    /// here.
    pub worktree: String,
    /// Repositories in primary-first order. `revive` reconstructs a single entry from legacy `repo`
    /// and `worktree` fields when the list is absent.
    #[serde(default)]
    pub repos: Vec<Repo>,
    /// The user-selected work stage, independent of runtime `Status`. `column` is its legacy name.
    #[serde(alias = "column")]
    pub stage: String,
    /// Hidden from the active list; the worktree, branch and transcript remain intact.
    #[serde(default)]
    pub archived: bool,
    /// Pinned to the top without changing work stage or runtime status.
    #[serde(default)]
    pub pinned: bool,
    /// Activity occurred while another workspace was visible.
    #[serde(default)]
    pub unread: bool,
    /// The workspace's default model, inherited unless a tab overrides it. Empty values preserve
    /// legacy CLI-default behavior; explicit values may be aliases or full model names.
    #[serde(default)]
    pub model: String,
    /// The default effort level. An empty legacy value leaves the CLI default unchanged.
    #[serde(default)]
    pub effort: String,
    /// The default provider, resolved from the launcher's model selection. `ProviderId` normalizes
    /// legacy empty Claude values; tabs inherit it unless they have an override.
    #[serde(default)]
    pub agent: ProviderId,
    /// The first of ten reserved ports, exposed as `$PROMETEU_PORT` through `+9`. Persisted so
    /// repeated script runs use the same ports. Legacy workspaces receive a reservation when
    /// scripts are first opened.
    #[serde(default)]
    pub port: Option<u16>,
    /// The Linear issue that originated this workspace, used by board badges and issue ownership.
    #[serde(default)]
    pub issue: Option<crate::linear::IssueRef>,
    /// Legacy workspace-level PR, moved to the primary repository by `revive` and never saved here
    /// again.
    #[serde(default, skip_serializing)]
    pub pr: Option<crate::domain::Pr>,
    /// The worktree and local branch have been removed. History remains, but terminals cannot
    /// reopen here.
    #[serde(default)]
    pub cleaned: bool,
    /// Sharing consent persists across app restarts. The frontend advertises this workspace and
    /// forwards conversation output to authorized viewers through the relay.
    #[serde(default)]
    pub shared: bool,
    /// Identity and organization that received explicit sharing consent. Legacy boards omit this.
    #[serde(default)]
    pub share_team: Option<String>,
    /// Member IDs allowed to view a shared workspace, or `None` for the entire team. The relay
    /// enforces access.
    #[serde(default)]
    pub audience: Option<Vec<String>>,
    /// Allow companion devices belonging to the owner to view and control this workspace.
    #[serde(default)]
    pub remote_control: bool,
    /// Worktree preparation is running after the launcher closes. The board can show progress
    /// before any agent tab exists; processes start only after their working directories are ready.
    #[serde(default)]
    pub preparing: bool,
    /// Preparation failure encoded for `fromBack`. Keeping the card lets the user inspect and
    /// resolve an existing branch or partially prepared workspace.
    #[serde(default)]
    pub failed: Option<String>,
    /// MCP servers selected from the hub for this workspace, as the workspace layer of the tool
    /// selection. `None` inherits the layers above; a replacement with an empty `add` selects no MCP
    /// servers. Legacy boards stored `Option<Vec<String>>`; `legacy_axis` migrates it on load.
    #[serde(default, deserialize_with = "legacy_axis")]
    pub mcp: Option<Selection>,
    /// Plugins selected from the hub for this workspace, as the workspace layer. Legacy boards
    /// stored `Option<Vec<String>>` and mixed standalone skills in; `revive` moves `skill-<id>`
    /// entries to the `skills` axis.
    #[serde(default, deserialize_with = "legacy_axis")]
    pub plugins: Option<Selection>,
    /// Standalone skills selected from the hub for this workspace, as their own axis. They still
    /// materialize through the plugin-package pipeline. Absent, therefore inherited, in old boards.
    #[serde(default, deserialize_with = "legacy_axis")]
    pub skills: Option<Selection>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub active: Option<String>,
}

/// Read one workspace tool axis from either the layered `Selection` object or the legacy
/// `Option<Vec<String>>` form, so a pre-migration board still loads: `null` and an absent field
/// inherit, `[]` and `[ids]` become a replacement, and an object is the layered form. The migration
/// is one-way; see docs/contracts/persistence.md and ADR 0043.
fn legacy_axis<'de, D>(deserializer: D) -> Result<Option<Selection>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        // The legacy array must be tried first: an empty array also deserializes as a `Selection`
        // with every field defaulted, which would lose the `base: none` replacement meaning.
        Legacy(Vec<String>),
        New(Selection),
    }
    Ok(
        Option::<Form>::deserialize(deserializer)?.map(|form| match form {
            Form::New(selection) => selection,
            Form::Legacy(ids) => Selection::only(ids),
        }),
    )
}

impl Workspace {
    /// The primary repository, with legacy fields as fallback before normalization or in tests.
    pub fn primary(&self) -> Repo {
        self.repos.first().cloned().unwrap_or_else(|| Repo {
            path: self.repo.clone(),
            name: self.repo_name.clone(),
            base: String::new(),
            pr: None,
            worktree: self.worktree.clone(),
        })
    }

    /// Existing PRs in repository order.
    pub fn prs(&self) -> impl Iterator<Item = (&Repo, &crate::domain::Pr)> {
        self.repos
            .iter()
            .filter_map(|r| r.pr.as_ref().map(|pr| (r, pr)))
    }

    /// True when at least one PR exists and every existing PR is merged, enabling completion
    /// controls.
    pub fn merged(&self) -> bool {
        let mut any = false;
        for (_, pr) in self.prs() {
            if !pr.merged() {
                return false;
            }
            any = true;
        }
        any
    }

    /// Multiple repositories use a parent directory containing their worktrees.
    pub fn multi(&self) -> bool {
        self.repos.len() > 1
    }

    /// The most urgent tab determines the workspace status.
    pub fn status(&self) -> Status {
        self.tabs
            .iter()
            .map(|t| t.status)
            .max_by_key(|s| s.rank())
            .unwrap_or(Status::Desligada)
    }

    /// The most urgent tab also supplies the workspace activity.
    pub fn note(&self) -> Option<String> {
        self.tabs
            .iter()
            .max_by_key(|t| t.status.rank())
            .and_then(|t| t.note.clone())
    }
}

/// One project-trust decision (ADR 0043). A repository's versioned `[tools]` declaration activates
/// only after the person approves it for the current hash; a changed hash needs a new decision. The
/// decision is app-local on the board and never written into the repository, so cloning a project
/// cannot activate its packages silently. See docs/contracts/persistence.md.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ToolTrust {
    /// Repository identity: the `origin` remote URL when one exists, else the clone's absolute path.
    pub repo: String,
    /// SHA-256 of the declared `[tools]` section, in canonical form.
    pub hash: String,
    pub approved: bool,
    /// Epoch seconds when the decision was recorded.
    #[serde(default)]
    pub at: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Board {
    #[serde(default)]
    pub actions: crate::actions::Catalog,
    /// The global layer of the tool selection, one `Selection` per axis, app-local under the root.
    /// Absent by default so an old board keeps injecting exactly what it used to. See
    /// docs/contracts/persistence.md and ADR 0043.
    #[serde(default)]
    pub tools: Tools,
    /// Per-repository decisions that let a versioned project `[tools]` declaration activate. Empty
    /// by default, so an old board has approved nothing and project-declared items stay uninjected
    /// until the person decides. See docs/contracts/persistence.md and ADR 0043.
    #[serde(default)]
    pub tool_trust: Vec<ToolTrust>,
    /// Ordered work stages. Position determines each stage's icon.
    #[serde(alias = "columns")]
    pub stages: Vec<String>,
    #[serde(default)]
    pub projects: Vec<Project>,
    /// `cards` is the legacy name from when a workspace contained one session.
    #[serde(alias = "cards")]
    pub workspaces: Vec<Workspace>,
}

/// Legacy generated titles (`conversa`, `conversa 2`) are removed during migration.
fn is_placeholder_title(title: &str) -> bool {
    match title.strip_prefix("conversa") {
        Some(rest) => rest.trim().chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Standalone skills ride the plugin hub as `skill-<id>` but are selected on their own axis
/// (ADR 0043). Legacy boards stored them inside `plugins`; move them to `skills`, preserving the
/// source layer's base mode. Idempotent, so it is safe on every load: a migrated board has no
/// `skill-<id>` left in `plugins`.
pub(crate) fn split_skills(plugins: &mut Option<Selection>, skills: &mut Option<Selection>) {
    let is_skill = |id: &str| id.starts_with("skill-");
    let Some(source) = plugins else { return };
    let moved_add: Vec<String> = source
        .add
        .iter()
        .filter(|id| is_skill(id))
        .cloned()
        .collect();
    let moved_remove: Vec<String> = source
        .remove
        .iter()
        .filter(|id| is_skill(id))
        .cloned()
        .collect();
    if moved_add.is_empty() && moved_remove.is_empty() {
        return;
    }
    source.add.retain(|id| !is_skill(id));
    source.remove.retain(|id| !is_skill(id));
    let base = source.base;
    let target = skills.get_or_insert_with(|| Selection {
        base,
        add: Vec::new(),
        remove: Vec::new(),
    });
    // A replacement on the source axis replaces on the destination too; keeping `inherit` would
    // silently re-add whatever the skills axis used to receive from the layers above.
    if base == Base::None {
        target.base = Base::None;
    }
    for id in moved_add {
        if !target.add.contains(&id) {
            target.add.push(id);
        }
    }
    for id in moved_remove {
        if !target.remove.contains(&id) {
            target.remove.push(id);
        }
    }
}

impl Default for Board {
    fn default() -> Self {
        Board {
            actions: Default::default(),
            tools: Tools::default(),
            tool_trust: Vec::new(),
            stages: ["Preparando", "Fazendo", "Code review", "Travado", "Feito"]
                .map(String::from)
                .to_vec(),
            projects: Vec::new(),
            workspaces: Vec::new(),
        }
    }
}

impl Board {
    pub fn load() -> Board {
        let current = path();
        let mut board = Self::load_at(&current);
        board.revive();
        board
    }

    fn load_at(current: &std::path::Path) -> Board {
        let backup = current.with_extension("json.bak");
        let read = |candidate: &std::path::Path| -> Result<Board, String> {
            let text = std::fs::read_to_string(candidate).map_err(|error| error.to_string())?;
            serde_json::from_str(&text).map_err(|error| error.to_string())
        };
        match read(current) {
            Ok(board) => board,
            Err(error) => match read(&backup) {
                Ok(board) => {
                    if current.exists() {
                        eprintln!(
                            "board.json inválido ({error}); tentando backup {}",
                            backup.display()
                        );
                    }
                    board
                }
                Err(backup_error) => {
                    if current.exists() || backup.exists() {
                        eprintln!("board.json e backup inválidos: {error}; {backup_error}");
                    }
                    Board::default()
                }
            },
        }
    }

    /// Reconcile runtime state, migrate older fields and expose interrupted preparation. Separated
    /// from filesystem loading because parallel tests must not share environment overrides.
    pub(crate) fn revive(&mut self) {
        self.actions.initialize_defaults();
        // Only legacy workspaces with no project ID reconstruct the catalog. Explicit IDs preserve
        // project removal.
        let legacy_projects: Vec<Project> = self
            .workspaces
            .iter()
            .filter(|ws| ws.project.is_empty())
            .map(|ws| Project {
                id: ws.repo.clone(),
                name: ws.repo_name.clone(),
                path: ws.repo.clone(),
            })
            .collect();
        for ws in &mut self.workspaces {
            // Preparation cannot survive an app restart; expose the interrupted worktree as a
            // failure.
            if ws.preparing {
                ws.preparing = false;
                ws.failed = Some(crate::i18n::t("err.session.interrupted"));
            }
            // Before tabs existed, the card ID was the session ID. Preserve that session unless
            // preparation never completed.
            if ws.tabs.is_empty() && ws.failed.is_none() {
                ws.tabs.push(Tab {
                    task: None,
                    id: ws.id.clone(),
                    agent_session: None,
                    title: String::new(),
                    status: Status::Desligada,
                    note: None,
                    pending_prompt: None,
                    tokens: None,
                    context_tokens: None,
                    choice: None,
                });
            }
            // Processes do not survive app restarts. Remove generated placeholder titles so the UI
            // can display the model.
            for tab in &mut ws.tabs {
                tab.status = Status::Desligada;
                if is_placeholder_title(&tab.title) {
                    tab.title.clear();
                }
            }
            if ws.active.is_none() {
                ws.active = ws.tabs.first().map(|t| t.id.clone());
            }
            if ws.project.is_empty() {
                ws.project = ws.repo.clone();
            }
            // Legacy single-repository workspaces gain one entry without changing their existing
            // paths.
            if ws.repos.is_empty() {
                ws.repos.push(Repo {
                    path: ws.repo.clone(),
                    name: ws.repo_name.clone(),
                    worktree: ws.worktree.clone(),
                    base: String::new(),
                    pr: None,
                });
            }
            // Move the legacy workspace PR to its primary repository.
            if let Some(pr) = ws.pr.take() {
                if let Some(main) = ws.repos.first_mut() {
                    main.pr.get_or_insert(pr);
                }
            }
            // Standalone skills left the plugin axis for their own; migrate legacy selections.
            split_skills(&mut ws.plugins, &mut ws.skills);
        }

        // An unknown stage would hide the workspace from every group. Fall back to the first stage.
        let first = self.stages.first().cloned().unwrap_or_default();
        let stages = self.stages.clone();
        for ws in &mut self.workspaces {
            if !stages.contains(&ws.stage) {
                ws.stage = first.clone();
            }
        }

        // Boards predating the project catalog reconstruct it from legacy workspaces.
        for project in legacy_projects {
            if !self.projects.iter().any(|p| p.path == project.path) {
                self.projects.push(project);
            }
        }
    }

    /// Write beside the current file and rename atomically so a failed save cannot leave truncated
    /// JSON.
    pub fn save(&self) -> Result<(), String> {
        let path = path();
        let json = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        // Only back up a board that still deserializes. Replacing the last valid backup with
        // external corruption would remove the recovery path.
        if let Ok(previous) = std::fs::read_to_string(&path) {
            if serde_json::from_str::<Board>(&previous).is_ok() {
                paths::write_private(&path.with_extension("json.bak"), &previous)?;
            }
        }
        paths::write_private(&path, &json)
    }

    pub fn workspace_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    /// Find a tab by session ID, including callers that know only the provider's local session key.
    pub fn tab_mut(&mut self, session: &str) -> Option<&mut Tab> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .find(|t| t.id == session)
    }

    /// Find the owning workspace so session events update the correct card.
    pub fn workspace_of_mut(&mut self, session: &str) -> Option<&mut Workspace> {
        self.workspaces
            .iter_mut()
            .find(|w| w.tabs.iter().any(|t| t.id == session))
    }

    pub fn workspace_of(&self, session: &str) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.tabs.iter().any(|t| t.id == session))
    }
}

fn path() -> std::path::PathBuf {
    paths::root().join("board.json")
}

/* ---------- publication ---------- */

/// Publish after releasing the board mutation lock. Snapshot, persistence queue and UI emission
/// share one order; disk I/O never holds the board lock.
pub fn publish(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.save.publish(&state.board, |board| {
        if let Err(error) = app.emit("board", board) {
            eprintln!("não publiquei o quadro para a webview: {error}");
        }
    });
}

/// Flush the current board before shutdown or before dispatching a persisted task. The publication
/// lock keeps concurrent snapshots from overtaking this synchronous save.
pub fn save_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let _publication = lock(&state.save.publication);
    let board = Arc::new(lock(&state.board).clone());
    if let Err(error) = state.save.now(board.clone()) {
        eprintln!("o saver não confirmou board.json ao sair: {error}");
        if let Err(error) = board.save() {
            eprintln!("não gravei board.json ao sair: {error}");
        }
    }
}

/// Coalesce frequent agent updates and save the latest board once.
const COALESCE: Duration = Duration::from_millis(250);

/// Queued snapshots and synchronous flush requests share one persistence thread.
enum Save {
    Later(Arc<Board>),
    Now(Arc<Board>, Sender<Result<(), String>>),
}

/// A flush uses the same queue as delayed saves so an older pending snapshot cannot overwrite it.
/// The worker remains available for later runtime updates.
#[derive(Clone)]
pub struct Saver {
    tx: Sender<Save>,
    publication: Arc<Mutex<()>>,
}

impl Saver {
    /// Serialize snapshots, queueing and emission without holding the board during I/O.
    fn publish(&self, current: &Mutex<Board>, emit: impl FnOnce(&Board)) {
        let _publication = lock(&self.publication);
        let board = Arc::new(lock(current).clone());
        if self.later(board.clone()).is_err() {
            if let Err(error) = board.save() {
                eprintln!("não gravei board.json depois de perder o saver: {error}");
            }
        }
        emit(&board);
    }

    fn later(&self, board: Arc<Board>) -> Result<(), ()> {
        self.tx.send(Save::Later(board)).map_err(|_| ())
    }

    fn now(&self, board: Arc<Board>) -> Result<(), String> {
        let (tx, rx) = channel();
        self.tx
            .send(Save::Now(board, tx))
            .map_err(|_| "thread de persistência encerrada".to_string())?;
        // No timeout: a concurrent fallback write could finish before the original and restore
        // stale state. The worker always acknowledges the result, including disk failures.
        rx.recv().map_err(|error| error.to_string())?
    }
}

pub fn spawn_saver() -> Saver {
    spawn_saver_with(Board::save)
}

fn spawn_saver_with<F>(save: F) -> Saver
where
    F: Fn(&Board) -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = channel::<Save>();
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let (mut board, mut flushes) = match first {
                Save::Later(board) => {
                    std::thread::sleep(COALESCE);
                    (board, Vec::new())
                }
                Save::Now(board, done) => (board, vec![done]),
            };
            // Drain queued snapshots and acknowledge every flush with the same final write result.
            while let Ok(newer) = rx.try_recv() {
                match newer {
                    Save::Later(next) => board = next,
                    Save::Now(next, done) => {
                        board = next;
                        flushes.push(done);
                    }
                }
            }
            let result = save(board.as_ref());
            if let Err(error) = &result {
                eprintln!("não gravei board.json: {error}");
            }
            if !flushes.is_empty() {
                for done in flushes {
                    let _ = done.send(result.clone());
                }
            }
        }
    });
    Saver {
        tx,
        publication: Arc::new(Mutex::new(())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_publications_keep_snapshot_and_emission_order() {
        let current = Mutex::new(Board {
            stages: vec!["old".into()],
            ..Board::default()
        });
        let written = Arc::new(Mutex::new(Vec::new()));
        let observed = written.clone();
        let saver = spawn_saver_with(move |board| {
            lock(&observed).push(board.stages[0].clone());
            Ok(())
        });
        let emitted = Mutex::new(Vec::new());
        let (captured, captured_rx) = channel();
        let (release, release_rx) = channel();
        let (competing, competing_rx) = channel();
        std::thread::scope(|scope| {
            let (saver, current, emitted) = (&saver, &current, &emitted);
            scope.spawn(move || {
                saver.publish(current, |board| {
                    captured.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    lock(emitted).push(board.stages[0].clone());
                });
            });
            captured_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            lock(current).stages[0] = "new".into();
            scope.spawn(move || {
                assert!(saver.publication.try_lock().is_err());
                competing.send(()).unwrap();
                saver.publish(current, |board| {
                    lock(emitted).push(board.stages[0].clone());
                });
            });
            competing_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            release.send(()).unwrap();
        });
        saver.now(Arc::new(lock(&current).clone())).unwrap();
        assert_eq!(*lock(&emitted), ["old", "new"]);
        assert_eq!(lock(&written).last().map(String::as_str), Some("new"));
    }

    #[test]
    fn flush_keeps_saver_available_for_runtime_publications() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let observed = written.clone();
        let saver = spawn_saver_with(move |board| {
            lock(&observed).push(board.stages[0].clone());
            Ok(())
        });
        for stage in ["first", "second"] {
            saver
                .now(Arc::new(Board {
                    stages: vec![stage.into()],
                    ..Board::default()
                }))
                .unwrap();
        }
        assert_eq!(*lock(&written), ["first", "second"]);
    }

    /// The smallest legacy workspace JSON; newly added fields must keep `serde(default)`
    /// compatibility.
    fn board_json(extra: &str) -> Board {
        let json = format!(
            r#"{{"stages":["Fazendo"],"projects":[],"workspaces":[
                 {{"id":"w","title":"t","repo":"/r","repo_name":"r",
                   "branch":"b","worktree":"/wt","stage":"Fazendo"{extra}}}]}}"#
        );
        serde_json::from_str(&json).expect("board não desserializou")
    }

    fn temporary_board_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("prometeu-board-{}", uuid::Uuid::new_v4()))
            .join("board.json")
    }

    #[test]
    fn flush_descarta_o_quadro_antigo_que_esperava_no_coalesce() {
        let written = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = written.clone();
        let saver = spawn_saver_with(move |board| {
            observed.lock().unwrap().push(board.stages[0].clone());
            Ok(())
        });
        let old = Board {
            stages: vec!["antigo".into()],
            ..Board::default()
        };
        let final_board = Board {
            stages: vec!["final".into()],
            ..Board::default()
        };

        saver.later(Arc::new(old)).unwrap();
        saver.now(Arc::new(final_board)).unwrap();

        assert_eq!(*written.lock().unwrap(), vec!["final"]);
    }

    #[test]
    fn quadro_corrompido_recupera_o_ultimo_backup_valido() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, "{cortado").unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = Board::load_at(&current);

        assert_eq!(recovered.stages, vec!["Fazendo"]);
        assert_eq!(recovered.workspaces.len(), 1);
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn backup_tambem_recupera_quadro_principal_ausente() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = Board::load_at(&current);

        assert_eq!(recovered.stages, vec!["Fazendo"]);
        assert_eq!(recovered.workspaces[0].id, "w");
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn provider_legado_e_normalizado_sem_quebrar_o_board() {
        let vazio = board_json(r#","agent":"""#);
        let explicito = board_json(r#","agent":"codex""#);
        let futuro = board_json(r#","agent":"provider-ainda-desconhecido""#);

        assert_eq!(vazio.workspaces[0].agent, ProviderId::Claude);
        assert_eq!(explicito.workspaces[0].agent, ProviderId::Codex);
        assert_eq!(futuro.workspaces[0].agent, ProviderId::Claude);

        let normalized = serde_json::to_value(vazio).unwrap();
        assert_eq!(normalized["workspaces"][0]["agent"], "claude");
    }

    /// Interrupted preparation becomes an explicit failure after restart.
    #[test]
    fn montagem_interrompida_vira_erro_escrito_no_card() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        let ws = &board.workspaces[0];
        assert!(!ws.preparing, "não pode voltar montando");
        assert_eq!(
            ws.failed.as_deref(),
            Some(crate::i18n::t("err.session.interrupted")).as_deref()
        );
    }

    /// Do not invent a resumable tab for a workspace whose preparation never completed.
    #[test]
    fn montagem_interrompida_nao_ganha_aba() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        assert!(board.workspaces[0].tabs.is_empty());
        assert!(board.workspaces[0].active.is_none());
    }

    /// Existing legacy sessions still receive their migrated tab.
    #[test]
    fn quadro_antigo_sem_aba_continua_ganhando_a_sua() {
        let mut board = board_json("");
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].id, "w");
        assert_eq!(ws.active.as_deref(), Some("w"));
        assert!(ws.failed.is_none());
        assert!(!ws.remote_control);
    }

    /// Remove generated placeholder titles while preserving user-authored titles, even with the
    /// same prefix.
    #[test]
    fn nome_inventado_de_aba_some_na_migracao() {
        let mut board = board_json(
            r#","tabs":[
              {"id":"a","title":"conversa","status":"pronta","note":null,"pending_prompt":null},
              {"id":"b","title":"conversa 2","status":"pronta","note":null,"pending_prompt":null},
              {"id":"c","title":"conversa sobre o login","status":"pronta","note":null,"pending_prompt":null},
              {"id":"d","title":"Corrigir o menu","status":"pronta","note":null,"pending_prompt":null}]"#,
        );
        board.revive();
        let titles: Vec<&str> = board.workspaces[0]
            .tabs
            .iter()
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["", "", "conversa sobre o login", "Corrigir o menu"]
        );
    }

    #[test]
    fn revive_distingue_projeto_legado_de_removido() {
        let mut legacy = board_json("");
        legacy.revive();
        assert_eq!(legacy.projects[0].id, "/r");

        let mut removed = board_json(r#", "project":"/r""#);
        removed.revive();
        assert!(removed.projects.is_empty());
        assert_eq!(removed.workspaces[0].project, "/r");
    }

    /// Legacy single-repository boards keep their original paths while gaining the repository list.
    #[test]
    fn quadro_antigo_ganha_a_lista_de_um_repositorio() {
        let mut board = board_json("");
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.repo, "/r");
        assert_eq!(ws.worktree, "/wt");
        assert_eq!(
            ws.repos,
            vec![Repo {
                path: "/r".into(),
                name: "r".into(),
                worktree: "/wt".into(),
                base: String::new(),
                pr: None
            }]
        );
        assert_eq!(ws.primary().worktree, "/wt");
        assert!(!ws.multi());
    }

    /// Move the legacy PR into the primary repository and stop serializing its old location.
    #[test]
    fn quadro_antigo_leva_o_pr_para_o_principal() {
        let mut board = board_json(r#","pr":{"number":3,"title":"t","state":"MERGED"}"#);
        board.revive();
        let ws = &board.workspaces[0];
        assert!(ws.pr.is_none());
        assert_eq!(ws.repos[0].pr.as_ref().map(|p| p.number), Some(3));
        assert!(ws.merged());
        let json = serde_json::to_string(&board).unwrap();
        assert!(!json.contains(r#""pr":{"number":3"#) || json.contains(r#""repos":[{"#));
        assert!(!json.contains(r#""stage":"Fazendo","pr""#));
    }

    /// Existing repository lists retain their entries without duplication.
    #[test]
    fn quadro_com_lista_fica_como_esta() {
        let mut board = board_json(
            r#","repos":[{"path":"/r","name":"r","worktree":"/wt/r"},{"path":"/s","name":"s","worktree":"/wt/s"}]"#,
        );
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.repos.len(), 2);
        assert!(ws.multi());
        assert_eq!(ws.primary().name, "r");
        assert_eq!(ws.repos[1].worktree, "/wt/s");
    }

    /// Tabs saved as running reopen stopped because their processes did not survive.
    #[test]
    fn aba_gravada_viva_volta_desligada() {
        let mut board = board_json(
            r#","tabs":[{"id":"t1","title":"conversa","status":"rodando","note":null,"pending_prompt":null}]"#,
        );
        board.revive();
        assert!(matches!(
            board.workspaces[0].tabs[0].status,
            Status::Desligada
        ));
    }

    #[test]
    fn tokens_somam_o_contexto_depois_de_compactar() {
        let mut board = board_json(
            r#","tabs":[{"id":"t1","title":"conversa","status":"pronta","note":null,"pending_prompt":null}]"#,
        );
        let tab = &mut board.workspaces[0].tabs[0];

        for current in [10_000, 20_000, 4_000, 12_000] {
            tab.observe_tokens(current);
        }

        assert_eq!(tab.tokens, Some(32_000));
        assert_eq!(tab.context_tokens, Some(12_000));
    }

    #[test]
    fn tokens_antigos_viram_inicio_do_contador_sem_duplicar() {
        let mut board = board_json(
            r#","tabs":[{"id":"t1","title":"conversa","status":"pronta","note":null,"pending_prompt":null,"tokens":20000}]"#,
        );
        let tab = &mut board.workspaces[0].tabs[0];

        tab.observe_tokens(24_000);
        assert_eq!(tab.tokens, Some(24_000));
        tab.observe_tokens(3_000);
        assert_eq!(tab.tokens, Some(27_000));
    }

    /// Legacy tool axes (`Option<Vec<String>>`) migrate to the layered `Selection` form on load.
    #[test]
    fn selecao_legada_de_ferramentas_vira_objeto() {
        use crate::selection::{Base, Selection};
        let legacy = board_json(r#","mcp":["a","b"],"plugins":[]"#);
        assert_eq!(
            legacy.workspaces[0].mcp,
            Some(Selection::only(vec!["a".into(), "b".into()]))
        );
        assert_eq!(legacy.workspaces[0].plugins, Some(Selection::only(vec![])));
        assert_eq!(legacy.workspaces[0].skills, None);

        // An absent axis inherits the layers above it.
        assert_eq!(board_json("").workspaces[0].mcp, None);

        // The new object form round-trips, including the `inherit` base.
        let novo = board_json(r#","mcp":{"base":"inherit","add":["x"],"remove":["y"]}"#);
        assert_eq!(
            novo.workspaces[0].mcp,
            Some(Selection {
                base: Base::Inherit,
                add: vec!["x".into()],
                remove: vec!["y".into()],
            })
        );
    }

    /// Standalone skills move from the plugin axis to their own on load, and stay moved.
    #[test]
    fn skills_saem_dos_plugins_na_migracao() {
        use crate::selection::Selection;
        let mut board = board_json(r#","plugins":["revisor","skill-review"]"#);
        board.revive();
        assert_eq!(
            board.workspaces[0].plugins,
            Some(Selection::only(vec!["revisor".into()]))
        );
        assert_eq!(
            board.workspaces[0].skills,
            Some(Selection::only(vec!["skill-review".into()]))
        );

        // The migration is idempotent: a second load changes nothing.
        board.revive();
        assert_eq!(
            board.workspaces[0].skills,
            Some(Selection::only(vec!["skill-review".into()]))
        );
    }

    /// A replacement on the plugins axis propagates to an existing skills axis, so moved skill
    /// ids do not degrade the replacement into inherit-plus-add.
    #[test]
    fn migracao_de_skills_preserva_substituicao_no_destino_existente() {
        use crate::selection::{Base, Selection};
        let mut plugins = Some(Selection::only(vec!["skill-a".into()]));
        let mut skills = Some(Selection {
            base: Base::Inherit,
            add: vec!["skill-b".into()],
            remove: vec![],
        });
        split_skills(&mut plugins, &mut skills);
        assert_eq!(plugins, Some(Selection::only(vec![])));
        let skills = skills.expect("eixo");
        assert_eq!(skills.base, Base::None);
        assert_eq!(skills.add, ["skill-b".to_string(), "skill-a".to_string()]);
    }

    /// The global tool layer is absent by default so an old board injects exactly what it used to.
    #[test]
    fn board_antigo_nao_tem_camada_global() {
        let board = board_json("");
        assert_eq!(board.tools, Tools::default());
        assert!(board.tools.mcp.is_none());
        assert!(board.tools.plugins.is_none());
        assert!(board.tools.skills.is_none());
        // An old board has approved no project declaration, so its `[tools]` stays gated.
        assert!(board.tool_trust.is_empty());
    }
}
