import { gitGroup } from "./components/git/group";
import { iconButton as componentIconButton } from "./components/icon-button";
import { gitFileRow } from "./components/git/file-row";
import { commitForm } from "./components/git/commit-form";
import { invoke } from "./ipc";
import { avatar, icon } from "./icons";
import { current as language, fromBack, t, tn, type Key } from "./i18n";
import { $, h, template } from "./util";
import { button as uiButton, confirmDialog, input as uiInput } from "./ui";
import * as diff from "./diff";
import * as menu from "./menu";
import { changesMenu, type Availability, type Refusal } from "./changes-menu";
import type * as fileMenu from "./file-menu";
import type { GitAction, GitConflict, GitDiff, GitFile, GitStatus, RepoDiff, Workspace } from "./types";

type Mode = "changes" | "staged" | "branches" | "history" | "compare" | "conflict";
type Selection = { path: string; scope: "staged" | "changes" | "conflict" };
type View = { repo: number; mode: Mode; selection: Selection | null; reference: string; filter: string; layout: "unified" | "split"; collapsed: Set<string>; messages: Map<number, string>; remotes: Map<number, string>; conflicts: Map<string, { source: GitConflict; text: string }> };
type Context = {
  workspace: () => Workspace | undefined;
  refresh: () => Promise<void>;
  show: () => void;
  say: (text: string, error?: boolean) => void;
  openFile: (repo: string, path: string) => void;
  launchBranch: (project: string, base: string, branch?: string) => void;
  openWorkspace: (id: string) => void;
  /// What a file of this repository offers beyond the panel: the conversation and its path, the
  /// same way the Files tree resolves them.
  fileHost: (repo: string) => fileMenu.Context;
};
let context: Context;
const views = new Map<string, View>();
const data = new Map<string, GitStatus[]>();
let busy = false, ticket = 0, sidebarSignature = "", editorSignature = "";
/// Remember a requested file scroll only for explicit navigation. Board redraws must preserve the reader's current position.
let pendingFocus = "";
let review: RepoDiff[] = [];

export function init(ctx: Context) {
  context = ctx;
  $("dlayout").setAttribute("aria-label", t("diff.layout"));
  for (const layout of ["unified", "split"] as const) {
    const control = uiButton(t(`diff.${layout}`), () => { state().layout = layout; void drawEditor(); }, "ghost");
    control.dataset.layout = layout;
    $("dlayout").append(control);
  }
  $("dnext").append(uiButton(t("diff.next"), () => {
    const ws = context.workspace(); if (!ws) return;
    const view = state(), all = review.flatMap(repo => repo.files.map(file => ({ repo: repo.name, file })));
    const start = all.findIndex(({ file }) => file.path === view.selection?.path);
    const next = [...all.slice(start + 1), ...all.slice(0, start + 1)].find(({ repo, file }) => !diff.isSeen(ws.id, repo, file));
    if (!next) return;
    view.filter = ""; pendingFocus = next.file.path;
    view.selection = { path: next.file.path, scope: view.mode === "staged" ? "staged" : "changes" };
    drawSidebar();
    void drawEditor();
  }, "ghost"));
  $("dfold").onclick = () => { diff.foldAll(diff.keys(review)); void drawEditor(); };
  $("dseen").onclick = () => {
    const ws = context.workspace();
    if (ws) diff.seeAll(ws.id, review);
    diff.invalidate(); void drawEditor();
  };
}

function state(): View {
  const id = context.workspace()!.id;
  if (!views.has(id)) views.set(id, { repo: 0, mode: "changes", selection: null, reference: "", filter: "", layout: "unified", collapsed: new Set(), messages: new Map(), remotes: new Map(), conflicts: new Map() });
  return views.get(id)!;
}
const current = () => data.get(context.workspace()?.id ?? "")?.find(repo => repo.repo === state().repo);
export const statuses = (id: string) => data.get(id);
export const count = (id: string) => (data.get(id) ?? []).reduce((n, repo) => n + new Set([...repo.staged, ...repo.changes, ...repo.conflicts].map(file => file.path)).size, 0);

function clearEditor() {
  ticket++; editorSignature = ""; review = []; heading(t("git.changes"));
  $("dlist").replaceChildren(h("div", "none", t("git.loading")));
}

