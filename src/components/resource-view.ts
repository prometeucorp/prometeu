import { button, input, menuButton } from "./primitives";
import { icon } from "../../packages/design-system/src/icons";
import { sectionHeader, toolbar, itemRow, overflowAction, listState } from "./compositions";
import { h } from "../util";
import * as menu from "./menu";
import { matchesResource, type ResourceFilter, type ResourceSelection, type ResourceSnapshot } from "../resources/model";
import "./resource-view.css";

export type ResourceLabels = {
  hint: string; defaults: string; add: string; search: string;
  name: string; type: string; origin: string; empty: string; noResults: string;
  loading: string; retry: string;
  filters: Record<ResourceFilter, string>;
  kinds: Record<Exclude<ResourceFilter, "all">, string>;
  count: (count: number) => string;
  more: (name: string) => string;
};

/** Presentation only. The host owns catalog operations, translated text and snapshots. */
export function resourceView(options: {
  labels: ResourceLabels;
  snapshot: ResourceSnapshot;
  selection?: ResourceSelection;
  changed?: (selection: ResourceSelection) => void;
  defaults: () => void;
  add: (anchor: HTMLElement) => menu.Item[];
  retry?: () => void;
}) {
  const { labels } = options;
  let snapshot = options.snapshot;
  let selection = { filter: "all" as ResourceFilter, query: "", ...options.selection };
  const root = h("div", "resources-view");
  const add = menuButton(labels.add, () => options.add(add));
  add.classList.replace("ghost", "pri"); add.dataset.focus = "resource-add";
  add.insertAdjacentHTML("afterbegin", icon("plus", 14));
  const intro = sectionHeader({ description: labels.hint,
    supporting: [button(labels.defaults, options.defaults, "ghost")], actions: [add] });
  intro.classList.add("resources-intro");

  const filters = h("div", "resource-filters");
  filters.setAttribute("role", "group"); filters.setAttribute("aria-label", labels.type);
  const controls = (Object.keys(labels.filters) as ResourceFilter[]).map(kind => {
    const control = button(labels.filters[kind], () => {
      selection.filter = kind; options.changed?.({ ...selection }); paint();
    }, "ghost");
    const count = h("span", "resource-count");
    control.append(count); control.dataset.filter = kind; control.dataset.focus = `resource-filter-${kind}`;
    filters.append(control); return { kind, control, count };
  });
  const search = input(selection.query); search.type = "search";
  search.placeholder = labels.search; search.setAttribute("aria-label", labels.search);
  search.dataset.focus = "resource-search";
  search.oninput = () => { selection.query = search.value; options.changed?.({ ...selection }); paint(); };
  const tools = toolbar([filters], [search]); tools.classList.add("resources-toolbar");

  const headings = h("div", "resource-headings"); headings.setAttribute("aria-hidden", "true");
  headings.append(h("span", "", labels.name), h("span", "resource-type", labels.type), h("span", "resource-origin", labels.origin));
  const list = h("div", "resource-list");
  const message = h("div", "resources-message");
  const count = h("p", "resources-count ui-hint"); count.setAttribute("role", "status");
  root.append(intro, tools, message, headings, list, count);

  function closeMenu() {
    if (root.querySelector('[aria-expanded="true"]')) menu.close();
  }
  function paint() {
    closeMenu();
    const focused = root.contains(document.activeElement) ? (document.activeElement as HTMLElement).dataset.focus : undefined;
    const scroll = root.scrollTop;
    list.replaceChildren(); message.replaceChildren();
    root.setAttribute("aria-busy", String(!!snapshot.loading));
    controls.forEach(({ kind, control, count }) => {
      control.setAttribute("aria-pressed", String(selection.filter === kind));
      count.textContent = String(snapshot.items.filter(item => kind === "all" || item.kind === kind).length);
    });
    const items = snapshot.items.filter(item => matchesResource(item, selection, labels.kinds[item.kind]));
    for (const item of items) {
      const origin = h("span", "resource-origin", item.origin); origin.title = item.origin;
      const row = itemRow({ title: item.id, description: item.description, glyph: item.glyph,
        status: item.status, busy: item.busy,
        metadata: [h("span", "resource-type", labels.kinds[item.kind]), origin],
        actions: item.actions.length ? [overflowAction({ label: labels.more(item.id), key: item.key,
          items: item.actions, busy: item.busy || snapshot.loading })] : [],
      });
      row.root.classList.add("resource-row");
      if (item.kind === "mcp") row.root.classList.add("mcp-server");
      row.root.dataset.resourceId = item.id; row.root.dataset.resourceType = item.kind;
      if (item.scope) row.root.dataset.resourceScope = item.scope;
      row.mark.classList.add("resource-glyph"); row.copy.classList.add("resource-copy");
      row.description.classList.add("resource-description"); row.actions.classList.add("resource-actions");
      list.append(row.root);
    }
    message.append(listState(snapshot.error
      ? { kind: "error", text: snapshot.error, retry: options.retry
        ? { label: labels.retry, run: options.retry, key: "resource-retry" } : undefined }
      : snapshot.loading ? { kind: "loading", text: labels.loading }
      : !items.length ? { kind: "empty", text: snapshot.items.length ? labels.noResults : labels.empty }
      : { kind: "ready" }));
    message.hidden = !snapshot.error && !snapshot.loading && !!items.length;
    list.hidden = headings.hidden = !items.length;
    count.textContent = snapshot.loading ? labels.loading : labels.count(items.length);
    if (focused) {
      const target = root.querySelector<HTMLElement>(`[data-focus="${CSS.escape(focused)}"]`);
      (target && !(target instanceof HTMLButtonElement && target.disabled) ? target : search).focus({ preventScroll: true });
    }
    root.scrollTop = scroll;
  }
  paint();
  return {
    root,
    update(next: ResourceSnapshot) { snapshot = next; paint(); },
    destroy() { closeMenu(); root.remove(); },
  };
}
