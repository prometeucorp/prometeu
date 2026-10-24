import { invoke } from "./ipc";
import { t, type Key } from "./i18n";
import { PT } from "./i18n.pt";
import type { ProviderId } from "./types";

/** Anonymous product events (ADR 0093). Strings are only identifiers we author: i18n keys and
 * static element IDs from index.html. Rust's `analytics::Event` rejects anything else. */
export type Control = "button" | "link" | "menuitem" | "tab" | "option" | "checkbox" | "input";
export type Screen = "desk" | "settings" | "issues" | "notifications" | "archived" | "workspace" | "project";
export type AnalyticsEvent =
  | { name: "app_opened" }
  | { name: "screen_viewed"; screen: Screen }
  | { name: "workspace_created"; provider: ProviderId; is_worktree: boolean }
  | { name: "conversation_created"; provider: ProviderId }
  | { name: "message_accepted"; provider: ProviderId; is_action: boolean; is_delegated: boolean }
  | { name: "turn_finished"; provider: ProviderId; outcome: "completed" | "failed" | "interrupted"; is_action: boolean; is_delegated: boolean; has_agent_activity: boolean }
  | { name: "ui_clicked"; control: Control; target: string | null; area: string | null };

/** Fire and forget: analytics never delays or fails a feature. */
export function track(event: AnalyticsEvent) {
  invoke("analytics_track", { event }).catch(() => {});
}

let shown: Screen | undefined;

/** Counts screen changes, not redraws or moves between workspaces. */
export function viewScreen(screen: Screen) {
  if (screen === shown) return;
  shown = screen;
  track({ name: "screen_viewed", screen });
}

/** `data-track` names a control whose label is user data, or a clickable part nested in one. */
const CONTROLS = [
  "[data-track]", "button", "a", "summary", "select", "input[type=checkbox]", "input[type=radio]",
  ...["button", "link", "menuitem", "menuitemcheckbox", "tab", "option", "switch", "checkbox"].map(role => `[role=${role}]`),
].join(",");

const ROLES: Record<string, Control> = {
  a: "link", link: "link", summary: "button", select: "input", input: "checkbox", switch: "checkbox",
  menuitemcheckbox: "menuitem", menuitem: "menuitem", tab: "tab", option: "option", checkbox: "checkbox",
};

/** Records every click on a control as its i18n key and the static region it sits in. Visible
 * text never leaves: a label is sent only as the catalog key whose translation it equals. */
export function trackClicks(root: Document = document) {
  const stable = new Set([...root.querySelectorAll("[id]")].map(el => el.id));
  root.addEventListener("click", event => {
    const el = (event.target as Element | null)?.closest?.(CONTROLS);
    if (!el) return;
    const role = el.getAttribute("role") ?? el.tagName.toLowerCase();
    const area = el.closest("[role=menu]") ? "menu" : ancestorId(el, stable);
    const target = (el as HTMLElement).dataset?.track ?? labelKey(el) ?? (stable.has(el.id) ? el.id : null);
    track({ name: "ui_clicked", control: ROLES[role] ?? "button", target, area });
  }, { capture: true });
}

function ancestorId(el: Element, stable: Set<string>): string | null {
  for (let node = el.parentElement; node; node = node.parentElement) {
    if (stable.has(node.id)) return node.id;
  }
  return null;
}

let catalog: Map<string, Key> | undefined;
const squash = (text: string) => text.replace(/\s+/g, " ").trim();

/** Built lazily from the active language; templates with parameters never match. */
export function labelKey(el: Element): Key | null {
  const html = el as HTMLElement;
  const marked = html.dataset?.tTitle ?? html.dataset?.t;
  if (marked && marked in PT) return marked as Key;
  catalog ??= new Map(
    (Object.keys(PT) as Key[]).filter(key => !PT[key].includes("{")).reverse().map(key => [squash(t(key)), key]),
  );
  const texts = [el.getAttribute("aria-label"), el.getAttribute("title"), el.querySelector(".ml")?.textContent, el.textContent];
  for (const text of texts) {
    const key = text && catalog.get(squash(text));
    if (key) return key;
  }
  return null;
}
