import { avatar, icon } from "./icons";
import { fromBack, t, tn } from "./i18n";
import { invoke } from "./ipc";
import { md } from "./markdown";
import { issueRefresh, issueTab } from "./issues-controls";
import { compareId, expandable, isRead, markAll, markRead, merge, readState, saveRead, subjectKey, threads, workspaceFor, type ReadState, type Thread } from "./notification-feed";
import { deliverGitHub } from "./notifications";
import * as team from "./team";
import type { Board, GitHubNotification, GitHubStatus, GitHubSubject, Workspace } from "./types";
import { button } from "./ui";
import { $, empty, h, template } from "./util";

/// One destination for GitHub activity from the Cloud feed and team comment mentions (ADR 0088).
/// The feed carries metadata; titles and text come from GitHub when shown. Mentions keep their
/// relay rule: they leave when the thread is resolved.

type Ctx = {
  say: (text: string, error?: boolean) => void;
  board: () => Board;
  github: () => GitHubStatus;
  open: (workspace: Workspace) => void;
  openMention: (workspace: string, note: string, tab: string | null) => void;
  /// The sidebar and Dock badge follow the unread count.
  changed: () => void;
  toSettings: () => void;
};
type Filter = "all" | "github" | "mentions";

const POLL_MS = 30_000;
const GLYPHS: Record<GitHubNotification["kind"], Parameters<typeof icon>[0]> = {
  review_approved: "check", changes_requested: "pencil", review_requested: "eye", commented: "message-square",
  mentioned: "at-sign", assigned: "user", merged: "git-merge", closed: "x", ci_failed: "terminal",
};

let ctx: Ctx;
/// The Prometeu account the feed, cursor and read marks belong to; empty when signed out.
let account = "";
let items: GitHubNotification[] = [];
/// undefined: no Prometeu account; null: account without a linked GitHub identity.
let linked: string | null | undefined;
let cursor: string | null = null;
/// The first feed response (after startup or signing in) is history and never notifies.
let primed = false;
let polling = false;
let visible = false;
let filter: Filter = "all";
let read: ReadState = { before: "", ids: [] };
/// The expanded thread key.
let expanded = "";
/// Titles and text fetched for `cachedFor`; a different account or GitHub login must check access again.
const subjects = new Map<string, GitHubSubject>();
const details = new Map<string, string>();
/// Detail requests in flight, so redraws while a thread loads do not repeat them.
const fetching = new Set<string>();
/// Failed detail requests, shown until the thread is expanded again, which retries them.
const failures = new Map<string, string>();
let cachedFor = "";
function cacheOwner() {
  const owner = `${account}|${ctx.github().login ?? ""}`;
  if (owner !== cachedFor) { subjects.clear(); details.clear(); failures.clear(); cachedFor = owner; }
  return owner;
}
let repositories: Promise<Map<string, string[]>> | null = null;

export function init(context: Ctx) {
  ctx = context;
  build();
  void poll();
  setInterval(() => void poll(), POLL_MS);
  window.addEventListener("focus", () => void poll());
  team.onChange(() => { if (visible) draw(); });
}

/// Unread issues and PRs, not events: a busy PR counts once.
const unreadGitHub = () => threads(items, read).filter(thread => thread.items.some(item => !isRead(read, item.id))).length;
/// Unread GitHub notifications plus pending mentions, for the sidebar and Dock.
export const count = () => unreadGitHub() + team.inboxCount();

async function poll() {
  if (polling) return;
  polling = true;
  try {
    let first = !primed;
    const fresh: GitHubNotification[] = [];
    for (let page = 0; page < 10; page++) {
      const feed = await invoke("github_feed", { after: cursor });
      if (!feed) {
        // Signing out forgets the feed; signing in again starts from history.
        if (account) forget("");
        linked = undefined;
        break;
      }
      if (feed.account !== account) {
        // Another account's feed, cursor and read marks replace these, and its first page is
        // history. A page fetched with the previous account's cursor is incomplete: fetch again.
        const stale = cursor !== null;
        forget(feed.account);
        fresh.length = 0; first = true;
        if (stale) continue;
      }
      linked = feed.github ? feed.github.login ?? "" : null;
      primed = true;
      fresh.push(...feed.notifications);
      for (const item of feed.notifications) if (!cursor || compareId(item.id, cursor) > 0) cursor = item.id;
      if (!feed.more) break;
    }
    if (!fresh.length) { if (visible) draw(); return; }
    items = merge(items, fresh);
    await loadSubjects();
    repositories = null;
    // The first load after startup is history; only later arrivals interrupt the person.
    if (!first) threads(fresh, read).forEach((thread, index) => void arrived(thread, index < 3));
    ctx.changed();
    if (visible) draw();
  } catch (error) {
    // The feed retries on the next poll; a dropped connection is not worth an alert each time.
    console.warn("github feed", fromBack(error));
  } finally { polling = false; }
}

