import type { Board } from "./types";
import { tabLabel } from "./types";
import type { TelemetryInsights, TelemetryUsage, UsageGroup } from "./telemetry";
import type { UsageHistoryData } from "./components/usage-history";
import { current, t, tn, type Key } from "./i18n";
import { costLabel } from "./usage-presentation";

/** A known sum can include one observed side; null still means no token measurement. */
export function knownTokens(usage: TelemetryUsage): number | null {
  return usage.inputTokens === null && usage.outputTokens === null ? null : (usage.inputTokens ?? 0) + (usage.outputTokens ?? 0);
}

export function usageHistoryData(insights: TelemetryInsights, board: Board, providers: ReadonlyMap<string, string>): UsageHistoryData {
  const number = (value: number | null) => value === null ? "—" : value.toLocaleString(current());
  const compact = (value: number | null) => value === null ? "—" : new Intl.NumberFormat(current(), { notation: "compact", maximumFractionDigits: 2 }).format(value);
  const cost = (value: number | null) => value === null ? "—" : costLabel(value);
  const identity = (kind: string, id: string) => board.telemetry_ids?.[`${kind}:${id}`] ?? id;
  const names = new Map(board.workspaces.flatMap(workspace => workspace.tabs.map(tab => [identity("conversation", tab.id), { title: tabLabel(workspace, tab), workspace: workspace.title }] as const)));
  const provider = (id: string | null) => id ? providers.get(id) ?? id : t("history.providerUnknown");
  const fields: [keyof TelemetryUsage, Key, "input" | "output" | "subset"][] = [
    ["inputTokens", "history.input", "input"], ["cacheReadTokens", "usage.cacheRead", "subset"], ["cacheWriteTokens", "usage.cacheWrite", "subset"],
    ["outputTokens", "history.output", "output"], ["reasoningTokens", "usage.reasoning", "subset"],
  ];
  const total = knownTokens(insights.usage);
  const sourceKeys: Record<string, Key> = { conversation: "usage.source.conversation", naming: "usage.source.naming", "plugin-maker": "usage.source.pluginMaker" };
  const sorted = [...insights.conversations].sort((a, b) => (knownTokens(b.usage) ?? -1) - (knownTokens(a.usage) ?? -1) || a.id.localeCompare(b.id));
  const status = (group: UsageGroup) => knownTokens(group.usage) === null ? "unknown" : group.coverage?.measuredTurns === group.turns ? "complete" : "partial";
  return {
    metrics: [
      { label: t("history.tokens"), value: compact(total), note: t("history.knownHint") },
      { label: t("history.cost"), value: cost(insights.usage.costUsd), note: t("usage.costHint") },
      { label: t("history.sessions"), value: number(insights.conversations.length), note: tn(insights.summary.turns, "usage.turns") },
    ],
    inputShare: total ? (insights.usage.inputTokens ?? 0) / total * 100 : null,
    tokens: fields.map(([key, label, kind]) => ({ label: t(label), value: number(insights.usage[key]), kind })),
    sources: insights.sources.map(row => ({ label: sourceKeys[row.id] ? t(sourceKeys[row.id]) : row.id, value: number(knownTokens(row.usage)), share: total ? (knownTokens(row.usage) ?? 0) / total * 100 : null })),
    sessions: sorted.map(group => {
      const coverage = group.coverage;
      const notes: string[] = [];
      if (coverage) {
        notes.push(t("history.detailCoverage", { measured: coverage.measuredTurns, turns: group.turns, partial: coverage.partialTurns, unknown: coverage.unknownTurns }));
        notes.push(t(coverage.wholeTreeTurns === group.turns ? "usage.wholeTree" : coverage.mainAgentTurns === group.turns ? "usage.mainAgent" : "history.mixedScope"));
      } else notes.push(t("history.coverageUnknown"));
      if (group.usage.costUsd !== null) notes.push(t("usage.costHint"));
      return {
        id: group.id, title: names.get(group.id)?.title ?? t("usage.conversationMissing", { id: group.id.slice(0, 8) }),
        workspace: names.get(group.id)?.workspace ?? "", provider: provider(group.provider), turns: number(group.turns),
        tokens: number(knownTokens(group.usage)), cost: cost(group.usage.costUsd), status: status(group),
        coverage: t(status(group) === "complete" ? "history.complete" : status(group) === "partial" ? "history.partial" : "history.unknown"),
        details: [
          ...fields.map(([key, label]) => ({ label: t(label), value: number(group.usage[key]) })),
          { label: t("history.cost"), value: cost(group.usage.costUsd) },
          ...([["peakContext", "history.peak"], ["modelCalls", "history.calls"], ["compactions", "usage.compactions"], ["cacheRebuilds", "usage.rebuilds"]] as const)
            .map(([key, label]) => ({ label: t(label), value: number(group.usage[key]) })),
        ], notes,
      };
    }),
    models: insights.models.map(row => ({ label: [row.id, row.provider ? provider(row.provider) : ""].filter(Boolean).join(" · "),
      value: `${t("history.input")}: ${number(row.usage.inputTokens)} · ${t("history.output")}: ${number(row.usage.outputTokens)}` })),
  };
}
