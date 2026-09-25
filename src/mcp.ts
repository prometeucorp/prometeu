import * as ui from "./ui";
import type { ResourceItem } from "./resources/model";
import { invoke } from "./ipc";
import { icon } from "./icons";
import { fromBack, t, tn, type Key } from "./i18n";
import * as menu from "./menu";
import * as catalog from "./catalog";
import * as toolPicker from "./tool-picker";
import type { McpCheck, McpServer, McpStep, ProviderId, Selection } from "./types";
import { $, h, template } from "./util";

/// Present the backend-owned MCP registry in Settings and shared launcher/conversation pickers. Writes replace the whole local snapshot. Keep secrets and persistence in the backend; reopen the existing single-choice menu after each checkbox selection.

let hub: McpServer[] = [];
/// Backend OAuth state identifies authenticated servers without exposing tokens.
let logins: string[] = [];
/// Connection results belong to this Mac and this configuration, never to the cloud catalog.
type Connection = { config: string; check?: McpCheck; error?: string; pending?: Key };
const connections = new Map<string, Connection>();
let loaded = false;
const watchers = new Set<() => void>();

/// The initial empty registry redraws when loading completes.
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
    hub = await invoke("mcp_hub");
    logins = await invoke("mcp_logins");
    announce();
  } catch {
    // Hide hub pickers if the backend is unavailable or older.
  }
}

/// Deleted registry entries remain in persisted workspace choices. Show unavailable selections explicitly so users can remove them.
export const known = (id: string) => hub.some((s) => s.id === id);

/// Servers discovered from the person's CLI configuration that the hub lacks (ADR 0046). They form
/// the visible inherited base of the workspace picker, cached per workspace and provider because discovery
/// reads files under the workspace directory.
const inherited = new Map<string, McpServer[]>();
/// Workspaces with a discovery in flight, so repeated paints do not stack fetches.
const inflight = new Set<string>();

const inheritedKey = (workspace: string, agent?: ProviderId) => `${workspace}\u0000${agent ?? ""}`;
export const inheritedOf = (workspace: string, agent?: ProviderId) => inherited.get(inheritedKey(workspace, agent)) ?? [];

/// Fetch the inherited base once per workspace and provider; the announce repaints gated composer buttons. A
/// failure leaves no cache entry, so a later paint retries instead of pinning the empty base;
/// opening the picker rediscovers it anyway.
export function loadInherited(workspace: string, agent?: ProviderId) {
  const key = inheritedKey(workspace, agent);
  if (inherited.has(key) || inflight.has(key)) return;
  inflight.add(key);
  void (async () => {
    try {
      inherited.set(key, await invoke("mcp_inherited", { id: workspace, agent }));
    } catch {
      // An unavailable backend leaves the base uncached for a retry.
    } finally {
      inflight.delete(key);
    }
    announce();
  })();
}

/// Authenticated state controls the Settings label and sign-in/sign-out action.
export const signedIn = (id: string) => logins.includes(id);

async function refreshLogins() {
  try {
    logins = await invoke("mcp_logins");
    announce();
  } catch {
    // Preserve the previous authentication list if refresh fails.
  }
}

/* Picker. */

type Pick = {
  /// The workspace whose layer is edited.
  workspace: string;
  agent?: ProviderId;
  /// The workspace MCP layer; null inherits the global and project layers.
  current: () => Selection | null;
  /// Persist the new layer, or null to return the axis to inherit.
  set: (sel: Selection | null) => Promise<void> | void;
  /// Menu anchor position.
  at: () => { x: number; y: number };
  /// Opens the project-trust prompt when the project layer has pending items.
  trust?: () => void;
};

