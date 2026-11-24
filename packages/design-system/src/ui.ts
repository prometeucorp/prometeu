import { icon } from "./icons.js";
import * as menu from "./menu.js";
import { h } from "./dom.js";
import { identifyControl, identifyArea, inheritTrackingArea } from "./tracking.js";
export { identifyControl, identifyArea } from "./tracking.js";

// Presentation primitives receive already-translated labels from their caller.
export type ButtonVariant = "outline" | "pri" | "ghost" | "danger";

export function button(label: string, run: () => void = () => {}, variant: ButtonVariant = "outline", track?: string) {
  const control = document.createElement("button");
  control.className = `ui-button ${variant} md`;
  control.type = "button";
  control.textContent = label;
  if (track) identifyControl(control, track);
  control.onclick = () => { control.focus(); run(); };
  return control;
}

/** Decorative photo or logo with a person/organization glyph fallback. The adjacent text supplies its
 * accessible name. */
export function avatar(image?: string, kind: "person" | "organization" = "person", size: "sm" | "md" | "lg" = "sm") {
  const root = document.createElement("span");
  root.className = `ui-avatar ${size}${kind === "organization" ? " org" : ""}`;
  root.setAttribute("aria-hidden", "true");
  if (image) {
    const img = document.createElement("img"); img.src = image; img.alt = "";
    root.append(img);
  } else root.innerHTML = icon(kind === "organization" ? "building" : "user", 14);
  return root;
}

export function input(value?: string, multiline?: false, track?: string): HTMLInputElement;
export function input(value: string | undefined, multiline: true, track?: string): HTMLTextAreaElement;
export function input(value?: string, multiline?: boolean, track?: string): HTMLInputElement | HTMLTextAreaElement;
export function input(value = "", multiline = false, track?: string) {
  const control = document.createElement(multiline ? "textarea" : "input");
  control.className = "ui-input";
  control.value = value;
  if (track) identifyControl(control, track);
  if (control instanceof HTMLTextAreaElement) control.rows = 5;
  return control;
}

export function field(label: string, control: HTMLElement, hint = "", track?: string) {
  const root = h("div", "ui-field");
  const target = control.matches("input, textarea, select, button") ? control : control.querySelector<HTMLElement>("input, textarea, select, button")!;
  if (!target.id) target.id = `ui-field-${crypto.randomUUID()}`;
  target.setAttribute("aria-label", label);
  if (track) identifyControl(target, track);
  const title = h("label", "ui-label", label);
  title.setAttribute("for", target.id);
  root.append(title, control);
  if (hint) {
    const help = h("span", "ui-hint", hint);
    help.id = `ui-hint-${crypto.randomUUID()}`;
    target.setAttribute("aria-describedby", help.id);
    root.append(help);
  }
  return root;
}

export function checkbox(label: string, checked: boolean, track?: string) {
  const control = document.createElement("input");
  control.type = "checkbox";
  control.checked = checked;
  if (track) identifyControl(control, track);
  const root = h("label", "ui-check");
  root.append(control, h("span", "", label));
  return { control, label: root };
}

/** A native checkbox with switch semantics keeps keyboard and form behavior. */
export function toggle(label: string, checked: boolean, track?: string) {
  const result = checkbox(label, checked, track);
  result.label.classList.add("ui-switch");
  result.control.setAttribute("role", "switch");
  return result;
}

export function radio(label: string, name: string, value: string, checked: boolean, track?: string) {
  const result = checkbox(label, checked, track);
  result.control.type = "radio";
  result.control.name = name;
  result.control.value = value;
  result.label.classList.add("ui-radio");
  return result;
}

export type Group = { head?: string; items: [string, string][] };

// Reuse the launcher menu in browsers and WKWebView. Read options on opening because the catalog may
// change between clicks.
export function dropdown(btn: HTMLButtonElement, groups: () => Group[], get: () => string, set: (id: string) => void) {
  const label = btn.querySelector("span")!;
  const draw = () => {
    label.textContent = groups().flatMap(g => g.items).find(([id]) => id === get())?.[1] ?? get();
  };
  btn.setAttribute("aria-haspopup", "menu");
  btn.setAttribute("aria-expanded", "false");
  btn.addEventListener("click", () => {
    const at = btn.getBoundingClientRect();
    const blocks = groups();
    const items: menu.Item[] = [];
    blocks.forEach((block, n) => {
      if (n) items.push("sep");
      if (block.head && blocks.length > 1) items.push({ label: block.head, disabled: true });
      for (const [id, name] of block.items) items.push({
        label: name, track: btn.dataset.track ? `${btn.dataset.track}.option` : undefined, checked: id === get(), run: () => { set(id); draw(); btn.focus(); },
      });
    });
    menu.openAt({ x: at.left, y: at.bottom + 4, anchor: btn }, items, "ui-select-menu", () => btn.setAttribute("aria-expanded", "false"), true);
    btn.setAttribute("aria-expanded", "true");
  });
  btn.addEventListener("keydown", event => {
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !menu.isOpen()) {
      event.preventDefault(); btn.click();
    } else if (event.key === "Tab" && menu.isOpen()) menu.close();
  });
  draw();
  return draw;
}

