import { button, field, formDialog, input, select } from "../ui";
import { sectionHeader } from "../components/compositions";
import { stage as stageName, t, type Key } from "../i18n";
import { h } from "../util";
import { Workspaces, type WorkspacePort } from "./workspaces";
import type { Session } from "./session";

export function workspaceError(error: unknown) {
  const code = String(error);
  const keys: Record<string, Key> = {
    workspace_folder_invalid: "wsl.workspaceFolderInvalid", workspace_title_invalid: "wsl.workspaceTitleInvalid",
    workspace_not_found: "wsl.workspaceNotFound", workspace_stage_invalid: "wsl.workspaceStageInvalid",
    workspace_catalog_invalid: "wsl.workspaceCatalogInvalid", workspace_limit: "wsl.workspaceLimit",
    workspace_repository_invalid: "wsl.worktreeRepositoryInvalid", workspace_branch_invalid: "wsl.worktreeBranchInvalid",
    workspace_base_invalid: "wsl.worktreeBaseInvalid", workspace_worktree_unsupported: "wsl.worktreeUnsupported",
    workspace_unsupported: "wsl.workspaceUnsupported",
  };
  for (const [prefix, key] of [["workspace_worktree_failed: ", "wsl.worktreeFailed"], ["workspace_worktree_unsaved: ", "wsl.worktreeUnsaved"]] as const) {
    if (code.startsWith(prefix)) return `${t(key)}\n${code.slice(prefix.length)}`;
  }
  return keys[code] ? t(keys[code]) : code;
}

export function workspaceView(port: WorkspacePort, session: Session, draft: HTMLTextAreaElement, changed: () => void) {
  const root = h("section", "wsl-workspaces"); root.setAttribute("aria-label", t("wsl.workspaces"));
  const list = h("nav", "wsl-workspace-list"); list.setAttribute("aria-label", t("wsl.workspaces"));
  const error = h("div", "wsl-error"); error.setAttribute("role", "alert");
  const stageHost = h("div", "wsl-workspace-stage");
  let connected = false;
  let revision = "";
  const controller = new Workspaces(port, session, () => { paint(); changed(); });
  const add = button(t("wsl.workspaceAdd"), () => {
    const title = input(""); title.required = true; title.maxLength = 200;
    const path = input(""); path.required = true;
    const dialog = formDialog({ title: t("wsl.workspaceAdd"), save: t("wsl.workspaceCreate"), cancel: t("actions.cancel"), error: workspaceError,
      submit: async () => { if (!await controller.create(title.value, path.value)) throw controller.error; } });
    dialog.body.append(h("p", "ui-hint", t("wsl.workspaceExisting")), field(t("wsl.workspaceName"), title), field(t("wsl.workdir"), path));
    dialog.open();
  });
  const worktree = button(t("wsl.worktreeAdd"), () => {
    const current = controller.catalog?.board.workspaces.find(w => w.id === controller.catalog?.active);
    const title = input(""); title.required = true; title.maxLength = 200;
    const path = input(current?.repo ?? ""); path.required = true;
    const branch = input(""); branch.required = true; branch.maxLength = 240;
    const base = input("HEAD"); base.required = true; base.maxLength = 1024;
    const dialog = formDialog({ title: t("wsl.worktreeAdd"), save: t("wsl.workspaceCreate"), cancel: t("actions.cancel"), error: workspaceError,
      submit: async () => {
        if (!await controller.createWorktree({ title: title.value, path: path.value, branch: branch.value, base: base.value })) throw controller.error;
      } });
    dialog.body.append(h("p", "ui-hint", t("wsl.worktreeHint")), field(t("wsl.workspaceName"), title),
      field(t("wsl.worktreeRepository"), path), field(t("wsl.worktreeBranch"), branch), field(t("wsl.worktreeBase"), base));
    dialog.open();
  }, "ghost");
  root.append(sectionHeader({ title: t("wsl.workspaces"), actions: [add, worktree] }), list, stageHost, error);
  function paint() {
    root.hidden = !connected;
    add.disabled = !connected || !controller.catalog || controller.pending || session.pending;
    worktree.disabled = add.disabled;
    error.textContent = workspaceError(controller.error);
    const catalog = controller.catalog;
    const signature = JSON.stringify([catalog, controller.pending, session.pending]);
    if (revision === signature) return;
    revision = signature;
    list.replaceChildren(); stageHost.replaceChildren();
    if (!catalog) return;
    for (const workspace of catalog.board.workspaces) {
      const control = button(workspace.title, async () => {
        const next = await controller.select(workspace.id, draft.value);
        if (next !== null) {
          draft.value = next; draft.dispatchEvent(new Event("input"));
          list.querySelector<HTMLButtonElement>(`[data-workspace="${CSS.escape(workspace.id)}"]`)?.focus();
        }
      }, "ghost");
      control.title = workspace.worktree;
      control.dataset.workspace = workspace.id;
      control.setAttribute("aria-pressed", String(workspace.id === catalog.active));
      control.disabled = controller.pending || session.pending;
      list.append(control);
    }
    const current = catalog.board.workspaces.find(w => w.id === catalog.active)!;
    const stage = select(current.stage, catalog.board.stages.map(s => [s, stageName(s)]), { disabled: controller.pending || session.pending });
    stage.onchange = () => { void controller.stage(current.id, stage.value); };
    stageHost.append(h("span", "ui-hint", [current.worktree, current.branch].filter(Boolean).join(" · ")), field(t("wsl.workspaceStage"), stage.control));
  }
  return { root, get pending() { return controller.pending; }, update(value: boolean) {
    const attached = value && !connected;
    const detached = !value && connected;
    connected = value; paint();
    if (detached) controller.detach(draft.value);
    if (attached) {
      if (controller.attach(session.targetKey)) draft.value = "";
      void controller.load().then(loaded => {
        if (loaded && connected) { draft.value = controller.draft(); draft.dispatchEvent(new Event("input")); }
      });
    }
  } };
}
