import { type TelemetryCursor, type TelemetryEvent, type TelemetryFilter, type TelemetryPage, type TelemetrySummary, type TelemetryMeasurement, type TelemetryUsage, type TelemetryInsights, type UsageGroup, type TurnMeasurement } from "./telemetry";

const KEY = "mock:telemetry";
let generation = 0;
const health = () => ({ failures: 0, lastFailureAt: null, unavailable: false });
/** Fixed local facts exercise settings independently of native adapters. Clearing persists an empty array. */
function events(): TelemetryEvent[] {
  const saved = localStorage.getItem(KEY);
  if (saved !== null) return JSON.parse(saved) as TelemetryEvent[];
  const at = Date.now() - 60_000;
  const measurement: TelemetryMeasurement = { usageScope: "mainAgent", complete: true, selectedModel: "example-model", observedModels: null, usageByModel: null,
    usage: { inputTokens: 10000, outputTokens: 2000, cacheReadTokens: 8000, cacheWriteTokens: null, reasoningTokens: 500,
      contextUsed: null, contextWindow: null, peakContext: null, modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null } };
  const scope = { schemaVersion: 1, category: "work" as const, workspaceId: "00000000-0000-4000-8000-000000000001", projectId: null,
    conversationId: "00000000-0000-4000-8000-000000000002", turnId: "00000000-0000-4000-8000-000000000003", provider: "codex" };
  const result: TelemetryEvent[] = [
    { ...scope, id: crypto.randomUUID(), occurrenceKey: crypto.randomUUID(), sequence: 1, occurredAt: at, recordedAt: at,
      type: "turn.started", payload: { measurement } },
    { ...scope, id: crypto.randomUUID(), occurrenceKey: crypto.randomUUID(), sequence: 2, occurredAt: at + 30_000, recordedAt: at + 30_000,
      type: "turn.completed", payload: { outcome: "ok", elapsedMs: 30_000, providerDurationMs: null, measurement } },
  ];
  localStorage.setItem(KEY, JSON.stringify(result)); return result;
}
const inPeriod = (at: number, filter: TelemetryFilter) => at >= (filter.from ?? 0) && at < (filter.to ?? Number.MAX_SAFE_INTEGER);
function selected(filter: TelemetryFilter): TelemetryEvent[] {
  if ((filter.from ?? 0) >= (filter.to ?? Number.MAX_SAFE_INTEGER)) throw new Error("err.telemetry.filter");
  const all = events();
  const related = new Set(all.filter(e => e.type === "pull_request.associated" && e.payload.repositoryId === filter.repositoryId && e.payload.pullRequest === filter.pullRequest).map(e => e.workspaceId));
  return all.filter(e => (!filter.workspaceId || e.workspaceId === filter.workspaceId) && (!filter.repositoryId || related.has(e.workspaceId)));
}
export function summary(filter: TelemetryFilter): TelemetrySummary {
  const all = selected(filter); const period = all.filter(e => inPeriod(e.occurredAt, filter));
  const turns = new Set(all.filter(e => e.type === "turn.started" && inPeriod(e.occurredAt, filter)).map(e => e.turnId));
  const observations = [...turns].flatMap(turn => {
    const candidates = all.filter(event => event.turnId === turn).reverse();
    const last = candidates.find(event => event.type === "turn.completed") ?? candidates.find(event => event.type === "turn.usage.observed");
    return last && (last.type === "turn.completed" || last.type === "turn.usage.observed") ? [{ completed: last.type === "turn.completed", measurement: last.payload.measurement }] : [];
  });
  const usage = observations.map(row => row.measurement.usage);
  const sum = (values: (number | null)[]) => values.some(v => v !== null) ? values.reduce<number>((a, v) => a + (v ?? 0), 0) : null;
  return { firstRecordedAt: period[0]?.occurredAt ?? null, lastRecordedAt: period[period.length - 1]?.occurredAt ?? null, events: period.length,
    turns: turns.size, completedTurns: observations.filter(row => row.completed).length,
    partialTurns: observations.filter(row => (!row.completed || !row.measurement.complete) && (row.measurement.usage.inputTokens !== null || row.measurement.usage.outputTokens !== null)).length,
    measuredTurns: observations.filter(row => row.measurement.complete && row.measurement.usage.inputTokens !== null && row.measurement.usage.outputTokens !== null).length,
    inputTokens: sum(usage.map(u => u.inputTokens)), outputTokens: sum(usage.map(u => u.outputTokens)), costUsd: sum(usage.map(u => u.costUsd)),
    incompleteExecutions: 0, completeExecutions: 0, clockAnomalies: 0, executionSumMs: null, activeAgentMs: null,
    respondedWaits: 0, cancelledWaits: 0, incompleteWaits: 0, humanWaitMs: null,
    workspaceIds: [...new Set(selected({ ...filter, workspaceId: undefined }).flatMap(e => e.workspaceId ? [e.workspaceId] : []))], health: health() };
}
export function page(filter: TelemetryFilter, cursor?: TelemetryCursor): TelemetryPage {
  const records = selected(filter).filter(e => inPeriod(e.occurredAt, filter) && (!cursor || e.occurredAt > cursor.occurredAt || (e.occurredAt === cursor.occurredAt && e.sequence > cursor.sequence)));
  const events = records.slice(0, 500); const last = events[events.length - 1];
  return { events, next: records.length > 500 && last ? { occurredAt: last.occurredAt, sequence: last.sequence } : null, health: health() };
}
export function exportData(filter: TelemetryFilter): string {
  return [JSON.stringify({ exportVersion: 1, filter, health: health(), summary: summary(filter) }), ...selected(filter).filter(e => inPeriod(e.occurredAt, filter)).map(e => JSON.stringify(e))].join("\n") + "\n";
}
export function clear(): void { generation++; localStorage.setItem(KEY, "[]"); }

