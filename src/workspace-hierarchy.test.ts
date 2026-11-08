import { describe, expect, it } from "vitest";
import { isDelegatedWorkspace, workspaceHierarchy } from "./workspace-hierarchy";
import type { Board, Delegation, Workspace } from "./types";

const workspace = (id: string, tabs = [id + "-tab"]) => ({
  id, title: id, tabs: tabs.map(id => ({ id })),
}) as Workspace;
const delegation = (workspace: string, owner: string): Delegation => ({
  id: workspace + "-tab", workspace, owner, task: "Review changes", request_key: workspace,
  request_hash: "hash", repository_heads: {}, executions: [], background: null, requests: [],
});
const board = (workspaces: Workspace[], delegations?: Delegation[]): Board => ({
  workspaces, delegations, stages: [], projects: [],
});

describe("workspace hierarchy", () => {
  it("reads legacy boards and preserves ordinary conversation creation", () => {
    const state = board([workspace("main")]);
    expect(workspaceHierarchy(state).parents.size).toBe(0);
    expect(isDelegatedWorkspace(state, "main")).toBe(false);
  });

  it("uses the owning conversation, not active tab, branch name or project", () => {
    const main = { ...workspace("main", ["responsible", "other"]), active: "other", project: "first" };
    const worker = { ...workspace("worker"), project: "second", pinned: true };
    const unrelated = { ...workspace("unrelated"), branch: "delegated-looking-name" };
    const state = board([worker, unrelated, main], [delegation("worker", "responsible")]);
    const hierarchy = workspaceHierarchy(state);
    expect(hierarchy.parents.get("worker")).toEqual({ workspace: main, tab: main.tabs[0] });
    expect(hierarchy.children.get("main")).toEqual([worker]);
    expect(hierarchy.parents.has("unrelated")).toBe(false);
    expect(isDelegatedWorkspace(state, "worker")).toBe(true);
  });

  it("flattens recursive delegation without rewriting ownership or losing legacy tabs", () => {
    const main = workspace("main");
    const worker = workspace("worker", ["worker-tab", "legacy-person-tab"]);
    const nested = workspace("nested");
    const state = board([main, worker, nested], [delegation("worker", "main-tab"), delegation("nested", "worker-tab")]);
    const before = JSON.stringify(state);
    const hierarchy = workspaceHierarchy(state);
    expect(hierarchy.children.get("main")).toEqual([worker, nested]);
    expect(hierarchy.children.has("worker")).toBe(false);
    expect(hierarchy.parents.get("nested")?.tab.id).toBe("main-tab");
    expect(JSON.stringify(state)).toBe(before);
  });

  it("keeps external, missing-owner and archived-owner workspaces independently accessible", () => {
    const main = { ...workspace("main"), archived: true };
    const state = board([main, workspace("external"), workspace("orphan"), workspace("worker")], [
      delegation("external", "client:external"), delegation("orphan", "deleted-tab"), delegation("worker", "main-tab"),
    ]);
    expect(workspaceHierarchy(state).parents.size).toBe(0);
    for (const id of ["external", "orphan", "worker"]) expect(isDelegatedWorkspace(state, id)).toBe(true);
    main.archived = false;
    expect(workspaceHierarchy(state).parents.get("worker")?.workspace.id).toBe("main");
  });

  it("never hides cycles or invents a local family for a remote share", () => {
    const remote = { ...workspace("remote"), remote: { owner: "peer", online: true } } as Workspace;
    const state = board([workspace("a"), workspace("b"), remote, workspace("worker")], [
      delegation("a", "b-tab"), delegation("b", "a-tab"), delegation("worker", "remote-tab"),
    ]);
    expect(workspaceHierarchy(state).parents.size).toBe(0);
  });

  it("omits archived children while keeping the workspace stage independent", () => {
    const main = workspace("main");
    const archived = { ...workspace("archived"), archived: true };
    const finished = { ...workspace("finished"), stage: "Done" };
    const hierarchy = workspaceHierarchy(board([main, archived, finished], [
      delegation("archived", "main-tab"), delegation("finished", "main-tab"),
    ]));
    expect(hierarchy.children.get("main")).toEqual([finished]);
  });
});
