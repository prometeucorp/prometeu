import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { fromBack, paint, t } from "./i18n";
import type { Board, Issue, Issues, LinearStatus, Workspace } from "./types";
import { $, empty, h, template } from "./util";

/// Show assigned and available Linear issues. Existing workspaces replace the create action with navigation. Reuse the backend's two-minute cache, allow forced refresh, and preserve the last successful list after failure.

type Ctx = {
  say: (text: string, isError?: boolean) => void;
  board: () => Board;
  connected: () => boolean;
  accountId: () => string;
  canAssign: () => boolean;
  /// Notify the sidebar when the issue count changes.
  redraw: () => void;
  open: (ws: Workspace) => void;
  create: (issue: Issue) => void;
  toSettings: () => void;
};

/// Match the backend cache lifetime before requesting another list on opening.
const STALE = 120_000;

/// Order state groups with localized labels while preserving team-defined state names in rows.
const KINDS: [string, string][] = [
  ["started", t("issues.kind.started")],
  ["unstarted", t("issues.kind.unstarted")],
  ["triage", t("issues.kind.triage")],
  ["backlog", t("issues.kind.backlog")],
];

/// Persist collapsed groups; searching expands matching results regardless of saved collapse state.
const FOLD = "prometeu:issues:grupo:";
const folded = (kind: string) => localStorage.getItem(FOLD + kind) === "1";

/// Persist the selected team; an empty string means all teams.
const TEAM = "prometeu:issues:time";

let ctx: Ctx;
let got: Issues | null = null;
let loading = false;
let revision = 0;
let error = "";
let query = "";
let team = localStorage.getItem(TEAM) ?? "";
let scope: "mine" | "available" = "mine";
let claiming = false;
let visible = false;
let accountId = "";
let find: HTMLInputElement;
let meta: HTMLElement;

export function init(context: Ctx) {
  ctx = context;
  accountId = ctx.accountId();
  buildTabs();
  buildBar();
  listen<LinearStatus>("linear", ({ payload }) => {
    revision++;
    loading = false;
    const nextAccountId = payload.who?.id ?? "";
    if (!payload.connected) {
      got = null;
      error = "";
    } else if (!payload.busy) {
      if (accountId !== nextAccountId) got = null;
      error = "";
      void refresh(false);
    }
    accountId = nextAccountId;
    ctx.redraw();
    draw();
  });
  if (ctx.connected()) void refresh(false);
}

/// Null means Linear is unavailable, so the sidebar hides its count.
export const count = () => (ctx?.connected() && got ? got.issues.length : null);

/// Launcher data: null means no Linear connection; an empty list while busy means loading.
export const list = () => (ctx?.connected() ? (got?.issues ?? []) : null);
export const busy = () => loading;

/// Load only when missing or stale; used when opening the launcher picker.
export function load(): Promise<void> {
  if (!ctx?.connected() || loading) return Promise.resolve();
  const old = !got || Date.now() / 1000 - got.fetched_at > STALE / 1000;
  return old ? refresh(false) : Promise.resolve();
}

export function show() {
  visible = true;
  void load();
  draw();
  find.focus();
}

export function hide() {
  visible = false;
}

async function refresh(force: boolean) {
  const request = ++revision;
  loading = true;
  drawMeta();
  try {
    const next = await invoke("linear_issues", { force });
    if (request !== revision) return;
    got = next;
    error = "";
  } catch (e) {
    if (request !== revision) return;
    error = fromBack(e);
  }
  loading = false;
  ctx.redraw();
  draw();
}

/* Toolbar. */

function buildBar() {
  const bar = $("ibar");
  bar.innerHTML = `
    <label class="ifind">${icon("search", 14)}<input spellcheck="false" /></label>
    <span class="spacer"></span>
    <span class="imeta" id="imeta"></span>
    <button class="ico" id="irefresh" data-t-title="issues.refresh">${icon("rotate")}</button>`;
  paint(bar);
  find = bar.querySelector("input")!;
  find.placeholder = t("issues.search");
  meta = bar.querySelector("#imeta")!;
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
  bar.querySelector("#irefresh")!.addEventListener("click", () => {
    if (ctx.connected() && !loading) void refresh(true);
  });
}

