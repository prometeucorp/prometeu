import { fromBack, t } from "./i18n";
import * as background from "./background";
import { marksInterval } from "./git-refresh";
import { invoke } from "./ipc";
import { fileIcon, icon } from "./icons";
import * as menu from "./menu";
import type { PathEntry } from "./paths";
import { mac } from "./platform";
import * as rename from "./rename";
import { gitMarks, latestOnly, type GitMarks, type GoneEntry } from "./tree-git";
import { parentOf, rootMenu, treeMenu, type Entry } from "./tree-menu";
import { relocate } from "./tree-moves";
import { confirmDialog } from "./ui";
import { $, debounce } from "./util";

/// Load worktree folders on demand in the side panel so large repositories do not require a full tree scan.

/// Preserve expanded folders across board redraws.
const openDirs = new Set<string>();

/// The file the viewer is showing, whatever opened it: this tree, Changes, a tab or the dock. The
/// workspace owns that choice and reports it through `select`; the tree only draws it.
let selected: string | null = null;

/// The row a context menu is acting on, file or folder, while that menu is open. A folder has no
/// open file to mark, and a file may differ from the one on screen; without this the menu would sit
/// over an unmarked row, leaving which entry it acts on to guesswork.
let target: string | null = null;

/// What the tree cannot know by itself: the absolute root, and whether a conversation takes files.
export type Host = {
  root: string | null;
  attach: ((absolute: string) => void) | null;
  /// Explain a conversation that exists but refuses attachments; absent in the project-only view.
  attachHint?: string;
  copy: (text: string) => void;
  reveal: (path: string) => void;
};

let openFile: (path: string) => void = () => {};
let workspace: () => string | null = () => null;
let host: () => Host = () => ({ root: null, attach: null, copy: () => {}, reveal: () => {} });
/// Open tabs and drafts of `id` follow an entry the tree renamed (`to`) or trashed (`to` is null).
let moved: (id: string, from: string, to: string | null) => void = () => {};
let say: (message: string, error?: boolean) => void = () => {};
/// A name is being typed in place; redraws wait so they do not drop the input.
let editing = false;
let marks: GitMarks = gitMarks([]);
/// Agents and terminals change files without board events, so visible marks refresh on a timer.
let marksTimer: ReturnType<typeof setTimeout> | null = null;
let delayedRedraw: ReturnType<typeof setTimeout> | null = null;
function scheduleMarks() {
  if (marksTimer) clearTimeout(marksTimer);
  const interval = marksInterval(background.currentOrDocument());
  if (interval === null) return;
  marksTimer = setTimeout(() => {
    const id = workspace();
    if (id && background.foreground(background.currentOrDocument()) && $("tree").offsetParent) void repaint(id);
    scheduleMarks();
  }, interval);
}

export function init(ctx: {
  openFile: (path: string) => void;
  workspace: () => string | null;
  host: () => Host;
  moved: (id: string, from: string, to: string | null) => void;
  say: (message: string, error?: boolean) => void;
}) {
  openFile = ctx.openFile;
  workspace = ctx.workspace;
  host = ctx.host;
  moved = ctx.moved;
  say = ctx.say;
  $("tree").addEventListener("contextmenu", (e) => {
    if ((e.target as HTMLElement).closest(".treerow, .treeedit")) return;
    e.preventDefault();
    if (!workspace()) return;
    menu.openAt({ x: e.clientX, y: e.clientY }, rootMenu({ create: (parent, dir) => void create(parent, dir), reveal: host().reveal }));
  });
  $("collapse").addEventListener("click", () => {
    openDirs.clear();
    redraw();
  });
  let wasForeground = background.foreground(background.currentOrDocument());
  background.subscribe((context) => {
    const id = workspace();
    const foreground = background.foreground(context);
    if (id && foreground && !wasForeground && $("tree").offsetParent) redraw();
    wasForeground = foreground;
    scheduleMarks();
  });
  window.addEventListener("focus", () => {
    if (!background.hasObservation()) {
      if (workspace() && $("tree").offsetParent) redraw();
      scheduleMarks();
    }
  });
  scheduleMarks();
}

