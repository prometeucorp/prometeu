import type { GitHubNotification, Workspace } from "./types";

/// Read state is local to this Mac and per Prometeu account (ADR 0088): everything up to
/// `before`, plus individual ids. Feed ids are only comparable within one account.
export type ReadState = { before: string; ids: string[] };
export const readKey = (account: string) => `prometeu:github-notifications:read:${account}`;
/// Keep the newest rows; the Cloud expires them after 30 days anyway.
const KEEP = 300;
const READ_IDS = 500;

/// Feed ids are decimal strings; compare by length first so "10" follows "9".
export const compareId = (a: string, b: string) => a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);

export function readState(account: string): ReadState {
  try {
    const value = JSON.parse(localStorage.getItem(readKey(account)) ?? "null");
    if (value && typeof value.before === "string" && /^\d*$/.test(value.before) && Array.isArray(value.ids)) {
      return { before: value.before, ids: value.ids.filter((id: unknown) => typeof id === "string").slice(-READ_IDS) };
    }
  } catch { /* A damaged record only resets read marks. */ }
  return { before: "", ids: [] };
}

export function saveRead(account: string, state: ReadState) {
  localStorage.setItem(readKey(account), JSON.stringify(state));
}

export const isRead = (state: ReadState, id: string) =>
  (state.before !== "" && compareId(id, state.before) <= 0) || state.ids.includes(id);

/// Ids already in the list move to its end, so marking a thread never evicts its own members.
export function markRead(state: ReadState, ...ids: string[]): ReadState {
  const marked = ids.filter(id => state.before === "" || compareId(id, state.before) > 0);
  return { ...state, ids: [...state.ids.filter(id => !marked.includes(id)), ...marked].slice(-READ_IDS) };
}

export function markAll(state: ReadState, items: GitHubNotification[]): ReadState {
  const newest = items.reduce((max, item) => (compareId(item.id, max) > 0 ? item.id : max), state.before);
  return { before: newest, ids: [] };
}

/// Ascending by id, without duplicates, bounded.
export function merge(existing: GitHubNotification[], incoming: GitHubNotification[]) {
  const byId = new Map(existing.map(item => [item.id, item]));
  for (const item of incoming) byId.set(item.id, item);
  return [...byId.values()].sort((a, b) => compareId(a.id, b.id)).slice(-KEEP);
}

export const subjectKey = (repository: string, number: number) => `${repository.toLowerCase()}#${number}`;

/// The live workspace for a notification: its GitHub item opened the workspace, or one of its
/// repositories has that PR. `repositories` maps a clone path to its GitHub repositories.
export function workspaceFor(
  item: GitHubNotification,
  workspaces: Workspace[],
  repositories: Map<string, string[]>,
): Workspace | undefined {
  const kind = item.subject === "pr" ? "pull" : "issues";
  const url = `https://github.com/${item.repository}/${kind}/${item.number}`.toLowerCase();
  const repository = item.repository.toLowerCase();
  return workspaces.find(workspace => !workspace.archived && !workspace.cleaned && !workspace.remote && (
    workspace.issue?.url.toLowerCase() === url ||
    (item.subject === "pr" && workspace.repos.some(repo =>
      repo.pr?.number === item.number && (repositories.get(repo.path) ?? []).includes(repository)))
  ));
}

/// Kinds that need the person first; a bot comment must not bury a failed CI or a review request.
const PRIORITY: GitHubNotification["kind"][] = [
  "ci_failed", "changes_requested", "review_requested", "mentioned",
  "review_approved", "merged", "closed", "assigned", "commented",
];

/// Every notification of one issue or PR, ascending by id. `lead` names the row: the most
/// urgent unread event, or the newest once everything is read.
export type Thread = { key: string; items: GitHubNotification[]; lead: GitHubNotification; latest: GitHubNotification };

/// One thread per issue or PR, newest activity first.
export function threads(items: GitHubNotification[], state: ReadState): Thread[] {
  const groups = new Map<string, GitHubNotification[]>();
  for (const item of [...items].sort((a, b) => compareId(a.id, b.id))) {
    const key = subjectKey(item.repository, item.number);
    groups.set(key, [...(groups.get(key) ?? []), item]);
  }
  return [...groups].map(([key, group]) => {
    const latest = group[group.length - 1];
    const unread = group.filter(item => !isRead(state, item.id));
    const lead = unread.length
      ? unread.reduce((best, item) => PRIORITY.indexOf(item.kind) <= PRIORITY.indexOf(best.kind) ? item : best)
      : latest;
    return { key, items: group, lead, latest };
  }).sort((a, b) => compareId(b.latest.id, a.latest.id));
}
