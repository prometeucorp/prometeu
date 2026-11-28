import { open } from "@tauri-apps/plugin-dialog";
import * as catalog from "./catalog";
import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import { button, checkbox, field, formDialog, input, select } from "./ui";
import { itemRow, listState, toolbar } from "./components/compositions";
import * as machines from "./machines";
import { icon } from "./icons";
import type { Project } from "./types";
import { h } from "./util";

export type CatalogProject = {
  id: string; source: string; note: string; revision: number | null;
  organization: string | null; organization_name: string | null; local_path: string | null;
};

let selectedMachine = "";

export function settingsRows(projects: Project[], say: (text: string, bad?: boolean) => void, redraw: () => void): HTMLElement[] {
  const options: [string, string][] = [["", t("machines.local")], ...machines.machines().map(machine => [machine.id, machine.name] as [string, string])];
  // Keep registered projects visible even if their machine is no longer configured.
  for (const project of projects) if (project.host && !options.some(([id]) => id === project.host)) options.push([project.host, machines.machineName(project.host)]);
  const machine = select(selectedMachine, options);
  machine.control.dataset.track = "projects.machine";
  machine.control.dataset.focus = "projects-machine";
  machine.onchange = () => { selectedMachine = machine.value; redraw(); };
  const host = selectedMachine || undefined;
  const add = button(t(host ? "projects.remoteFolder" : "projects.add"), () => host ? openRemoteProjects(host, projects, () => {}) : openProjects(say), undefined, "projects.add");
  add.id = "projects-add";
  add.disabled = !!host && !machines.online(host);
  const actions = [add];
  if (host) {
    const clone = button(t("projects.clone"), () => openRemoteClone(host, () => {}), "outline", "projects.clone");
    clone.dataset.focus = "projects-clone";
    clone.disabled = !machines.online(host);
    actions.push(clone);
  }
  const shown = machines.projectsOn(projects, host);
  return [
    toolbar([field(t("projects.machine"), machine.control)], actions),
    ...(host && !machines.online(host) ? [h("p", "ui-hint", t("projects.offline", { name: machines.machineName(host) }))] : []),
    ...shown.map(project => itemRow({ title: project.name, description: project.path, glyph: "folder" }).root),
    listState(shown.length ? { kind: "ready" } : { kind: "empty", text: t("projects.none") }),
  ];
}

