import { getCurrentWindow } from "@tauri-apps/api/window";
import { parseConversationEvent } from "./conversation";
import { count as notificationCount } from "./notification-center";
import type { Board } from "./types";

/// Track unread activity for the Dock badge.

/* State. */

type Ctx = {
  /// Visible tabs in the workspace or desk; window focus is checked here.
  visible: (tab: string) => boolean;
  notify?: (kind: "approval" | "done" | "error", tab: string) => void;
  /// Unread activity changed, e.g. for the notch island.
  changed?: () => void;
};

let ctx: Ctx = { visible: () => false };
type Conversation = {
  workspace: string;
  phase: "idle" | "sent" | "running";
  background: boolean;
  /// A turn that ended while background tasks were still running, waiting for them to drain.
  held: "ok" | "error" | null;
  pending: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  requests: Set<string>;
};
const conversations = new Map<string, Conversation>();
// Let immediate continuations invalidate a terminal event. Silence alone never means completion.
const SETTLE_MS = 1_000;
const watching = (tab: string) => document.hasFocus() && ctx.visible(tab);
function cancel(conversation: Conversation) {
  if (conversation.timer !== null) clearTimeout(conversation.timer);
  conversation.timer = null;
}
/// A terminal from the main agent becomes a notice candidate after a second of silence. Background
/// tasks hold it back until they drain, so the person still hears about the work that outlived the turn.
function complete(tab: string, conversation: Conversation, failed: boolean) {
  if (conversation.phase === "idle" || conversation.timer !== null) return;
  if (watching(tab)) {
    conversation.phase = "idle";
    return;
  }
  conversation.timer = setTimeout(() => {
    conversation.timer = null;
    conversation.phase = "idle";
    conversation.pending = !watching(tab);
    if (conversation.pending) ctx.notify?.(failed ? "error" : "done", tab);
    badge();
  }, SETTLE_MS);
}
let unread = new Set<string>();

export function init(context: Ctx) {
  ctx = context;
  // Returning to the window acknowledges visible activity.
  window.addEventListener("focus", looked);
}

/// Acknowledge visible activity without creating another pending completion.
export function looked() {
  for (const [tab, conversation] of conversations) {
    if (!watching(tab)) continue;
    conversation.pending = false;
    // Looking at the conversation acknowledges a terminal, including one its subagents still hold.
    if (conversation.held !== null || conversation.timer !== null) {
      conversation.held = null;
      cancel(conversation);
      conversation.phase = "idle";
    }
  }
  badge();
}

/// Snapshots only reconcile ownership and unread indicators, never execution state.
export function boardChanged(board: Board) {
  const local = board.workspaces.filter((w) => !w.archived && !w.cleaned && !w.remote);
  const owners = new Map(local.flatMap((w) => w.tabs.map((tab) => [tab.id, w.id] as const)));
  unread = new Set(local.filter((w) => w.unread).map((w) => w.id));
  for (const [tab, conversation] of conversations) {
    if (!owners.has(tab)) {
      cancel(conversation);
      conversations.delete(tab);
    }
  }
  for (const [tab, workspace] of owners) {
    const conversation = conversations.get(tab);
    if (conversation) conversation.workspace = workspace;
    else conversations.set(tab, { workspace, phase: "idle", background: false, held: null, pending: false, timer: null, requests: new Set() });
  }
  looked();
}

/// Track accepted live input for the Dock; provider echoes and request responses cannot start another execution.
export function chatChanged(tab: string, line: string) {
  const conversation = conversations.get(tab);
  if (!conversation) return;
  let value: unknown;
  try { value = JSON.parse(line); } catch { return; }
  const event = parseConversationEvent(value);
  if (!event) return;
  switch (event.type) {
    case "session.state":
      if (event.state === "starting") {
        cancel(conversation);
        conversation.phase = "idle";
        conversation.background = false;
        conversation.held = null;
        conversation.pending = false;
        conversation.requests.clear();
        badge();
        return;
      }
      if (event.state !== "busy") return;
      cancel(conversation);
      conversation.phase = "sent";
      conversation.held = null;
      conversation.pending = false;
      badge();
      return;
    case "assistant.started":
    case "assistant.block.started":
    case "assistant.block":
    case "assistant.delta":
    case "tool.input.delta":
    case "tool.completed":
      cancel(conversation);
      // The main agent answering again invalidates the terminal its subagents were holding.
      conversation.held = null;
      if (conversation.phase === "sent") conversation.phase = "running";
      return;
    case "user.message":
      cancel(conversation);
      return;
    case "context.compaction":
      if (event.state === "started") cancel(conversation);
      return;
    case "background.changed": {
      conversation.background = event.tasks.length > 0;
      if (conversation.background) {
        cancel(conversation);
        return;
      }
      // Draining is not the primary agent's final response; it only releases one already sent.
      if (conversation.held === null) return;
      const failed = conversation.held === "error";
      conversation.held = null;
      complete(tab, conversation, failed);
      return;
    }
    case "request.opened":
    case "request.closed":
      cancel(conversation);
      if (event.type === "request.opened" && !conversation.requests.has(event.requestId)) {
        conversation.requests.add(event.requestId);
        if (!watching(tab)) ctx.notify?.("approval", tab);
      }
      if (event.type === "request.closed") conversation.requests.delete(event.requestId);
      conversation.pending = event.type === "request.opened" && !watching(tab);
      badge();
      return;
    case "turn.completed":
      if (event.outcome === "interrupted") {
        cancel(conversation);
        conversation.held = null;
        conversation.phase = "idle";
        return;
      }
      if (conversation.phase === "sent" && event.outcome !== "error") {
        conversation.held = null;
        conversation.phase = "idle";
        return;
      }
      if (conversation.phase === "idle") return;
      // Subagents outlive the turn that started them; wait for them before offering the notice.
      if (conversation.background) {
        conversation.held = event.outcome === "error" ? "error" : "ok";
        return;
      }
      complete(tab, conversation, event.outcome === "error");
  }
}

/// Team mentions and GitHub notifications change the badge.
export function teamChanged() {
  badge();
}

/* Dock badge. */

/// Count unread workspaces plus mentions and unread GitHub notifications; zero clears the badge.
export function waiting() {
  const workspaces = new Set(unread);
  for (const conversation of conversations.values()) {
    if (conversation.pending) workspaces.add(conversation.workspace);
  }
  return workspaces.size + notificationCount();
}

/// Activity the person has not looked at yet, the same signal the Dock counts.
export const unseen = (tab: string) => conversations.get(tab)?.pending ?? false;

function badge() {
  ctx.changed?.();
  getCurrentWindow()
    .setBadgeCount(waiting() || undefined)
    .catch((e) => {
      // The web mock has no Dock badge.
      console.warn("badge", e);
    });
}
