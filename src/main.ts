import * as actions from "./actions";
import * as background from "./background";
import * as cloud from "./cloud";
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { openProjects } from "./projects";
import * as alert from "./alert";
import * as notifications from "./notifications";
import { installed, loadAgents } from "./agents";
import * as appmenu from "./appmenu";
import type { Info } from "./chat";
import * as archived from "./archived";
import * as sidebar from "./sidebar";
import { openCleanup } from "./cleanup";
import { openInbox } from "./inbox";
import * as desk from "./desk";
import * as dock from "./dock";
import * as dockbar from "./dockbar";
import { icon } from "./icons";
import * as links from "./links";
import { initCodeCopy } from "./markdown";
import * as feedback from "./feedback";
import { current, fromBack, paint, t } from "./i18n";
import * as issues from "./issues";
import * as mcp from "./mcp";
import * as plugins from "./plugins";
import { PrScanPolicy } from "./pr-refresh";
import { fileDropTarget as launcherDropTarget, openLauncher, type Draft, type Open } from "./launcher";
import * as menu from "./menu";
import * as news from "./news";
import * as rename from "./rename";
import * as session from "./session";
import * as settings from "./settings";
import * as statusbar from "./statusbar";
import * as team from "./team";
import * as trust from "./trust";
import "./style.css";
import type { Board, Issue, Project, Tab, Workspace } from "./types";
import * as update from "./update";
import { $ } from "./util";
import { mac } from "./platform";
import * as viewer from "./viewer";
import * as ws from "./workspace";

// Use the backend mock when running outside Tauri.
if (!("__TAURI_INTERNALS__" in window)) await import("./mock");
await background.start().catch(() => {});
// The resource sampler wakes natively on context changes; quota refresh on return stays explicit.
background.subscribe((context) => {
  if (background.foreground(context)) void invoke("usage_refresh", {}).catch(() => {});
});

let state: Board = { stages: [], projects: [], workspaces: [] };

/// Render local board state plus frontend-only remote shares. Backend commands use the local state alone.
const view = (): Board => {
  const remotes = team.remotes();
  return remotes.length ? { ...state, workspaces: [...state.workspaces, ...remotes] } : state;
};

/// Clear transient header notices automatically. Messages ending in an ellipsis remain until their operation clears them.
let fade = 0;
let messageVersion = 0;
function say(text: string, isError = false) {
  messageVersion++;
  $("msg").textContent = text;
  $("msg").classList.toggle("err", isError);
  clearTimeout(fade);
  if (text && !text.endsWith("…")) fade = setTimeout(() => say(""), 6000);
}

/* Navigation. */

const hooks: sidebar.Hooks = {
  open: (w, tab) => openWorkspace(w, true, tab),
  activeTab: session.currentSession,
  setStage: ws.setStage,
  drop: (id) => {
    if (ws.id() === id) showDesk();
    invoke("remove_workspace", { id });
  },
  rename: ws.renameWorkspace,
  // Return to the desk when archiving the active workspace.
  archive: (id, archived) => {
    const target = state.workspaces.find((workspace) => workspace.id === id);
    if (archived && ws.id() === id) showDesk();
    invoke("archive_workspace", { id, archived })
      .then(() => {
        if (archived && target && !target.cleaned && target.worktree !== target.repo) openCleanup(say, id);
      })
      .catch((e) => say(fromBack(e), true));
  },
  finish: (id) => ws.finish(id),
  cleanup: () => openCleanup(say),
  inbox: () =>
    openInbox((workspace, note, tab) => {
      const target = view().workspaces.find((w) => w.id === workspace);
      if (!target) return say(t("err.team.noShare"), true);
      if (tab && !target.remote) invoke("focus_tab", { workspace: target.id, tab });
      void openWorkspace(tab ? { ...target, active: tab } : target).then(() => ws.showNote(note));
    }),
  pin: (id, pinned) => invoke("pin_workspace", { id, pinned }),
  unread: (id, unread) => invoke("set_unread", { id, unread }),
  reveal: (id) => invoke("reveal_path", { id, rel: "" }).catch((e) => say(fromBack(e), true)),
  copyPath: async (w) => {
    try {
      await navigator.clipboard.writeText(w.worktree);
      say(t("say.copied", { path: w.worktree }));
    } catch {
      say(t("say.copyPathFailed"), true);
    }
  },
  toDesk: () => showDesk(),
  toIssues: () => showIssues(),
  toArchived: () => showArchived(),
  issues: () => issues.count(),
  addProject: () => openProjects(say),
  removeProject: (id) => invoke("remove_project", { id }),
  projectTools: (id) => void trust.open(id, say),
  openProject: (id) => {
    const project = state.projects.find((p) => p.id === id);
    if (project) showProject(project);
  },
  newWorkspace: (projectId) => launch(projectId),
};

