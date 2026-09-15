import { modelLabelOf } from "./agents";
import { current as locale, t } from "./i18n";

/// Import team-member shapes from the relay protocol, the single source of network types.
export type { Member } from "../relay/src/protocol";

export type Status = "rodando" | "querendo" | "pronta" | "desligada";

/// Carry stable provider identity explicitly alongside the model; only the catalog resolves their association.
export type ProviderId = "claude" | "codex";

/// Share localized status labels across views while preserving protocol/CSS status values.
export const label = (status: Status) => t(`status.${status}`);

/// Share workspace labels too; preparation is workspace activity before any tab exists.
export const stateLabel = (ws: Workspace) =>
  pending(ws) ? t(ws.failed ? "card.failed" : "card.building") : label(statusOf(ws));

export type Tab = {
  task?: import("./actions").TaskRun | null;
  id: string;
  /// An unnamed tab displays its model through tabLabel.
  title: string;
  status: Status;
  note: string | null;
  /// Incremental token estimate for this conversation.
  tokens: number | null;
  /// Last observed context and the backend's persisted counting cursor.
  context_tokens?: number | null;
  /// A prompt queued until worktree setup finishes.
  pending_prompt?: string | null;
  /// An optional tab-specific model choice; absent choices inherit workspace defaults.
  choice?: Choice | null;
};

/// Provider, model, and effort form one choice because each model belongs to a provider and has supported effort levels.
export type Choice = { agent: ProviderId; model: string; effort: string };

export type Project = { id: string; name: string; path: string };

/// A repository's source clone, workspace path, base branch, and PR. Each repository retains independent Git history.
export type Repo = { path: string; name: string; worktree: string; base: string; pr: Pr | null };

/// Local Git contract; repository names and paths remain user data.
export type GitFile = { path: string; status: string };
export type GitStatus = {
  repo: number;
  name: string;
  branch: string | null;
  base: string;
  upstream: string | null;
  remotes: string[];
  ahead: number;
  behind: number;
  has_head: boolean;
  merging: boolean;
  index: string;
  staged: GitFile[];
  changes: GitFile[];
  conflicts: GitFile[];
  error: string | null;
};
export type GitScope = "staged" | "changes" | "compare" | "commit";
export type GitDiff = { base: string; head: string; files: Change[] };
export type GitCommit = { oid: string; subject: string; author: string; date: string; outgoing: boolean };
export type GitBranch = { name: string; current: boolean; remote: boolean; worktree: string | null; workspace: string | null };
export type GitConflict = { current: string; ours: string | null; theirs: string | null };
export type GitAction = "stage" | "unstage" | "commit" | "fetch" | "pull" | "push" | "publish";

/// Name the primary repository or every repository when a workspace spans several.
export const repoLabel = (ws: Workspace) =>
  ws.repos.length > 1 ? ws.repos.map((r) => r.name).join(" + ") : ws.repo_name;

/// An MCP registry entry retains its complete provider configuration object.
export type McpServer = {
  id: string;
  config: Record<string, unknown>;
  /// Source or purpose. Empty imported descriptions identify the user's existing registration in the UI.
  note: string;
};

/// Probe MCP directly through JSON-RPC without starting an agent, so results describe server configuration alone.
export type McpProbe = {
  ok: boolean;
  /// HTTP 401 means configuration is reachable but authentication is required.
  auth: boolean;
  tools: number;
  /// Server-reported name.
  name: string;
  /// Untranslated server or system failure detail.
  detail: string;
};

/// An ordered probe step: key is translated, note carries result data, and detail preserves the original failure.
export type McpStep = {
  key: string;
  ok: boolean;
  note: string;
  detail: string;
};

/// Full probe progress and summary.
export type McpCheck = {
  steps: McpStep[];
  probe: McpProbe;
};

/// A portable plugin entry names its source and description. Backend adapters materialize it for each provider. App-owned made entries may be deleted from disk; from identifies the repository used for updates.
export type Plugin = {
  id: string;
  source: string;
  note: string;
  made?: boolean;
  from?: string;
};

