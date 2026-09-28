import { avatar, avatars, brand, icon, stageIcon } from "./icons";
import { stage as stageName, t, tn } from "./i18n";
import * as menu from "./menu";
import { keys } from "./platform";
import * as team from "./team";
import { accountButton } from "./cloud";
import * as rename from "./rename";
import { hasWorktree, label, repoLabel, stateLabel, statusOf, tabLabel, type Board, type Status, type Workspace } from "./types";
import { h, template } from "./util";

/// Actions shared by sidebar rows and workspace menus.
export type Hooks = {
  /// Open comments that mention the current user.
  inbox: () => void;
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
  projectTools: (id: string) => void;
  /// Open clone files without creating a workspace.
  openProject: (id: string) => void;
  newWorkspace: (projectId?: string) => void;
};

/// The desk page uses a sentinel that cannot collide with a workspace ID.
export const DESK = "@mesa";
/// The issues page follows the same sentinel rule.
export const ISSUES = "@issues";
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
      ? { label: t("ws.menu.read"), glyph: icon("mail-open"), run: () => hooks.unread(ws.id, false) }
      : { label: t("ws.menu.unread"), glyph: icon("mail"), run: () => hooks.unread(ws.id, true) },
    ws.pinned
      ? { label: t("ws.menu.unpin"), glyph: icon("pin-off"), run: () => hooks.pin(ws.id, false) }
      : { label: t("ws.menu.pin"), glyph: icon("pin"), run: () => hooks.pin(ws.id, true) },
    {
      label: t("ws.menu.stage"),
      glyph: stageIcon(at, total),
      sub: board.stages.map((name, i) => ({
        label: stageName(name),
        glyph: stageIcon(i, total),
        checked: name === ws.stage,
        run: () => hooks.setStage(ws.id, name),
      })),
    },
    {
      label: t("ws.menu.rename"),
      glyph: icon("pencil"),
      run: () => rename.start(label, ws.title, (title) => hooks.rename(ws.id, title), kind),
    },
    ...(!ws.remote ? [{ label: t("tools.project"), glyph: icon("plug"), run: () => hooks.projectTools(ws.id) } as menu.Item] : []),
    { label: t("ws.menu.copyPath"), glyph: icon("copy"), run: () => hooks.copyPath(ws) },
    { label: t("ws.menu.reveal"), glyph: icon("external-link"), run: () => hooks.reveal(ws.id) },
    "sep",
    // Archived workspaces have already completed the finish action.
    ...(ws.archived
      ? []
      : [{ label: t("ws.menu.finish"), glyph: icon("check"), run: () => hooks.finish(ws.id) } as menu.Item]),
    ...(hasWorktree(ws)
      ? [{ label: t("ws.menu.cleanup"), glyph: icon("trash"), run: () => hooks.cleanup() } as menu.Item]
      : []),
    ws.archived
      ? {
          label: t("ws.menu.unarchive"),
          glyph: icon("archive-restore"),
          // Removed worktrees cannot be restored; keep the history card and explain why restoration is unavailable.
          disabled: ws.cleaned,
          hint: ws.cleaned ? t("ws.menu.gone") : undefined,
          run: () => hooks.archive(ws.id, false),
        }
      : {
          label: t("ws.menu.archive"),
          glyph: icon("archive"),
          // Show the shortcut only for the active workspace it would affect.
          hint: ws.id === openId ? keys("⌘⇧A") : undefined,
          run: () => hooks.archive(ws.id, true),
        },
    {
      label: t("ws.menu.drop"),
      glyph: icon("x"),
      danger: true,
      run: () => hooks.drop(ws.id),
    },
  ];
}

export function attachMenu(node: HTMLElement, ws: Workspace, board: Board, hooks: Hooks, label: HTMLElement, kind: string) {
  node.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    menu.openAt({ x: e.clientX, y: e.clientY }, wsMenu(ws, board, hooks, label, kind));
  });
}

/* Sidebar: creation, issues, and project workspaces. */

/// Persist collapsed groups across visits.
const FOLD = "prometeu:grupo:";
const folded = (name: string) => localStorage.getItem(FOLD + name) === "1";

