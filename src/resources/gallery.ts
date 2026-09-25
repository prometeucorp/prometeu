import { t, type Key } from "../i18n";
import { button, field, select } from "../ui";
import { h } from "../util";
import { resourceView } from "../components/resource-view";
import { resourceLabels } from "./labels";
import type { ResourceItem, ResourceSnapshot } from "./model";

/** Executable example: imports the production presentation, never the hubs or browser mock. */
export function resourceExample(initialState = "ready") {
  const root = h("section", "ui-gallery-section resource-example"); root.id = "resources-preview";
  root.append(h("h2", "", t("ui.resourcePreview")));
  const states: [string, Key][] = [["ready", "ui.resourceReady"], ["empty", "ui.resourceEmpty"],
    ["loading", "ui.resourceLoading"], ["error", "ui.resourceError"], ["busy", "ui.resourceBusy"],
    ["unavailable", "ui.resourceUnavailable"], ["long", "ui.resourceLong"]];
  const choice = select(initialState, states.map(([value, label]) => [value, t(label)]));
  const feedback = h("p", "ui-hint"); feedback.setAttribute("role", "status");
  const report = (name: string) => { feedback.textContent = t("ui.resourceAction", { name }); };
  let revision = 0;
  const item = (id: string, kind: ResourceItem["kind"], description: string): ResourceItem => ({
    key: `${kind}-${id}`, id, kind, description, origin: t("catalog.local"),
    glyph: kind === "mcp" ? "plug" : kind === "plugins" ? "puzzle" : "sparkles",
    actions: [{ label: t("actions.edit"), run: () => { report(id); revision++; view.update(snapshot()); } },
      { label: t("actions.remove"), danger: true, run: () => report(id) }],
  });
  function snapshot(): ResourceSnapshot {
    const items = [item("workspace-tools", "mcp", `Tools for local workspace files · ${revision}`),
      item("review-kit", "plugins", "Reusable review commands and checks"),
      item("release-notes", "skills", "Draft release notes from the current changes")];
    if (choice.value === "empty") return { items: [] };
    if (choice.value === "loading") return { items: [], loading: true };
    if (choice.value === "error") return { items: [], error: t("ui.resourceFailure") };
    if (choice.value === "busy") { items[0].busy = true; items[0].status = t("mcp.check.doing"); }
    if (choice.value === "unavailable") items[0].actions = [{ label: t("actions.edit"), disabled: true }];
    if (choice.value === "long") {
      items[0].id = "workspace-tools-with-a-long-name-that-must-remain-readable";
      items[0].description = "Local workspace tools with a detailed description that wraps across multiple lines without moving the actions outside the panel.";
      items[0].origin = "Example organization with a long name";
    }
    return { items };
  }
  const view = resourceView({
    labels: resourceLabels(), snapshot: snapshot(),
    defaults: () => report(t("settings.resourceDefaults")),
    add: () => [{ label: t("mcp.add"), run: () => report(t("mcp.add")) }],
    retry: () => { choice.value = "ready"; view.update(snapshot()); },
  });
  choice.onchange = () => { feedback.textContent = ""; view.update(snapshot()); };
  const refresh = button(t("ui.resourceRefresh"), () => {
    revision++; view.update(snapshot()); feedback.textContent = t("ui.resourceUpdated");
  });
  const controls = h("div", "ui-gallery-row"); controls.append(field(t("ui.resourceState"), choice.control), refresh);
  root.append(controls, view.root, feedback);
  return root;
}