function drawMeta() {
  meta.classList.toggle("err", !!error && !loading);
  meta.textContent = loading
    ? t("issues.busy")
    : error
      ? error
      : got
        ? t("issues.updated", { when: ago(got.fetched_at * 1000) })
        : "";
  ($("irefresh") as HTMLButtonElement).disabled = loading || !ctx.connected();
}

/* Issue list. */

export function draw() {
  if (!visible) return;
  drawTabs();
  drawMeta();
  drawList();
}

function buildTabs() {
  const tabs = $("itabs");
  tabs.setAttribute("aria-label", t("issues.scope"));
  const list = $("ilist");
  list.setAttribute("role", "tabpanel");
  for (const key of ["mine", "available"] as const) {
    const tab = template("button", "tab itab", `<span></span><span class="c"></span>`);
    tab.setAttribute("role", "tab");
    tab.id = `issues-${key}`;
    tab.setAttribute("aria-controls", "ilist");
    tab.children[0].textContent = t(key === "mine" ? "issues.mine" : "issues.available");
    tab.addEventListener("click", () => {
      scope = key;
      pickTeam("");
      draw();
    });
    tabs.append(tab);
  }
  tabs.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const buttons = tabs.querySelectorAll<HTMLButtonElement>(".itab");
    const next = scope === "mine" ? 1 : 0;
    buttons[next].focus();
    buttons[next].click();
  });
}

function drawTabs() {
  const tabs = $("itabs").querySelectorAll<HTMLButtonElement>(".itab");
  tabs.forEach((tab, index) => {
    const selected = scope === (index === 0 ? "mine" : "available");
    tab.classList.toggle("on", selected);
    tab.setAttribute("aria-selected", String(selected));
    tab.tabIndex = selected ? 0 : -1;
    tab.children[0].textContent = t(index === 0 ? "issues.mine" : "issues.available");
    tab.children[1].textContent = String(index === 0 ? got?.issues.length ?? 0 : got?.available.length ?? 0);
    if (selected) $("ilist").setAttribute("aria-labelledby", tab.id);
  });
}

function drawList() {
  const list = $("ilist");
  list.replaceChildren();
  // Hide team filters until a list exists; drawTeams restores them afterward.
  const teams = $("iteams");
  teams.replaceChildren();
  teams.hidden = true;

  if (!ctx.connected()) {
    list.append(
      empty(t("issues.off.title"), t("issues.off.body"), [t("issues.off.action"), ctx.toSettings]),
    );
    return;
  }
  if (!got) {
    if (!loading && error) {
      list.append(empty(t("issues.failed.title"), error, [t("issues.failed.action"), () => refresh(true)]));
    }
    return;
  }

  const source = scope === "mine" ? got.issues : got.available;
  const found = source.filter(matches);
  drawTeams(found, source);
  const hits = team ? found.filter((i) => i.team === team) : found;
  if (!hits.length) {
    if (!query && !team && scope === "mine" && got.available.length) {
      drawSuggestions(list, got.available);
      return;
    }
    list.append(
      query || team
        ? empty(t("issues.noMatch.title"), t("issues.noMatch.body"))
        : empty(t(scope === "mine" ? "issues.empty.title" : "issues.available.empty.title"),
          t(scope === "mine" ? "issues.empty.body" : "issues.available.empty.body")),
    );
    return;
  }

  const kinds = [...KINDS, ...unknownKinds(hits)];
  for (const [kind, label] of kinds) {
    const grouped = hits.filter((i) => i.state.kind === kind).sort(byUrgency);
    if (!grouped.length) continue;
    const shut = !query && folded(kind);
    const head = template(
      "button",
      "igroup" + (shut ? " shut" : ""),
      `<span class="gc"></span><span class="t"></span><span class="c"></span>`,
    );
    head.children[0].innerHTML = icon(shut ? "chevron-right" : "chevron-down", 14);
    head.children[1].textContent = label;
    head.children[2].textContent = String(grouped.length);
    head.title = t(shut ? "issues.group.show" : "issues.group.fold", { group: label });
    head.addEventListener("click", () => {
      localStorage.setItem(FOLD + kind, shut ? "0" : "1");
      drawList();
    });
    list.append(head);
    if (shut) continue;
    for (const issue of grouped) list.append(row(issue, scope === "available"));
  }
  if (scope === "mine" && !query && got.available.length) {
    const callout = template("div", "iavailable-callout", `<span></span><button class="ghost sm"></button>`);
    callout.children[0].textContent = t("issues.available.count", { n: got.available.length });
    callout.children[1].textContent = t("issues.viewAvailable");
    callout.children[1].addEventListener("click", showAvailable);
    list.append(callout);
  }
}

