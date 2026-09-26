import { expect, test, vi } from "vitest";
vi.mock("../ipc", () => ({ invoke: vi.fn() }));
import { invoke } from "../ipc";
import * as mcp from "../mcp";
import * as plugins from "../plugins";
import * as skills from "../skills";
import * as catalog from "../catalog";
import { matchesResource } from "./model";
import { t } from "../i18n";

// No DOM environment: constructing resource snapshots must not construct hidden controls.
test("hub snapshots preserve grouped origins, allowed operations and pending locks without DOM", async () => {
  let finishLogin!: () => void;
  const login = new Promise<void>(resolve => { finishLogin = resolve; });
  let finishInstall!: () => void;
  const installation = new Promise<void>(resolve => { finishInstall = resolve; });
  const state: catalog.CatalogState = {
    connected: true, revision: 42, plugins: [], mcp: ["notes"], skills: [], shared: { "mcp:notes": "notes" },
    organization_items: ["one", "two"].flatMap(organization => [
      { organization, organization_name: organization, revision: 42, kind: "mcp" as const,
        id: "notes", description: "Shared tools", installed: false },
      { organization, organization_name: organization, revision: 42, kind: "skills" as const,
        id: organization === "one" ? "Audit" : "audit", description: "Audit instructions", installed: false },
    ]),
  };
  vi.mocked(invoke).mockImplementation(async (...[command]) => {
    if (command === "catalog_install_organization_item") { await installation; return undefined as never; }
    if (command === "catalog_state") return state as never;
    if (command === "plugin_hub") return [{ id: "review", source: "/tmp/review", note: "Review commands" }] as never;
    if (command === "skill_hub") return [{ id: "release", description: "Release notes", content: "Example instructions" }] as never;
    if (command === "mcp_hub") return [
      { id: "prometeu", config: { builtin: true }, note: "" },
      { id: "notes", config: { url: "https://example.test/mcp" }, note: "Shared notes" },
    ] as never;
    if (command === "mcp_logins") return [] as never;
    if (command === "mcp_login") { await login; return undefined as never; }
    if (command === "mcp_check") return { steps: [], probe: { ok: true, auth: false, tools: 1, name: "Notes", detail: "" } } as never;
    throw new Error(`Unexpected command: ${command}`);
  });
  mcp.init({ say: vi.fn() }); plugins.init({ say: vi.fn() });
  await Promise.all([catalog.load(), mcp.refresh(), plugins.refresh(), skills.refresh()]);
  const all = [...mcp.resourceItems(), ...plugins.resourceItems(), ...skills.resourceItems()];
  expect(new Set(all.map(item => item.key)).size).toBe(all.length);
  expect(all.filter(item => item.id === "notes")).toHaveLength(1);
  expect(all.find(item => item.id === "prometeu")!.actions).toEqual([]);
  expect(all.find(item => item.id === "release")!.description).toBe("Release notes");
  expect(matchesResource(all.find(item => item.id === "review")!, { filter: "plugins", query: "review commands" }, "Plugin")).toBe(true);
  expect(matchesResource(all.find(item => item.id === "review")!, { filter: "mcp", query: "review" }, "Plugin")).toBe(false);
  const notes = all.find(item => item.key === "mcp-actions-notes")!;
  expect(notes.origins.map(origin => origin.label)).toEqual([t("catalog.thisMac"), "Personal ⇄", "one ≠", "two ≠"]);
  expect(matchesResource(notes, { filter: "mcp", query: "two" }, "MCP")).toBe(true);
  expect(notes.actions).toContainEqual(expect.objectContaining({ label: "Install from one" }));
  expect(notes.actions).toContainEqual(expect.objectContaining({ label: "Install from two" }));
  expect(notes.actions).toContainEqual(expect.objectContaining({ label: "Create local copy" }));
  expect(notes.actions).toContainEqual(expect.objectContaining({ label: "Delete from cloud", danger: true }));
  const authenticate = notes.actions.find(action => action !== "sep" && action.label === "Authenticate");
  if (!authenticate || authenticate === "sep") throw new Error("Missing authentication action");
  authenticate.run!();
  expect(mcp.resourceItems().find(item => item.key === notes.key)!.busy).toBe(true);
  await mcp.refresh();
  expect(mcp.resourceItems().find(item => item.key === notes.key)!.busy).toBe(true);
  authenticate.run!();
  expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === "mcp_login")).toHaveLength(1);
  finishLogin();
  await vi.waitFor(() => expect(mcp.resourceItems().find(item => item.key === notes.key)!.busy).toBe(false));
  expect(vi.mocked(invoke).mock.calls.some(([command]) => command === "mcp_save")).toBe(false);

  const pending = all.filter(item => item.id.toLowerCase() === "audit");
  expect(pending).toHaveLength(1);
  expect(pending[0].origins.map(origin => origin.label)).toEqual(["one", "two"]);
  const install = pending[0].actions.find(action => action !== "sep" && action.label === "Install from two");
  if (!install || install === "sep") throw new Error("Missing installation action");
  install.run!();
  await catalog.load();
  expect(skills.resourceItems().find(item => item.key === pending[0].key)!.busy).toBe(true);
  install.run!();
  const calls = vi.mocked(invoke).mock.calls.filter(([command]) => command === "catalog_install_organization_item");
  expect(calls).toEqual([["catalog_install_organization_item", { organization: "two", kind: "skills", id: "audit", revision: 42 }]]);
  finishInstall();
  await vi.waitFor(() => expect(skills.resourceItems().find(item => item.key === pending[0].key)!.busy).toBe(false));
});
