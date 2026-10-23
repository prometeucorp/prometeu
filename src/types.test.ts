import { describe, expect, it } from "vitest";
import { use } from "./i18n";
import { branchTaken, dockLabel, fmtTokens, repoPath, tabLabel, terminalName, toggleSelection, type Board, type Selection, type Tab, type Workspace } from "./types";

use("en");

describe("fmtTokens", () => {
  it("shows numbers below a thousand and rounded k or M suffixes above", () => {
    expect(fmtTokens(812)).toBe("812");
    expect(fmtTokens(56_748)).toBe("57k");
    expect(fmtTokens(999_400)).toBe("999k");
    expect(fmtTokens(1_234_000)).toBe("1.2M");
  });
});

describe("branchTaken", () => {
  const ws = (id: string, branch: string, repos: string[], cloneBound = false) =>
    ({
      id,
      title: id,
      branch,
      cleaned: false,
      repo: repos[0],
      worktree: cloneBound ? repos[0] : `/wt/${repos.map((r) => r.slice(1)).join("+")}/${branch}`,
      repos: repos.map((path) => ({ path, name: path.slice(1), worktree: "", base: "", pr: null })),
    }) as unknown as Workspace;

  const board = (...workspaces: Workspace[]) =>
    ({ stages: [], projects: [], workspaces }) as unknown as Board;

  it("finds the workspace already using the branch when repository sets differ", () => {
    const first = ws("one", "aut-49", ["/rules"]);
    expect(branchTaken(board(first), ["/rules", "/autonomous"], "aut-49")?.id).toBe("one");
  });

  it("reuses the same directory for the same repositories without reporting a conflict", () => {
    const first = ws("one", "aut-49", ["/rules"]);
    expect(branchTaken(board(first), ["/rules"], "aut-49")).toBeNull();
  });

  it("reports clone-bound branches as conflicts even with the same repositories", () => {
    const cloneBound = ws("unbound", "aut-49", ["/rules"], true);
    expect(branchTaken(board(cloneBound), ["/rules"], "aut-49")?.id).toBe("unbound");
  });

  it("ignores other branches, other repositories and released worktrees", () => {
    const otherBranch = ws("other", "aut-50", ["/rules"]);
    const otherRepository = ws("unrelated", "aut-49", ["/other"]);
    const cleaned = { ...ws("clean", "aut-49", ["/rules"]), cleaned: true } as Workspace;
    expect(branchTaken(board(otherBranch, otherRepository, cleaned), ["/rules", "/autonomous"], "aut-49")).toBeNull();
  });
});

describe("tabLabel", () => {
  const tab = (id: string, title = "", choice?: Tab["choice"]) =>
    ({ id, title, status: "pronta", note: null, tokens: null, choice }) as Tab;
  const ws = (...tabs: Tab[]) => ({ tabs, agent: "claude", model: "opus" }) as Workspace;

  it("prefers explicit names, then the tab or workspace model", () => {
    const named = tab("a", "Fix the menu");
    const own = tab("b", "", { agent: "claude", model: "sonnet", effort: "low" });
    const inherited = tab("c");
    const board = ws(named, own, inherited);
    expect(tabLabel(board, named)).toBe("Fix the menu");
    expect(tabLabel(board, own)).toBe("Sonnet");
    expect(tabLabel(board, inherited)).toBe("Opus");
  });

  it("numbers unnamed sibling tabs using the same model without numbering other models", () => {
    const first = tab("a");
    const second = tab("b");
    const otherBranch = tab("c", "", { agent: "codex", model: "gpt-5-codex", effort: "medium" });
    const board = ws(first, second, otherBranch);
    expect(tabLabel(board, first)).toBe("Opus 1");
    expect(tabLabel(board, second)).toBe("Opus 2");
    expect(tabLabel(board, otherBranch)).not.toMatch(/\d$/);
  });

  it("falls back to the provider name for unknown models and remote workspaces without models", () => {
    const remote = tab("r");
    expect(tabLabel({ tabs: [remote], agent: "claude", model: "" } as Workspace, remote)).toBe("Claude Code");
  });
});

describe("toggleSelection", () => {
  it("inherits and adds the first selection without clearing the parent layer", () => {
    expect(toggleSelection(null, "notion", true)).toEqual({ base: "inherit", add: ["notion"], remove: [] });
  });

  it("moves disabled items from add to remove while preserving the base", () => {
    const on: Selection = { base: "none", add: ["notion"], remove: [] };
    expect(toggleSelection(on, "notion", false)).toEqual({ base: "none", add: [], remove: ["notion"] });
  });

  it("moves reenabled items from remove back to add", () => {
    const off: Selection = { base: "inherit", add: [], remove: ["notion"] };
    expect(toggleSelection(off, "notion", true)).toEqual({ base: "inherit", add: ["notion"], remove: [] });
  });

  it("preserves other IDs in the layer", () => {
    const layer: Selection = { base: "inherit", add: ["a"], remove: ["r"] };
    expect(toggleSelection(layer, "b", true)).toEqual({ base: "inherit", add: ["a", "b"], remove: ["r"] });
    expect(toggleSelection(layer, "a", false)).toEqual({ base: "inherit", add: [], remove: ["r", "a"] });
  });
});

describe("repoPath", () => {
  const repo = (name: string, worktree: string) => ({ path: `/src/${name}`, name, worktree, base: "main", pr: null });
  const single = { worktree: "/wt/api/feat", repos: [repo("api", "/wt/api/feat")] };
  const multi = { worktree: "/wt/api+web/feat", repos: [repo("api", "/wt/api+web/feat/api"), repo("web", "/wt/api+web/feat/web")] };

  it("keeps repository paths as they are in a single-repository workspace", () => {
    expect(repoPath(single, "api", "src/main.ts")).toBe("src/main.ts");
    expect(repoPath(single, undefined, ".prometeu/settings.toml")).toBe(".prometeu/settings.toml");
  });

  it("prefixes the named repository's directory in a multi-repository workspace", () => {
    expect(repoPath(multi, "web", "src/app.ts")).toBe("web/src/app.ts");
    expect(repoPath(multi, "api", "src/main.ts")).toBe("api/src/main.ts");
  });

  it("places a path without a repository, like the dock's scripts file, under the primary repository", () => {
    expect(repoPath(multi, undefined, ".prometeu/settings.toml")).toBe("api/.prometeu/settings.toml");
  });

  it("falls back to the workspace root for an unknown repository", () => {
    expect(repoPath(multi, "gone", "README.md")).toBe("README.md");
  });
});

describe("dockLabel", () => {
  it("numbers later shells the way their dock tabs do", () => {
    expect((["terminal", "terminal-2", "terminal-10"] as const).map(dockLabel)).toEqual(["Terminal", "Terminal 2", "Terminal 10"]);
  });
});

describe("terminalName", () => {
  const session = (folder: string, title = "") => ({ id: "terminal-a", folder, title, running: false });

  it("shows the folder's own name until the person names the session, as the backend does", () => {
    expect(terminalName(session("/Users/person/dev/api"))).toBe("api");
    expect(terminalName(session("/Users/person/dev/api/"))).toBe("api");
    expect(terminalName(session("/Users/person/dev/api", "  server logs "))).toBe("server logs");
    expect(terminalName(session("/Users/person/dev/api", "   "))).toBe("api");
    expect(terminalName(session("/"))).toBe("/");
  });
});