/// Remember board redraws deferred by an open menu so its latest selection appears even if no later backend event arrives.
let missed = false;

function draw() {
  ws.observeTurns();
  // Avoid replacing rows containing rename inputs or open menu anchors during agent-driven updates.
  if (rename.editing() || menu.isOpen()) {
    missed = true;
    return;
  }
  missed = false;
  sidebar.render(view(), hooks);
  archived.draw();
  desk.draw();
  if (ws.id()) ws.draw();
  alert.looked();
}

menu.onClose(() => missed && draw());

/* Navigation history contains pages and workspaces with noncolliding IDs. */
const SETTINGS = "@configurações";
const ISSUES = sidebar.ISSUES;
const ARCHIVED = sidebar.ARCHIVED;
const DESK = sidebar.DESK;
const pages = new Set([SETTINGS, ISSUES, ARCHIVED, DESK]);
const hist: string[] = [];
let at = -1;
function visit(to: string) {
  if (hist[at] === to) return;
  hist.splice(at + 1);
  hist.push(to);
  at = hist.length - 1;
  drawNav();
}
function drawNav() {
  ($("back") as HTMLButtonElement).disabled = at <= 0;
  ($("fwd") as HTMLButtonElement).disabled = at >= hist.length - 1;
}
function travel(dir: -1 | 1) {
  const next = at + dir;
  if (next < 0 || next >= hist.length) return;
  at = next;
  if (hist[at] === SETTINGS) {
    showSettings(false);
  } else if (hist[at] === ISSUES) {
    showIssues(false);
  } else if (hist[at] === ARCHIVED) {
    showArchived(false);
  } else if (hist[at] === DESK) {
    showDesk(false);
  } else {
    const target = view().workspaces.find((w) => w.id === hist[at]);
    const project = state.projects.find((p) => p.id === hist[at]);
    if (target) openWorkspace(target, false);
    else if (project) showProject(project, false);
    else showDesk(false);
  }
  drawNav();
}
$("back").addEventListener("click", () => travel(-1));
$("fwd").addEventListener("click", () => travel(1));

/// Show one non-workspace page at a time.
function showOnly(view: "settingsView" | "issuesView" | "archivedView" | "deskView" | null) {
  $("deskView").hidden = view !== "deskView";
  $("settingsView").hidden = view !== "settingsView";
  $("issuesView").hidden = view !== "issuesView";
  $("archivedView").hidden = view !== "archivedView";
  if (view !== "issuesView") issues.hide();
  if (view !== "archivedView") archived.hide();
  if (view !== "deskView") desk.hide();
}

/// Non-workspace breadcrumbs use the page name; ws.draw owns workspace breadcrumbs.
const crumbLabel = (text: string) =>
  Object.assign(document.createElement("span"), { textContent: text });

async function openWorkspace(target: Workspace, push = true, tab?: string) {
  if (push) visit(target.id);
  showOnly(null);
  await ws.open(target, tab);
  alert.looked();
}

/// The desk is the start page, showing every active conversation.
function showDesk(push = true) {
  if (push) visit(DESK);
  ws.leave();
  sidebar.setOpen(DESK);
  showOnly("deskView");
  $("crumb").replaceChildren(crumbLabel(t("crumb.desk")));
  desk.show();
  draw();
}

/// Open clone files directly without creating a branch, worktree, or conversation.
function showProject(project: Project, push = true) {
  if (push) visit(project.id);
  showOnly(null);
  ws.openProject(project);
  sidebar.setOpen(project.id);
  draw();
}

/// Assigned Linear issues are an entry point for work.
function showIssues(push = true) {
  if (push) visit(ISSUES);
  ws.leave();
  sidebar.setOpen(ISSUES);
  showOnly("issuesView");
  $("crumb").replaceChildren(crumbLabel(t("crumb.issues")));
  issues.show();
  draw();
}

