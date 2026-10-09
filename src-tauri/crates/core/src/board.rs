//! Projects register repositories. Workspaces group branches and worktrees. Tabs hold agent
//! sessions that share those files. A conversation can close without deleting its worktree; another
//! tab can continue using the same files.

use crate::selection::{Base, Selection, Tools};
use serde::{Deserialize, Serialize};

/// Persisted agent runtime identity. Legacy empty strings deserialize as Claude and save as
/// `"claude"`. Unknown values use the default so newer boards remain readable by older app
/// versions.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    #[default]
    Claude,
    Codex,
    Antigravity,
    #[serde(rename = "gemini")]
    RetiredGemini,
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "codex" => Self::Codex,
            "antigravity" => Self::Antigravity,
            "gemini" => Self::RetiredGemini,
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
    /// Restart-only runtime modes must survive process loss until the person approves execution.
    #[serde(default)]
    pub plan: bool,
    #[serde(default)]
    pub permission: Option<crate::actions::Permission>,
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
    /// Last provider-reported capacity; absent in older boards and unsupported providers.
    #[serde(default)]
    pub context_window: Option<u64>,
    /// An optional model override selected when opening or retuning a tab. `None` inherits the
    /// workspace; selecting its model again removes the override. Resume preserves the tab's
    /// choice.
    #[serde(default)]
    pub choice: Option<Choice>,
    /// The "Start with" skill this conversation began from, as `<package>/<skill>`. Resuming adds
    /// its package to the resolved set again without touching any selection layer; absent in older
    /// boards and in every other conversation (ADR 0057).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kickoff: Option<String>,
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

/// Person IDs allowed to act on a shared workspace's agent. The frontend enforces them for remote input.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShareRights {
    /// Send messages: prompts and interrupts.
    #[serde(default)]
    pub send: Vec<String>,
    /// Control: answers to the agent's approvals and questions.
    #[serde(default)]
    pub control: Vec<String>,
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
    pub issue: Option<crate::domain::IssueRef>,
    /// Legacy workspace-level PR, moved to the primary repository by `revive` and never saved here
    /// again.
    #[serde(default, skip_serializing)]
    pub pr: Option<crate::domain::Pr>,
    /// The worktree and local branch have been removed. History remains, but terminals cannot
    /// reopen here.
    #[serde(default)]
    pub cleaned: bool,
    /// Clone paths whose branches must survive cleanup, including selected remote branches.
    #[serde(default)]
    pub preserve_branches: Vec<String>,
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
    /// People the owner lets act beyond viewing and commenting (ADR 0090). `None` on shares made before rights
    /// existed, which grant viewing and commenting only.
    #[serde(default)]
    pub rights: Option<ShareRights>,
    /// Teammates' messages wait in the owner's composer until the owner sends, edits or discards them (ADR 0090).
    /// On by default, including boards saved before the option existed.
    #[serde(default = "confirm_by_default")]
    pub confirm_messages: bool,
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

fn confirm_by_default() -> bool {
    true
}

/// Read one workspace tool axis from either the layered `Selection` object or the legacy
/// `Option<Vec<String>>` form, so a pre-migration board still loads: `null` and an absent field
/// inherit, `[]` and `[ids]` become a replacement, and an object is the layered form. The migration
/// is one-way; see docs/contracts/persistence.md and ADR 0045.
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