/// One axis of one tool-selection layer. `null` inherits the layers above; `base: "none"` replaces
/// the inherited set with `add`; `base: "inherit"` applies `add`/`remove` over it. See ADR 0043.
export type Selection = { base: "none" | "inherit"; add: string[]; remove: string[] };

/// The three independent axes of a tool-selection layer, shared by the board's global layer and the
/// workspace triple. The project layer lives in the repository's `[tools]` table, not on the board.
export type Tools = { mcp: Selection | null; plugins: Selection | null; skills: Selection | null };

/// One project-trust decision (ADR 0043): a repository's versioned `[tools]` activates only after the
/// person approves its current hash. App-local on the board, never written into the repository, so a
/// changed declaration re-prompts. `repo` is the origin URL when present, else the clone's path.
export type ToolTrust = { repo: string; hash: string; approved: boolean; at: number };

/// The project `[tools]` declaration and its trust state, returned by `project_tools`. An empty
/// `hash` means the primary repository declares nothing, so the layer inherits and needs no approval.
export type ProjectTools = {
  repo: string;
  file: string | null;
  hash: string;
  tools: Tools;
  /// True when a declaration exists whose current hash has no decision yet (approved or rejected),
  /// so the interface prompts.
  pending: boolean;
  decision: ToolTrust | null;
};

/// Where an effective item came from (ADR 0043). `inherited` flows down from a layer above,
/// `added`/`removed` come from this layer's deltas, `pending` is a project-declared item held
/// back until the person decides on its hash, `rejected` is a project-declared item the person
/// refused (it stays visible but is never injected), and `cli` is an active MCP server the
/// person's CLI configuration loads by itself — the visible inherited base of the mcp axis
/// (ADR 0044).
export type Provenance = "inherited" | "added" | "removed" | "pending" | "rejected" | "cli";

/// One ID of the axis universe in a resolved axis, tagged with its origin so the picker can
/// explain each row.
export type EffectiveItem = { id: string; provenance: Provenance };

/// The resolved effective set for a workspace, one list per axis. `removed` items stay in the list so
/// the picker can show what this layer turned off; `pending` and `rejected` items show project
/// declarations awaiting or refused by the trust decision.
export type WorkspaceTools = {
  mcp: EffectiveItem[];
  plugins: EffectiveItem[];
  skills: EffectiveItem[];
};

/// Toggle one hub id in a layer while preserving what the layers above contribute (ADR 0043). Turning
/// an item on records an `add` and drops any `remove`; turning it off records a `remove` and drops any
/// `add`. The layer's `base` is preserved, defaulting to `inherit` so a first pick never erases the
/// set flowing down from the global and project layers.
export function toggleSelection(current: Selection | null, id: string, on: boolean): Selection {
  const base = current?.base ?? "inherit";
  const add = (current?.add ?? []).filter((x) => x !== id);
  const remove = (current?.remove ?? []).filter((x) => x !== id);
  if (on) add.push(id);
  else remove.push(id);
  return { base, add, remove };
}

