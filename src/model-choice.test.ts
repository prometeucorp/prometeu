import { beforeEach, expect, it, vi } from "vitest";
import { loadAgents, refreshModels, installed } from "./agents";
import { defaultChoice, defaultEffort, effortLadder, effortStep, favoriteChoices, fitsEffort, isFavorite, setDefaultChoice, setDefaultEffort, toggleFavorite } from "./model-choice";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("./ipc", () => ({ invoke: mocks.invoke }));
let storage: Map<string, string>;
beforeEach(async () => {
  storage = new Map();
  vi.stubGlobal("localStorage", { getItem: (k: string) => storage.get(k) ?? null, setItem: (k: string, v: string) => storage.set(k, v), removeItem: (k: string) => storage.delete(k) });
  mocks.invoke.mockImplementation(async (cmd, args) => cmd === "agents" ? { providers: ["claude", "codex", "antigravity"].map(id => ({ id, label: id, installed: true, models: [], authMethods: [], capabilities: {} })) } : { fetchedAt: Date.now(), models: args.agent === "claude" ? [{ id: "opus", label: "Live Opus", efforts: ["high", "xhigh"] }] : [{ id: "shared", label: args.agent, efforts: args.agent === "codex" ? ["none", "low", "ultra", "future"] : [] }] });
  await loadAgents();
  await Promise.all(installed().map(p => refreshModels(p.id)));
});
it("retains explicit provider identity and preserves removed preferences", () => {
  setDefaultChoice({ agent: "antigravity", model: "removed" });
  expect(defaultChoice()).toEqual({ agent: "antigravity", model: "removed" });
});
it("migrates an unambiguous legacy preference but asks for ambiguous IDs", () => {
  storage.set("prometeu:model", "opus");
  expect(defaultChoice()).toEqual({ agent: "claude", model: "opus" });
  storage.delete("prometeu:model-choice");
  storage.set("prometeu:model", "shared");
  expect(defaultChoice()).toBeNull();
  expect(storage.get("prometeu:model")).toBe("shared");
});
it("retains favorites independently for identical IDs and tolerates corrupt storage", () => {
  const a = { agent: "codex" as const, model: "shared" }, b = { agent: "antigravity" as const, model: "shared" };
  toggleFavorite(a); toggleFavorite(b); toggleFavorite(a);
  expect(isFavorite(a)).toBe(false);
  expect(isFavorite(b)).toBe(true);
  expect(favoriteChoices()).toEqual([b]);
  storage.set("prometeu:model-favorites", "oops");
  expect(favoriteChoices()).toEqual([]);
});
it("does not invent unsupported efforts and does not drop unknown native levels", () => {
  expect(effortLadder("shared", "codex").map(([id]) => id)).toEqual(["", "none", "low", "ultra", "future"]);
  expect(effortLadder("shared", "antigravity").map(([id]) => id)).toEqual([""]);
  expect(fitsEffort("opus", "low", "claude")).toBe("");
  expect(fitsEffort("shared", "ultracode", "codex")).toBe("ultra");
  expect(fitsEffort("opus", "ultracode", "claude")).toBe("");
  expect(effortStep("shared", "", "antigravity")).toBeNull();
});
it("keeps historical invalid effort visible until explicitly edited", () => {
  setDefaultEffort("ultracode");
  expect(defaultEffort({ agent: "claude", model: "opus" })).toBe("ultracode");
  expect(defaultEffort({ agent: "codex", model: "shared" })).toBe("ultra");
  expect(effortStep("opus", "ultracode", "claude")?.label).toContain("ultracode");
});
it("does not infer a legacy provider while another installed catalog is unavailable", async () => {
  storage.set("prometeu:model", "shared");
  mocks.invoke.mockImplementation(async (cmd, args) => {
    if (cmd === "agents") return { providers: ["claude", "codex"].map(id => ({ id, label: id, installed: true, models: [], authMethods: [], capabilities: {} })) };
    if (args.agent === "codex") throw { code: "err.modelsCatalog.timeout" };
    return { fetchedAt: Date.now(), models: [{ id: "shared", label: "Shared", efforts: [] }] };
  });
  await loadAgents();
  await Promise.all(installed().map(p => refreshModels(p.id)));
  expect(defaultChoice()).toBeNull();
  expect(storage.has("prometeu:model-choice")).toBe(false);
  mocks.invoke.mockResolvedValue({ fetchedAt: Date.now(), models: [{ id: "shared", label: "Shared Codex", efforts: [] }] });
  await refreshModels("codex", true);
  expect(defaultChoice()).toBeNull();
});

it("preserves Cursor ACP model IDs in saved selections and favorites", () => {
  const choice = { agent: "cursor" as const, model: "composer-2.5[fast=true]" };
  setDefaultChoice(choice);
  expect(defaultChoice()).toEqual(choice);
  toggleFavorite(choice);
  expect(favoriteChoices()).toContainEqual(choice);
});
