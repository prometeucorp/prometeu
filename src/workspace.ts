import * as actions from "./actions";
import * as background from "./background";
import { GitRefreshPolicy, changesInterval } from "./git-refresh";
import { invoke } from "./ipc";
import * as sidebar from "./sidebar";
import * as browser from "./browser";
import { openCleanup } from "./cleanup";
import * as diff from "./diff";
import * as dockbar from "./dockbar";
import type * as fileMenu from "./file-menu";
import { avatar, icon, stageIcon, wave } from "./icons";
import { fromBack, stage as stageName, t, tn } from "./i18n";
import { fitsEffort, modelLabel } from "./model-choice";
import { openModelPicker } from "./model-picker";
import { openQuickOpen } from "./quick-open";
import * as menu from "./menu";
import * as notes from "./notes";
import * as rename from "./rename";
import * as session from "./session";
import * as team from "./team";
import * as tree from "./tree";
import { relocate, relocateTabs } from "./tree-moves";
import {
  fmtTokens,
  label,
  merged,
  prs,
  pending,
  repoPath,
  tabLabel,
  type Board,
  type Choice,
  type Project,
  type Tab,
  type Workspace,
} from "./types";
import { $, debounce, h, template } from "./util";
import * as viewer from "./viewer";
import * as changesUi from "./workspace-changes";

/// Coordinate workspace breadcrumbs, tabs, center views, and the side panel.

export type Ctx = {
  say: (text: string, isError?: boolean) => void;
  board: () => Board;
  /// Redraw the application sidebar and workspace.
  redraw: () => void;
  /// Return home when the active workspace disappears.
  home: () => void;
  launchBranch: (project: string, base: string, branch?: string) => void;
  /// Open the launcher with a selected project.
  newWorkspace: (project: string) => void;
  openGitWorkspace: (id: string) => void;
};

let ctx: Ctx;
let openWs: string | null = null;
/// Invalidate asynchronous continuations on every entry/exit, including returning to the same workspace ID.
let navigation = 0;
const gitRefresh = new GitRefreshPolicy();

export const id = () => openWs;
/// Resolve file-tree and viewer roots from either the selected workspace or a directly opened project.
const root = () => openWs ?? proj?.id ?? null;
const current = () => ctx.board().workspaces.find((w) => w.id === openWs);
const stillHere = (epoch: number, id: string) => navigation === epoch && openWs === id;

export function init(context: Ctx) {
  ctx = context;
  changesUi.init({
    workspace: current,
    refresh: async () => { if (openWs && hasDiff()) { await loadChanges(openWs); tree.redrawSoon(); } },
    show: activateChanges,
    say: ctx.say,
    openFile: (repo, path) => void openRepoFile(repo, path),
    launchBranch: ctx.launchBranch,
    openWorkspace: ctx.openGitWorkspace,
    fileHost: changesHost,
  });

  tree.init({
    openFile,
    workspace: root,
    host: treeHost,
    moved: (id, from, to) => void treeMoved(id, from, to),
    say: ctx.say,
  });
  dockbar.init({
    workspace: root,
    say: ctx.say,
    // The scripts file is relative to the primary repository.
    openFile: (path) => openRepoFile(undefined, path),
    newTab,
    openBrowser: showWeb,
    enter: showShell,
    exit: showTerm,
    drawTabs: () => {
      if (proj) return drawProjectTabs();
      const ws = current();
      if (ws) drawTabs(ws);
    },
  });
  browser.init((id) => invoke("open_run", { id }).catch((e) => ctx.say(fromBack(e), true)), ctx.say);
  notes.init({
    workspace: id,
    tab: session.currentSession,
    open: openComments,
    focus: session.focusAnchor,
    say: ctx.say,
  });

  $("tab-files").addEventListener("click", () => setSidePane("files"));
  $("tab-diff").addEventListener("click", () => {
    // Clicking the already-selected Changes panel restores its center diff after the tab was closed.
    if (sidePane === "diff") changesUi.show();
    else setSidePane("diff");
  });
  $("tab-comments").addEventListener("click", openComments);
  // Review opens the full stacked diff in the center.
  $("review").innerHTML = `${icon("eye", 13)}<span></span>`;
  $("review").querySelector("span")!.textContent = t("side.review");
  $("review").addEventListener("click", () => {
    setSidePane("diff");
    changesUi.show("compare");
  });
  $("reveal").addEventListener("click", () => {
    const here = root();
    if (here) invoke("reveal_path", { id: here, rel: "" }).catch((e) => ctx.say(fromBack(e), true));
  });
  // Native focus/power changes choose the visible fallback budget; external edits still surface.
  let wasForeground = background.foreground(background.currentOrDocument());
  background.subscribe((context) => {
    const foreground = background.foreground(context);
    if (foreground && !wasForeground && hasDiff()) reloadChanges(openWs!);
    wasForeground = foreground;
    scheduleChanges();
  });
  window.addEventListener("focus", () => {
    if (!background.hasObservation()) {
      if (hasDiff()) reloadChanges(openWs!);
      scheduleChanges();
    }
  });
  scheduleChanges();

  // Build the static preparation indicator once instead of on every board redraw.
  $("offwave").innerHTML = wave(22);

  // paintPr chooses among creating, updating, opening, and finishing PR work.
  $("pr").innerHTML = `${icon("git-pull-request", 14)}<span></span>`;
  $("pr").querySelector("span")!.textContent = t("ws.pr");

  // Handle tab rename double-clicks on the strip because the first click may replace the tab button.
  $("tabbar").addEventListener("dblclick", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>(".tab[data-tab]");
    if (b?.dataset.tab && !current()?.remote) editTab(b.dataset.tab);
  });
}

/* Workspace entry and exit. */

