import { ChatView, type Info } from "./chat";
import { arrange } from "./desk-layout";
import { icon } from "./icons";
import { t } from "./i18n";
import { label, pending, tabLabel, type Board, type Tab, type Workspace } from "./types";
import { $, empty, h, template } from "./util";

/// Display local conversations as independent ChatViews with locally persisted order, size, and collapsed state. Use pointer gestures because WebKit resizing and Tauri's native file-drag interception prevent reliable CSS/HTML alternatives.

export type Ctx = {
  say: (text: string, isError?: boolean) => void;
  board: () => Board;
  /// Composer context for a tab; see chat.ts.
  info: (tab: string) => Info;
  /// Open the workspace at the panel's tab.
  open: (ws: Workspace, tab: string) => void;
  /// The empty desk offers the launcher.
  create: () => void;
  newConversation: (workspace: string) => void;
  looked: () => void;
};

type Tile = { el: HTMLElement; head: HTMLElement; view: ChatView };
type Layout = { order: string[]; sizes: Record<string, [number, number]>; hidden: string[] };
type Pair = { w: Workspace; tab: Tab };

const STORE = "prometeu:mesa";
/// Movement below this threshold remains a click; create the drag ghost only afterward.
const DRAG_START = 4;

function load(): Layout {
  try {
    const raw = JSON.parse(localStorage.getItem(STORE) ?? "");
    return {
      order: Array.isArray(raw.order) ? raw.order : [],
      sizes: raw.sizes ?? {},
      hidden: Array.isArray(raw.hidden) ? raw.hidden : [],
    };
  } catch {
    return { order: [], sizes: {}, hidden: [] };
  }
}

let ctx: Ctx;
let layout: Layout;
let shown = false;
/// Board updates must not restore the previous DOM order during a drag.
let dragging = false;
/// Keep one view per tab. Detach hidden panels and retain them until their tabs leave the board.
const tiles = new Map<string, Tile>();

const save = () => localStorage.setItem(STORE, JSON.stringify(layout));

export function init(context: Ctx) {
  ctx = context;
  layout = load();
}

/// Include active local workspaces; archived, cleaned, preparing, and remote workspaces use other lifecycle paths.
const live = (board: Board): Pair[] =>
  board.workspaces
    .filter((w) => !w.archived && !w.cleaned && !w.remote && !pending(w))
    .flatMap((w) => w.tabs.map((tab) => ({ w, tab })));

const wsOf = (tab: string) => ctx.board().workspaces.find((w) => w.tabs.some((x) => x.id === tab));

export const visible = (tab: string): boolean => shown && tiles.has(tab) && !tiles.get(tab)!.el.hidden;

export function show() {
  shown = true;
  $("deskView").hidden = false;
}

/// Detach desk panels when leaving so the workspace view owns the active conversation stream.
export function hide() {
  if (!shown) return;
  shown = false;
  $("deskView").hidden = true;
  for (const { view } of tiles.values()) view.detach();
}

/// Reconcile panel membership and update headers/composers. Collapsed panels stay hidden and detached until reopened.
export function draw() {
  if (!shown) return;
  const host = $("tiles");
  const board = ctx.board();
  const pairs = live(board);
  const byTab = new Map(pairs.map((p) => [p.tab.id, p]));
  const alive = new Set(board.workspaces.flatMap((w) => w.tabs.map((tab) => tab.id)));
  for (const [id, tile] of tiles) {
    if (byTab.has(id)) continue;
    tile.view.dispose(!alive.has(id));
    tile.el.remove();
    tiles.delete(id);
    delete layout.sizes[id];
  }
  layout.order = arrange([...byTab.keys()], layout.order);
  layout.hidden = layout.hidden.filter((id) => byTab.has(id));
  const hidden = new Set(layout.hidden);
  for (const id of layout.order) {
    const { w, tab } = byTab.get(id)!;
    let tile = tiles.get(id);
    if (!tile) tile = mount(id);
    tile.el.hidden = hidden.has(id);
    if (tile.el.hidden) {
      if (tile.view.current()) tile.view.detach();
      continue;
    }
    if (!tile.view.current()) void tile.view.attach(id);
    paintHead(tile.head, w, tab);
    tile.view.refresh();
  }
  // Move DOM nodes only when order changes; unnecessary moves would disrupt composer focus on board events.
  const have = [...host.querySelectorAll<HTMLElement>(".tile")].map((el) => el.dataset.tab);
  if (!dragging && layout.order.some((id, i) => id !== have[i])) {
    host.append(...layout.order.map((id) => tiles.get(id)!.el));
  }
  drawBar(byTab, hidden);
  host.querySelector(".iempty")?.remove();
  if (!pairs.length) host.append(empty(t("desk.empty"), t("desk.empty.hint"), [t("rail.create"), ctx.create]));
  save();
}

