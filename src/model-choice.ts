import { catalogOf, descriptor, effortsOf, installed, legacyProvider, modelLabelOf, nativeEffort } from "./agents";
import { t, type Key } from "./i18n";
import type { ProviderId } from "./types";

export type ModelChoice = { agent: ProviderId; model: string };
const CHOICE_KEY = "prometeu:model-choice";
const FAVORITES_KEY = "prometeu:model-favorites";
const EFFORT_KEY = "prometeu:effort";
const PROVIDERS = ["claude", "codex", "antigravity", "cursor", "gemini"];
function read(key: string): string | null { try { return localStorage.getItem(key); } catch { return null; } }
function write(key: string, value: string) { try { localStorage.setItem(key, value); } catch { /* Selection remains usable when storage is unavailable. */ } }
function parse(raw: string | null): unknown { try { return JSON.parse(raw ?? "null"); } catch { return null; } }
function isChoice(value: unknown): value is ModelChoice {
  return !!value && typeof value === "object" && "agent" in value && "model" in value && PROVIDERS.includes(String(value.agent)) && typeof value.model === "string";
}
export const choiceKey = (choice: ModelChoice): string => JSON.stringify([choice.agent, choice.model]);
export const sameChoice = (a: ModelChoice, b: ModelChoice): boolean => a.agent === b.agent && a.model === b.model;

/// Existing selections survive catalog removals. A legacy ID is resolved only after discovery,
/// never by guessing the first provider containing it while other queries are still pending.
export function defaultChoice(): ModelChoice | null {
  const saved = parse(read(CHOICE_KEY));
  if (isChoice(saved)) return saved;
  const legacy = read("prometeu:model");
  if (legacy) {
    if (installed().some(p => catalogOf(p.id).status !== "ready")) return null;
    const agent = legacyProvider(legacy);
    if (!agent) return null;
    const choice = { agent, model: legacy };
    setDefaultChoice(choice);
    return choice;
  }
  return { agent: installed()[0]?.id ?? "claude", model: "" };
}
export function setDefaultChoice(choice: ModelChoice) { write(CHOICE_KEY, JSON.stringify(choice)); }
export function defaultEffort(choice: ModelChoice): string { return nativeEffort(choice.agent, read(EFFORT_KEY) ?? ""); }
export function setDefaultEffort(effort: string) { write(EFFORT_KEY, effort); }
export function favoriteChoices(): ModelChoice[] {
  const saved = parse(read(FAVORITES_KEY));
  if (!Array.isArray(saved)) return [];
  const seen = new Set<string>();
  return saved.filter((choice): choice is ModelChoice => {
    if (!isChoice(choice) || !choice.model || seen.has(choiceKey(choice))) return false;
    seen.add(choiceKey(choice)); return true;
  });
}
export function isFavorite(choice: ModelChoice): boolean { return favoriteChoices().some(item => sameChoice(item, choice)); }
export function toggleFavorite(choice: ModelChoice) {
  const favorites = favoriteChoices();
  write(FAVORITES_KEY, JSON.stringify(isFavorite(choice) ? favorites.filter(item => !sameChoice(item, choice)) : [...favorites, choice]));
}
const effortKeys: Record<string, Key> = { none: "effort.none", minimal: "effort.minimal", low: "effort.low", medium: "effort.medium", high: "effort.high", xhigh: "effort.xhigh", max: "effort.max", ultra: "effort.ultra" };
export const effortLabel = (effort: string): string => !effort ? t("models.default") : effortKeys[effort] ? t(effortKeys[effort]) : effort;
export function effortLadder(model: string, provider: ProviderId): [string, string][] {
  return [["", t("models.default")], ...[...new Set(effortsOf(provider, model))].filter(Boolean).map((effort): [string, string] => [effort, effortLabel(effort)])];
}
export function fitsEffort(model: string, effort: string, provider: ProviderId): string {
  const native = nativeEffort(provider, effort);
  return effortsOf(provider, model).includes(native) ? native : "";
}
export function effortStep(model: string, effort: string, provider: ProviderId): { label: string; step: number; total: number } | null {
  const native = nativeEffort(provider, effort);
  const ladder = effortLadder(model, provider);
  if (ladder.length === 1 && !native) return null;
  const step = ladder.findIndex(([id]) => id === native);
  return { label: step < 0 ? t("models.unavailableEffort", { effort }) : ladder[step][1], step, total: ladder.length };
}
export function modelLabel(model: string, provider: ProviderId): string { return model ? modelLabelOf(model, provider) : t("models.default"); }
export const choiceLabel = (choice: ModelChoice): string => `${modelLabel(choice.model, choice.agent)} · ${descriptor(choice.agent).label}`;