export async function open(ws: Workspace, tab?: string) {
  proj = null;
  if (tab && openWs === ws.id) return selectTab(ws.id, tab);
  const epoch = ++navigation;
  // Direct workspace switching bypasses leave; hide the previous native webview here.
  browser.hide();
  session.detach();
  center("chatwrap");
  const first = ws.tabs.find((t) => t.id === (tab ?? ws.active)) ?? ws.tabs[0];
  sidebar.setOpen((openWs = ws.id));
  changesUi.enter();
  $("wsView").hidden = false;
  $("wsctl").hidden = false;
  // Remote workspaces expose conversation and comments only; their files and processes belong to another Mac.
  if (ws.remote) {
    showTerm();
    if (first && !(await session.attach(first.id, ws.id))) return;
    if (!stillHere(epoch, ws.id)) return;
    ctx.redraw();
    return;
  }
  // Opening acknowledges unread state and keeps visible activity from becoming unread again.
  invoke("look_at", { id: ws.id });
  tree.reset();
  dockbar.reset();
  // Preparing or failed workspaces have no usable tabs or files. catchUp attaches after a later board update creates the first tab.
  if (pending(ws)) {
    showTerm();
    ctx.redraw();
    return;
  }
  // Cleaned workspaces display retained history without local files or processes.
  if (ws.cleaned) {
    ctx.redraw();
    return;
  }
  // Attach before drawing tabs because attachment establishes the selected session.
  if (first && !(await session.attach(first.id))) return;
  if (!stillHere(epoch, ws.id)) return;
  if (tab && first) invoke("focus_tab", { workspace: ws.id, tab: first.id });
  // Restore the previously selected file or diff view.
  const fs = files(ws.id);
  if (tab) showTerm();
  else if (fs.web) await showWeb();
  else if (fs.diff) showChanges();
  else if (fs.active) await showFile();
  else showTerm();
  if (!stillHere(epoch, ws.id)) return;
  if (!ws.archived) void invoke("pr_open", { id: ws.id }).catch(() => {});
  ctx.redraw();
}

/// Attach newly created tabs when preparation completes, and follow the backend when the attached
/// tab leaves the board so closing a conversation selects the remaining one without a click.
/// Current-session state changes before awaiting the snapshot so consecutive redraws do not duplicate attachment.
function catchUp(ws: Workspace) {
  if (ws.remote || ws.cleaned || pending(ws)) return;
  const attached = session.currentSession();
  if (attached && ws.tabs.some((t) => t.id === attached)) return;
  const first = ws.tabs.find((t) => t.id === ws.active) ?? ws.tabs[0];
  if (first) {
    const epoch = navigation;
    void session.attach(first.id).then((attached) => {
      if (attached && stillHere(epoch, ws.id)) ctx.redraw();
    });
  }
}

export function leave() {
  navigation++;
  gitRefresh.clear();
  proj = null;
  browser.hide();
  restoreBrowserSide();
  session.detach();
  invoke("look_at", { id: null });
  sidebar.setOpen((openWs = null));
  $("wsView").hidden = true;
  $("wsctl").hidden = true;
}

/* Rendering. */

export function draw() {
  const ws = current();
  if (!ws) return ctx.home();
  catchUp(ws);

  // Show project, workspace, and branch breadcrumbs; remote conversations identify their owner first.
  const remote = ws.remote;
  const owner = remote ? team.nameOf(remote.owner) : null;
  const crumb = $("crumb");
  // Rebuild breadcrumb structure only when identity changes. Ordinary board updates replace values to avoid flicker and accumulated listeners.
  const crumbKey = `${ws.id}\u0000${owner ?? ws.repo_name}`;
  if (crumb.dataset.workspace !== crumbKey || !crumb.querySelector(".nm")) {
    crumb.dataset.workspace = crumbKey;
    crumb.innerHTML =
      `${avatar(owner ?? ws.repo_name)}<span class="who"></span><button class="repos" hidden></button>` +
      `<span class="sep">${icon("chevron-right", 12)}</span><span class="nm"></span>` +
      `<button class="branch" hidden>${icon("git-branch", 12)}<span></span></button>`;
  }
  // Use a repository count and picker instead of crowding the header with every name.
  const many = !owner && ws.repos.length > 1;
  const who = crumb.querySelector<HTMLElement>(".who")!;
  const repos = crumb.querySelector<HTMLElement>(".repos")!;
  who.hidden = many;
  who.textContent = owner ?? ws.repo_name;
  repos.hidden = !many;
  if (many) {
    repos.innerHTML = `${icon("folder", 12)}<span></span>`;
    repos.children[1].textContent = tn(ws.repos.length, "diff.repos");
    repos.title = ws.repos.map((r) => r.name).join(" · ");
    repos.onclick = () => {
      const at = repos.getBoundingClientRect();
      menu.openAt({ x: at.left, y: at.bottom + 4 }, repoItems(ws));
    };
  }
  const name = crumb.querySelector<HTMLElement>(".nm")!;
  name.textContent = ws.title;
  if (!remote) {
    // The breadcrumb title supports unobstructed double-click renaming.
    name.title = t("ws.rename");
    name.ondblclick = () => rename.start(name, ws.title, (title) => renameWorkspace(ws.id, title), "crumb");
  } else {
    name.title = "";
    name.ondblclick = null;
  }
  // Render the persisted branch immediately, then refresh it from Git.
  paintBranchName(branchOf.get(ws.id) ?? ws.branch);

  drawTabs(ws);
  const tab = ws.tabs.find((t) => t.id === session.currentSession());
  drawShare(ws, tab);
  drawMore(ws);
  // The composer explains detached sessions and offline remote owners.
  session.refresh();
  const collaborative = !!team.status().config && (team.sharedHere(ws) || !!remote);
  $("tab-comments").hidden = !collaborative;
  if (!collaborative && sidePane === "comments") setSidePane("files");
  notes.draw();

  if (remote) {
    // Remote branch metadata comes from its owner; local PR, Git, file-tree, and dock actions are unavailable.
    paintBranchName(ws.branch);
    $("offline").hidden = true;
    layout({ tabs: true, side: true });
    setSidePane("comments");
    return;
  }

  // Preparing or failed workspaces show only their status panel because no usable worktree exists.
  if (pending(ws)) {
    paintBranchName(ws.branch);
    layout({});
    $("offline").hidden = false;
    $("offwave").hidden = !!ws.failed;
    $("offtitle").textContent = t(ws.failed ? "build.failed.title" : "build.title");
    // Preparation needs only its indicator and title; failures additionally show the translated backend reason.
    $("offbody").hidden = !ws.failed;
    if (ws.failed) $("offbody").textContent = fromBack(ws.failed);
    $("offpath").textContent = ws.worktree;
    return;
  }

  // Hide controls requiring a worktree when only retained history is available.
  layout({ tabs: !ws.cleaned, pr: true, side: !ws.cleaned, files: true, changes: true, dock: true });
  $("offpath").textContent = ws.worktree;
  drawBranch(ws);
  drawPr(ws);
  if (gitRefresh.consider(ws) && background.foreground(background.currentOrDocument())) {
    reloadChanges(ws.id);
    if (sidePane === "files") tree.redrawSoon();
  }
  // Board events refresh the displayed file after agent edits without polling.
  const file = files(ws.id).active;
  if (file) viewer.show(ws.id, file);

  // Detached conversations resume on input; cleaned workspaces instead explain that only history remains.
  $("offline").hidden = !ws.cleaned;
  $("offwave").hidden = true;
  $("offbody").hidden = false;
  $("offtitle").textContent = t("gone.title");
  $("offbody").textContent = t("gone.body");
}