/// The strip lists every conversation in desk order and toggles its panel visibility.
function drawBar(byTab: Map<string, Pair>, hidden: Set<string>) {
  $("deskbar").replaceChildren(
    ...layout.order.map((id) => {
      const { w, tab } = byTab.get(id)!;
      const off = hidden.has(id);
      const b = template("button", "tab" + (off ? "" : " on"), `<i class="dot"></i><span></span><span class="n"></span>`);
      (b.children[0] as HTMLElement).style.background = `var(--dot-${tab.status})`;
      b.children[1].textContent = w.title;
      b.children[2].textContent = tabLabel(w, tab);
      b.title = t(off ? "desk.chip.show" : "desk.chip.hide");
      b.dataset.tab = id;
      b.addEventListener("click", () => toggle(id));
      return b;
    }),
  );
}

function toggle(id: string) {
  const hidden = new Set(layout.hidden);
  if (!hidden.delete(id)) hidden.add(id);
  layout.hidden = [...hidden];
  draw();
  ctx.looked();
}

function mount(id: string): Tile {
  const el = template(
    "div",
    "tile",
    `<div class="tile-head"><i class="dot"></i><b></b><span class="ttab"></span><span class="spacer"></span><button class="ico sm tmin"></button><button class="ico sm topen"></button></div><div class="chatwrap"></div>`,
  );
  el.dataset.tab = id;
  const head = el.querySelector<HTMLElement>(".tile-head")!;
  const min = head.querySelector<HTMLElement>(".tmin")!;
  min.innerHTML = icon("chevron-down", 14);
  min.title = t("desk.min");
  min.addEventListener("click", () => toggle(id));
  const open = head.querySelector<HTMLElement>(".topen")!;
  open.innerHTML = icon("arrow-right", 14);
  open.title = t("desk.open");
  open.addEventListener("click", () => {
    const w = wsOf(id);
    if (w) ctx.open(w, id);
  });
  const size = layout.sizes[id];
  if (size) {
    el.style.width = `${size[0]}px`;
    el.style.height = `${size[1]}px`;
  }
  const view = new ChatView();
  view.open(el.querySelector<HTMLElement>(".chatwrap")!, { say: ctx.say, info: () => ctx.info(id), newConversation: ctx.newConversation });
  drag(el, head);
  grip(el, id);
  $("tiles").append(el);
  const tile = { el, head, view };
  tiles.set(id, tile);
  return tile;
}

function paintHead(head: HTMLElement, w: Workspace, tab: Tab) {
  head.className = `tile-head s-${tab.status}`;
  head.querySelector("b")!.textContent = w.title;
  head.querySelector(".ttab")!.textContent = tabLabel(w, tab);
  head.title = `${label(tab.status)} · ${t("desk.head.title")}`;
}