export function enter() {
  sidebarSignature = ""; clearEditor();
  $("difflist").replaceChildren();
  if (context.workspace() && !context.workspace()!.remote) drawSidebar();
}

export function forget(alive: Set<string>) {
  for (const map of [views, data]) for (const id of map.keys()) if (!alive.has(id)) map.delete(id);
}

export function update(id: string, repos: GitStatus[]) {
  const previous = data.get(id);
  data.set(id, repos.map(repo => repo.error ? { ...(previous?.find(old => old.repo === repo.repo) ?? repo), error: repo.error } : repo));
  if (context.workspace()?.id !== id) return;
  const view = state();
  if (!repos.some(repo => repo.repo === view.repo)) view.repo = repos[0]?.repo ?? 0;
  const repo = current();
  if (view.selection && repo && !repo.error) {
    const selection = view.selection;
    const group = selection.scope === "conflict" ? repo.conflicts : repo[selection.scope];
    if (!group.some(file => file.path === selection.path)) view.selection = null;
  }
  drawSidebar();
  if (!$("diffview").hidden) void drawEditor();
}

export function fail(id: string, error: unknown) {
  const ws = context.workspace();
  const existing = data.get(id);
  if (existing) update(id, existing.map(repo => ({ ...repo, error: String(error) })));
  else if (ws?.id === id) {
    $("difflist").replaceChildren(h("div", "git-error", fromBack(error)));
    $("difflist").append(button(t("git.refresh"), () => void context.refresh()));
  }
}

export function show(mode?: Mode) {
  const ws = context.workspace();
  if (!ws || ws.remote || ws.cleaned) return;
  if (mode) {
    const view = state();
    view.mode = mode;
    if (mode !== "conflict") view.selection = null;
    view.filter = "";
  }
  clearEditor();
  if (mode === "compare") state().reference = current()?.base ?? "";
  context.show(); drawSidebar(); void drawEditor();
}

export function selectRepo(index: number) {
  state().repo = index; state().selection = null; state().reference = ""; pendingFocus = ""; show("changes");
}

function button(label: string, click: () => void, disabled = false, className = "") {
  const node = uiButton(label, click, className === "pri" ? "pri" : "ghost");
  if (className) node.classList.add(...className.split(" "));
  node.disabled = disabled;
  return node;
}

function iconButton(label: string, glyph: Parameters<typeof icon>[0], click: () => void, disabled = false) {
  const node = componentIconButton({ label, glyph, run: click, disabled });
  node.classList.add("git-icon");
  return node;
}

function selectFile(file: GitFile, scope: Selection["scope"]) {
  const view = state();
  const mode = scope === "conflict" ? "conflict" : scope;
  const stay = !$("diffview").hidden && view.mode === mode;
  view.selection = { path: file.path, scope };
  view.mode = mode;
  pendingFocus = scope === "conflict" ? "" : file.path;
  // Reuse the mounted diff when the scope is unchanged; only scroll.
  if (stay) { drawSidebar(); void drawEditor(); return; }
  show();
}