/// Centralize feature visibility for local, remote, preparing, cleaned, and project-only modes so controls cannot leak between modes.
type Parts = {
  /// Center tab strip.
  tabs?: boolean;
  /// Header PR action.
  pr?: boolean;
  /// Side panel and its visibility toggle.
  side?: boolean;
  /// Files panel with collapse and Finder actions.
  files?: boolean;
  /// Changes panel and its Review action.
  changes?: boolean;
  /// Setup and Run at the bottom of the side panel.
  dock?: boolean;
};

function layout(parts: Parts) {
  $("tabbar").hidden = !parts.tabs;
  $("prsplit").hidden = !parts.pr;
  $("side").hidden = !parts.side;
  $("sidetoggle").hidden = !parts.side;
  $("tab-files").hidden = !parts.files;
  $("collapse").hidden = !parts.files;
  $("reveal").hidden = !parts.files;
  $("tab-diff").hidden = !parts.changes;
  // Diff state enables Review; mode changes only disable it when Changes is unavailable.
  if (!parts.changes) $("review").hidden = true;
  $("dock").hidden = !parts.dock;
}

/// Local shared workspaces offer audience selection and viewer avatars. The relay enforces visibility; remote workspaces belong to their owners.
function drawShare(ws: Workspace, tab?: Tab) {
  const btn = $("share") as HTMLButtonElement;
  const chips = $("watchers");
  chips.replaceChildren();
  // Keep inactive sharing in the menu; active sharing uses its icon and avatars without a long status label.
  if (ws.remote || ws.cleaned || !team.status().config || !team.sharedWithTeam(ws)) {
    btn.hidden = true;
    return;
  }
  btn.hidden = false;
  btn.className = "ico on";
  btn.innerHTML = icon("share-2", 14);
  btn.title = `${shareLabel(ws)} — ${t("share.off.title")}`;
  btn.onclick = () => {
    const at = btn.getBoundingClientRect();
    menu.openAt({ x: at.left, y: at.bottom + 4 }, shareItems(ws));
  };
  if (!tab) return;
  const watching = team.watchersOf(tab.id);
  for (const name of watching.slice(0, 3)) {
    const c = template("span", "watcher", avatar(name));
    c.title = t("share.watching", { name });
    chips.append(c);
  }
  if (watching.length > 3) {
    const rest = template("span", "watcher more", `+${watching.length - 3}`);
    rest.title = watching.slice(3).join(" · ");
    chips.append(rest);
  }
}

/// Move infrequent actions into menus. The sidebar already shows stage; header sharing appears after activation.
function drawMore(ws: Workspace) {
  const btn = $("wsmore") as HTMLButtonElement;
  const available = !ws.remote && !ws.cleaned && !pending(ws);
  btn.hidden = !available;
  if (!available) return;

  btn.innerHTML = icon("ellipsis", 16);
  const stages = ctx.board().stages;
  const at = stages.indexOf(ws.stage);
  const items: menu.Item[] = [
    {
      label: t("ws.menu.stage"),
      glyph: stageIcon(at, stages.length),
      hint: stageName(ws.stage),
      sub: stages.map((name, i) => ({
        label: stageName(name),
        glyph: stageIcon(i, stages.length),
        checked: name === ws.stage,
        run: () => setStage(ws.id, name),
      })),
    },
  ];
  if (team.status().config && !team.sharedWithTeam(ws)) {
    items.unshift({ label: t("share.on"), glyph: icon("share-2", 14), sub: shareItems(ws) });
  }
  btn.onclick = () => {
    const box = btn.getBoundingClientRect();
    menu.openAt({ x: box.left, y: box.bottom + 4 }, items);
  };
}

function shareLabel(ws: Workspace): string {
  if (!team.sharedWithTeam(ws)) return t("share.on");
  if (!ws.audience) return t("share.off");
  return tn(ws.audience.length, "share.some");
}

/// Toggle individual viewers or the whole team; removing the final viewer stops sharing.
function shareItems(ws: Workspace): menu.Item[] {
  const me = team.status();
  const others = team.people().filter((m) => m.id !== team.personOf(me.you));
  const set = (audience: string[] | null | false) => team.share(ws.id, audience).catch((e) => ctx.say(fromBack(e), true));
  const all = team.sharedWithTeam(ws) && !ws.audience;
  const some = team.sharedWithTeam(ws) && ws.audience ? ws.audience : [];
  const items: menu.Item[] = [
    { label: t("share.all"), glyph: icon("users", 14), checked: all, run: () => set(all ? false : null) },
    "sep",
    ...others.map((m): menu.Item => {
      const on = some.includes(m.id);
      return {
        label: m.name,
        glyph: avatar(m.name),
        hint: m.online ? undefined : t("team.offline"),
        checked: on,
        run: () => set(on ? some.filter((id) => id !== m.id) : [...some, m.id]),
      };
    }),
  ];
  if (!others.length) items.push({ label: t("share.alone"), disabled: true });
  if (team.sharedWithTeam(ws)) items.push("sep", { label: t("share.stop"), glyph: icon("x", 14), danger: true, run: () => set(false) });
  return items;
}

/// Repository chips open a change summary and navigation menu instead of permanently listing every repository in the header.
function repoItems(ws: Workspace): menu.Item[] {
  const all = changesUi.statuses(ws.id) ?? [];
  return ws.repos.map((repo, index): menu.Item => {
    const status = all.find(item => item.repo === index);
    const count = status ? new Set([...status.staged, ...status.changes, ...status.conflicts].map(file => file.path)).size : 0;
    return { label: repo.name, glyph: avatar(repo.name), hint: count ? tn(count, "diff.files") : t("git.clean"), run: () => changesUi.selectRepo(index) };
  });
}

/* Workspace actions. */

export function renameWorkspace(id: string, title: string | null) {
  ctx.redraw();
  if (title) invoke("rename_workspace", { id, title }).catch((e) => ctx.say(fromBack(e), true));
}