export type Workspace = {
  id: string;
  title: string;
  project: string;
  /// The primary repository, first in repos.
  repo: string;
  repo_name: string;
  branch: string;
  /// Agent working directory: one worktree or a directory containing multiple repository worktrees.
  worktree: string;
  /// Workspace repositories, primary first.
  repos: Repo[];
  /// User-selected work stage, independent of observed agent Status.
  stage: string;
  archived: boolean;
  pinned: boolean;
  unread: boolean;
  /// Workspace provider. Rust normalizes empty legacy values to claude when loading.
  agent: ProviderId;
  /// Workspace model/effort defaults for new or resumed tabs; empty values use CLI defaults.
  model: string;
  effort: string;
  /// MCP selection from the hub, as the workspace layer. Null inherits the layers above and the CLI;
  /// a replacement with an empty `add` selects no MCP servers. See ADR 0043.
  mcp: Selection | null;
  /// Plugin selection from the hub, as the workspace layer, following the MCP inheritance rules.
  plugins: Selection | null;
  /// Standalone-skill selection from the hub, as its own axis. Skills still materialize through the
  /// plugin pipeline. Absent, therefore inherited, on boards saved before the axis existed.
  skills: Selection | null;
  /// Base of the ten ports reserved for this worktree.
  port: number | null;
  /// The originating Linear issue, when present.
  issue: IssueRef | null;
  /// A cleaned worktree retains transcript history but has no terminal, dock, or files.
  cleaned: boolean;
  /// team.ts announces shared workspaces and forwards their output.
  shared: boolean;
  /** Sharing consent belongs to this organization and member; absent on legacy boards. */
  share_team?: string | null;
  /// Audience member IDs, or null for the whole team; enforced by the relay only while shared.
  audience: string[] | null;
  /// Allow companion devices belonging to the owner to view and control this workspace.
  remote_control: boolean;
  /// Publish the workspace card before its worktree finishes preparing. No tabs exist during preparation.
  preparing: boolean;
  /// Structured backend preparation error, translated by fromBack.
  failed: string | null;
  /// Frontend-only remote workspace state; Rust never receives these entries. An offline owner freezes the view and disables input.
  remote: Remote | null;
  tabs: Tab[];
  active: string | null;
};

export type Remote = { owner: string; online: boolean };

/// GitHub CLI PR data; state is OPEN, MERGED, or CLOSED.
export type Pr = { number: number; title: string; isDraft: boolean; state: string };

/// PRs in workspace repository order.
export const prs = (ws: Workspace) => ws.repos.flatMap((r) => (r.pr ? [{ repo: r.name, pr: r.pr }] : []));

/// A workspace is merged when at least one repository has a PR and all such PRs are merged.
export const merged = (ws: Workspace) => {
  const all = prs(ws);
  return all.length > 0 && all.every(({ pr }) => pr.state === "MERGED");
};

/// Only archived, uncleaned workspaces with dedicated worktrees are eligible for cleanup; direct-clone workspaces are not.
export const hasWorktree = (ws: Workspace) => ws.archived && !ws.cleaned && ws.worktree !== ws.repo;

/// Preparing or failed workspaces have no usable tabs, terminals, files, diffs, or docks; their cards explain the state.
export const pending = (ws: Workspace) => ws.preparing || !!ws.failed;

/// Detect a branch already checked out at another path. Reusing the same repository set can reuse its worktree; changing that set may produce a conflicting destination.
export const branchTaken = (board: Board, repos: string[], branch: string) =>
  board.workspaces.find(
    (w) =>
      !w.cleaned &&
      w.branch === branch &&
      w.repos.some((r) => repos.includes(r.path)) &&
      !(w.worktree !== w.repo && sameRepos(w, repos)),
  ) ?? null;

const sameRepos = (w: Workspace, repos: string[]) =>
  w.repos.length === repos.length && w.repos.every((r) => repos.includes(r.path));

/// Cleanup candidates include disk usage and a backend reason requiring explicit force confirmation.
export type Cleanable = {
  id: string;
  title: string;
  repoName: string;
  branch: string;
  worktree: string;
  sizeKb: number;
  pr: number | null;
  blocked: string | null;
};

/// Dock PTYs are separate from agent sessions. Setup/run are fixed repository scripts; terminal, terminal-2, and later shells exist only after explicit creation.
export type DockKind = "setup" | "run" | `terminal${string}`;

/// Distinguish shell-tab closure from stopping a fixed repository script.
export const isTerm = (kind: DockKind) => kind.startsWith("terminal");

/// The unsuffixed terminal is number one; later suffixes determine tab order and labels.
export const termNumber = (kind: DockKind) => Number(kind.slice("terminal-".length)) || 1;
export const termKind = (n: number): DockKind => (n === 1 ? "terminal" : `terminal-${n}`);

