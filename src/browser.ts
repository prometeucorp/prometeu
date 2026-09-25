/// Native preview lifecycle and the design tools surrounding its reserved DOM surface.
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { fromBack, t } from "./i18n";
import * as session from "./session";
import { button, input, select } from "./ui";
import { $, h } from "./util";
import type { BrowserSelection } from "./browser-types";

let shown: string | null = null;
let visible: string | null = null;
let placed = "";
let version = 0;
let poll = 0;
let frame = 0;
let sizeObserver: ResizeObserver | null = null;
let positionObserver: MutationObserver | null = null;
let resizing = false;
let inspecting = false;
let selectionVersion = 0;
let selected: { data: BrowserSelection; image?: string } | null = null;
let say: (message: string, error?: boolean) => void;
let inspectButton: HTMLButtonElement;
let captureButton: HTMLButtonElement;
let viewport: HTMLInputElement;
const bar = () => $("wurl") as HTMLInputElement;
const typing = () => document.activeElement === bar();

// Serialize native visibility changes; an old open must finish hiding before a newer open runs.
let pending: Promise<unknown> = Promise.resolve();
function order<T>(run: () => Promise<T>): Promise<T> {
  const result = pending.then(run);
  pending = result.catch(() => {});
  return result;
}
const report = (error: unknown) => say(fromBack(error), true);

function bounds() {
  const rect = $("webbody").getBoundingClientRect();
  if (!rect.width || !rect.height || resizing || !$("veil").hidden || document.querySelector(
    "dialog[open], .menu, [popover]:popover-open:not(.ui-feedback), .ui-feedback:not(.ui-feedback-capturing) .ui-feedback-panel:not([hidden])",
  )) return null;
  return { x: rect.left, y: rect.top, w: rect.width, h: rect.height };
}

async function conceal() {
  if (!visible) return;
  const id = visible;
  visible = null;
  placed = "";
  await invoke("browser_hide", { id });
}

async function position() {
  const id = shown;
  const rect = bounds();
  if (visible && (visible !== id || !rect)) await conceal();
  if (!id || !rect) return;
  if (visible !== id) {
    await invoke("browser_open", { id });
    visible = id;
  }
  const current = bounds();
  if (shown !== id || !current) return conceal();
  const signature = JSON.stringify([id, current]);
  if (signature === placed) return;
  await invoke("browser_bounds", { id, ...current });
  placed = signature;
}

function place() {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    void order(position).catch(report);
  });
}

function watchPosition() {
  if (positionObserver) return;
  sizeObserver = new ResizeObserver(() => {
    // Opening details resizes the page after capture; only pending selections need invalidation.
    if (!selected) selectionVersion++;
    place();
  });
  sizeObserver.observe($("webbody"));
  // Native views cannot participate in DOM stacking, including menus outside the preview.
  positionObserver = new MutationObserver(place);
  positionObserver.observe(document.body, {
    subtree: true, childList: true, attributes: true, attributeFilter: ["hidden", "open", "class"],
  });
  document.addEventListener("toggle", place, true);
  place();
}

function stopPosition() {
  sizeObserver?.disconnect();
  sizeObserver = null;
  positionObserver?.disconnect();
  positionObserver = null;
  document.removeEventListener("toggle", place, true);
  cancelAnimationFrame(frame);
  frame = 0;
}

