import { avatar, avatars, brand, icon, stageIcon, wave } from "./icons";
import { stage as stageName, t, tn } from "./i18n";
import * as menu from "./menu";
import { keys } from "./platform";
import { isDelegatedWorkspace, workspaceHierarchy } from "./workspace-hierarchy";
import { button, confirmDialog } from "./ui";
import * as team from "./team";
import * as terminals from "./terminals";
import { accountButton } from "./cloud";
import * as rename from "./rename";
import { issueSearch } from "./issues-controls";
import { hasWorktree, label, projectMatches, refillOrder, repoLabel, stateLabel, statusOf, tabLabel, type Board, type Project, type Status, type TerminalSession, type Workspace } from "./types";
import { h, template } from "./util";

/// Actions shared by sidebar rows and workspace menus.
export type Hooks = {
  /// GitHub activity and comments that mention the current user.
  toNotifications: () => void;
  /// Unread GitHub notifications plus pending mentions.
  notifications: () => number;
  open: (ws: Workspace, tab?: string) => void;
  activeTab: () => string | null;
  setStage: (id: string, stage: string) => void;
  /// Remove the workspace reference while keeping its worktree and branch.
  drop: (id: string) => void;
  /// Null title cancels editing and restores the row.
  rename: (id: string, title: string | null) => void;
  archive: (id: string, archived: boolean) => void;
  /// Finish moves work to the final stage and archives it in one action.
  finish: (id: string) => void;
  /// Open worktree disk cleanup.
  cleanup: () => void;
  pin: (id: string, pinned: boolean) => void;
  unread: (id: string, unread: boolean) => void;
  reveal: (id: string) => void;
  copyPath: (ws: Workspace) => void;
  toDesk: () => void;
  toIssues: () => void;
  toArchived: () => void;
  /// Null issue count means Linear is unavailable and hides the badge.
  issues: () => number | null;
  addProject: () => void;
  removeProject: (id: string) => void;
  /// Persist the project order chosen by dragging headings.
  reorderProjects: (ids: string[]) => void;
  /// Move projects into a folder, rename it with all its members, or take them out with null.
  groupProjects: (ids: string[], group: string | null) => void;
  projectTools: (id: string) => void;
  /// Open clone files without creating a workspace.
  openProject: (id: string) => void;
  newWorkspace: (projectId?: string) => void;
  terminals: TerminalHooks;
};

/// Terminal sessions are shells owned by the app, listed apart from projects and workspaces (ADR 0096).
export type TerminalHooks = {
  /// False where the host has no terminal sessions, which hides the group.
  available: () => boolean;
  /// Whether the session's shell is running.
  live: (id: string) => boolean;
  open: (id: string) => void;
  create: () => void;
  createIn: () => void;
  /// Null returns the session to its folder's name.
  rename: (id: string, name: string | null) => void;
  close: (id: string) => void;
};

/// The desk page uses a sentinel that cannot collide with a workspace ID.
export const DESK = "@mesa";
/// The issues page follows the same sentinel rule.
export const ISSUES = "@issues";
/// The notifications page follows the same sentinel rule.
export const NOTIFICATIONS = "@notificacoes";
/// The archive page follows the same sentinel rule.
export const ARCHIVED = "@arquivados";

let openId: string | null = null;
export function setOpen(id: string | null) {
  openId = id;
}

/// A board update during a project drag would replace the box being dragged; hold it until the drop.
let held: (() => void) | null = null;
let dragging = false;
/// Heading moved by keyboard; it regains focus after the redraw only if it still holds focus then.
let refocus: { id: string; head: HTMLElement } | null = null;

export function render(board: Board, hooks: Hooks) {
  if (dragging) held = () => renderRail(board, hooks);
  else renderRail(board, hooks);
}

const el = (id: string) => document.getElementById(id)!;

/* Rename and context menu. */