export const setStage = (id: string, stage: string) => invoke("set_stage", { id, stage });

/// Send the backend-generated PR prompt to the active conversation, resuming it if needed.
async function openPr() {
  const ws = current();
  if (!ws) return;
  const configured = actions.catalog().commands.find(a => a.name === actions.catalog().pr_action && a.kind === "agent");
  if (configured) {
    try { await actions.start(ws.id, configured); } catch (error) { ctx.say(fromBack(error), true); }
    return;
  }
  const epoch = navigation;
  const tab = ws.tabs.find((t) => t.id === session.currentSession()) ?? ws.tabs[0];
  if (!tab) return;
  try {
    const prompt = await invoke("pr_prompt", { id: ws.id });
    await invoke("chat_send", { session: tab.id, text: prompt });
    // Follow the request into its conversation view.
    if (tab.id !== session.currentSession() && !(await session.attach(tab.id))) return;
    if (!stillHere(epoch, ws.id)) return;
    showTerm();
    drawTabs(ws);
    session.focus();
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
}

/* Current worktree branch. */

/// Read the actual Git branch rather than the creation-time value. Cache it to avoid flickering during frequent board updates.
const branchOf = new Map<string, string>();

function paintBranch(id: string) {
  paintBranchName(branchOf.get(id));
}

/// Display the latest known local branch or remote-owner metadata.
function paintBranchName(name: string | undefined) {
  const chip = $("crumb").querySelector<HTMLElement>(".branch");
  if (!chip || !name) return;
  chip.hidden = false;
  chip.children[1].textContent = name;
  chip.title = t("git.branches");
  chip.onclick = () => { if (current() && diffable(current()!)) changesUi.show("branches"); };
}

function drawBranch(ws: Workspace) {
  paintBranch(ws.id);
  askBranch(ws.id);
}

/// Throttle Git branch reads because adjacent agent events rarely change the result.
const askBranch = debounce(400, async (id: string) => {
  // An empty branch name means detached HEAD; label it explicitly.
  const name = (await invoke("workspace_branch", { id })) ?? t("ws.branch.detached");
  branchOf.set(id, name);
  if (openWs === id) paintBranch(id);
});

/// Diff and PR creation require a prepared local worktree, excluding remote, cleaned, and preparing workspaces.
const diffable = (ws: Workspace) => !ws.remote && !ws.cleaned && !pending(ws);
const hasDiff = () => {
  const ws = current();
  return !!ws && ws.id === openWs && diffable(ws);
};

/* Branch PRs. */

/// Choose PR actions from observed state: create, update unpublished work, open existing up-to-date PRs, or finish merged work. List multiple repositories in a menu and throttle backend refreshes.
function paintPr(ws: Workspace) {
  const done = merged(ws);
  const all = prs(ws);
  const ask = $("pr");
  $("prsplit").hidden = !diffable(ws);
  // Offer opening existing PRs only after the diff confirms nothing needs updating; keep the request action while state is unknown.
  const left = outstanding(ws.id);
  const quiet = all.length > 0 && !done && left !== null && !left.dirty && !left.unpushed;
  ask.querySelector("span")!.textContent = done
    ? t("ws.finish")
    : quiet
      ? tn(all.length, "ws.pr.open")
      : all.length
        ? t("ws.pr.update")
        : t("ws.pr");
  ask.title = done ? t("top.finish") : quiet ? t("top.pr.go") : all.length ? t("top.pr.update") : t("top.pr");
  ask.classList.toggle("done", done);
  ask.firstElementChild!.outerHTML = icon(done ? "check" : "git-pull-request", 14);
  ask.onclick = () => {
    if (done) return finish(ws.id);
    if (quiet) return all.length === 1 ? openIn(ws, all[0].repo) : prMenu(ws, ask);
    void openPr();
  };

  // Use one PR menu instead of a header button per repository.
  const pick = $("prpick");
  pick.hidden = !all.length;
  pick.innerHTML = icon("chevron-down", 14);
  pick.onclick = () => prMenu(ws, pick);
}

/// List PR state per repository; the backend resolves each browser destination through gh.
function prMenu(ws: Workspace, at: HTMLElement) {
  const many = ws.repos.length > 1;
  const box = at.getBoundingClientRect();
  menu.openAt(
    { x: box.left, y: box.bottom + 4 },
    prs(ws).map(({ repo, pr }) => ({
      label: many ? `${repo} · #${pr.number}` : `#${pr.number} ${pr.title}`,
      glyph: icon(pr.state === "MERGED" ? "check" : "git-pull-request", 14),
      hint: t(pr.state === "MERGED" ? "pr.merged" : pr.isDraft ? "pr.draft" : "pr.open"),
      run: () => openIn(ws, repo),
    })),
  );
}

const openIn = (ws: Workspace, repo: string) =>
  invoke("open_pr", { id: ws.id, repo }).catch((e) => ctx.say(fromBack(e), true));

function drawPr(ws: Workspace) {
  paintPr(ws);
}

/// Finish moves work to the final stage and archives it, stopping its agent and docks.
export function finish(id: string) {
  const target = ctx.board().workspaces.find((workspace) => workspace.id === id);
  if (openWs === id) ctx.home();
  invoke("finish_workspace", { id })
    .then(() => {
      if (target && !target.remote && !target.cleaned && target.worktree !== target.repo) openCleanup(ctx.say, id);
    })
    .catch((e) => ctx.say(fromBack(e), true));
}

/* Tabs. */

let tabsFrame = 0;

// Defer strip redraws until pointer gestures finish so press/release keeps its click target.
function deferTabs() {
  const active = $("tabbar").matches(":active");
  if (active && !tabsFrame) {
    tabsFrame = requestAnimationFrame(() => {
      tabsFrame = 0;
      if (proj) drawProjectTabs();
      else {
        const ws = current();
        if (ws) drawTabs(ws);
      }
    });
  }
  return active;
}

/// Order conversation tabs before explicitly opened Changes, browser, shell, and file tabs, followed by creation.
function drawTabs(ws: Workspace) {
  // Preserve active rename inputs during strip redraws.
  if (rename.editing() || deferTabs()) return;
  const bar = $("tabbar");
  bar.replaceChildren();
  const fs = files(ws.id);
  const elsewhere = fs.diff || fs.active || !!dockbar.front();

  const remote = !!ws.remote;
  for (const tab of ws.tabs) {
    const b = document.createElement("button");
    b.className = "tab" + (!elsewhere && tab.id === session.currentSession() ? " on" : "");
    b.innerHTML = `<i class="dot"></i><span></span><span class="n tokens"></span>`;
    (b.children[0] as HTMLElement).style.background = `var(--dot-${tab.status})`;
    b.dataset.tab = tab.id;
    b.children[1].textContent = tabLabel(ws, tab);
    b.children[2].textContent = tab.tokens ? `~${fmtTokens(tab.tokens)}` : "";
    b.title =
      label(tab.status) +
      (tab.tokens ? t("tab.tokens", { n: fmtTokens(tab.tokens) }) : "") +
      // Show model labels only when sibling conversations differ.
      (tab.choice ? t("tab.model", { model: modelLabel(tab.choice.model, tab.choice.agent) }) : "") +
      t("tab.rename");
    b.addEventListener("click", () => selectTab(ws.id, tab.id));
    if (!remote) {
      b.addEventListener("contextmenu", (e) => {
        e.preventDefault();
        menu.openAt({ x: e.clientX, y: e.clientY }, tabMenu(ws, tab));
      });
    }

    if (ws.tabs.length > 1 && !remote) {
      const x = document.createElement("span");
      x.className = "tabx ico sm";
      x.innerHTML = icon("x", 12);
      x.title = t("tab.close");
      x.addEventListener("click", (e) => {
        e.stopPropagation();
        invoke("close_tab", { workspace: ws.id, tab: tab.id });
      });
      b.append(x);
    }
    bar.append(b);
  }

  // Changes-tab visibility belongs to the user. Dirty Git state must not reopen a closed tab; the side-panel count still indicates changes.
  const changes = total(ws.id);
  if (fs.diffTab) {
    const b = document.createElement("button");
    b.className = "tab file" + (fs.diff ? " on" : "");
    b.innerHTML = `${icon("diff", 14)}<span></span><span class="n"></span>`;
    b.children[1].textContent = t("tab.changes");
    b.children[2].textContent = String(changes);
    b.children[2].classList.remove("fresh");
    b.title = t("tab.changes.title");
    b.addEventListener("click", () => showChanges());
    const x = document.createElement("span");
    x.className = "tabx ico sm";
    x.innerHTML = icon("x", 12);
    x.title = t(fs.diff ? "tab.changes.closeKey" : "tab.changes.close");
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      closeChanges();
    });
    b.append(x);
    bar.append(b);
  }

  // Keep the browser tab until explicitly closed, independently of Run process restarts.
  if (fs.webTab) {
    const b = document.createElement("button");
    b.className = "tab file" + (fs.web ? " on" : "");
    b.innerHTML = `${icon("globe", 14)}<span></span><span class="n"></span>`;
    b.children[1].textContent = t("tab.browser");
    b.children[2].textContent = fs.port ? `:${fs.port}` : "";
    b.title = t("tab.browser.title", { port: fs.port });
    b.addEventListener("click", () => void showWeb());
    const x = document.createElement("span");
    x.className = "tabx ico sm";
    x.innerHTML = icon("x", 12);
    x.title = t(fs.web ? "tab.browser.closeKey" : "tab.browser.close");
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      void closeWeb();
    });
    b.append(x);
    bar.append(b);
  }

  // Shells occupy center tabs for interactive work; Setup and Run remain side-panel output.
  if (!remote && !ws.cleaned && !pending(ws)) appendTermTabs(bar);

  appendFileTabs(bar, fs);

  // New conversations require the owner's local worktree.
  if (remote) return;
  // The primary plus action inherits workspace defaults; its menu offers alternative models.
  appendTabAdd(bar, {
    title: t("tab.new"),
    add: () => void newTab(),
    pickTitle: t("tab.new.model"),
    pick: (at) => pickModel(at, ws),
  });
}

