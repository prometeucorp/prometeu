import defaultsText from "./action-defaults.json?raw";
import { invoke } from "./ipc";
import type { Board, Choice, ProviderId, Tab } from "./types";

export type Watch = { interval_seconds: number; comments: boolean; ci: boolean; max_turns: number };
export type ProviderRule = "fixed" | "different_from_builder";
export type Access = "default" | "read_only";
export type Profile = {
  id: string; name: string; prompt: string; choice: Choice;
  /// `different_from_builder` starts with the first usable candidate that did not build the workspace; `choice` mirrors candidates[0].
  provider_rule: ProviderRule; candidates: Choice[]; access: Access;
  mcp: string[] | null; plugins: string[] | null; skills: string[];
  permission: "ask" | "auto"; watch: Watch | null;
};
export type Action = { name: string; description: string; kind: "prompt" | "agent"; prompt: string; profile: string | null };
export type Catalog = { defaults_initialized?: boolean; defaults_revision?: number; profiles: Profile[]; commands: Action[]; overrides: Record<string, Record<string, Profile>>; pr_action: string | null };
export type TaskRun = {
  command: string; profile: Profile; paused: boolean; done: boolean; turns: number;
  checked_at: number; error: string | null; seen: Record<string, string>; prs: Record<string, number>;
};
export const emptyCatalog = (): Catalog => ({ profiles: [], commands: [], overrides: {}, pr_action: null });
type Seed = { revision: number; profile: Profile; command: Action; previous: Profile[] };
/// Mirror of the backend's serde defaults for catalogs saved before the provider rule and access level.
function withDefaults(catalog: Catalog) {
  for (const p of [...catalog.profiles, ...Object.values(catalog.overrides).flatMap(map => Object.values(map))]) {
    p.provider_rule ??= "fixed"; p.candidates ??= []; p.access ??= "default";
  }
}
const same = (a: Profile, b: Profile) => {
  const shape = (p: Profile) => JSON.stringify([p.id, p.name, p.prompt, [p.choice.agent, p.choice.model, p.choice.effort],
    p.provider_rule, p.candidates.map(c => [c.agent, c.model, c.effort]), p.access, p.mcp, p.plugins, p.skills, p.permission,
    p.watch && [p.watch.interval_seconds, p.watch.comments, p.watch.ci, p.watch.max_turns]]);
  return shape(a) === shape(b);
};
/// Mirror of Catalog::initialize_defaults.
export function initializeDefaults(catalog: Catalog): Catalog {
  const seed = JSON.parse(defaultsText) as Seed;
  if (catalog.defaults_initialized && (catalog.defaults_revision ?? 0) >= seed.revision) return catalog;
  const next = structuredClone(catalog);
  withDefaults(next);
  for (const old of seed.previous) withDefaults({ ...emptyCatalog(), profiles: [old] });
  if (!next.defaults_initialized) {
    if (!next.profiles.some(p => p.id === seed.profile.id) && !next.commands.some(c => c.name === seed.command.name)) {
      next.profiles.push(seed.profile); next.commands.push(seed.command);
    }
  } else {
    const at = next.profiles.findIndex(p => seed.previous.some(old => same(old, p)));
    if (at >= 0) next.profiles[at] = seed.profile;
  }
  next.defaults_initialized = true;
  next.defaults_revision = seed.revision;
  return next;
}
/// Mirror of the backend's provider-rule and access validation, so the editor can explain what the backend would refuse.
export function validRules(p: Profile, readOnlyCapable: (agent: ProviderId) => boolean): boolean {
  const different = p.provider_rule === "different_from_builder";
  const agents = different ? p.candidates.map(c => c.agent) : [p.choice.agent];
  const first = p.candidates[0];
  const rule = different
    ? agents.length >= 1 && agents.length <= 3 && new Set(agents).size === agents.length
      && first.agent === p.choice.agent && first.model === p.choice.model && first.effort === p.choice.effort
    : p.candidates.length === 0;
  return rule && (p.access !== "read_only" || (agents.every(readOnlyCapable) && !p.mcp?.length));
}
let board: Board | null = null;
let opened: (workspace: string, tab: Tab) => Promise<void> = async () => {};
const watchers = new Set<() => void>();
export const catalog = () => board?.actions ?? emptyCatalog();
export const projects = () => board?.projects ?? [];
export function update(value: Board) { board = value; for (const watch of watchers) watch(); }
export function init(onOpened: typeof opened) { opened = onOpened; }
export const onChange = (watch: () => void) => { watchers.add(watch); return () => watchers.delete(watch); };
export async function save(value: Catalog) { await invoke("actions_save", { catalog: value }); }

/// Provider commands retain their original names on collisions. The application namespace is also accepted without a collision.
export function commandNames(commands: Action[], provider: { name: string }[]) {
  const reserved = new Set(provider.map(c => c.name.toLowerCase()));
  return commands.map(action => ({ ...action, name: reserved.has(action.name) ? `prometeu:${action.name}` : action.name, action }));
}
export function findCommand(text: string, commands: Action[], provider: { name: string }[]): { action: Action; rest: string } | null {
  const match = /^\/([^\s]+)(?:\s+([\s\S]*))?$/.exec(text);
  if (!match) return null;
  const found = commandNames(commands, provider).find(c => c.name === match[1] || `prometeu:${c.action.name}` === match[1]);
  return found ? { action: found.action, rest: match[2] ?? "" } : null;
}
export const expand = (prompt: string, draft: string) => [prompt.trim(), draft.trim()].filter(Boolean).join("\n\n");
export async function start(workspace: string, action: Action, context = "") {
  const tab = await invoke("action_start", { workspace, name: action.name, context });
  await opened(workspace, tab);
  return tab;
}
