import { describe, expect, it } from "vitest";
import { use } from "./i18n";
import { branchTaken, fmtTokens, tabLabel, toggleSelection, type Board, type Selection, type Tab, type Workspace } from "./types";

// Portuguese formatting uses a decimal comma, as in 1,2M.
use("pt-BR");

describe("fmtTokens", () => {
  it("abaixo de mil é o número; depois k e M arredondados", () => {
    expect(fmtTokens(812)).toBe("812");
    expect(fmtTokens(56_748)).toBe("57k");
    expect(fmtTokens(999_400)).toBe("999k");
    expect(fmtTokens(1_234_000)).toBe("1,2M");
  });
});

describe("branchTaken", () => {
  const ws = (id: string, branch: string, repos: string[], solto = false) =>
    ({
      id,
      title: id,
      branch,
      cleaned: false,
      repo: repos[0],
      worktree: solto ? repos[0] : `/wt/${repos.map((r) => r.slice(1)).join("+")}/${branch}`,
      repos: repos.map((path) => ({ path, name: path.slice(1), worktree: "", base: "", pr: null })),
    }) as unknown as Workspace;

  const board = (...workspaces: Workspace[]) =>
    ({ stages: [], projects: [], workspaces }) as unknown as Board;

  it("acusa o workspace que já abriu a branch quando os repos não são os mesmos", () => {
    const um = ws("um", "aut-49", ["/rules"]);
    expect(branchTaken(board(um), ["/rules", "/autonomous"], "aut-49")?.id).toBe("um");
  });

  it("mesmos repos caem na mesma pasta: reaproveitar não é disputar", () => {
    const um = ws("um", "aut-49", ["/rules"]);
    expect(branchTaken(board(um), ["/rules"], "aut-49")).toBeNull();
  });

  it("branch presa ao clone (sem worktree) disputa até com os mesmos repos", () => {
    const solto = ws("solto", "aut-49", ["/rules"], true);
    expect(branchTaken(board(solto), ["/rules"], "aut-49")?.id).toBe("solto");
  });

  it("outra branch, outro repo ou worktree já devolvido não disputam nada", () => {
    const outra = ws("outra", "aut-50", ["/rules"]);
    const alheio = ws("alheio", "aut-49", ["/outro"]);
    const limpo = { ...ws("limpo", "aut-49", ["/rules"]), cleaned: true } as Workspace;
    expect(branchTaken(board(outra, alheio, limpo), ["/rules", "/autonomous"], "aut-49")).toBeNull();
  });
});

describe("tabLabel", () => {
  const tab = (id: string, title = "", choice?: Tab["choice"]) =>
    ({ id, title, status: "pronta", note: null, tokens: null, choice }) as Tab;
  const ws = (...tabs: Tab[]) => ({ tabs, agent: "claude", model: "opus" }) as Workspace;

  it("nome dado vence; sem nome, é o modelo da aba ou do workspace", () => {
    const named = tab("a", "Corrigir o menu");
    const own = tab("b", "", { agent: "claude", model: "sonnet", effort: "low" });
    const inherited = tab("c");
    const board = ws(named, own, inherited);
    expect(tabLabel(board, named)).toBe("Corrigir o menu");
    expect(tabLabel(board, own)).toBe("Sonnet");
    expect(tabLabel(board, inherited)).toBe("Opus");
  });

  it("duas irmãs sem nome no mesmo modelo ganham número; a de outro modelo não", () => {
    const um = tab("a");
    const dois = tab("b");
    const outra = tab("c", "", { agent: "codex", model: "gpt-5-codex", effort: "medium" });
    const board = ws(um, dois, outra);
    expect(tabLabel(board, um)).toBe("Opus 1");
    expect(tabLabel(board, dois)).toBe("Opus 2");
    expect(tabLabel(board, outra)).not.toMatch(/\d$/);
  });

  it("modelo que o catálogo não conhece e workspace remoto sem modelo caem no nome do provider", () => {
    const remote = tab("r");
    expect(tabLabel({ tabs: [remote], agent: "claude", model: "" } as Workspace, remote)).toBe("Claude Code");
  });
});

describe("toggleSelection", () => {
  it("a primeira escolha herda e adiciona, sem apagar o que vem de cima", () => {
    expect(toggleSelection(null, "notion", true)).toEqual({ base: "inherit", add: ["notion"], remove: [] });
  });

  it("desligar troca o add por remove e preserva a base", () => {
    const on: Selection = { base: "none", add: ["notion"], remove: [] };
    expect(toggleSelection(on, "notion", false)).toEqual({ base: "none", add: [], remove: ["notion"] });
  });

  it("religar um item removido o tira do remove e devolve ao add", () => {
    const off: Selection = { base: "inherit", add: [], remove: ["notion"] };
    expect(toggleSelection(off, "notion", true)).toEqual({ base: "inherit", add: ["notion"], remove: [] });
  });

  it("os outros ids da camada ficam intactos", () => {
    const layer: Selection = { base: "inherit", add: ["a"], remove: ["r"] };
    expect(toggleSelection(layer, "b", true)).toEqual({ base: "inherit", add: ["a", "b"], remove: ["r"] });
    expect(toggleSelection(layer, "a", false)).toEqual({ base: "inherit", add: [], remove: ["r", "a"] });
  });
});