/// Browse archived workspaces and reclaim their disk space.
function showArchived(push = true) {
  if (push) visit(ARCHIVED);
  ws.leave();
  sidebar.setOpen(ARCHIVED);
  showOnly("archivedView");
  $("crumb").replaceChildren(crumbLabel(t("crumb.archived")));
  archived.show();
  draw();
}

/// Settings does not select a sidebar entry.
function showSettings(push = true) {
  if (push) visit(SETTINGS);
  ws.leave();
  sidebar.setOpen(SETTINGS);
  showOnly("settingsView");
  $("crumb").replaceChildren(crumbLabel(t("crumb.settings")));
  settings.draw();
  draw();
}
$("settings").addEventListener("click", () => showSettings());

/* Backend events. */

listen<Board>("board", ({ payload }) => {
  state = payload;
  actions.update(state);
  team.boardChanged(state);
  alert.boardChanged(state);
  refresh();
});

/// Reconcile local or remote workspace changes, remove vanished history entries, and redraw.
function refresh() {
  // Remove vanished workspaces/projects from history and merge adjacent duplicates. Projects also own file-view state.
  const alive = new Set([...view().workspaces.map((w) => w.id), ...state.projects.map((p) => p.id)]);
  for (let i = hist.length - 1; i >= 0; i--) {
    const id = hist[i];
    if ((!pages.has(id) && !alive.has(id)) || (i > 0 && id === hist[i - 1])) {
      hist.splice(i, 1);
      if (i <= at) at--;
    }
  }
  ws.forget(alive);
  session.forget(new Set(view().workspaces.flatMap((w) => w.tabs.map((t) => t.id))));
  // Only local agents keep this Mac awake; shared remote agents run on their owners' Macs.
  statusbar.boardChanged(state);
  drawNav();
  draw();
}
team.onChange(refresh);
team.onChange(alert.teamChanged);
alert.init({ visible: (tab) =>
  (ws.id() !== null && session.currentSession() === tab) || desk.visible(tab),
  notify: (kind, tab) => {
    const workspace = state.workspaces.find(item => item.tabs.some(item => item.id === tab));
    if (workspace) void notifications.deliver(kind, tab, workspace.title).catch(error => say(fromBack(error), true));
  },
});
listen<string>("notification-open", ({ payload: tab }) => {
  const workspace = state.workspaces.find(item => !item.archived && !item.cleaned && item.tabs.some(item => item.id === tab));
  if (workspace) void openWorkspace(workspace, true, tab).catch(error => say(fromBack(error), true));
});
listen<[string, string, number]>("chat", ({ payload: [tab, line] }) => alert.chatChanged(tab, line));

/// React to script exits instead of polling to restore the start action.
listen<[string, number | null]>("pty-closed", ({ payload: [key] }) => dockbar.closed(key));

/// The local MCP authorizes the exact delegated conversation before requesting desktop navigation.
listen<{ workspace_id: string; conversation_id: string }>("workspace-preview", async ({ payload }) => {
  if (!payload || typeof payload.workspace_id !== "string" || typeof payload.conversation_id !== "string") return;
  try {
    const board = await invoke("load_board");
    const target = board.workspaces.find((workspace) => workspace.id === payload.workspace_id);
    if (!target || target.archived || target.cleaned || target.preparing || target.failed
      || !target.tabs.some((tab) => tab.id === payload.conversation_id)) return;
    state = board;
    await openWorkspace(target, true, payload.conversation_id);
    if (ws.id() === target.id && session.currentSession() === payload.conversation_id) await ws.showWeb();
  } catch (error) {
    say(fromBack(error), true);
  }
});

/// Usage updates affect the whole account, so any tab's activity refreshes the status bar.
listen<statusbar.Usage>("usage", ({ payload }) => statusbar.showUsage(payload));
invoke("usage").then(statusbar.showUsage).catch(() => {});
listen<statusbar.Accounts>("accounts", ({ payload }) => {
  if (statusbar.showAccounts(payload)) {
    void loadAgents().then(() => statusbar.showAgents(installed()));
  }
});
invoke("accounts").then(statusbar.showAccounts).catch((error) => say(fromBack(error), true));
listen<string>("account-error", ({ payload }) => say(fromBack(payload), true));