export function init(external: (id: string) => void, notify: (message: string, error?: boolean) => void) {
  say = notify;
  for (const [control, run] of [
    ["wback", (id: string) => invoke("browser_back", { id })],
    ["wfwd", (id: string) => invoke("browser_forward", { id })],
    ["wreload", (id: string) => invoke("browser_reload", { id })],
  ] as const) $(control).addEventListener("click", () => {
    if (!shown) return;
    clearSelection();
    void run(shown).catch(report);
  });
  $("wext").addEventListener("click", () => shown && external(shown));
  bar().setAttribute("aria-label", t("web.url"));
  bar().addEventListener("keydown", event => {
    if (event.key === "Enter") go();
    else if (event.key === "Escape") { bar().blur(); void refresh().catch(report); }
  });
  bar().addEventListener("focus", () => bar().select());
  bar().addEventListener("blur", () => void refresh().catch(report));
  void listen<[string, string]>("browser:url", ({ payload: [id, url] }) => {
    if (id !== shown) return;
    if (!typing()) bar().value = url;
    clearSelection();
    inspecting = false;
    paintInspect();
  });

  inspectButton = button(t("web.inspect"), () => void toggleInspect().catch(report), "ghost");
  inspectButton.id = "winspect";
  inspectButton.setAttribute("aria-pressed", "false");
  captureButton = button(t("web.capture"), () => void capture(), "ghost");
  captureButton.id = "wcapture";
  const size = select("fit", [["fit", t("web.fit")], ["390", t("web.mobile")], ["768", t("web.tablet")]]);
  size.control.setAttribute("aria-label", t("web.viewport"));
  size.onchange = () => {
    viewport.value = size.value === "fit" ? "" : size.value;
    resizeViewport();
  };
  viewport = input();
  viewport.id = "wwidth";
  viewport.type = "number";
  viewport.min = "240";
  viewport.max = "2560";
  viewport.placeholder = t("web.auto");
  viewport.setAttribute("aria-label", t("web.width"));
  viewport.addEventListener("change", resizeViewport);
  $("webtools").append(inspectButton, captureButton, h("span", "spacer"), size.root, viewport);
  $("webdetails").setAttribute("aria-label", t("web.selection"));
  initSplitter();
  document.addEventListener("keydown", event => {
    if (!shown || document.querySelector("dialog[open]")) return;
    if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === "d") {
      event.preventDefault();
      void toggleInspect().catch(report);
    }
  });
}

export async function show(id: string): Promise<number> {
  const epoch = ++version;
  shown = id;
  clearTimeout(poll);
  inspecting = false;
  paintInspect();
  clearSelection();
  const port = await order(async () => {
    if (epoch !== version || shown !== id) return 0;
    await conceal();
    const port = await invoke("browser_open", { id });
    visible = id;
    if (epoch !== version || shown !== id) { await conceal(); return port; }
    watchPosition();
    await position();
    return port;
  });
  if (epoch !== version || shown !== id) return port;
  bar().value = `http://localhost:${port}`;
  void tick(epoch);
  return port;
}

export function hide() {
  const id = shown;
  shown = null;
  stopPosition();
  version++;
  inspecting = false;
  clearTimeout(poll);
  clearSelection();
  void order(async () => {
    await conceal();
    if (id) await invoke("browser_inspect", { id, enabled: false }).catch(() => {});
  }).catch(report);
}

export function close(id: string) {
  if (shown === id) hide();
  void order(() => invoke("browser_close", { id })).catch(report);
}

function go() {
  const id = shown;
  if (!id) return;
  const typed = bar().value.trim();
  if (!typed) return void refresh().catch(report);
  const url = /^[a-z][a-z0-9+.-]*:\/\//i.test(typed) ? typed : `http://${typed}`;
  bar().blur();
  clearSelection();
  void invoke("browser_navigate", { id, url }).catch(error => { report(error); void refresh().catch(report); });
}

async function refresh() {
  const id = shown;
  const epoch = version;
  if (!id || typing()) return;
  const url = await invoke("browser_url", { id });
  if (shown !== id || version !== epoch || !url || typing()) return;
  if (selected && selected.data.url !== url) clearSelection();
  bar().value = url;
}

async function tick(epoch: number) {
  const id = shown;
  if (!id || epoch !== version) return;
  try {
    await refresh();
    if (inspecting && visible === id && bounds()) {
      const inspection = selectionVersion;
      const result = await invoke("browser_selection", { id });
      if (shown !== id || epoch !== version || inspection !== selectionVersion) return;
      inspecting = result.active;
      paintInspect();
      if (result.selection) {
        // Capture before expanding the details panel changes the native viewport geometry.
        const data = result.selection;
        let image: string | undefined;
        try { image = await invoke("browser_capture", { id, rect: data.rect }); }
        catch (error) { if (epoch === version) report(error); }
        if (shown !== id || epoch !== version || inspection !== selectionVersion) return;
        const url = await invoke("browser_url", { id });
        if (shown !== id || epoch !== version || inspection !== selectionVersion || url !== data.url) return;
        selected = { data, image };
        drawSelection();
      }
    }
  } catch (error) {
    if (epoch === version) { inspecting = false; paintInspect(); report(error); }
  } finally {
    if (epoch === version && shown === id) poll = window.setTimeout(() => void tick(epoch), inspecting ? 200 : 1000);
  }
}