/// Provenance-aware MCP picker (ADR 0045): it shows the resolved effective set and writes the
/// workspace layer as deltas over what the global and project layers already contribute. Rows come
/// from the hub plus the CLI-inherited base (ADR 0046), so servers Claude Code loads on its own are
/// visible and removable without importing them first.
export async function openPicker(p: Pick) {
  try {
    inherited.set(inheritedKey(p.workspace, p.agent), await invoke("mcp_inherited", { id: p.workspace, agent: p.agent }));
    // Rediscovery may change the base; repaint the gated composer buttons that read the cache.
    announce();
  } catch {
    // Keep the cached base when the backend cannot rediscover it.
  }
  const rows: toolPicker.Row[] = hub.map((server) => ({
    id: server.id,
    label: server.id,
    hint: subtitle(server),
    section: t("tools.section.hub"),
  }));
  for (const server of inheritedOf(p.workspace, p.agent)) {
    if (!rows.some((r) => r.id === server.id))
      rows.push({ id: server.id, label: server.id, hint: subtitle(server), section: t("tools.section.cli") });
  }
  const current = p.current();
  // Retain ids the registry no longer has so they can still be dropped from the layer.
  for (const id of [...(current?.add ?? []), ...(current?.remove ?? [])]) {
    if (!known(id) && !rows.some((r) => r.id === id)) rows.push({ id, label: t("mcp.gone", { name: id }) });
  }
  void toolPicker.open({
    workspace: p.workspace,
    agent: p.agent,
    axis: "mcp",
    rows,
    current: p.current,
    set: p.set,
    at: p.at,
    noneLabel: t("mcp.none"),
    trust: p.trust,
  });
}

/// Summarize the workspace layer for the button: inherit, a single pick, or its +/- deltas.
export function label(sel: Selection | null): string {
  if (!sel) return t("mcp.default");
  if (sel.base === "none") {
    // Under an explicit none base the layer is a flat list; removals are no-ops there.
    if (!sel.add.length) return t("mcp.zero");
    if (sel.add.length === 1) return sel.add[0];
    return t("mcp.count", { n: String(sel.add.length) });
  }
  if (sel.add.length === 1 && !sel.remove.length) return sel.add[0];
  const bits = [sel.add.length ? `+${sel.add.length}` : "", sel.remove.length ? `−${sel.remove.length}` : ""].filter(Boolean);
  return bits.length ? bits.join(" ") : t("mcp.default");
}

/// Label for the launcher's flat default preset, which is a plain id list rather than a layered delta.
export function flatLabel(ids: string[] | null): string {
  if (ids === null) return t("mcp.default");
  if (!ids.length) return t("mcp.zero");
  if (ids.length === 1) return ids[0];
  return t("mcp.count", { n: String(ids.length) });
}

/// Picker for the launcher's default preset: a flat list over the registry, stored as `string[] | null`
/// rather than as a board layer. New workspaces start from this preset.
export function openDefaultPicker(p: {
  chosen: () => string[] | null;
  set: (ids: string[] | null) => void;
  at: () => { x: number; y: number };
}) {
  const rows: toolPicker.Row[] = hub.map((server) => ({ id: server.id, label: server.id, hint: subtitle(server) }));
  const asSel = (): Selection | null => {
    const ids = p.chosen();
    return ids === null ? null : { base: "none", add: ids, remove: [] };
  };
  toolPicker.openFlat({
    rows,
    current: asSel,
    set: (sel) => p.set(sel === null ? null : sel.add),
    at: p.at,
    noneLabel: t("mcp.none"),
  });
}

/// Picker for the board's global MCP layer (ADR 0045), the base every project and workspace inherits.
/// It is a flat selection because nothing sits above it.
export function openGlobalPicker(p: {
  current: () => Selection | null;
  set: (sel: Selection | null) => void;
  at: () => { x: number; y: number };
}) {
  const rows: toolPicker.Row[] = hub.map((server) => ({ id: server.id, label: server.id, hint: subtitle(server) }));
  toolPicker.openFlat({ rows, current: p.current, set: p.set, at: p.at, noneLabel: t("mcp.none") });
}

/* Settings list. */

type Ctx = { say: (text: string, isError?: boolean) => void };
let ctx: Ctx;

export function init(context: Ctx) {
  ctx = context;
}

