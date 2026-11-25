import { listen } from "@tauri-apps/api/event";
import { fromBack, t } from "./i18n";
import { icon } from "./icons";
import { iconButton } from "./components/icon-button";
import { invoke } from "./ipc";
import * as menu from "./menu";
import { button, confirmDialog, field, input, notice } from "./ui";
import type { Project } from "./types";
import { h, template } from "./util";

/// Remote machines (ADR 0101): computers reached over SSH whose `prometeu-host` daemon runs workspaces for this app.
/// Rust owns the connections and publishes their state on the `machines` event; this module mirrors it for Settings,
/// the launcher and the badges that show where each workspace runs.

export type MachinePhase = "offline" | "connecting" | "connected" | "not_installed" | "replaced" | "failed";
export type Limits = { memory_max: string; cpu_quota: string };
export type HostInfo = { home: string; user: string; os: string; arch: string; version: string; linger: boolean; scopes: boolean; executable: string | null };
export type ClaudeStatus = { installed: boolean; loggedIn: boolean; email: string | null; method: string | null };
export type AgentsStatus = { enabled: boolean; socket: string; serving: boolean; error: string | null };
/** Shown once after enabling agents or rotating their token: the container mount and the MCP configuration with the token. */
export type AgentsSetup = { mount: string; config: string; preview: string; client_uid: number; local_uid: number; opened: string[]; moved_from: string | null };
/** Enabling stops without changing anything when folders under the machine's home must be opened to other users first. */
export type AgentsEnable = AgentsSetup | { needs_open: string[] };
export type MachineView = { id: string; name: string; ssh: string; limits: Limits; phase: MachinePhase; detail: string | null; host: HostInfo | null; update_pending: boolean; claude?: ClaudeStatus | null; agents?: AgentsStatus | null };
export type MachineInput = { id?: string | null; name: string; ssh: string; limits: Limits };
export type WorkspaceStats = { memory: number | null; memory_max: number | null; cpu_usec: number | null; cpu_max: string | null };
export type MachineStats = { workspaces: Record<string, WorkspaceStats>; memory_total: number | null; memory_available: number | null; load: number | null; cpus: number; terminals: number };
export type RemoteListing = { path: string; home: string; entries: { name: string; git: boolean }[] };

let list: MachineView[] = [];
const listeners = new Set<() => void>();

export async function init() {
  void listen<MachineView[]>("machines", ({ payload }) => { list = payload; listeners.forEach(fn => fn()); });
  try { list = await invoke("machines"); } catch { list = []; }
  listeners.forEach(fn => fn());
}

export const machines = () => list;
export const machineOf = (id: string | undefined | null) => (id ? list.find(m => m.id === id) : undefined);
export function onChange(fn: () => void) { listeners.add(fn); return () => listeners.delete(fn); }

/// The machine's display name; a machine forgotten while a workspace still names it falls back to its id.
export const machineName = (id: string) => machineOf(id)?.name ?? id;
export const online = (id: string | undefined) => machineOf(id)?.phase === "connected";

/** Only registered folders on the selected machine can be chosen for a new workspace. */
export function projectsOn(projects: Project[], host?: string): Project[] {
  return projects.filter(project => (project.host ?? "") === (host ?? ""));
}

/// A machine's clone of a repository that is also a project on this computer is shown as that project (ADR 0101):
/// remote project ID → local project ID, matched by normalized `origin`.
export function aliases(projects: Project[]): Map<string, string> {
  const local = new Map(projects.filter(p => !p.host && p.origin).map(p => [p.origin!, p.id]));
  return new Map(projects.flatMap(p => (p.host && p.origin && local.has(p.origin) ? [[p.id, local.get(p.origin)!] as const] : [])));
}