/// User clicks redraw immediately to avoid visible lag.
export function redraw() {
  if (delayedRedraw) clearTimeout(delayedRedraw);
  delayedRedraw = null;
  if (!background.foreground(background.currentOrDocument())) return;
  const id = workspace();
  if (id) void draw(id);
}

/// Reset expanded folders when switching workspaces.
export function reset() {
  openDirs.clear();
  selected = null;
  target = null;
}

/// Debounce board-driven refreshes because each open folder requires list_dir; agent bursts would otherwise repeat identical IPC work.
export function redrawSoon() {
  if (delayedRedraw) clearTimeout(delayedRedraw);
  delayedRedraw = setTimeout(redraw, 200);
}

/// Saving an open file changes marks but does not change the tree's file list.
export const refreshMarksSoon = debounce(200, () => {
  const id = workspace();
  if (id) void repaint(id);
});

/// Mark the file the viewer shows, or none. Paths are relative to the tree's root, as list_dir
/// returns them. The row is only marked, never scrolled into view: the person may be browsing
/// elsewhere in the tree.
export function select(path: string | null) {
  selected = path;
  markRows("selected", path);
}

function markRows(cls: "selected" | "targeted", path: string | null) {
  for (const row of $("tree").querySelectorAll(`.treerow.${cls}`)) row.classList.remove(cls);
  if (path) $("tree").querySelector(`.treerow[data-path="${CSS.escape(path)}"]`)?.classList.add(cls);
}

function aim(path: string | null) {
  target = path;
  markRows("targeted", path);
}

let drawn = 0;

/// Marks load before the rows because deleted files add rows of their own. Rows are built off
/// screen and swapped in at once: only the latest draw for the workspace still on screen may
/// replace the tree, and never while a name is typed in place: the edit redraws when it ends.
async function draw(id: string) {
  if (editing) return;
  const mine = ++drawn;
  const [entries] = await Promise.all([invoke("list_dir", { id, rel: "" }), loadMarks(id)]);
  const rows = document.createDocumentFragment();
  await fill(id, "", rows, 0, entries);
  if (mine !== drawn || editing || workspace() !== id) return;
  $("tree").replaceChildren(rows);
}

/// A slow repository must not stack scans: a timer tick waits for the one still running.
let loading: Promise<void> | null = null;

/// A draw and the timer's repaint can scan at once; only the latest scan started replaces the marks.
const scan = latestOnly();

async function loadMarks(id: string) {
  await scan(invoke("tree_git_status", { id }).catch(() => []), (files) => {
    if (workspace() === id) marks = gitMarks(files);
  });
}

/// Repaint rows in place, and rebuild them only when a file was deleted or restored.
async function repaint(id: string) {
  if (!background.foreground(background.currentOrDocument())) return;
  if (loading) return;
  const before = marks.goneKey;
  loading = loadMarks(id).finally(() => (loading = null));
  await loading;
  if (workspace() !== id) return;
  if (marks.goneKey !== before) await draw(id);
  else paintAll();
}

function paintAll() {
  for (const row of $("tree").querySelectorAll<HTMLElement>(".treerow")) paint(row);
}

/// Tint the row and show the mark beside the name; folders show a dot for changes inside them.
function paint(row: HTMLElement) {
  const dir = row.dataset.dir === "1";
  const gone = row.dataset.gone === "1";
  const mark = gone ? "D" : marks.mark(row.dataset.path!, dir);
  const badge = row.children[2] as HTMLElement;
  if (mark) row.dataset.git = mark;
  else delete row.dataset.git;
  badge.textContent = mark ? (dir && !gone ? "•" : mark) : "";
  badge.title = mark ? t(dir && !gone ? "tree.git.folder" : `tree.git.${mark}`) : "";
}