export function resourceItems(): ResourceItem[] {
  return [...hub.map(serverResource), ...catalog.pendingResources("mcp", hub.map(server => server.id), ctx.say)];
}

export function settingsActions(anchor: HTMLElement): menu.Item[] {
  return [
    { label: t("mcp.add"), run: () => editor(null) },
    { label: t("mcp.import"), run: () => void importer(anchor) },
  ];
}

function serverResource(server: McpServer): ResourceItem {
  const item: ResourceItem = {
    key: `mcp-actions-${server.id}`, id: server.id, kind: "mcp", description: subtitle(server),
    origins: server.config.builtin === true ? [{ label: t("settings.builtin") }] : catalog.installedOrigins("mcp", server.id),
    glyph: kind(server) === "stdio" ? "terminal" : "globe", actions: [],
  };
  if (server.config.builtin === true) return item;

  const config = JSON.stringify(server.config);
  const previous = connections.get(server.id);
  // Preserve pending operations across catalog replacement, then discard results for old definitions.
  if (!previous || (previous.config !== config && !previous.pending)) connections.set(server.id, { config });
  const connection = connections.get(server.id)!;
  const check = connection.check;
  const status: Key = connection.pending ?? (connection.error ? "mcp.status.error"
    : check?.probe.auth ? "mcp.status.auth"
    : check?.probe.ok ? "mcp.connected"
    : check ? "mcp.status.error" : "mcp.status.unchecked");
  const detail = connection.error || check?.steps.find(s => !s.ok)?.detail || check?.probe.detail;
  item.status = [t(status), detail ? fromBack(detail) : ""].filter(Boolean).join(" · ");
  item.busy = !!connection.pending;
  item.actions.push({ label: t("mcp.check"), run: () => void connect(server, connection, "check") });
  if (kind(server) === "url") {
    if (check?.probe.auth || (!signedIn(server.id) && !check?.probe.ok)) {
      item.actions.push({ label: t("mcp.authenticate"), run: () => void connect(server, connection, "login") });
    }
    if (signedIn(server.id)) item.actions.push({ label: t("mcp.logout"), run: () => void connect(server, connection, "logout") });
  }
  item.actions.push(
    { label: t("mcp.edit"), run: () => editor(server) },
    ...catalog.resourceActions("mcp", server.id, ctx.say),
    { label: t(catalog.shared("mcp", server.id) ? "catalog.delete" : "mcp.remove"), danger: true, run: () => void remove(server) },
  );
  return item;
}

/// Operate on the registered definition without saving or publishing it. Recheck after login/logout
/// so a stored token alone never implies a working connection.
async function connect(server: McpServer, connection: Connection, action: "check" | "login" | "logout") {
  if (connection.pending) return;
  connection.pending = action === "login" ? "mcp.login.doing" : "mcp.check.doing";
  connection.error = undefined;
  connection.check = undefined;
  announce();
  try {
    if (action === "login") await invoke("mcp_login", { server });
    if (action === "logout") await invoke("mcp_logout", { id: server.id });
    if (action !== "check") await refreshLogins();
    connection.pending = "mcp.check.doing";
    announce();
    connection.check = await invoke("mcp_check", { server });
  } catch (e) {
    connection.error = fromBack(e);
  } finally {
    connection.pending = undefined;
    announce();
  }
}

/// Transport selects the fields and icon: local stdio process or remote URL.
function kind(server: McpServer): "stdio" | "url" {
  return server.config.builtin === true || typeof server.config.command === "string" ? "stdio" : "url";
}

/// Describe the server and its source beneath its name.
function subtitle(server: McpServer): string {
  if (server.config.builtin === true) return t("mcp.builtin.description");
  const what =
    kind(server) === "stdio"
      ? [server.config.command, ...((server.config.args as string[]) ?? [])].join(" ")
      : String(server.config.url ?? "");
  const from = server.note.trim();
  return from ? `${what} · ${from}` : what;
}

