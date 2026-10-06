import type { GitHubNotification, Workspace } from "./types";

/// Read state is local to this Mac (ADR 0088): everything up to `before`, plus individual ids.
export type ReadState = { before: string; ids: string[] };
export const READ_KEY = "prometeu:github-notifications:read";
/// Keep the newest rows; the Cloud expires them after 30 days anyway.
const KEEP = 300;
const READ_IDS = 500;

/// Feed ids are decimal strings; compare by length first so "10" follows "9".
export const compareId = (a: string, b: string) => a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);

export function readState(): ReadState {
  try {
    const value = JSON.parse(localStorage.getItem(READ_KEY) ?? "null");
    if (value && typeof value.before === "string" && /^\d*$/.test(value.before) && Array.isArray(value.ids)) {
      return { before: value.before, ids: value.ids.filter((id: unknown) => typeof id === "string").slice(-READ_IDS) };
    }
  } catch { /* A damaged record only resets read marks. */ }
  return { before: "", ids: [] };
}

export function saveRead(state: ReadState) {
  localStorage.setItem(READ_KEY, JSON.stringify(state));
}

export const isRead = (state: ReadState, id: string) =>
  (state.before !== "" && compareId(id, state.before) <= 0) || state.ids.includes(id);

export function markRead(state: ReadState, id: string): ReadState {
  return isRead(state, id) ? state : { ...state, ids: [...state.ids, id].slice(-READ_IDS) };
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
