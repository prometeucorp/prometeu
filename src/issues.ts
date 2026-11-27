import { track } from "./analytics";
import { invoke } from "./ipc";
import { listen } from "@tauri-apps/api/event";
import { icon } from "./icons";
import { fromBack, t } from "./i18n";
import type { Board, Issue, Issues, LinearStatus, Workspace } from "./types";
import { $, empty, h, template } from "./util";
import { button } from "./ui";
import { issueTab, issueSearch, issueRefresh, issueFilter, issueColumns } from "./issues-controls";
import { sectionHeader, toolbar } from "./components/compositions";
import { filterLinearIssues } from "./issues-filter";
import { githubIssues } from "./github-issues";
import { todoList } from "./todos";
import type { GitHubItem, GitHubPrepared } from "./types";

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
  createGitHub: (item: GitHubItem, project: string, git?: GitHubPrepared) => void;
  toSettings: () => void;
  github: () => import("./types").GitHubStatus;
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

/// Persist collapsed groups; searching or filtering expands matching results regardless of saved collapse state.
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
let state = "";
let teamFilter: ReturnType<typeof issueFilter>;
let stateFilter: ReturnType<typeof issueFilter>;
let clearFilters: HTMLButtonElement;
let claiming = false;
let visible = false;
let accountId = "";
let find: HTMLInputElement;
let meta: HTMLElement;
let columns: HTMLElement;
type Provider = "linear" | "github" | "todos";
const PROVIDERS: readonly Provider[] = ["linear", "github", "todos"];
const saved = localStorage.getItem("prometeu:issues:provider");
let provider: Provider = PROVIDERS.find(id => id === saved) ?? "linear";
let github: ReturnType<typeof githubIssues>;
let todos: ReturnType<typeof todoList>;

export function init(context: Ctx) {
  ctx = context;
  $("tasks-heading").append(sectionHeader({ title: t("rail.issues"), description: t("issues.intro") }));
  github = githubIssues($("github-issues-pane"), { ...ctx, create: ctx.createGitHub });
  todos = todoList($("todos-issues-pane"), { say: ctx.say });
  const providers = $("iproviders");
  providers.setAttribute("aria-label", t("issues.providers"));
  for (const [id, label] of [["linear", "Linear"], ["github", "GitHub"], ["todos", t("todos.tab")]] as const) {
    const control = issueTab(label, () => {
      provider = id; localStorage.setItem("prometeu:issues:provider", id); showProvider();
    });
    control.dataset.track = `issues.provider.${id}`;
    control.id = `issues-provider-${id}`;
    control.setAttribute("aria-controls", `${id}-issues-pane`);
    $(`${id}-issues-pane`).setAttribute("aria-labelledby", control.id);
    providers.append(control);
  }
  providers.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const index = PROVIDERS.indexOf(provider);
    const next = event.key === "Home" ? PROVIDERS[0] : event.key === "End" ? PROVIDERS[PROVIDERS.length - 1]
      : PROVIDERS[(index + (event.key === "ArrowRight" ? 1 : PROVIDERS.length - 1)) % PROVIDERS.length];
    $("issues-provider-" + next).click(); $("issues-provider-" + next).focus();
  });
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
export const count = () => {
  const linear = ctx?.connected() && got ? got.issues.length : null;
  const assigned = github?.count() ?? null;
  return linear === null && assigned === null ? null : (linear ?? 0) + (assigned ?? 0);
};

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
  showProvider();
}

function showProvider() {
  for (const id of PROVIDERS) {
    const control = $("issues-provider-" + id);
    control.setAttribute("aria-selected", String(provider === id)); control.tabIndex = provider === id ? 0 : -1;
    control.classList.toggle("on", provider === id);
  }
  $("linear-issues-pane").hidden = provider !== "linear";
  if (provider === "todos") todos.show(); else todos.hide();
  if (provider === "github") github.show(); else github.hide();
  if (provider !== "linear") return;
  void load();
  draw();
}

