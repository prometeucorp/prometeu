import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { button, checkbox, field, formDialog, input, select } from "./ui";
import { issueAction, issueTab, issueSearch, issueRefresh, issueFilter } from "./issues-controls";
import { icon } from "./icons";
import { empty, h, template } from "./util";
import { githubWorkspace } from "./github-issues-model";
import type { Board, GitHubItem, GitHubIssues, GitHubPrepared, GitHubScope, Workspace } from "./types";

type Context = {
  board: () => Board;
  open: (workspace: Workspace) => void;
  create: (item: GitHubItem, project: string, git?: GitHubPrepared) => void;
  say: (message: string, error?: boolean) => void;
  redraw: () => void;
};

export function githubIssues(host: HTMLElement, ctx: Context) {
  let scope: GitHubScope = "mine", login = "", selected: string[] = [], query = "", repository = "";
  let visible = false, loading = false, revision = 0, error = "", opening = false;
  const snapshots = new Map<GitHubScope, GitHubIssues>();
  const tabs = h("div", "subbar tabbar itabs"); tabs.setAttribute("role", "tablist"); tabs.setAttribute("aria-label", t("github.scope"));
  const bar = h("div", "subbar ibar");
  const searchField = issueSearch(t("issues.search"));
  const search = searchField.control;
  const meta = h("span", "imeta"); meta.setAttribute("role", "status");
  const refreshButton = issueRefresh(t("issues.refresh"), () => void refresh(true));
  const repositoriesButton = issueAction(t("github.repositories.choose"), () => void chooseRepositories());
  bar.append(searchField.root, h("span", "spacer"), meta, repositoriesButton, refreshButton);
  const filters = h("div", "iteams");
  const list = h("div", "ilist"); list.id = "github-list"; list.setAttribute("role", "tabpanel");
  const controls = new Map<GitHubScope, HTMLButtonElement>();
  for (const key of ["mine", "repositories", "authored", "reviews"] as const) {
    const control = issueTab(t(`github.${key}`), () => {
      scope = key; repository = ""; error = ""; draw(); void refresh(false);
    });
    control.id = `github-${key}`; control.setAttribute("aria-controls", list.id);
    controls.set(key, control); tabs.append(control);
  }
  tabs.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const keys = [...controls.keys()];
    const index = event.key === "Home" ? 0 : event.key === "End" ? keys.length - 1 : (keys.indexOf(scope) + (event.key === "ArrowRight" ? 1 : -1) + keys.length) % keys.length;
    controls.get(keys[index])!.click(); controls.get(keys[index])!.focus();
  });
  search.oninput = () => { query = search.value.toLowerCase().trim(); drawList(); };
  search.onkeydown = event => { if (event.key === "Escape") { query = search.value = ""; drawList(); } };
  host.append(tabs, bar, filters, list);

  async function refresh(force: boolean) {
    const request = ++revision, requestedScope = scope;
    loading = true; error = ""; draw();
    try {
      const found = await invoke("github_issues", { scope: requestedScope, force });
      if (request !== revision) return;
      if (login !== found.login) { snapshots.clear(); repository = ""; }
      login = found.login; selected = found.repositories;
      snapshots.set(requestedScope, found);
    } catch (cause) {
      if (request !== revision) return;
      // gh can switch accounts outside the app. Never retain a previous account's private results on failure.
      snapshots.clear(); login = ""; selected = []; error = fromBack(cause);
    }
    loading = false; draw(); ctx.redraw();
  }

  function draw() {
    if (!visible) return;
    controls.forEach((control, key) => {
      const active = key === scope;
      control.classList.toggle("on", active); control.setAttribute("aria-selected", String(active)); control.tabIndex = active ? 0 : -1;
      const count = snapshots.get(key)?.items.length;
      control.children[0].textContent = t(`github.${key}`);
      (control.children[1] as HTMLElement).hidden = count === undefined;
      control.children[1].textContent = count === undefined ? "" : String(count);
      if (active) list.setAttribute("aria-labelledby", control.id);
    });
    meta.classList.toggle("err", !!error); meta.textContent = loading ? t("issues.busy") : error || (login ? `@${login}` : "");
    refreshButton.disabled = loading; repositoriesButton.disabled = loading || !login;
    drawList();
  }

  function drawList() {
    list.replaceChildren(); filters.replaceChildren(); filters.hidden = true;
    const snapshot = snapshots.get(scope);
    list.setAttribute("aria-busy", String(loading));
    if (!snapshot) {
      if (error) list.append(empty(t("github.unavailable"), error, [t("issues.failed.action"), () => void refresh(true)]));
      return;
    }
    if (snapshot.truncated) list.append(h("p", "ui-notice warning", t("github.truncated")));
    const repositories = [...new Set(snapshot.items.map(item => item.repository))].sort();
    if (repository && !repositories.includes(repository)) repository = "";
    filters.hidden = repositories.length < 2;
    const matching = snapshot.items.filter(item => query.split(/\s+/).every(word => `${item.identifier} ${item.title} ${item.repository} ${item.labels.join(" ")}`.toLowerCase().includes(word)));
    if (!filters.hidden) {
      const label = template("span", "tlabel", `${icon("filter", 13)}<span></span>`);
      label.children[1].textContent = t("github.repositories.label");
      const pills = h("div", "tpills");
      for (const name of ["", ...repositories]) {
        const count = name ? matching.filter(item => item.repository === name).length : matching.length;
        const control = issueFilter(name || t("github.repositories.all"), count, name === repository, () => {
          repository = name; drawList();
          [...filters.querySelectorAll<HTMLButtonElement>(".tpill")].find(control => control.getAttribute("aria-pressed") === "true")?.focus();
        });
        pills.append(control);
      }
      filters.append(label, pills);
    }
    const found = matching.filter(item => !repository || item.repository === repository);
    if (!found.length) {
      list.append(scope === "repositories" && !selected.length
        ? empty(t("github.repositories.empty"), t("github.repositories.hint"), [t("github.repositories.choose"), () => void chooseRepositories()])
        : empty(t("github.empty"), t(query || repository ? "issues.noMatch.body" : "github.empty.hint")));
      return;
    }
    for (const item of found) {
      const row = template("div", "irow github-row", `<span class="github-glyph"></span><span class="iid"></span><span class="ititle"><b></b><span class="iproj"></span></span><span class="istate"><i class="dot"></i><span></span></span><span class="iact"></span><span class="iago"></span>`);
      row.tabIndex = 0; row.setAttribute("role", "link"); row.setAttribute("aria-label", item.title); row.title = item.description ?? item.title;
      row.querySelector(".github-glyph")!.innerHTML = icon(item.kind === "pr" ? "git-pull-request" : "inbox", 14);
      row.querySelector(".iid")!.textContent = `#${item.number}`;
      row.querySelector(".ititle b")!.textContent = item.title; row.querySelector(".iproj")!.textContent = item.repository;
      row.querySelector(".istate span")!.textContent = t(item.draft ? "github.draft" : scope === "reviews" ? "github.reviewRequested" : "github.open");
      (row.querySelector(".istate .dot") as HTMLElement).style.background = item.draft ? "var(--fg-3)" : "var(--done)";
      row.querySelector(".iago")!.textContent = new Date(item.updated_at).toLocaleDateString(undefined, { month: "short", day: "numeric" });
      const visit = () => void invoke("github_issue_open", { url: item.url }).catch(cause => ctx.say(fromBack(cause), true));
      row.onclick = visit;
      row.onkeydown = event => { if (event.key === "Enter" && event.target === row) visit(); };
      const action = button(t("issues.open"), () => void launch(item), "ghost"); action.classList.add("sm"); action.dataset.focus = item.id;
      action.addEventListener("click", event => event.stopPropagation()); action.disabled = opening;
      row.querySelector(".iact")!.append(action); list.append(row);
    }
  }

  async function chooseRepositories() {
    const account = login;
    try {
      const projects = await invoke("github_projects");
      const choices = new Map<string, HTMLInputElement>();
      const extra = input("", true); extra.rows = 3; extra.placeholder = "owner/repository";
      const dialog = formDialog({ title: t("github.repositories.choose"), save: t("github.repositories.save"), cancel: t("actions.cancel"), error: fromBack,
        submit: async () => {
          const names = [...choices].filter(([, control]) => control.checked).map(([name]) => name);
          const updated = await invoke("github_repositories", { login: account, selected: [...names, ...extra.value.split(/[\s,]+/).filter(Boolean)] });
          ++revision; selected = updated; snapshots.delete("repositories"); loading = false;
          await refresh(true);
        },
      });
      dialog.body.append(h("p", "ui-hint", t("github.repositories.hint")));
      for (const name of [...new Set([...selected, ...projects.map(project => project.repository)])].sort()) {
        const check = checkbox(name, selected.includes(name)); choices.set(name, check.control); dialog.body.append(check.label);
      }
      dialog.body.append(field(t("github.repositories.other"), extra, t("github.repositories.format"))); dialog.open();
    } catch (cause) { ctx.say(fromBack(cause), true); }
  }

  async function launch(item: GitHubItem) {
    if (opening) return;
    const existing = githubWorkspace(item, ctx.board().workspaces);
    if (existing) { ctx.open(existing); return; }
    opening = true; drawList();
    try {
      const mappings = await invoke("github_projects");
      const matches = mappings.filter(mapping => mapping.repository.toLowerCase() === item.repository.toLowerCase());
      const options = ctx.board().projects.filter(project => matches.some(match => match.project === project.id));
      if (options.length === 1) { await start(options[0].id); return; }
      const picker = select(options[0]?.id ?? "", options.map(project => [project.id, `${project.name} · ${project.path}`]));
      const dialog = formDialog({ title: t("github.project.choose"), save: t("issues.open"), cancel: t("actions.cancel"), error: fromBack,
        submit: async () => {
          const project = picker.value;
          if (!project) throw new Error(t("github.project.required"));
          await start(project, dialog.close);
        },
      });
      dialog.body.append(h("p", "ui-hint", t("github.project.hint", { repository: item.repository })), field(t("launcher.project"), picker.control));
      dialog.save.disabled = !options.length;
      const add = button(t("github.project.add"), async () => {
        try {
          const path = await open({ directory: true, title: t("say.pickRepo") });
          if (!path || typeof path !== "string") return;
          const project = await invoke("add_project", { path });
          const mappings = await invoke("github_projects");
          if (!mappings.some(mapping => mapping.project === project.id && mapping.repository === item.repository.toLowerCase())) { ctx.say(t("err.github.project"), true); return; }
          // The board event can arrive after the command response.
          if (!ctx.board().projects.some(value => value.id === project.id)) ctx.board().projects.push(project);
          dialog.close(); await start(project.id);
        } catch (cause) { ctx.say(fromBack(cause), true); }
      });
      dialog.body.append(add); dialog.open();
      async function start(project: string, before = () => {}) {
        const git = item.kind === "pr" ? await invoke("github_prepare", { project, url: item.url }) : undefined;
        before();
        ctx.create(item, project, git);
      }
    } catch (cause) { ctx.say(fromBack(cause), true); }
    finally { opening = false; drawList(); }
  }

  return {
    show() { visible = true; host.hidden = false; draw(); void refresh(false); },
    hide() { visible = false; host.hidden = true; },
    draw,
    count: () => snapshots.get("mine")?.items.length ?? null,
  };
}
