import { invoke } from "./ipc";
import type { ProviderId } from "./types";

export type AgentModel = {
  id: string;
  label: string;
  efforts: string[];
  additional?: boolean;
};

export type AgentCapabilities = {
  initialPlanMode: boolean;
  workspaceMcpSelection: boolean;
  workspacePluginSelection: boolean;
  resume: boolean;
  compact: boolean;
  contextReport: boolean;
  usageTokens: boolean;
  usageCost: boolean;
  contextWindow: boolean;
  approvals: boolean;
  userQuestions: boolean;
  attachments: boolean;
};

export type AuthMethod = { id: string; kind: "browser" | "external"; label: string };

export type AgentDescriptor = {
  authMethods: AuthMethod[];
  unavailableReason?: string | null;
  accountNotice?: string | null;
  id: ProviderId;
  label: string;
  installed: boolean;
  models: AgentModel[];
  capabilities: AgentCapabilities;
};

/// Historical aliases label saved choices only; they never establish availability.
const CLAUDE_FALLBACK_MODELS: AgentModel[] = [
  { id: "fable", label: "Fable", efforts: [] },
  { id: "fable[1m]", label: "Fable · 1M", efforts: [] },
  { id: "opus", label: "Opus", efforts: [] },
  { id: "opus[1m]", label: "Opus · 1M", efforts: [] },
  { id: "sonnet", label: "Sonnet", efforts: [] },
  { id: "sonnet[1m]", label: "Sonnet · 1M", efforts: [] },
  { id: "haiku", label: "Haiku", efforts: [] },
];

const NO_CAPABILITIES: AgentCapabilities = {
  initialPlanMode: false,
  workspaceMcpSelection: false,
  workspacePluginSelection: false,
  resume: false,
  compact: false,
  contextReport: false,
  usageTokens: false,
  usageCost: false,
  contextWindow: false,
  approvals: false,
  userQuestions: false,
  attachments: false,
};

/// Explicit bootstrap and failure states allow Claude startup without advertising unconfirmed optional capabilities.
const BOOTSTRAP: AgentDescriptor[] = [
  {
    id: "claude",
    label: "Claude",
    authMethods: [{ id: "browser", kind: "browser", label: "Claude" }],
    installed: true,
    models: [],
    capabilities: NO_CAPABILITIES,
  },
  {
    id: "codex",
    label: "Codex",
    authMethods: [{ id: "browser", kind: "browser", label: "Codex" }],
    installed: false,
    models: [],
    capabilities: NO_CAPABILITIES,
  },
  {
    id: "antigravity", label: "Antigravity", installed: false, models: [],
    authMethods: [{ id: "external", kind: "external", label: "Antigravity" }],
    capabilities: NO_CAPABILITIES,
  },
];

const RETIRED: AgentDescriptor = {
  id: "gemini", label: "Gemini CLI", installed: false, models: [], authMethods: [],
  unavailableReason: 'i18n:{"code":"err.provider.retired"}',
  capabilities: NO_CAPABILITIES,
};
export type ModelCatalog = { models: AgentModel[]; fetchedAt: number };
export type CatalogState = {
  status: "idle" | "loading" | "ready" | "error";
  fetchedAt: number | null;
  error: string | null;
};
let catalog = BOOTSTRAP;
let generation = 0;
const states = new Map<ProviderId, CatalogState>();
const pending = new Map<ProviderId, Promise<void>>();
const listeners = new Set<() => void>();
export function onCatalogChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
function changed() { for (const listener of listeners) listener(); }
export function catalogOf(provider: ProviderId): CatalogState {
  return states.get(provider) ?? { status: "idle", fetchedAt: null, error: null };
}