function forget(next: string) {
  account = next; items = []; cursor = null; primed = false;
  read = next ? readState(next) : { before: "", ids: [] };
  ctx.changed();
}

/// One notice per issue or PR; after a long sleep only the newest few interrupt, the rest wait in
/// the list and badge.
async function arrived(thread: Thread, announce: boolean) {
  if (announce) void deliverGitHub(threadHeadline(thread), subjectLine(thread.lead)).catch(error => ctx.say(fromBack(error), true));
  const item = thread.lead;
  const workspace = await workspaceOf(item);
  if (!workspace) return;
  // The PR changed on GitHub: refresh its state now instead of waiting for the next scan.
  void invoke("set_unread", { id: workspace.id, unread: true }).catch(() => {});
  if (item.subject === "pr") void invoke("pr_open", { id: workspace.id }).catch(() => {});
}

/// Projects can be added later, so `draw` and `poll` drop this before each round of lookups.
async function workspaceOf(item: GitHubNotification) {
  repositories ??= invoke("github_projects").then(projects => {
    const paths = new Map(ctx.board().projects.map(project => [project.id, project.path]));
    const byPath = new Map<string, string[]>();
    for (const { project, repository } of projects) {
      const path = paths.get(project);
      if (path) byPath.set(path, [...(byPath.get(path) ?? []), repository.toLowerCase()]);
    }
    return byPath;
  }).catch(() => new Map());
  return workspaceFor(item, ctx.board().workspaces, await repositories);
}

async function loadSubjects() {
  if (!ctx.github().connected) return;
  const owner = cacheOwner();
  const missing = [...new Map(items
    .filter(item => !subjects.has(subjectKey(item.repository, item.number)))
    .map(item => [subjectKey(item.repository, item.number), { repository: item.repository, number: item.number }])).values()];
  for (let start = 0; start < missing.length; start += 50) {
    try {
      const found = await invoke("github_subjects", { keys: missing.slice(start, start + 50) });
      // Results that finish after an account switch belong to the previous credential.
      if (cacheOwner() !== owner) return;
      for (const subject of found) {
        subjects.set(subjectKey(subject.repository, subject.number), subject);
      }
    } catch { return; }
  }
}

const headline = (item: GitHubNotification) => t(`notifications.github.${item.kind}`, { actor: item.actor || "GitHub" });
const threadHeadline = (thread: Thread) =>
  thread.items.length > 1 ? `${headline(thread.lead)} · ${tn(thread.items.length - 1, "notifications.more")}` : headline(thread.lead);
function subjectLine(item: GitHubNotification) {
  cacheOwner();
  const title = subjects.get(subjectKey(item.repository, item.number))?.title;
  return `${item.repository}#${item.number}${title ? ` · ${title}` : ""}`;
}

/* View. */

export function show() {
  visible = true;
  // Titles may have been unavailable before GitHub was connected.
  void loadSubjects().then(() => { if (visible) draw(); });
  draw();
  void poll();
}

export function hide() {
  visible = false;
}