async function remove(server: McpServer) {
  if (!await catalog.confirmRemoval("mcp", server.id)) return;
  try {
    hub = await invoke("mcp_remove", { id: server.id });
    connections.delete(server.id);
    await catalog.load();
    announce();
  } catch (e) {
    ctx.say(fromBack(e), true);
  }
}

async function save(server: McpServer, revision = catalog.current().revision) {
  hub = await invoke("mcp_save", { server, revision });
  announce();
}

/// Refresh after backend changes from cloud catalogs.
export async function refresh() {
  hub = await invoke("mcp_hub");
  announce();
}

/* Editor. */

/// Keep incomplete form input separate from McpServer until checking or saving.
export type Draft = {
  stdio: boolean;
  id: string;
  cmd: string;
  url: string;
  /// Preserve environment/header row order while editing rather than rebuilding from object key order.
  pairs: [string, string][];
  note: string;
};

export function toDraft(server: McpServer | null): Draft {
  const config = server?.config ?? {};
  return {
    stdio: server ? kind(server) === "stdio" : true,
    id: server?.id ?? "",
    cmd: [config.command, ...((config.args as string[]) ?? [])].filter(Boolean).join(" "),
    url: String(config.url ?? ""),
    pairs: Object.entries((config.env ?? config.headers ?? {}) as Record<string, string>),
    note: server?.note ?? "",
  };
}

/// Convert form input into provider configuration; return null when required fields are missing.
export function toServer(d: Draft): McpServer | null {
  const id = d.id.trim();
  const parts = d.cmd.trim().split(/\s+/).filter(Boolean);
  const url = d.url.trim();
  if (!id || (d.stdio ? !parts.length : !url)) return null;
  const pairs = Object.fromEntries(
    d.pairs.map(([k, v]) => [k.trim(), v.trim()] as const).filter(([k]) => k),
  );
  return {
    id,
    config: d.stdio
      ? { type: "stdio", command: parts[0], args: parts.slice(1), env: pairs }
      : { type: "http", url, headers: pairs },
    note: d.note.trim(),
  };
}

