import { button } from "../primitives";
import * as menu from "../menu";
import { h } from "../../util";

export type UsagePanel = {
  title: string; description?: string;
  metrics?: { label: string; value: string }[];
  groups?: { title: string; description?: string; rows: { label: string; value: string; detail?: string }[] }[];
  actions?: { label: string; disabled?: boolean; run: () => void }[];
};

/** A read-only measurement panel; callbacks and translated values come from its host. */
export function usagePanel(data: UsagePanel): HTMLElement {
  const root = h("div", "menu usage-popover");
  root.tabIndex = -1; root.setAttribute("role", "dialog"); root.setAttribute("aria-label", data.title);
  root.append(h("h3", "usage-title", data.title));
  if (data.description) root.append(h("p", "usage-description", data.description));
  if (data.metrics?.length) {
    const metrics = h("dl", "usage-metrics");
    for (const row of data.metrics) metrics.append(h("dt", "", row.label), h("dd", "", row.value));
    root.append(metrics);
  }
  for (const group of data.groups ?? []) {
    if (!group.rows.length) continue;
    const section = h("section", "usage-group"); section.append(h("h4", "", group.title));
    if (group.description) section.append(h("p", "usage-description", group.description));
    for (const row of group.rows) {
      const entry = h("div", "usage-row");
      const label = h("span", "usage-row-label", row.label);
      if (row.detail) label.append(h("small", "", row.detail));
      entry.append(label, h("span", "usage-row-value", row.value)); section.append(entry);
    }
    root.append(section);
  }
  if (data.actions?.length) {
    const actions = h("div", "usage-actions");
    for (const action of data.actions) {
      const control = button(action.label, () => { menu.close(); action.run(); }, "ghost");
      control.disabled = action.disabled ?? false; actions.append(control);
    }
    root.append(actions);
  }
  return root;
}

export function openUsagePanel(anchor: HTMLElement, data: UsagePanel, closed: () => void = () => {}) {
  const panel = usagePanel(data);
  menu.openPanel(anchor, panel, event => {
    if (event.key !== "Tab") return;
    const controls = [...panel.querySelectorAll<HTMLButtonElement>("button:enabled")];
    if (!controls.length) { menu.close(); return; }
    // WebKit can skip native buttons when full keyboard access is off.
    event.preventDefault(); event.stopPropagation();
    const index = controls.indexOf(document.activeElement as HTMLButtonElement);
    controls[index < 0 ? event.shiftKey ? controls.length - 1 : 0 : (index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length].focus();
  }, closed);
  panel.focus({ preventScroll: true });
  return panel;
}

export type ContextGauge = {
  label: string; percent: number; band: "normal" | "warn" | "hot"; panel: UsagePanel;
};

/** Uses the Desktop meter styling also consumed by the global quota bar. */
export function contextGauge() {
  let data: ContextGauge | null = null;
  let panel: HTMLElement | null = null;
  const root = button("", () => {
    if (!data) return;
    if (panel?.isConnected) { menu.close(); return; }
    root.setAttribute("aria-expanded", "true");
    panel = openUsagePanel(root, data.panel, () => { panel = null; root.setAttribute("aria-expanded", "false"); });
  }, "ghost");
  root.classList.add("context-gauge"); root.hidden = true;
  root.setAttribute("aria-haspopup", "dialog"); root.setAttribute("aria-expanded", "false");
  const meter = h("span", "meter"); meter.setAttribute("aria-hidden", "true");
  const fill = h("i", ""); meter.append(fill);
  const label = h("span", "context-gauge-label"); root.append(meter, label);
  const close = () => { if (panel?.isConnected) menu.close(); panel = null; };
  return {
    root, close,
    update(next: ContextGauge | null) {
      data = next; root.hidden = !next;
      if (!next) return close();
      root.setAttribute("aria-label", next.label); root.title = next.label;
      label.textContent = next.label;
      meter.className = `meter${next.band === "normal" ? "" : ` ${next.band}`}`;
      fill.style.width = `${Math.max(0, Math.min(100, next.percent))}%`;
      if (panel?.isConnected) {
        const focused = panel.contains(document.activeElement) ? document.activeElement : null;
        const action = focused?.closest("button")?.textContent;
        const scroll = panel.scrollTop;
        panel.replaceChildren(...usagePanel(next.panel).childNodes);
        panel.setAttribute("aria-label", next.panel.title);
        if (focused) {
          const control = [...panel.querySelectorAll<HTMLButtonElement>("button:enabled")].find(button => button.textContent === action);
          (control ?? panel).focus({ preventScroll: true });
        }
        panel.scrollTop = scroll;
      }
    },
  };
}