function build() {
  const tabs = $("ntabs");
  tabs.setAttribute("aria-label", t("notifications.center"));
  for (const key of ["all", "github", "mentions"] as const) {
    const tab = issueTab(t(`notifications.filter.${key}`), () => { filter = key; draw(); });
    tab.id = `notifications-${key}`;
    tab.setAttribute("aria-controls", "nlist");
    tabs.append(tab);
  }
  tabs.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
    event.preventDefault();
    const keys = ["all", "github", "mentions"] as const;
    const next = keys[(keys.indexOf(filter) + (event.key === "ArrowRight" ? 1 : 2)) % 3];
    $(`notifications-${next}`).click(); $(`notifications-${next}`).focus();
  });
  const bar = $("nbar");
  const meta = h("span", "imeta"); meta.id = "nmeta"; meta.setAttribute("role", "status");
  const all = button(t("notifications.readAll"), () => { read = markAll(read, items); saveRead(account, read); ctx.changed(); draw(); }, "ghost");
  all.classList.remove("md"); all.id = "nreadall";
  bar.append(h("span", "spacer"), meta, all, issueRefresh(t("issues.refresh"), () => void poll()));
  $("nlist").setAttribute("role", "tabpanel");
}

type Row = { at: number; node: HTMLElement };

export function draw() {
  if (!visible) return;
  const mentions = team.inboxList();
  const counts: Record<Filter, number> = { all: unreadGitHub() + mentions.length, github: unreadGitHub(), mentions: mentions.length };
  $("ntabs").querySelectorAll<HTMLButtonElement>(".itab").forEach(tab => {
    const key = tab.id.replace("notifications-", "") as Filter;
    const selected = key === filter;
    tab.classList.toggle("on", selected); tab.setAttribute("aria-selected", String(selected)); tab.tabIndex = selected ? 0 : -1;
    (tab.children[1] as HTMLElement).hidden = !counts[key];
    tab.children[1].textContent = String(counts[key]);
    if (selected) $("nlist").setAttribute("aria-labelledby", tab.id);
  });
  ($("nreadall") as HTMLButtonElement).disabled = !unreadGitHub();
  $("nmeta").textContent = linked ? `@${linked}` : "";

  const list = $("nlist");
  list.replaceChildren();
  repositories = null;
  const rows: Row[] = [];
  if (filter !== "mentions") {
    for (const thread of threads(items, read)) rows.push({ at: Date.parse(thread.latest.created_at), node: githubRow(thread) });
  }
  if (filter !== "github") {
    for (const mention of mentions) rows.push({ at: mention.ts, node: mentionRow(mention) });
  }
  if (filter !== "mentions" && linked === undefined) list.append(empty(t("notifications.account.title"), t("notifications.account.body")));
  else if (filter !== "mentions" && linked === null) list.append(empty(t("notifications.link.title"), t("notifications.link.body")));
  else if (filter !== "mentions" && items.length && !ctx.github().connected) list.append(h("p", "ui-notice", t("notifications.connect")));
  if (!rows.length) {
    if (!list.childElementCount) list.append(empty(t("notifications.empty.title"), t("notifications.empty.body")));
    return;
  }
  let day = "";
  for (const row of rows.sort((a, b) => b.at - a.at)) {
    const label = new Date(row.at).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
    if (label !== day) { day = label; list.append(h("div", "nday", label)); }
    list.append(row.node);
  }
}

const clock = (item: GitHubNotification) =>
  new Date(item.created_at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });

const visit = (item: GitHubNotification) => void invoke("open_external", { url: item.url }).catch(error => ctx.say(fromBack(error), true));

function rowAction(glyph: Parameters<typeof icon>[0], label: string, run: () => void) {
  const action = button("", run, "ghost"); action.classList.remove("md"); action.classList.add("sm");
  action.innerHTML = icon(glyph, 14); action.title = label; action.setAttribute("aria-label", label);
  action.addEventListener("click", event => event.stopPropagation());
  return action;
}

const githubLink = (item: GitHubNotification) => rowAction("external-link", t("notifications.openGitHub"), () => visit(item));