function showAvailable() {
  scope = "available";
  pickTeam("");
  draw();
  $("itabs").querySelectorAll<HTMLButtonElement>(".itab")[1].focus();
}

function drawSuggestions(list: HTMLElement, available: Issue[]) {
  const hero = empty(t("issues.emptyQueue.title"), t("issues.emptyQueue.body"));
  hero.classList.add("queue-empty");
  list.append(hero);
  const box = template("div", "isuggestions", `<div class="isuggestions-head"><strong></strong><button class="ghost sm"></button></div>`);
  box.querySelector("strong")!.textContent = t("issues.suggestions");
  const view = box.querySelector("button")!;
  view.textContent = t("issues.viewAll", { n: available.length });
  view.addEventListener("click", showAvailable);
  for (const issue of [...available].sort(byUrgency).slice(0, 3)) box.append(row(issue, true));
  list.append(box);
}

/// Show team filters only for multiple teams. Derive pills from the full list and counts from search matches so typing does not reshape the toolbar.
function drawTeams(found: Issue[], source: Issue[]) {
  const box = $("iteams");
  box.replaceChildren();
  const keys = [...new Set(source.map((i) => i.team).filter(Boolean))].sort();
  // Clear a selected team that disappeared from the list instead of hiding all results.
  if (team && !keys.includes(team)) pickTeam("");
  box.hidden = keys.length < 2;
  if (box.hidden) return;

  const label = template("span", "tlabel", `${icon("filter", 13)}<span></span>`);
  label.children[1].textContent = t("issues.team");
  box.append(label);

  const pills = h("div", "tpills");
  for (const key of ["", ...keys]) {
    const mine = key ? found.filter((i) => i.team === key) : found;
    const pill = template("button", "tpill" + (key === team ? " on" : ""), `<span></span><span class="c"></span>`);
    pill.children[0].textContent = key || t("issues.team.all");
    pill.children[1].textContent = String(mine.length);
    pill.addEventListener("click", () => {
      pickTeam(key);
      drawList();
    });
    pills.append(pill);
  }
  box.append(pills);
}

function pickTeam(key: string) {
  team = key;
  if (key) localStorage.setItem(TEAM, key);
  else localStorage.removeItem(TEAM);
}

/// Keep issues with future, unknown Linear state types visible.
function unknownKinds(list: Issue[]): [string, string][] {
  const known = new Set(KINDS.map(([k]) => k));
  const extra = new Set(list.map((i) => i.state.kind).filter((k) => !known.has(k)));
  return [...extra].map((k) => [k, k]);
}

/// Sort urgent issues first and unprioritized issues last, breaking ties by recency.
function byUrgency(a: Issue, b: Issue) {
  const rank = (p: number) => (p === 0 ? 5 : p);
  return rank(a.priority) - rank(b.priority) || b.updated_at.localeCompare(a.updated_at);
}

function matches(i: Issue) {
  if (!query) return true;
  const hay = [i.identifier, i.title, i.project ?? "", i.team, i.state.name, i.description ?? ""]
    .join(" ")
    .toLowerCase();
  return query.split(/\s+/).every((word) => hay.includes(word));
}

