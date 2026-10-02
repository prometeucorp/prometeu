import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Board, Workspace } from "./types";
import type { TelemetryInsights } from "./telemetry";
import type { WorkspaceUsage } from "./workspace-usage";

const state = vi.hoisted(() => ({
  invoke: vi.fn(), listeners: new Map<string, () => void>(), cleared: new Set<() => void>(),
}));
const element = (text = "") => ({
  textContent: text, title: "", hidden: false, classList: { add() {} },
  replaceChildren() {}, setAttribute() {},
});
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (event: string, listener: () => void) => {
  state.listeners.set(event, listener); return () => state.listeners.delete(event);
}) }));
vi.mock("./ipc", () => ({ invoke: state.invoke }));
vi.mock("./telemetry", () => ({ onTelemetryCleared: (listener: () => void) => {
  state.cleared.add(listener); return () => state.cleared.delete(listener);
} }));
vi.mock("./util", () => ({ h: (_tag: string, _className: string, text?: string) => element(text) }));
vi.mock("./ui", () => ({ button: () => element() }));
vi.mock("./menu", () => ({ close: vi.fn() }));
vi.mock("./components/chat/usage", () => ({ openUsagePanel: vi.fn(), usagePanel: vi.fn() }));

const workspace = (id: string) => ({ id, title: id, repos: [], tabs: [] }) as unknown as Workspace;
const board: Board = { workspaces: [], stages: [], projects: [], telemetry_ids: {
  "workspace:first": "first-measurements", "workspace:second": "second-measurements",
} };
function insights(inputTokens: number): TelemetryInsights {
  return {
    summary: { firstRecordedAt: 1, lastRecordedAt: 2, events: 2, turns: 1, completedTurns: 1,
      partialTurns: 0, measuredTurns: 1, inputTokens, outputTokens: 0, costUsd: null,
      incompleteExecutions: 0, completeExecutions: 0, clockAnomalies: 0, executionSumMs: null, activeAgentMs: null,
      respondedWaits: 0, cancelledWaits: 0, incompleteWaits: 0, humanWaitMs: null, workspaceIds: [],
      health: { failures: 0, lastFailureAt: null, unavailable: false } },
    usage: { inputTokens, outputTokens: 0, cacheReadTokens: null, cacheWriteTokens: null,
      reasoningTokens: null, contextUsed: null, contextWindow: null, peakContext: null,
      modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null },
    conversations: [], models: [], sources: [], origins: [], pullRequests: [],
  };
}

let view: WorkspaceUsage;
beforeEach(async () => {
  vi.useFakeTimers(); vi.stubGlobal("window", globalThis); vi.resetModules();
  state.listeners.clear(); state.cleared.clear(); state.invoke.mockReset();
  state.invoke.mockResolvedValue(insights(100));
  view = new (await import("./workspace-usage")).WorkspaceUsage();
});
afterEach(() => { view.hide(); vi.useRealTimers(); vi.unstubAllGlobals(); });

it("refreshes the current workspace on coalesced telemetry changes without a board update", async () => {
  view.update(workspace("first"), board);
  await vi.runAllTimersAsync();
  expect(view.root.title).toContain("100 tokens");
  expect(state.listeners.has("telemetry-changed")).toBe(true);
  state.invoke.mockResolvedValue(insights(200));
  state.listeners.get("telemetry-changed")!();
  state.listeners.get("telemetry-changed")!();
  state.listeners.get("telemetry-changed")!();
  await vi.runAllTimersAsync();
  expect(state.invoke).toHaveBeenCalledTimes(2);
  expect(state.invoke).toHaveBeenLastCalledWith("telemetry_insights", { filter: { workspaceId: "first-measurements" } });
  expect(view.root.title).toContain("200 tokens");
  view.hide(); state.listeners.get("telemetry-changed")!();
  await vi.runAllTimersAsync();
  expect(state.invoke).toHaveBeenCalledTimes(2);
});

it("discards a telemetry read after navigating away and back to the same workspace", async () => {
  view.update(workspace("first"), board); await vi.runAllTimersAsync();
  let resolve!: (snapshot: TelemetryInsights) => void;
  state.invoke.mockImplementationOnce(() => new Promise<TelemetryInsights>(done => { resolve = done; }));
  state.listeners.get("telemetry-changed")!(); await vi.runAllTimersAsync();
  view.update(workspace("second"), board); await vi.runAllTimersAsync();
  state.invoke.mockResolvedValue(insights(300));
  view.update(workspace("first"), board); await vi.runAllTimersAsync();
  resolve(insights(900)); await Promise.resolve();
  expect(view.root.title).toContain("300 tokens");
  expect(view.root.title).not.toContain("900 tokens");
});

it("invalidates an event read when history is cleared", async () => {
  view.update(workspace("first"), board); await vi.runAllTimersAsync();
  let resolve!: (snapshot: TelemetryInsights) => void;
  state.invoke.mockImplementationOnce(() => new Promise<TelemetryInsights>(done => { resolve = done; }));
  state.listeners.get("telemetry-changed")!(); await vi.runAllTimersAsync();
  state.invoke.mockResolvedValue(insights(0));
  for (const listener of state.cleared) listener();
  await Promise.resolve();
  resolve(insights(900)); await Promise.resolve();
  expect(view.root.title).toContain("0 tokens");
  expect(view.root.title).not.toContain("900 tokens");
});
