import { invoke } from "./ipc";
import * as dock from "./dock";
import { icon, wave } from "./icons";
import { fromBack, t } from "./i18n";
import { keys } from "./platform";
import * as menu from "./menu";
import { dockLabel, isTerm, termKind, termNumber, type DockKind, type DockState, type Scripts } from "./types";
import { $ } from "./util";

/// Coordinate repository scripts in the side panel and shell tabs in the center. dock.ts owns terminal rendering; this module owns declared scripts, observed process state, and actions. workspace.ts renders center tabs from tabs().

const NO_SCRIPTS: Scripts = { file: null, inherited: false, setup: null, runs: [], archive: null, copy: [], port: null };

/// Refresh declared scripts and live dock state on workspace opening and process changes, without polling.
let info: { scripts: Scripts; docks: DockState[] } = { scripts: NO_SCRIPTS, docks: [] };
/// The selected scripts.run entry; absent selection uses the repository default.
let runName: string | undefined;
/// The script selected in the side panel.
let pane: DockKind | null = null;
/// The selected center shell, or null when another center view is active.
let shell: DockKind | null = null;
let open = true;
/// Reserve pending shell numbers before backend confirmation so rapid creation cannot request the same slot twice.
const asked = new Set<DockKind>();

type Ctx = {
  workspace: () => string | null;
  say: (m: string, err?: boolean) => void;
  /// Open a worktree file in the center viewer.
  openFile: (path: string) => Promise<void>;
  /// Ask for script configuration; the host reuses delegated conversations.
  newTab: (prompt: string) => Promise<void>;
  /// Open the center browser at the Run port.
  openBrowser: () => Promise<void>;
  /// Activate the center terminal view.
  enter: () => void;
  /// Restore the conversation when the final shell closes.
  exit: () => void;
  /// Redraw the center tab strip containing shell tabs.
  drawTabs: () => void;
};
let ctx: Ctx;

const isUp = (kind: DockKind) => info.docks.some((d) => d.kind === kind && d.alive);
/// Exited scripts retain their output and exit-code marker.
const hasLog = (kind: DockKind) => info.docks.some((d) => d.kind === kind);
/// Setup includes inherited-file copying, so workspaces receiving only .env still have setup output.
const declares = (kind: DockKind) =>
  kind === "setup"
    ? !!info.scripts.setup || info.scripts.copy.length > 0
    : info.scripts.runs.length > 0;

/// Restore backend-reported shell tabs in order. Unlike fixed script tabs, shells are explicitly created and removed.
const terminals = (): DockKind[] => shellTabs(info.docks, asked);

/// Shell tabs in number order, pending ones included until dock_state confirms them. A shell that exited while its owner was off screen can still be reported with `alive: false`, but gets no tab: selecting it would quietly start a new shell under its key.
export function shellTabs(docks: DockState[], pending: Iterable<DockKind>): DockKind[] {
  const live = docks.filter((d) => d.alive && isTerm(d.kind)).map((d) => d.kind);
  return [...new Set([...live, ...pending])].sort((a, b) => termNumber(a) - termNumber(b));
}

/// Only repository scripts appear in the side-panel tab strip.
const order = (): DockKind[] => ["setup", "run"];

/// Reuse the lowest available shell number instead of accumulating gaps.
function nextTerm(): DockKind {
  const taken = new Set(terminals().map(termNumber));
  let n = 1;
  while (taken.has(n)) n++;
  return termKind(n);
}