/// Reuse the same split creation control across tab strips, varying only its action and menu choices.
function appendTabAdd(
  bar: HTMLElement,
  opts: { title: string; add: () => void; pickTitle: string; pick: (at: HTMLElement) => void },
) {
  const box = h("div", "tabadd");
  const plus = document.createElement("button");
  plus.className = "ico";
  plus.innerHTML = icon("plus");
  plus.title = opts.title;
  plus.addEventListener("click", opts.add);
  const pick = document.createElement("button");
  pick.className = "ico caret";
  pick.innerHTML = icon("chevron-down", 12);
  pick.title = opts.pickTitle;
  pick.addEventListener("click", () => opts.pick(pick));
  box.append(plus, pick);
  bar.append(box);
}

/// Shell tabs work in both workspaces and directly opened projects.
function appendTermTabs(bar: HTMLElement) {
  for (const d of dockbar.tabs()) {
    const b = document.createElement("button");
    b.className = "tab file" + (d.on ? " on" : "");
    b.innerHTML = `${icon("terminal", 14)}<span></span>`;
    b.children[1].textContent = d.label;
    b.title = d.label;
    b.addEventListener("click", () => dockbar.select(d.kind));
    const x = document.createElement("span");
    x.className = "tabx ico sm";
    x.innerHTML = icon("x", 12);
    x.title = t("dock.closeTerm");
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      dockbar.closeTab(d.kind);
    });
    b.append(x);
    bar.append(b);
  }
}

/// Share file tabs between workspaces and projects because their state does not depend on branch or conversation identity.
function appendFileTabs(bar: HTMLElement, fs: Files) {
  for (const path of fs.open) {
    const b = document.createElement("button");
    b.className = "tab file" + (path === fs.active ? " on" : "");
    b.innerHTML = `${icon("file", 14)}<span></span><span class="tabx ico sm">${icon("x", 12)}</span>`;
    b.children[1].textContent = path.slice(path.lastIndexOf("/") + 1);
    b.title = path;
    b.addEventListener("click", () => openFile(path));
    b.querySelector(".tabx")!.addEventListener("click", (e) => {
      e.stopPropagation();
      closeFile(path);
    });
    bar.append(b);
  }
}

/// New-tab model selection can cross providers; changing an existing conversation's model stays within its resumable provider.
function pickModel(at: HTMLElement, ws: Workspace) {
  openModelPicker(at, {
    current: { agent: ws.agent, model: ws.model },
    select: choice => {
      const effort = fitsEffort(choice.model, ws.effort, choice.agent);
      if (effort !== ws.effort) ctx.say(t("models.effortAdjusted"));
      void newTab("", { ...choice, effort });
    },
    terminal: dockbar.newTerm,
  });
}

