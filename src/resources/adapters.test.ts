import { expect, test, vi } from "vitest";
vi.mock("../ipc", () => ({ invoke: vi.fn() }));
import { invoke } from "../ipc";
import * as mcp from "../mcp";
import * as plugins from "../plugins";
import * as skills from "../skills";
import * as catalog from "../catalog";
import { matchesResource } from "./model";

// No DOM environment: constructing resource snapshots must not construct hidden controls.
test("hub snapshots preserve resource scope, allowed operations and pending MCP locks without DOM", async () => {
  let finishLogin!: () => void;
  const login = new Promise<void>(resolve => { finishLogin = resolve; });
  const state: catalog.CatalogState = {
    connected: true, revision: 42, plugins: [], mcp: ["notes"], skills: [], shared: { "mcp:notes": "notes" },
    organization_items: ["one", "two"].map(organization => ({ organization, organization_name: organization,
      revision: 42, kind: "mcp", id: "notes", description: "Shared tools", installed: false })),
  };
  vi.mocked(invoke).mockImplementation(async (...[command]) => {
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
  expect(all.filter(item => item.id === "notes")).toHaveLength(3);
  expect(all.find(item => item.id === "prometeu")!.actions).toEqual([]);
  expect(all.find(item => item.id === "release")!.description).toBe("Release notes");
  expect(matchesResource(all.find(item => item.id === "review")!, { filter: "plugins", query: "review commands" }, "Plugin")).toBe(true);
  expect(matchesResource(all.find(item => item.id === "review")!, { filter: "mcp", query: "review" }, "Plugin")).toBe(false);
  const notes = all.find(item => item.key === "mcp-actions-notes")!;
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
});