/// One row per issue or PR. Opening it reads the whole thread and opens it on GitHub; the actions
/// open the matching workspace and expand the thread's events, newest first.
function githubRow(thread: Thread) {
  const item = thread.lead;
  const unread = thread.items.some(entry => !isRead(read, entry.id));
  const row = template("div", "inboxrow notification-row" + (unread ? " unread" : ""),
    `<span class="av"></span><span class="txt"><b></b><span class="what"></span></span><span class="when"></span><span class="go"></span>`);
  row.tabIndex = 0; row.setAttribute("role", "link");
  row.querySelector(".av")!.innerHTML = icon(GLYPHS[item.kind], 16);
  row.querySelector("b")!.textContent = threadHeadline(thread);
  row.querySelector(".what")!.textContent = subjectLine(item);
  row.querySelector(".when")!.textContent = clock(thread.latest);
  const readThread = () => { read = markRead(read, ...thread.items.map(entry => entry.id)); saveRead(account, read); ctx.changed(); };
  const activate = () => { readThread(); visit(item); draw(); };
  const go = row.querySelector(".go")!;
  void workspaceOf(item).then(workspace => {
    if (!workspace) return;
    go.prepend(rowAction("arrow-right", t("notifications.openWorkspace"), async () => {
      readThread();
      // The board changes without redrawing this list: the workspace may be gone since.
      repositories = null;
      const current = await workspaceOf(item);
      if (current) ctx.open(current); else draw();
    }));
  });
  if (expandable(thread)) {
    const open = expanded === thread.key;
    const toggle = rowAction(open ? "chevron-up" : "chevron-down", t("notifications.details"), () => {
      readThread();
      expanded = open ? "" : thread.key;
      for (const entry of thread.items) failures.delete(entry.id);
      draw();
    });
    toggle.setAttribute("aria-expanded", String(open));
    go.append(toggle);
  }
  row.addEventListener("click", activate);
  row.addEventListener("keydown", event => { if (event.key === "Enter" && event.target === row) activate(); });
  if (expanded !== thread.key) return row;
  const wrap = h("div", "notification-entry");
  wrap.append(row);
  const several = thread.items.length > 1;
  for (const entry of [...thread.items].reverse()) {
    const detail = eventDetail(entry, several);
    if (detail) wrap.append(detail);
  }
  if (!ctx.github().connected && thread.items.some(entry => entry.target && entry.target_id)) {
    const connect = button(t("github.connect"), ctx.toSettings, "ghost"); connect.classList.add("sm");
    const notice = h("div", "notification-detail");
    notice.append(t("notifications.connect"), " ", connect);
    wrap.append(notice);
  }
  return wrap;
}

/// One event's current text; in a thread of several, each event also names who did what and when.
function eventDetail(item: GitHubNotification, labelled: boolean): HTMLElement | null {
  const detail = h("div", "notification-detail");
  if (labelled) {
    const head = h("div", "notification-event", `${headline(item)} · ${clock(item)}`);
    head.append(githubLink(item));
    detail.append(head);
  }
  const body = h("div", "");
  detail.append(body);
  const owner = cacheOwner();
  if (!item.target || !item.target_id) {
    if (!labelled) body.textContent = subjectLine(item);
  } else if (details.has(item.id)) {
    const text = details.get(item.id);
    if (text && item.target !== "workflow_run") { body.className = "md"; body.innerHTML = md(text, { html: true }); }
    else body.textContent = text || t("notifications.noText");
  } else if (failures.has(item.id)) {
    body.textContent = failures.get(item.id)!;
  } else if (ctx.github().connected) {
    body.textContent = t("issues.busy");
    if (fetching.has(item.id)) return detail;
    fetching.add(item.id);
    void invoke("github_detail", { repository: item.repository, number: item.number, target: item.target, id: item.target_id })
      .then(text => { if (cacheOwner() === owner) details.set(item.id, text); if (visible) draw(); })
      .catch(error => { if (cacheOwner() === owner) failures.set(item.id, fromBack(error)); if (visible) draw(); })
      .finally(() => fetching.delete(item.id));
  }
  if (!labelled && !body.textContent) return null;
  return detail;
}

function mentionRow(mention: ReturnType<typeof team.inboxList>[number]) {
  const row = template("button", "inboxrow notification-row unread",
    `<span class="av"></span><span class="txt"><b></b><span class="what"></span></span><span class="when"></span><span class="go">${icon("arrow-right", 14)}</span>`);
  row.querySelector(".av")!.innerHTML = avatar(mention.author);
  row.querySelector("b")!.textContent = t("inbox.from", { name: mention.author });
  row.querySelector(".what")!.textContent = mention.text || mention.title;
  row.querySelector(".when")!.textContent = new Date(mention.ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  row.title = t("inbox.open");
  row.addEventListener("click", () => {
    const at = team.readInbox(mention.id);
    if (at) ctx.openMention(at.workspace, at.note, at.tab);
  });
  return row;
}