function drawSidebar() {
  const ws = context.workspace(); if (!ws || ws.remote) return;
  const view = state(), repo = current();
  for (const row of $("difflist").querySelectorAll<HTMLElement>(".git-file")) {
    const selected = row.dataset.path === view.selection?.path && row.closest<HTMLElement>(".git-group")?.dataset.scope === view.selection?.scope;
    row.classList.toggle("selected", selected);
    row.querySelector(".git-file-name")?.setAttribute("aria-current", String(selected));
  }
  const activeMode = view.mode === "conflict" ? "changes" : view.mode;
  for (const item of $("difflist").querySelectorAll<HTMLElement>(".git-nav button")) item.setAttribute("aria-current", String(item.dataset.mode === activeMode));
  const signature = JSON.stringify([ws.id, language(), view.repo, view.mode, view.filter, view.remotes.get(view.repo), data.get(ws.id), busy]);
  if (signature === sidebarSignature) return;
  sidebarSignature = signature;
  const list = $("difflist");
  const focused = list.contains(document.activeElement) ? document.activeElement as HTMLElement : null;
  const focusId = focused?.id;
  const focusMode = focused?.dataset.mode;
  const range = focused instanceof HTMLTextAreaElement || focused instanceof HTMLInputElement ? [focused.selectionStart, focused.selectionEnd] : null;
  list.classList.add("git-panel");
  list.replaceChildren();
  const picker = h("div", "git-repository");
  picker.append(template("span", "", avatar(repo?.name ?? ws.repo_name)));
  const select = button(repo?.name ?? ws.repos[view.repo]?.name ?? ws.repo_name, () => {
    const at = select.getBoundingClientRect();
    menu.openAt({ x: at.left, y: at.bottom + 4 }, ws.repos.map((item, index) => ({
      label: item.name, glyph: avatar(item.name), checked: index === view.repo,
      run: () => selectRepo(index),
    })));
  });
  select.setAttribute("aria-label", t("git.repository"));
  select.append(template("span", "spacer", ""), template("span", "", icon("chevron-down", 12)));
  picker.append(select, h("span", "spacer"), iconButton(t("git.refresh"), "rotate", () => { editorSignature = ""; void context.refresh(); }, busy)); list.append(picker);
  if (!repo) { list.append(h("div", "none", t("git.loading"))); return; }
  const target = { id: ws.id, repo: view.repo, index: repo.index };
  const act = (operation: GitAction, paths: string[] = [], remote?: string) => perform(target, operation, paths, remote);
  const disabled = busy || !!repo.error;
  // Read live, not at draw time: the agent's status does not redraw this list.
  const availability = (): Availability => {
    const latest = current();
    return { agentRunning: !!context.workspace()?.tabs.some(tab => tab.status === "rodando"), blocked: busy || !latest || !!latest.error };
  };
  // A blocked operation already shows its progress or error in the panel; only the agent needs saying.
  const refused = (why: Refusal) => { if (why === "agent") context.say(t("err.git.agent"), true); };
  const branch = button(repo.branch ?? t("git.detached"), () => show("branches"), false, "ghost git-branch");
  branch.prepend(template("span", "", icon("git-branch", 13)));
  branch.title = `${repo.branch ?? t("git.detached")} · ${t("git.branches")}`;
  branch.append(template("span", "", icon("chevron-down", 12)));
  const meta = h("div", "git-meta"); meta.append(branch, h("span", "spacer")); list.append(meta);
  if (repo.error) list.append(h("div", "git-error", fromBack(repo.error)));
  if (!repo.branch) list.append(h("div", "git-hint", t("err.git.detached")));
  const tools = h("div", "git-tools");
  const pullDisabled = disabled || !repo.branch || !repo.upstream || !!repo.conflicts.length || !!repo.staged.length || !!repo.changes.length || ws.tabs.some(tab => tab.status === "rodando");
  const pull = button(`↓${repo.behind}`, () => void act("pull"), pullDisabled, "ghost");
  pull.setAttribute("aria-label", t("git.pull", { n: repo.behind }));
  pull.title = ws.tabs.some(tab => tab.status === "rodando") ? t("err.git.agent") : repo.staged.length || repo.changes.length ? t("err.git.dirtyPull") : t("git.pull.hint");
  tools.append(pull);
  if (repo.upstream) {
    const push = button(t("git.push", { n: repo.ahead }), () => void act("push"), disabled || !repo.branch || !repo.ahead, "ghost");
    push.setAttribute("aria-label", t("git.push", { n: repo.ahead }));
    push.title = `${t("git.push.hint")} · ${repo.upstream}`; tools.append(push);
  } else {
    tools.append(button(t("git.publish"), () => {
      const remote = view.remotes.get(view.repo) ?? repo.remotes[0];
      void act("publish", [], remote);
    }, disabled || !repo.branch || !repo.has_head || !repo.remotes.length, "ghost"));
  }
  meta.append(tools);
  const more = iconButton(t("git.actions"), "ellipsis", () => {
    const at = more.getBoundingClientRect();
    menu.openAt({ x: at.right, y: at.bottom + 4 }, [
      { label: t("git.fetch"), hint: t("git.fetch.hint"), disabled: disabled || !repo.remotes.length, run: () => void act("fetch") },
      { label: t("git.pull", { n: repo.behind }), disabled: pullDisabled, run: () => void act("pull") },
      { label: t("git.push", { n: repo.ahead }), disabled: disabled || !repo.branch || !repo.upstream || !repo.ahead, run: () => void act("push") },
      "sep", { label: t("git.branches"), run: () => show("branches") },
    ]);
  });
  picker.append(more);
  if (!repo.upstream && repo.remotes.length > 1) {
    const remotes = button(view.remotes.get(view.repo) ?? repo.remotes[0], () => {
      const at = remotes.getBoundingClientRect();
      menu.openAt({ x: at.left, y: at.bottom + 4 }, repo.remotes.map(remote => ({
        label: remote, checked: remote === (view.remotes.get(view.repo) ?? repo.remotes[0]),
        run: () => { view.remotes.set(view.repo, remote); drawSidebar(); },
      })));
    }, busy, "git-remotes");
    remotes.id = "git-remote"; remotes.setAttribute("aria-label", t("git.remote")); list.append(remotes);
  }
  const destination = repo.upstream ?? (repo.remotes.length && repo.branch ? `${view.remotes.get(view.repo) ?? repo.remotes[0]}/${repo.branch}` : t("git.noUpstream"));
  const upstream = h("div", "git-hint git-upstream", destination);
  upstream.title = t("git.destination", { name: destination }); list.append(upstream);
  const nav = h("div", "git-nav");
  nav.setAttribute("aria-label", t("git.scope"));
  for (const mode of ["changes", "staged", "history", "compare"] as const) {
    const item = button(t(`git.tab.${mode}`), () => show(mode), false, "ghost");
    item.dataset.mode = mode; item.setAttribute("aria-current", String(mode === activeMode));
    if (mode === "changes" || mode === "staged") item.append(h("span", "git-count", String(repo[mode].length)));
    nav.append(item);
  }
  list.append(nav);
  const local = activeMode === "changes" || activeMode === "staged";
  if (activeMode === "history") {
    list.append(h("div", "git-history-list", t("git.loading")));
  } else if (local || activeMode === "compare") {
    const search = uiInput(view.filter); search.type = "search"; search.id = "git-filter";
    search.placeholder = t("git.filter"); search.setAttribute("aria-label", t("git.filter"));
    search.oninput = () => { view.filter = search.value; drawSidebar(); void drawEditor(); };
    const filter = h("div", "git-filter"); filter.append(search);
    const clear = iconButton(t("git.filter.clear"), "x", () => {
      view.filter = ""; drawSidebar(); void drawEditor(); $("git-filter").focus();
    });
    clear.hidden = !view.filter; filter.append(clear); list.append(filter);
  }
  if (activeMode === "compare") list.append(h("div", "git-comparison-files"));
  const scopes: Selection["scope"][] = local ? ["conflict", activeMode === "staged" ? "staged" : "changes"] : [];
  for (const scope of scopes) {
    const group = scope === "conflict" ? repo.conflicts : repo[scope];
    if (scope === "conflict" && !group.length) continue;
    const groupKey = `${view.repo}/${scope}`;
    const { root: section, body } = gitGroup({ scope,
      title: scope === "conflict" ? t("git.conflicts") : tn(group.length, "diff.files"),
      collapsed: view.collapsed.has(groupKey),
      changed: collapsed => { if (collapsed) view.collapsed.add(groupKey); else view.collapsed.delete(groupKey); },
      action: scope === "conflict" ? undefined : {
        label: t(scope === "staged" ? "git.unstageAll" : "git.stageAll"), disabled: disabled || !group.length,
        run: () => void act(scope === "staged" ? "unstage" : "stage", group.map(file => file.path)),
      },
    });
    const visible = group.filter(file => file.path.toLowerCase().includes(view.filter.toLowerCase()));
    for (const file of visible) {
      const selected = view.selection?.path === file.path && view.selection.scope === scope;
      const row = gitFileRow({
        path: file.path, status: file.status, statusLabel: t(`git.status.${file.status}` as Key), selected,
        select: () => selectFile(file, scope),
        open: file.status !== "D" ? () => context.openFile(repo.name, file.path) : undefined,
        contextMenu: event => {
        const host = context.fileHost(repo.name);
        menu.openAt({ x: event.clientX, y: event.clientY }, changesMenu(file, {
          ...host, scope, availability,
          hooks: {
            ...host.hooks, refused,
            review: () => selectFile(file, scope),
            open: () => context.openFile(repo.name, file.path),
            stage: () => void act("stage", [file.path]),
            unstage: () => void act("unstage", [file.path]),
            discard: () => void discard(file.path, () => act("discard", [file.path])),
          },
        }));
        },
        action: scope !== "conflict" ? {
          label: t(scope === "staged" ? "git.unstage.short" : "git.stage.short"),
          description: `${t(scope === "staged" ? "git.unstage" : "git.stage")}: ${file.path}`, disabled,
          run: () => void act(scope === "staged" ? "unstage" : "stage", [file.path]),
        } : undefined,
      });
      body.append(row);
    }
    if (!visible.length) body.append(h("div", "git-hint", t(group.length ? "git.filter.empty" : "git.empty")));
    list.append(section);
  }
  if (local) {
    const composer = h("div", activeMode === "staged" ? "" : "git-composer");
    if (activeMode === "staged") {
      composer.append(commitForm({
        message: view.messages.get(view.repo) ?? "", label: t("git.message"), placeholder: t("git.message.placeholder"),
        submit: repo.merging ? t("git.commit.merge") : tn(repo.staged.length, "git.commit.files"),
        hint: t("git.commit.hint"),
        title: t(repo.conflicts.length ? "git.conflict.hint" : repo.staged.length ? "git.commit.hint" : "git.stage.hint"),
        busy, disabled: disabled || !repo.branch || !!repo.conflicts.length || (!repo.staged.length && !repo.merging),
        changed: message => view.messages.set(view.repo, message), commit: () => void act("commit"),
      }));
    } else {
      composer.append(h("strong", "", repo.staged.length ? tn(repo.staged.length, "git.ready") : t("git.prepare")), h("p", "git-hint", t("git.stage.hint")));
      if (repo.staged.length || repo.merging) composer.append(button(t("git.reviewStaged"), () => show("staged")));
    }
    list.append(composer);
  }
  if (busy) list.append(h("div", "git-hint", t("git.busy")));
  if (focusId) {
    const control = document.getElementById(focusId); control?.focus({ preventScroll: true });
    if (range && (control instanceof HTMLTextAreaElement || control instanceof HTMLInputElement)) control.setSelectionRange(range[0], range[1]);
  } else if (focusMode) list.querySelector<HTMLElement>(`[data-mode="${focusMode}"]`)?.focus({ preventScroll: true });
  syncReview();
}