/// Keep workspace actions together, including stage changes without opening the conversation.
function wsMenu(ws: Workspace, board: Board, hooks: Hooks, label: HTMLElement, kind: string): menu.Item[] {
  const total = board.stages.length;
  const at = board.stages.indexOf(ws.stage);
  return [
    ws.unread
      ? { label: t("ws.menu.read"), track: "ws.menu.read", glyph: icon("mail-open"), run: () => hooks.unread(ws.id, false) }
      : { label: t("ws.menu.unread"), track: "ws.menu.unread", glyph: icon("mail"), run: () => hooks.unread(ws.id, true) },
    ws.pinned
      ? { label: t("ws.menu.unpin"), track: "ws.menu.unpin", glyph: icon("pin-off"), run: () => hooks.pin(ws.id, false) }
      : { label: t("ws.menu.pin"), track: "ws.menu.pin", glyph: icon("pin"), run: () => hooks.pin(ws.id, true) },
    {
      label: t("ws.menu.stage"), track: "ws.menu.stage",
      glyph: stageIcon(at, total),
      sub: board.stages.map((name, i) => ({
        label: stageName(name), track: "workspace.stage.select",
        glyph: stageIcon(i, total),
        checked: name === ws.stage,
        run: () => hooks.setStage(ws.id, name),
      })),
    },
    {
      label: t("ws.menu.rename"), track: "ws.menu.rename",
      glyph: icon("pencil"),
      run: () => rename.start(label, ws.title, (title) => hooks.rename(ws.id, title), kind),
    },
    ...(!ws.remote ? [{ label: t("tools.project"), track: "tools.project", glyph: icon("plug"), run: () => hooks.projectTools(ws.id) } as menu.Item] : []),
    { label: t("ws.menu.copyPath"), track: "ws.menu.copyPath", glyph: icon("copy"), run: () => hooks.copyPath(ws) },
    { label: t("ws.menu.reveal"), track: "ws.menu.reveal", glyph: icon("external-link"), run: () => hooks.reveal(ws.id) },
    "sep",
    // Archived workspaces have already completed the finish action.
    ...(ws.archived
      ? []
      : [{ label: t("ws.menu.finish"), track: "ws.menu.finish", glyph: icon("check"), run: () => hooks.finish(ws.id) } as menu.Item]),
    ...(hasWorktree(ws)
      ? [{ label: t("ws.menu.cleanup"), track: "ws.menu.cleanup", glyph: icon("trash"), run: () => hooks.cleanup() } as menu.Item]
      : []),
    ws.archived
      ? {
          label: t("ws.menu.unarchive"), track: "ws.menu.unarchive",
          glyph: icon("archive-restore"),
          // Removed worktrees cannot be restored; keep the history card and explain why restoration is unavailable.
          disabled: ws.cleaned,
          hint: ws.cleaned ? t("ws.menu.gone") : undefined,
          run: () => hooks.archive(ws.id, false),
        }
      : {
          label: t("ws.menu.archive"), track: "ws.menu.archive",
          glyph: icon("archive"),
          // Show the shortcut only for the active workspace it would affect.
          hint: ws.id === openId ? keys("⌘⇧A") : undefined,
          run: () => hooks.archive(ws.id, true),
        },
    {
      label: t("ws.menu.drop"), track: "ws.menu.drop",
      glyph: icon("x"),
      danger: true,
      run: () => hooks.drop(ws.id),
    },
  ];
}

export function attachMenu(node: HTMLElement, ws: Workspace, board: Board, hooks: Hooks, label: HTMLElement, kind: string) {
  node.dataset.trackContextMenu = "workspace.menu";
  node.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    menu.openAt({ area: "sidebar", x: e.clientX, y: e.clientY }, wsMenu(ws, board, hooks, label, kind));
  });
}

/// Each session shows its name, a wave while its shell runs, and a close action; the folder is in the tooltip.
function renderTerminals(rail: HTMLElement, sessions: TerminalSession[], hooks: TerminalHooks) {
  const sect = template("div", "sect", `<span></span>`);
  sect.children[0].textContent = t("rail.terminals");
  const add = template("button", "ico sm", icon("plus"));
  add.title = t("terminal.new");
  add.dataset.track = "rail.terminal.add";
  add.addEventListener("click", () => {
    const at = add.getBoundingClientRect();
    menu.openAt({ area: "sidebar", x: at.left, y: at.bottom + 4 }, [
      { label: t("terminal.new"), track: "terminal.new", glyph: icon("square-terminal"), hint: keys("⌘⇧T"), run: hooks.create },
      { label: t("terminal.newIn"), track: "terminal.newIn", glyph: icon("folder-open"), run: hooks.createIn },
    ]);
  });
  sect.append(add);
  rail.append(sect);

  for (const session of sessions) {
    const row = template(
      "button",
      "navitem railterminal" + (openId === session.id ? " on" : ""),
      `${icon("square-terminal")}<span class="lbl"></span>`,
    );
    const label = row.querySelector<HTMLElement>(".lbl")!;
    label.textContent = terminals.title(session);
    row.title = session.folder;
    row.dataset.track = "rail.terminal";
    if (hooks.live(session.id)) row.insertAdjacentHTML("beforeend", wave());
    const close = template("span", "tabx ico sm", icon("x", 12));
    close.title = t("terminal.close");
    close.dataset.track = "rail.terminal.close";
    close.addEventListener("click", (e) => {
      e.stopPropagation();
      hooks.close(session.id);
    });
    row.append(close);
    row.addEventListener("click", () => hooks.open(session.id));
    row.dataset.trackContextMenu = "terminal.menu";
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      menu.openAt({ area: "sidebar", x: e.clientX, y: e.clientY }, [
        {
          label: t("terminal.rename"), track: "terminal.rename",
          glyph: icon("pencil"),
          run: () => rename.start(label, terminals.title(session), (name) => {
            if (name !== null) hooks.rename(session.id, name);
          }),
        },
        ...(session.name
          ? [{ label: t("terminal.folderName"), track: "terminal.folderName", glyph: icon("folder"), run: () => hooks.rename(session.id, null) } as menu.Item]
          : []),
        "sep",
        { label: t("terminal.close"), track: "terminal.close", glyph: icon("x"), danger: true, run: () => hooks.close(session.id) },
      ]);
    });
    rail.append(row);
  }
}

