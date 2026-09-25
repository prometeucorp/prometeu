import * as mcp from "./mcp";
import * as plugins from "./plugins";
import * as skills from "./skills";
import { icon } from "./icons";
import { t, tn, type Key } from "./i18n";
import { button, input, menuButton } from "./ui";
import { h } from "./util";
import { matchesSettings, type ResourceFilter } from "./settings-navigation";

let filter: ResourceFilter = "all";
let query = "";
export function setResourceFilter(next: ResourceFilter) { filter = next; }

/** Compose existing hub rows so their validation, authentication and Cloud actions stay at the edge. */
export function resourceSettings(defaults: () => void) {
  const root = h("div", "settings-resources");
  const intro = h("div", "resources-intro");
  const hint = h("div", "resources-hint");
  hint.append(h("p", "ui-hint", t("settings.resourceHint")), button(t("settings.resourceDefaults"), defaults, "ghost"));
  const add = menuButton(t("settings.resourceAdd"), () => [
    ...plugins.settingsActions(), "sep", ...mcp.settingsActions(add), "sep", ...skills.settingsActions(),
  ]);
  add.classList.replace("ghost", "pri"); add.dataset.focus = "resource-add";
  add.insertAdjacentHTML("afterbegin", icon("plus", 14));
  intro.append(hint, add);

  const groups = [
    { kind: "plugins", title: "settings.plugins", type: "settings.resourcePlugin", glyph: "puzzle", rows: plugins.settingsRows().slice(1) },
    { kind: "mcp", title: "settings.resourceMcp", type: "settings.resourceMcp", glyph: "plug", rows: mcp.settingsRows().slice(1) },
    { kind: "skills", title: "skill.title", type: "settings.resourceSkill", glyph: "sparkles", rows: skills.settingsRows().slice(1) },
  ] as const;
  const entries = groups.flatMap(group => group.rows.filter(row => row.dataset.resourceId).map(row => {
    row.classList.add("resource-row"); row.dataset.resourceType = group.kind;
    if (!row.querySelector(".glyph")) {
      const glyph = h("span", "glyph"); glyph.innerHTML = icon(group.glyph, 18); row.prepend(glyph);
    }
    const type = h("span", "resource-type", t(group.type));
    const origin = row.querySelector<HTMLElement>(".resource-origin") ?? h("span", "resource-origin");
    const actions = row.querySelector<HTMLElement>(".act")!;
    row.insertBefore(type, actions); row.insertBefore(origin, actions);
    const search = `${row.dataset.resourceId} ${row.querySelector(".txt")?.textContent} ${row.dataset.resourceOrigin} ${t(group.type)}`;
    return { kind: group.kind, row, search };
  }));

  const toolbar = h("div", "resources-toolbar");
  const filters = h("div", "resource-filters"); filters.setAttribute("aria-label", t("settings.resourceType"));
  const choices: { kind: ResourceFilter; title: Key }[] = [{ kind: "all", title: "settings.resourceAll" }, ...groups];
  const controls = choices.map(choice => {
    const count = entries.filter(entry => choice.kind === "all" || entry.kind === choice.kind).length;
    const control = button(t(choice.title), () => { filter = choice.kind; paint(); }, "ghost");
    control.append(h("span", "resource-count", String(count)));
    control.dataset.filter = choice.kind; control.dataset.focus = `resource-filter-${choice.kind}`;
    filters.append(control); return control;
  });
  const search = input(query); search.type = "search"; search.placeholder = t("settings.resourceSearch");
  search.setAttribute("aria-label", t("settings.resourceSearch")); search.dataset.focus = "resource-search";
  search.oninput = () => { query = search.value; paint(); };
  toolbar.append(filters, search);

  const headings = h("div", "resource-headings");
  headings.append(h("span", "", t("settings.resourceName")), h("span", "resource-type", t("settings.resourceType")), h("span", "resource-origin", t("settings.resourceOrigin")));
  const list = h("div", "settings-panel resource-list"); list.append(...entries.map(entry => entry.row));
  const empty = h("p", "settings-empty", t("settings.resourceEmpty")); list.append(empty);
  const count = h("p", "resources-count ui-hint"); count.setAttribute("role", "status");
  root.append(intro, toolbar, headings, list, count);
  function paint() {
    let visible = 0;
    for (const entry of entries) {
      entry.row.hidden = (filter !== "all" && filter !== entry.kind) || !matchesSettings(query, entry.search);
      if (!entry.row.hidden) visible++;
    }
    controls.forEach(control => control.setAttribute("aria-pressed", String(control.dataset.filter === filter)));
    empty.hidden = visible > 0; headings.hidden = visible === 0;
    count.textContent = tn(visible, "settings.resourceCount");
  }
  paint();
  return root;
}