/// Discarding loses work Git cannot bring back, so it always asks first.
async function discard(path: string, run: () => Promise<void>) {
  const confirmed = await confirmDialog({
    title: t("git.menu.discard.title", { path }), message: t("git.menu.discard.body"),
    accept: t("git.menu.discard"), cancel: t("actions.cancel"),
  });
  if (confirmed) await run();
}

async function perform(target: { id: string; repo: number; index: string }, operation: GitAction, paths: string[] = [], remote?: string) {
  const ws = context.workspace(), repo = current(); if (!ws || !repo || repo.error || busy) return;
  if (ws.id !== target.id || state().repo !== target.repo || repo.index !== target.index) { context.say(t("err.git.changed"), true); return; }
  const view = state(), selectedRepo = target.repo, message = view.messages.get(selectedRepo) ?? "";
  busy = true; drawSidebar();
  try {
    await invoke("workspace_git_action", { id: ws.id, repo: selectedRepo, operation, paths, message, expected: target.index, remote: remote ?? null });
    if (operation === "commit") view.messages.delete(selectedRepo);
    if ((operation === "stage" || operation === "unstage") && view.selection && paths.includes(view.selection.path)) {
      view.selection = null;
      if (view.mode === "conflict") view.mode = "staged";
    }
    context.say(t("git.done"));
  } catch (error) { context.say(fromBack(error), true); }
  finally {
    busy = false; await context.refresh(); drawSidebar();
    if ((operation === "stage" || operation === "unstage") && context.workspace()?.id === target.id && state().repo === target.repo) {
      $("difflist").querySelector<HTMLElement>(`.git-nav [data-mode="${state().mode}"]`)?.focus({ preventScroll: true });
    }
  }
}