/// A compact chip naming the machine a workspace or project runs on, with its connection state. Local work shows none.
/// `creator` names the agent that created the workspace through the machine's host (ADR 0103); empty when it gave no name.
export function badge(host: string | undefined, compact = false, creator?: string): HTMLElement | null {
  if (!host) return null;
  const machine = machineOf(host);
  const state = machine?.phase ?? "offline";
  const byAgent = creator !== undefined;
  const by = byAgent ? t("remote.createdBy", { name: creator || t("remote.agent") }) : "";
  const chip = template("span", `machine-badge ${state === "connected" ? "on" : "off"}${compact ? " compact" : ""}`, `${icon(byAgent ? "sparkles" : "server", 12)}<span></span>`);
  chip.querySelector("span")!.textContent = compact ? "" : [machineName(host), by].filter(Boolean).join(" · ");
  const where = state === "connected"
    ? t("remote.badge", { name: machineName(host) })
    : t("remote.offline", { name: machineName(host) });
  const title = [by, where].filter(Boolean).join(" · ");
  chip.title = title;
  chip.setAttribute("aria-label", title);
  return chip;
}

/* ---------- Settings ---------- */

let editing: string | null = null;
/// A Claude sign-in waiting for the code the person sees after signing in, per machine.
const signing = new Map<string, string>();
/// The agent configuration just generated, per machine. Kept only in memory until the person closes it: the token is
/// never stored, logged or sent anywhere else (ADR 0103).
const agentSetups = new Map<string, { setup: AgentsSetup; ssh: string }>();
const busy = new Set<string>();
let feedback: { id: string; text: string; error: boolean } | null = null;

export function settingsRows(say: (text: string, isError?: boolean) => void, redraw: () => void): HTMLElement[] {
  const intro = template("div", "setrow head machine-intro", `<div class="txt"><span></span></div><div class="act"></div>`);
  intro.querySelector(".txt span")!.textContent = t("machines.intro");
  if (editing !== "") {
    const add = button(t("machines.add"), () => { editing = ""; redraw(); }, "outline", "machines.add");
    add.insertAdjacentHTML("afterbegin", `<span class="ic">${icon("plus", 14)}</span>`);
    intro.querySelector(".act")!.append(add);
  }
  const rows: HTMLElement[] = [intro];
  if (editing === "") rows.push(form(null, say, redraw));
  for (const machine of list) rows.push(...(editing === machine.id ? [form(machine, say, redraw)] : machineRows(machine, say, redraw)));
  return rows;
}

function run(id: string, redraw: () => void, say: (text: string, isError?: boolean) => void, work: () => Promise<string | null>) {
  if (busy.has(id)) return;
  busy.add(id); redraw();
  void work()
    .then(text => { feedback = text ? { id, text, error: false } : null; })
    .catch(error => { feedback = { id, text: fromBack(error), error: true }; say(fromBack(error), true); })
    .finally(() => { busy.delete(id); redraw(); });
}

