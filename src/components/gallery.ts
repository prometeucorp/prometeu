import catalog from "./catalog.json";
import catalogUrl from "./catalog.json?url";
import { stories } from "./stories";
import { input, select, field, notice } from "./primitives";
import { h } from "../util";
import { t } from "../i18n";
import { initCodeCopy } from "../markdown";

const link = (label: string, href: string) => {
  const node = h("a", "ui-link", label); node.setAttribute("href", href); return node;
};
const url = (id: string, state?: string, embed = false) => {
  const query = new URLSearchParams({ component: id });
  if (state) query.set("state", state);
  if (embed) query.set("embed", "1");
  return `/design-system.html?${query}`;
};

/** URL-addressable stories use production components. No app bootstrap or backend mock. */
export function mountCatalog(host: HTMLElement): boolean {
  const query = new URLSearchParams(location.search);
  const id = query.get("component");
  const entry = catalog.find(item => item.id === id);
  const section = h("section", "ui-gallery-section component-catalog"); section.id = "component-catalog";
  section.append(h("h2", "", t("ui.catalog")));
  const search = input(); search.type = "search"; search.setAttribute("aria-label", t("ui.catalogSearch")); search.placeholder = t("ui.catalogSearch");
  const list = h("nav", "catalog-list"); list.setAttribute("aria-label", t("ui.catalog"));
  const drawList = () => {
    list.replaceChildren(...catalog.filter(item => `${item.name} ${item.source}`.toLowerCase().includes(search.value.toLowerCase())).map(item => {
      const row = link(item.name, url(item.id)); row.dataset.component = item.id;
      if (item.id === id) row.setAttribute("aria-current", "page");
      row.append(h("small", "ui-hint", item.source)); return row;
    }));
  };
  search.oninput = drawList; drawList();
  section.append(search, link(t("ui.catalogManifest"), catalogUrl), list);
  if (!id) { host.append(section); return false; }
  host.classList.add("catalog-page");
  if (query.get("embed") !== "1") host.append(section);
  if (!entry || !stories[id]) { host.append(notice(t("ui.catalogMissing"), "error")); return true; }
  const requested = query.get("state");
  const state = requested && entry.states.includes(requested) ? requested : entry.states[0];
  const content = h("section", "catalog-story");
  content.dataset.component = id; content.dataset.state = state;
  if (query.get("embed") !== "1") {
    const header = h("div", "ui-gallery-section");
    const choice = select(state, entry.states.map(value => [value, value]));
    choice.onchange = () => { location.href = url(id, choice.value); };
    header.append(h("h1", "", entry.name), field(t("ui.catalogState"), choice.control),
      h("p", "ui-hint", `${t("ui.catalogSource")}: ${entry.source}`),
      h("p", "ui-hint", `${t("ui.catalogConsumers")}: ${entry.consumers.join(", ")}`),
      link(t("ui.catalogIsolated"), url(id, state, true)), link(t("ui.catalogBack"), "/design-system.html"));
    content.append(header);
  }
  const canvas = h("div", "story-canvas"); canvas.id = "story-canvas";
  const feedback = h("p", "story-feedback ui-hint"); feedback.setAttribute("role", "status");
  const report = (value: string) => { feedback.textContent = value; };
  const story = stories[id](state, report);
  canvas.append(story.root); content.append(canvas, feedback); host.append(content);
  initCodeCopy(() => message => report(message));
  window.addEventListener("pagehide", () => story.destroy?.(), { once: true });
  return true;
}