function heading(title: string, reader = false) {
  $("dcrumb").replaceChildren(h("span", "nm", title));
  $("dcrumb").title = "";
  if (!reader) for (const id of ["dseen", "dfold", "dlayout", "dprogress", "dnext"]) $(id).hidden = true;
}

function syncReview() {
  const ws = context.workspace(); if (!ws) return;
  const total = diff.keys(review).length, remaining = diff.unseen(ws.id, review);
  const progress = h("progress", "") as HTMLProgressElement;
  progress.max = Math.max(1, total); progress.value = total - remaining;
  progress.setAttribute("aria-label", t("diff.progress", { n: total - remaining, total }));
  $("dprogress").replaceChildren(progress, h("span", "", t("diff.progress", { n: total - remaining, total })));
  ($("dnext").firstElementChild as HTMLButtonElement).disabled = remaining === 0;
  for (const button of $("dlayout").querySelectorAll<HTMLButtonElement>("button")) button.setAttribute("aria-pressed", String(button.dataset.layout === state().layout));
  for (const row of $("difflist").querySelectorAll<HTMLElement>(".git-file")) {
    const repo = review.find(repo => repo.name === current()?.name), file = repo?.files.find(file => file.path === row.dataset.path);
    row.classList.toggle("reviewed", !!repo && !!file && diff.isSeen(ws.id, repo.name, file));
  }
}

