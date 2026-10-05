import defaultsText from "./action-defaults.json?raw";
import { invoke } from "./ipc";
import type { Board, Choice, Tab } from "./types";

export type Profile = {
  id: string; name: string; prompt: string; choice: Choice;
  mcp: string[] | null; plugins: string[] | null; skills: string[];
  permission: "ask" | "auto";
};
export type Action = { name: string; description: string; kind: "prompt" | "agent"; prompt: string; profile: string | null };
export type Catalog = { defaults_initialized?: boolean; profiles: Profile[]; commands: Action[]; overrides: Record<string, Record<string, Profile>>; pr_action: string | null };
export type TaskRun = {
  command: string; profile: Profile; done: boolean; error: string | null;
};
export const emptyCatalog = (): Catalog => ({ profiles: [], commands: [], overrides: {}, pr_action: null });
export function initializeDefaults(catalog: Catalog): Catalog {
  if (catalog.defaults_initialized) return catalog;
  const next = structuredClone(catalog);
  const seed = JSON.parse(defaultsText) as { profile: Profile; command: Action };
  if (!next.profiles.some(p => p.id === seed.profile.id) && !next.commands.some(c => c.name === seed.command.name)) {
    next.profiles.push(seed.profile); next.commands.push(seed.command);
  }
  next.defaults_initialized = true;
  return next;
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
