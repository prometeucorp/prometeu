import { listen } from "@tauri-apps/api/event";
import { fromBack, t } from "./i18n";
import { icon } from "./icons";
import { iconButton } from "./components/icon-button";
import { invoke } from "./ipc";
import * as menu from "./menu";
import { button, confirmDialog, field, input, notice } from "./ui";
import type { Project, ProviderId } from "./types";
import { h, template } from "./util";
import { showMachineClients } from "./machine-clients";
import { showAgentConnection, connectionTestMessage } from "./machine-supervision";
import { showMachineProcesses } from "./machine-processes";

/// Remote machines (ADR 0101): computers reached over SSH whose `prometeu-host` daemon runs workspaces for this app.
/// Rust owns the connections and publishes their state on the `machines` event; this module mirrors it for Settings,
/// the launcher and the badges that show where each workspace runs.

export type MachinePhase = "offline" | "connecting" | "connected" | "not_installed" | "replaced" | "failed";
export type Limits = { memory_max: string; cpu_quota: string };
export type HostInfo = { home: string; user: string; os: string; arch: string; version: string; linger: boolean; scopes: boolean; executable: string | null;
  /** The host serves several computers at once (ADR 0104); absent on older hosts. */
  clients?: boolean };
export type ClaudeStatus = { installed: boolean; loggedIn: boolean; email: string | null; method: string | null };
/// Codex signs in on the machine itself (ADR 0105); only exit codes and the version line are read there.
export type CodexStatus = { installed: boolean; version: string | null; supported: boolean; loggedIn: boolean; minimum: string };
export type AgentsStatus = { enabled: boolean; socket: string; serving: boolean; error: string | null };
/** Shown once after enabling agents or rotating their token: the local MCP configuration with the token. */
export type AgentsSetup = { config: string; preview: string; moved_from: string | null };
export type MachineView = { id: string; name: string; ssh: string; limits: Limits; phase: MachinePhase; detail: string | null; host: HostInfo | null; update_pending: boolean; claude?: ClaudeStatus | null; codex?: CodexStatus | null; agents?: AgentsStatus | null };
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

/// Whether an agent can start on a machine, as the launcher shows it: an empty problem when it can, and the sign-in the
/// agent uses there. Each agent signs in on the machine itself, so only the chosen one has to be ready. An unchecked
/// machine reports no problem; the launch itself checks again.
export type MachineReadiness = { problem: string; account: string };
export function readinessOf(machine: MachineView | undefined, agent: ProviderId): MachineReadiness {
  const name = machine?.name ?? "";
  const claude = machine?.claude;
  const codex = machine?.codex;
  const byAgent: Partial<Record<ProviderId, () => MachineReadiness>> = {
    claude: () => ({
      problem: !claude ? "" : !claude.installed ? t("machines.claude.missing") : !claude.loggedIn ? t("launcher.remoteSignIn", { name }) : "",
      account: claude?.loggedIn ? claude.email ?? t("machines.claude.signedIn") : t("machines.claude.signedOut"),
    }),
    codex: () => ({
      problem: !codex ? ""
        : !codex.installed ? t("err.remote.codexMissing", { name })
        : !codex.supported ? t("err.remote.codexVersion", { name, version: codex.version ?? "", minimum: codex.minimum })
        : !codex.loggedIn ? t("err.remote.codexSignIn", { name }) : "",
      account: t(codex?.loggedIn ? "machines.codex.account" : "machines.codex.accountOut"),
    }),
  };
  return byAgent[agent]?.() ?? { problem: t("err.remote.provider"), account: "" };
}
export const readiness = (host: string, agent: ProviderId) => readinessOf(machineOf(host), agent);

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
/// It says only where the work runs; who created the workspace is a separate chip (`creatorBadge`).
export function badge(host: string | undefined, compact = false): HTMLElement | null {
  if (!host) return null;
  const machine = machineOf(host);
  const state = machine?.phase ?? "offline";
  const chip = template("span", `machine-badge ${state === "connected" ? "on" : "off"}${compact ? " compact" : ""}`, `${icon("server", 12)}<span></span>`);
  chip.querySelector("span")!.textContent = compact ? "" : machineName(host);
  const where = state === "connected"
    ? t("remote.badge", { name: machineName(host) })
    : t("remote.offline", { name: machineName(host) });
  chip.title = where;
  chip.setAttribute("aria-label", where);
  return chip;
}