const emptyUsage = (): TelemetryUsage => ({ inputTokens: null, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null,
  reasoningTokens: null, contextUsed: null, contextWindow: null, peakContext: null, modelCalls: null,
  compactions: null, cacheRebuilds: null, costUsd: null });

function addUsage(total: TelemetryUsage, usage: TelemetryUsage): void {
  for (const key of Object.keys(total) as (keyof TelemetryUsage)[]) {
    if (key === "contextUsed" || key === "contextWindow" || usage[key] === null) continue;
    total[key] = key === "peakContext" ? Math.max(total[key] ?? 0, usage[key]) : (total[key] ?? 0) + usage[key];
  }
}

function group(rows: UsageGroup[], id: string, provider: string | null, usage: TelemetryUsage): void {
  let row = rows.find(candidate => candidate.id === id);
  if (!row) { row = { id, provider, turns: 0, usage: emptyUsage() }; rows.push(row); }
  if (row.provider !== provider) row.provider = null;
  row.turns++;
  addUsage(row.usage, usage);
}

/** Use the same final-or-latest start cohort as native queries; never sum snapshots. */
export function insights(filter: TelemetryFilter): TelemetryInsights {
  const all = selected(filter);
  const result: TelemetryInsights = { summary: summary(filter), usage: emptyUsage(), conversations: [], models: [], sources: [], origins: [], pullRequests: [] };
  for (const start of all.filter(event => event.type === "turn.started" && inPeriod(event.occurredAt, filter))) {
    const observations = all.filter(event => event.turnId === start.turnId);
    const end = observations.slice().reverse().find(event => event.type === "turn.completed")
      ?? observations.slice().reverse().find(event => event.type === "turn.usage.observed");
    const measurement = end && (end.type === "turn.completed" || end.type === "turn.usage.observed") ? end.payload.measurement : null;
    const usage = measurement?.usage ?? emptyUsage();
    addUsage(result.usage, usage);
    if (start.conversationId) group(result.conversations, start.conversationId, start.provider, usage);
    group(result.sources, "conversation", start.provider, usage);
    if (start.type === "turn.started") {
      if (start.payload.origin?.actionId) group(result.origins, "action", start.provider, usage);
      if (start.payload.origin?.delegatedBy) group(result.origins, "delegation", start.provider, usage);
    }
    for (const model of measurement?.usageByModel ?? []) group(result.models, model.model, start.provider, model.usage);
  }
  for (const start of all) {
    if (start.type !== "app.call.started" || !inPeriod(start.occurredAt, filter)) continue;
    const end = all.slice().reverse().find(event => event.type === "app.call.completed" && event.payload.callId === start.payload.callId);
    const measurement = end?.type === "app.call.completed" ? end.payload.measurement : start.payload.measurement;
    addUsage(result.usage, measurement.usage);
    group(result.sources, start.payload.source, start.provider, measurement.usage);
    for (const model of measurement.usageByModel ?? []) group(result.models, model.model, start.provider, model.usage);
  }
  for (const event of all) {
    if (event.type !== "pull_request.associated") continue;
    if (result.pullRequests.some(row => row.repositoryId === event.payload.repositoryId && row.pullRequest === event.payload.pullRequest)) continue;
    result.pullRequests.push({ ...event.payload, turns: null, usage: emptyUsage(), attribution: "related" });
  }
  return result;
}

