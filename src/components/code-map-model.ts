import type { CodeMapSnapshot, CodeMapSymbol } from "../code-map-types";

export const GRAPH_LIMIT = { nodes: 80, edges: 160, rows: 40 } as const;
export type MapFile = { path: string; language: string; symbols: number; files: number; kind: "file" | "directory"; match: boolean; context: boolean };
export type MapEdge = { from: string; to: string; count: number; uncertain: boolean; highlighted: boolean };
export type MapSymbol = { path: string; symbol: CodeMapSymbol; match: boolean };
export type FileGraph = { files: MapFile[]; entries: MapFile[]; edges: MapEdge[]; symbols: MapSymbol[]; directories: boolean; truncated: boolean };
const edgeKey = (from: string, to: string) => JSON.stringify([from, to]);
export const directory = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : ".";

/** Render directory context or a bounded file neighborhood, never the full repository's edge set. */
export function fileGraph(data: CodeMapSnapshot, folder: string | null = null, selected?: string | null): FileGraph {
  const directories = !data.result && folder === null;
  const group = (path: string) => directory(path).split("/").slice(0, 2).join("/");
  const files = new Map<string, MapFile>();
  const symbols: MapSymbol[] = [];
  const locations = new Map<number, string>();
  const matches = new Set(data.result?.roots.map(root => root.id));
  const edges = new Map<string, MapEdge>();
  const addEdge = (from: string, to: string, count: number, uncertain: boolean, highlighted: boolean) => {
    if (from === to || !files.has(from) || !files.has(to)) return;
    const key = edgeKey(from, to), previous = edges.get(key);
    if (previous) { previous.count += count; previous.uncertain ||= uncertain; }
    else edges.set(key, { from, to, count, uncertain, highlighted });
  };
  if (!data.result) {
    for (const file of data.overview.files) {
      if (folder !== null && !(folder === "." ? directory(file.path) === "." : file.path.startsWith(folder + "/"))) continue;
      const path = directories ? group(file.path) : file.path, previous = files.get(path);
      if (previous) { previous.symbols += file.symbols; previous.files++; }
      else files.set(path, { ...file, path, kind: directories ? "directory" : "file", files: 1, match: false, context: false });
    }
    for (const edge of data.overview.edges) addEdge(directories ? group(edge.from) : edge.from, directories ? group(edge.to) : edge.to, edge.count, edge.uncertain, false);
  }
  for (const file of data.result?.files ?? []) {
    files.set(file.path, { path: file.path, language: "", symbols: file.symbols.length, files: 1, kind: "file", match: false, context: true });
    for (const symbol of file.symbols) {
      locations.set(symbol.id, file.path);
      symbols.push({ path: file.path, symbol, match: matches.has(symbol.id) });
    }
  }
  for (const location of data.result?.roots ?? []) {
    locations.set(location.id, location.path);
    if (!files.has(location.path)) files.set(location.path, { path: location.path, language: "", symbols: 0, files: 1, kind: "file", match: true, context: true });
    files.get(location.path)!.match = true;
  }
  for (const edge of data.result?.edges ?? []) {
    const from = locations.get(edge.from), to = edge.to === null ? undefined : locations.get(edge.to);
    if (from && to) addEdge(from, to, 1, edge.ambiguous || edge.confidence === "low" || edge.confidence === "unknown", true);
  }
  const degree = new Map<string, number>();
  for (const edge of edges.values()) for (const path of [edge.from, edge.to]) degree.set(path, (degree.get(path) ?? 0) + edge.count);
  const neighbors = new Set<string>();
  for (const edge of edges.values()) {
    if (edge.from === selected) neighbors.add(edge.to);
    if (edge.to === selected) neighbors.add(edge.from);
  }
  const entries = [...files.values()].sort((a, b) => a.path.localeCompare(b.path));
  const visible = [...entries].sort((a, b) => Number(b.path === selected) - Number(a.path === selected)
    || Number(neighbors.has(b.path)) - Number(neighbors.has(a.path)) || Number(b.match) - Number(a.match)
    || (degree.get(b.path) ?? 0) - (degree.get(a.path) ?? 0) || a.path.localeCompare(b.path)).slice(0, GRAPH_LIMIT.nodes);
  const shown = new Set(visible.map(file => file.path));
  const connected = [...edges.values()].filter(edge => shown.has(edge.from) && shown.has(edge.to)).sort((a, b) =>
    Number(b.from === selected || b.to === selected) - Number(a.from === selected || a.to === selected) || b.count - a.count);
  return { files: visible, entries, edges: connected.slice(0, GRAPH_LIMIT.edges), directories,
    symbols: symbols.sort((a, b) => Number(b.match) - Number(a.match)),
    truncated: visible.length < entries.length || connected.length > GRAPH_LIMIT.edges || (data.result ? data.result.truncated : data.overview.truncated) };
}
