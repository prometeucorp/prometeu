/// Shared view helpers.

export const $ = (id: string) => document.getElementById(id)!;

import { h } from "../packages/design-system/src/dom";
export { h };

/// Build static application-owned markup, including icons. Naming this helper makes innerHTML use explicit during review.
export function template(tag: string, className: string, html: string): HTMLElement {
  const node = h(tag, className);
  node.innerHTML = html;
  return node;
}

/// A full-page empty state with a title, explanation, and optional action.
export function empty(title: string, text: string, action?: [string, () => void]): HTMLElement {
  const box = template("div", "iempty", `<h3></h3><p></p>`);
  box.children[0].textContent = title;
  box.children[1].textContent = text;
  if (action) {
    const b = h("button", "outline md", action[0]);
    b.addEventListener("click", action[1]);
    box.append(b);
  }
  return box;
}

/// Coalesce event bursts into one call with the latest arguments.
export function debounce<A extends unknown[]>(ms: number, fn: (...args: A) => void) {
  let timer = 0;
  return (...args: A) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), ms);
  };
}

/// Show at most two duration units, such as 3h 15m or 3d 4h.
export function span(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return h ? `${d}d ${h}h` : `${d}d`;
  if (h) return m ? `${h}h ${m}m` : `${h}h`;
  return `${m}m`;
}