/// A chip saying an agent created the workspace, wherever it runs: authorship is independent of the machine
/// (`badge`), so a workspace an agent created on this computer shows it too. `creator` is the name the agent's MCP
/// configuration gives, possibly empty; absent means the person created the workspace and shows nothing.
export function creatorBadge(creator: string | undefined, compact = false): HTMLElement | null {
  if (creator === undefined) return null;
  const chip = template("span", `creator-badge${compact ? " compact" : ""}`, `${icon("sparkles", 12)}<span></span>`);
  const label = t("workspace.createdByAgent");
  chip.querySelector("span")!.textContent = compact ? "" : label;
  const title = creator ? t("workspace.createdByNamedAgent", { name: creator }) : label;
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
const eventTests = new Map<string, { ssh: string; message: string }>();
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

export function codexLine(codex: CodexStatus): string {
  if (!codex.installed) return t("machines.codex.missing");
  if (!codex.supported) return t("machines.codex.old", { version: codex.version ?? "", minimum: codex.minimum });
  return codex.loggedIn ? t("machines.codex.signedIn") : t("machines.codex.signedOut");
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
    // Codex is optional on a machine: a missing install is information, a broken one a warning.
    if (machine.codex) line(codexLine(machine.codex), machine.codex.installed && !(machine.codex.supported && machine.codex.loggedIn) ? "warn" : "");
  }

  const eventTest = eventTests.get(machine.id);
  if (eventTest?.ssh === machine.ssh) line(eventTest.message);

  const working = busy.has(`events:${machine.id}`) || busy.has(machine.id) || busy.has(`claude:${machine.id}`) || busy.has(`agents:${machine.id}`);
  const install = () => run(machine.id, redraw, say, async () => { await invoke("machine_install", { id: machine.id }); return t("machines.installed"); });
  const reconnect = () => run(machine.id, redraw, say, async () => { await invoke("machine_connect", { id: machine.id }); return null; });
  const signIn = () => run(`claude:${machine.id}`, redraw, say, async () => {
    const url = await invoke("machine_claude_login", { id: machine.id });
    signing.set(machine.id, url);
    void invoke("open_external", { url }).catch(() => {});
    return null;
  });
  const checkClaude = () => run(`claude:${machine.id}`, redraw, say, async () => { await invoke("machine_claude_status", { id: machine.id }); return null; });
  const testEvents = () => run(`events:${machine.id}`, redraw, say, async () => {
    eventTests.set(machine.id, { ssh: machine.ssh, message: t("machines.events.testing") }); redraw();
    try {
      const result = await invoke("machine_supervision_test", { id: machine.id });
      eventTests.set(machine.id, { ssh: machine.ssh, message: connectionTestMessage(result) });
    } catch (error) {
      eventTests.set(machine.id, { ssh: machine.ssh, message: t("machines.events.failed") });
      throw error;
    }
    return null;
  });
  const connectAgents = (existing = false) => showAgentConnection(machine.id, machine.name, existing, (setup, events) => {
    if (machineOf(machine.id)?.ssh !== machine.ssh) return;
    if (setup) agentSetups.set(machine.id, { setup, ssh: machine.ssh });
    redraw();
    if (events) testEvents();
  });
  const enableAgents = (rotate: boolean) => run(`agents:${machine.id}`, redraw, say, async () => {
    if (rotate && !await confirmDialog({ track: "machines.agents.rotate", area: "settings", title: t("machines.agents.rotate"),
      message: t("machines.agents.rotateConfirm"), accept: t("machines.agents.rotate"), cancel: t("machines.cancel") })) return null;
    // A configuration is only ever shown for the destination it was made on.
    const ssh = machine.ssh;
    const result = await invoke("machine_agents_enable", { id: machine.id, rotate });
    if (machineOf(machine.id)?.ssh === ssh) agentSetups.set(machine.id, { setup: result, ssh });
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
    : agents && !agents.enabled ? button(t("machines.agents.enable"), () => connectAgents(), "outline", "machines.agents.enable")
    : null;
  if (primary) { primary.disabled = working; act.append(primary); }
  const more = iconButton({ label: t("machines.more"), glyph: "ellipsis", track: "machines.more", disabled: working, run: () => {
    const at = more.getBoundingClientRect();
    menu.openAt({ x: at.right - 180, y: at.bottom + 4, anchor: more, area: "settings" }, [
      { track: "machines.edit", label: t("machines.edit"), glyph: icon("pencil"), run: () => { editing = machine.id; redraw(); } },
      ...(machine.phase === "connected" ? [{ track: "machines.claude.check", label: t("machines.claude.check"), glyph: icon("check"), run: checkClaude }] : []),
      ...(machine.phase === "connected" && machine.host?.clients ? [{ track: "machines.clients", label: t("machines.clients"), glyph: icon("signal"), run: () => showMachineClients(machine.id, machine.name) }] : []),
      ...(agents && !agents.enabled ? [{ track: "machines.agents.enable", label: t("machines.agents.enable"), glyph: icon("plug"), run: () => connectAgents() }] : []),
      ...(agents?.enabled ? [
        { track: "machines.events.enable", label: t("machines.events.enable"), glyph: icon("plug"), run: () => connectAgents(true) },
        { track: "machines.events.test", label: t("machines.events.test"), glyph: icon("check"), run: testEvents },
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
  if (machine.phase === "connected") {
    const processes = button(t("machines.processes"), () => showMachineProcesses(machine.id, machine.name), "outline", "machines.processes");
    processes.dataset.focus = `machine-processes:${machine.id}`;
    act.prepend(processes);
  }

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
  if (feedback?.error && [machine.id, `claude:${machine.id}`, `agents:${machine.id}`, `events:${machine.id}`].includes(feedback.id)) note(feedback.text, "error");
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

/// Local stdio MCP configuration for any client (ADR 0103). The token is masked on screen
/// and included only when copying, until the person closes the panel.
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
    copyable(t("machines.agents.config"), setup.config, setup.preview, true, "machines.agents.config"),
    notice(t("machines.agents.secret"), "warning"),
  );
  if (setup.moved_from) panel.append(notice(t("machines.agents.moved", { path: setup.moved_from }), "warning"));
  panel.append(done);
  return panel;
}