/// Registration uses two steps: identify the server and probe it, then resolve authentication, variables/headers, and description. Show each probe stage. Existing servers open at the second step; the summary returns to connection details.
function editor(server: McpServer | null) {
  let revision = server ? catalog.current().revision : null;
  const veil = $("veil");
  const sheet = template(
    "div",
    "sheet hubedit",
    `<div class="sheettop"><b class="mt"></b></div><div class="mbody"></div><div class="sheetbar"></div>`,
  );
  const at = <T extends HTMLElement>(sel: string) => sheet.querySelector(sel) as T;
  const draft = toDraft(server);
  /// Reset the probe result when changing transport because it describes the previous server.
  let check: McpCheck | null = null;
  let checking = false;
  /// Disable the primary action while probing so navigation cannot outrun its result.
  let go: HTMLButtonElement | null = null;

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

  /// Both steps share footer layout and inline status.
  function foot(left: [Key, () => void], right: [Key, () => void]): HTMLButtonElement {
    const back = h("button", "ghost", t(left[0]));
    back.addEventListener("click", left[1]);
    go = h("button", "pri", t(right[0])) as HTMLButtonElement;
    go.addEventListener("click", right[1]);
    go.disabled = checking;
    at(".sheetbar").replaceChildren(back, hint, go);
    return go;
  }

  /* Step one: identity and connection. */

  /// Update probe rows without rebuilding fields and disrupting focus.
  const checkBox = h("div", "mcheck");
  const paintCheck = () => {
    checkBox.hidden = !checking && !check;
    checkBox.replaceChildren(...checkRows(draft, check, checking));
  };

  async function examine() {
    const built = toServer(draft);
    if (!built) return;
    checking = true;
    check = null;
    say("");
    if (go) go.disabled = true;
    paintCheck();
    try {
      check = await invoke("mcp_check", { server: built });
    } catch (e) {
      say(fromBack(e), true);
    }
    checking = false;
    if (go) go.disabled = false;
    paintCheck();
    // Scroll the newly added probe result into view.
    checkBox.scrollIntoView({ block: "end" });
  }

  function first() {
    at(".mt").textContent = t(server ? "mcp.title.edit" : "mcp.title.new");
    // Local programs and remote URLs are mutually exclusive transports; switching resets their fields and probe.
    const kinds = h("div", "mkind");
    const sw = (on: boolean, key: Key, pick: () => void) => {
      const b = template("button", `ghost sw${on ? " on" : ""}`, `<span></span><i class="knob"></i>`);
      b.setAttribute("role", "switch");
      b.setAttribute("aria-checked", String(on));
      b.children[0].textContent = t(key);
      b.addEventListener("click", () => {
        pick();
        check = null;
        first();
      });
      return b;
    };
    kinds.append(
      sw(draft.stdio, "mcp.kind.stdio", () => (draft.stdio = true)),
      sw(!draft.stdio, "mcp.kind.url", () => (draft.stdio = false)),
    );
    at(".mbody").replaceChildren(
      h("p", "msay", t("mcp.intro")),
      kinds,
      field({
        label: "mcp.field.name",
        hint: "mcp.field.name.hint",
        value: draft.id,
        on: (v) => (draft.id = v),
      }),
      draft.stdio
        ? field({
            label: "mcp.field.command",
            hint: "mcp.field.command.hint",
            value: draft.cmd,
            on: (v) => (draft.cmd = v),
            done: examine,
          })
        : field({
            label: "mcp.field.url",
            hint: "mcp.field.url.hint",
            value: draft.url,
            on: (v) => (draft.url = v),
            done: examine,
          }),
      checkBox,
    );
    // Reset scroll when changing steps so the new step starts at its heading.
    if (server) { const name = at(".mbody").querySelector<HTMLInputElement>("input"); if (name) name.readOnly = true; }
    at(".mbody").scrollTop = 0;
    paintCheck();
    foot(["mcp.cancel", hide], ["mcp.next", advance]);
  }

  /// Probe before continuing when needed. Allow proceeding after failure so temporarily unreachable servers can still be registered.
  async function advance() {
    if (!toServer(draft)) return say(t("mcp.needFields"), true);
    if (!check) await examine();
    second();
  }

  /* Step two: probe-dependent choices. */

  function second() {
    at(".mt").textContent = t(server ? "mcp.title.edit" : "mcp.title.new");
    at(".mbody").replaceChildren(
      h("p", "ui-hint", t(server && catalog.shared("mcp", server.id) ? "catalog.liveHint" : "catalog.privateHint")),
      resume(),
      // Only remote servers use OAuth; local processes receive credentials through environment variables.
      ...(draft.stdio ? [] : [auth()]),
      pairsSection(),
      field({
        label: "mcp.field.note",
        hint: "mcp.field.note.hint",
        value: draft.note,
        on: (v) => (draft.note = v),
      }),
    );
    at(".mbody").scrollTop = 0;
    // Both steps offer cancellation. The summary navigates back, including when editing starts on step two.
    foot(["mcp.cancel", hide], ["mcp.save", store]);
  }

  /// The identity/probe summary is a button that returns to connection details.
  function resume(): HTMLElement {
    const row = template(
      "button",
      "mhead",
      `<div class="txt"><b></b><span class="addr"></span><span class="said"></span></div><span class="pen"></span>`,
    );
    row.querySelector("b")!.textContent = draft.id.trim();
    row.querySelector(".addr")!.textContent = (draft.stdio ? draft.cmd : draft.url).trim();
    const said = check?.probe.ok
      ? [check.probe.name, tn(check.probe.tools, "mcp.found.tools")].filter(Boolean).join(" · ")
      : "";
    row.querySelector(".said")!.textContent = said;
    row.querySelector(".pen")!.innerHTML = icon("pencil", 14);
    row.addEventListener("click", first);
    return row;
  }

  /// Authentication requirements come from the server; the UI offers sign-in or sign-out.
  function auth(): HTMLElement {
    const inside = signedIn(draft.id.trim());
    const asks = check?.probe.auth ?? false;
    // If a server requires login but cannot register clients dynamically, show the failed probe step instead of an unusable login.
    const open = asks && check!.steps.every((s) => s.ok);
    const body: Key = inside
      ? "mcp.auth.in"
      : asks
        ? open
          ? "mcp.auth.needed"
          : "mcp.auth.blocked"
        : check?.probe.ok
          ? "mcp.auth.no"
          : "mcp.auth.unknown";
    const box = section("mcp.auth", body);
    // Offer login for authentication-required or unprobed servers so attempting it can reveal a useful error.
    if (inside || open || !check) {
      const btn = h("button", "outline md", t(inside ? "mcp.logout" : "mcp.login")) as HTMLButtonElement;
      btn.addEventListener("click", () => void enter(btn));
      box.append(btn);
    }
    return box;
  }

  async function enter(btn: HTMLButtonElement) {
    const built = toServer(draft);
    if (!built) return say(t("mcp.needFields"), true);
    if (signedIn(built.id)) {
      try {
        await invoke("mcp_logout", { id: built.id });
      } catch (e) {
        return say(fromBack(e), true);
      }
      connections.delete(built.id);
      await refreshLogins();
      return second();
    }
    // Existing definitions authenticate independently of cloud revisions. Connection edits must
    // be saved explicitly first; only new servers need registration before their first login.
    if (server && JSON.stringify(built.config) !== JSON.stringify(toServer(toDraft(server))!.config)) {
      return say(t("mcp.auth.saveFirst"), true);
    }
    btn.disabled = true;
    say(t("mcp.login.doing"));
    try {
      if (!server) {
        await save(built, revision);
        await catalog.load(); revision = catalog.current().revision;
      }
      await invoke("mcp_login", { server: server ?? built });
      connections.delete(built.id);
      await refreshLogins();
      say(t("mcp.login.ok"));
      // Probe again after authentication to replace the previous login-required status.
      await examine();
      second();
    } catch (e) {
      say(fromBack(e), true);
      btn.disabled = false;
    }
  }

  /// Edit variables or headers as individual key/value rows with explicit removal.
  function pairsSection(): HTMLElement {
    const box = section(
      draft.stdio ? "mcp.pairs.env" : "mcp.pairs.headers",
      draft.stdio ? "mcp.pairs.env.body" : "mcp.pairs.headers.body",
    );
    const rows = h("div", "mpairs");
    const paint = () => {
      rows.replaceChildren(
        ...draft.pairs.map((pair, i) =>
          pairRow(pair, () => {
            draft.pairs.splice(i, 1);
            paint();
          }),
        ),
      );
    };
    paint();
    const add = template("button", "ghost md", `${icon("plus", 14)}<span></span>`);
    add.children[1].textContent = t("mcp.pairs.add");
    add.addEventListener("click", () => {
      draft.pairs.push(["", ""]);
      paint();
      (rows.lastElementChild?.querySelector("input") as HTMLInputElement | null)?.focus();
    });
    box.append(rows, add);
    return box;
  }

  /// Save the current fields even if probing failed.
  function store() {
    const built = toServer(draft);
    if (!built) return say(t("mcp.needFields"), true);
    save(built, revision)
      .then(hide)
      .catch((e) => say(fromBack(e), true));
  }

  if (server) second();
  else first();
  veil.replaceChildren(sheet);
  veil.hidden = false;
  // Focus the first field only for new registrations; editing begins with populated details.
  if (!server) at<HTMLInputElement>("input")?.focus();
}