/// Another owner's share only offers leaving: every device of this person stops receiving it until the owner lets them
/// back, even while the owner is offline.
function attachLeave(node: HTMLElement, ws: Workspace, owner: string) {
  const leave = async () => {
    if (await confirmDialog({ track: "share.leave.title", area: "sidebar", title: t("share.leave.title"), message: t("share.leave.message", { name: owner }),
      accept: t("share.leave"), cancel: t("actions.cancel") })) await team.leaveShare(ws.id);
  };
  node.dataset.trackContextMenu = "workspace.menu";
  node.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    menu.openAt({ area: "sidebar", x: e.clientX, y: e.clientY }, [{ label: t("share.leave"), track: "share.leave", glyph: icon("x"), danger: true, run: () => void leave() }]);
  });
}

/* Sidebar: creation, issues, and project workspaces. */

/// Persist collapsed groups across visits.
const FOLD = "prometeu:grupo:";
const folded = (name: string) => localStorage.getItem(FOLD + name) === "1";

/// Shorter project lists fit at a glance; longer ones get a filter.
const FILTER_FROM = 7;
/// The filter outlives redraws so agent updates neither clear nor blur what the person is typing.
let finder: ReturnType<typeof issueSearch> | null = null;
let last: [Board, Hooks] | null = null;
/// A folder created from a project menu opens its name for editing once the board shows it.
let naming: string | null = null;

function filterField() {
  if (finder) return finder;
  const f = issueSearch("");
  f.control.dataset.track = "rail.projects.search";
  f.control.addEventListener("input", () => last && renderRail(...last));
  f.control.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || !f.control.value) return;
    e.stopPropagation();
    f.control.value = "";
    if (last) renderRail(...last);
  });
  return (finder = f);
}

/// Pick a folder name not already in use, so a new folder never merges into an existing one.
function untitled(taken: string[]) {
  const base = t("rail.folder.untitled");
  let name = base;
  for (let i = 2; taken.includes(name); i++) name = `${base} ${i}`;
  return name;
}