export function init(context: Ctx) {
  ctx = context;

  $("run-go").addEventListener("click", toggleRun);
  $("run-pick").addEventListener("click", (e) => {
    const at = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const chosen = runName ?? info.scripts.runs[0]?.name;
    menu.openAt({ area: "dockbar", x: at.left - 40, y: at.bottom + 4 }, [
      ...info.scripts.runs.map((r) => ({
        label: r.name, track: "dock.script.select",
        hint: r.command.length > 36 ? `${r.command.slice(0, 35)}…` : r.command,
        checked: chosen === r.name,
        run: async () => {
          runName = r.name;
          const id = ctx.workspace();
          if (id && isUp("run")) await dock.kill(id, "run");
          setDock("run", true);
        },
      })),
      "sep",
      {
        // Editing inherited settings first copies them into the worktree; label this action explicitly.
        label: t(info.scripts.inherited ? "dock.settings.copy" : "dock.settings.open"), track: "dock.settings",
        glyph: icon("file", 14),
        run: writeScriptsFile,
      },
    ]);
  });
  // Click opens the embedded browser; Option-click opens the system browser.
  $("run-open").addEventListener("click", (e) => {
    const id = ctx.workspace();
    if (!id) return;
    if (e.altKey) invoke("open_run", { id }).catch((e) => ctx.say(fromBack(e), true));
    else void ctx.openBrowser();
  });
  $("dock-again").addEventListener("click", () => setDock("setup", true));
  $("dock-toggle").addEventListener("click", () => {
    open = !open;
    draw();
  });
  draw();
}

/// Reset presentation on workspace changes without stopping old processes. Project-only mode disables Setup/Run and keeps free shells available.
export function reset(panel = true) {
  pane = null;
  shell = null;
  asked.clear();
  runName = undefined;
  info = { scripts: NO_SCRIPTS, docks: [] };
  dock.detach();
  draw();
  // Select Setup on workspace entry to show preparation output or failure without starting a process.
  void refresh().then(() => {
    if (panel && pane === null) void setDock("setup");
  });
}

/// Read declared scripts and live docks from the backend.
export async function refresh() {
  const id = ctx.workspace();
  if (!id) return;
  const [scripts, docks] = await Promise.all([
    invoke("workspace_scripts", { id }),
    invoke("dock_state", { id }),
  ]);
  // Discard responses if the selected workspace changed while waiting.
  if (ctx.workspace() !== id) return;
  info = { scripts, docks };
  draw();
}

/// Handle script or shell exit for the active workspace.
export function closed(key: string) {
  const id = ctx.workspace();
  if (!id || !key.startsWith(`${id}:`)) return;
  const kind = key.slice(id.length + 1) as DockKind;
  // Exited shells disappear; exited scripts retain their logs in fixed tabs.
  if (isTerm(kind)) void closeTerm(kind);
  else void refresh();
}