function pairRow(pair: [string, string], drop: () => void): HTMLElement {
  const row = h("div", "mpair");
  const cell = (which: 0 | 1, place: Key) => {
    const input = h("input", "") as HTMLInputElement;
    input.spellcheck = false;
    input.placeholder = t(place);
    input.value = pair[which];
    input.addEventListener("input", () => (pair[which] = input.value));
    return input;
  };
  const x = template("button", "ico sm", icon("x", 14));
  x.title = t("mcp.pairs.drop");
  x.addEventListener("click", drop);
  row.append(cell(0, "mcp.pairs.key"), cell(1, "mcp.pairs.value"), x);
  return row;
}

/// Keep field explanations visible below labels instead of hiding them in placeholders while typing.
function field(o: {
  label: Key;
  hint: Key;
  value: string;
  on: (v: string) => void;
  /// Probe after a changed field loses focus, not on each keystroke; probes start processes or access the network.
  done?: () => void;
}): HTMLElement {
  const input = ui.input(o.value);
  input.spellcheck = false;
  const box = ui.field(t(o.label), input, t(o.hint));
  input.addEventListener("input", () => o.on(input.value));
  if (o.done) input.addEventListener("change", o.done);
  return box;
}

/// A second-step section groups its title, explanation, and controls.
function section(title: Key, body: Key): HTMLElement {
  const box = template("div", "msect", `<b></b><span class="sb"></span>`);
  box.children[0].textContent = t(title);
  box.children[1].textContent = t(body);
  return box;
}