/// One row per machine: what it is and whether it can run work now. Only the action the machine needs shows as a
/// button; the rest stay in its menu.
function machineRows(machine: MachineView, say: (text: string, isError?: boolean) => void, redraw: () => void): HTMLElement[] {
  const node = template("div", "setrow machine-row", `<span class="glyph">${icon("server", 18)}</span><div class="txt"><b><span class="machine-name"></span><span class="machine-status"></span></b></div><div class="act"></div>`);
  node.querySelector(".machine-name")!.textContent = machine.name;
  const status = node.querySelector<HTMLElement>(".machine-status")!;
  status.dataset.phase = machine.phase;
  status.textContent = machine.phase === "failed" ? t("machines.phase.error") : t(`machines.phase.${machine.phase}`);
  const txt = node.querySelector(".txt")!;
  const line = (text: string, cls = "") => { const span = h("span", cls, text); txt.append(span); return span; };
  line([machine.ssh, machine.host && `${machine.host.user} · ${machine.host.os} ${machine.host.arch}`].filter(Boolean).join(" · "), "machine-where");
  if (machine.phase === "failed" && machine.detail) line(machine.detail, "bad");
  const { memory_max, cpu_quota } = machine.limits;
  line(memory_max || cpu_quota
    ? t("machines.limitsSummary", { memory: memory_max || "∞", cpu: cpu_quota || "∞" })
    : t("machines.noLimits"));
  const claude = machine.phase === "connected" ? machine.claude : undefined;
  const agents = machine.phase === "connected" ? machine.agents : undefined;
  if (agents) {
    if (agents.enabled && agents.error) line(t("machines.agents.blocked", { error: agents.error }), "warn");
    else line(agents.enabled ? t("machines.agents.on") : t("machines.agents.off"));
  }
  if (machine.phase === "connected") {
    line(!claude ? t("machines.claude.checking")
      : !claude.installed ? t("machines.claude.missing")
      : claude.loggedIn ? (claude.email ? t("machines.claude.signedInAs", { email: claude.email }) : t("machines.claude.signedIn"))
      : t("machines.claude.signedOut"), claude && !(claude.installed && claude.loggedIn) ? "warn" : "");
  }

  const working = busy.has(machine.id) || busy.has(`claude:${machine.id}`) || busy.has(`agents:${machine.id}`);
  const install = () => run(machine.id, redraw, say, async () => { await invoke("machine_install", { id: machine.id }); return t("machines.installed"); });
  const reconnect = () => run(machine.id, redraw, say, async () => { await invoke("machine_connect", { id: machine.id }); return null; });
  const signIn = () => run(`claude:${machine.id}`, redraw, say, async () => {
    const url = await invoke("machine_claude_login", { id: machine.id });
    signing.set(machine.id, url);
    void invoke("open_external", { url }).catch(() => {});
    return null;
  });
  const checkClaude = () => run(`claude:${machine.id}`, redraw, say, async () => { await invoke("machine_claude_status", { id: machine.id }); return null; });
  const enableAgents = (rotate: boolean) => run(`agents:${machine.id}`, redraw, say, async () => {
    if (rotate && !await confirmDialog({ track: "machines.agents.rotate", area: "settings", title: t("machines.agents.rotate"),
      message: t("machines.agents.rotateConfirm"), accept: t("machines.agents.rotate"), cancel: t("machines.cancel") })) return null;
    // A configuration is only ever shown for the destination it was made on.
    const ssh = machine.ssh;
    let result = await invoke("machine_agents_enable", { id: machine.id, rotate, open_home: false });
    if ("needs_open" in result) {
      if (!await confirmDialog({ track: "machines.agents.openHome", area: "settings", title: t("machines.agents.openHome"),
        message: t("machines.agents.openHomeConfirm", { folders: result.needs_open.join(", ") }),
        accept: t("machines.agents.openHome"), cancel: t("machines.cancel") })) return null;
      result = await invoke("machine_agents_enable", { id: machine.id, rotate, open_home: true });
    }
    if (!("needs_open" in result) && machineOf(machine.id)?.ssh === ssh) agentSetups.set(machine.id, { setup: result, ssh });
    return null;
  });
  const disableAgents = () => run(`agents:${machine.id}`, redraw, say, async () => {
    if (!await confirmDialog({ track: "machines.agents.disable", area: "settings", title: t("machines.agents.disable"),
      message: t("machines.agents.disableConfirm"), accept: t("machines.agents.disable"), cancel: t("machines.cancel") })) return null;
    await invoke("machine_agents_disable", { id: machine.id });
    agentSetups.delete(machine.id);
    return null;
  });
  const offline = machine.phase === "offline" || machine.phase === "failed" || machine.phase === "replaced";
  const act = node.querySelector(".act")!;
  const primary = machine.phase === "not_installed" ? button(t("machines.install"), install, "pri", "machines.install")
    : offline ? button(t("machines.reconnect"), reconnect, "outline", "machines.reconnect")
    : claude?.installed && !claude.loggedIn && !signing.has(machine.id) ? button(t("machines.claude.signIn"), signIn, "pri", "machines.claude.signIn")
    : agents && !agents.enabled ? button(t("machines.agents.enable"), () => enableAgents(false), "outline", "machines.agents.enable")
    : null;
  if (primary) { primary.disabled = working; act.append(primary); }
  const more = iconButton({ label: t("machines.more"), glyph: "ellipsis", track: "machines.more", disabled: working, run: () => {
    const at = more.getBoundingClientRect();
    menu.openAt({ x: at.right - 180, y: at.bottom + 4, anchor: more, area: "settings" }, [
      { track: "machines.edit", label: t("machines.edit"), glyph: icon("pencil"), run: () => { editing = machine.id; redraw(); } },
      ...(machine.phase === "connected" ? [{ track: "machines.claude.check", label: t("machines.claude.check"), glyph: icon("check"), run: checkClaude }] : []),
      ...(agents && !agents.enabled ? [{ track: "machines.agents.enable", label: t("machines.agents.enable"), glyph: icon("plug"), run: () => enableAgents(false) }] : []),
      ...(agents?.enabled ? [
        { track: "machines.agents.rotate", label: t("machines.agents.rotate"), glyph: icon("rotate"), run: () => enableAgents(true) },
        { track: "machines.agents.disable", label: t("machines.agents.disable"), glyph: icon("x"), run: disableAgents },
      ] : []),
      ...(machine.phase !== "not_installed" ? [{ track: "machines.install", label: t("machines.update"), glyph: icon("server"), run: install }] : []),
      ...(!offline && machine.phase !== "not_installed" ? [{ track: "machines.reconnect", label: t("machines.reconnect"), glyph: icon("rotate"), run: reconnect }] : []),
      "sep",
      { track: "machines.remove", label: t("machines.remove"), glyph: icon("trash"), danger: true, run: () => run(machine.id, redraw, say, async () => { await invoke("machine_remove", { id: machine.id }); agentSetups.delete(machine.id); return null; }) },
    ]);
  } });
  act.append(more);

  const rows: HTMLElement[] = [node];
  if (signing.has(machine.id)) rows.push(claudeLogin(machine, signing.get(machine.id)!, say, redraw));
  // A token made for an earlier SSH destination must never be copied into a bot for this one.
  const shown = agentSetups.get(machine.id);
  if (shown && shown.ssh !== machine.ssh) agentSetups.delete(machine.id);
  else if (shown) rows.push(agentsPanel(machine, shown.setup, say, redraw));
  const note = (text: string, kind: "warning" | "error") => { const n = notice(text, kind); n.classList.add("machine-note"); rows.push(n); };
  if (machine.host && !machine.host.linger) note(t("machines.lingerOff"), "warning");
  if (machine.update_pending) note(t("machines.updatePending"), "warning");
  // Success shows in the row itself; only failures need a note.
  if (feedback?.error && [machine.id, `claude:${machine.id}`, `agents:${machine.id}`].includes(feedback.id)) note(feedback.text, "error");
  return rows;
}

