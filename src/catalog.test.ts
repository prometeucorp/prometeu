import { beforeAll, describe, expect, it, vi } from "vitest";
import type { CatalogState } from "./catalog";

const state: CatalogState = {
  connected: true, revision: 1,
  plugins: [
    // An unlinked equivalent of the local caveman, reported by the backend as installed.
    { id: "caveman", source: "https://github.com/juliusbrussee/caveman", note: "", local_id: "caveman", installed: true, source_changed: false },
    { id: "typesafe", source: "https://github.com/typesafe-ai/skills", note: "", local_id: "typesafe", installed: false, source_changed: false },
  ],
  mcp: ["capisce"],
  skills: [],
  shared: { "mcp:capisce": "capisce" },
  organization_items: [
    { organization: "capim", organization_name: "Capim", revision: 1, kind: "plugins", id: "Caveman", description: "", installed: true, local_id: "caveman" },
    { organization: "capim", organization_name: "Capim", revision: 1, kind: "plugins", id: "Typesafe", description: "", installed: false, local_id: null },
    { organization: "capim", organization_name: "Capim", revision: 1, kind: "mcp", id: "postgres", description: "", installed: false, local_id: null },
  ],
};
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("./ipc", () => ({ invoke: vi.fn(() => Promise.resolve(state)) }));
vi.mock("./util", () => ({ h: vi.fn() }));
vi.mock("./ui", async importOriginal => ({
  ...await importOriginal<typeof import("./ui")>(),
  formDialog: vi.fn((_options: Parameters<typeof import("./ui").formDialog>[0]) => ({
    body: { append: vi.fn() }, open: vi.fn(),
  })),
}));
const { invoke } = await import("./ipc");
const { formDialog } = await import("./ui");

const catalog = await import("./catalog");
const { t, use } = await import("./i18n");
use("en");
beforeAll(() => catalog.load());

describe("catalog origins", () => {
  it("lists every catalog that offers an installed item on the same row", () => {
    expect(catalog.installedOrigins("plugins", "caveman").map(o => o.label)).toEqual([t("catalog.thisMac"), "Personal", "Capim"]);
    expect(catalog.installedOrigins("mcp", "capisce").map(o => o.label)).toEqual([t("catalog.thisMac"), "Personal ⇄"]);
  });

  it("flags a same-name definition that differs instead of listing it again", () => {
    const origins = catalog.installedOrigins("mcp", "postgres");
    expect(origins.map(o => o.label)).toEqual([t("catalog.thisMac"), "Capim ≠"]);
    expect(origins[1].hint).toBeTruthy();
  });

  it("offers one row per missing name, installable from each catalog", () => {
    const groups = catalog.pendingGroups("plugins", ["caveman"]);
    expect(groups.map(items => items.map(item => `${item.id}@${item.origin}`))).toEqual([["typesafe@Personal", "Typesafe@Capim"]]);
    expect(catalog.pendingGroups("mcp", ["postgres"])).toEqual([]);
  });
});

// The dialog owns retries and cancellation; the row must stay locked until it closes.
it.each(["cancel", "success", "retry"])("holds the plugin row lock through dialog %s", async outcome => {
  vi.mocked(formDialog).mockClear();
  vi.mocked(invoke).mockClear();
  let finish!: () => void;
  const installation = new Promise<void>(resolve => { finish = resolve; });
  let fail = outcome === "retry";
  vi.mocked(invoke).mockImplementation(async (...[command]) => {
    if (command === "catalog_state") return state as never;
    if (command === "catalog_install_plugin") {
      if (fail) { fail = false; throw new Error("Installation failed"); }
      await installation; return undefined as never;
    }
    throw new Error(`Unexpected command: ${command}`);
  });
  const row = () => catalog.pendingResources("plugins", ["caveman"], vi.fn())[0];
  const action = row().actions.find(item => item !== "sep" && item.label === t("catalog.installFrom", { origin: "Personal" }));
  if (!action || action === "sep") throw new Error("Missing personal installation");
  action.run!();
  await catalog.load();
  expect(row().busy).toBe(true);
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "catalog_install_plugin")).toHaveLength(0);
  action.run!();
  expect(formDialog).toHaveBeenCalledTimes(1);
  const dialog = vi.mocked(formDialog).mock.calls[0][0];
  if (outcome !== "cancel") {
    if (outcome === "retry") {
      await expect(dialog.submit()).rejects.toThrow("Installation failed");
      expect(row().busy).toBe(true);
      action.run!();
      expect(formDialog).toHaveBeenCalledTimes(1);
    }
    const submitted = dialog.submit();
    await catalog.load();
    expect(row().busy).toBe(true);
    expect(invoke).toHaveBeenCalledWith("catalog_install_plugin", { id: "typesafe" });
    finish(); await submitted;
    expect(row().busy).toBe(true);
  }
  dialog.closed!();
  await vi.waitFor(() => expect(row().busy).toBe(false));
});