function renderRail(board: Board, hooks: Hooks) {
  const rail = el("railbody");
  const keep = refocus && document.activeElement === refocus.head ? refocus.id : null;
  refocus = null;
  rail.replaceChildren();
  const live = board.workspaces.filter((w) => !w.archived && !w.remote);

  rail.append(accountButton());

  const create = template("button", "navitem", `${icon("plus")}<span></span>`);
  create.children[1].textContent = t("rail.create");
  create.title = t("rail.create.title");
  create.addEventListener("click", () => hooks.newWorkspace());
  rail.append(create);

  // The desk shows all conversations and is the start page.
  const desk = template("button", "navitem" + (openId === DESK ? " on" : ""), `${icon("terminal")}<span></span>`);
  desk.children[1].textContent = t("rail.desk");
  desk.title = t("rail.desk.title");
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
  issues.title = n === null ? t("rail.issues.off") : t("rail.issues.title");
  issues.addEventListener("click", hooks.toIssues);
  rail.append(issues);

  // Comment mentions expose pending collaboration outside individual sessions.
  const waiting = team.inboxCount();
  if (waiting) {
    const mine = template("button", "navitem mentions", `${icon("at-sign")}<span></span><span class="n"></span>`);
    mine.children[1].textContent = t("inbox.title");
    mine.querySelector(".n")!.textContent = String(waiting);
    mine.addEventListener("click", hooks.inbox);
    rail.append(mine);
  }
  rail.append(document.createElement("hr"));

  // Pinned workspaces move to the top instead of appearing twice.
  const pinned = live.filter((w) => w.pinned);
  if (pinned.length) {
    renderGroup(rail, board, hooks, t("rail.pinned"), icon("pin", 14), pinned, "@fixados", { avatars: true });
  }

  // Remote shares precede local projects because they belong to other owners.
  const shared = board.workspaces.filter((w) => w.remote);
  if (shared.length) {
    renderGroup(rail, board, hooks, t("rail.team"), icon("users", 14), shared, "@time", { avatars: true });
  }

  const sect = template("div", "sect", `<span></span>`);
  sect.children[0].textContent = t("rail.projects");
  const add = template("button", "ico sm", icon("folder-plus"));
  add.title = t("rail.addProject");
  add.addEventListener("click", hooks.addProject);
  sect.append(add);
  rail.append(sect);

  if (!board.projects.length) {
    rail.append(h("div", "railhint", t("rail.noProjects")));
  }

  // Project headings disambiguate repositories with matching initials.
  const stageAt = (ws: Workspace) => board.stages.indexOf(ws.stage);
  // Workspaces spanning repositories belong to collection groups.
  const single = live.filter((w) => !w.pinned && w.repos.length < 2);
  for (const project of board.projects) {
    const mine = single.filter((w) => w.project === project.id);
    // Order project workspaces by stage, with ongoing work first.
    mine.sort((a, b) => stageAt(a) - stageAt(b));
    const plus = template("button", "ico sm", icon("plus"));
    // Create directly inside the selected project.
    plus.title = t("rail.newIn", { project: project.name });
    plus.addEventListener("click", (e) => {
      e.stopPropagation();
      hooks.newWorkspace(project.id);
    });
    const more = template("button", "ico sm", icon("ellipsis"));
    more.title = t("rail.projectActions", { project: project.name });
    more.addEventListener("click", (e) => {
      e.stopPropagation();
      const at = more.getBoundingClientRect();
      menu.openAt({ x: at.left, y: at.bottom + 4 }, [
        { label: t("tools.project"), glyph: icon("plug"), run: () => hooks.projectTools(project.id) },
        {
          label: t("rail.removeProject"),
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
      on: openId === project.id,
    });
    reorderable(box, head, hooks);
    rail.append(box);
  }

  // Group multi-repository workspaces by repository set. These transient collections have split avatars but no project creation/menu controls.
  const sets = new Map<string, Workspace[]>();
  for (const w of live) {
    if (w.pinned || w.repos.length < 2) continue;
    sets.set(repoLabel(w), [...(sets.get(repoLabel(w)) ?? []), w]);
  }
  if (sets.size) {
    rail.append(document.createElement("hr"));
    const head = template("div", "sect", `<span></span>`);
    head.children[0].textContent = t("rail.sets");
    rail.append(head);
    for (const [name, mine] of sets) {
      mine.sort((a, b) => stageAt(a) - stageAt(b));
      renderGroup(rail, board, hooks, name, avatars(mine[0].repos.map((r) => r.name)), mine, `@set:${name}`);
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
  } = {},
): HTMLElement {
  const shut = folded(key);
  // Use a focusable div because project controls cannot be nested inside another button.
  const head = template(
    "div",
    "group",
    `<span class="gg">${glyph}</span><span></span><span class="n"></span><span class="gc"></span>`,
  );
  head.tabIndex = 0;
  head.setAttribute("role", "button");
  if (opts.on) head.classList.add("on");
  head.setAttribute("aria-expanded", String(!shut));
  head.children[1].textContent = name;
  head.children[2].textContent = list.length ? String(list.length) : "";
  // Empty groups do not collapse; their heading only offers creation.
  head.children[3].innerHTML = list.length ? icon(shut ? "chevron-right" : "chevron-down", 14) : "";
  const fold = () => {
    if (!list.length) return;
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
    head.children[3].addEventListener("click", (e) => {
      e.stopPropagation();
      fold();
    });
  }
  if (opts.extra) head.append(...opts.extra);
  rail.append(head);
  if (shut) return head;

  for (const ws of list) {
    const card = h("div", "railworkspace" + (ws.id === openId ? " on" : ""));
    card.dataset.workspace = ws.id;
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
    b.title = [owner, repoLabel(ws), ws.branch].filter(Boolean).join(" · ");
    b.addEventListener("click", () => hooks.open(ws));
    if (owner || opts.avatars) b.children[0].after(template("span", "av", owner ? avatar(owner) : avatars(ws.repos.map((r) => r.name))));
    if (ws.remote && !ws.remote.online) card.classList.add("off");
    if (!ws.remote) attachMenu(b, ws, board, hooks, title, "sub");
    card.append(b);

    if (ws.tabs.length) {
      const key = `@ws:${ws.id}`;
      // Display a single agent directly without a redundant collapsible group row.
      const shut = ws.tabs.length > 1 && folded(key);
      const toggle = template("button", "railagents-toggle", `<span></span>${icon(shut ? "chevron-right" : "chevron-down", 12)}`);
      toggle.children[0].textContent = tn(ws.tabs.length, "rail.agents");
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
    rail.append(card);
  }
  return head;
}

/// Pixels the pointer must travel before a press on a heading becomes a drag instead of a click.
const DRAG_START = 4;

/// Reorder projects by dragging their headings, or with Alt+Arrow on a focused heading. Pointer
/// gestures are used because Tauri intercepts HTML drag events for native file drops.
function reorderable(box: HTMLElement, head: HTMLElement, hooks: Hooks) {
  const commit = () => {
    const ids = [...box.parentElement!.querySelectorAll<HTMLElement>(":scope > .railproject")].map((p) => p.dataset.project!);
    hooks.reorderProjects(ids);
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