export function select(value: string, options: [string, string][], attributes: { name?: string; required?: boolean; disabled?: boolean; track?: string; area?: string } = {}) {
  const control = button("", () => {});
  control.className = "ui-select pick";
  if (attributes.track) identifyControl(control, attributes.track, attributes.area);
  else if (attributes.area) identifyArea(control, attributes.area);
  control.append(h("span", ""));
  control.insertAdjacentHTML("beforeend", icon("chevron-down", 14));
  const native = document.createElement("select");
  native.className = "ui-native-select"; native.tabIndex = -1;
  native.setAttribute("aria-hidden", "true");
  native.name = attributes.name ?? ""; native.required = attributes.required ?? false;
  native.disabled = control.disabled = attributes.disabled ?? false;
  control.setAttribute("aria-required", String(native.required));
  const fillOptions = () => {
    native.replaceChildren(...options.map(([id, label]) => new Option(label, id)));
    native.value = value;
  };
  fillOptions();
  const root = h("div", "ui-select-field"); root.append(control, native);
  const choice = {
    control, root, native,
    get value() { return native.value; },
    set value(next: string) { value = next; native.value = next; draw(); },
    onchange: () => {},
    setOptions(next: [string, string][], selected = value) { options = next; value = selected; fillOptions(); draw(); },
  };
  const draw = dropdown(control, () => [{ items: options }], () => native.value, id => {
    value = id; native.value = id; control.removeAttribute("aria-invalid");
    native.dispatchEvent(new Event("change", { bubbles: true }));
  });
  native.addEventListener("change", () => { value = native.value; draw(); choice.onchange(); });
  native.addEventListener("invalid", event => { event.preventDefault(); control.setAttribute("aria-invalid", "true"); control.focus(); });
  return choice;
}

export function disclosure(title: string, ...content: HTMLElement[]) {
  const root = document.createElement("details");
  root.className = "ui-disclosure";
  root.append(h("summary", "", title), ...content);
  return root;
}

export function formDialog(options: {
  title: string; save: string; cancel: string; variant?: ButtonVariant;
  submit: () => Promise<void>; error: (error: unknown) => string;
  closed?: () => void;
  track?: string; area?: string;
}) {
  let previousFocus: HTMLElement | null = null;
  const dialog = document.createElement("dialog");
  dialog.className = "sheet ui-dialog";
  dialog.setAttribute("aria-label", options.title);
  const form = document.createElement("form");
  const header = h("div", "sheettop");
  const title = h("b", "", options.title);
  title.tabIndex = -1;
  title.setAttribute("autofocus", "");
  header.append(title);
  const body = h("div", "ui-form-body");
  const footer = h("div", "sheetbar ui-form-footer");
  const error = h("span", "ui-hint ui-error");
  error.setAttribute("role", "alert");
  const close = () => {
    if (!dialog.isConnected) return;
    menu.close(); dialog.close(); dialog.remove();
    options.closed?.();
    let target = previousFocus;
    if (target && !target.isConnected) {
      target = target.id ? document.getElementById(target.id)
        : target.dataset.focus ? document.querySelector<HTMLElement>(`[data-focus="${CSS.escape(target.dataset.focus)}"]`) : null;
    }
    target?.focus();
  };
  const save = button(options.save, () => {}, options.variant ?? "pri", options.track ? `${options.track}.save` : undefined);
  save.type = "submit";
  const cancel = button(options.cancel, close, "outline", options.track ? `${options.track}.cancel` : undefined);
  // WebKit can skip buttons in native tab order unless tabindex is explicit.
  save.tabIndex = cancel.tabIndex = 0;
  footer.append(cancel, error, save);
  form.append(header, body, footer);
  dialog.append(form);
  let busy = false;
  form.onsubmit = async event => {
    event.preventDefault();
    if (busy) return;
    busy = true; save.disabled = cancel.disabled = true;
    form.setAttribute("aria-busy", "true"); error.textContent = "";
    try { await options.submit(); close(); }
    catch (cause) { error.textContent = options.error(cause); }
    finally { busy = false; save.disabled = cancel.disabled = false; form.removeAttribute("aria-busy"); }
  };
  dialog.addEventListener("cancel", event => { event.preventDefault(); if (!busy) close(); });
  dialog.addEventListener("keydown", event => {
    if (event.key !== "Tab") return;
    menu.close();
    const controls = Array.from(form.querySelectorAll<HTMLElement>("button, input, textarea, select, summary, [tabindex]"))
      .filter(node => !node.matches(":disabled") && node.tabIndex >= 0 && node.getClientRects().length);
    const first = controls[0], last = controls[controls.length - 1];
    if (event.shiftKey && (document.activeElement === first || document.activeElement === title)) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && (document.activeElement === last || document.activeElement === title)) { event.preventDefault(); first?.focus(); }
  });
  return {
    root: dialog, body, save, cancel, close,
    open() {
      if (dialog.isConnected) return;
      previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      inheritTrackingArea(dialog, previousFocus, options.area);
      document.body.append(dialog); dialog.showModal();
      const first = body.querySelector<HTMLElement>('input:not([type="hidden"]):enabled, textarea:enabled, button:enabled, select:enabled');
      // Start typing in forms; action and selection dialogs start at their title.
      if (first?.matches('textarea, input:not([type="checkbox"]):not([type="radio"]):not([type="button"]):not([type="submit"])')) first.focus();
      else title.focus();
    },
  };
}