/// One project-trust decision (ADR 0045). A repository's versioned `[tools]` declaration activates
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
    /// Opaque telemetry identities for legacy path-based projects and repository branches.
    #[serde(default)]
    pub telemetry_ids: crate::identities::Identities,
    /// Delegation ownership belongs to conversations, not processes or workspace membership.
    #[serde(default)]
    pub delegations: Vec<crate::delegation::Delegation>,
    #[serde(default)]
    pub actions: crate::actions::Catalog,
    /// The global layer of the tool selection, one `Selection` per axis, app-local under the root.
    /// Absent by default so an old board keeps injecting exactly what it used to. See
    /// docs/contracts/persistence.md and ADR 0045.
    #[serde(default)]
    pub tools: Tools,
    /// Per-repository decisions that let a versioned project `[tools]` declaration activate. Empty
    /// by default, so an old board has approved nothing and project-declared items stay uninjected
    /// until the person decides. See docs/contracts/persistence.md and ADR 0045.
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
/// (ADR 0045). Legacy boards stored them inside `plugins`; move them to `skills`, preserving the
/// source layer's base mode. Idempotent, so it is safe on every load: a migrated board has no
/// `skill-<id>` left in `plugins`.
pub fn split_skills(plugins: &mut Option<Selection>, skills: &mut Option<Selection>) {
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
            telemetry_ids: Default::default(),
            delegations: Vec::new(),
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
    /// Reconcile runtime state, migrate older fields and expose interrupted preparation. Separated
    /// from filesystem loading because parallel tests must not share environment overrides.
    pub fn revive(&mut self) {
        self.actions.initialize_defaults();
        for delegation in &mut self.delegations {
            let pending = self
                .workspaces
                .iter()
                .find(|w| w.id == delegation.workspace)
                .and_then(|w| w.tabs.iter().find(|t| t.id == delegation.id))
                .is_some_and(|t| t.pending_prompt.is_some());
            delegation.reconcile_restart(pending);
        }
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
                ws.failed = Some(crate::error::code("err.session.interrupted"));
            }
            // Before tabs existed, the card ID was the session ID. Preserve that session unless
            // preparation never completed.
            if ws.tabs.is_empty() && ws.failed.is_none() {
                ws.tabs.push(Tab {
                    plan: false,
                    permission: None,
                    task: None,
                    id: ws.id.clone(),
                    agent_session: None,
                    title: String::new(),
                    status: Status::Desligada,
                    note: None,
                    pending_prompt: None,
                    tokens: None,
                    context_tokens: None,
                    context_window: None,
                    choice: None,
                    kickoff: None,
                });
            }
            // Processes do not survive app restarts. Remove generated placeholder titles so the UI
            // can display the model.
            for tab in &mut ws.tabs {
                tab.status = Status::Desligada;
                // Tasks from versions with the removed PR monitor carry a start time and may wait
                // for PR updates that no longer arrive. Settle them once so the command can start
                // again; their transcript and any queued message stay visible in the tab.
                if let Some(run) = tab.task.as_mut().filter(|run| run.checked_at > 0) {
                    run.done = true;
                    run.paused = false;
                    run.turns = 0;
                    run.checked_at = 0;
                }
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

impl Workspace {
    /// The workspace contributes one explicit layer without resolving its parents or provider.
    pub fn tools(&self) -> Tools {
        Tools {
            mcp: self.mcp.clone(),
            plugins: self.plugins.clone(),
            skills: self.skills.clone(),
        }
    }

    /// Choosing workspace defaults restores inheritance. Provider changes cannot resume the same
    /// transcript, and task profiles remain frozen for the execution.
    pub fn retune(&mut self, tab: &str, choice: Choice) -> Result<(), String> {
        if self.tabs.iter().any(|t| t.id == tab && t.task.is_some()) {
            return Err(crate::error::code("err.actions.frozen"));
        }
        // The tab is ordinary, so its provider is the model override or the workspace default.
        let current = self
            .tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.choice.clone())
            .map_or(self.agent, |c| c.agent);
        if current != choice.agent {
            return Err(crate::error::code("err.session.otherAgent"));
        }
        let follows = choice.agent == self.agent
            && choice.model == self.model
            && choice.effort == self.effort;
        let tab = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab)
            .ok_or_else(|| crate::error::code("err.session.noTab"))?;
        tab.choice = (!follows).then_some(choice);
        Ok(())
    }
}

