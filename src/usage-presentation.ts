import type { TelemetryInsights, TelemetryMeasurement, TelemetryUsage, UsageGroup } from "./telemetry";
import { current as locale, t, tn, type Key } from "./i18n";
import { kilo } from "./context";
import { took } from "./chat-presentation";
import type { TurnMetadata } from "./timeline";
import type { UsagePanel } from "./components/chat/usage";

/** Input and output already include cache and reasoning subsets. */
export function measuredTokens(usage: TelemetryUsage): number | null {
  return usage.inputTokens === null || usage.outputTokens === null ? null : usage.inputTokens + usage.outputTokens;
}

export function visibleUsage(usage: TelemetryUsage, capabilities: { usageTokens: boolean; usageCost: boolean }) {
  return { tokens: capabilities.usageTokens ? measuredTokens(usage) : null, cost: capabilities.usageCost ? usage.costUsd : null };
}

/** These advisory thresholds never trigger a conversation action. */
export function contextBand(used: number, window: number | null): "normal" | "warn" | "hot" | null {
  if (!Number.isFinite(used) || used < 0 || window === null || !Number.isFinite(window) || window <= 0) return null;
  if (used >= window * 0.8) return "hot";
  return used >= Math.min(window * 0.6, 220000) ? "warn" : "normal";
}

/** Native replay can preserve occupancy while the current tab retains the reported capacity. */
export function contextForGauge(observed: { used: number; window: number | null } | null, persisted: { used: number; window: number | null } | null | undefined) {
  return observed ? { used: observed.used, window: observed.window ?? persisted?.window ?? null } : persisted ?? null;
}

/** A key alone cannot distinguish leaving and returning while a read is in flight. */
export function replayIsCurrent(start: { key: string; version: number }, current: { key: string | null; version: number }, disposed: boolean): boolean {
  return !disposed && start.key === current.key && start.version === current.version;
}

export function costLabel(cost: number): string {
  return `~${new Intl.NumberFormat(locale(), { style: "currency", currency: "USD", maximumFractionDigits: cost > 0 && cost < 0.01 ? 4 : 2 }).format(cost)}`;
}

export function usageLabel(usage: TelemetryUsage, capabilities = { usageTokens: true, usageCost: true }): string {
  const { tokens, cost } = visibleUsage(usage, capabilities);
  return [tokens === null ? "" : t("usage.tokens", { n: kilo(tokens) }), cost === null ? "" : costLabel(cost)].filter(Boolean).join(" · ");
}

/** The breakdown never synthesizes missing components or adds subsets to totals. */
export function usageMetrics(usage: TelemetryUsage, capabilities = { usageTokens: true, usageCost: true }): { label: string; value: string }[] {
  const rows: { label: string; value: string }[] = [];
  const fields: [keyof TelemetryUsage, Key][] = [
    ["inputTokens", "telemetry.input"], ["outputTokens", "telemetry.output"], ["cacheReadTokens", "usage.cacheRead"],
    ["cacheWriteTokens", "usage.cacheWrite"], ["reasoningTokens", "usage.reasoning"],
  ];
  if (capabilities.usageTokens) for (const [field, label] of fields) {
    const value = usage[field];
    if (value !== null) rows.push({ label: t(label), value: value.toLocaleString(locale()) });
  }
  if (capabilities.usageCost && usage.costUsd !== null) rows.push({ label: t("usage.cost"), value: costLabel(usage.costUsd) });
  return rows;
}

export function usageTooltip(measurement: TelemetryMeasurement, capabilities = { usageTokens: true, usageCost: true }): string {
  const lines = [t(measurement.usageScope === "wholeTree" ? "usage.wholeTree" : "usage.mainAgent")];
  if (!measurement.complete) lines.push(t("usage.partial"));
  lines.push(...usageMetrics(measurement.usage, capabilities).map(row => `${row.label}: ${row.value}`));
  for (const row of measurement.usageByModel ?? []) {
    const detail = usageMetrics(row.usage, capabilities).map(metric => `${metric.label}: ${metric.value}`).join(" · ");
    if (detail) lines.push(`${row.model}${measuredTokens(row.usage) === null ? ` (${t("usage.partial")})` : ""} — ${detail}`);
  }
  if (capabilities.usageCost && (measurement.usage.costUsd !== null || measurement.usageByModel?.some(row => row.usage.costUsd !== null))) lines.push(t("usage.costHint"));
  if (measurement.usage.cacheRebuilds) lines.push(`${t("usage.rebuilds")}: ${measurement.usage.cacheRebuilds}`);
  if (measurement.usage.compactions) lines.push(`${t("usage.compactions")}: ${measurement.usage.compactions}`);
  return lines.join("\n");
}

