import * as ui from "./ui";
import type { ResourceItem } from "./resources/model";
import { listen } from "@tauri-apps/api/event";
import { invoke, type IpcResult } from "./ipc";
import { fromBack, t, type Key } from "./i18n";
import * as menu from "./menu";
import * as catalog from "./catalog";
import * as skills from "./skills";
import * as toolPicker from "./tool-picker";
import type { Plugin, Selection } from "./types";
import { $, h, template } from "./util";

/// Present the backend-owned plugin registry and shared launcher/conversation picker using the same interaction patterns as MCP. Refresh the full snapshot after writes. Installation clones repositories into app storage; creation uses a separate agent-driven flow.

let hub: Plugin[] = [];
let loaded = false;
const watchers = new Set<() => void>();

/// The initial empty registry redraws after loading.
export const list = () => hub;

export const onChange = (fn: () => void) => {
  watchers.add(fn);
  return () => watchers.delete(fn);
};

function announce() {
  for (const fn of watchers) fn();
}

/// Load once per app session; mutations already return the updated registry.
export async function load() {
  if (loaded) return;
  loaded = true;
  try {
    hub = await invoke("plugin_hub");
    announce();
  } catch {
    // Hide hub pickers if the backend is unavailable or older.
  }
}

/// Deleted registry entries remain in workspace choices and stay visible in the picker until deselected.
export const known = (id: string) => hub.some((p) => p.id === id);

/* Picker. */

type Pick = {
  /// The workspace whose layer is edited.
  workspace: string;
  /// The workspace layer for this axis; null inherits the global and project layers.
  current: () => Selection | null;
  /// Persist the new layer, or null to return the axis to inherit.
  set: (sel: Selection | null) => Promise<void> | void;
  /// Menu anchor position.
  at: () => { x: number; y: number };
  /// Opens the project-trust prompt when the project layer has pending items.
  trust?: () => void;
};

/// Registry entries that are standalone-skill packages, which ride the plugin pipeline as `skill-<id>`
/// but live on their own selection axis (ADR 0045).
const isSkillPackage = (id: string) => skills.packageIds().has(id);

/// Add rows for ids the registry no longer has so they can still be dropped from the layer.
function withGone(rows: toolPicker.Row[], current: Selection | null, gone: (id: string) => string) {
  for (const id of [...(current?.add ?? []), ...(current?.remove ?? [])]) {
    if (!known(id) && !rows.some((r) => r.id === id)) rows.push({ id, label: gone(id) });
  }
  return rows;
}

/// Provenance-aware plugin picker (ADR 0045): shows the resolved effective set and writes the
/// workspace layer as deltas over the global and project layers.
export function openPicker(p: Pick) {
  const rows = hub.filter((pl) => !isSkillPackage(pl.id)).map((pl) => ({ id: pl.id, label: pl.id, hint: pl.note.trim() }));
  void toolPicker.open({
    workspace: p.workspace,
    axis: "plugins",
    rows: withGone(rows, p.current(), (id) => t("plugin.gone", { name: id })),
    current: p.current,
    set: p.set,
    at: p.at,
    noneLabel: t("plugin.none"),
    trust: p.trust,
  });
}

/// Standalone-skill picker: its own axis, but the ids are the `skill-<id>` plugin packages.
export function openSkillPicker(p: Pick) {
  const rows = hub.filter((pl) => isSkillPackage(pl.id)).map((pl) => ({ id: pl.id, label: pl.id.replace(/^skill-/, ""), hint: pl.note.trim() }));
  void toolPicker.open({
    workspace: p.workspace,
    axis: "skills",
    rows: withGone(rows, p.current(), (id) => t("plugin.gone", { name: id.replace(/^skill-/, "") })),
    current: p.current,
    set: p.set,
    at: p.at,
    noneLabel: t("skill.none"),
    trust: p.trust,
  });
}

/// Wording for one axis label: the inherit text, the zero/count texts for an explicit empty
/// selection, and the optional prefix stripped from ids before display (skills ride `skill-<id>`).
export type Words = { def: Key; zero: Key; count: Key; prefix?: string };

const PLUGIN_WORDS: Words = { def: "plugin.default", zero: "plugin.zero", count: "plugin.count" };
export const SKILL_WORDS: Words = { def: "skill.default", zero: "skill.zero", count: "skill.count", prefix: "skill-" };