function form(machine: MachineView | null, say: (text: string, isError?: boolean) => void, redraw: () => void): HTMLElement {
  const root = h("div", "machine-form");
  const name = input(machine?.name ?? "", false, "machines.name");
  name.placeholder = "vps";
  const ssh = input(machine?.ssh ?? "", false, "machines.ssh");
  ssh.placeholder = "vps  ·  me@100.64.0.1";
  ssh.spellcheck = false; ssh.autocapitalize = "off";
  const memory = input(machine?.limits.memory_max ?? "", false, "machines.memory");
  memory.placeholder = "4G";
  const cpu = input(machine?.limits.cpu_quota ?? "", false, "machines.cpu");
  cpu.placeholder = "200%";
  const limits = h("div", "machine-limits");
  limits.append(field(t("machines.memory"), memory), field(t("machines.cpu"), cpu));
  const id = machine?.id ?? "";
  const save = button(t("machines.save"), () => {
    if (!ssh.value.trim()) { ssh.focus(); return; }
    run(id || "new", redraw, say, async () => {
      if (machine && machine.ssh !== ssh.value.trim()) agentSetups.delete(machine.id);
      await invoke("machine_save", { machine: { id: machine?.id ?? null, name: name.value, ssh: ssh.value, limits: { memory_max: memory.value, cpu_quota: cpu.value } } });
      editing = null;
      return null;
    });
  }, "pri", "machines.save");
  const cancel = button(t("machines.cancel"), () => { editing = null; redraw(); }, "ghost", "machines.cancel");
  save.disabled = busy.has(id || "new");
  const actions = h("div", "machine-actions"); actions.append(save, cancel);
  root.append(
    field(t("machines.name"), name),
    field(t("machines.ssh"), ssh, t("machines.sshHint")),
    limits,
    h("span", "ui-hint", t("machines.limitsHint")),
    actions,
  );
  for (const control of [name, ssh, memory, cpu]) {
    control.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); save.click(); } });
  }
  queueMicrotask(() => (machine ? name : ssh).focus());
  return root;
}