/// Merge deleted entries into the listing in list_dir's order: folders first, then by name.
/// A name that changed type (a deleted file where a folder now stands, or the reverse) keeps both rows.
function withGone(rel: string, listed: PathEntry[]): (PathEntry | GoneEntry)[] {
  const kind = (entry: PathEntry) => `${entry.dir ? "d" : "f"}${entry.name}`;
  const names = new Set(listed.map(kind));
  const gone = marks.gone(rel).filter(entry => !names.has(kind(entry)));
  if (!gone.length) return listed;
  const key = (entry: PathEntry) => `${entry.dir ? 0 : 1}${entry.name.toLowerCase()}`;
  return [...listed, ...gone].sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
}

async function fill(id: string, rel: string, into: HTMLElement | DocumentFragment, depth: number, listed?: PathEntry[]) {
  const entries = withGone(rel, listed ?? await invoke("list_dir", { id, rel }));
  for (const entry of entries) {
    const gone = "gone" in entry;
    const row = document.createElement("button");
    row.className = "treerow";
    row.dataset.path = entry.path;
    if (entry.path === selected) row.classList.add("selected");
    if (entry.path === target) row.classList.add("targeted");
    row.style.paddingLeft = `${14 + depth * 20}px`;
    row.dataset.dir = entry.dir ? "1" : "0";
    row.dataset.depth = String(depth);
    if (gone) row.dataset.gone = "1";
    row.innerHTML = `<span class="tw"></span><span class="tn"></span><span class="tg"></span><span class="tc"></span>`;
    // Expanded folders change their icon and hover chevron.
    const glyph = (open: boolean) => {
      row.children[0].innerHTML = entry.dir ? icon(open ? "folder-open" : "folder") : fileIcon(entry.name);
      row.children[3].innerHTML = entry.dir ? icon(open ? "chevron-down" : "chevron-right", 14) : "";
    };
    glyph(false);
    row.children[1].textContent = entry.name;
    paint(row);
    into.append(row);

    const kids = document.createElement("div");
    let flip = async () => {};
    if (entry.dir) {
      kids.hidden = true;
      into.append(kids);
      flip = async () => {
        const isOpen = openDirs.has(entry.path);
        if (isOpen) {
          openDirs.delete(entry.path);
        } else {
          openDirs.add(entry.path);
          if (!kids.childElementCount) await fill(id, entry.path, kids, depth + 1, gone ? [] : undefined);
        }
        kids.hidden = isOpen;
        glyph(!isOpen);
      };
    }

    // Replace the engine's page menu, which offers web actions over the file's name. Scope it to the
    // row so the viewer and the composer keep the native editing menu.
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const at = host();
      menu.openAt(
        { x: e.clientX, y: e.clientY },
        treeMenu(entry, {
          root: at.root,
          expanded: openDirs.has(entry.path),
          gone,
          attachHint: at.attachHint,
          hooks: {
            open: openFile,
            toggle: () => void flip(),
            attach: at.attach,
            copy: at.copy,
            reveal: at.reveal,
            create: (parent, dir) => void create(parent, dir),
            rename: startRename,
            trash: (entry) => void trash(entry),
            restore: (path) => void restore(path),
          },
        }),
        undefined,
        () => aim(null),
      );
      // After openAt: opening closes any previous menu, whose callback clears the old mark.
      aim(entry.path);
    });

    // A focused row takes the file manager's shortcuts; a deleted one has nothing to rename or trash.
    row.addEventListener("keydown", (e) => {
      if (gone) return;
      if (e.key === "F2") startRename(entry);
      else if (mac ? e.metaKey && e.key === "Backspace" : e.key === "Delete") void trash(entry);
      else return;
      e.preventDefault();
    });

    // Opening a file marks it through the workspace's `select`, like every other route to the viewer.
    row.addEventListener("click", () => {
      if (entry.dir) void flip();
      else if (!gone) openFile(entry.path);
    });

    if (entry.dir && openDirs.has(entry.path)) {
      glyph(true);
      kids.hidden = false;
      await fill(id, entry.path, kids, depth + 1, gone ? [] : undefined);
    }
  }
}

/* File manager actions. Each one captures the tree's id when it starts: the person may switch
   workspaces while a name is typed or a confirmation is open. */

const join = (parent: string, name: string) => (parent ? `${parent}/${name}` : name);
/// A deleted entry can share its path with one now on disk of the other kind; actions take the live one.
const rowOf = (path: string) => $("tree").querySelector<HTMLElement>(`.treerow:not([data-gone])[data-path="${CSS.escape(path)}"]`);
const fail = (error: unknown) => say(fromBack(error), true);