/// Only start explicitly launches scripts. Selecting a tab attaches to a live process or displays retained output.
async function setDock(next: DockKind | null, start = false) {
  const id = ctx.workspace();
  if (!id) return;
  pane = next;
  open = true;
  draw();
  if (!next) return dock.detach();
  try {
    if (start || isUp(next)) {
      await dock.open(id, next, next === "run" ? runName : undefined);
      dock.focus("scripts");
    } else if (hasLog(next)) {
      // Show an exited script's output without restarting it.
      await dock.show(id, next);
    } else {
      dock.detach();
    }
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
  await refresh();
}

/// Selecting an existing shell reuses its PTY and retained output.
async function setShell(kind: DockKind) {
  const id = ctx.workspace();
  if (!id) return;
  shell = kind;
  asked.add(kind);
  ctx.enter();
  ctx.drawTabs();
  try {
    await dock.open(id, kind);
    dock.focus("shell");
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
  await refresh();
  // After backend confirmation or failure, release the temporary shell reservation.
  asked.delete(kind);
}

/// Expose center shell-tab presentation data.
export type DockTab = { kind: DockKind; label: string; on: boolean };

export const tabs = (): DockTab[] =>
  terminals().map((kind) => ({ kind, label: dockLabel(kind), on: shell === kind }));

/// Identify the active shell so conversation tabs are not incorrectly highlighted.
export const front = () => shell;

export const select = (kind: DockKind) => void setShell(kind);
export const closeTab = (kind: DockKind) => void closeTerm(kind);
export const newTerm = () => void setShell(nextTerm());

/// Leaving the center shell view preserves its process, tab, and retained output.
export function leave() {
  if (shell === null) return;
  shell = null;
  ctx.drawTabs();
}

/// Closing a shell kills its process and removes the tab; fixed script tabs instead retain their logs.
async function closeTerm(kind: DockKind) {
  const id = ctx.workspace();
  if (!id) return;
  const bar = terminals();
  const at = bar.indexOf(kind);
  const wasOn = shell === kind;
  // Remove the shell from both confirmed and pending lists before IPC so intermediate redraws cannot restore it.
  info.docks = info.docks.filter((d) => d.kind !== kind);
  asked.delete(kind);
  if (wasOn) shell = null;
  await dock.kill(id, kind);
  // Select the right neighbor, then left neighbor, or return to the conversation when no shells remain.
  if (wasOn) {
    const next = bar[at + 1] ?? bar[at - 1];
    if (next) await setShell(next);
    else ctx.exit();
  }
  ctx.drawTabs();
  await refresh();
}

/// Handle Command-W only while a terminal owns focus; otherwise workspace.ts closes the center tab.
export function closeFocused(): boolean {
  if (!shell || !$("termview").contains(document.activeElement)) return false;
  void closeTerm(shell);
  return true;
}

/// Stopping a fixed script preserves its tab and exposes the restart action.
async function killPane(kind: DockKind) {
  const id = ctx.workspace();
  if (!id) return;
  await dock.kill(id, kind);
  await refresh();
}

export function draw() {
  $("dock").classList.toggle("closed", !open);
  $("dock-toggle").innerHTML = icon(open ? "chevron-down" : "chevron-right");
  $("dock-toggle").dataset.tTitle = open ? "dock.collapse" : "dock.expand";
  $("dock-toggle").dataset.track = open ? "dock.collapse" : "dock.expand";
  $("dock-toggle").title = t(open ? "dock.collapse" : "dock.expand");
  drawStrip();
  // Backend dock changes also update center shell tabs.
  ctx.drawTabs();

  // Run and Command-R remain global to the workspace regardless of the selected script tab.
  const up = isUp("run");
  $("runsplit").hidden = !info.scripts.runs.length;
  $("run-pick").hidden = info.scripts.runs.length < 2;
  $("run-go").innerHTML = `${icon(up ? "square" : "play", 13)}<span></span><kbd>${keys("⌘R")}</kbd>`;
  $("run-go").querySelector("span")!.textContent = t(up ? "dock.stop" : "dock.run");
  $("run-go").dataset.track = up ? "dock.run.stop" : "dock.run.start";

  // Offer browser opening only with a running script and reserved local port.
  const goOpen = $("run-open");
  const port = info.scripts.port;
  goOpen.hidden = !up || !port;
  if (port && !goOpen.hidden) {
    goOpen.innerHTML = `${icon("globe", 13)}<span></span><span class="port">:${port}</span>`;
    goOpen.querySelector("span")!.textContent = t("dock.open");
    goOpen.title = t("dock.open.title", { port });
  }

  const filled = pane !== null && (isUp(pane) || hasLog(pane));
  $("dockwrap").hidden = !filled;
  $("dockempty").hidden = filled;
  // Exited Setup retains its output and uses a dedicated rerun button.
  $("dock-again").hidden = !(pane === "setup" && hasLog("setup") && !isUp("setup"));
  if (!filled) drawEmpty();
}

/// Rebuild fixed script tabs when live-state indicators or close actions change.
function drawStrip() {
  const strip = $("dockstrip");
  strip.replaceChildren();

  for (const kind of order()) {
    const b = document.createElement("button");
    b.className = "docktab" + (pane === kind ? " on" : "");
    b.dataset.track = open && pane === kind
      ? kind === "setup" ? "dock.setup.hide" : "dock.run.hide"
      : kind === "setup" ? "dock.setup.open" : "dock.run.open";
    // Animate the live indicator even when a different script tab is selected.
    if (isUp(kind)) b.insertAdjacentHTML("beforeend", wave());
    const name = document.createElement("span");
    name.textContent = dockLabel(kind);
    b.append(name);
    // Clicking the selected script tab hides output without stopping its process.
    b.addEventListener("click", () => setDock(open && pane === kind ? null : kind));

    // Stopping Setup leaves its tab available for retained output.
    if (kind === "setup" && isUp(kind)) {
      const x = document.createElement("span");
      x.className = "tabx ico sm";
      x.innerHTML = icon("x", 12);
      x.title = t("dock.killSetup");
      x.dataset.track = "dock.killSetup";
      x.addEventListener("click", (e) => {
        e.stopPropagation();
        void killPane(kind);
      });
      b.append(x);
    }
    strip.append(b);
  }
}

/// Empty-state actions distinguish missing scripts from stopped processes, emphasizing one primary action.
function drawEmpty() {
  const row = $("empty-row");
  row.replaceChildren();
  const glyph = $("empty-glyph");
  glyph.hidden = true;
  glyph.className = "glyph";
  $("dockempty").classList.toggle("idle", pane === null);

  const button = (label: string, cls: string, run: () => void, track: string, key?: string) => {
    const b = document.createElement("button");
    b.className = cls;
    b.dataset.track = track;
    b.innerHTML = `<span></span>${key ? `<kbd>${key}</kbd>` : ""}`;
    b.firstElementChild!.textContent = label;
    b.addEventListener("click", run);
    row.append(b);
  };
  const title = (text: string) => {
    $("empty-title").textContent = text;
    $("empty-title").hidden = !text;
  };

  // When output is hidden, avoid repeating the tab choices already visible above it.
  if (pane === null) {
    title("");
    $("empty-sub").textContent = t("dock.idle");
    return;
  }

  if (!declares(pane)) {
    title(t(pane === "setup" ? "dock.noSetup.title" : "dock.noRun.title"));
    $("empty-sub").textContent = t(pane === "setup" ? "dock.noSetup.body" : "dock.noRun.body");
    button(t("dock.ask"), "outline", askForScripts, "dock.ask");
    button(t("dock.write"), "ghost", writeScriptsFile, "dock.write");
    return;
  }

  // A declared but stopped script needs only an explicit start.
  const setup = pane === "setup";
  const port = info.scripts.port;
  glyph.hidden = false;
  glyph.className = setup ? "glyph" : "glyph solid";
  glyph.innerHTML = icon(setup ? "rotate" : "play", 56);
  title(t(setup ? "dock.setup.idle.title" : "dock.run.idle.title"));
  // Command-R belongs only to Run, never to the Setup button.
  button(
    t(setup ? "dock.setup.start" : "dock.run.start"),
    "outline",
    () => setDock(pane, true),
    setup ? "dock.setup.start" : "dock.run.start",
    setup ? undefined : keys("⌘R"),
  );
  $("empty-sub").textContent = setup
    ? t("dock.setup.idle.body")
    : port
      ? t("dock.run.idle.port", { port })
      : t("dock.run.idle.body");
}

/// Ask the host to inspect the repository and write settings.toml in an eligible conversation.
async function askForScripts() {
  const id = ctx.workspace();
  if (!id) return;
  try {
    await ctx.newTab(await invoke("scripts_prompt", { id }));
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
}

/// Create commented example settings and open them in the viewer.
async function writeScriptsFile() {
  const id = ctx.workspace();
  if (!id) return;
  try {
    await ctx.openFile(await invoke("create_scripts_file", { id }));
    await refresh();
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
}

/// Command-R opens the script-configuration invitation when no Run command exists.
export async function toggleRun() {
  const id = ctx.workspace();
  if (!id) return;
  if (!info.scripts.runs.length) return setDock("run");
  if (!isUp("run")) return setDock("run", true);
  await dock.kill(id, "run");
  await refresh();
}