/// Render expected probe stages while the backend works. OAuth stages appear only when the response indicates authentication is needed.
const AHEAD: Record<"stdio" | "url", string[]> = {
  stdio: ["spawn", "handshake", "tools"],
  url: ["connect", "handshake", "tools"],
};

function checkRows(draft: Draft, check: McpCheck | null, running: boolean): HTMLElement[] {
  const keys = check ? check.steps.map((s) => s.key) : AHEAD[draft.stdio ? "stdio" : "url"];
  const rows = keys.map((key, i) => stepRow(key, check?.steps[i] ?? null));
  return [
    h("b", "ch", t(running ? "mcp.check.doing" : "mcp.check.done")),
    ...rows,
    ...(check ? [verdict(check)] : []),
  ];
}

function stepRow(key: string, step: McpStep | null): HTMLElement {
  const row = template(
    "div",
    `crow${step ? (step.ok ? " ok" : " bad") : ""}`,
    `<span class="cg"></span><div class="txt"><b></b><span></span></div><span class="cc"></span>`,
  );
  row.querySelector(".cg")!.innerHTML = step ? icon(step.ok ? "check" : "x", 14) : `<i class="spin"></i>`;
  row.querySelector("b")!.textContent = t(`mcp.step.${key}` as Key);
  row.querySelector(".txt span")!.textContent = !step
    ? t("mcp.step.wait")
    : step.ok
      ? t("mcp.step.ok")
      : // An empty tool list needs a local explanation even when the server responded successfully.
        step.detail || t(key === "tools" ? "mcp.step.tools.none" : "mcp.step.fail");
  row.querySelector(".cc")!.textContent = step?.note ?? "";
  return row;
}

/// Summarize what the probe means for the next step.
function verdict(check: McpCheck): HTMLElement {
  const { ok, auth } = check.probe;
  const box = template("div", `cnote${ok || auth ? "" : " bad"}`, `<span class="ic"></span><span></span>`);
  box.querySelector(".ic")!.innerHTML = icon(ok || auth ? "check" : "x", 14);
  box.children[1].textContent = t(ok ? "mcp.found.ok" : auth ? "mcp.found.auth" : "mcp.found.fail");
  return box;
}

/* Import. */

/// Discover CLI configuration missing from the hub without modifying the original files.
async function importer(btn: HTMLElement) {
  let found: McpServer[] = [];
  try {
    found = await invoke("mcp_found");
  } catch (e) {
    ctx.say(fromBack(e), true);
    return;
  }
  const at = btn.getBoundingClientRect();
  if (!found.length) {
    menu.openAt({ x: at.left, y: at.bottom + 4 }, [{ label: t("mcp.import.none"), disabled: true }]);
    return;
  }
  menu.openAt(
    { x: at.left, y: at.bottom + 4 },
    found.map((server) => ({
      label: server.id,
      hint: server.note.trim() || t("mcp.origin.user"),
      run: () => {
        save(server).catch((e) => ctx.say(fromBack(e), true));
      },
    })),
  );
}
