import type { Board, Tab, Workspace } from "./types";

/** Delegation ownership reserves one conversation even if its coordinator is external or gone. */
export function isDelegatedWorkspace(board: Board, id: string): boolean {
  return !!board.delegations?.some(delegation => delegation.workspace === id);
}

export type WorkspaceParent = { workspace: Workspace; tab: Tab };

/** Flatten ownership chains for navigation without changing MCP authority or persisted records. */
export function workspaceHierarchy(board: Board) {
  const live = board.workspaces.filter(workspace => !workspace.archived && !workspace.remote);
  const byTab = new Map(live.flatMap(workspace => workspace.tabs.map(tab => [tab.id, { workspace, tab }] as const)));
  const direct = new Map<string, WorkspaceParent>();
  for (const delegation of board.delegations ?? []) {
    const owner = byTab.get(delegation.owner);
    if (owner) direct.set(delegation.workspace, owner);
  }
  const parents = new Map<string, WorkspaceParent>();
  const children = new Map<string, Workspace[]>();
  for (const workspace of live) {
    let parent = direct.get(workspace.id);
    const seen = new Set([workspace.id]);
    while (parent) {
      if (seen.has(parent.workspace.id)) { parent = undefined; break; }
      seen.add(parent.workspace.id);
      const next = direct.get(parent.workspace.id);
      if (!next) break;
      parent = next;
    }
    if (!parent) continue;
    parents.set(workspace.id, parent);
    const siblings = children.get(parent.workspace.id) ?? [];
    siblings.push(workspace);
    children.set(parent.workspace.id, siblings);
  }
  return { parents, children };
}
