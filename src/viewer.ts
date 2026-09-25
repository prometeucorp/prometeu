import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { highlight } from "./highlight";
import { md } from "./markdown";
import { decode, parse } from "./csv";
import { fileIcon, icon } from "./icons";
import { findCapped, follow, markup, nearest, step, type Match } from "./find";
import { button, input } from "./ui";
import { relocateKeys } from "./tree-moves";
import { $ } from "./util";

/// The file editor layers highlighted code over a transparent textarea.
/// Native text input keeps editing and undo behavior without another editor dependency.

/// The original disk text is the optimistic save guard.
/// If the agent changes the file during editing, the backend rejects the write.
let shown: { id: string; path: string; text: string } | null = null;
let request = 0;

/// Drafts survive file navigation and shield user edits from board refreshes.
/// Returning to the original text resumes disk updates unless a save is pending.
type Draft = { was: string; text: string };
const drafts = new Map<string, Draft>();
const saving = new Set<string>();
const key = (id: string, path: string) => `${id}\n${path}`;

let fail: (m: string) => void = () => {};
/// Notify Changes when a local edit modifies disk without an agent event.
let saved: (id: string) => void = () => {};
let frame = 0;
let reading = false;

/// Find bar state. Matches are recomputed from the live buffer after every paint so edits never
/// leave marks on stale offsets.
let finding = false;
let hits: Match[] = [];
let active = -1;
/// The buffer had more matches than `hits` holds.
let more = false;
/// The buffer `hits` was computed from, so an edit can carry the active match to its new offset.
let seen = "";
/// What the marker layer currently shows; a repaint that changes none of it skips the rebuild.
let drawn = { text: "", query: "", active: -1 };
let query: HTMLInputElement;

const box = () => $("vtext") as HTMLTextAreaElement;
const here = () => (shown ? key(shown.id, shown.path) : "");
const draft = () => drafts.get(here());
const imageTypes: Record<string, string> = {
  png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif",
  webp: "image/webp", avif: "image/avif", bmp: "image/bmp", svg: "image/svg+xml", ico: "image/x-icon",
};

export function init(onError: (m: string) => void, onSaved: (id: string) => void) {
  fail = onError;
  saved = onSaved;
  const source = button(t("viewer.edit"), () => view(false), "ghost");
  source.id = "vsource";
  const preview = button(t("viewer.preview"), () => view(true), "ghost");
  preview.id = "vpreview";
  $("vview").setAttribute("aria-label", t("viewer.mode"));
  $("vview").append(source, preview);
  $("vcopy").innerHTML = icon("copy");
  $("vcopy").addEventListener("click", () => {
    if (!shown) return;
    navigator.clipboard.writeText(shown.path).catch((e) => fail(fromBack(e)));
  });
  $("vsave").innerHTML = icon("check");
  $("vsave").addEventListener("click", () => void save());
  $("vcancel").innerHTML = icon("x");
  $("vcancel").addEventListener("click", () => void revert());

  initFind();
  const text = box();
  text.addEventListener("input", typed);
  // Only .vcode scrolls; native textarea scrolling would misalign the two layers.
  text.addEventListener("scroll", () => {
    text.scrollTop = 0;
    text.scrollLeft = 0;
  });
  text.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      // With the find bar open, Escape dismisses it before it can discard a draft.
      if (finding) closeFind();
      else void revert();
      return;
    }
    if (e.key === "s" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void save();
      return;
    }
    // insertText indents without replacing the browser undo history.
    if (e.key === "Tab") {
      e.preventDefault();
      document.execCommand("insertText", false, "  ");
      typed();
    }
  });
}

function initFind() {
  query = input();
  query.id = "vfindq";
  query.type = "search";
  query.spellcheck = false;
  query.placeholder = t("viewer.find.placeholder");
  query.setAttribute("aria-label", t("viewer.find.placeholder"));
  $("vfind").prepend(query);
  $("vfindprev").innerHTML = icon("chevron-up");
  $("vfindnext").innerHTML = icon("chevron-down");
  $("vfindclose").innerHTML = icon("x");
  $("vfindprev").addEventListener("click", () => go(-1));
  $("vfindnext").addEventListener("click", () => go(1));
  $("vfindclose").addEventListener("click", () => closeFind());
  query.addEventListener("input", () => search(box().selectionStart));
  query.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      go(e.shiftKey ? -1 : 1);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeFind();
    }
  });
  // The viewer owns Command-F only while an editable code buffer is on screen.
  document.addEventListener("keydown", (e) => {
    if (e.key.toLowerCase() !== "f" || !(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey) return;
    if (openFind()) e.preventDefault();
  });
}