/// Listen for pointer movement and completion on document. Reordering DOM nodes can release element pointer capture; pointerup and pointercancel share cleanup.
/// Cancelling pointerdown does not stop WebKit from starting a text selection, so block selectstart for the whole gesture.
function gesture(move: (m: PointerEvent) => void, stop: () => void) {
  const select = (e: Event) => e.preventDefault();
  const end = () => {
    document.removeEventListener("pointermove", move);
    document.removeEventListener("pointerup", end);
    document.removeEventListener("pointercancel", end);
    document.removeEventListener("selectstart", select);
    stop();
  };
  document.addEventListener("selectstart", select);
  document.addEventListener("pointermove", move);
  document.addEventListener("pointerup", end);
  document.addEventListener("pointercancel", end);
}

/// Drag headers to reorder. A ghost follows the pointer while the real panel marks its destination; disable its pointer events so hit testing sees the panel underneath.
function drag(el: HTMLElement, head: HTMLElement) {
  head.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || (e.target as Element).closest("button")) return;
    e.preventDefault();
    const x0 = e.clientX;
    const y0 = e.clientY;
    let ghost: HTMLElement | null = null;
    const start = () => {
      getSelection()?.removeAllRanges();
      const box = el.getBoundingClientRect();
      ghost = h("div", "tile ghost");
      ghost.append(head.cloneNode(true));
      ghost.style.width = `${box.width}px`;
      ghost.style.height = `${box.height}px`;
      ghost.style.left = `${box.left}px`;
      ghost.style.top = `${box.top}px`;
      document.body.append(ghost);
      el.classList.add("dragging");
      dragging = true;
    };
    gesture(
      (m) => {
        if (!ghost) {
          if (Math.hypot(m.clientX - x0, m.clientY - y0) < DRAG_START) return;
          start();
        }
        ghost!.style.transform = `translate(${m.clientX - x0}px, ${m.clientY - y0}px)`;
        const over = document.elementFromPoint(m.clientX, m.clientY)?.closest<HTMLElement>(".tile");
        if (!over || over === el) return;
        const box = over.getBoundingClientRect();
        if (m.clientX < box.left + box.width / 2) over.before(el);
        else over.after(el);
      },
      () => {
        if (!ghost) return;
        ghost.remove();
        el.classList.remove("dragging");
        dragging = false;
        layout.order = [...$("tiles").querySelectorAll<HTMLElement>(".tile")].map((x) => x.dataset.tab!);
        save();
      },
    );
  });
}

/// Resize with the corner handle, storing CSS-clamped dimensions. Double-click restores the default size.
function grip(el: HTMLElement, id: string) {
  const handle = template(
    "div",
    "tile-grip",
    `<svg viewBox="0 0 10 10" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M9 1 1 9M9 5 5 9"/></svg>`,
  );
  handle.title = t("desk.grip");
  handle.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const x0 = e.clientX;
    const y0 = e.clientY;
    const w0 = el.offsetWidth;
    const h0 = el.offsetHeight;
    let moved = false;
    el.classList.add("sizing");
    gesture(
      (m) => {
        if (!moved) {
          if (m.clientX === x0 && m.clientY === y0) return;
          getSelection()?.removeAllRanges();
          moved = true;
        }
        el.style.width = `${w0 + m.clientX - x0}px`;
        el.style.height = `${h0 + m.clientY - y0}px`;
      },
      () => {
        el.classList.remove("sizing");
        layout.sizes[id] = [el.offsetWidth, el.offsetHeight];
        save();
      },
    );
  });
  handle.addEventListener("dblclick", () => {
    el.style.width = "";
    el.style.height = "";
    delete layout.sizes[id];
    save();
  });
  el.append(handle);
}

/// Drop files only into the conversation panel under the pointer when it accepts attachments.
export function dropTarget(el: Element | null): { host: HTMLElement; put: (paths: string[]) => void; wait: () => () => void } | null {
  const id = el?.closest<HTMLElement>(".tile")?.dataset.tab;
  const tile = id ? tiles.get(id) : undefined;
  const target = tile?.view.fileDropTarget();
  return tile && target ? { host: tile.el, ...target } : null;
}