function paintInspect() {
  if (!inspectButton) return;
  inspectButton.setAttribute("aria-pressed", String(inspecting));
  inspectButton.textContent = t(inspecting ? "web.inspecting" : "web.inspect");
}

async function toggleInspect() {
  const id = shown;
  const epoch = version;
  if (!id || inspectButton.disabled) return;
  const enabled = !inspecting;
  clearSelection();
  inspectButton.disabled = true;
  try {
    await order(position);
    if (shown === id && version === epoch) await invoke("browser_inspect", { id, enabled });
    if (shown !== id || version !== epoch) return;
    inspecting = enabled;
    paintInspect();
  } finally { inspectButton.disabled = false; }
}

function clearSelection() {
  selectionVersion++;
  selected = null;
  const details = $("webdetails");
  details.hidden = true;
  details.replaceChildren();
}

function drawSelection() {
  if (!selected) return;
  const { data, image } = selected;
  const details = $("webdetails");
  const actions = h("div", "webselectionbar");
  const name = h("code", "webselector", data.selector);
  name.title = data.selector;
  const add = button(t("web.addToChat"), () => {
    const target = session.contextTarget();
    if (!target) return say(t("web.noChat"), true);
    target({ selection: data, image });
    say(t("web.added"));
    clearSelection();
  }, "pri");
  add.id = "wadd";
  const copy = button(t("web.copyHtml"), () => void navigator.clipboard.writeText(data.html).catch(report), "ghost");
  const dismiss = button(t("web.dismiss"), clearSelection, "ghost");
  actions.append(name, add, copy, dismiss);
  const meta = h("p", "webmeta", `${Math.round(data.rect.width)} × ${Math.round(data.rect.height)} px · ${image ? t("web.imageReady") : t("web.imageMissing")}`);
  const html = h("pre", "webhtml", data.html);
  html.setAttribute("aria-label", "HTML");
  const styles = h("pre", "webstyles", Object.entries(data.styles).map(([key, value]) => `${key}: ${value};`).join("\n"));
  styles.setAttribute("aria-label", "CSS");
  const content = h("div", "webcode");
  content.append(html, styles);
  details.replaceChildren(actions, meta, content);
  details.hidden = false;
}

async function capture() {
  const id = shown;
  const target = session.fileDropTarget();
  if (!id || !target) return say(t("web.noChat"), true);
  const done = target.wait();
  captureButton.disabled = true;
  try {
    target.put([await invoke("browser_capture", { id })]);
    say(t("web.added"));
  } catch (error) { report(error); }
  finally { done(); captureButton.disabled = false; }
}

function resizeViewport() {
  if (viewport.value && !viewport.checkValidity()) { viewport.reportValidity(); return; }
  $("webbody").style.width = viewport.value ? `${viewport.value}px` : "100%";
  clearSelection();
  place();
}

function initSplitter() {
  const divider = $("websplit");
  const host = $("tabbody");
  let width = 40;
  let dragging = false;
  divider.setAttribute("aria-label", t("web.resize"));
  divider.setAttribute("aria-valuemin", "25");
  divider.setAttribute("aria-valuemax", "70");
  const set = (value: number) => {
    clearSelection();
    width = Math.max(25, Math.min(70, value));
    host.style.setProperty("--web-chat-width", `${width}%`);
    divider.setAttribute("aria-valuenow", String(Math.round(width)));
    place();
  };
  set(width);
  divider.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "Home"].includes(event.key)) return;
    event.preventDefault();
    set(event.key === "Home" ? 40 : width + (event.key === "ArrowLeft" ? -2 : 2));
  });
  divider.addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    clearSelection();
    dragging = resizing = true;
    divider.setPointerCapture(event.pointerId);
    place();
    event.preventDefault();
  });
  divider.addEventListener("pointermove", event => {
    if (!dragging) return;
    const rect = host.getBoundingClientRect();
    set((event.clientX - rect.left) / rect.width * 100);
  });
  const end = () => { dragging = resizing = false; place(); };
  divider.addEventListener("pointerup", end);
  divider.addEventListener("pointercancel", end);
  divider.addEventListener("lostpointercapture", end);
  divider.addEventListener("dblclick", () => set(40));
}