export function hide() {
  visible = false;
  github.hide();
  todos.hide();
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
  const search = issueSearch(t("issues.search"));
  find = search.control;
  meta = h("span", "imeta"); meta.id = "imeta"; meta.setAttribute("role", "status");
  const refreshButton = issueRefresh(t("issues.refresh"), () => {
    if (ctx.connected() && !loading) void refresh(true);
  });
  refreshButton.id = "irefresh";
  teamFilter = issueFilter(t("issues.team"), value => { pickTeam(value); drawList(); });
  stateFilter = issueFilter(t("issues.column.status"), value => { state = value; drawList(); });
  clearFilters = button(t("issues.clearFilters"), () => {
    pickTeam(""); state = query = find.value = ""; drawList(); find.focus();
  }, "ghost", "issues.filter.clear");
  const filters = toolbar([teamFilter.control, stateFilter.control, search.root], [clearFilters, refreshButton]);
  bar.replaceChildren(filters);
  $("itabs").append(meta);
  columns = issueColumns();
  $("ilist").before(columns);
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
}

function drawMeta() {
  const problem = error || (scope === "available" ? got?.available_error : "");
  meta.classList.toggle("err", !!problem && !loading);
  meta.textContent = loading
    ? t("issues.busy")
    : problem
      ? problem
      : got
        ? t("issues.updated", { when: ago(got.fetched_at * 1000) })
        : "";
  ($("irefresh") as HTMLButtonElement).disabled = loading || !ctx.connected();
}

/* Issue list. */

export function draw() {
  if (!visible) return;
  if (provider === "github") { github.draw(); return; }
  if (provider === "todos") { todos.draw(); return; }
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
    const tab = issueTab(t(key === "mine" ? "issues.mine" : "issues.available"), () => {
      scope = key;
      pickTeam(""); state = "";
      draw();
    });
    tab.id = `issues-${key}`;
    tab.setAttribute("aria-controls", "ilist");
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
    (tab.children[1] as HTMLElement).hidden = !got;
    tab.children[1].textContent = String(index === 0 ? got?.issues.length ?? 0 : got?.available.length ?? 0);
    if (selected) $("ilist").setAttribute("aria-labelledby", tab.id);
  });
}

function drawList() {
  const list = $("ilist");
  list.replaceChildren();
  const connected = ctx.connected();
  $("linear-issues-pane").classList.toggle("task-disconnected", !connected);
  $("itabs").hidden = $("ibar").hidden = !connected;
  columns.hidden = true;
  teamFilter.control.disabled = stateFilter.control.disabled = !ctx.connected() || !got;
  clearFilters.hidden = !query && !team && !state;
  if (!got || !ctx.connected()) {
    teamFilter.setOptions([["", t("issues.team.all")]], "");
    stateFilter.setOptions([["", t("issues.status.all")]], "");
  }

  if (!ctx.connected()) {
    list.setAttribute("aria-labelledby", "issues-provider-linear");
    const connect = empty(t("issues.off.title"), t("issues.off.body"));
    connect.append(button(t("issues.off.action"), ctx.toSettings, "outline", "issues.connect"));
    list.append(connect);
    return;
  }
  if (!got) {
    if (!loading && error) {
      list.append(empty(t("issues.failed.title"), error, [t("issues.failed.action"), () => refresh(true), "issues.refresh"]));
    }
    return;
  }
  if (scope === "available" && got.available_error && !got.available.length) {
    list.append(empty(t("issues.failed.title"), got.available_error, [t("issues.failed.action"), () => refresh(true), "issues.refresh"]));
    return;
  }

  const source = scope === "mine" ? got.issues : got.available;
  drawFilters(source);
  const hits = filterLinearIssues(source, query, team, state);
  if (!hits.length) {
    if (!query && !team && !state && scope === "mine" && got.available.length) {
      drawSuggestions(list, got.available);
      return;
    }
    list.append(
      query || team || state
        ? empty(t("issues.noMatch.title"), t("issues.noMatch.body"))
        : empty(t(scope === "mine" ? "issues.empty.title" : "issues.available.empty.title"),
          t(scope === "mine" ? "issues.empty.body" : "issues.available.empty.body")),
    );
    return;
  }

  const kinds = [...KINDS, ...unknownKinds(hits)];
  columns.hidden = false;
  for (const [kind, label] of kinds) {
    const grouped = hits.filter((i) => i.state.kind === kind).sort(byUrgency);
    if (!grouped.length) continue;
    const shut = !query && !team && !state && folded(kind);
    const head = button("", undefined, "ghost", "issues.group.toggle");
    head.classList.add("igroup"); head.classList.toggle("shut", shut);
    head.innerHTML = `<span class="gc"></span><span class="t"></span><span class="c"></span>`;
    head.setAttribute("aria-expanded", String(!shut));
    head.children[0].innerHTML = icon(shut ? "chevron-right" : "chevron-down", 14);
    head.children[1].textContent = label;
    head.children[2].textContent = String(grouped.length);
    head.title = t(shut ? "issues.group.show" : "issues.group.fold", { group: label });
    head.dataset.track = "issues.group.toggle";
    head.addEventListener("click", () => {
      localStorage.setItem(FOLD + kind, shut ? "0" : "1");
      drawList();
    });
    list.append(head);
    if (shut) continue;
    for (const issue of grouped) list.append(row(issue, scope === "available"));
  }
  if (scope === "mine" && !query && !team && !state && got.available.length) {
    const callout = template("div", "iavailable-callout", `<span></span><button class="ghost sm"></button>`);
    callout.children[0].textContent = t("issues.available.count", { n: got.available.length });
    callout.children[1].textContent = t("issues.viewAvailable");
    (callout.children[1] as HTMLElement).dataset.track = "issues.available";
    callout.children[1].addEventListener("click", showAvailable);
    list.append(callout);
  }
}