/// Claude signs in on the machine itself, as in Orca: the machine runs `claude auth login`, the person signs in in this
/// computer's browser and pastes the code Claude shows back. The credential stays on the machine.
function claudeLogin(machine: MachineView, link: string, say: (text: string, isError?: boolean) => void, redraw: () => void): HTMLElement {
  const key = `claude:${machine.id}`;
  const working = busy.has(key);
  const panel = h("div", "machine-claude-login");
  const steps = h("p", "ui-hint", t("machines.claude.steps"));
  const open = button(t("machines.claude.openLink"), () => void invoke("open_external", { url: link }), "ghost", "machines.claude.open");
  open.insertAdjacentHTML("beforeend", `<span class="ic">${icon("external-link", 12)}</span>`);
  const code = input("", false, "machines.claude.code");
  code.placeholder = t("machines.claude.codePlaceholder");
  code.spellcheck = false; code.autocomplete = "off";
  const submit = button(t("machines.claude.submit"), () => {
    if (!code.value.trim()) { code.focus(); return; }
    run(key, redraw, say, async () => {
      const result = await invoke("machine_claude_code", { id: machine.id, code: code.value });
      signing.delete(machine.id);
      return result.email ? t("machines.claude.signedInAs", { email: result.email }) : t("machines.claude.signedIn");
    });
  }, "pri", "machines.claude.submit");
  const cancel = button(t("machines.cancel"), () => {
    signing.delete(machine.id);
    void invoke("machine_claude_login_cancel", { id: machine.id }).catch(() => {});
    redraw();
  }, "ghost", "machines.claude.cancel");
  code.addEventListener("keydown", event => { if (event.key === "Enter") { event.preventDefault(); submit.click(); } });
  for (const control of [open, submit, cancel, code]) control.disabled = working;
  const entry = h("div", "machine-claude-entry"); entry.append(field(t("machines.claude.code"), code), submit, cancel);
  panel.append(steps, open, entry);
  queueMicrotask(() => code.focus());
  return panel;
}

/// The configuration a bot platform needs to drive this machine through its agent socket (ADR 0103): a read-only mount
/// of the agent folder into the bot's container and an MCP server entry, each with a copy button. The token inside is
/// shown only here, until the person closes the panel.
function agentsPanel(machine: MachineView, setup: AgentsSetup, say: (text: string, isError?: boolean) => void, redraw: () => void): HTMLElement {
  const panel = h("div", "machine-agents");
  /** Shows `shown` and copies `value`: the configuration's token stays off the screen, so a screenshot never carries it. */
  const copyable = (label: string, value: string, shown: string, multiline: boolean, track: string) => {
    const control = input(shown, multiline, track);
    control.readOnly = true; control.spellcheck = false;
    if (control instanceof HTMLTextAreaElement) control.rows = Math.min(16, shown.split("\n").length);
    const copy = button(t("machines.agents.copy"), () => {
      void navigator.clipboard.writeText(value)
        .then(() => say(t("machines.agents.copied", { what: label })))
        .catch(error => say(fromBack(error), true));
    }, "outline", `${track}.copy`);
    const row = h("div", "machine-agents-value"); row.append(field(label, control), copy);
    return row;
  };
  const done = button(t("machines.agents.done"), () => { agentSetups.delete(machine.id); redraw(); }, "ghost", "machines.agents.done");
  panel.append(
    h("p", "ui-hint", t("machines.agents.steps")),
    copyable(t("machines.agents.mount"), setup.mount, setup.mount, false, "machines.agents.mount"),
    copyable(t("machines.agents.config"), setup.config, setup.preview, true, "machines.agents.config"),
    h("p", "ui-hint", t("machines.agents.uid", { uid: String(setup.client_uid), local: String(setup.local_uid) })),
    notice(t("machines.agents.secret"), "warning"),
  );
  if (setup.moved_from) panel.append(notice(t("machines.agents.moved", { path: setup.moved_from }), "warning"));
  if (setup.opened.length) panel.append(notice(t("machines.agents.opened", { folders: setup.opened.join(", ") }), "warning"));
  panel.append(done);
  return panel;
}