function renderRail(board: Board, hooks: Hooks) {
  const rail = el("railbody");
  const keep = refocus && document.activeElement === refocus.head ? refocus.id : null;
  refocus = null;
  last = [board, hooks];
  const typing = finder !== null && document.activeElement === finder.control;
  rail.replaceChildren();
  const hierarchy = workspaceHierarchy(board);
  const live = board.workspaces.filter((w) => !w.archived && !w.remote && !hierarchy.parents.has(w.id));

  rail.append(accountButton());

  const create = template("button", "navitem", `${icon("plus")}<span></span>`);
  create.children[1].textContent = t("rail.create");
  create.title = t("rail.create.title");
  create.dataset.track = "rail.create.title";
  create.addEventListener("click", () => hooks.newWorkspace());
  rail.append(create);

  // The desk shows all conversations and is the start page.
  const desk = template("button", "navitem" + (openId === DESK ? " on" : ""), `${icon("terminal")}<span></span>`);
  desk.children[1].textContent = t("rail.desk");
  desk.title = t("rail.desk.title");
  desk.dataset.track = "rail.desk.title";
  desk.addEventListener("click", hooks.toDesk);
  rail.append(desk);

  // Assigned Linear issues are another entry point for work.
  const issues = template(
    "button",
    "navitem" + (openId === ISSUES ? " on" : ""),
    `${icon("inbox")}<span></span><span class="n"></span>`,
  );
  issues.children[1].textContent = t("rail.issues");
  const n = hooks.issues();
  issues.querySelector(".n")!.textContent = n === null ? "" : String(n);
  issues.dataset.track = "rail.issues";
  issues.title = n === null ? t("rail.issues.off") : t("rail.issues.title");
  issues.addEventListener("click", hooks.toIssues);
  rail.append(issues);

  // GitHub activity and comment mentions wait outside individual sessions.
  const waiting = hooks.notifications();
  const notifications = template(
    "button",
    "navitem" + (waiting ? " mentions" : "") + (openId === NOTIFICATIONS ? " on" : ""),
    `${icon("bell")}<span></span><span class="n"></span>`,
  );
  notifications.id = "rail-notifications";
  notifications.dataset.track = "rail.notifications";
  notifications.children[1].textContent = t("notifications.center");
  notifications.querySelector(".n")!.textContent = waiting ? String(waiting) : "";
  notifications.addEventListener("click", hooks.toNotifications);
  rail.append(notifications);
  rail.append(document.createElement("hr"));

  // Pinned workspaces move to the top instead of appearing twice.
  const pinned = live.filter((w) => w.pinned || hierarchy.children.get(w.id)?.some(child => child.pinned));
  if (pinned.length) {
    renderGroup(rail, board, hooks, t("rail.pinned"), icon("pin", 14), pinned, "@fixados", { avatars: true });
  }

  // Remote shares precede local projects because they belong to other owners.
  const shared = board.workspaces.filter((w) => w.remote);
  if (shared.length) {
    renderGroup(rail, board, hooks, t("rail.team"), icon("users", 14), shared, "@time", { avatars: true });
  }

  if (hooks.terminals.available()) renderTerminals(rail, board.terminals ?? [], hooks.terminals);

  const sect = template("div", "sect", `<span></span>`);
  sect.children[0].textContent = t("rail.projects");
  const add = template("button", "ico sm", icon("plus"));
  add.title = t("rail.add");
  add.dataset.track = "rail.add";
  add.addEventListener("click", () => {
    const at = add.getBoundingClientRect();
    const taken = board.projects.flatMap((p) => (p.group ? [p.group] : []));
    // A folder exists only through its members, so a new one starts with the project picked here.
    const start = (id: string) => {
      naming = untitled(taken);
      hooks.groupProjects([id], naming);
    };
    menu.openAt({ area: "sidebar", x: at.left, y: at.bottom + 4 }, [
      { label: t("rail.addProject"), track: "rail.addProject", glyph: icon("folder-plus"), run: hooks.addProject },
      {
        label: t("rail.folder.newWith"), track: "rail.folder.newWith",
        glyph: icon("folder"),
        disabled: !board.projects.length,
        sub: board.projects.map((p) => ({ label: p.name, track: "project.folder.create", glyph: avatar(p.name), run: () => start(p.id) })),
      },
    ]);
  });
  sect.append(add);
  rail.append(sect);

  if (!board.projects.length) {
    rail.append(h("div", "railhint", t("rail.noProjects")));
  }

  let query = "";
  if (board.projects.length >= FILTER_FROM) {
    const f = filterField();
    f.control.placeholder = t("rail.filter");
    f.control.setAttribute("aria-label", t("rail.filter"));
    rail.append(f.root);
    if (typing) f.control.focus();
    query = f.control.value.trim();
  } else if (finder) finder.control.value = "";

  // Project headings disambiguate repositories with matching initials.
  const stageAt = (ws: Workspace) => board.stages.indexOf(ws.stage);
  // Workspaces spanning repositories belong to collection groups.
  const single = live.filter((w) => !pinned.includes(w) && w.repos.length < 2);
  const mineOf = (id: string) => single.filter((w) => w.project === id);
  const familyOf = (workspaces: Workspace[]) => workspaces.flatMap(w => [w, ...(hierarchy.children.get(w.id) ?? [])]);
  const shown = board.projects.filter((p) => projectMatches(p, familyOf(mineOf(p.id)), query));
  // Group multi-repository workspaces by repository set. These transient collections have split avatars but no project creation/menu controls.
  const sets = new Map<string, Workspace[]>();
  for (const w of live) {
    if (pinned.includes(w) || w.repos.length < 2) continue;
    if (!projectMatches({ id: "", name: repoLabel(w), path: "" }, familyOf([w]), query)) continue;
    sets.set(repoLabel(w), [...(sets.get(repoLabel(w)) ?? []), w]);
  }
  if (query && !shown.length && !sets.size) rail.append(h("div", "railhint", t("rail.filter.none")));
  const order = board.projects.map((p) => p.id);
  const folders = [...new Set(board.projects.flatMap((p) => (p.group ? [p.group] : [])))].sort((a, b) => a.localeCompare(b));

  const projectBox = (project: Project) => {
    const mine = mineOf(project.id);
    // Order project workspaces by stage, with ongoing work first.
    mine.sort((a, b) => stageAt(a) - stageAt(b));
    const plus = template("button", "ico sm", icon("plus"));
    // Create directly inside the selected project.
    plus.title = t("rail.newIn", { project: project.name });
    plus.dataset.track = "rail.newIn";
    plus.addEventListener("click", (e) => {
      e.stopPropagation();
      hooks.newWorkspace(project.id);
    });
    const more = template("button", "ico sm", icon("ellipsis"));
    more.title = t("rail.projectActions", { project: project.name });
    more.dataset.track = "rail.projectActions";
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      const at = more.getBoundingClientRect();
      const move: menu.Item[] = [
        ...folders.map((name) => ({
          label: name, track: "project.group.select",
          checked: project.group === name,
          run: () => hooks.groupProjects([project.id], name),
        })),
        ...(folders.length ? ["sep" as const] : []),
        {
          label: t("rail.folder.new"), track: "rail.folder.new",
          glyph: icon("folder-plus"),
          run: () => {
            naming = untitled(folders);
            hooks.groupProjects([project.id], naming);
          },
        },
        ...(project.group
          ? [{ label: t("rail.folder.leave"), track: "rail.folder.leave", glyph: icon("x"), run: () => hooks.groupProjects([project.id], null) }]
          : []),
      ];
      menu.openAt({ area: "sidebar", x: at.left, y: at.bottom + 4 }, [
        { label: t("tools.project"), track: "tools.project", glyph: icon("plug"), run: () => hooks.projectTools(project.id) },
        { label: t("rail.folder.move"), track: "rail.folder.move", glyph: icon("folder"), sub: move },
        {
          label: t("rail.removeProject"), track: "rail.removeProject",
          glyph: icon("x"),
          danger: true,
          run: () => hooks.removeProject(project.id),
        },
      ]);
    });
    // Wrap each project so dragging its heading moves the workspaces along with it.
    const box = h("div", "railproject");
    box.dataset.project = project.id;
    const head = renderGroup(box, board, hooks, project.name, avatar(project.name), mine, `@proj:${project.id}`, {
      extra: [more, plus],
      open: () => hooks.openProject(project.id),
      searching: !!query,
      on: openId === project.id,
    });
    // Hidden projects cannot be placed relative to visible ones, so dragging waits for an empty filter.
    if (!query) reorderable(box, head, order, hooks);
    return box;
  };

  // Folders come first, alphabetically, so dragging within a folder or among loose projects never
  // depends on where a folder sits in the board order.
  for (const name of folders) {
    const members = shown.filter((p) => p.group === name);
    if (!members.length) continue;
    const ids = board.projects.filter((p) => p.group === name).map((p) => p.id);
    const key = `@folder:${name}`;
    // A search shows every match, so folders open while it is active.
    const shut = !query && folded(key);
    const head = template(
      "div",
      "group",
      `<span class="gg">${icon(shut ? "folder" : "folder-open", 14)}</span><span></span><span class="n"></span><span class="gc">${icon(shut ? "chevron-right" : "chevron-down", 14)}</span>`,
    );
    head.tabIndex = 0;
    head.setAttribute("role", "button");
    head.setAttribute("aria-expanded", String(!shut));
    head.dataset.track = "rail.folder";
    const title = head.children[1] as HTMLElement;
    title.textContent = name;
    head.children[2].textContent = String(members.length);
    // A collapsed folder still reports agents that need attention or are working.
    if (shut) {
      const statuses = single.filter((w) => ids.includes(w.project)).map(statusOf);
      const urgent = (["querendo", "rodando"] as const).find((s) => statuses.includes(s));
      if (urgent) head.children[2].before(statusDot(urgent));
    }
    const fold = () => {
      localStorage.setItem(FOLD + key, shut ? "0" : "1");
      renderRail(board, hooks);
    };
    head.addEventListener("click", fold);
    head.addEventListener("keydown", (e) => {
      if (e.target === head && (e.key === "Enter" || e.key === " ")) {
        e.preventDefault();
        fold();
      }
    });
    const renameFolder = () => rename.start(title, name, (next) => next && hooks.groupProjects(ids, next), "sub");
    const more = template("button", "ico sm", icon("ellipsis"));
    more.title = t("rail.folder.actions", { folder: name });
    more.dataset.track = "rail.folder.actions";
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      const at = more.getBoundingClientRect();
      menu.openAt({ area: "sidebar", x: at.left, y: at.bottom + 4 }, [
        { label: t("rail.folder.rename"), track: "rail.folder.rename", glyph: icon("pencil"), run: renameFolder },
        { label: t("rail.folder.ungroup"), track: "rail.folder.ungroup", glyph: icon("x"), run: () => hooks.groupProjects(ids, null) },
      ]);
    });
    head.append(more);
    const body = h("div", "railfolder-body");
    rail.append(head, body);
    if (!shut) for (const project of members) body.append(projectBox(project));
    if (naming === name) {
      naming = null;
      renameFolder();
    }
  }
  for (const project of shown) if (!project.group) rail.append(projectBox(project));

  if (sets.size) {
    rail.append(document.createElement("hr"));
    const head = template("div", "sect", `<span></span>`);
    head.children[0].textContent = t("rail.sets");
    rail.append(head);
    for (const [name, mine] of sets) {
      mine.sort((a, b) => stageAt(a) - stageAt(b));
      renderGroup(rail, board, hooks, name, avatars(mine[0].repos.map((r) => r.name)), mine, `@set:${name}`, { searching: !!query });
    }
  }

  // Keep workspaces visible if their project registration disappears.
  const known = new Set(board.projects.map((p) => p.id));
  const loose = single.filter((w) => !known.has(w.project));
  if (loose.length) {
    renderGroup(rail, board, hooks, t("rail.loose"), icon("folder", 14), loose, "@soltos", { avatars: true });
  }

  // Represent archived work with a count and separate page so it does not crowd active projects out of the sidebar.
  const gone = board.workspaces.filter((w) => w.archived).length;
  if (gone) {
    rail.append(document.createElement("hr"));
    const arch = template(
      "button",
      "navitem" + (openId === ARCHIVED ? " on" : ""),
      `${icon("archive")}<span></span><span class="n"></span>`,
    );
    arch.children[1].textContent = t("rail.archived");
    arch.querySelector(".n")!.textContent = String(gone);
    arch.title = t("rail.archived.title");
    arch.dataset.track = "rail.archived.title";
    arch.addEventListener("click", hooks.toArchived);
    rail.append(arch);
  }

  if (keep) rail.querySelector<HTMLElement>(`.railproject[data-project="${CSS.escape(keep)}"] > .group`)?.focus();
}

