/** Content-free telemetry IPC. Usage totals include their cache/reasoning subsets. */
import type { ConversationUsage as TelemetryMeasurement, UsageCounts as TelemetryUsage } from "./conversation";
export type { ConversationUsage as TelemetryMeasurement, UsageCounts as TelemetryUsage, TurnMeasurement } from "./conversation";
export type TelemetryFilter = { from?: number; to?: number; workspaceId?: string; repositoryId?: string; pullRequest?: number };
export type TelemetryHealth = { failures: number; lastFailureAt: number | null; unavailable: boolean };
export type TelemetrySummary = {
  firstRecordedAt: number | null; lastRecordedAt: number | null; events: number;
  turns: number; completedTurns: number; partialTurns: number; measuredTurns: number;
  inputTokens: number | null; outputTokens: number | null; costUsd: number | null;
  incompleteExecutions: number; completeExecutions: number; clockAnomalies: number;
  executionSumMs: number | null; activeAgentMs: number | null;
  respondedWaits: number; cancelledWaits: number; incompleteWaits: number; humanWaitMs: number | null;
  workspaceIds: string[]; health: TelemetryHealth;
};
export type TelemetryFact =
  | { type: "workspace.created"; payload: { mode: "worktree" | "repository" } }
  | { type: "workspace.archived" | "workspace.resumed" | "conversation.created"; payload: Record<string, never> }
  | { type: "provider.selected"; payload: { scope: "workspace" | "conversation" } }
  | { type: "pull_request.associated"; payload: { repositoryId: string; branchId: string | null; pullRequest: number } }
  | { type: "turn.started" | "turn.usage.observed"; payload: { measurement: TelemetryMeasurement; origin?: { actionId: string | null; delegatedBy: string | null; repositories: { repositoryId: string; branchId: string | null }[] } } }
  | { type: "pull_request.observed"; payload: { repositoryId: string; branchId: string | null; pullRequest: number;
      state: "open" | "closed" | "merged"; createdAt: number | null; closedAt: number | null; mergedAt: number | null;
      historyComplete: boolean; historySize: number; observedAfter: number; snapshotId: string } }
  | { type: "turn.completed"; payload: { outcome: "ok" | "error" | "interrupted"; elapsedMs: number; providerDurationMs: number | null; measurement: TelemetryMeasurement; messageKey?: string | null } }
  | { type: "app.call.started"; payload: { callId: string; source: "naming" | "plugin-maker"; measurement: TelemetryMeasurement } }
  | { type: "app.call.completed"; payload: { callId: string; source: "naming" | "plugin-maker"; outcome: "ok" | "error" | "interrupted"; elapsedMs: number; measurement: TelemetryMeasurement } }
  | { type: "agent.execution.started"; payload: { executionId: string } }
  | { type: "agent.execution.completed"; payload: { executionId: string; elapsedMs: number } }
  | { type: "human_input.requested"; payload: { requestId: string; kind: "approval" | "question" | "plan" } }
  | { type: "human_input.received" | "human_input.cancelled"; payload: { requestId: string; elapsedMs: number } }
  | { type: "context.compacted"; payload: { before: number | null; after: number | null } };
export type TelemetryCursor = { occurredAt: number; sequence: number };
export type TelemetryEvent = TelemetryFact & {
  sequence: number; id: string; occurrenceKey: string; schemaVersion: number;
  occurredAt: number; recordedAt: number; category: "journey" | "work";
  projectId: string | null; workspaceId: string | null; conversationId: string | null;
  turnId: string | null; provider: string | null;
};
export type TelemetryPage = { events: TelemetryEvent[]; next: TelemetryCursor | null; health: TelemetryHealth };
export type UsageGroup = { id: string; provider: string | null; turns: number; usage: TelemetryUsage };
export type PullRequestUsage = { repositoryId: string; branchId: string | null; pullRequest: number;
  turns: number | null; usage: TelemetryUsage; attribution: "tenure" | "related" };
export type TelemetryInsights = { summary: TelemetrySummary; usage: TelemetryUsage; conversations: UsageGroup[];
  models: UsageGroup[]; sources: UsageGroup[]; origins: UsageGroup[]; pullRequests: PullRequestUsage[] };

const clearedListeners = new Set<() => void>();
/** Invalidate local presentation caches after confirmed history deletion. */
export function onTelemetryCleared(listener: () => void): () => void {
  clearedListeners.add(listener);
  return () => { clearedListeners.delete(listener); };
}
export function telemetryCleared(): void {
  for (const listener of clearedListeners) listener();
}

/** Native date inputs represent local calendar days, including daylight-saving transitions. */
export function telemetryPeriod(from: string, through: string): TelemetryFilter {
  const filter: TelemetryFilter = {};
  if (from) filter.from = new Date(`${from}T00:00:00`).getTime();
  if (through) { const end = new Date(`${through}T00:00:00`); end.setDate(end.getDate() + 1); filter.to = end.getTime(); }
  if ((filter.from !== undefined && !Number.isFinite(filter.from)) || (filter.to !== undefined && !Number.isFinite(filter.to))
    || (filter.from ?? 0) >= (filter.to ?? Number.MAX_SAFE_INTEGER)) throw new Error("err.telemetry.filter");
  return filter;
}