/// Summarize the workspace layer for the button: inherit, a single pick, or its +/- deltas. The
/// skills axis shares this function and passes its own wording.
export function label(sel: Selection | null, words: Words = PLUGIN_WORDS): string {
  const bare = (id: string) => (words.prefix ? id.replace(new RegExp(`^${words.prefix}`), "") : id);
  if (!sel) return t(words.def);
  if (sel.base === "none") {
    // Under an explicit none base the layer is a flat list; removals are no-ops there.
    if (!sel.add.length) return t(words.zero);
    if (sel.add.length === 1) return bare(sel.add[0]);
    return t(words.count, { n: String(sel.add.length) });
  }
  if (sel.add.length === 1 && !sel.remove.length) return bare(sel.add[0]);
  const bits = [sel.add.length ? `+${sel.add.length}` : "", sel.remove.length ? `−${sel.remove.length}` : ""].filter(Boolean);
  return bits.length ? bits.join(" ") : t(words.def);
}

/// Label for the launcher's flat default preset, a plain id list rather than a layered delta.
export function flatLabel(ids: string[] | null): string {
  if (ids === null) return t("plugin.default");
  if (!ids.length) return t("plugin.zero");
  if (ids.length === 1) return ids[0];
  return t("plugin.count", { n: String(ids.length) });
}

/// Picker for the launcher's default plugin preset, stored as `string[] | null`.
export function openDefaultPicker(p: {
  chosen: () => string[] | null;
  set: (ids: string[] | null) => void;
  at: () => { x: number; y: number };
}) {
  const rows = hub.filter((pl) => !isSkillPackage(pl.id)).map((pl) => ({ id: pl.id, label: pl.id, hint: pl.note.trim() }));
  const asSel = (): Selection | null => {
    const ids = p.chosen();
    return ids === null ? null : { base: "none", add: ids, remove: [] };
  };
  toolPicker.openFlat({
    rows,
    current: asSel,
    set: (sel) => p.set(sel === null ? null : sel.add),
    at: p.at,
    noneLabel: t("plugin.none"),
  });
}

type GlobalPick = {
  current: () => Selection | null;
  set: (sel: Selection | null) => void;
  at: () => { x: number; y: number };
};

/// Picker for the board's global plugin layer (ADR 0045), the base every project and workspace inherits.
export function openGlobalPicker(p: GlobalPick) {
  const rows = hub.filter((pl) => !isSkillPackage(pl.id)).map((pl) => ({ id: pl.id, label: pl.id, hint: pl.note.trim() }));
  toolPicker.openFlat({ rows, current: p.current, set: p.set, at: p.at, noneLabel: t("plugin.none") });
}

/// Picker for the board's global standalone-skill layer; ids are the `skill-<id>` plugin packages.
export function openSkillGlobalPicker(p: GlobalPick) {
  const rows = hub.filter((pl) => isSkillPackage(pl.id)).map((pl) => ({ id: pl.id, label: pl.id.replace(/^skill-/, ""), hint: pl.note.trim() }));
  toolPicker.openFlat({ rows, current: p.current, set: p.set, at: p.at, noneLabel: t("skill.none") });
}

/* Settings list. */

type Ctx = { say: (text: string, isError?: boolean) => void };
let ctx: Ctx;

export function init(context: Ctx) {
  ctx = context;
}

const updating = new Set<string>();
export function resourceItems(): ResourceItem[] {
  const local = hub.filter(p => !skills.packageIds().has(p.id));
  return [...local.map(pluginResource), ...catalog.pendingResources("plugins", local.map(plugin => plugin.id), ctx.say)];
}

export function settingsActions(): menu.Item[] {
  return [
    { label: t("plugin.install"), run: () => installer() },
    { label: t("plugin.make"), run: () => maker() },
    { label: t("plugin.add"), run: () => editor(null) },
  ];
}

function installCatalogPlugin(p: catalog.CatalogPlugin, replacing = false) {
  const dialog = ui.formDialog({ title: t(replacing ? "catalog.replaceSource" : "catalog.install"), save: t("catalog.install"), cancel: t("plugin.cancel"), error: fromBack,
    submit: async () => { await invoke("catalog_install_plugin", { id: p.id }); await catalog.refresh(); } });
  dialog.body.append(h("p", "ui-hint", p.source), h("p", "ui-hint", t("catalog.installHint"))); dialog.open();
}

