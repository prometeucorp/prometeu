import { expect, test } from "vitest";
import { knownTokens, usageHistoryData } from "./usage-history";
import type { TelemetryInsights, TelemetryUsage } from "./telemetry";
import type { Board } from "./types";

const usage = (values: Partial<TelemetryUsage> = {}): TelemetryUsage => ({ inputTokens: null, outputTokens: null, cacheReadTokens: null,
  cacheWriteTokens: null, reasoningTokens: null, contextUsed: null, contextWindow: null, peakContext: null, modelCalls: null,
  compactions: null, cacheRebuilds: null, costUsd: null, ...values });
const board: Board = { stages: [], projects: [], workspaces: [] };
const snapshot = (): TelemetryInsights => ({ summary: { firstRecordedAt: null, lastRecordedAt: null, events: 0, turns: 3, completedTurns: 2,
  partialTurns: 1, measuredTurns: 1, inputTokens: 100, outputTokens: 20, costUsd: null, incompleteExecutions: 0, completeExecutions: 0, clockAnomalies: 0,
  executionSumMs: null, activeAgentMs: null, respondedWaits: 0, cancelledWaits: 0, incompleteWaits: 0, humanWaitMs: null, workspaceIds: [],
  health: { unavailable: false, failures: 0, lastFailureAt: null } }, usage: usage({ inputTokens: 104, outputTokens: 22, cacheReadTokens: 80, reasoningTokens: 10 }),
  conversations: [], sources: [], models: [], origins: [], pullRequests: [] });

test("known consumption includes partial sides once and preserves unknown versus zero", () => {
  expect(knownTokens(usage())).toBeNull();
  expect(knownTokens(usage({ inputTokens: 0, outputTokens: 0 }))).toBe(0);
  expect(knownTokens(usage({ inputTokens: 100, outputTokens: 20, cacheReadTokens: 80, reasoningTokens: 10 }))).toBe(120);
  expect(knownTokens(usage({ outputTokens: 20 }))).toBe(20);
  const result = usageHistoryData(snapshot(), board, new Map());
  expect(result.metrics[0].value).toBe("126");
  expect(result.metrics[1].value).toBe("—");
});

test("sessions rank known usage and keep coverage independent of aggregate token availability", () => {
  const insight = snapshot();
  insight.conversations = [
    { id: "unknown", provider: null, turns: 1, usage: usage() },
    { id: "partial", provider: "codex", turns: 2, usage: usage({ inputTokens: 100, outputTokens: 20 }),
      coverage: { measuredTurns: 1, partialTurns: 0, unknownTurns: 1, mainAgentTurns: 1, wholeTreeTurns: 1 } },
    { id: "zero", provider: "retired-provider", turns: 1, usage: usage({ inputTokens: 0, outputTokens: 0, costUsd: 0 }),
      coverage: { measuredTurns: 1, partialTurns: 0, unknownTurns: 0, mainAgentTurns: 1, wholeTreeTurns: 0 } },
    { id: "legacy", provider: "claude", turns: 1, usage: usage({ inputTokens: 1, outputTokens: 1 }) },
  ];
  const result = usageHistoryData(insight, board, new Map([["codex", "Codex"]]));
  expect(result.sessions.map(row => row.id)).toEqual(["partial", "legacy", "zero", "unknown"]);
  expect(result.sessions[0]).toMatchObject({ status: "partial", tokens: "120", provider: "Codex", title: "Conversation partial" });
  expect(result.sessions[0].notes).toContain("Measurements have mixed main-agent and whole-tree scopes.");
  expect(result.sessions[1].notes).toContain("Measurement coverage and scope are unavailable for this session.");
  expect(result.sessions[2]).toMatchObject({ status: "complete", tokens: "0", cost: "~$0.00", provider: "retired-provider" });
  expect(result.sessions[3].status).toBe("unknown");
});

test("sources partition totals while model and origin views are not added again", () => {
  const insight = snapshot();
  insight.sources = [{ id: "conversation", provider: "claude", turns: 1, usage: usage({ inputTokens: 100, outputTokens: 20 }) },
    { id: "naming", provider: "claude", turns: 1, usage: usage({ inputTokens: 4, outputTokens: 2 }) }];
  insight.origins = [{ ...insight.sources[0], id: "action" }, { ...insight.sources[0], id: "delegation" }];
  insight.models = [{ id: "observed-model", provider: "claude", turns: 1, usage: usage({ inputTokens: 100 }) }];
  const result = usageHistoryData(insight, board, new Map([["claude", "Claude"]]));
  expect(result.sources.map(row => row.value)).toEqual(["120", "6"]);
  expect(result.sources.reduce((sum, row) => sum + row.share!, 0)).toBeCloseTo(100);
  expect(result.models[0]).toEqual({ label: "observed-model · Claude", value: "Input: 100 · Output: —" });
  expect(result.metrics[0].value).toBe("126");
});
