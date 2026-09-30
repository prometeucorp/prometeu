import { icon } from "./icons.js";

/// Context menus group workspace actions such as rename, stage changes and archive.
export type Item =
  | "sep"
  | {
      label: string;
      /// A rendered glyph supports both computed stage icons and named icons.
      glyph?: string;
      /// Display only keyboard shortcuts that actually exist.
      hint?: string;
      badge?: string;
      checked?: boolean;
      danger?: boolean;
      /// Keep unavailable actions visible but disabled so users can discover them.
      disabled?: boolean;
      sub?: Item[];
      run?: () => void;
    };

let root: HTMLElement | null = null;
let panelKey: ((event: KeyboardEvent) => void) | undefined;
let afterClose: (() => void) | undefined;
/// The mouse or keyboard selection activated by Enter.
let sel: HTMLElement | null = null;
let keyboardFocus = false;
let previousFocus: HTMLElement | null = null;
const submenus = new WeakMap<HTMLElement, () => HTMLElement>();
const openers = new WeakMap<HTMLElement, HTMLElement>();

export const isOpen = () => root !== null;

/// Notify listeners after the menu closes so deferred redraws cannot move controls during a click.
const closers = new Set<() => void>();
export const onClose = (fn: () => void) => { closers.add(fn); return () => { closers.delete(fn); }; };

export function close() {
  const was = root !== null;
  const restore = keyboardFocus && root?.contains(document.activeElement);
  root?.remove();
  root = null;
  panelKey = undefined;
  const done = afterClose;
  afterClose = undefined;
  done?.();
  sel = null;
  document.removeEventListener("mousedown", onDown, true);
  document.removeEventListener("keydown", onKey, true);
  window.removeEventListener("blur", close);
  if (restore && previousFocus?.isConnected) previousFocus.focus();
  previousFocus = null;
  // Reopening a selector is not a close. Defer notification until the next tick and suppress it if
  // another panel opens.
  if (was) setTimeout(() => root === null && closers.forEach((fn) => fn()), 0);
}

function onDown(e: MouseEvent) {
  if (!(e.target as HTMLElement).closest(".menu")) close();
}

/// The open menu owns Escape, arrow navigation and Enter. With no selection, Enter stays with the
/// focused editor.
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    close();
  } else if (panelKey) {
    panelKey(e);
  } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    e.stopPropagation();
    move(e.key === "ArrowDown" ? 1 : -1);
  } else if (e.key === "Enter" && sel) {
    e.preventDefault();
    e.stopPropagation();
    sel.click();
  } else if (keyboardFocus && (e.key === "Home" || e.key === "End") && root) {
    e.preventDefault(); e.stopPropagation();
    const rows = rowsIn(sel?.parentElement ?? root);
    select(e.key === "Home" ? rows[0] : rows[rows.length - 1]);
  } else if (e.key === "ArrowRight" && sel && submenus.has(sel)) {
    e.preventDefault(); e.stopPropagation();
    select(rowsIn(submenus.get(sel)!())[0]);
  } else if (e.key === "ArrowLeft" && sel?.parentElement && openers.has(sel.parentElement)) {
    e.preventDefault(); e.stopPropagation();
    const panel = sel.parentElement, opener = openers.get(panel)!;
    panel.remove(); opener.setAttribute("aria-expanded", "false"); select(opener);
  } else if (keyboardFocus && e.key === "Tab") {
    // Autocomplete keeps editor focus and uses Tab to accept the selected option.
    close();
  }
}

function select(row: HTMLElement | null | undefined) {
  sel?.classList.remove("sel");
  sel = row ?? null;
  sel?.classList.add("sel");
  if (keyboardFocus) sel?.focus();
}

function rowsIn(panel: HTMLElement) {
  return [...panel.children].filter((el): el is HTMLElement => el.matches(".mrow:not(.off)"));
}

/// Wrap through top-level menu items; submenus use pointer navigation.
function move(delta: number) {
  if (!root) return;
  const rows = rowsIn(sel?.isConnected ? sel.parentElement! : root);
  if (!rows.length) return;
  const at = sel ? rows.indexOf(sel) : -1;
  const next = at < 0 ? (delta > 0 ? 0 : rows.length - 1) : (at + delta + rows.length) % rows.length;
  select(rows[next]);
  rows[next].scrollIntoView({ block: "nearest" });
}

export type Where = {
  x: number;
  y: number;
  /// Open upward to keep the editor below the menu visible.
  above?: boolean;
};

