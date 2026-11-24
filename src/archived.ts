import { attachMenu, type Hooks } from "./sidebar";
import { avatars, icon } from "./icons";
import { paint, t, tn } from "./i18n";
import { hasWorktree, merged, prs, repoLabel, type Board, type Workspace } from "./types";
import { $, empty, template } from "./util";

/// Browse, reopen, restore, and clean archived workspaces in a searchable page rather than an expanded sidebar list. Reverse board order to show newer workspaces first.

type Ctx = {
  board: () => Board;
  hooks: () => Hooks;
};

let ctx: Ctx;
let visible = false;
let query = "";
let find: HTMLInputElement;

export function init(context: Ctx) {
  ctx = context;
  buildBar();
}

export function show() {
  visible = true;
  draw();
  find.focus();
}

export function hide() {
  visible = false;
}

/* Toolbar. */

function buildBar() {
  const bar = $("abar");
  bar.innerHTML = `
    <label class="ifind">${icon("search", 14)}<input spellcheck="false" data-track="archived.search" /></label>
    <span class="spacer"></span>
    <span class="imeta" id="ameta"></span>
    <button class="ghost md" id="aclean" data-t-title="rail.cleanup.title">${icon("trash", 14)}<span data-t="rail.cleanup"></span></button>`;
  paint(bar);
  find = bar.querySelector("input")!;
  find.placeholder = t("arch.search");
  find.addEventListener("input", () => {
    query = find.value.trim().toLowerCase();
    drawList();
  });
  find.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      find.value = "";
      query = "";
      drawList();
    }
  });
  bar.querySelector("#aclean")!.addEventListener("click", () => ctx.hooks().cleanup());
}

/* Workspace list. */

export function draw() {
  if (!visible) return;
  drawList();
}

function drawList() {
  const list = $("alist");
  list.replaceChildren();
  const gone = ctx
    .board()
    .workspaces.filter((w) => w.archived)
    .reverse();

  // Offer disk cleanup only while archived worktrees remain.
  const disk = gone.filter(hasWorktree).length;
  $("ameta").textContent = gone.length
    ? disk
      ? `${tn(gone.length, "arch.count")} · ${tn(disk, "arch.disk")}`
      : tn(gone.length, "arch.count")
    : "";
  ($("aclean") as HTMLButtonElement).hidden = !disk;

  if (!gone.length) {
    list.append(empty(t("arch.empty.title"), t("arch.empty.body")));
    return;
  }
  const hits = gone.filter(matches);
  if (!hits.length) {
    list.append(empty(t("arch.noMatch.title"), t("arch.noMatch.body")));
    return;
  }
  for (const ws of hits) list.append(row(ws));
}

function matches(ws: Workspace) {
  if (!query) return true;
  const hay = [ws.title, ws.branch, repoLabel(ws), ...prs(ws).map(({ pr }) => pr.title), ws.issue?.identifier ?? ""]
    .join(" ")
    .toLowerCase();
  return query.split(/\s+/).every((word) => hay.includes(word));
}

function row(ws: Workspace): HTMLElement {
  const hooks = ctx.hooks();
  const el = template(
    "div",
    "arow" + (ws.cleaned ? " gone" : ""),
    `<span class="av"></span>` +
      `<span class="atitle"><b></b><span class="abranch"></span></span>` +
      `<span class="apr"></span>` +
      `<span class="atag"></span>` +
      `<span class="iact"></span>`,
  );
  el.tabIndex = 0;
  el.dataset.track = "archived.workspace.open";
  el.querySelector(".av")!.innerHTML = avatars(ws.repos.map((r) => r.name));
  (el.querySelector(".av") as HTMLElement).title = repoLabel(ws);
  el.querySelector(".atitle b")!.textContent = ws.title;
  el.querySelector(".abranch")!.textContent = ws.branch;
  // Show each repository's PR number; mark merged only when every PR has merged.
  const all = prs(ws);
  if (all.length) {
    const pr = el.querySelector(".apr")!;
    pr.textContent = all.map(({ pr }) => `#${pr.number}`).join(" ");
    pr.classList.toggle("merged", merged(ws));
    (pr as HTMLElement).title = all.map(({ repo, pr }) => (ws.repos.length > 1 ? `${repo}: ${pr.title}` : pr.title)).join("\n");
  }
  // Label removed worktrees explicitly while retaining their workspace history.
  if (ws.cleaned) {
    el.querySelector(".atag")!.textContent = t("ws.menu.gone");
    (el.querySelector(".atag") as HTMLElement).title = t("card.cleaned.title", { path: ws.worktree });
  }

  const openIt = () => hooks.open(ws);
  el.addEventListener("click", openIt);
  el.addEventListener("keydown", (e) => e.key === "Enter" && openIt());
  attachMenu(el, ws, hooks, el.querySelector(".atitle b") as HTMLElement, "arow");

  const act = el.querySelector(".iact")!;
  const back = template("button", "ghost", `${icon("archive-restore", 14)}<span></span>`);
  back.dataset.track = "ws.menu.unarchive";
  back.children[1].textContent = t("ws.menu.unarchive");
  back.title = ws.cleaned ? t("ws.menu.gone") : t("ws.menu.unarchive");
  (back as HTMLButtonElement).disabled = ws.cleaned;
  back.addEventListener("click", (e) => {
    e.stopPropagation();
    hooks.archive(ws.id, false);
  });
  act.append(back);
  return el;
}