function showAvailable() {
  scope = "available";
  pickTeam(""); state = "";
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
  view.dataset.track = "issues.available";
  view.addEventListener("click", showAvailable);
  for (const issue of [...available].sort(byUrgency).slice(0, 3)) box.append(row(issue, true));
  list.append(box);
}

// Keep controls mounted so menu selection and refresh preserve keyboard focus.
function drawFilters(source: Issue[]) {
  const teams = [...new Set(source.map(item => item.team).filter(Boolean))].sort();
  const states = [...new Set(source.map(item => item.state.name))].sort();
  if (team && !teams.includes(team)) pickTeam("");
  if (state && !states.includes(state)) state = "";
  teamFilter.setOptions([["", t("issues.team.all")], ...teams.map(key => [key, key] as [string, string])], team);
  stateFilter.setOptions([["", t("issues.status.all")], ...states.map(key => [key, key] as [string, string])], state);
  clearFilters.hidden = !query && !team && !state;
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

function row(issue: Issue, available = false): HTMLElement {
  const el = template(
    "div",
    "irow" + (available ? " available" : ""),
    `<span class="prio p${Math.min(issue.priority, 4)}"><i></i><i></i><i></i></span>` +
      `<span class="iid"></span>` +
      `<span class="ititle"><b></b><span class="iproj"></span></span>` +
      `<span class="istate"><i class="dot"></i><span></span></span>` +
      `<span class="iago"></span>` +
      `<span class="iact"></span>`,
  );
  el.tabIndex = 0; el.setAttribute("role", "link"); el.setAttribute("aria-label", issue.title);
  el.title = issue.description ? issue.description.slice(0, 400) : issue.title;
  (el.querySelector(".prio") as HTMLElement).title = issue.priority_label;
  el.querySelector(".iid")!.textContent = issue.identifier;
  el.querySelector(".ititle b")!.textContent = issue.title;
  const context = el.querySelector(".iproj")!;
  context.append(h("span", "", [issue.team, issue.project].filter(Boolean).join(" · ")));
  for (const label of issue.labels) context.append(h("span", "task-label", label.name));
  (el.querySelector(".istate .dot") as HTMLElement).style.background = issue.state.color;
  el.querySelector(".istate span")!.textContent = issue.state.name;
  el.querySelector(".iago")!.textContent = ago(Date.parse(issue.updated_at));

  // Row clicks open Linear; buttons own other actions.
  const openLinear = () => invoke("linear_open", { url: issue.url }).then(() => track({ name: "issue_action_used", integration: "linear", action: "open_source" })).catch((e) => ctx.say(fromBack(e), true));
  el.dataset.track = "issues.linear.open_source";
  el.addEventListener("click", openLinear);
  el.addEventListener("keydown", (e) => e.key === "Enter" && e.target === el && openLinear());

  const act = el.querySelector(".iact")!;
  const owner = ctx.board().workspaces.find((w) => w.issue?.id === issue.id && !w.archived);
  const btn = button(available ? t(ctx.canAssign() ? "issues.claim" : "issues.allowClaim") : t(owner ? "issues.open" : "issues.start"), undefined, "outline");
  btn.dataset.track = available ? "issues.linear.claim" : "issues.linear.open_workspace";
  btn.classList.add("sm");
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
        track({ name: "issue_action_used", integration: "linear", action: "claim" });
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
        if (keyboard && visible && provider === "linear") {
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
    track({ name: "issue_action_used", integration: "linear", action: owner ? "open_workspace" : "prepare_workspace" });
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