function pluginResource(plugin: Plugin): ResourceItem {
  const actions = catalog.resourceActions("plugins", plugin.id, ctx.say);
  const cloud = catalog.current().plugins.find(p => p.local_id === plugin.id);
  if (cloud?.source_changed) actions.push({ label: t("catalog.replaceSource"), run: () => installCatalogPlugin(cloud, true) });
  if (plugin.from) actions.push({ label: t("plugin.update"), run: () => {
    if (updating.has(plugin.id)) return;
    updating.add(plugin.id); announce();
    ctx.say(t("plugin.updating", { name: plugin.id }));
    void invoke("plugin_update", { id: plugin.id })
      .then(fresh => { hub = fresh; ctx.say(t("plugin.updated", { name: plugin.id })); })
      .catch(error => ctx.say(fromBack(error), true))
      .finally(() => { updating.delete(plugin.id); announce(); });
  } });
  actions.push({ label: t("plugin.edit"), run: () => editor(plugin) }, {
    label: t(catalog.shared("plugins", plugin.id) ? "catalog.delete" : "plugin.remove"), danger: true,
    run: async () => {
      if (plugin.made && !await ui.confirmDialog({
        title: t("plugin.remove"), message: t("plugin.remove.made"),
        accept: t("plugin.remove.made"), cancel: t("plugin.cancel"),
      })) return;
      await remove(plugin);
    },
  });
  return { key: `plugin-actions-${plugin.id}`, id: plugin.id, kind: "plugins", description: subtitle(plugin),
    origins: catalog.installedOrigins("plugins", plugin.id), glyph: remote(plugin.source) ? "globe" : "puzzle",
    busy: updating.has(plugin.id), actions,
  };
}

const remote = (source: string) => /^https?:\/\//.test(source.trim());

/// Show the source URL for installed plugins; the internal clone path is not useful context.
function subtitle(plugin: Plugin): string {
  const where = plugin.from?.trim() || plugin.source;
  const note = plugin.note.trim();
  return note ? `${where} · ${note}` : where;
}

async function remove(plugin: Plugin) {
  if (!await catalog.confirmRemoval("plugins", plugin.id)) return;
  try {
    hub = await invoke("plugin_remove", { id: plugin.id });
    await catalog.load();
    announce();
  } catch (e) {
    ctx.say(fromBack(e), true);
  }
}

/// Refresh the snapshot after backend-owned creation.
export async function refresh() {
  hub = await invoke("plugin_hub");
  announce();
}

async function save(plugin: Plugin, revision = catalog.current().revision) {
  hub = await invoke("plugin_save", { plugin, revision });
  announce();
}

/* Editor. */

