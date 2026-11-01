import { describe, expect, it } from "vitest";
import type { CodeMapSnapshot, CodeMapSymbol } from "../code-map-types";
import { GRAPH_LIMIT, fileGraph } from "./code-map-model";

const symbol = (id: number, name: string): CodeMapSymbol => ({ id, name, qualified_name: name, kind: "function", start_line: 4, end_line: 6,
  snippet_start_line: 4, snippet: `function ${name}() {}`, snippet_truncated: false, syntax_errors: false });
const snapshot = (): CodeMapSnapshot => ({
  roots: [{ id: "repo", label: "Repository", path: "/worktree", relative: "" }], root_id: "repo",
  status: { generation: 2, last_indexed: null, files: 20, symbols: 40, references: 30 },
  overview: { generation: 1, files: ["src/main.ts", "src/search.ts", "src/background.ts"].map(path => ({ path, language: "typescript", symbols: 4 })),
    edges: [{ from: "src/main.ts", to: "src/search.ts", count: 3, uncertain: false }], truncated: true },
  result: { generation: 2, roots: [{ id: 1, name: "search", qualified_name: "search", path: "src/search.ts", start_line: 4 }],
    files: [{ path: "src/search.ts", symbols: [symbol(1, "search")] }, { path: "extra/worker.ts", symbols: [symbol(2, "worker")] }],
    edges: [{ from: 1, to: 2, target: "worker", line: 5, kind: "call", resolution: "local", confidence: "low", ambiguous: true },
      { from: 2, to: null, target: "unknown", line: 6, kind: "call", resolution: "unresolved", confidence: "low", ambiguous: false },
      { from: 999, to: 1, target: "search", line: 8, kind: "call", resolution: "local", confidence: "high", ambiguous: false }],
    path: [], impact: [], affected_tests: [], ambiguous: true, truncated: false },
});

describe("code map presentation model", () => {
  it("renders only returned query context with resolved connections, independent of overview truncation", () => {
    const graph = fileGraph(snapshot());
    expect(graph.files.map(file => file.path)).toEqual(["src/search.ts", "extra/worker.ts"]);
    expect(graph.files.filter(file => file.match).map(file => file.path)).toEqual(["src/search.ts"]);
    expect(graph.truncated).toBe(false);
    expect(graph.edges).toEqual([
      { from: "src/search.ts", to: "extra/worker.ts", count: 1, uncertain: true, highlighted: true },
    ]);
    expect(graph.symbols.map(item => [item.symbol.id, item.match])).toEqual([[1, true], [2, false]]);
  });

  it("keeps a root omitted from bounded snippets and uses query evidence without unrelated overview counts", () => {
    const data = snapshot();
    data.result!.roots.push({ id: 3, name: "omitted", qualified_name: "omitted", path: "extra/omitted.ts", start_line: 1 });
    data.overview.edges.push({ from: "src/search.ts", to: "extra/worker.ts", count: 8, uncertain: false });
    const graph = fileGraph(data);
    expect(graph.files.find(file => file.path === "extra/omitted.ts")).toMatchObject({ match: true });
    expect(graph.edges.filter(edge => edge.to === "extra/worker.ts")).toEqual([
      { from: "src/search.ts", to: "extra/worker.ts", count: 1, uncertain: true, highlighted: true },
    ]);
  });

  it("marks weak evidence without treating medium confidence as ambiguous", () => {
    const data = snapshot();
    data.result!.edges[0].ambiguous = false;
    for (const confidence of ["low", "unknown", "medium", "high"]) {
      data.result!.edges[0].confidence = confidence;
      expect(fileGraph(data).edges[0].uncertain).toBe(["low", "unknown"].includes(confidence));
    }
  });

  it("aggregates real directory connections and preserves counts and uncertainty", () => {
    const data = snapshot(); data.result = null; data.overview.truncated = false;
    data.overview.files.push({ path: "relay/worker.ts", language: "typescript", symbols: 2 });
    data.overview.edges.push({ from: "src/main.ts", to: "relay/worker.ts", count: 2, uncertain: false },
      { from: "src/search.ts", to: "relay/worker.ts", count: 3, uncertain: true });
    const graph = fileGraph(data);
    expect(graph.directories).toBe(true);
    expect(graph.entries.find(file => file.path === "src")).toMatchObject({ files: 3, symbols: 12, kind: "directory" });
    expect(graph.edges).toEqual([{ from: "src", to: "relay", count: 5, uncertain: true, highlighted: false }]);
    expect(fileGraph(data, "src").entries.map(file => file.path)).toEqual(["src/background.ts", "src/main.ts", "src/search.ts"]);
  });

  it("bounds a dense repository and keeps a selected file beyond the initial limit visible", () => {
    const data = snapshot(); data.result = null; data.overview.truncated = false;
    data.overview.files = Array.from({ length: 1000 }, (_, index) => ({ path: `src/group-${index % 30}/file-${index}.ts`, language: "typescript", symbols: 1 }));
    data.overview.edges = data.overview.files.flatMap((file, index, files) => Array.from({ length: 4 }, (_, offset) => ({
      from: file.path, to: files[(index + offset + 1) % files.length].path, count: 1, uncertain: false,
    })));
    expect(data.overview.edges).toHaveLength(4000);
    expect(fileGraph(data).files).toHaveLength(30);
    const scoped = fileGraph(data, "src");
    expect(scoped.entries).toHaveLength(1000);
    expect(scoped.files).toHaveLength(GRAPH_LIMIT.nodes);
    expect(scoped.edges.length).toBeLessThanOrEqual(GRAPH_LIMIT.edges);
    expect(scoped.truncated).toBe(true);
    const omitted = scoped.entries.find(file => !scoped.files.some(node => node.path === file.path))!;
    const focused = fileGraph(data, "src", omitted.path);
    expect(focused.files[0].path).toBe(omitted.path);
    expect(focused.edges.some(edge => edge.from === omitted.path || edge.to === omitted.path)).toBe(true);
    expect(focused.edges[0].from === omitted.path || focused.edges[0].to === omitted.path).toBe(true);
  });

});
