import { describe, expect, it } from "vitest";
import { contextBand, contextForGauge, measuredTokens, visibleUsage, replayIsCurrent, workspaceUsageData, turnUsageText } from "./usage-presentation";
import type { TelemetryUsage, TelemetryInsights } from "./telemetry";
import { t } from "./i18n";

const usage = (values: Partial<TelemetryUsage> = {}): TelemetryUsage => ({
  inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null,
  reasoningTokens: null, contextUsed: null, contextWindow: null, peakContext: null,
  modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null, ...values,
});

describe("usage presentation", () => {
  it("counts inclusive input and output once without adding their subsets", () => {
    expect(measuredTokens(usage({ inputTokens: 100, outputTokens: 20, cacheReadTokens: 80, cacheWriteTokens: 10, reasoningTokens: 15 }))).toBe(120);
    expect(measuredTokens(usage({ inputTokens: 100 }))).toBeNull();
    expect(measuredTokens(usage({ inputTokens: 0, outputTokens: 0 }))).toBe(0);
  });

  it("hides unsupported measurements without hiding measured zero", () => {
    const value = usage({ inputTokens: 0, outputTokens: 0, costUsd: 0 });
    expect(visibleUsage(value, { usageTokens: true, usageCost: true })).toEqual({ tokens: 0, cost: 0 });
    expect(visibleUsage(value, { usageTokens: false, usageCost: false })).toEqual({ tokens: null, cost: null });
    expect(visibleUsage(usage(), { usageTokens: true, usageCost: true })).toEqual({ tokens: null, cost: null });
  });

  it("shows a cache-rebuild signal only when observed and explains partial model coverage", () => {
    const measurement = { usageScope: "wholeTree" as const, complete: true, selectedModel: null, observedModels: ["model"],
      usage: usage({ inputTokens: 100, outputTokens: 20, cacheRebuilds: 1 }), usageByModel: [{ model: "model", usage: usage({ inputTokens: 50 }) }] };
    const shown = turnUsageText({ durationMs: 1000, usage: measurement }, { usageTokens: true, usageCost: true });
    expect(shown.label).toContain("Cache rebuilds: 1");
    expect(shown.title).toContain("Usage including child agents");
    expect(shown.title).toContain("model (Partial measurement)");
    measurement.usage.cacheRebuilds = 0;
    expect(turnUsageText({ durationMs: 1000, usage: measurement }, { usageTokens: true, usageCost: true }).label).not.toContain("Cache rebuilds");
  });

  it.each([
    [119999, 200000, "normal"], [120000, 200000, "warn"], [160000, 200000, "hot"],
    [220000, 1000000, "warn"], [800000, 1000000, "hot"], [0, 200000, "normal"],
    [100, null, null], [100, 0, null], [-1, 100, null], [NaN, 100, null],
  ] as const)("bands context %s / %s as %s", (used, window, wanted) => {
    expect(contextBand(used, window)).toBe(wanted);
  });

  it("rejects a replay read after navigation away and back to the same conversation", () => {
    expect(replayIsCurrent({ key: "chat", version: 2 }, { key: "chat", version: 2 }, false)).toBe(true);
    expect(replayIsCurrent({ key: "chat", version: 2 }, { key: "other", version: 2 }, false)).toBe(false);
    expect(replayIsCurrent({ key: "chat", version: 2 }, { key: "chat", version: 4 }, false)).toBe(false);
    expect(replayIsCurrent({ key: "chat", version: 2 }, { key: "chat", version: 2 }, true)).toBe(false);
  });

  it("keeps replayed post-compaction occupancy with the persisted context capacity", () => {
    const persisted = { used: 30000, window: 200000 };
    expect(contextForGauge({ used: 2000, window: null }, persisted)).toEqual({ used: 2000, window: 200000 });
    expect(contextForGauge({ used: 4000, window: 100000 }, persisted)).toEqual({ used: 4000, window: 100000 });
    expect(contextForGauge(null, persisted)).toEqual(persisted);
    expect(contextForGauge({ used: 2000, window: null }, null)).toEqual({ used: 2000, window: null });
    expect(contextForGauge(null, undefined)).toBeNull();
  });

  it("shows a workspace total once with related PRs and explicit partial groups", () => {
    const measured = usage({ inputTokens: 100, outputTokens: 20, cacheReadTokens: 80 });
    const insight: TelemetryInsights = {
      summary: { firstRecordedAt: 1, lastRecordedAt: 2, events: 3, turns: 2, completedTurns: 2,
        partialTurns: 1, measuredTurns: 1, inputTokens: 100, outputTokens: 20, costUsd: null,
        incompleteExecutions: 0, completeExecutions: 1, clockAnomalies: 0, executionSumMs: 60000, activeAgentMs: 30000,
        respondedWaits: 1, cancelledWaits: 0, incompleteWaits: 0, humanWaitMs: 10000, workspaceIds: ["workspace"],
        health: { failures: 0, lastFailureAt: null, unavailable: false } },
      usage: measured,
      conversations: [{ id: "present", provider: null, turns: 1, usage: measured }, { id: "missing", provider: null, turns: 1, usage: usage({ inputTokens: 30 }) }],
      models: [{ id: "observed-model", provider: null, turns: 2, usage: measured }],
      sources: [{ id: "naming", provider: null, turns: 1, usage: usage({ inputTokens: 40, costUsd: 0.01 }) }],
      origins: [{ id: "action", provider: null, turns: 1, usage: measured }],
      pullRequests: [1, 2].map(pullRequest => ({ repositoryId: `repo-${pullRequest}`, branchId: null, pullRequest, turns: null, usage: usage(), attribution: "related" as const })),
    };
    const data = workspaceUsageData(insight, { conversations: new Map([["present", "Review"]]), repositories: new Map([["repo-1", "Frontend"]]) });
    expect(data.metrics?.find(row => row.label === "Input served from cache")?.value).toBe("80%");
    expect(data.groups?.find(group => group.title === "Conversations")?.rows).toEqual([
      { label: "Review", value: "120 tokens", detail: "1 turn" },
      { label: "Conversation missing", value: "Partial measurement", detail: "1 turn · Known input tokens: 30" },
    ]);
    expect(data.groups?.find(group => group.title === "Related pull requests")?.rows.map(row => row.value)).toEqual(["", ""]);
    expect(data.metrics?.find(row => row.label === "Known input tokens")?.value).toBe("100");
    expect(data.groups?.find(group => group.title === t("usage.automations"))?.rows[0].value).toBe("120 tokens");
    expect(data.groups?.find(group => group.title === t("usage.sources"))?.rows[0]).toEqual({
      label: t("usage.source.naming"), value: "Partial measurement · ~$0.01", detail: "Known input tokens: 40 · CLI-reported cost: ~$0.01",
    });
    const partial = workspaceUsageData({ ...insight, usage: { ...measured, cacheReadTokens: 200 } }, { conversations: new Map(), repositories: new Map() });
    expect(partial.metrics?.some(row => row.label === t("usage.cacheShare"))).toBe(false);

    const providers = workspaceUsageData({ ...insight, models: [
      { id: "shared-model", provider: "claude", turns: 1, usage: measured },
      { id: "shared-model", provider: "codex", turns: 1, usage: usage({ inputTokens: 50, outputTokens: 10 }) },
      { id: "historical-model", provider: "old-provider", turns: 1, usage: measured },
      { id: "unattributed-model", provider: null, turns: 1, usage: measured },
    ] }, { conversations: new Map(), repositories: new Map(), providers: new Map([["claude", "Claude"], ["codex", "Codex"]]) });
    expect(providers.groups?.find(group => group.title === t("usage.models"))?.rows.map(({ label, value }) => ({ label, value }))).toEqual([
      { label: "shared-model · Claude", value: "120 tokens" },
      { label: "shared-model · Codex", value: "60 tokens" },
      { label: "historical-model · old-provider", value: "120 tokens" },
      { label: "unattributed-model", value: "120 tokens" },
    ]);
  });
});