/// Type a name in place: an indented input that replaces `hide` (or sits before `before`) until
/// Enter, Escape or blur. `done` receives the new name, or null when nothing changed.
function editName(
  at: { into: Element; before: Element | null; depth: number; hide?: HTMLElement },
  value: string,
  file: boolean,
  done: (name: string | null) => Promise<void>,
) {
  const wrap = document.createElement("div");
  wrap.className = "treeedit";
  wrap.style.paddingLeft = `${14 + at.depth * 20}px`;
  const slot = document.createElement("span");
  wrap.append(slot);
  at.into.insertBefore(wrap, at.before);
  if (at.hide) at.hide.hidden = true;
  editing = true;
  rename.start(slot, value, (name) => {
    editing = false;
    wrap.remove();
    if (at.hide) at.hide.hidden = false;
    // Redraws skipped while typing are made up here, whatever the edit did.
    void done(name).finally(redraw);
  }, "tree");
  const input = wrap.querySelector("input");
  if (!input) return;
  input.setAttribute("aria-label", t("tree.name"));
  // Like a file manager, renaming a file selects its name without the extension.
  const dot = value.lastIndexOf(".");
  if (file && dot > 0) input.setSelectionRange(0, dot);
}

async function create(parent: string, dir: boolean) {
  const id = workspace();
  if (!id || editing) return;
  if (parent) openDirs.add(parent);
  await draw(id);
  if (workspace() !== id || editing) return;
  const row = parent ? rowOf(parent) : null;
  const into = row ? row.nextElementSibling : $("tree");
  if (!into) return;
  const depth = row ? Number(row.dataset.depth) + 1 : 0;
  editName({ into, before: into.firstElementChild, depth }, "", false, async (name) => {
    if (!name) return;
    const rel = join(parent, name);
    try {
      await invoke("create_path", { id, rel, dir });
    } catch (error) {
      return fail(error);
    }
    if (workspace() !== id) return;
    if (dir) openDirs.add(rel);
    else openFile(rel);
  });
}

function startRename(entry: Entry) {
  const id = workspace();
  const row = rowOf(entry.path);
  if (!id || !row?.parentElement || editing) return;
  const depth = Number(row.dataset.depth);
  editName({ into: row.parentElement, before: row, depth, hide: row }, entry.name, !entry.dir, async (name) => {
    if (!name) return;
    const to = join(parentOf(entry.path), name);
    try {
      await invoke("rename_path", { id, from: entry.path, to });
    } catch (error) {
      return fail(error);
    }
    moved(id, entry.path, to);
    if (workspace() !== id) return;
    // Expanded folders stay expanded under their new name.
    for (const path of [...openDirs]) {
      const next = relocate(path, entry.path, to);
      if (next !== path) {
        openDirs.delete(path);
        if (next) openDirs.add(next);
      }
    }
  });
}

async function trash(entry: Entry) {
  const id = workspace();
  if (!id) return;
  // Keep showing which entry the confirmation is about.
  aim(entry.path);
  const sure = await confirmDialog({
    title: t("tree.trash.title", { name: entry.name }),
    message: t(entry.dir ? "tree.trash.folder" : "tree.trash.file"),
    accept: t("tree.trash.accept"),
    cancel: t("tree.trash.cancel"),
  });
  aim(null);
  if (!sure) return;
  try {
    await invoke("trash_path", { id, rel: entry.path });
  } catch (error) {
    return fail(error);
  }
  moved(id, entry.path, null);
  if (workspace() !== id) return;
  for (const path of [...openDirs]) if (!relocate(path, entry.path, null)) openDirs.delete(path);
  await draw(id);
}

/// Bring a deleted file or folder back from the last commit.
async function restore(path: string) {
  const id = workspace();
  if (!id) return;
  try {
    await invoke("tree_restore", { id, rel: path });
  } catch (error) {
    return fail(error);
  }
  if (workspace() === id) await draw(id);
}
