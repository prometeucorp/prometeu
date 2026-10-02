import { beforeEach, expect, test, vi } from "vitest";
import { telemetryPeriod } from "./telemetry";
import * as mock from "./mock-telemetry";

beforeEach(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) });
});
test("local date filters use an exclusive next-day boundary", () => {
  const period = telemetryPeriod("2026-09-24", "2026-09-25");
  expect(new Date(period.from!).getDate()).toBe(24);
  expect(new Date(period.to!).getDate()).toBe(26);
  expect(new Date(period.to!).getHours()).toBe(0);
  expect(() => telemetryPeriod("2026-09-26", "2026-09-24")).toThrow("err.telemetry.filter");
  expect(telemetryPeriod("", "")).toEqual({});
});
test("mock summary uses the turn start cohort and export matches event filters", () => {
  const first = mock.page({}).events[0];
  const filter = { from: first.occurredAt, to: first.occurredAt + 1 };
  expect(mock.summary(filter)).toMatchObject({ turns: 1, inputTokens: 10000, outputTokens: 2000 });
  expect(mock.summary({ workspaceId: "missing" }).turns).toBe(0);
  const lines = mock.exportData(filter).trim().split("\n").map(line => JSON.parse(line));
  expect(lines[0].exportVersion).toBe(1); expect(lines).toHaveLength(2);
  expect(mock.page({}, { occurredAt: first.occurredAt, sequence: first.sequence }).events).toHaveLength(1);
});
test("mock clear removes retained history and does not regenerate fixture data", () => {
  expect(mock.summary({}).turns).toBe(1); mock.clear();
  expect(mock.summary({})).toMatchObject({ events: 0, turns: 0, inputTokens: null, outputTokens: null });
  expect(mock.page({}).events).toEqual([]);
  expect(mock.exportData({}).trim().split("\n")).toHaveLength(1);
});

test("mock insights count a final measurement once and keep token subsets separate", () => {
  const records = mock.page({}).events;
  const last = records[1];
  if (last.type !== "turn.completed") throw new Error("Expected a completed fixture turn");
  last.payload.measurement.usageByModel = [{ model: "observed-model", usage: last.payload.measurement.usage }];
  localStorage.setItem("mock:telemetry", JSON.stringify([
    records[0], { ...last, sequence: 2, type: "turn.usage.observed", payload: { measurement: last.payload.measurement } },
    { ...last, sequence: 3 },
  ]));
  const result = mock.insights({});
  expect(result.usage).toMatchObject({ inputTokens: 10000, outputTokens: 2000, cacheReadTokens: 8000, reasoningTokens: 500, costUsd: null, contextUsed: null });
  expect(result.conversations).toHaveLength(1);
  expect(result.models.map(model => model.id)).toEqual(["observed-model"]);
  expect(result.sources).toMatchObject([{ id: "conversation", turns: 1 }]);
  mock.clear();
  expect(mock.insights({}).usage.inputTokens).toBeNull();
});

test("mock reply lookup uses conversation identity and is erased with history", async () => {
  const records = mock.page({}).events;
  const last = records[1];
  if (last.type !== "turn.completed") throw new Error("Expected a completed fixture turn");
  const conversation = last.conversationId!;
  const bytes = new TextEncoder().encode(`${conversation}\0reply-one`);
  const hash = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map(byte => byte.toString(16).padStart(2, "0")).join("");
  localStorage.setItem("mock:telemetry", JSON.stringify([records[0], { ...last, payload: { ...last.payload, messageKey: hash } }]));
  expect(await mock.turns(conversation, ["missing", "reply-one", "reply-one"])).toMatchObject([{ messageId: "reply-one", durationMs: 30000 }]);
  expect(await mock.turns("another-conversation", ["reply-one"])).toEqual([]);
  mock.clear();
  expect(await mock.turns(conversation, ["reply-one"])).toEqual([]);
  await expect(mock.turns(conversation, Array(501).fill("reply-one"))).rejects.toThrow("err.telemetry.filter");
});

test("mock live capture is idempotent and deletion suppresses pending capture", async () => {
  mock.clear();
  const scope = { workspaceId: "workspace", projectId: "project", conversationId: "conversation", provider: "claude" };
  const measurement = mock.sampleUsage("claude");
  await mock.recordTurn(scope, "reply", 1000, measurement);
  await mock.recordTurn(scope, "reply", 1000, measurement);
  expect(mock.insights({ workspaceId: "workspace" })).toMatchObject({ summary: { turns: 1 }, usage: { inputTokens: 24000, costUsd: 0.12 } });
  const pending = mock.recordTurn(scope, "late-reply", 1000, measurement);
  mock.clear();
  await pending;
  expect(mock.page({}).events).toEqual([]);
});

test("mock insights preserve partial observations and separate app-call consumption", () => {
  const records = mock.page({}).events;
  const last = records[1];
  if (last.type !== "turn.completed") throw new Error("Expected a completed fixture turn");
  const measurement = { ...last.payload.measurement, complete: false, usage: { ...last.payload.measurement.usage, outputTokens: null } };
  localStorage.setItem("mock:telemetry", JSON.stringify([records[0],
    { ...last, type: "turn.usage.observed", payload: { measurement } },
    { ...last, id: "app-start", type: "app.call.started", conversationId: null, turnId: null,
      payload: { callId: "app", source: "naming", measurement } },
    { ...last, id: "app-end", type: "app.call.completed", conversationId: null, turnId: null,
      payload: { callId: "app", source: "naming", outcome: "ok", elapsedMs: 100, measurement } },
  ]));
  const result = mock.insights({});
  expect(result.summary).toMatchObject({ turns: 1, completedTurns: 0, partialTurns: 1, measuredTurns: 0, inputTokens: 10000, outputTokens: null });
  expect(result.usage).toMatchObject({ inputTokens: 20000, outputTokens: null });
  expect(result.conversations).toHaveLength(1);
  expect(result.sources.map(source => [source.id, source.turns])).toEqual([["conversation", 1], ["naming", 1]]);
});