/// Resolve the current tab label when renaming because selection may have rebuilt the original button.
function editTab(tabId: string) {
  const ws = current();
  const tab = ws?.tabs.find((t) => t.id === tabId);
  const b = $("tabbar").querySelector<HTMLElement>(`.tab[data-tab="${CSS.escape(tabId)}"]`);
  if (!ws || !tab || !b) return;
  rename.start(
    b.children[1] as HTMLElement,
    tab.title,
    (title) => {
      ctx.redraw();
      if (title) {
        invoke("rename_tab", { workspace: ws.id, tab: tabId, title }).catch((e) =>
          ctx.say(fromBack(e), true),
        );
      }
    },
    "tab",
  );
}

/// Group conversation actions in the context menu rather than adding a button for every action.
function tabMenu(ws: Workspace, tab: Tab): menu.Item[] {
  const items: menu.Item[] = [
    { label: t("ws.menu.rename"), glyph: icon("pencil"), run: () => editTab(tab.id) },
  ];
  // Keep at least one conversation in each workspace.
  if (ws.tabs.length > 1) {
    items.push("sep", {
      label: t("tab.close"),
      glyph: icon("x"),
      danger: true,
      run: () => invoke("close_tab", { workspace: ws.id, tab: tab.id }),
    });
  }
  return items;
}

async function selectTab(workspace: string, tab: string) {
  const fs = files(workspace);
  // Selecting the already-active tab preserves its node so double-click rename remains possible.
  if (tab === session.currentSession() && !fs.diff && !fs.active && !dockbar.front()) return;
  const remote = team.isRemote(workspace);
  if (!remote) invoke("focus_tab", { workspace, tab });
  if (!fs.web) showTerm();
  const epoch = navigation;
  if (!(await session.attach(tab, remote ? workspace : undefined))) return;
  if (!stillHere(epoch, workspace)) return;
  ctx.redraw();
}

/// Explicit choice comes from the plus menu; other new-tab paths inherit workspace defaults.
export async function newTab(prompt = "", choice: Choice | null = null) {
  const ws = current();
  if (!ws || ws.remote) return;
  const epoch = navigation;
  try {
    const tab = await invoke("new_tab", { workspace: ws.id, prompt, choice });
    if (!stillHere(epoch, ws.id)) return;
    if (!files(ws.id).web) showTerm();
    if (!(await session.attach(tab.id))) return;
    if (!stillHere(epoch, ws.id)) return;
    ctx.redraw();
  } catch (err) {
    ctx.say(fromBack(err), true);
  }
}

/* Open files. */

/// Keep center selection per workspace in memory. Restarting the app returns to conversation view.
type Files = {
  open: string[];
  active: string | null;
  diff: boolean;
  /// The Changes tab exists only after explicit opening through the side panel or Review.
  diffTab: boolean;
  /// webTab records tab existence; web records center selection.
  web: boolean;
  webTab: boolean;
  port: number;
};
const filesOf = new Map<string, Files>();

function files(id: string): Files {
  let f = filesOf.get(id);
  if (!f) filesOf.set(id, (f = { open: [], active: null, diff: false, diffTab: false, web: false, webTab: false, port: 0 }));
  return f;
}

/// Prune workspace-owned presentation caches when their workspaces disappear.
export function forget(alive: Set<string>) {
  for (const [id, f] of filesOf) if (!alive.has(id) && f.webTab) browser.close(id);
  diff.pruneSeen(alive);
  changesUi.forget(alive);
  for (const map of [filesOf, branchOf] as Map<string, unknown>[]) {
    for (const id of map.keys()) if (!alive.has(id)) map.delete(id);
  }
}

/// Resolve what the file tree cannot know by itself. The project view shares the tree but has no
/// conversation, so there an attachment has no destination at all.
function treeHost(): tree.Host {
  const host = fileHost(proj ? proj.path : (current()?.worktree ?? null), (path) => path);
  return { root: host.root, attachHint: host.attachHint, ...host.hooks };
}

/// The same resolution for a changed file, whose path is relative to its repository. The system
/// file manager addresses the workspace, so reveal maps the path back like `openRepoFile`.
function changesHost(repo: string): fileMenu.Context {
  const ws = current();
  return fileHost(ws ? repoRoot(ws, repo) : null, (path) => (ws ? repoPath(ws, repo, path) : path));
}

function fileHost(base: string | null, relative: (path: string) => string): fileMenu.Context {
  const ws = current();
  const target = session.fileDropTarget();
  return {
    root: base,
    attachHint: !target && ws ? t("file.menu.attach.unsupported") : undefined,
    hooks: {
      // The composer shortens the absolute path back against the worktree when it sends the message.
      attach: target ? (absolute) => target.put([absolute]) : null,
      // Announce the copy only once the clipboard accepted it; a refused write must not read as done.
      copy: (text) => {
        void navigator.clipboard
          .writeText(text)
          .then(() => ctx.say(t("say.copied", { path: text })))
          .catch((e) => ctx.say(fromBack(e), true));
      },
      reveal: (path) => {
        const id = root();
        if (id) void invoke("reveal_path", { id, rel: relative(path) }).catch((e) => ctx.say(fromBack(e), true));
      },
    },
  };
}

export async function openFile(path: string) {
  if (proj) return openProjectFile(path);
  const ws = current();
  if (!ws) return;
  const fs = files(ws.id);
  if (!fs.open.includes(path)) fs.open.push(path);
  fs.active = path;
  await showFile();
  drawTabs(ws);
}

/// Command-P lists the open workspace's or project's files by name. Remote shares have no local
/// index, so the shortcut does nothing for them.
export function quickOpen() {
  const id = root();
  if (!id || (openWs && team.isRemote(openWs))) return;
  // Open tabs, most recent last, rank first among equal matches.
  openQuickOpen($("tabbar"), id, [...files(id).open].reverse(), (path) => void openFile(path));
}

async function showFile() {
  const ws = current();
  const path = ws && files(ws.id).active;
  if (!ws || !path) return showTerm();
  files(ws.id).diff = false;
  leaveWeb(files(ws.id));
  center("viewer");
  await viewer.show(ws.id, path);
}

function showTerm() {
  if (proj) return void showProjectFile();
  const ws = current();
  if (ws) {
    files(ws.id).active = null;
    files(ws.id).diff = false;
    leaveWeb(files(ws.id));
  }
  center("chatwrap");
}