function renderReview(ws: Workspace, repo: GitStatus, result: GitDiff, caption: string, empty: string) {
  const view = state(), host = $("dlist");
  let target = host.querySelector<HTMLElement>(".git-review-list");
  if (!target) {
    target = h("div", "git-review-list dlist");
    host.replaceChildren(target, h("div", "git-review-scope")); diff.invalidate();
  }
  host.querySelector<HTMLElement>(".git-review-scope")!.textContent = caption;
  review = [{ name: repo.name, base: view.reference, ahead: 0, unpushed: repo.ahead, dirty: 0, files: result.files }];
  const filtered = review.map(repo => ({ ...repo, files: repo.files.filter(file => file.path.toLowerCase().includes(view.filter.toLowerCase())) }));
  const focus = pendingFocus; pendingFocus = "";
  diff.render(target, {
    id: ws.id, repos: filtered, layout: view.layout, focus: focus ? diff.key(repo.name, focus) : undefined,
    empty: result.files.length && view.filter ? t("git.filter.empty") : empty,
    onSeen: syncReview, onOpen: context.openFile,
  });
  if (view.mode === "compare") {
    const files = $("difflist").querySelector<HTMLElement>(".git-comparison-files");
    if (files) {
      const previous = files.querySelector<HTMLElement>(":focus")?.dataset.path;
      files.replaceChildren(...filtered[0].files.map(file => {
        const pick = button(file.path, () => {
          pendingFocus = file.path; void drawEditor();
        }, false, "git-file-name");
        pick.dataset.path = file.path; pick.title = file.path; return pick;
      }));
      if (previous) files.querySelector<HTMLElement>(`[data-path="${CSS.escape(previous)}"]`)?.focus({ preventScroll: true });
      if (!files.children.length) files.append(h("p", "git-hint", t(result.files.length ? "git.filter.empty" : "git.empty")));
    }
  }
  syncReview();
  for (const id of ["dseen", "dfold", "dlayout", "dprogress", "dnext"]) $(id).hidden = !result.files.length;
}