/// Account changes invalidate outstanding work before discovering installations. No catalog is
/// reused across account generations; each provider's slow query completes independently.
export async function loadAgents() {
  const current = ++generation;
  pending.clear();
  states.clear();
  catalog = catalog.map(provider => ({ ...provider, models: [] }));
  changed();
  try {
    const discovered = await invoke("agents");
    if (current !== generation) return;
    catalog = discovered.providers.length ? discovered.providers.map(p => ({ ...p, models: [] })) : BOOTSTRAP;
  } catch {
    if (current !== generation) return;
    catalog = BOOTSTRAP;
  }
  changed();
  for (const provider of installed()) void refreshModels(provider.id, true);
}

const CATALOG_TTL = 5 * 60_000;
const CATALOG_ERRORS = new Set(["err.modelsCatalog.noAccount", "err.modelsCatalog.unavailable", "err.modelsCatalog.timeout", "err.modelsCatalog.invalid", "err.modelsCatalog.failed"]);
export function refreshModels(provider: ProviderId, force = false): Promise<void> {
  const running = pending.get(provider);
  if (running) return running;
  if (!descriptor(provider).installed) return Promise.resolve();
  const previous = catalogOf(provider);
  if (!force && previous.status === "ready" && previous.fetchedAt !== null && Date.now() - previous.fetchedAt < CATALOG_TTL) return Promise.resolve();
  const current = generation;
  states.set(provider, { ...previous, status: "loading", error: null });
  const task = Promise.resolve().then(() => invoke("agent_models", { agent: provider })).then(result => {
    if (current !== generation) return;
    catalog = catalog.map(item => item.id === provider ? { ...item, models: result.models } : item);
    states.set(provider, { status: "ready", fetchedAt: result.fetchedAt, error: null });
  }).catch((error: unknown) => {
    if (current !== generation) return;
    const code = typeof error === "object" && error !== null && "code" in error ? String(error.code) : "err.modelsCatalog.failed";
    states.set(provider, { ...previous, status: "error", error: CATALOG_ERRORS.has(code) ? code : "err.modelsCatalog.failed" });
  }).finally(() => {
    if (current !== generation) return;
    pending.delete(provider);
    changed();
  });
  pending.set(provider, task);
  changed();
  return task;
}

export const descriptors = (): readonly AgentDescriptor[] => catalog;
export const installed = (): AgentDescriptor[] => catalog.filter(provider => provider.installed);
export function descriptor(id: ProviderId): AgentDescriptor {
  return catalog.find(provider => provider.id === id) ?? BOOTSTRAP.find(provider => provider.id === id) ?? RETIRED;
}
export const capabilitiesOf = (id: ProviderId): AgentCapabilities => descriptor(id).capabilities;
export const modelsOf = (id: ProviderId): readonly AgentModel[] => descriptor(id).models;
export const modelOf = (model: string, provider: ProviderId): AgentModel | undefined => modelsOf(provider).find(candidate => candidate.id === model);
export function modelLabelOf(model: string, provider: ProviderId): string {
  return modelOf(model, provider)?.label ?? (provider === "claude" ? CLAUDE_FALLBACK_MODELS.find(candidate => candidate.id === model)?.label : undefined) ?? model;
}
export const isKnownModel = (provider: ProviderId, model: string): boolean => modelOf(model, provider) !== undefined;
export const effortsOf = (provider: ProviderId, model: string): readonly string[] => modelOf(model, provider)?.efforts ?? [];
/// Old Codex choices used the application's spelling; new choices use the CLI's native values.
export const nativeEffort = (provider: ProviderId, effort: string): string => provider === "codex" && effort === "ultracode" ? "ultra" : effort;
/// Legacy preferences lacked provider identity. Migrate only when the result cannot be confused.
export function legacyProvider(model: string): ProviderId | undefined {
  const matches = installed().filter(provider => isKnownModel(provider.id, model));
  if (matches.length === 1) return matches[0].id;
  if (matches.length === 0 && CLAUDE_FALLBACK_MODELS.some(candidate => candidate.id === model)) return "claude";
  return undefined;
}