/// dockbar chooses the shell; this module activates its center view.
function showShell() {
  if (proj) {
    files(proj.id).active = null;
    $("offline").hidden = true;
    center("termview");
    drawProjectTabs();
    return;
  }
  const ws = current();
  if (!ws) return;
  const fs = files(ws.id);
  fs.active = null;
  fs.diff = false;
  leaveWeb(fs);
  center("termview");
  drawTabs(ws);
}

/* Browser. */

/// Invalidate pending preview opens even when the user returns to the same workspace and view.
let webRequest = 0;
let sideBeforeBrowser: boolean | null = null;

function restoreBrowserSide() {
  if (sideBeforeBrowser === null) return;
  document.body.classList.toggle("noside", sideBeforeBrowser);
  sideBeforeBrowser = null;
}

/// Give the conversation and preview the center width while retaining the previous panel state.
export async function showWeb() {
  const ws = current();
  if (!ws || ws.remote || ws.cleaned || pending(ws)) return;
  const tab = ws.tabs.find((tab) => tab.id === session.currentSession())
    ?? ws.tabs.find((tab) => tab.id === ws.active)
    ?? ws.tabs[0];
  if (!tab) return;
  const epoch = navigation;
  const request = ++webRequest;
  const fs = files(ws.id);
  if (sideBeforeBrowser === null) {
    sideBeforeBrowser = document.body.classList.contains("noside");
    document.body.classList.add("noside");
  }
  fs.active = null;
  fs.diff = false;
  fs.web = true;
  fs.webTab = true;
  // Attachment clears the previous transcript before its asynchronous snapshot loads.
  const attached = tab.id === session.currentSession() ? undefined : session.attach(tab.id);
  center("webview");
  try {
    const [port] = await Promise.all([browser.show(ws.id), attached]);
    if (!stillHere(epoch, ws.id) || request !== webRequest || !fs.web) return;
    fs.port = port;
  } catch (err) {
    if (!stillHere(epoch, ws.id) || request !== webRequest || !fs.web) return;
    ctx.say(fromBack(err), true);
    fs.webTab = false;
    showTerm();
  }
  drawTabs(ws);
}

/// Hide the native page and invalidate any pending preview activation.
function leaveWeb(fs: Files) {
  if (!fs.web) return;
  webRequest++;
  fs.web = false;
  browser.hide();
}

/// Closing the preview preserves the attached conversation and its draft.
function closeWeb() {
  const ws = current();
  if (!ws) return;
  const fs = files(ws.id);
  const wasOpen = fs.web;
  fs.webTab = false;
  if (wasOpen) showTerm();
  browser.close(ws.id);
  drawTabs(ws);
}

/// Git presentation owns selection; workspace.ts owns the tab lifecycle.
function showChanges() { changesUi.show(); }

function activateChanges() {
  const ws = current();
  if (!ws || !diffable(ws)) return;
  const fs = files(ws.id);
  fs.active = null;
  fs.diff = true;
  fs.diffTab = true;
  leaveWeb(fs);
  center("diffview");
  setSidePane("diff");
  drawTabs(ws);
}

/// Closing Changes removes its tab until another explicit open through the side panel or Review.
async function closeChanges() {
  const ws = current();
  if (!ws) return;
  const fs = files(ws.id);
  const wasOpen = fs.diff;
  fs.diffTab = false;
  // Retain diff selection until showTerm lets selectTab recognize the view change.
  if (wasOpen) {
    const tab = session.currentSession();
    if (tab) await selectTab(ws.id, tab);
    else showTerm();
  }
  drawTabs(ws);
}

/// The preview shares the center with the conversation; other views replace both.
function center(show: "chatwrap" | "viewer" | "diffview" | "webview" | "termview") {
  if (show !== "webview") restoreBrowserSide();
  // Every route to the viewer, and every route away from it, passes here; the tree marks what it shows.
  const id = root();
  tree.select(show === "viewer" && id ? files(id).active : null);
  for (const id of ["chatwrap", "viewer", "diffview", "webview", "termview"] as const) $(id).hidden = id !== show;
  $("tabbody").classList.toggle("browser", show === "webview");
  $("websplit").hidden = show !== "webview";
  if (show === "webview") $("chatwrap").hidden = false;
  if (show !== "termview") dockbar.leave();
}

/// Keep file tabs and unsaved drafts on entries the tree renamed, and drop the ones it trashed.
/// `id` is the workspace or project the action ran in, which may no longer be the one on screen;
/// its tabs then change in memory only and show when it returns.
async function treeMoved(id: string, from: string, to: string | null) {
  viewer.moveDrafts(id, from, to);
  const fs = files(id);
  const was = fs.active;
  const shown = root() === id;
  if (to === null && shown) {
    // Closing picks the neighbor or returns to the conversation, as the tab's own close does.
    for (const path of fs.open.filter((path) => relocate(path, from, null) === null)) await closeFile(path);
    return;
  }
  Object.assign(fs, relocateTabs(fs, from, to));
  if (!shown) return;
  // The viewer reopens the file under its new name, keeping the draft, and the tree marks it.
  if (fs.active !== was && fs.active) return openFile(fs.active);
  if (proj) drawProjectTabs();
  else {
    const ws = current();
    if (ws) drawTabs(ws);
  }
}

async function closeFile(path: string) {
  if (proj) return closeProjectFile(path);
  const ws = current();
  if (!ws) return;
  const fs = files(ws.id);
  const at = fs.open.indexOf(path);
  if (at !== -1) fs.open.splice(at, 1);
  if (fs.active === path) {
    // Select a neighboring file or return to the conversation.
    const next = fs.open[at] ?? fs.open[at - 1];
    if (next) return openFile(next);
    const tab = session.currentSession();
    tab ? await selectTab(ws.id, tab) : showTerm();
  }
  drawTabs(ws);
}

/// Command-W closes eligible center content through the appropriate lifecycle.
export function closeActive(): boolean {
  const fs = openWs && files(openWs);
  if (!fs) return false;
  if (fs.web) {
    closeWeb();
    return true;
  }
  if (fs.active) {
    closeFile(fs.active);
    return true;
  }
  if (fs.diff) {
    closeChanges();
    return true;
  }
  return false;
}

/* Changes. */

