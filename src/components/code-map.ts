import cytoscape, { type Core, type ElementDefinition, type LayoutOptions } from "cytoscape";
import type { CodeMapSnapshot } from "../code-map-types";
import { h } from "../util";
import { button, input, select } from "./primitives";
import { iconButton } from "./icon-button";
import { icon } from "./icons";
import { listState } from "./compositions";
import * as menu from "./menu";
import { GRAPH_LIMIT, directory, fileGraph, type FileGraph, type MapSymbol } from "./code-map-model";
import "./code-map.css";

export type CodeMapLabels = {
  search: string; clearSearch: string; root: string; overview: string; results: string;
  empty: string; noResults: string; selectFile: string; selectSymbol: string; loading: string;
  sync: string; syncing: string; retry: string; expand: string; collapse: string;
  zoomIn: string; zoomOut: string; fit: string; openSource: string;
  truncated: string; uncertain: string; excerptTruncated: string; syntaxErrors: string; unresolved: string;
  directories: string; back: string; previous: string; next: string;
  directoryCount: (groups: number, files: number) => string;
  fileCount: (count: number) => string;
  range: (start: number, end: number, total: number) => string;
  graphCount: (files: number, edges: number, total: number) => string;
  resultCount: (count: number) => string;
  lastSync: (timestamp: number | null) => string;
  generation: (value: number) => string;
  relationship: (kind: string, resolution: string, confidence: string, ambiguous: boolean) => string;
};
export type CodeMapViewState = {
  data: CodeMapSnapshot | null; query: string;
  loading?: boolean; syncing?: boolean; error?: string; expanded?: boolean;
};
const symbolKey = (item: MapSymbol) => JSON.stringify([item.path, item.symbol.qualified_name, item.symbol.start_line]);
const emptyGraph = (): FileGraph => ({ files: [], entries: [], edges: [], symbols: [], directories: true, truncated: false });
const zoomRange = { min: .05, max: 3 };
/** File overview and symbol context. The host owns requests, timing, root access and source opening. */
export function codeMapView(options: {
  labels: CodeMapLabels; snapshot: CodeMapViewState;
  queryChanged: (query: string) => void; rootChanged: (root: string) => void;
  inspect: (path: string) => void; sync: () => void; retry: () => void;
  openSource: (path: string, line: number) => void; expandedChanged: (expanded: boolean) => void;
}): { root: HTMLElement; update(next: CodeMapViewState): void; focusSearch(): void; destroy(): void } {
  const { labels } = options;
  let state = options.snapshot, scope = "", selected: string | null = null, selectedFile: string | null = null;
  let graph = emptyGraph(), folder: string | null = null, page = 0;
  let renderer: Core | undefined, needsGraph = true, needsLayout = true, destroyed = false, width = 0;
  let paintedOverview: CodeMapSnapshot["overview"] | undefined, paintedResult: CodeMapSnapshot["result"] | undefined;
  let paintedFolder: string | null = null, rootOptions = "", frame = 0;
  const root = h("section", "code-map"); root.dataset.trackArea = "workspace.side";
  const head = h("div", "code-map-head"), scopeRow = h("div", "code-map-scope");
  const scopeName = h("span", "code-map-scope-name");
  const roots = select("", [], { track: "code_map.root" });
  roots.control.setAttribute("aria-label", labels.root);
  roots.onchange = () => options.rootChanged(roots.value);
  const back = button(labels.back, () => {
    folder = null; selected = selectedFile = null; page = 0;
    if (state.data?.result) { search.value = ""; changeQuery(); } else paint();
  }, "ghost", "code_map.back");
  scopeRow.append(back, scopeName, roots.root);
  const searchRow = h("div", "code-map-search");
  const search = input(state.query, false, "code_map.search"); search.type = "search";
  search.placeholder = labels.search; search.setAttribute("aria-label", labels.search);
  const clear = iconButton({ label: labels.clearSearch, glyph: "x", track: "code_map.clear", run: () => {
    search.value = ""; changeQuery(); search.focus();
  } });
  const expand = iconButton({ label: labels.expand, glyph: "external-link", track: "code_map.expand", run: () => options.expandedChanged(!state.expanded) });
  searchRow.append(search, clear, expand); head.append(scopeRow, searchRow);
  const feedback = h("div", "code-map-feedback");
  const body = h("div", "code-map-body"), canvas = h("div", "code-map-canvas");
  const heading = h("div", "code-map-heading"), graphTitle = h("strong", "", labels.overview), count = h("span", "");
  heading.append(graphTitle, count);
  const stage = h("div", "code-map-stage");
  const drawing = h("div", "code-map-graph");
  drawing.setAttribute("role", "img"); drawing.setAttribute("aria-label", labels.overview);
  drawing.dataset.track = "code_map.node"; stage.append(drawing);
  stage.addEventListener("wheel", event => {
    // Let native scrolling reach the inspector when the panel is shorter than its content.
    if (!event.ctrlKey && !event.metaKey && root.scrollHeight > root.clientHeight) event.stopPropagation();
  }, { capture: true, passive: true });
  const graphMessage = h("div", "code-map-graph-message"); graphMessage.setAttribute("role", "status"); stage.append(graphMessage);
  const graphTools = h("div", "code-map-graph-tools"), uncertainty = h("span", "code-map-uncertainty", labels.uncertain);
  const zoomControls = h("div", "code-map-zoom"), zoomValue = h("span", "");
  const zoomOut = button("−", () => changeZoom(1 / 1.25), "ghost", "code_map.zoom_out");
  zoomOut.setAttribute("aria-label", labels.zoomOut); zoomOut.title = labels.zoomOut;
  const zoomIn = iconButton({ label: labels.zoomIn, glyph: "plus", track: "code_map.zoom_in", run: () => changeZoom(1.25) });
  const fitButton = iconButton({ label: labels.fit, glyph: "square", track: "code_map.fit", run: () => fit() });
  zoomControls.append(zoomOut, zoomValue, zoomIn, fitButton); graphTools.append(uncertainty, zoomControls);
  canvas.append(heading, stage, graphTools);
  const inspector = h("section", "code-map-inspector"), resultHeading = h("div", "code-map-results-heading");
  const resultsTitle = h("span", ""), resultCount = h("span", ""); resultCount.setAttribute("aria-live", "polite");
  resultHeading.append(resultsTitle, resultCount);
  const inspectorBody = h("div", "code-map-inspector-body"), results = h("div", "code-map-results"), detail = h("div", "code-map-detail");
  results.setAttribute("aria-label", labels.results); detail.tabIndex = -1;
  const pages = h("div", "code-map-pages"), range = h("span", "");
  const previous = button(labels.previous, () => { page--; paintResults(); }, "ghost", "code_map.previous");
  const next = button(labels.next, () => { page++; paintResults(); }, "ghost", "code_map.next");
  pages.append(previous, range, next);
  inspectorBody.append(results, detail); inspector.append(resultHeading, inspectorBody, pages); body.append(canvas, inspector);
  const truncated = h("p", "code-map-truncated", labels.truncated);
  const footer = h("div", "code-map-footer"), syncCopy = h("div", "code-map-sync-copy"), syncTime = h("span", ""), generation = h("small", "");
  syncCopy.append(syncTime, generation);
  const sync = button(labels.sync, options.sync, "outline", "code_map.sync");
  footer.append(syncCopy, sync); root.append(head, feedback, body, truncated, footer);

  function changeQuery() {
    selected = selectedFile = null; page = 0; clear.hidden = !search.value;
    paintDetail(); paintHighlights();
    options.queryChanged(search.value);
  }
  search.addEventListener("input", changeQuery);
  search.addEventListener("keydown", event => {
    if (event.key === "Escape" && search.value) { event.preventDefault(); search.value = ""; changeQuery(); }
  });
  function inspect(path: string) {
    if (graph.entries.find(file => file.path === path)?.kind === "directory") {
      folder = path; page = 0; selected = selectedFile = null; paint(); return;
    }
    selectedFile = path; selected = null; ensureVisible(path); paintHighlights(); options.inspect(path);
  }
  function ensureVisible(path: string) {
    if (!state.data || (graph.files.some(file => file.path === path) && !graph.truncated)) return;
    graph = fileGraph(state.data, folder, path); queueGraph(true, true);
  }
  function selectSymbol(item: MapSymbol) {
    selected = symbolKey(item); selectedFile = item.path;
    ensureVisible(item.path);
    for (const row of results.querySelectorAll<HTMLButtonElement>("[data-symbol]")) row.setAttribute("aria-pressed", String(row.dataset.symbol === selected));
    paintDetail(); paintHighlights(); reveal(item.path);
  }
  const nodeId = (path: string) => `file:${path}`;
  const edgeId = (from: string, to: string) => `edge:${JSON.stringify([from, to])}`;
  const shorten = (value: string, length: number) => value.length > length ? `${value.slice(0, length - 1)}…` : value;
  function updateZoom() {
    const zoom = renderer?.zoom() ?? 1;
    zoomValue.textContent = `${Math.round(zoom * 100)}%`;
    zoomIn.disabled = !graph.files.length || zoom >= zoomRange.max;
    zoomOut.disabled = !graph.files.length || zoom <= zoomRange.min;
    fitButton.disabled = !graph.files.length;
  }
  function changeZoom(factor: number) {
    if (!renderer) return;
    renderer.zoom({ level: Math.max(zoomRange.min, Math.min(zoomRange.max, renderer.zoom() * factor)),
      renderedPosition: { x: renderer.width() / 2, y: renderer.height() / 2 } });
  }
  function fit(readable = false) {
    if (!renderer || !graph.files.length) return;
    renderer.fit(undefined, 20);
    // Keep dense projections readable; the list and native pan reveal offscreen context.
    const zoom = renderer.zoom();
    if (zoom > 1.25) { renderer.zoom(1.25); renderer.center(); }
    else if (readable && zoom < .9) {
      renderer.zoom(.9); renderer.center();
    }
  }
  function reveal(path: string) {
    if (!renderer || needsGraph) return;
    const node = renderer.getElementById(nodeId(path)); if (!node.length) return;
    const bounds = node.renderedBoundingBox();
    const offset = (start: number, end: number, size: number) => end - start > size ? (size - start - end) / 2
      : start < 0 ? -start : end > size ? size - end : 0;
    renderer.panBy({ x: offset(bounds.x1, bounds.x2, renderer.width()), y: offset(bounds.y1, bounds.y2, renderer.height()) });
  }
  function queueGraph(layout = false, changed = false) {
    needsLayout ||= layout; needsGraph ||= changed;
    if (!frame) frame = requestAnimationFrame(() => { frame = 0; renderGraph(); });
  }
  function renderGraph() {
    if (destroyed || !drawing.isConnected || !drawing.clientWidth || !drawing.clientHeight) return;
    if (!renderer) {
      const style = getComputedStyle(root), color = (name: string) => style.getPropertyValue(name).trim();
      renderer = cytoscape({ container: drawing, elements: [], layout: { name: "preset" },
        minZoom: zoomRange.min, maxZoom: zoomRange.max, boxSelectionEnabled: false, selectionType: "single", pixelRatio: Math.min(devicePixelRatio, 2),
        style: [
          { selector: "node", style: { shape: "round-rectangle", width: 152, height: 34, padding: "2px",
            "background-color": color("--bg-side"), "border-width": 1, "border-color": color("--fg-3"),
            label: "data(label)", color: color("--fg-2"), "font-family": color("--mono"), "font-size": 11,
            "text-valign": "center", "text-halign": "center", "text-wrap": "wrap", "line-height": 1.4, "overlay-opacity": 0 } },
          { selector: "node.match", style: { "border-color": color("--accent") } },
          { selector: "node.context", style: { "border-color": color("--run") } },
          { selector: "node.selected", style: { "border-width": 2, "border-color": color("--fg"),
            "background-color": color("--bg-raised-2"), color: color("--fg") } },
          { selector: "edge", style: { width: 1, "line-color": color("--fg-3"), "target-arrow-color": color("--fg-3"),
            "target-arrow-shape": "triangle", "arrow-scale": .8, "curve-style": "bezier", opacity: .45 } },
          { selector: "edge.uncertain", style: { "line-style": "dashed" } },
          { selector: "edge.lit", style: { width: 1.5, "line-color": color("--accent"), "target-arrow-color": color("--accent"), opacity: .8 } },
          { selector: ".dim", style: { opacity: .18 } },
        ] });
      renderer.on("zoom", updateZoom);
      renderer.on("tap", "node", event => inspect(event.target.data("path")));
      renderer.on("mouseover", "node, edge", event => { drawing.title = event.target.data("title"); drawing.style.cursor = "pointer"; });
      renderer.on("mouseout", "node, edge", () => { drawing.title = ""; drawing.style.cursor = "grab"; });
    }
    renderer.resize();
    if (needsGraph) {
      const nodes: ElementDefinition[] = graph.files.map(file => ({ data: { id: nodeId(file.path), path: file.path,
        label: file.kind === "directory" ? `${shorten(file.path, 22)}\n${labels.fileCount(file.files)}`
          : `${shorten(file.path.split("/").pop()!, 20)}\n${shorten(directory(file.path), 22)}`,
        title: file.kind === "directory" ? `${file.path} · ${labels.fileCount(file.files)}` : file.path } }));
      const edges: ElementDefinition[] = graph.edges.map(edge => ({ data: { id: edgeId(edge.from, edge.to),
        source: nodeId(edge.from), target: nodeId(edge.to),
        title: `${edge.from} → ${edge.to} · ${edge.count}${edge.uncertain ? ` · ${labels.uncertain}` : ""}` }, classes: edge.uncertain ? "uncertain" : "" }));
      renderer.json({ elements: { nodes, edges } });
      needsGraph = false; paintHighlights();
    }
    if (needsLayout && graph.files.length) {
      const common = { animate: false, fit: false, nodeDimensionsIncludeLabels: true };
      // Reciprocal arrowheads need more space than disconnected cards.
      const layout: LayoutOptions = graph.directories || !graph.edges.length || graph.files.length > 20
        ? { ...common, name: "grid", cols: Math.max(1, Math.floor(drawing.clientWidth / 180)), condense: true,
          avoidOverlap: true, avoidOverlapPadding: graph.edges.length ? 20 : 4, spacingFactor: 1.04, sort: (a, b) => a.id().localeCompare(b.id()) }
        : { ...common, name: "breadthfirst", directed: false, direction: "rightward", circle: false, spacingFactor: .65, avoidOverlap: true };
      renderer.layout(layout).run(); fit(true);
      if (selectedFile) reveal(selectedFile);
    }
    needsLayout = false; updateZoom();
  }
  function paintHighlights() {
    if (!renderer || needsGraph) return;
    const active = !!state.data?.result || !!selectedFile;
    renderer.batch(() => {
      for (const file of graph.files) {
        const node = renderer!.getElementById(nodeId(file.path));
        node.toggleClass("match", file.match); node.toggleClass("context", file.context && !file.match);
        node.toggleClass("selected", file.path === selectedFile);
        node.toggleClass("dim", active && !file.match && !file.context && file.path !== selectedFile);
      }
      for (const edge of graph.edges) {
        const line = renderer!.getElementById(edgeId(edge.from, edge.to));
        const highlighted = edge.highlighted || (!!selectedFile && (edge.from === selectedFile || edge.to === selectedFile));
        line.toggleClass("lit", highlighted); line.toggleClass("dim", active && !highlighted);
      }
    });
  }
  function paintResults() {
    const focused = document.activeElement instanceof HTMLElement && results.contains(document.activeElement) ? document.activeElement.dataset.focus : undefined;
    const scroll = results.scrollTop; results.replaceChildren();
    resultsTitle.textContent = state.data?.result ? labels.results : graph.directories ? labels.directories : folder ?? labels.overview;
    resultCount.textContent = state.data?.result ? labels.resultCount(graph.symbols.length) : "";
    const total = state.data?.result ? graph.symbols.length : graph.entries.length;
    page = Math.max(0, Math.min(page, Math.ceil(total / GRAPH_LIMIT.rows) - 1));
    const start = page * GRAPH_LIMIT.rows, end = Math.min(total, start + GRAPH_LIMIT.rows);
    range.textContent = labels.range(total ? start + 1 : 0, end, total); pages.hidden = total <= GRAPH_LIMIT.rows;
    previous.disabled = page === 0; next.disabled = end >= total;
    const rows: HTMLButtonElement[] = [];
    if (state.data?.result) {
      for (const item of graph.symbols.slice(start, end)) {
        const row = button("", () => selectSymbol(item), "ghost", "code_map.select"); row.classList.add("code-map-result");
        row.dataset.symbol = row.dataset.focus = symbolKey(item); row.classList.toggle("match", item.match);
        row.setAttribute("aria-pressed", String(row.dataset.symbol === selected)); row.title = `${item.path}:${item.symbol.start_line}`;
        row.append(h("span", "code-map-result-name", item.symbol.qualified_name), h("small", "", `${item.path}:${item.symbol.start_line}`));
        rows.push(row);
      }
    } else {
      for (const file of graph.entries.slice(start, end)) {
        const row = button("", () => inspect(file.path), "ghost", "code_map.inspect"); row.classList.add("code-map-result"); row.dataset.focus = file.path;
        row.dataset.kind = file.kind; row.title = file.path;
        row.append(h("span", "code-map-result-name", file.kind === "directory" ? file.path : file.path.split("/").pop()!), h("small", "", file.kind === "directory" ? labels.fileCount(file.files) : file.path)); rows.push(row);
      }
    }
    rows.forEach((row, index) => { row.tabIndex = index === 0 ? 0 : -1; results.append(row); });
    if (!rows.length) results.append(h("p", "ui-hint", state.loading ? labels.loading : state.data?.result
      ? state.data.result.roots.length ? labels.truncated : labels.noResults : labels.empty));
    if (focused) {
      const target = rows.find(row => row.dataset.focus === focused) ?? rows[0];
      target?.focus({ preventScroll: true });
      if (!target) search.focus({ preventScroll: true });
    }
    results.scrollTop = scroll;
  }
  results.addEventListener("keydown", event => {
    const rows = [...results.querySelectorAll<HTMLButtonElement>("button")], index = rows.indexOf(document.activeElement as HTMLButtonElement);
    let next = index;
    if (event.key === "ArrowDown") next = Math.min(rows.length - 1, index + 1);
    else if (event.key === "ArrowUp") next = Math.max(0, index - 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = rows.length - 1;
    else return;
    event.preventDefault(); rows[next]?.focus();
  });
  results.addEventListener("focusin", event => {
    const target = (event.target as Element).closest("button");
    for (const row of results.querySelectorAll("button")) row.tabIndex = row === target ? 0 : -1;
  });
  function paintDetail() {
    const hadFocus = detail.contains(document.activeElement), scroll = detail.scrollTop;
    const focused = hadFocus && document.activeElement instanceof HTMLElement ? document.activeElement.dataset.focus : undefined;
    detail.replaceChildren();
    const item = graph.symbols.find(item => symbolKey(item) === selected);
    detail.hidden = !item;
    inspectorBody.classList.toggle("without-detail", !item);
    if (!item) { selected = null; detail.append(h("p", "ui-hint", state.data?.result ? labels.selectSymbol : labels.selectFile)); return; }
    const { symbol } = item;
    detail.append(h("strong", "code-map-detail-name", symbol.qualified_name), h("p", "code-map-detail-path", `${item.path}:${symbol.start_line}–${symbol.end_line}`));
    const open = button(labels.openSource, () => options.openSource(item.path, symbol.start_line), "ghost", "code_map.open_source");
    open.dataset.focus = "open-source";
    open.insertAdjacentHTML("afterbegin", icon("external-link", 12)); detail.append(open);
    const code = h("pre", "code-map-snippet");
    symbol.snippet.split("\n").forEach((line, index) => {
      const row = h("span", "code-map-code-line"), number = h("span", "code-map-line-number", String(symbol.snippet_start_line + index));
      number.setAttribute("aria-hidden", "true"); row.append(number, document.createTextNode(line)); code.append(row);
    });
    detail.append(code);
    if (symbol.snippet_truncated) detail.append(h("p", "ui-hint", labels.excerptTruncated));
    if (symbol.syntax_errors) detail.append(h("p", "ui-hint", labels.syntaxErrors));
    const relationships = state.data?.result?.edges.filter(edge => edge.from === symbol.id || edge.to === symbol.id) ?? [];
    for (const edge of relationships.slice(0, 20)) {
      const other = graph.symbols.find(item => item.symbol.id === (edge.from === symbol.id ? edge.to : edge.from));
      const metadata = labels.relationship(edge.kind, edge.resolution, edge.confidence, edge.ambiguous);
      const text = `${edge.from === symbol.id ? "→" : "←"} ${other?.symbol.qualified_name ?? edge.target}`;
      const row = other ? button(text, () => { selectSymbol(other); detail.focus({ preventScroll: true }); }, "ghost", "code_map.relationship") : h("div", "", text);
      if (other) row.dataset.focus = symbolKey(other);
      row.classList.add("code-map-relationship"); row.append(h("small", "", `${metadata}${edge.to === null ? ` · ${labels.unresolved}` : ""}`)); detail.append(row);
    }
    if (relationships.length > 20) detail.append(h("p", "ui-hint", labels.truncated));
    detail.scrollTop = scroll;
    if (hadFocus) ([...detail.querySelectorAll<HTMLElement>("[data-focus]")].find(node => node.dataset.focus === focused) ?? detail).focus({ preventScroll: true });
  }
  function closeMenu() { if (roots.control.getAttribute("aria-expanded") === "true") menu.close(); }
  function paint() {
    const data = state.data, nextScope = data?.root_id ?? "";
    const scopeChanged = nextScope !== scope;
    if (scopeChanged) {
      scope = nextScope; selected = selectedFile = folder = null; page = 0;
      paintedOverview = undefined; paintedResult = undefined;
    }
    const changed = scopeChanged || paintedOverview !== data?.overview || paintedResult !== data?.result || paintedFolder !== folder;
    if (changed) {
      const nextGraph = data ? fileGraph(data, folder, selectedFile) : emptyGraph();
      const paths = new Set(nextGraph.files.map(file => file.path));
      const projectionChanged = scopeChanged || paintedFolder !== folder || !!paintedResult !== !!data?.result
        || graph.files.length !== nextGraph.files.length || graph.files.some(file => !paths.has(file.path));
      paintedOverview = data?.overview; paintedResult = data?.result; paintedFolder = folder;
      graph = nextGraph;
      if (selected && !graph.symbols.some(item => symbolKey(item) === selected)) selected = null;
      if (selectedFile && !graph.files.some(file => file.path === selectedFile)) selectedFile = null;
      queueGraph(projectionChanged, true); paintResults(); paintDetail();
    } else if (!data) { paintResults(); paintDetail(); }
    if (search.value !== state.query) search.value = state.query;
    clear.hidden = !search.value;
    back.hidden = !folder && !data?.result;
    const choices = JSON.stringify(data?.roots ?? []);
    if (choices !== rootOptions) { closeMenu(); rootOptions = choices; roots.setOptions(data?.roots.map(root => [root.id, root.label]) ?? [], scope); }
    else if (roots.value !== scope) roots.value = scope;
    roots.root.hidden = (data?.roots.length ?? 0) < 2;
    const selectedRoot = data?.roots.find(root => root.id === scope);
    scopeName.textContent = selectedRoot?.label ?? labels.root; scopeName.title = selectedRoot?.path ?? "";
    expand.setAttribute("aria-pressed", String(!!state.expanded)); expand.setAttribute("aria-label", state.expanded ? labels.collapse : labels.expand);
    expand.title = state.expanded ? labels.collapse : labels.expand;
    root.setAttribute("aria-busy", String(!!state.loading || !!state.syncing));
    feedback.replaceChildren(listState(state.error ? { kind: "error", text: state.error, retry: { label: labels.retry, run: options.retry, track: "code_map.retry" } }
      : state.loading ? { kind: "loading", text: labels.loading } : { kind: "ready" }));
    feedback.hidden = !state.error && !state.loading;
    graphTitle.textContent = data?.result ? labels.results : graph.directories ? labels.directories : folder ?? labels.overview;
    count.textContent = graph.directories ? labels.directoryCount(graph.files.length, graph.entries.reduce((count, file) => count + file.files, 0))
      : labels.graphCount(graph.files.length, graph.edges.length, graph.entries.length);
    drawing.setAttribute("aria-label", `${graphTitle.textContent} · ${count.textContent}`);
    graphMessage.textContent = !graph.files.length ? state.loading ? labels.loading : data?.result ? labels.noResults : labels.empty
      : data?.result && !data.result.roots.length && !graph.symbols.length ? labels.noResults : "";
    graphMessage.hidden = !graphMessage.textContent;
    truncated.hidden = !graph.truncated;
    uncertainty.hidden = !graph.edges.some(edge => edge.uncertain) && !data?.result?.ambiguous;
    sync.disabled = !data || !!state.loading || !!state.syncing;
    sync.textContent = state.syncing ? labels.syncing : labels.sync;
    syncTime.textContent = labels.lastSync(data?.status.last_indexed ?? null);
    generation.textContent = data ? labels.generation(data.overview.generation) + (data.result && data.result.generation !== data.overview.generation ? ` / ${labels.generation(data.result.generation)}` : "") : "";
  }
  paint();
  const resize = new ResizeObserver(() => {
    const nextWidth = drawing.clientWidth;
    queueGraph(width > 0 && nextWidth > 0 && width !== nextWidth);
    if (nextWidth > 0) width = nextWidth;
  }); resize.observe(stage);
  return {
    root,
    update(next: CodeMapViewState) { state = next; paint(); },
    focusSearch() { search.focus(); },
    destroy() { destroyed = true; closeMenu(); resize.disconnect(); cancelAnimationFrame(frame); renderer?.destroy(); root.remove(); },
  };
}