export function confirmDialog(options: { title: string; message: string; accept: string; cancel: string; signal?: AbortSignal; track?: string; area?: string }) {
  if (options.signal?.aborted) return Promise.resolve(false);
  return new Promise<boolean>(resolve => {
    let confirmed = false;
    const dialog = formDialog({
      title: options.title, save: options.accept, cancel: options.cancel, variant: "danger", track: options.track, area: options.area,
      submit: async () => { confirmed = true; }, error: String,
      closed: () => { options.signal?.removeEventListener("abort", cancel); resolve(confirmed); },
    });
    const cancel = () => dialog.close();
    options.signal?.addEventListener("abort", cancel, { once: true });
    dialog.body.append(h("p", "ui-hint", options.message));
    dialog.open();
  });
}

export function menuButton(label: string, items: () => menu.Item[], track?: string) {
  const control = button(label, () => {
    if (control.getAttribute("aria-expanded") === "true") { menu.close(); return; }
    const at = control.getBoundingClientRect();
    menu.openAt({ x: at.left, y: at.bottom + 4, anchor: control }, items(), undefined,
      () => control.setAttribute("aria-expanded", "false"), true);
    control.setAttribute("aria-expanded", "true");
  }, "ghost", track);
  control.setAttribute("aria-haspopup", "menu");
  control.setAttribute("aria-expanded", "false");
  control.addEventListener("keydown", event => {
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && !menu.isOpen()) {
      event.preventDefault(); control.click();
    }
  });
  return control;
}

/** Enhance an existing input without replacing its value, name, validation or focus. */
export function passwordToggle(control: HTMLInputElement, labels: { show: string; hide: string }, track?: string) {
  const toggle = button(labels.show, () => {
    const reveal = control.type === "password";
    control.type = reveal ? "text" : "password";
    toggle.textContent = reveal ? labels.hide : labels.show;
    toggle.setAttribute("aria-pressed", String(reveal));
  }, "ghost");
  if (track) identifyControl(toggle, `${track}.visibility`);
  toggle.setAttribute("aria-pressed", "false");
  if (!control.id) control.id = `ui-password-${crypto.randomUUID()}`;
  toggle.setAttribute("aria-controls", control.id);
  return toggle;
}

export function password(value: string, labels: { show: string; hide: string }, track?: string) {
  const control = input(value, false, track) as HTMLInputElement;
  control.type = "password";
  const root = h("div", "ui-password");
  root.append(control, passwordToggle(control, labels, track));
  return { control, root };
}

export function badge(label: string, success = false) {
  return h("span", `ui-badge${success ? " success" : ""}`, label);
}

export function notice(message: string, tone: "success" | "error" | "warning" = "success") {
  const root = h("p", `ui-notice ${tone}`, message);
  root.setAttribute("role", tone === "error" ? "alert" : "status");
  return root;
}

export function card(title: string, ...content: HTMLElement[]) {
  const root = h("section", "ui-card ui-stack");
  root.append(h("h2", "", title), ...content);
  return root;
}

export { searchablePicker, type SearchPickerItem, type SearchPickerOptions } from "./search-picker.js";