impl Board {
    pub fn prepare_telemetry_ids(&mut self) {
        for p in &self.projects {
            self.telemetry_ids.ensure("project", &p.id);
            self.telemetry_ids.ensure("repository", &p.path);
        }
        for w in &self.workspaces {
            self.telemetry_ids.ensure("workspace", &w.id);
            self.telemetry_ids.ensure("project", &w.project);
            for t in &w.tabs {
                self.telemetry_ids.ensure("conversation", &t.id);
            }
            for r in &w.repos {
                self.telemetry_ids.ensure("repository", &r.path);
                self.telemetry_ids
                    .ensure("branch", &format!("{}\0{}", r.path, w.branch));
                if let Some(pr) = &r.pr {
                    if !pr.head_ref_name.is_empty() {
                        self.telemetry_ids
                            .ensure("branch", &format!("{}\0{}", r.path, pr.head_ref_name));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn removed_gemini_identity_never_becomes_another_runtime() {
        let provider: super::ProviderId = serde_json::from_str("\"gemini\"").unwrap();
        assert_eq!(provider, super::ProviderId::RetiredGemini);
        assert_eq!(serde_json::to_string(&provider).unwrap(), "\"gemini\"");
    }

    use super::*;

    /// The smallest legacy workspace JSON; newly added fields must keep `serde(default)`
    /// compatibility.
    fn board_json(extra: &str) -> Board {
        let json = format!(
            r#"{{"stages":["Fazendo"],"projects":[],"workspaces":[
                 {{"id":"w","title":"t","repo":"/r","repo_name":"r",
                   "branch":"b","worktree":"/wt","stage":"Fazendo"{extra}}}]}}"#
        );
        serde_json::from_str(&json).expect("board did not deserialize")
    }

    #[test]
    fn normalizes_legacy_provider_without_breaking_the_board() {
        let empty = board_json(r#","agent":"""#);
        let explicit = board_json(r#","agent":"codex""#);
        let future = board_json(r#","agent":"provider-ainda-desconhecido""#);

        assert_eq!(empty.workspaces[0].agent, ProviderId::Claude);
        assert_eq!(explicit.workspaces[0].agent, ProviderId::Codex);
        assert_eq!(future.workspaces[0].agent, ProviderId::Claude);

        let normalized = serde_json::to_value(empty).unwrap();
        assert_eq!(normalized["workspaces"][0]["agent"], "claude");
    }

    /// Interrupted preparation becomes an explicit failure after restart.
    #[test]
    fn interrupted_setup_becomes_an_error_on_the_card() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        let ws = &board.workspaces[0];
        assert!(!ws.preparing, "must not restore while preparing");
        assert_eq!(
            ws.failed.as_deref(),
            Some(crate::error::code("err.session.interrupted")).as_deref()
        );
    }

    /// Do not invent a resumable tab for a workspace whose preparation never completed.
    #[test]
    fn interrupted_setup_does_not_create_a_tab() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        assert!(board.workspaces[0].tabs.is_empty());
        assert!(board.workspaces[0].active.is_none());
    }

    /// Existing legacy sessions still receive their migrated tab.
    #[test]
    fn legacy_boards_without_tabs_still_receive_one() {
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
    fn invented_tab_names_are_removed_during_migration() {
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
    fn revive_settles_tasks_waiting_on_the_removed_pr_monitor() {
        let profile = r#"{"id":"p","name":"P","prompt":"x","choice":{"agent":"claude","model":"","effort":""},"mcp":null,"plugins":null,"skills":[],"permission":"ask","watch":{"interval_seconds":60,"comments":true,"ci":true,"max_turns":10}}"#;
        let task = |checked: u64| {
            format!(
                r#"{{"command":"entregar","profile":{profile},"paused":false,"done":false,"turns":2,"checked_at":{checked},"error":null,"seen":{{}},"prs":{{"repo":1}}}}"#
            )
        };
        let mut board: Board = serde_json::from_str(&format!(
            r#"{{"stages":[],"workspaces":[{{"id":"w","title":"","repo":"/r","repo_name":"r","branch":"b","worktree":"/r","stage":"s","tabs":[
              {{"id":"legacy","title":"","status":"pronta","note":null,"pending_prompt":"PR updates","task":{}}},
              {{"id":"current","title":"","status":"pronta","note":null,"pending_prompt":null,"task":{}}}]}}]}}"#,
            task(1_700_000_000),
            task(0)
        ))
        .unwrap();
        board.revive();
        let tabs = &board.workspaces[0].tabs;
        let legacy = tabs[0].task.as_ref().unwrap();
        assert!(legacy.done && legacy.checked_at == 0 && legacy.turns == 0);
        assert_eq!(tabs[0].pending_prompt.as_deref(), Some("PR updates"));
        assert!(!tabs[1].task.as_ref().unwrap().done);
    }

    #[test]
    fn revive_distinguishes_legacy_projects_from_removed_projects() {
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
    fn legacy_boards_receive_a_single_repository_list() {
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
    fn legacy_boards_move_the_pull_request_to_the_primary_repository() {
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
    fn boards_with_repository_lists_remain_unchanged() {
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
    fn persisted_live_tabs_restore_as_stopped() {
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
    fn tokens_sum_context_after_compaction() {
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
    fn legacy_tokens_seed_the_counter_without_double_counting() {
        let mut board = board_json(
            r#","tabs":[{"id":"t1","title":"conversa","status":"pronta","note":null,"pending_prompt":null,"tokens":20000}]"#,
        );
        let tab = &mut board.workspaces[0].tabs[0];

        assert_eq!(tab.context_window, None);
        tab.observe_tokens(24_000);
        assert_eq!(tab.tokens, Some(24_000));
        tab.observe_tokens(3_000);
        assert_eq!(tab.tokens, Some(27_000));
    }

    #[test]
    fn context_capacity_round_trips_separately_from_accumulated_tokens() {
        let board = board_json(
            r#","tabs":[{"id":"t1","title":"Conversation","status":"pronta","note":null,"pending_prompt":null,"tokens":30000,"context_tokens":5000,"context_window":200000}]"#,
        );
        let encoded = serde_json::to_value(&board.workspaces[0].tabs[0]).unwrap();
        assert_eq!(encoded["context_window"], 200000);
        assert_eq!(encoded["context_tokens"], 5000);
        assert_eq!(encoded["tokens"], 30000);
    }

    /// Legacy tool axes (`Option<Vec<String>>`) migrate to the layered `Selection` form on load.
    #[test]
    fn legacy_tool_selection_becomes_an_object() {
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
        let current = board_json(r#","mcp":{"base":"inherit","add":["x"],"remove":["y"]}"#);
        assert_eq!(
            current.workspaces[0].mcp,
            Some(Selection {
                base: Base::Inherit,
                add: vec!["x".into()],
                remove: vec!["y".into()],
            })
        );
    }

    /// Standalone skills move from the plugin axis to their own on load, and stay moved.
    #[test]
    fn migration_separates_skills_from_plugins() {
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
    fn skill_migration_preserves_replacement_in_existing_destination() {
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
    fn legacy_boards_have_no_global_layer() {
        let board = board_json("");
        assert_eq!(board.tools, Tools::default());
        assert!(board.tools.mcp.is_none());
        assert!(board.tools.plugins.is_none());
        assert!(board.tools.skills.is_none());
        // An old board has approved no project declaration, so its `[tools]` stays gated.
        assert!(board.tool_trust.is_empty());
    }

    /// Tabs saved before ADR 0057 have no kickoff; a kickoff tab keeps its skill across a save, and
    /// ordinary tabs keep writing the previous shape.
    #[test]
    fn tabs_without_a_kickoff_keep_the_previous_format() {
        let board = board_json(
            r#","tabs":[
              {"id":"old","title":"","status":"pronta","note":null,"pending_prompt":null},
              {"id":"new","title":"","status":"pronta","note":null,"pending_prompt":null,
               "kickoff":"sdd-kit/specify"}]"#,
        );
        let tabs = &board.workspaces[0].tabs;
        assert_eq!(tabs[0].kickoff, None);
        assert_eq!(tabs[1].kickoff.as_deref(), Some("sdd-kit/specify"));
        let old = serde_json::to_value(&tabs[0]).unwrap();
        assert!(old.get("kickoff").is_none());
        let new = serde_json::to_value(&tabs[1]).unwrap();
        assert_eq!(new["kickoff"], "sdd-kit/specify");
    }
}