/// Open the find bar, seeded with a single-line selection. Return whether the viewer took the shortcut.
function openFind(): boolean {
  if (!shown || box().hidden || $("viewer").hidden || !$("viewer").getClientRects().length) return false;
  if (document.querySelector("dialog[open]")) return false;
  if (reading) view(false);
  const ta = box();
  // Only an editor selection seeds the query; repeating the shortcut in the bar keeps the typed text.
  const picked = document.activeElement === ta ? ta.value.slice(ta.selectionStart, ta.selectionEnd) : "";
  if (picked && !picked.includes("\n")) query.value = picked;
  finding = true;
  $("vfind").hidden = false;
  query.focus();
  query.select();
  search(ta.selectionStart);
  return true;
}

/// Close the bar, drop every mark and return to the editor with the active match still selected.
function closeFind(refocus = true) {
  if (!finding) return;
  finding = false;
  hits = [];
  more = false;
  active = -1;
  $("vfind").hidden = true;
  $("vmarks").textContent = "";
  drawn = { text: "", query: "", active: -1 };
  if (refocus && !box().hidden) box().focus({ preventScroll: true });
}

/// Recompute matches for a new query, starting from the caret, and reveal the first one.
function search(from: number) {
  collect();
  active = nearest(hits, from);
  marks();
  reveal();
}

function go(delta: number) {
  if (!finding || !hits.length) return;
  active = step(active, hits.length, delta);
  marks();
  reveal();
}

/// After an edit or a disk refresh, keep the active match where it was, carried across the edit,
/// without moving the caret.
function refind() {
  if (!finding) return;
  const was = active >= 0 ? hits[active]?.start ?? 0 : 0;
  const before = seen;
  collect();
  active = nearest(hits, follow(before, seen, was));
  marks();
}

function collect() {
  seen = box().value;
  ({ matches: hits, more } = findCapped(seen, query.value));
}

function marks() {
  const now = { text: box().value, query: query.value, active };
  if (now.text !== drawn.text || now.query !== drawn.query || now.active !== drawn.active) {
    $("vmarks").innerHTML = markup(now.text, hits, active);
    drawn = now;
  }
  const n = $("vfindn");
  n.classList.toggle("none", !!query.value && !hits.length);
  n.textContent = !query.value
    ? ""
    : !hits.length
      ? t("viewer.find.none")
      : t(more ? "viewer.find.countMany" : "viewer.find.count", {
          at: active + 1,
          total: hits.length,
        });
  ($("vfindprev") as HTMLButtonElement).disabled = !hits.length;
  ($("vfindnext") as HTMLButtonElement).disabled = !hits.length;
}

/// Select the active match in the editor and scroll .vcode, the only scroller, to show it.
function reveal() {
  const m = hits[active];
  if (!m) return;
  const typing = document.activeElement === query;
  box().setSelectionRange(m.start, m.end);
  // Some engines focus a textarea whose selection changes; keep typing in the find field.
  if (typing && document.activeElement !== query) query.focus({ preventScroll: true });
  const mark = $("vmarks").querySelector("mark.on");
  if (!mark) return;
  const code = $("vcode");
  const r = mark.getBoundingClientRect();
  const c = code.getBoundingClientRect();
  if (r.top < c.top || r.bottom > c.bottom) code.scrollTop += r.top - c.top - (c.height - r.height) / 2;
  // The sticky gutter covers the left edge of the scroller.
  const left = c.left + $("vgutter").offsetWidth;
  if (r.left < left || r.right > c.right) code.scrollLeft += r.left - left - Math.max(0, (c.right - left - r.width) / 3);
}

/// Unsaved drafts follow an entry the file tree renamed (`to`), or go with it to the trash (`to` is
/// null), so reopening the new path keeps the edits and a trashed file leaves nothing behind.
export function moveDrafts(id: string, from: string, to: string | null) {
  relocateKeys(drafts, key(id, ""), from, to);
}