function renderGroup(
  rail: HTMLElement,
  board: Board,
  hooks: Hooks,
  name: string,
  glyph: string,
  list: Workspace[],
  /// Persist collapse state using stable keys independent of translated labels.
  key = name,
  opts: {
    /// Mixed-repository groups retain avatars to identify each workspace's project.
    avatars?: boolean;
    /// Header controls for project actions and creation.
    extra?: HTMLElement[];
    /// Project headings open clone files; the chevron separately controls collapse.
    open?: () => void;
    /// Mark the heading when its project page is open.
    on?: boolean;
    /// A search opens the group to show its matches without changing the saved collapse state.
    searching?: boolean;
  } = {},
): HTMLElement {
  const hierarchy = workspaceHierarchy(board);
  const count = list.reduce((n, ws) => n + 1 + (hierarchy.children.get(ws.id)?.length ?? 0), 0);
  const shut = !opts.searching && folded(key);
  // Use a focusable div because project controls cannot be nested inside another button.
  const head = template(
    "div",
    "group",
    `<span class="gg">${glyph}</span><span></span><span class="n"></span><span class="gc"></span>`,
  );
  head.tabIndex = 0;
  head.setAttribute("role", "button");
  head.dataset.track = "rail.group";
  if (opts.on) head.classList.add("on");
  head.setAttribute("aria-expanded", String(!shut));
  head.children[1].textContent = name;
  head.children[2].textContent = count ? String(count) : "";
  // Empty groups do not collapse; their heading only offers creation.
  head.children[3].innerHTML = list.length ? icon(shut ? "chevron-right" : "chevron-down", 14) : "";
  const fold = () => {
    if (!list.length || opts.searching) return;
    localStorage.setItem(FOLD + key, shut ? "0" : "1");
    renderRail(board, hooks);
  };
  const act = opts.open ?? fold;
  head.addEventListener("click", act);
  head.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      act();
    }
  });
  // The chevron collapses groups whose heading opens a page.
  if (opts.open) {
    (head.children[3] as HTMLElement).dataset.track = "rail.group.fold";
    head.children[3].addEventListener("click", (e) => {
      e.stopPropagation();
      fold();
    });
  }
  if (opts.extra) head.append(...opts.extra);
  rail.append(head);
  if (shut) return head;

  const renderWorkspace = (ws: Workspace) => {
    const compact = !ws.remote && isDelegatedWorkspace(board, ws.id) && ws.tabs.length <= 1;
    const provider = ws.tabs[0]?.choice?.agent ?? ws.agent;
    const card = h("div", "railworkspace" + (ws.id === openId ? " on" : ""));
    card.dataset.workspace = ws.id;
    card.classList.toggle("subworkspace", compact);
    const owner = ws.remote ? team.nameOf(ws.remote.owner) : null;
    const status = ws.remote && !ws.remote.online ? "desligada" : statusOf(ws);
    const b = template(
      "button",
      "navitem sub" + (ws.unread ? " unread" : ""),
      `<span class="wsidentity"><span class="lbl"></span><span class="wsbranch"></span></span>`,
    );
    b.prepend(statusDot(ws.preparing ? "rodando" : status, ws.remote ? label(status) : stateLabel(ws)));
    if (ws.failed) b.querySelector(".rail-status")!.classList.add("failed");
    const title = b.querySelector<HTMLElement>(".lbl")!;
    title.textContent = ws.title;
    b.querySelector(".wsbranch")!.textContent = ws.branch;
    if (compact) {
      b.querySelector(".wsbranch")!.remove();
      b.children[0].after(template("span", "provider", brand(provider, 15)));
    }
    b.title = [ws.title, owner, repoLabel(ws), ws.branch, ...(compact && ws.tabs[0] ? [t(`model.${provider}`), tabLabel(ws, ws.tabs[0]), label(status)] : [])].filter(Boolean).join(" · ");
    if (compact) {
      b.setAttribute("aria-label", [ws.title, t(`model.${provider}`), label(status)].join(" · "));
      if (ws.id === openId) b.setAttribute("aria-current", "page");
    }
    b.dataset.track = "rail.workspace";
    b.addEventListener("click", () => hooks.open(ws, compact ? ws.tabs[0]?.id : undefined));
    if (!compact && (owner || opts.avatars)) b.children[0].after(template("span", "av", owner ? avatar(owner) : avatars(ws.repos.map((r) => r.name))));
    if (ws.remote && !ws.remote.online) card.classList.add("off");
    if (!ws.remote) attachMenu(b, ws, board, hooks, title, "sub");
    else attachLeave(b, ws, owner!);
    card.append(b);

    if (ws.tabs.length && !compact) {
      const key = `@ws:${ws.id}`;
      // Display a single agent directly without a redundant collapsible group row.
      const shut = ws.tabs.length > 1 && folded(key);
      const toggle = template("button", "railagents-toggle", `<span></span>${icon(shut ? "chevron-right" : "chevron-down", 12)}`);
      toggle.children[0].textContent = tn(ws.tabs.length, "rail.agents");
      toggle.dataset.track = "rail.agents";
      toggle.setAttribute("aria-expanded", String(!shut));
      toggle.setAttribute("aria-controls", `railagents-${ws.id}`);
      const agents = h("div", "railagents");
      agents.id = `railagents-${ws.id}`;
      agents.hidden = shut;
      toggle.addEventListener("click", () => {
        agents.hidden = !agents.hidden;
        localStorage.setItem(FOLD + key, agents.hidden ? "1" : "0");
        toggle.setAttribute("aria-expanded", String(!agents.hidden));
        toggle.lastElementChild!.outerHTML = icon(agents.hidden ? "chevron-right" : "chevron-down", 12);
      });
      for (const tab of ws.tabs) {
        const provider = tab.choice?.agent ?? ws.agent;
        const status = ws.remote && !ws.remote.online ? "desligada" : tab.status;
        // Remote identity belongs to the share owner because the relay does not announce the provider. Keep one owner/model line; activity appears in the conversation.
        const agent = template(
          "button",
          "railagent",
          `<span class="provider">${owner ? avatar(owner) : brand(provider, 15)}</span><span class="lbl"></span>`,
        );
        agent.prepend(statusDot(status));
        agent.dataset.tab = tab.id;
        agent.dataset.track = "rail.agent";
        const name = tabLabel(ws, tab);
        agent.querySelector(".lbl")!.textContent = name;
        agent.title = [name, owner ?? t(`model.${provider}`), label(status)].filter(Boolean).join(" · ");
        agent.setAttribute("aria-label", agent.title);
        if (ws.id === openId && tab.id === hooks.activeTab()) {
          agent.classList.add("on");
          agent.setAttribute("aria-current", "true");
        }
        agent.addEventListener("click", () => hooks.open(ws, tab.id));
        agents.append(agent);
      }
      if (ws.tabs.length > 1) card.append(toggle);
      card.append(agents);
    }
    return card;
  };
  for (const ws of list) {
    rail.append(renderWorkspace(ws));
    const children = hierarchy.children.get(ws.id);
    if (!children?.length) continue;
    const key = `@subworkspaces:${ws.id}`;
    const body = h("div", "rail-subworkspaces");
    body.id = `subworkspaces-${ws.id}`;
    body.hidden = !opts.searching && folded(key);
    const toggle = button("", () => {
      body.hidden = !body.hidden;
      localStorage.setItem(FOLD + key, body.hidden ? "1" : "0");
      paint();
    }, "ghost", "rail.subworkspaces");
    toggle.classList.add("rail-subworkspaces-toggle");
    toggle.setAttribute("aria-controls", body.id);
    const paint = () => {
      toggle.setAttribute("aria-expanded", String(!body.hidden));
      toggle.innerHTML = `${icon(body.hidden ? "chevron-right" : "chevron-down", 12)}<span></span>`;
      toggle.lastElementChild!.textContent = tn(children.length, "rail.subworkspaces");
    };
    paint();
    for (const child of children) body.append(renderWorkspace(child));
    rail.append(toggle, body);
  }
  return head;
}

