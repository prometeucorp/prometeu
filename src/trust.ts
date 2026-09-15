import { invoke } from "./ipc";
import { fromBack, t } from "./i18n";
import * as ui from "./ui";
import type { ProjectTools, Selection } from "./types";
import { h } from "./util";

/// Project trust (ADR 0043): a repository's versioned `[tools]` activates only after the person
/// approves its current hash. The decision is stored app-local on the board, never in the repository,
/// so a changed declaration re-prompts. This dialog shows what the project declares and records the
/// choice; the backend recomputes the current hash itself, so the decision always covers the
/// declaration as it stands now. An undecided declaration resolves but is not injected, and its
/// items show as pending; a refused one keeps its items visible as rejected and quiets the prompt
/// until the declaration changes.

/// One line per declared axis, naming what the project layer adds or removes.
function axisLine(name: string, sel: Selection | null): HTMLElement | null {
  if (!sel) return null;
  const parts = [
    sel.add.length ? `${t("tools.trust.adds")} ${sel.add.join(", ")}` : "",
    sel.remove.length ? `${t("tools.trust.removes")} ${sel.remove.join(", ")}` : "",
  ].filter(Boolean);
  if (!parts.length) return null;
  return h("p", "ui-hint", `${name}: ${parts.join(" · ")}`);
}

/// Open the trust prompt for a workspace's primary repository. `say` reports backend errors inline.
export async function open(workspace: string, say: (text: string, isError?: boolean) => void) {
  let decl: ProjectTools;
  try {
    decl = await invoke("project_tools", { id: workspace });
  } catch (e) {
    return say(fromBack(e), true);
  }
  // An empty hash means the repository declares no `[tools]`, so there is nothing to trust.
  if (!decl.hash) return say(t("tools.trust.empty"));

  const dialog = ui.formDialog({
    title: t("tools.trust.title"),
    save: t("tools.trust.approve"),
    cancel: t("tools.trust.later"),
    error: fromBack,
    submit: async () => {
      await invoke("project_tools_trust", { id: workspace, approved: true });
    },
  });
  dialog.body.append(
    h("p", "ui-hint", t("tools.trust.body")),
    ...[
      decl.file ? h("p", "ui-hint", decl.file) : null,
      axisLine(t("settings.mcp"), decl.tools.mcp),
      axisLine(t("settings.plugins"), decl.tools.plugins),
      axisLine(t("skill.title"), decl.tools.skills),
    ].filter((el): el is HTMLElement => el !== null),
    h("p", "ui-hint", `${t("tools.trust.hash")}: ${decl.hash.slice(0, 12)}`),
  );
  // Rejecting records the decision so the prompt stops until the declaration's hash changes.
  const reject = ui.button(t("tools.trust.reject"), () => {
    void invoke("project_tools_trust", { id: workspace, approved: false })
      .then(() => dialog.close())
      .catch((e) => say(fromBack(e), true));
  }, "ghost");
  dialog.body.append(reject);
  dialog.open();
}
