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
vi.mock("./ipc", () => ({ invoke: () => Promise.resolve(state) }));

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