/// Open at the click position, clamped to the viewport. `cls` allows a consumer-specific panel size.
/// Returns the panel so a consumer that reopens it after a choice can keep its scroll offset.
export function openAt(at: Where, items: Item[], cls?: string, onClosed?: () => void, focus = false): HTMLElement {
  close();
  keyboardFocus = focus;
  previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  root = panel(items);
  if (cls) root.classList.add(cls);
  afterClose = onClosed;
  (document.querySelector("dialog[open]") ?? document.body).append(root);
  place(root, at.x, at.y, at.above);
  document.addEventListener("mousedown", onDown, true);
  document.addEventListener("keydown", onKey, true);
  window.addEventListener("blur", close);
  if (focus) root.focus();
  return root;
}

/** Join the shared menu lifetime with a panel that owns its internal keyboard interaction. */
export function openPanel(anchor: HTMLElement, panel: HTMLElement, key: (event: KeyboardEvent) => void, closed: () => void) {
  close();
  root = panel; keyboardFocus = true; previousFocus = anchor; panelKey = key;
  afterClose = closed;
  (anchor.closest("dialog[open]") ?? document.body).append(panel);
  const reposition = () => {
    if (root !== panel) return;
    const at = anchor.getBoundingClientRect();
    place(panel, at.left, at.bottom + 4);
  };
  reposition();
  document.addEventListener("mousedown", onDown, true);
  document.addEventListener("keydown", onKey, true);
  window.addEventListener("blur", close);
  return reposition;
}

function place(el: HTMLElement, x: number, y: number, above = false) {
  const { width, height } = el.getBoundingClientRect();
  const top = above ? y - height : y;
  el.style.left = `${Math.max(8, Math.min(x, innerWidth - width - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(top, innerHeight - height - 8))}px`;
}

function panel(items: Item[]): HTMLElement {
  const box = document.createElement("div");
  box.className = "menu";
  box.tabIndex = -1;
  box.setAttribute("role", "menu");
  // Each panel has one submenu; opening another replaces it.
  let sub: HTMLElement | null = null;
  let opener: HTMLElement | null = null;
  const drop = () => {
    sub?.remove();
    sub = null;
    opener?.setAttribute("aria-expanded", "false");
  };

  for (const item of items) {
    if (item === "sep") {
      box.append(document.createElement("hr"));
      continue;
    }
    const row = document.createElement("button");
    row.type = "button";
    row.tabIndex = -1;
    row.className = "mrow" + (item.danger ? " danger" : "") + (item.disabled ? " off" : "");
    row.disabled = item.disabled ?? false;
    row.setAttribute("role", item.checked === undefined ? "menuitem" : "menuitemcheckbox");
    if (item.checked !== undefined) row.setAttribute("aria-checked", String(item.checked));
    row.innerHTML =
      `<span class="mg">${item.glyph ?? ""}</span><span class="ml"></span>` +
      `<span class="mh"></span>${item.sub ? icon("chevron-right", 14) : ""}` +
      `<span class="mc">${item.checked ? icon("check", 14) : ""}</span>`;
    row.children[1].textContent = item.label;
    row.children[2].textContent = item.hint ?? "";
    if (item.badge) {
      const badge = document.createElement("span");
      badge.className = "mbadge";
      badge.textContent = item.badge;
      row.children[1].after(badge);
    }
    box.append(row);

    const showSubmenu = () => {
      drop();
      // Keep fixed-position submenus inside their parent so closing the menu removes them automatically.
      sub = panel(item.sub!);
      box.append(sub);
      opener = row; openers.set(sub, row);
      row.setAttribute("aria-expanded", "true");
      const at = row.getBoundingClientRect();
      // Anchor the submenu to its row rather than the pointer position.
      place(sub, at.right - 4, at.top - 6);
      return sub;
    };
    if (item.sub && !item.disabled) {
      row.setAttribute("aria-haspopup", "menu"); row.setAttribute("aria-expanded", "false");
      submenus.set(row, showSubmenu);
      row.addEventListener("click", () => select(rowsIn(showSubmenu())[0]));
    }
    row.addEventListener("mouseenter", () => {
      if (!item.disabled) select(row);
      if (item.sub && !item.disabled) showSubmenu(); else drop();
    });
    if (item.run) {
      row.addEventListener("click", () => {
        close();
        item.run!();
      });
    }
  }
  return box;
}