export function workspaceUsageLabel(insights: TelemetryInsights): string {
  return [usageLabel(insights.usage), tn(insights.summary.turns, "usage.turns"),
    tn(new Set(insights.conversations.map(row => row.id)).size, "usage.conversationCount")].filter(Boolean).join(" · ");
}

/** Names remain in the current board, outside the private measurement store. */
export function workspaceUsageData(insights: TelemetryInsights, names: { conversations: Map<string, string>; repositories: Map<string, string>; providers?: ReadonlyMap<string, string> }): UsagePanel {
  const { summary, usage } = insights;
  const metrics = usageMetrics(usage);
  if (usage.inputTokens !== null && usage.inputTokens > 0 && usage.cacheReadTokens !== null && usage.cacheReadTokens <= usage.inputTokens) {
    metrics.push({ label: t("usage.cacheShare"), value: `${Math.round(usage.cacheReadTokens / usage.inputTokens * 100)}%` });
  }
  metrics.push({ label: t("telemetry.turns"), value: String(summary.turns) });
  for (const [key, value] of [["usage.agentTime", summary.executionSumMs], ["usage.active", summary.activeAgentMs], ["usage.wait", summary.humanWaitMs]] as const) {
    if (value !== null) metrics.push({ label: t(key), value: took(value) });
  }
  for (const [key, value] of [["usage.rebuilds", usage.cacheRebuilds], ["usage.compactions", usage.compactions]] as const) {
    if (value !== null) metrics.push({ label: t(key), value: String(value) });
  }
  const amount = (usage: TelemetryUsage, detail = "") => {
    const partial = measuredTokens(usage) === null;
    const known = partial ? usageMetrics(usage).map(metric => `${metric.label}: ${metric.value}`) : [];
    return { value: [partial ? t("usage.partial") : "", usageLabel(usage)].filter(Boolean).join(" · "),
      detail: [detail, ...known].filter(Boolean).join(" · ") };
  };
  const row = (group: UsageGroup, label: string) => ({ label, ...amount(group.usage, tn(group.turns, "usage.turns")) });
  const sourceLabel = (source: string) => t(source === "naming" ? "usage.source.naming" : source === "plugin-maker" ? "usage.source.pluginMaker"
    : source === "action" ? "usage.source.action" : source === "delegation" ? "usage.source.delegation" : "usage.source.conversation");
  const notes = [summary.events ? t("usage.known") : t("usage.noHistory"), t("usage.coverage", { measured: summary.measuredTurns, turns: summary.turns, partial: summary.partialTurns })];
  if (usage.costUsd !== null) notes.push(t("usage.costHint"));
  if (summary.health.failures) notes.push(t("telemetry.incompleteHistory"));
  if (summary.incompleteExecutions) notes.push(`${t("telemetry.incomplete")}: ${summary.incompleteExecutions}`);
  return {
    title: t("usage.title"), description: notes.join(" "), metrics,
    groups: [
      { title: t("usage.pullRequests"), description: t("usage.prRelated"), rows: insights.pullRequests.map(pr => ({
        label: `${names.repositories.get(pr.repositoryId) ?? t("usage.repositoryMissing", { id: pr.repositoryId.slice(0, 8) })} · ${t("usage.pr", { n: pr.pullRequest })}`,
        ...(pr.attribution === "tenure" ? amount(pr.usage, t("usage.prTenure")) : { value: "" }),
      })) },
      { title: t("usage.models"), rows: insights.models.map(group => row(group,
        group.provider ? `${group.id} · ${names.providers?.get(group.provider) ?? group.provider}` : group.id)) },
      { title: t("usage.conversations"), rows: insights.conversations.map(group => row(group, names.conversations.get(group.id) ?? t("usage.conversationMissing", { id: group.id.slice(0, 8) }))) },
      { title: t("usage.sources"), rows: insights.sources.filter(group => group.id !== "conversation").map(group => ({ label: sourceLabel(group.id), ...amount(group.usage) })) },
      { title: t("usage.automations"), description: t("usage.originsHint"), rows: insights.origins.map(group => row(group, sourceLabel(group.id))) },
    ],
  };
}

export function turnUsageText(turn: TurnMetadata, capabilities: { usageTokens: boolean; usageCost: boolean }) {
  const duration = turn.durationMs !== null && turn.durationMs >= 100 ? took(turn.durationMs) : "";
  const rebuilds = turn.usage?.usage.cacheRebuilds;
  const signal = capabilities.usageTokens && rebuilds ? `${t("usage.rebuilds")}: ${rebuilds}` : "";
  return {
    label: [duration, turn.usage ? usageLabel(turn.usage.usage, capabilities) : "", signal].filter(Boolean).join(" · "),
    title: turn.usage ? usageTooltip(turn.usage, capabilities) : "",
  };
}