async function drawEditor() {
  const ws = context.workspace(), repo = current(); if (!ws || ws.remote || $("diffview").hidden) return;
  const view = state(), mode = view.mode;
  if (mode === "compare" && document.activeElement?.classList.contains("git-base")) return;
  const mine = ++ticket;
  const selectedRepo = view.repo;
  const valid = () => mine === ticket && context.workspace()?.id === ws.id && state().repo === selectedRepo && !$("diffview").hidden;
  const host = $("dlist"); host.classList.add("git-content");
  if (!repo) { clearEditor(); return; }
  if (repo.error) { editorSignature = ""; heading(t("git.changes")); host.replaceChildren(h("div", "git-error", fromBack(repo.error))); return; }
  const args = { id: ws.id, repo: view.repo };
  const act = (operation: GitAction, paths: string[] = []) => perform({ ...args, index: repo.index }, operation, paths);
  try {
    if (mode === "branches") {
      if (editorSignature === `${ws.id}/${view.repo}/branches`) return;
      const branches = await invoke("workspace_git_branches", args); if (!valid()) return;
      heading(t("git.branches"));
      const box = h("div", "git-page"), search = uiInput(); search.classList.add("git-search");
      search.placeholder = t("git.branch.search"); search.setAttribute("aria-label", t("git.branch.search"));
      const list = h("div", "git-branches");
      const render = () => {
        list.replaceChildren();
        for (const branch of branches.filter(branch => branch.name.toLowerCase().includes(search.value.toLowerCase()))) {
          const row = h("div", "git-branch-row"); row.append(h("code", "", branch.name), h("span", "spacer"));
          if (branch.current) row.append(h("span", "git-badge", t("git.branch.current")));
          else if (branch.workspace) row.append(button(t("git.branch.open"), () => context.openWorkspace(branch.workspace!)));
          else if (branch.worktree) { const label = h("span", "git-hint", t("git.branch.occupied")); label.title = branch.worktree; row.append(label); }
          else row.append(button(t("git.branch.create"), () => context.launchBranch(ws.repos[args.repo].path, branch.name, branch.remote ? undefined : branch.name)));
          if (branch.remote) row.append(h("span", "git-badge", t("git.branch.remote")));
          list.append(row);
        }
        if (!list.children.length) list.append(h("div", "none", t("git.branch.none")));
      };
      search.oninput = render; render();
      box.append(search, list, button(t("git.branch.new"), () => context.launchBranch(ws.repos[args.repo].path, repo.branch ?? "HEAD")), h("p", "git-hint", t("git.branch.hint")));
      host.replaceChildren(box); editorSignature = `${ws.id}/${view.repo}/branches`; return;
    }
    if (mode === "history") {
      const history = await invoke("workspace_git_history", args); if (!valid()) return;
      if (!history.some(commit => commit.oid === view.reference)) view.reference = history[0]?.oid ?? "";
      const box = $("difflist").querySelector<HTMLElement>(".git-history-list")!;
      const focused = box.querySelector<HTMLElement>(":focus")?.dataset.oid;
      box.replaceChildren(h("p", "git-hint", t("git.history.limit")));
      for (const commit of history) {
        const row = button("", () => {
          view.reference = commit.oid;
          const list = host.querySelector<HTMLElement>(".git-review-list"); if (list) list.scrollTop = 0;
          void drawEditor();
        }, false, "git-history-row");
        row.dataset.oid = commit.oid; row.setAttribute("aria-current", String(commit.oid === view.reference));
        const description = h("div", ""); description.append(h("strong", "", commit.subject), h("small", "", `${commit.oid.slice(0,7)} · ${commit.author} · ${new Date(commit.date).toLocaleString()}`));
        row.append(h("span", "git-history-node"), description, h("span", "spacer"));
        if (commit.outgoing) row.append(h("span", "git-badge", t("git.outgoing")));
        box.append(row);
      }
      if (!history.length) box.append(h("div", "none", t("git.history.empty")));
      if (focused) box.querySelector<HTMLElement>(`[data-oid="${CSS.escape(focused)}"]`)?.focus({ preventScroll: true });
      if (!history.length) { heading(t("git.history")); host.replaceChildren(h("div", "none", t("git.history.empty"))); return; }
    }
    if (mode === "compare" || mode === "history") {
      heading(mode === "compare" ? t("git.compare") : t("git.saved"), true);
      if (mode === "compare") {
        const base = uiInput(view.reference || repo.base); base.classList.add("git-base"); base.setAttribute("aria-label", t("git.base"));
        const apply = () => {
          view.reference = base.value; base.blur(); editorSignature = "";
          void drawEditor().then(() => {
            if (context.workspace()?.id === ws.id && state().repo === args.repo && state().mode === "compare" && !$("diffview").hidden) $("dcrumb").querySelector<HTMLElement>(".git-base")?.focus();
          });
        };
        base.onkeydown = e => { if (e.key === "Enter") apply(); };
        $("dcrumb").append(base, button(t("git.tab.compare"), apply));
      }
      if (!repo.has_head) { host.replaceChildren(h("div", "none", t("git.history.empty"))); return; }
      const result = await invoke("workspace_git_diff", { ...args, scope: mode === "compare" ? "compare" : "commit", path: null, reference: view.reference || null }); if (!valid()) return;
      const caption = mode === "compare" ? t("git.compare.scope", { base: view.reference || repo.base }) : `${t("git.saved")} · ${result.head.slice(0, 7)}`;
      $("dcrumb").title = caption;
      renderReview(ws, repo, result, caption, t("git.compare.empty")); return;
    }
    if (mode === "conflict" && view.selection?.scope === "conflict") {
      const selected = view.selection;
      const signature = `${ws.id}/${view.repo}/conflict/${selected.path}`;
      if (signature === editorSignature) return;
      const result = await invoke("workspace_git_conflict", { ...args, path: selected.path }); if (!valid()) return;
      const draftKey = `${args.repo}/${selected.path}`;
      let draft = view.conflicts.get(draftKey);
      if (!draft) { draft = { source: result, text: result.current }; view.conflicts.set(draftKey, draft); }
      const saved = draft;
      heading(`${t("git.conflicts")} · ${selected.path}`);
      const box = h("div", "git-page"), sides = h("div", "git-conflict-sides");
      const input = uiInput("", true); input.classList.add("git-conflict-result");
      input.value = saved.text; input.oninput = () => { saved.text = input.value; }; input.setAttribute("aria-label", t("git.conflict.result")); input.spellcheck = false;
      for (const [text, key] of [[saved.source.ours, "ours"], [saved.source.theirs, "theirs"]] as const) {
        const side = h("section", "git-conflict-side");
        side.append(h("h4", "", t(key === "ours" ? "git.conflict.current" : "git.conflict.incoming", { branch: repo.branch ?? "HEAD" })), h("pre", "", text ?? t("git.conflict.deleted")), button(t(`git.conflict.${key}`), () => { input.value = text!; saved.text = text!; }, text === null)); sides.append(side);
      }
      const resolve = button(t("git.conflict.resolve"), () => {
        if (busy || context.workspace()?.id !== args.id || state().repo !== args.repo || current()?.error) return; busy = true; resolve.disabled = true; drawSidebar();
        void invoke("workspace_git_resolve", { ...args, path: selected.path, was: saved.source.current, text: input.value }).then(() => {
          view.conflicts.delete(draftKey);
          if (context.workspace()?.id === ws.id && state().repo === args.repo) { view.mode = "staged"; view.selection = { scope: "staged", path: selected.path }; editorSignature = ""; }
        }).catch(error => context.say(fromBack(error), true)).finally(async () => { busy = false; resolve.disabled = false; await context.refresh(); });
      }, busy, "pri");
      if (saved.source.current !== result.current) {
        box.append(h("p", "git-error", t("err.git.changed")), button(t("git.conflict.refresh"), () => { saved.source = result; editorSignature = ""; void drawEditor(); }));
      }
      box.append(sides, h("label", "git-hint", t("git.conflict.result")), input, resolve, button(t("git.conflict.manual"), () => void act("stage", [selected.path]), busy), h("p", "git-hint", t("git.conflict.hint")));
      host.replaceChildren(box); editorSignature = signature; return;
    }
    heading(t(view.mode === "staged" ? "git.tab.staged" : "git.tab.changes"), true);
    if (view.mode === "conflict" && !view.selection && repo.conflicts.length) {
      view.selection = { path: repo.conflicts[0].path, scope: "conflict" }; view.mode = "conflict";
      drawSidebar(); return void drawEditor();
    }
    if (!repo.staged.length && !repo.changes.length && !repo.conflicts.length && !repo.merging) {
      review = []; heading(t("git.changes"));
      host.replaceChildren(h("div", "git-clean", t("git.clean")), h("p", "git-clean-hint", t("git.clean.hint"))); editorSignature = ""; return;
    }
    // The explicit tab selects the reviewed snapshot, even when empty.
    const scope = view.mode === "staged" ? "staged" : "changes";
    const result = await invoke("workspace_git_diff", { ...args, scope, path: null, reference: null }); if (!valid()) return;
    renderReview(ws, repo, result, t(scope === "staged" ? "git.scope.staged" : "git.scope.changes"), t("git.empty"));
  } catch (error) {
    if (!valid()) return;
    editorSignature = ""; review = [];
    for (const id of ["dseen", "dfold", "dlayout", "dprogress", "dnext"]) $(id).hidden = true;
    if (mode !== "compare") heading(t(mode === "conflict" ? "git.conflicts" : "git.changes"));
    host.replaceChildren(h("div", "git-error", fromBack(error)));
    if (mode === "conflict" && view.selection) {
      const path = view.selection.path;
      host.append(button(t("git.openFile"), () => context.openFile(repo.name, path)), button(t("git.conflict.manual"), () => void act("stage", [path]), busy));
    }
  }
}
