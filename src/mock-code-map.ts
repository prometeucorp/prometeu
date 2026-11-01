import type { CodeMapRequest, CodeMapRoot, CodeMapSnapshot, CodeMapSymbol } from "./code-map-types";

/** Synthetic browser source shared with the mock file viewer; never used by native indexing. */
export const codeMapFiles: Record<string, string> = {
  "src/search.ts": 'export function findSymbol(query: string) {\n  return query.trim().toLowerCase();\n}\n',
  "src/session.ts": 'import { findSymbol } from "./search";\nexport function searchSession(query: string) {\n  return findSymbol(query);\n}\n',
  "src/main.ts": 'import { searchSession } from "./session";\nexport function start() {\n  return searchSession("session");\n}\n',
};
const indexed = new Map<string, { text: string; generation: number }>();

export function mockCodeMap(roots: CodeMapRoot[], request: CodeMapRequest, files: Record<string, string>): CodeMapSnapshot {
  const root = request.root ? roots.find(root => root.id === request.root) : roots[0];
  if (!root) throw 'i18n:{"code":"err.search","args":{"cause":"unknown root"}}';
  const entries = Object.entries(files).filter(([path]) => /\.(ts|js|rb|rs)$/.test(path));
  const text = JSON.stringify(entries);
  const previous = indexed.get(root.id);
  const generation = (previous?.generation ?? 0) + Number(previous?.text !== text);
  indexed.set(root.id, { text, generation });
  const symbols = entries.map(([path, snippet], index) => ({ path, symbol: {
    id: index + 1, name: snippet.match(/function\s+(\w+)|class\s+(\w+)/)?.slice(1).find(Boolean) ?? path.split("/").pop()!,
    qualified_name: "", kind: "function", start_line: snippet.startsWith("import ") ? 2 : 1,
    end_line: snippet.trimEnd().split("\n").length, snippet_start_line: 1, snippet,
    snippet_truncated: false, syntax_errors: false,
  } satisfies CodeMapSymbol }));
  for (const entry of symbols) entry.symbol.qualified_name = entry.symbol.name;
  const overview = {
    generation, truncated: false,
    files: symbols.map(({ path }) => ({ path, language: path.endsWith(".rb") ? "ruby" : "typescript", symbols: 1 })),
    edges: [["src/main.ts", "src/session.ts"], ["src/session.ts", "src/search.ts"]]
      .filter(([from, to]) => symbols.some(s => s.path === from) && symbols.some(s => s.path === to))
      .map(([from, to]) => ({ from, to, count: 1, uncertain: false })),
  };
  const query = request.query?.trim().toLowerCase();
  const matches = query ? symbols.filter(({ path, symbol }) => request.path ? path === request.path
    : `${path} ${symbol.name} ${symbol.snippet}`.toLowerCase().includes(query)) : [];
  const neighbors = new Set(matches.map(match => match.path));
  for (const edge of overview.edges) if (neighbors.has(edge.from) || neighbors.has(edge.to)) { neighbors.add(edge.from); neighbors.add(edge.to); }
  return {
    roots, root_id: root.id,
    status: { generation, last_indexed: Math.floor(Date.now() / 1000), files: symbols.length, symbols: symbols.length, references: overview.edges.length },
    overview,
    result: query ? {
      generation, roots: matches.map(({ path, symbol }) => ({ id: symbol.id, path, name: symbol.name, qualified_name: symbol.name, start_line: symbol.start_line })),
      files: symbols.filter(s => neighbors.has(s.path)).map(({ path, symbol }) => ({ path, symbols: [symbol] })),
      edges: overview.edges.filter(edge => neighbors.has(edge.from) && neighbors.has(edge.to)).map(edge => ({
        from: symbols.find(s => s.path === edge.from)!.symbol.id, to: symbols.find(s => s.path === edge.to)!.symbol.id,
        target: symbols.find(s => s.path === edge.to)!.symbol.name, line: 3, kind: "call", resolution: "import", confidence: "medium", ambiguous: false,
      })),
      path: [], impact: [], affected_tests: [], ambiguous: false, truncated: false,
    } : null,
  };
}