/// Board events refresh open files; preserve scrolling when the content is unchanged.
export async function show(id: string, path: string) {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  const kind = /\.pdf$/i.test(path) ? "pdf" : /\.csv$/i.test(path) ? "csv"
    : Object.prototype.hasOwnProperty.call(imageTypes, extension) ? "image" : null;
  if (kind) return showBlob(id, path, kind);
  const k = key(id, path);
  const same = shown?.id === id && shown.path === path;
  const currentRequest = ++request;
  // An unsaved draft owns the displayed text.
  if (same && drafts.has(k)) return;

  let text = "";
  let error = "";
  try {
    text = await invoke("read_file", { id, rel: path });
  } catch (e) {
    error = fromBack(e);
  }
  if (currentRequest !== request) return;
  // The user may start typing while the disk read is pending.
  if (shown?.id === id && shown.path === path && drafts.has(k)) return;
  if (same && shown!.text === text && !error) return;
  shown = { id, path, text };
  crumb(path);
  blob(false);
  if (!same) reading = false;
  $("vview").hidden = !!error || !/\.(md|markdown)$/i.test(path);

  // Unreadable, binary, or oversized files display an error instead of an editable buffer.
  const ta = box();
  ta.hidden = !!error;
  if (error) {
    drafts.delete(k);
    closeFind(false);
    $("vgutter").textContent = "";
    $("vpre").innerHTML = `<span class="h-c"></span>`;
    $("vpre").children[0].textContent = error;
    chrome();
    return;
  }

  // Restore the draft and the disk baseline from when editing began.
  const pending = drafts.get(k);
  if (pending) shown = { id, path, text: pending.was };
  const value = pending ? pending.text : text;

  // Preserve the selection when refreshing the same file.
  const at = same ? [ta.selectionStart, ta.selectionEnd] : [0, 0];
  ta.value = value;
  if (same) ta.setSelectionRange(Math.min(at[0], value.length), Math.min(at[1], value.length));
  // A different file starts its matches from the top.
  if (!same) active = -1;
  paint();
  chrome();
  view(reading);
  if (!same) {
    $("vcode").scrollTo(0, 0);
    $("vread").scrollTo(0, 0);
  }
}

function view(next: boolean) {
  reading = next;
  // Marks belong to the source layout; the rendered preview has no find bar.
  if (next) closeFind(false);
  $("vcode").hidden = next;
  $("vread").hidden = !next;
  $("vsource").setAttribute("aria-pressed", String(!next));
  $("vpreview").setAttribute("aria-pressed", String(next));
  if (next) $("vread").innerHTML = md(box().value);
}

function crumb(path: string) {
  const cut = path.lastIndexOf("/");
  const el = $("vcrumb");
  el.innerHTML = `${fileIcon(path.slice(cut + 1), 14)}<span class="dir"></span><span class="nm"></span>`;
  el.children[1].textContent = cut === -1 ? "" : path.slice(0, cut + 1);
  el.children[2].textContent = path.slice(cut + 1);
}

/// Non-code files use #vfile. Release the previous object URL, which retains the file bytes.
let fileUrl = "";
function blob(on: boolean) {
  // The tab already names the file, and these formats have no save controls.
  $("vbar").hidden = on;
  $("vcode").hidden = on;
  $("vread").hidden = true;
  $("vfile").hidden = !on;
  if (fileUrl) URL.revokeObjectURL(fileUrl);
  fileUrl = "";
  $("vfile").replaceChildren();
}

/// WebKit renders PDF in an iframe; CSV uses a table; images use the browser decoder.
/// Read bytes only when the disk stamp changes to avoid large reads on every board event.
async function showBlob(id: string, path: string, kind: "pdf" | "csv" | "image") {
  const same = shown?.id === id && shown.path === path;
  const currentRequest = ++request;
  let stamp = "";
  let bytes: ArrayBuffer | null = null;
  let error = "";
  try {
    stamp = await invoke("file_stamp", { id, rel: path });
    if (currentRequest !== request) return;
    if (same && shown!.text === stamp) return;
    bytes = await invoke("read_bytes", { id, rel: path });
  } catch (e) {
    error = fromBack(e);
  }
  if (currentRequest !== request) return;
  shown = { id, path, text: stamp };
  crumb(path);
  reading = false;
  $("vview").hidden = true;
  chrome();
  $("vgutter").textContent = "";
  $("vpre").innerHTML = "";
  box().hidden = true;
  closeFind(false);
  blob(!error);
  if (error) {
    $("vpre").innerHTML = `<span class="h-c"></span>`;
    $("vpre").children[0].textContent = error;
    return;
  }
  const into = $("vfile");
  into.className = `vfile ${kind}`;
  if (kind === "pdf") {
    fileUrl = URL.createObjectURL(new Blob([bytes!], { type: "application/pdf" }));
    const frame = document.createElement("iframe");
    frame.src = fileUrl;
    into.append(frame);
    return;
  }
  if (kind === "image") {
    const type = imageTypes[path.split(".").pop()?.toLowerCase() ?? ""];
    fileUrl = URL.createObjectURL(new Blob([bytes!], { type }));
    const img = document.createElement("img");
    img.alt = path.split("/").pop() || path;
    img.onerror = () => {
      if (currentRequest !== request) return;
      blob(false);
      $("vpre").textContent = t("viewer.imageError");
    };
    img.src = fileUrl;
    into.append(img);
    return;
  }
  table(into, parse(decode(bytes!)));
}