/** Register existing repositories without putting filesystem management in the workspace form. */
export function openRemoteProjects(machine: string, projects: Project[], added: (project: Project) => void) {
  let path = "";
  let loading = false;
  let saving = false;
  const picked = new Set<string>();
  const dialog = formDialog({ track: "projects.remoteFolder", area: "projects",
    title: t("launcher.remoteBrowse", { name: machines.machineName(machine) }), save: t("launcher.remoteAdd"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      if (!picked.size) throw t("projects.selectFolder");
      saving = true;
      for (const control of dialog.body.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input,button")) control.disabled = true;
      try {
        for (const folder of [...picked]) {
          const project = await invoke("remote_add_project", { machine, path: folder, url: null });
          if (!projects.some(existing => existing.id === project.id)) projects.push(project);
          picked.delete(folder);
          added(project);
        }
      } finally { saving = false; if (picked.size) await openFolder(path); }
    },
  });
  const up = button("", () => void openFolder(path.replace(/\/[^/]*\/?$/, "") || "/"), "ghost", "launcher.remoteUp");
  up.innerHTML = icon("arrow-up", 14);
  up.setAttribute("aria-label", t("launcher.remoteUp"));
  const location = h("span", "launcher-browse-path");
  const head = h("div", "launcher-browse-head"); head.append(up, location);
  const rows = h("div", "launcher-browse-list");
  const feedback = h("p", "ui-hint bad"); feedback.setAttribute("role", "alert");
  const retry = button(t("projects.retry"), () => void openFolder(path), "outline", "projects.retry"); retry.hidden = true;
  dialog.body.append(head, rows, feedback, retry);
  function update() { dialog.save.disabled = loading || saving || !picked.size; }
  async function openFolder(next: string) {
    if (loading || saving) return;
    loading = true; update(); up.disabled = true; retry.hidden = true;
    feedback.textContent = "";
    rows.replaceChildren(h("p", "ui-hint", t("launcher.remoteLoading")));
    try {
      const listing = await invoke("remote_list_dir", { machine, path: next || null });
      if (!dialog.root.isConnected) return;
      path = listing.path; location.textContent = path;
      rows.replaceChildren();
      for (const entry of listing.entries) {
        if (entry.name.startsWith(".")) continue;
        const folder = `${path.replace(/\/+$/, "")}/${entry.name}`;
        if (entry.git) {
          const known = projects.some(project => project.host === machine && project.path === folder);
          const choice = checkbox(known ? `${entry.name} · ${t("launcher.remoteAdded")}` : entry.name, known || picked.has(folder), "launcher.remotePick");
          choice.control.disabled = known;
          choice.control.onchange = () => { if (choice.control.checked) picked.add(folder); else picked.delete(folder); update(); };
          choice.label.classList.add("launcher-browse-repo");
          rows.append(choice.label);
        } else {
          const control = button(entry.name, () => void openFolder(folder), "ghost", "launcher.remoteFolder");
          control.insertAdjacentHTML("afterbegin", icon("folder", 14));
          control.classList.add("launcher-browse-folder"); rows.append(control);
        }
      }
      if (!rows.childElementCount) rows.append(h("p", "ui-hint", t("launcher.remoteEmpty")));
    } catch (error) { feedback.textContent = fromBack(error); rows.replaceChildren(); retry.hidden = false; }
    finally { loading = false; retry.disabled = false; up.disabled = !path || path === "/"; update(); }
  }
  dialog.open();
  void openFolder("");
}

export function openRemoteClone(machine: string, added: (project: Project) => void) {
  const url = input("", false, "projects.cloneUrl");
  url.required = true; url.spellcheck = false; url.placeholder = "git@github.com:org/app.git";
  const directory = input(`${machines.machineOf(machine)?.host?.home ?? ""}/code`, false, "projects.cloneDirectory");
  directory.required = true; directory.pattern = "/.*"; directory.spellcheck = false;
  const dialog = formDialog({ track: "projects.clone", area: "projects",
    title: t("projects.cloneOn", { name: machines.machineName(machine) }), save: t("projects.clone"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      const source = url.value.trim();
      const name = source.replace(/\/+$/, "").replace(/\.git$/, "").split(/[/:]/).pop();
      if (!name || name === "." || name === "..") throw t("projects.cloneUrl");
      url.disabled = directory.disabled = true;
      try {
        const project = await invoke("remote_add_project", { machine, path: `${directory.value.trim().replace(/\/+$/, "")}/${name}`, url: source });
        added(project);
      } finally { url.disabled = directory.disabled = false; }
    },
  });
  dialog.body.append(field(t("projects.cloneUrl"), url), field(t("projects.cloneDirectory"), directory, t("projects.cloneHint")));
  dialog.open();
}

export function openProjects(say: (text: string, bad?: boolean) => void) {
  let directory: string | null = null;
  let running = false;
  const choices: { item: CatalogProject; control: HTMLInputElement; status: HTMLElement }[] = [];
  const dialog = formDialog({ track: "projects.title", area: "projects",
    title: t("projects.title"), save: t("projects.add"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      if (!directory || !choices.some(choice => choice.control.checked)) throw t("projects.choose");
      running = true;
      for (const control of dialog.body.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input,button")) control.disabled = true;
      let failed = false;
      try {
        for (const choice of choices.filter(choice => choice.control.checked)) {
          choice.status.classList.remove("bad");
          choice.status.textContent = t("projects.cloning", { name: choice.item.id });
          try {
            const project = await install(choice.item, directory, false);
            choice.control.checked = false;
            choice.status.textContent = project.path;
          } catch (error) {
            failed = true;
            choice.status.classList.add("bad");
            choice.status.textContent = fromBack(error);
          }
        }
        await catalog.load();
        if (failed) throw t("projects.failed");
        say(t("projects.done"));
      } finally {
        running = false;
        for (const control of dialog.body.querySelectorAll<HTMLInputElement | HTMLButtonElement>("input,button")) control.disabled = false;
      }
    },
  });
  const local = button(t("projects.local"), async () => {
    try {
      const path = await open({ directory: true, title: t("say.pickRepo") });
      if (typeof path !== "string") return;
      await invoke("add_project", { path });
      dialog.close();
    } catch (error) { say(fromBack(error), true); }
  }, "outline", "projects.local");
  const destination = button(t("projects.destination"), async () => {
    try {
      const path = await open({ directory: true, title: t("projects.destination") });
      if (typeof path === "string") { directory = path; destination.textContent = path; update(); }
    } catch (error) { say(fromBack(error), true); }
  }, "outline", "projects.destination");
  destination.hidden = true;
  const list = h("div", "ui-stack");
  const create = button(t("projects.create"), () => openNewProject(say, dialog.close), "outline", "projects.create");
  const github = button(t("projects.github"), () => openGitHubImport(say, dialog.close), "outline", "projects.github");
  dialog.body.append(local, create, github, h("p", "ui-hint", t("projects.hint")), destination, list);
  dialog.save.disabled = true;
  function update() { dialog.save.disabled = running || !directory || !choices.some(choice => choice.control.checked); }
  dialog.open();
  void catalog.load().then(() => {
    if (!dialog.root.isConnected) return;
    const items = (catalog.current().projects ?? []).filter(item => !item.local_path);
    destination.hidden = !items.length;
    for (const item of items) {
      const row = h("div", "setrow");
      const text = h("div", "txt");
      const choice = checkbox(`${item.id} · ${item.organization_name ?? t("catalog.cloud")}`, false, "projects.select");
      choice.control.addEventListener("change", update);
      const status = h("div", "ui-hint", item.local_path ?? item.note);
      status.setAttribute("role", "status");
      status.style.whiteSpace = "pre-wrap";
      status.style.overflowWrap = "anywhere";
      text.append(choice.label, h("span", "", item.source), status);
      const link = button(t("projects.link"), async () => {
        try {
          const path = await open({ directory: true, title: t("say.pickRepo") });
          if (typeof path !== "string") return;
          link.disabled = true;
          status.classList.remove("bad");
          const project = await install(item, path, true);
          status.textContent = project.path;
          choice.control.checked = false;
          update();
        } catch (error) { status.classList.add("bad"); status.textContent = fromBack(error); }
        finally { link.disabled = false; }
      }, "ghost", "projects.link");
      row.append(text, link); list.append(row);
      choices.push({ item, control: choice.control, status });
    }
    if (!choices.length) list.append(h("p", "ui-hint", t("projects.empty")));
  }).catch(error => { list.textContent = fromBack(error); });
}

function openNewProject(say: (text: string, bad?: boolean) => void, created: () => void) {
  let directory = "";
  const name = input();
  name.required = true;
  name.autocomplete = "off";
  const destination = button(t("projects.destination"), async () => {
    try {
      const path = await open({ directory: true, title: t("projects.destination") });
      if (typeof path === "string") { directory = path; destination.textContent = path; }
    } catch (error) { say(fromBack(error), true); }
  }, "outline", "projects.destination");
  const dialog = formDialog({ track: "projects.create", area: "projects",
    title: t("projects.create"), save: t("projects.create"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      if (!directory) throw t("projects.chooseParent");
      name.disabled = destination.disabled = true;
      try {
        await invoke("create_project", { directory, name: name.value });
        created();
      } finally { name.disabled = destination.disabled = false; }
    },
  });
  dialog.body.append(
    h("p", "ui-hint", t("projects.createHint")),
    field(t("projects.name"), name, undefined, "projects.name"),
    field(t("projects.parent"), destination, undefined, "projects.parent"),
  );
  dialog.open();
}

/** Clone a GitHub repository, or any https/ssh Git URL, into a subfolder of a local folder. */
function openGitHubImport(say: (text: string, bad?: boolean) => void, imported: () => void) {
  let directory = "";
  const source = input("", false, "projects.githubSource");
  source.required = true;
  source.spellcheck = false;
  source.autocomplete = "off";
  source.placeholder = "org/app";
  const suggestions = document.createElement("datalist");
  suggestions.id = `projects-github-${crypto.randomUUID()}`;
  source.setAttribute("list", suggestions.id);
  const destination = button(t("projects.destination"), async () => {
    try {
      const path = await open({ directory: true, title: t("projects.destination") });
      if (typeof path === "string") { directory = path; destination.textContent = path; }
    } catch (error) { say(fromBack(error), true); }
  }, "outline", "projects.destination");
  const dialog = formDialog({ track: "projects.github", area: "projects",
    title: t("projects.github"), save: t("projects.githubImport"), cancel: t("actions.cancel"), error: fromBack,
    submit: async () => {
      if (!directory) throw t("projects.chooseParent");
      source.disabled = destination.disabled = true;
      try {
        await invoke("clone_project", { source: source.value, directory });
        imported();
      } finally { source.disabled = destination.disabled = false; }
    },
  });
  dialog.body.append(
    h("p", "ui-hint", t("projects.githubHint")),
    field(t("projects.githubSource"), source, undefined, "projects.githubSource"),
    suggestions,
    field(t("projects.parent"), destination, undefined, "projects.parent"),
  );
  dialog.open();
  // Suggestions need a GitHub connection; without one the person types the source.
  void invoke("github_repositories").then(names => {
    suggestions.replaceChildren(...names.map(name => { const option = document.createElement("option"); option.value = name; return option; }));
  }).catch(() => {});
}

function install(item: CatalogProject, directory: string, existing: boolean) {
  return invoke("catalog_install_project", { id: item.id, organization: item.organization, revision: item.revision, directory, existing });
}
