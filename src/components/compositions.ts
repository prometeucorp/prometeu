import { h } from "../../packages/design-system/src/dom";
import { icon, type IconName } from "../../packages/design-system/src/icons";
import { button, menuButton, notice } from "./primitives";
import type { Item } from "./menu";
import "./compositions.css";

/** Layout slots contain presentation controls; their callbacks belong to the host. */
export function toolbar(leading: HTMLElement[], trailing: HTMLElement[] = []) {
  const root = h("div", "desktop-toolbar");
  const start = h("div", "desktop-tools"); start.append(...leading);
  const end = h("div", "desktop-tools"); end.append(...trailing);
  end.hidden = !trailing.length;
  root.append(start, end);
  return root;
}

export function sectionHeader(options: {
  title?: string; count?: number; description?: string;
  supporting?: HTMLElement[]; actions?: HTMLElement[];
}) {
  const copy = h("div", "desktop-section-copy");
  if (options.title !== undefined) {
    const heading = h("h2", "desktop-section-title", options.title);
    if (options.count !== undefined) heading.append(h("span", "desktop-count", String(options.count)));
    copy.append(heading);
  }
  if (options.description) copy.append(h("p", "ui-hint", options.description));
  copy.append(...options.supporting ?? []);
  const root = toolbar([copy], options.actions);
  root.classList.add("desktop-section-header");
  return root;
}

export function overflowAction(options: { label: string; key: string; items: Item[]; busy?: boolean }) {
  const control = menuButton("…", () => options.items);
  control.classList.add("desktop-overflow");
  control.setAttribute("aria-label", options.label);
  control.dataset.focus = options.key;
  control.disabled = !!options.busy || options.items.every(item => item === "sep" || item.disabled);
  return control;
}

/** Text stays text. Metadata and actions are explicit slots, never inferred from DOM selectors. */
export function itemRow(options: {
  title: string; description: string; glyph: IconName; status?: string; busy?: boolean;
  metadata?: HTMLElement[]; actions?: HTMLElement[];
}) {
  const root = h("article", "desktop-item");
  root.setAttribute("aria-busy", String(!!options.busy));
  const mark = h("span", "desktop-item-mark"); mark.innerHTML = icon(options.glyph, 18);
  mark.setAttribute("aria-hidden", "true");
  const copy = h("div", "desktop-item-copy");
  const heading = h("div", "desktop-item-title"); heading.append(h("b", "", options.title));
  const description = h("p", "desktop-item-description", options.description);
  copy.append(heading, description);
  if (options.status) {
    const status = h("span", "desktop-item-status", options.status); status.setAttribute("role", "status"); copy.append(status);
  }
  const actions = h("div", "desktop-item-actions"); actions.append(...options.actions ?? []);
  root.append(mark, copy, ...options.metadata ?? [], actions);
  return { root, mark, copy, heading, description, actions };
}

export type ListState = { kind: "ready" } | { kind: "empty" | "loading"; text: string }
  | { kind: "error"; text: string; retry?: { label: string; run: () => void; key?: string } };

export function listState(state: ListState) {
  const root = h("div", "desktop-list-state");
  root.dataset.state = state.kind;
  root.hidden = state.kind === "ready";
  if (state.kind === "error") {
    root.append(notice(state.text, "error"));
    if (state.retry) {
      const retry = button(state.retry.label, state.retry.run);
      if (state.retry.key) retry.dataset.focus = state.retry.key;
      root.append(retry);
    }
  } else if (state.kind !== "ready") {
    const text = h("p", "ui-hint", state.text); text.setAttribute("role", "status"); root.append(text);
  }
  return root;
}
