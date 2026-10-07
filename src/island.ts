import { parseConversationEvent } from "./conversation";
import { fromBack } from "./i18n";
import { modelLabelOf } from "./agents";
import { invoke } from "./ipc";
import { readPreferences } from "./notifications";
import { pending, tabLabel, type Board, type ProviderId, type Status } from "./types";
import { debounce } from "./util";

/// The notch island renders what the main window already knows. This module reduces live chat
/// events into a bounded snapshot and pushes it only while the `notch` style is on.

export type IslandRequest = {
  id: string;
  requestKind: "approval" | "question" | "plan";
  tool: string;
  input: Record<string, unknown>;
  toolUseId: string | null;
};
export type IslandTab = {
  id: string;
  title: string;
  agent: ProviderId;
  model: string;
  status: Exclude<Status, "desligada">;
  /// Finished or waiting activity the person has not looked at, as counted by the Dock.
  unseen: boolean;
  note: string | null;
  /// Last prompt and the latest tool of the current turn; unknown before this app start.
  prompt: string | null;
  activity: { tool: string; target: string } | null;
  since: number | null;
  request: IslandRequest | null;
};
export type IslandUsage = { agent: ProviderId; windows: { label: string; pct: number; resets: number }[] };
export type IslandSnapshot = { tabs: IslandTab[]; usage: IslandUsage[] };

type Live = Pick<IslandTab, "prompt" | "activity" | "since"> & { requests: Map<string, IslandRequest> };

const MAX_TABS = 20;
const MAX_TEXT = 2000;
const RANK = { querendo: 0, rodando: 1, pronta: 2 } as const;

const live = new Map<string, Live>();
let board: Board | null = null;
let usage: () => IslandUsage[] = () => [];
let unseen: (tab: string) => boolean = () => false;
let on = false;

export function init(hooks: { usage: () => IslandUsage[]; unseen: (tab: string) => boolean }) {
  ({ usage, unseen } = hooks);
  sync();
}

/// Follow the notification preferences; the island exists only for the notch style.
export function sync() {
  const preferences = readPreferences();
  const next = preferences.enabled && preferences.style === "notch";
  if (next !== on) {
    on = next;
    void invoke("island_enable", { enabled: on }).catch(console.warn);
  }
  changed();
}

export const changed = debounce(100, () => {
  if (on && board) void invoke("island_update", { snapshot: snapshot(board, live, usage(), unseen) }).catch(console.warn);
});

export function boardChanged(next: Board) {
  board = next;
  const tabs = new Set(next.workspaces.flatMap(w => w.tabs.map(tab => tab.id)));
  for (const tab of live.keys()) if (!tabs.has(tab)) live.delete(tab);
  changed();
}

/// Live events only; snapshots and replay never reach this path.
export function chatChanged(tab: string, line: string) {
  if (track(live, tab, line)) changed();
}

export function track(store: Map<string, Live>, tab: string, line: string): boolean {
  let value: unknown;
  try { value = JSON.parse(line); } catch { return false; }
  const event = parseConversationEvent(value);
  if (!event) return false;
  const state = store.get(tab) ?? { prompt: null, activity: null, since: null, requests: new Map() };
  store.set(tab, state);
  switch (event.type) {
    case "user.message": {
      const text = event.content.flatMap(part => part.kind === "text" ? [part.text] : []).join(" ").trim();
      state.prompt = text.slice(0, 300) || null;
      state.since = event.at;
      state.activity = null;
      return true;
    }
    case "assistant.block":
      if (event.block.kind !== "tool") return false;
      state.activity = { tool: event.block.name, target: target(event.block.input) };
      return true;
    case "request.opened":
      state.requests.set(event.requestId, {
        id: event.requestId, requestKind: event.kind, tool: event.tool ?? "",
        // Question text keys the answers, so only tool inputs are shortened.
        input: event.kind === "question" ? event.input : clamp(event.input) as Record<string, unknown>,
        toolUseId: event.toolId,
      });
      return true;
    case "request.closed":
      return state.requests.delete(event.requestId);
    case "turn.completed":
      state.requests.clear();
      state.activity = null;
      return true;
    case "session.state":
      if (event.state === "busy" || event.state === "waiting") return false;
      state.requests.clear();
      return true;
    default:
      return false;
  }
}

/// Show what a tool touches: a file name, the first line of a command, or a search.
function target(input: unknown): string {
  const value = (input ?? {}) as Record<string, unknown>;
  const path = value.file_path ?? value.notebook_path ?? value.path;
  if (typeof path === "string") return path.split("/").pop() ?? path;
  const text = [value.command, value.pattern, value.url, value.query, value.description].find(item => typeof item === "string");
  return typeof text === "string" ? (text.split("\n")[0] ?? "").slice(0, 80) : "";
}

/// Cap long tool inputs such as large edits; the backend answers from its own copy of the request.
function clamp(value: unknown): unknown {
  if (typeof value === "string") return value.length > MAX_TEXT ? `${value.slice(0, MAX_TEXT)}…` : value;
  if (Array.isArray(value)) return value.map(clamp);
  if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, clamp(item)]));
  return value;
}

/// Local active conversations, most urgent first, then most recent.
export function snapshot(board: Board, store: Map<string, Live>, quota: IslandUsage[], unseen: (tab: string) => boolean = () => false): IslandSnapshot {
  const tabs = board.workspaces
    .filter(w => !w.archived && !w.cleaned && !w.remote && !pending(w))
    .flatMap(w => w.tabs.flatMap((tab): IslandTab[] => {
      if (tab.status === "desligada") return [];
      const state = store.get(tab.id);
      const agent = tab.choice?.agent ?? w.agent;
      return [{
        id: tab.id,
        title: w.tabs.length > 1 ? `${w.title} · ${tabLabel(w, tab)}` : w.title,
        agent,
        model: modelLabelOf(tab.choice?.model ?? w.model, agent),
        status: tab.status,
        unseen: unseen(tab.id),
        note: tab.note ? fromBack(tab.note) : null,
        prompt: state?.prompt ?? null,
        activity: state?.activity ?? null,
        since: state?.since ?? null,
        request: state?.requests.values().next().value ?? null,
      }];
    }))
    .sort((a, b) => RANK[a.status] - RANK[b.status] || (b.since ?? 0) - (a.since ?? 0))
    .slice(0, MAX_TABS);
  return { tabs, usage: quota.filter(item => item.windows.length) };
}