/// Read plugin.json after entering the source and populate its name/description. One step suffices because discovery reads local metadata without starting processes.
function editor(plugin: Plugin | null) {
  const revision = plugin ? catalog.current().revision : null;
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  const draft: Plugin = {
    id: plugin?.id ?? "",
    source: plugin?.source ?? "",
    note: plugin?.note ?? "",
    made: plugin?.made ?? false,
    from: plugin?.from ?? "",
  };

  const hide = () => {
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  /// Validate and inspect the source before the user finishes the remaining fields.
  async function look() {
    if (!draft.source.trim()) return;
    say(t("plugin.looking"));
    try {
      const found = await invoke("plugin_look", { source: draft.source });
      // Populate empty fields without replacing user-entered values.
      if (!draft.id.trim()) draft.id = found.id;
      if (!draft.note.trim()) draft.note = found.note;
      say("");
      paint();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function paint() {
    at(".mt").textContent = t(plugin ? "plugin.title.edit" : "plugin.title.new");
    at(".mbody").replaceChildren(
      h("p", "msay", t("plugin.intro")),
      h("p", "ui-hint", t(plugin && catalog.shared("plugins", plugin.id) ? "catalog.liveHint" : "catalog.privateHint")),
      field({
        label: "plugin.field.source",
        hint: "plugin.field.source.hint",
        value: draft.source,
        on: (v) => (draft.source = v),
        done: look,
      }),
      field({
        label: "plugin.field.name",
        hint: "plugin.field.name.hint",
        value: draft.id,
        on: (v) => (draft.id = v),
      }),
      field({
        label: "plugin.field.note",
        hint: "plugin.field.note.hint",
        value: draft.note,
        on: (v) => (draft.note = v),
      }),
    );
    const nameField = at(".mbody").querySelectorAll<HTMLInputElement>("input")[1];
    if (plugin && nameField) nameField.readOnly = true;
    if (plugin && catalog.shared("plugins", plugin.id)) at(".mbody").querySelector<HTMLInputElement>("input")!.readOnly = true;
    const back = h("button", "ghost", t("plugin.cancel"));
    back.addEventListener("click", hide);
    const go = h("button", "pri", t("plugin.save"));
    go.addEventListener("click", store);
    at(".sheetbar").replaceChildren(back, hint, go);
  }

  function store() {
    if (!draft.id.trim() || !draft.source.trim()) return say(t("plugin.needFields"), true);
    save({ ...draft, id: draft.id.trim(), source: draft.source.trim(), note: draft.note.trim() }, revision)
      .then(hide)
      .catch((e) => say(fromBack(e), true));
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  if (!plugin) at<HTMLInputElement>("input")?.focus();
}

/// Keep explanations below labels instead of in placeholders that disappear during editing.
function field(o: {
  label: Key;
  hint: Key;
  value: string;
  on: (v: string) => void;
  /// Inspect changed fields when they lose focus.
  done?: () => void;
}): HTMLElement {
  const input = ui.input(o.value);
  input.spellcheck = false;
  const box = ui.field(t(o.label), input, t(o.hint));
  input.addEventListener("input", () => o.on(input.value));
  if (o.done) input.addEventListener("change", o.done);
  return box;
}

/* Installation. */

/// Backend discovery result for a repository URL.
type Found = IpcResult<"plugin_install">;

/// Install a standalone plugin directly or offer marketplace entries for selection. Closing without selecting removes the unused clone.
function installer(prefill = "") {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  let source = prefill;
  let busy = false;
  let found: Found | null = null;
  const chosen = new Set<string>();

  const hide = () => {
    // Remove unselected clones instead of retaining unused files.
    if (found && !found.saved) void invoke("plugin_scrap", { dir: found.dir });
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  async function go() {
    if (!source.trim()) return say(t("plugin.install.needSource"), true);
    busy = true;
    say(t("plugin.install.working"));
    paint();
    try {
      const got = await invoke("plugin_install", { source });
      busy = false;
      if (got.saved) {
        await refresh();
        found = null;
        hide();
        return;
      }
      found = got;
      for (const plugin of got.plugins) chosen.add(plugin.id);
      say("");
      paint();
    } catch (e) {
      busy = false;
      say(fromBack(e), true);
      paint();
    }
  }

  /// Register selected entries while retaining their shared clone for later selections.
  async function keep() {
    const picked = found?.plugins.filter((p) => chosen.has(p.id)) ?? [];
    if (!picked.length) return hide();
    try {
      for (const plugin of picked) await save(plugin);
      if (found) found.saved = true;
      hide();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function paint() {
    at(".mt").textContent = t("plugin.install.title");
    at(".mbody").replaceChildren(...(found ? pick() : ask()));
    at(".sheetbar").replaceChildren(...bar());
  }

  function ask(): HTMLElement[] {
    return [
      h("p", "msay", t("plugin.install.intro")),
      field({
        label: "plugin.install.field",
        hint: "plugin.install.field.hint",
        value: source,
        on: (v) => (source = v),
        done: () => void go(),
      }),
    ];
  }

  /// Marketplace entry selection.
  function pick(): HTMLElement[] {
    const list = h("div", "mpick", "");
    for (const plugin of found?.plugins ?? []) {
      const line = template(
        "label",
        "mpickrow",
        `<input type="checkbox" /><div class="txt"><b></b><span></span></div>`,
      );
      const box = line.querySelector("input")!;
      box.checked = chosen.has(plugin.id);
      box.addEventListener("change", () => {
        if (box.checked) chosen.add(plugin.id);
        else chosen.delete(plugin.id);
      });
      line.querySelector("b")!.textContent = plugin.id;
      line.querySelector("span")!.textContent = plugin.note;
      list.append(line);
    }
    return [h("p", "msay", t("plugin.install.pick")), list];
  }

  function bar(): HTMLElement[] {
    const back = h("button", "ghost", t("plugin.cancel"));
    back.addEventListener("click", hide);
    const go2 = h("button", "pri", t(found ? "plugin.install.add" : "plugin.install.go"));
    go2.addEventListener("click", () => void (found ? keep() : go()));
    (go2 as HTMLButtonElement).disabled = busy;
    return [back, hint, go2];
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  at<HTMLInputElement>("input")?.focus();
}

/* Creation. */

/// File events use localized UI text; agent speech remains unchanged.
type Step = { kind: string; text: string };

/// Keep only recent progress lines; this dialog is not a full transcript viewer.
const STEPS = 8;

/// Creation switches from the name/request form to agent progress, showing written files during the potentially long operation.
function maker() {
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  const draft = { name: "", ask: "" };
  /// Null run identity means the request form, initially or after failure.
  let run: number | null = null;
  let steps: Step[] = [];
  const off: (() => void)[] = [];
  let gone = false;

  const hide = () => {
    gone = true;
    for (const stop of off.splice(0)) stop();
    veil.hidden = true;
    veil.replaceChildren();
  };

  const hint = h("span", "hint");
  const say = (text: string, bad = false) => {
    hint.textContent = text;
    hint.title = text;
    hint.classList.toggle("bad", bad);
  };

  /// Listeners belong to the dialog and filter by run identity. If registration finishes after closing, unsubscribe immediately.
  const hear = <T,>(event: string, fn: (payload: T) => void) => {
    void listen<T>(event, ({ payload }) => fn(payload)).then((stop) =>
      gone ? stop() : off.push(stop),
    );
  };
  hear<[number, Step]>("plugin-make", ([id, step]) => {
    if (id !== run) return;
    steps = [...steps, step].slice(-STEPS);
    paint();
  });
  hear<[number, string]>("plugin-made", ([id, error]) => {
    if (id !== run) return;
    // Restore the request form after failure without losing its name or instructions.
    if (error) {
      run = null;
      steps = [];
      paint();
      say(fromBack(error), true);
      return;
    }
    void refresh().finally(hide);
  });

  async function start() {
    if (!draft.name.trim() || !draft.ask.trim()) return say(t("plugin.make.needFields"), true);
    say("");
    try {
      const made = await invoke("plugin_make", {
        name: draft.name,
        ask: draft.ask,
      });
      run = made.run;
      paint();
    } catch (e) {
      say(fromBack(e), true);
    }
  }

  function stop() {
    if (run !== null) void invoke("plugin_make_stop", { run });
    hide();
  }

  /// The requested plugin name and behavior.
  function ask(): HTMLElement[] {
    return [
      h("p", "msay", t("plugin.make.intro")),
      field({
        label: "plugin.make.field.name",
        hint: "plugin.make.field.name.hint",
        value: draft.name,
        on: (v) => (draft.name = v),
      }),
      area({
        label: "plugin.make.field.ask",
        hint: "plugin.make.field.ask.hint",
        value: draft.ask,
        on: (v) => (draft.ask = v),
      }),
    ];
  }

  /// Show waiting state until the agent produces its first progress event.
  function working(): HTMLElement[] {
    const list = h("div", "mrun", "");
    if (!steps.length) list.append(h("div", "mstep wait", t("plugin.make.working")));
    for (const step of steps) {
      list.append(
        h("div", "mstep", step.kind === "file" ? t("plugin.make.wrote", { file: step.text }) : step.text),
      );
    }
    return [list];
  }

  function paint() {
    at(".mt").textContent = t("plugin.make.title");
    at(".mbody").replaceChildren(...(run === null ? ask() : working()));
    if (run === null) {
      const back = h("button", "ghost", t("plugin.cancel"));
      back.addEventListener("click", hide);
      const go = h("button", "pri", t("plugin.make.go"));
      go.addEventListener("click", () => void start());
      at(".sheetbar").replaceChildren(back, hint, go);
      return;
    }
    const halt = h("button", "ghost", t("plugin.make.stop"));
    halt.addEventListener("click", stop);
    say(t("plugin.make.working"));
    at(".sheetbar").replaceChildren(halt, hint);
  }

  paint();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  at<HTMLInputElement>("input")?.focus();
}

/// Use a multiline field so the request can describe a complete workflow.
function area(o: { label: Key; hint: Key; value: string; on: (v: string) => void }): HTMLElement {
  const input = ui.input(o.value, true);
  if (input instanceof HTMLTextAreaElement) input.rows = 6;
  input.spellcheck = true;
  const box = ui.field(t(o.label), input, t(o.hint));
  input.addEventListener("input", () => o.on(input.value));
  return box;
}