/// A dock may be running or retain output after exit.
export type DockState = { kind: DockKind; alive: boolean };

/// Repository script settings from .prometeu/settings.toml or legacy .conductor/settings.toml, plus the workspace port.
export type Scripts = {
  /// Null settings source means no declared scripts; show the setup invitation instead of an empty terminal.
  file: string | null;
  /// Inherit source-clone settings when the worktree lacks them, including ignored .prometeu directories. Editing first copies the inherited file locally.
  inherited: boolean;
  setup: string | null;
  runs: { name: string; command: string }[];
  archive: string | null;
  /// Copy source-clone ignored files such as .env before setup. Keep the declared list after copying so Setup remains available even without a setup command.
  copy: string[];
  port: number | null;
};

export type Board = { actions?: import("./actions").Catalog; tools?: Tools; tool_trust?: ToolTrust[]; stages: string[]; projects: Project[]; workspaces: Workspace[] };

/// The authenticated Linear user and organization.
export type LinearWho = { name: string; email: string; org: string; org_key: string };
/// Persist enough issue metadata for workspace labels, links, and existing-workspace detection.
export type IssueRef = { id: string; identifier: string; title: string; url: string };

/// Linear issues group by state kind. Priority zero is unset; one is urgent and four is low.
export type Issue = IssueRef & {
  description: string | null;
  branch_name: string;
  priority: number;
  priority_label: string;
  state: { name: string; kind: string; color: string };
  team: string;
  project: string | null;
  labels: { name: string; color: string }[];
  updated_at: string;
};
export type Issues = { issues: Issue[]; fetched_at: number };

/// Busy identifies browser-based authentication, which remains visible across navigation.
export type LinearStatus = { connected: boolean; who: LinearWho | null; busy: boolean };

/// A changed worktree file. Patch contains unified hunks and is empty for binary or oversized content.
export type Change = {
  path: string;
  added: number;
  removed: number;
  new_file: boolean;
  deleted: boolean;
  /// Indicate uncommitted changes beside the filename.
  dirty: boolean;
  patch: string;
};

/// Repository changes include branch commits and uncommitted edits relative to the base, in workspace repository order. Unchanged repositories return empty file lists.
export type RepoDiff = {
  name: string;
  /// Branch base used for ahead counts and comparisons.
  base: string;
  ahead: number;
  /// Commits not yet pushed, distinguishing local commits from remote PR contents.
  unpushed: number;
  /// Number of files with uncommitted changes.
  dirty: number;
  files: Change[];
};

const RANK: Record<Status, number> = { querendo: 3, rodando: 2, pronta: 1, desligada: 0 };

/// The most urgent tab determines workspace status, matching Rust rank ordering.
export function worst(ws: Workspace): Tab | undefined {
  return [...ws.tabs].sort((a, b) => RANK[b.status] - RANK[a.status])[0];
}

export function statusOf(ws: Workspace): Status {
  return worst(ws)?.status ?? "desligada";
}

/// Compact localized token counts; values below one thousand remain unscaled.
export function fmtTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toLocaleString(locale(), { maximumFractionDigits: 1 })}M`;
}

/// Unnamed tabs use their model label. Add a number only when siblings share that same unnamed model.
export function tabLabel(ws: Pick<Workspace, "tabs" | "agent" | "model">, tab: Tab): string {
  if (tab.title) return tab.title;
  const model = (t: Tab) => t.choice?.model ?? ws.model;
  const provider = (t: Tab) => t.choice?.agent ?? ws.agent;
  const name = modelLabelOf(model(tab), provider(tab)) || t(`model.${provider(tab)}`);
  const twins = ws.tabs.filter((t) => !t.title && model(t) === model(tab) && provider(t) === provider(tab));
  return twins.length > 1 ? `${name} ${twins.indexOf(tab) + 1}` : name;
}
