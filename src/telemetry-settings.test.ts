import { afterEach, beforeEach, expect, test, vi } from "vitest";

const state = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn(), confirm: vi.fn(), update: vi.fn(), clear: vi.fn(), destroy: vi.fn(),
  controls: [] as { value: string; options: [string, string][]; onchange: () => void }[],
  inputs: [] as { value: string; onchange: () => void }[], actions: new Map<string, () => unknown>(),
  listeners: new Map<string, () => void>(), unlisten: vi.fn(),
}));
const element = () => ({ dataset: {}, append() {}, replaceChildren() {}, setAttribute() {}, removeAttribute() {},
  classList: { add() {} }, textContent: "", hidden: false, isConnected: true, closest: () => null, remove() {} });
vi.mock("./util", () => ({ h: () => element() }));
vi.mock("./i18n", () => ({ t: (key: string) => key }));
vi.mock("./ipc", () => ({ invoke: state.invoke }));
vi.mock("./agents", () => ({ descriptors: () => [] }));
vi.mock("./usage-history", () => ({ usageHistoryData: (insights: unknown) => insights }));
vi.mock("./components/usage-history", () => ({ usageHistoryView: () => ({ root: element(), update: state.update, clear: state.clear, destroy: state.destroy }) }));
vi.mock("./components/compositions", () => ({ sectionHeader: () => element() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: state.save }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name: string, run: () => void) => { state.listeners.set(name, run); return state.unlisten; }) }));
vi.mock("./ui", () => ({
  input: (value: string) => { const input = { ...element(), value, onchange() {} }; state.inputs.push(input); return input; },
  field: () => element(), confirmDialog: state.confirm,
  button: (label: string, run: () => unknown) => { state.actions.set(label, run); return element(); },
  select: (value: string, options: [string, string][]) => {
    const control = { value, options, control: element(), onchange() {},
      setOptions(next: [string, string][], selected?: string) { control.options = next; if (selected !== undefined) control.value = selected; },
    };
    state.controls.push(control); return control;
  },
}));
let controller: typeof import("./telemetry-settings");
const insight = (marker: string, ids: string[] = []) => ({ marker, summary: { workspaceIds: ids, health: { unavailable: false, failures: 0 } } });
const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };
beforeEach(async () => {
  vi.resetModules(); vi.clearAllMocks(); state.controls.length = 0; state.inputs.length = 0; state.actions.clear(); state.listeners.clear();
  state.invoke.mockImplementation(async command => command === "load_board" ? { workspaces: [] } : insight("first"));
  controller = await import("./telemetry-settings");
});
afterEach(() => { controller.disposeTelemetry(); vi.useRealTimers(); });

test("refresh preserves workspace selection and resolves renamed and removed workspaces", async () => {
  let ids = ["workspace-one"], title = "First workspace";
  state.invoke.mockImplementation(async command => command === "load_board" ? { workspaces: ids.map(id => ({ id, title })) } : insight("current", ids));
  controller.telemetryRows(); await settle();
  const workspace = state.controls[0];
  expect(workspace.options).toContainEqual(["workspace-one", "First workspace"]);
  workspace.value = "workspace-one"; workspace.onchange(); await settle();
  expect(state.invoke).toHaveBeenCalledWith("telemetry_insights", { filter: { workspaceId: "workspace-one" } });
  ids = ["workspace-two"]; title = "Updated workspace";
  await state.actions.get("telemetry.refresh")!(); await settle();
  expect(workspace.value).toBe("workspace-one");
  expect(workspace.options).toContainEqual(["workspace-one", "telemetry.missingWorkspace"]);
  expect(workspace.options).toContainEqual(["workspace-two", "Updated workspace"]);
  expect(state.invoke).toHaveBeenCalledWith("analytics_track", { event: { name: "usage_history_action_used", action: "filter" } });
});

test("live updates coalesce and stale filter reads cannot replace newer results", async () => {
  vi.useFakeTimers(); controller.telemetryRows(); await settle();
  const pending: ((value: unknown) => void)[] = [];
  state.invoke.mockImplementation(command => command === "load_board" ? Promise.resolve({ workspaces: [] }) : command === "telemetry_insights" ? new Promise(resolve => pending.push(resolve)) : Promise.resolve());
  state.listeners.get("telemetry-changed")!(); state.listeners.get("telemetry-changed")!();
  await vi.advanceTimersByTimeAsync(150); expect(pending).toHaveLength(1);
  state.controls[0].value = "other"; state.controls[0].onchange();
  pending[1](insight("new")); await settle(); pending[0](insight("stale")); await settle();
  expect(state.update).toHaveBeenLastCalledWith(expect.objectContaining({ marker: "new" }));
  controller.disposeTelemetry(); state.listeners.get("telemetry-changed")!();
  await vi.runAllTimersAsync(); expect(pending).toHaveLength(2); expect(state.unlisten).toHaveBeenCalled();
});

test("Settings redraws reuse the same view and subscriptions until leaving the section", async () => {
  vi.useFakeTimers();
  const first = controller.telemetryRows()[0]; await settle();
  const next = controller.telemetryRows()[0];
  expect(next).toBe(first); expect(state.destroy).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(150);
  expect(state.update).toHaveBeenCalledTimes(2);
  controller.disposeTelemetry();
  expect(state.destroy).toHaveBeenCalledTimes(1);
  expect(state.unlisten).toHaveBeenCalledTimes(1);
});

test("deletion clears open details and invalidates reads and pending export dialogs", async () => {
  controller.telemetryRows(); await settle();
  let read!: (value: unknown) => void, path!: (value: string) => void;
  state.invoke.mockImplementation(command => command === "load_board" ? Promise.resolve({ workspaces: [] }) : command === "telemetry_insights" ? new Promise(resolve => { read = resolve; }) : Promise.resolve());
  state.save.mockImplementation(() => new Promise(resolve => { path = resolve; }));
  const exporting = state.actions.get("telemetry.export")!();
  const refreshing = state.actions.get("telemetry.refresh")!();
  const stale = read;
  const { telemetryCleared } = await import("./telemetry"); telemetryCleared();
  read(insight("empty")); await settle(); stale(insight("erased")); path("history.jsonl");
  await Promise.all([exporting, refreshing]); await settle();
  expect(state.clear).toHaveBeenCalled();
  expect(state.update).toHaveBeenLastCalledWith(expect.objectContaining({ marker: "empty" }));
  expect(state.invoke).not.toHaveBeenCalledWith("telemetry_export", expect.anything());
});

test("invalid periods and unavailable storage hide old values instead of reporting zero", async () => {
  controller.telemetryRows(); await settle();
  state.inputs[0].value = "2026-11-02"; state.inputs[1].value = "2026-11-01";
  state.inputs[0].onchange(); await settle(); expect(state.clear).toHaveBeenCalled();
  state.invoke.mockImplementation(async command => command === "load_board" ? { workspaces: [] } : { summary: { workspaceIds: [], health: { unavailable: true } } });
  state.update.mockClear(); state.inputs[0].value = ""; state.inputs[1].value = "";
  await state.actions.get("telemetry.refresh")!();
  expect(state.update).not.toHaveBeenCalled();
});
