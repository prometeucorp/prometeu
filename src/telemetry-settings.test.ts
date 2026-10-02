import { expect, test, vi } from "vitest";

const state = vi.hoisted(() => ({
  invoke: vi.fn(),
  controls: [] as { value: string; options: [string, string][]; onchange: () => void }[],
  actions: new Map<string, () => unknown>(),
  labels: [] as string[],
}));
const element = () => ({ append() {}, replaceChildren() {}, setAttribute() {}, textContent: "" });
vi.mock("./util", () => ({ h: (_tag: string, _cls: string, text?: string) => { if (text) state.labels.push(text); return element(); }, took: (ms: number) => String(ms) }));
vi.mock("./i18n", () => ({ t: (key: string) => key, current: () => "en", tn: (key: string) => key }));
vi.mock("./ipc", () => ({ invoke: state.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
vi.mock("./ui", () => ({
  input: (value: string) => ({ value }), field: () => element(), confirmDialog: vi.fn(),
  button: (label: string, run: () => unknown) => { state.actions.set(label, run); return element(); },
  select: (value: string, options: [string, string][]) => {
    const control = { value, options, control: element(), onchange() {},
      setOptions(next: [string, string][], selected?: string): void { control.options = next; if (selected !== undefined) control.value = selected; },
    };
    state.controls.push(control); return control;
  },
}));
import { telemetryRows } from "./telemetry-settings";

test("refresh updates workspace choices and names while preserving the selected filter", async () => {
  let ids = ["workspace-one"];
  let title = "First workspace";
  state.invoke.mockImplementation(async (command, args) => {
    if (command === "load_board") return { workspaces: ids.map(id => ({ id, title })) } as never;
    if (command === "telemetry_insights") return { summary: { workspaceIds: ids, health: {}, firstRecordedAt: null, lastRecordedAt: null },
      usage: { inputTokens: 200, outputTokens: 100, costUsd: null }, sources: [{ id: "plugin-maker", turns: 1, usage: { inputTokens: 200, outputTokens: 100, costUsd: null } }], origins: [] } as never;
    throw new Error(`Unexpected command: ${command} ${JSON.stringify(args)}`);
  });
  telemetryRows();
  const workspace = state.controls[0];
  await vi.waitFor(() => expect(workspace.options).toContainEqual(["workspace-one", "First workspace"]));
  workspace.value = "workspace-one";
  workspace.onchange();
  await vi.waitFor(() => expect(state.invoke).toHaveBeenCalledWith("telemetry_insights", { filter: { workspaceId: "workspace-one" } }));
  ids = ["workspace-one", "workspace-two"];
  title = "Updated workspace";
  await state.actions.get("telemetry.refresh")!();
  await vi.waitFor(() => expect(workspace.options).toContainEqual(["workspace-two", "Updated workspace"]));
  expect(workspace.value).toBe("workspace-one");
  expect(workspace.options).toContainEqual(["workspace-one", "Updated workspace"]);
  expect(state.labels).toContain("usage.source.pluginMaker");
});