/// Append CSV rows in batches to avoid rendering a large file into the DOM at once.
function table(into: HTMLElement, rows: string[][]) {
  const [head, ...body] = rows;
  const t = document.createElement("table");
  const thead = t.createTHead().insertRow();
  for (const h of head ?? []) thead.append(Object.assign(document.createElement("th"), { textContent: h }));
  const tbody = t.createTBody();
  const end = document.createElement("div");
  into.append(t, end);
  let at = 0;
  const more = () => {
    // Keep the viewport at the bottom after each batch while the user remains there.
    // Require positive scrollTop so an empty container does not load every batch immediately.
    const bottom = into.scrollTop > 0 && into.scrollTop + into.clientHeight >= into.scrollHeight - 1;
    const stop = Math.min(body.length, at + 500);
    for (; at < stop; at++) {
      const tr = tbody.insertRow();
      for (let c = 0; c < head.length; c++) tr.insertCell().textContent = body[at][c] ?? "";
    }
    if (at >= body.length) return watch.disconnect();
    if (bottom) {
      into.scrollTop = into.scrollHeight;
      requestAnimationFrame(more);
    }
  };
  const watch = new IntersectionObserver((hits) => hits[0].isIntersecting && more(), { root: into });
  more();
  watch.observe(end);
}

function typed() {
  if (!shown) return;
  const value = box().value;
  const k = here();
  const was = drafts.get(k)?.was ?? shown.text;
  // Reverting to the baseline resumes disk updates, except while a save can still change that baseline.
  if (value === was && !saving.has(k)) drafts.delete(k);
  else drafts.set(k, { was, text: value });
  chrome();
  soon();
}

/// Highlighting scans the whole file; coalesce typing into one pass per animation frame.
function soon() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(paint);
}

function paint() {
  cancelAnimationFrame(frame);
  const text = box().value;
  // Include the trailing empty line because it remains editable.
  const rows: string[] = [];
  for (let i = 1; i <= text.split("\n").length; i++) rows.push(String(i));
  $("vgutter").textContent = rows.join("\n");
  $("vpre").innerHTML = highlight(text, shown?.path ?? "");
  refind();
}

/// Show save and discard controls only for unsaved drafts.
function chrome() {
  const dirty = !!draft();
  $("vsave").hidden = !dirty;
  $("vcancel").hidden = !dirty;
  ($("vsave") as HTMLButtonElement).disabled = saving.has(here());
  ($("vcancel") as HTMLButtonElement).disabled = saving.has(here());
  $("vcrumb").classList.toggle("dirty", dirty);
}

/// Discard the draft and reload the current disk contents.
async function revert() {
  if (!shown || !draft() || saving.has(here())) return;
  const { id, path, text } = shown;
  drafts.delete(here());
  box().value = text;
  paint();
  if (reading) view(true);
  chrome();
  await show(id, path);
}

async function save() {
  if (!shown || !draft() || saving.has(here())) return;
  const { id, path, text: was } = shown;
  const k = here();
  const text = box().value;
  saving.add(k);
  chrome();
  try {
    await invoke("write_file", { id, rel: path, text, was });
    // Only the submitted version becomes clean; newer edits use the saved text as their baseline.
    const pending = drafts.get(k);
    if (pending && pending.text !== text) drafts.set(k, { was: text, text: pending.text });
    else drafts.delete(k);
    if (here() === k) shown = { id, path, text };
    saved(id);
  } catch (e) {
    fail(fromBack(e));
  } finally {
    saving.delete(k);
    const pending = drafts.get(k);
    if (pending?.text === pending?.was) drafts.delete(k);
    chrome();
  }
}