/// Pixels the pointer must travel before a press on a heading becomes a drag instead of a click.
const DRAG_START = 4;

/// Reorder projects by dragging their headings, or with Alt+Arrow on a focused heading. Pointer
/// gestures are used because Tauri intercepts HTML drag events for native file drops.
function reorderable(box: HTMLElement, head: HTMLElement, order: string[], hooks: Hooks) {
  const commit = () => {
    const moved = [...box.parentElement!.querySelectorAll<HTMLElement>(":scope > .railproject")].map((p) => p.dataset.project!);
    hooks.reorderProjects(refillOrder(order, moved));
  };
  let dropped = false;
  head.addEventListener("click", (e) => dropped && e.stopImmediatePropagation(), { capture: true });
  head.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || (e.target as Element).closest("button, .gc")) return;
    const y0 = e.clientY;
    const list = box.parentElement!;
    const home = box.nextSibling;
    let moving = false;
    const move = (m: PointerEvent) => {
      if (!moving) {
        if (Math.abs(m.clientY - y0) < DRAG_START) return;
        moving = dragging = true;
        box.classList.add("dragging");
        document.body.classList.add("reordering");
      }
      const over = document.elementFromPoint(m.clientX, m.clientY)?.closest<HTMLElement>(".railproject");
      if (!over || over === box || over.parentElement !== list) return;
      // Compare with the target's heading, not its whole box, so its workspaces count as "after".
      const at = over.firstElementChild!.getBoundingClientRect();
      if (m.clientY < at.top + at.height / 2) over.before(box);
      else over.after(box);
    };
    const end = (u: PointerEvent) => {
      document.removeEventListener("pointermove", move);
      document.removeEventListener("pointerup", end);
      document.removeEventListener("pointercancel", end);
      if (!moving) return;
      box.classList.remove("dragging");
      document.body.classList.remove("reordering");
      dragging = false;
      const redraw = held;
      held = null;
      if (u.type === "pointercancel") {
        // An interrupted gesture is not a drop: put the project back and apply any held update.
        list.insertBefore(box, home);
        redraw?.();
        return;
      }
      // The release may still fire a click on the heading; swallow it so dropping does not open the project.
      dropped = true;
      setTimeout(() => (dropped = false));
      // The command publishes the board, which redraws the sidebar with every held change.
      commit();
    };
    document.addEventListener("pointermove", move);
    document.addEventListener("pointerup", end);
    document.addEventListener("pointercancel", end);
  });
  head.addEventListener("keydown", (e) => {
    if (e.target !== head || !e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const next = e.key === "ArrowUp" ? box.previousElementSibling : box.nextElementSibling;
    if (!next?.classList.contains("railproject")) return;
    if (e.key === "ArrowUp") next.before(box);
    else next.after(box);
    head.focus();
    refocus = { id: box.dataset.project!, head };
    commit();
  });
}

function statusDot(status: Status, text = label(status)) {
  const dot = h("span", "rail-status");
  dot.dataset.status = status;
  dot.setAttribute("role", "img");
  dot.setAttribute("aria-label", text);
  dot.title = text;
  return dot;
}