async function messageKey(conversation: string, messageId: string): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(`${conversation}\0${messageId}`));
  return [...new Uint8Array(hash)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

export async function turns(conversation: string, messageIds: string[]): Promise<TurnMeasurement[]> {
  if (messageIds.length > 500 || messageIds.some(id => !id || new TextEncoder().encode(id).length > 1024)) throw new Error("err.telemetry.filter");
  const requested = await Promise.all([...new Set(messageIds)].map(async messageId => ({ messageId, key: await messageKey(conversation, messageId) })));
  const all = events();
  return requested.flatMap(({ messageId, key }) => {
    const matches = all.filter(event => event.type === "turn.completed" && event.conversationId === conversation && event.payload.messageKey === key);
    const event = matches.length === 1 ? matches[0] : undefined;
    return event?.type === "turn.completed" ? [{ messageId, durationMs: event.payload.providerDurationMs ?? event.payload.elapsedMs, usage: event.payload.measurement }] : [];
  });
}

export function sampleUsage(provider: string): TelemetryMeasurement {
  const usage = { ...emptyUsage(), inputTokens: 24000, outputTokens: 1600, cacheReadTokens: 20000,
    cacheWriteTokens: provider === "claude" ? 1000 : null, reasoningTokens: provider === "codex" ? 400 : null,
    contextUsed: 24000, contextWindow: provider === "antigravity" ? null : 200000,
    peakContext: 24000, modelCalls: 2, costUsd: provider === "claude" ? 0.12 : null };
  return { usageScope: "mainAgent", complete: true, selectedModel: null, observedModels: null, usageByModel: null, usage };
}

/** Capture mock live work only; reloading a saved transcript does not backfill history. */
export async function recordTurn(scope: { workspaceId: string; projectId: string; conversationId: string; provider: string },
  messageId: string, durationMs: number, measurement: TelemetryMeasurement): Promise<void> {
  const startedGeneration = generation;
  const key = await messageKey(scope.conversationId, messageId);
  if (generation !== startedGeneration) return;
  const all = events();
  if (all.some(event => event.type === "turn.completed" && event.conversationId === scope.conversationId && event.payload.messageKey === key)) return;
  const at = Date.now();
  const shared = { ...scope, schemaVersion: 1, category: "work" as const, turnId: crypto.randomUUID(), recordedAt: at };
  const sequence = all.reduce((max, event) => Math.max(max, event.sequence), 0) + 1;
  all.push({ ...shared, id: crypto.randomUUID(), occurrenceKey: crypto.randomUUID(), sequence, occurredAt: at - durationMs,
    type: "turn.started", payload: { measurement } },
  { ...shared, id: crypto.randomUUID(), occurrenceKey: crypto.randomUUID(), sequence: sequence + 1, occurredAt: at,
    type: "turn.completed", payload: { outcome: "ok", elapsedMs: durationMs, providerDurationMs: durationMs, measurement, messageKey: key } });
  localStorage.setItem(KEY, JSON.stringify(all));
}