/// Machine resource updates arrive every three seconds only when values change.
listen<statusbar.Machine>("machine", ({ payload }) => statusbar.showMachine(payload));
statusbar.init({ say, accounts: () => { void invoke("usage_refresh", {}).catch(() => {}); settings.showAccounts(); showSettings(); } });
invoke("machine").then(statusbar.showMachine).catch(() => {});

/* File drops into conversations and terminals. */

/// Tauri intercepts HTML drag events to expose real file paths. Native drop events create conversation attachments or insert paths into dock PTYs.
type Drag = {
  type: "enter" | "over" | "leave" | "drop" | "pending" | "received";
  position?: { x: number; y: number };
  paths: string[];
  id?: string;
  error?: string;
};
/// Escape dropped paths as the macOS Terminal does, protecting shell-special characters with backslashes.
const escapePath = (p: string) => p.replace(/([\s!"#$&'()*,:;<>?[\\\]^`{|}~])/g, "\\$1");

/// A file drop targets a conversation, dock terminal, or no valid destination.
type Drop = { host: HTMLElement; put: (paths: string[]) => void; wait?: () => () => void } | null;
function targetFrom(el: Element | null): Drop {
  if (!el) return null;
  const feedbackTarget = feedback.fileDropTarget(el);
  if (feedbackTarget) return feedbackTarget;
  // Side-panel and center xterms route input to their own active PTYs.
  const where = el.closest("#dock") ? "scripts" : el.closest("#termview") ? "shell" : null;
  if (where) {
    const pty = dock.currentKey(where);
    if (!pty) return null;
    return {
      host: $(where === "scripts" ? "dock" : "termview"),
      put: (paths) => {
        // Append a space so subsequent paths or typed text do not concatenate.
        const text = paths.map(escapePath).join(" ") + " ";
        void invoke("pty_write", { session: pty, data: text })
          .then(() => dock.focus(where))
          .catch((e) => say(fromBack(e), true));
      },
    };
  }
  if (el.closest("#chatwrap")) {
    const target = session.fileDropTarget();
    if (target) return { host: $("chatwrap"), ...target };
  }
  // Desk drops target the panel under the pointer.
  return desk.dropTarget(el);
}

function dropTarget(at?: { x: number; y: number }): Drop {
  if (!at) return null;
  // macOS wry 0.55 forwards draggingLocation in logical window coordinates despite the PhysicalPosition type. Do not divide by DPR or use :hover; native drags do not update webview mouse state.
  return targetFrom(document.elementFromPoint(at.x, at.y));
}

let activeDrop: Drop = null;
function markDrop(target: Drop) {
  const host = target?.host ?? null;
  const dropHost = activeDrop?.host ?? null;
  if (dropHost === host) {
    activeDrop = target;
    return;
  }
  dropHost?.classList.remove("dropping");
  activeDrop = target;
  host?.classList.add("dropping");
}

const pendingDrops = new Map<string, { target: NonNullable<Drop>; done?: () => void }>();
function receiveDrop(drag: Drag, target: Drop) {
  if (!target) return;
  if (drag.type === "pending" && drag.id) {
    if (pendingDrops.has(drag.id)) return;
    pendingDrops.set(drag.id, { target, done: target.wait?.() });
    say(t("chat.drop.receiving"));
    return;
  }
  if (drag.paths.length) target.put(drag.paths);
  if (drag.error || !drag.paths.length) say(t(drag.error ? "chat.drop.failed" : "chat.drop.noFiles"), true);
}

listen<Drag>("file-drag", ({ payload: drag }) => {
  if (drag.type === "received") {
    const pending = drag.id ? pendingDrops.get(drag.id) : undefined;
    if (drag.id) pendingDrops.delete(drag.id);
    if (!pending) return;
    say("");
    receiveDrop(drag, pending.target);
    pending.done?.();
    return;
  }
  if (drag.type === "leave") return markDrop(null);
  if (drag.type === "enter") markDrop(null);

  // While the launcher is open, dropped files attach to its first prompt.
  if (!$("veil").hidden) {
    markDrop(null);
    if ((drag.type === "drop" || drag.type === "pending") && $("veil").querySelector("#d-prompt")) {
      const target = launcherDropTarget();
      receiveDrop(drag, target ? { host: $("veil"), ...target } : null);
    }
    return;
  }

  const target = dropTarget(drag.position);
  if (document.querySelector("dialog:modal") && !target?.host.closest(".ui-feedback-panel")) return markDrop(null);
  if (drag.type !== "drop" && drag.type !== "pending") return markDrop(target);

  // Use the final drop location, falling back to the prior target only outside the viewport. Revalidate the host so hidden or removed conversations cannot receive attachments.
  const at = drag.position;
  const outside = !at || at.x < 0 || at.y < 0 || at.x >= innerWidth || at.y >= innerHeight;
  const host = activeDrop?.host;
  const accepted = outside && host?.isConnected && host.getClientRects().length
    ? targetFrom(host)
    : target;
  markDrop(null);
  receiveDrop(drag, accepted);
});

/* Actions. */

function launch(projectId?: string, seed?: Issue, git?: Open["git"]) {
  if (!state.projects.length) return hooks.addProject();
  openLauncher(state, {
    preset: projectId,
    seed,
    git,
    toSettings: () => showSettings(),
    // Create publishes a workspace immediately while preparing its worktree in the background. The workspace view already shows preparation progress.
    go: async (draft: Draft) => {
      try {
        const created = await invoke("create_workspace", { draft, ...dock.dims() });
        // The command response and board event can arrive in either order. Insert the returned workspace locally until the next full board replaces state, avoiding premature navigation away.
        if (!state.workspaces.some((w) => w.id === created.id)) state.workspaces.push(created);
        openWorkspace(created);
      } catch (err) {
        say(fromBack(err), true);
      }
    },
  });
}

/* Side panels. */

function toggleRail() {
  const hidden = document.body.classList.toggle("norail");
  $("railshow").hidden = !hidden;
  // Move navigation arrows into the header when the sidebar is collapsed.
  (hidden ? $("railshow") : $("railtoggle")).after($("back"), $("fwd"));
}
$("railtoggle").addEventListener("click", toggleRail);
$("railshow").addEventListener("click", toggleRail);
$("sidetoggle").addEventListener("click", () => document.body.classList.toggle("noside"));

/* Persist side-panel width after dragging its left edge; double-click restores CSS defaults. Clamp width to preserve useful center space. Terminal ResizeObservers refit their hosts. */
const SIDE_W = "side-w";
let sideW = Number(localStorage.getItem(SIDE_W)) || 0;
const clampSide = (w: number) => Math.round(Math.max(280, Math.min(w, window.innerWidth * 0.6)));
function paintSide() {
  if (sideW) document.documentElement.style.setProperty("--side-w", `${sideW}px`);
  else document.documentElement.style.removeProperty("--side-w");
}
paintSide();
const grip = $("sideresize");
grip.addEventListener("pointerdown", (e) => {
  grip.setPointerCapture(e.pointerId);
  grip.classList.add("dragging");
});
grip.addEventListener("pointermove", (e) => {
  if (!grip.hasPointerCapture(e.pointerId)) return;
  sideW = clampSide(window.innerWidth - e.clientX);
  paintSide();
});
// Pointer-capture loss handles both normal release and cancellation.
grip.addEventListener("lostpointercapture", () => {
  grip.classList.remove("dragging");
  if (sideW) localStorage.setItem(SIDE_W, String(sideW));
});
grip.addEventListener("dblclick", () => {
  sideW = 0;
  paintSide();
  localStorage.removeItem(SIDE_W);
});

/// Dispatch actions from document shortcuts and native Mac menu accelerators. Return whether the shortcut was handled so its key event can be consumed.
function act(a: appmenu.Action): boolean {
  if (document.querySelector("dialog[open], .ui-feedback-panel:not([hidden])")) return true;
  const open = ws.id();
  // Local workspace shortcuts do nothing for remote shares instead of sending unknown IDs to the backend.
  const own = open && !team.isRemote(open) ? open : null;
  switch (a) {
    case "novoWorkspace":
      launch(state.workspaces.find((w) => w.id === open)?.project);
      return true;
    case "novaConversa":
      if (!own) return false;
      ws.newTab();
      return true;
    case "arquivar":
      if (!own) return false;
      hooks.archive(own, true);
      return true;
    // Shift-Command-D finishes and archives work in one action.
    case "concluir":
      if (!own) return false;
      hooks.finish(own);
      return true;
    case "run":
      if (!own) return false;
      dockbar.toggleRun();
      return true;
    case "lateral":
      toggleRail();
      return true;
    // Shift-Command-M comments on the current conversation selection.
    case "nota":
      return !!open && ws.quoteSelection();
    // Command-comma opens preferences following macOS conventions.
    case "ajustes":
      showSettings();
      return true;
    // Command-P opens a file of the current workspace or project by name. It is consumed even
    // without a local workspace so the webview never falls back to printing.
    case "quickOpen":
      ws.quickOpen();
      return true;
    // Command-W closes the focused dock terminal first. When a native browser view owns focus, ignore stale dock focus and close the active center tab.
    case "fechar":
      return (document.hasFocus() && dockbar.closeFocused()) || ws.closeActive();
    case "voltar":
      travel(-1);
      return true;
    case "avancar":
      travel(1);
      return true;
  }
}

/// Normalize shortcut keys to lowercase because Shift changes event.key casing.
function shortcut(e: KeyboardEvent): appmenu.Action | null {
  const k = e.key.toLowerCase();
  if (e.shiftKey) {
    if (k === "a") return "arquivar";
    if (k === "d") return "concluir";
    if (k === "m") return "nota";
    return null;
  }
  if (k === "n") return "novoWorkspace";
  if (k === "p") return "quickOpen";
  if (k === "t") return "novaConversa";
  if (k === "r") return "run";
  if (k === "b") return "lateral";
  if (k === ",") return "ajustes";
  if (k === "w") return "fechar";
  if (k === "[") return "voltar";
  if (k === "]") return "avancar";
  return null;
}

document.addEventListener("keydown", (e) => {
  // preventDefault avoids triggering the same action again through the native menu accelerator.
  const a = e.metaKey || e.ctrlKey ? shortcut(e) : null;
  if (a && act(a)) e.preventDefault();
  // Native dialogs own Escape before the launcher beneath them can be dismissed.
  if (e.key === "Escape" && !document.querySelector("dialog[open]") && !$("veil").hidden) {
    $("veil").hidden = true;
    $("veil").replaceChildren();
  }
});

void appmenu.install(act);

/* Startup. */

// The system draws the frame outside macOS.
document.body.classList.toggle("framed", !mac);
// Translate static index.html labels before building the remaining interface.
paint();
// Send the active language to backend-generated text such as dock output and OAuth completion pages.
invoke("set_lang", { lang: current() });

for (const [id, name] of [
  ["railtoggle", "panel-left"],
  ["railshow", "panel-left"],
  ["back", "arrow-left"],
  ["fwd", "arrow-right"],
  ["sidetoggle", "panel-right"],
  ["reveal", "external-link"],
  ["wback", "arrow-left"],
  ["wfwd", "arrow-right"],
  ["wreload", "rotate"],
  ["wext", "external-link"],
  ["collapse", "list-tree"],
  ["dock-again", "rotate"],
  ["run-pick", "chevron-down"],
  ["dseen", "check"],
  ["dfold", "chevron-up"],
  ["settings", "settings"],
] as const) {
  $(id).innerHTML = icon(name);
}

links.init(say);
initCodeCopy(() => {
  const version = messageVersion;
  return (message, isError) => {
    // A delayed copy confirmation must not replace a newer application status.
    if (isError || version === messageVersion) say(message, isError);
  };
});
feedback.init();
void update.init(say);
// Discover installations without waiting for their independently loaded model catalogs.
void loadAgents().then(() => statusbar.showAgents(installed()));
// Initialize team state before Settings and sidebar render its data.
team.onError((m) => say(m, true));
await team.init();
cloud.init(() => { draw(); void team.refreshOrganizations(cloud.current()); }, message => say(message, true));
// Load the MCP hub during startup so Settings and both pickers have their registry at first use.
mcp.init({ say });
void mcp.load();
// Load plugins during startup for the same shared picker behavior.
plugins.init({ say });
void plugins.load();
// Initialize Settings before Issues because it discovers the Linear connection.
await settings.init({ say });
issues.init({
  say,
  board: () => state,
  connected: () => settings.linear().connected,
  redraw: draw,
  open: (w) => openWorkspace(w),
  create: (issue) => launch(state.workspaces.find((w) => w.id === ws.id())?.project, issue),
  toSettings: () => showSettings(),
});
archived.init({ board: () => state, hooks: () => hooks });
ws.init({
  say, board: view, redraw: draw, home: () => showDesk(),
  launchBranch: (project, base, branch) => launch(project, undefined, { base, branch }),
  newWorkspace: (project) => launch(project),
  openGitWorkspace: async (id) => {
    const target = state.workspaces.find((workspace) => workspace.id === id);
    if (!target) return;
    try {
      if (target.archived) {
        await invoke("archive_workspace", { id, archived: false });
        target.archived = false;
      }
      await openWorkspace(state.workspaces.find((workspace) => workspace.id === id) ?? target);
    } catch (err) {
      say(fromBack(err), true);
    }
  },
});
// Provide the same tab ownership, connection, and collaboration context to workspace and desk composers.
actions.init(async (workspace, tab) => {
  const target = state.workspaces.find(w => w.id === workspace);
  if (!target) return;
  if (!target.tabs.some(t => t.id === tab.id)) target.tabs.push(tab);
  target.active = tab.id;
  await openWorkspace({ ...target, active: tab.id });
});
const infoOf = (w: Workspace | undefined, tab: Tab | undefined): Info => ({
  workspace: w?.id ?? null,
  status: tab?.status ?? null,
  task: tab?.task ?? null,
  mcp: tab?.task ? null : (w?.mcp ?? null),
  plugins: tab?.task ? null : (w?.plugins ?? null),
  skills: tab?.task ? null : (w?.skills ?? null),
  pending: tab?.pending_prompt ?? null,
  worktree: w?.worktree ?? null,
  remote: w?.remote ? { name: team.nameOf(w.remote.owner), online: w.remote.online } : null,
  team: !!team.status().config && !!w && (team.sharedHere(w) || !!w.remote),
  remoteControl: w?.remote_control ?? false,
  // Tab choice presence controls inheritance. An explicitly empty model still selects the CLI default rather than workspace defaults.
  agent: tab?.choice ? tab.choice.agent : (w?.agent ?? "claude"),
  model: tab?.choice ? tab.choice.model : (w?.model ?? ""),
  effort: tab?.choice ? tab.choice.effort : (w?.effort ?? ""),
});
session.init(
  (m) => say(m, true),
  () => {
    const open = ws.id();
    const w = open ? view().workspaces.find((x) => x.id === open) : undefined;
    return infoOf(w, w?.tabs.find((t) => t.id === session.currentSession()));
  },
  { comment: ws.comment, thread: ws.showNote },
);
desk.init({
  say,
  board: () => state,
  info: (tab) => {
    const w = state.workspaces.find((x) => x.tabs.some((t) => t.id === tab));
    return infoOf(w, w?.tabs.find((t) => t.id === tab));
  },
  // Opening a desk panel selects its tab locally while the backend publishes the active-tab update.
  open: (w, tab) => {
    invoke("focus_tab", { workspace: w.id, tab });
    openWorkspace({ ...w, active: tab });
  },
  create: () => launch(),
  looked: alert.looked,
});
viewer.init((m) => say(m, true), ws.fileSaved);
dock.init($("dockterm"), $("shellterm"));
state = await invoke("load_board");
actions.update(state);
team.boardChanged(state);
alert.boardChanged(state);
showDesk();

// Show release notes after the initial page renders so the dialog overlays the application.
void news.init();

// General discovery is advisory; explicit task monitors keep their configured clock in Rust.
const prScan = new PrScanPolicy();
let prTimer: ReturnType<typeof setTimeout> | null = null;
const schedulePrs = () => {
  if (prTimer) clearTimeout(prTimer);
  prTimer = setTimeout(() => scanPrs(), prScan.remaining(background.current(), Date.now()));
};
const scanPrs = () => {
  prScan.mark(Date.now());
  void invoke("refresh_prs").catch(() => {});
  schedulePrs();
};
let previousPrContext = background.current();
background.subscribe((next) => {
  const returned = prScan.returnedToForeground(previousPrContext, next, Date.now());
  previousPrContext = next;
  if (returned) scanPrs();
  else schedulePrs();
});
scanPrs();