function row(issue: Issue, available = false): HTMLElement {
  const el = template(
    "div",
    "irow" + (available ? " available" : ""),
    `<span class="prio p${Math.min(issue.priority, 4)}"><i></i><i></i><i></i></span>` +
      `<span class="iid"></span>` +
      `<span class="ititle"><b></b><span class="iproj"></span></span>` +
      `<span class="istate"><i class="dot"></i><span></span></span>` +
      `<span class="iact"></span>` +
      `<span class="iago"></span>`,
  );
  el.tabIndex = 0;
  el.title = issue.description ? issue.description.slice(0, 400) : issue.title;
  (el.querySelector(".prio") as HTMLElement).title = issue.priority_label;
  el.querySelector(".iid")!.textContent = issue.identifier;
  el.querySelector(".ititle b")!.textContent = issue.title;
  el.querySelector(".iproj")!.textContent = issue.project ?? "";
  (el.querySelector(".istate .dot") as HTMLElement).style.background = issue.state.color;
  el.querySelector(".istate span")!.textContent = issue.state.name;
  el.querySelector(".iago")!.textContent = ago(Date.parse(issue.updated_at));

  // Row clicks open Linear; buttons own other actions.
  const openLinear = () => invoke("linear_open", { url: issue.url }).catch((e) => ctx.say(fromBack(e), true));
  el.addEventListener("click", openLinear);
  el.addEventListener("keydown", (e) => e.key === "Enter" && e.target === el && openLinear());

  const act = el.querySelector(".iact")!;
  const owner = ctx.board().workspaces.find((w) => w.issue?.id === issue.id && !w.archived);
  const btn = h("button", owner ? "ghost sm" : "pri sm",
    available ? t(ctx.canAssign() ? "issues.claim" : "issues.allowClaim") : t(owner ? "issues.open" : "issues.create")) as HTMLButtonElement;
  if (available) {
    if (claiming) btn.setAttribute("aria-disabled", "true");
    btn.addEventListener("click", async (e) => {
      e.stopPropagation();
      if (claiming) return;
      if (!ctx.canAssign()) {
        ctx.toSettings();
        return;
      }
      const keyboard = e.detail === 0;
      let succeeded = false;
      claiming = true;
      $("ilist").setAttribute("aria-busy", "true");
      $("ilist").querySelectorAll(".irow.available .iact button").forEach((action) => action.setAttribute("aria-disabled", "true"));
      try {
        const claimed = await invoke("linear_claim", { id: issue.id });
        succeeded = true;
        if (got) {
          got = {
            ...got,
            issues: [claimed, ...got.issues],
            available: got.available.filter((item) => item.id !== issue.id),
          };
        }
        ctx.say(t("issues.claimed", { id: claimed.identifier }));
        ctx.redraw();
        draw();
        void refresh(true);
      } catch (e) {
        ctx.say(fromBack(e), true);
      } finally {
        claiming = false;
        $("ilist").removeAttribute("aria-busy");
        draw();
        if (keyboard && visible) {
          const same = [...$("ilist").querySelectorAll<HTMLElement>(".irow.available")]
            .find((candidate) => candidate.querySelector(".iid")?.textContent === issue.identifier);
          (succeeded ? $("itabs").querySelectorAll<HTMLButtonElement>(".itab")[1]
            : same?.querySelector<HTMLButtonElement>(".iact button"))?.focus();
        }
      }
    });
    act.append(btn);
    return el;
  }
  btn.title = owner
    ? `${owner.title} · ${owner.branch}`
    : t("issues.create.title", { branch: issue.branch_name });
  btn.addEventListener("click", (e) => {
    e.stopPropagation();
    owner ? ctx.open(owner) : ctx.create(issue);
  });
  act.append(btn);
  return el;
}

/// Compact relative time indicates recent issue activity.
function ago(ms: number): string {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 60) return t("ago.now");
  if (s < 3600) return t("ago.min", { n: Math.round(s / 60) });
  if (s < 86_400) return t("ago.hour", { n: Math.round(s / 3600) });
  return t("ago.day", { n: Math.round(s / 86_400) });
}