const total = changesUi.count;
let changesTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleChanges() {
  if (changesTimer) clearTimeout(changesTimer);
  const interval = changesInterval(background.currentOrDocument());
  if (interval === null) return;
  changesTimer = setTimeout(() => {
    if (hasDiff() && (sidePane === "diff" || files(openWs!).diff)) reloadChanges(openWs!);
    scheduleChanges();
  }, interval);
}
const reloadChanges = debounce(250, (id: string) => void loadChanges(id));
let request = 0;

async function loadChanges(id: string) {
  if (!background.foreground(background.currentOrDocument())) return;
  const mine = ++request;
  try {
    const repos = await invoke("workspace_git_status", { id });
    if (mine !== request || openWs !== id) return;
    changesUi.update(id, repos);
    const n = total(id);
    $("review").hidden = !repos.some(repo => repo.has_head);
    $("diffcount").textContent = n ? String(n) : "";
    $("diffcount").classList.remove("fresh");
    const ws = current();
    if (ws?.id !== id) return;
    drawTabs(ws);
    paintPr(ws);
  } catch (error) {
    if (mine === request && openWs === id) changesUi.fail(id, error);
  }
}

export function fileSaved(id: string) {
  const ws = current();
  if (ws?.id === id && diffable(ws) && background.foreground(background.currentOrDocument())) {
    reloadChanges(id);
    tree.refreshMarksSoon();
  }
}

function outstanding(id: string): { dirty: number; unpushed: number } | null {
  const repos = changesUi.statuses(id);
  if (!repos || repos.some(repo => repo.error)) return null;
  return {
    dirty: repos.reduce((n, repo) => n + new Set([...repo.staged, ...repo.changes, ...repo.conflicts].map(file => file.path)).size, 0),
    unpushed: repos.reduce((n, repo) => n + (repo.upstream ? repo.ahead : Number(repo.has_head)), 0),
  };
}

/// Open a repository-relative path (see `repoPath`); the result is the string list_dir gives that
/// file's row, so the tree can mark it.
async function openRepoFile(repo: string | undefined, path: string) {
  const ws = current();
  if (ws) await openFile(repoPath(ws, repo, path));
}

const repoRoot = (ws: Workspace, repo: string) => ws.repos.find((r) => r.name === repo)?.worktree ?? ws.worktree;

/* Project-only view. */

/// Open clone files directly with the shared tree/viewer and project ID resolution. This mode has no agent conversation, worktree, PR diff, or fixed script dock.
let proj: Project | null = null;

export function openProject(project: Project) {
  leave();
  proj = project;
  tree.reset();
  $("wsView").hidden = false;
  const crumb = $("crumb");
  crumb.dataset.workspace = "";
  crumb.innerHTML = `${avatar(project.name)}<span class="who"></span>`;
  crumb.querySelector<HTMLElement>(".who")!.textContent = project.name;
  // Project-only mode keeps file controls and tabs while hiding conversation, changes, and comments.
  layout({ tabs: true, side: true, files: true });
  $("tab-comments").hidden = true;
  dockbar.reset(false);
  document.body.classList.remove("noside");
  setSidePane("files");
  drawProjectTabs();
  void showProjectFile();
}

/// Project tabs contain shells and files. Without agent conversations, the primary creation action opens a shell.
function drawProjectTabs() {
  if (!proj || deferTabs()) return;
  const bar = $("tabbar");
  bar.replaceChildren();
  appendTermTabs(bar);
  appendFileTabs(bar, files(proj.id));
  // Reuse the workspace creation control with project-specific shell and workspace actions.
  appendTabAdd(bar, {
    title: t("dock.new"),
    add: () => dockbar.newTerm(),
    pickTitle: t("project.new"),
    pick: (at) => {
      const box = at.getBoundingClientRect();
      menu.openAt({ x: box.left - 40, y: box.bottom + 4 }, [
        { label: t("dock.new"), glyph: icon("terminal", 14), run: () => dockbar.newTerm() },
        { label: t("project.newWorkspace"), glyph: icon("plus", 14), run: () => ctx.newWorkspace(proj!.id) },
      ]);
    },
  });
}

/// Keep the center empty until a file is selected, reusing the full-size empty panel without explanatory text.
function showProjectEmpty() {
  if (proj) files(proj.id).active = null;
  center("chatwrap");
  $("offline").hidden = false;
  $("offwave").hidden = true;
  $("offtitle").textContent = "";
  $("offbody").hidden = true;
  $("offpath").textContent = "";
}

async function openProjectFile(path: string) {
  if (!proj) return;
  const fs = files(proj.id);
  if (!fs.open.includes(path)) fs.open.push(path);
  fs.active = path;
  await showProjectFile();
  drawProjectTabs();
}

async function showProjectFile() {
  const path = proj && files(proj.id).active;
  if (!proj || !path) return showProjectEmpty();
  $("offline").hidden = true;
  center("viewer");
  await viewer.show(proj.id, path);
}

/// Closing the last project tab restores empty center content because no conversation exists.
async function closeProjectFile(path: string) {
  if (!proj) return;
  const fs = files(proj.id);
  const at = fs.open.indexOf(path);
  if (at !== -1) fs.open.splice(at, 1);
  if (fs.active === path) {
    fs.active = fs.open[at] ?? fs.open[at - 1] ?? null;
    await showProjectFile();
  }
  drawProjectTabs();
}

/* Side panel. */

type Pane = "files" | "diff" | "comments";
let sidePane: Pane = "files";

function setSidePane(pane: Pane) {
  const changed = sidePane !== pane;
  sidePane = pane;
  $("tab-files").classList.toggle("on", pane === "files");
  $("tab-diff").classList.toggle("on", pane === "diff");
  $("tab-comments").classList.toggle("on", pane === "comments");
  $("tree").hidden = pane !== "files";
  $("difflist").hidden = pane !== "diff";
  $("comments").hidden = pane !== "comments";
  $("side").classList.toggle("comments-open", pane === "comments");
  if (pane === "files") tree.redraw();
  if (changed && pane === "diff" && hasDiff()) reloadChanges(openWs!);
  if (pane === "comments") notes.draw();
}

function openComments() {
  document.body.classList.remove("noside");
  setSidePane("comments");
}

/// Shift-Command-M quotes the selected conversation text in a comment.
export function quoteSelection(): boolean {
  if (!openWs) return false;
  return session.quoteSelection();
}

export const comment = (target: notes.Target) => notes.compose(target);

/// Navigate to a comment from the mention inbox.
export function showNote(id: string) {
  if (!openWs) return;
  showTerm();
  notes.openThread(id);
}
