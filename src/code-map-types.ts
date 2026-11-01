/** Local code-map IPC. Symbol IDs belong to their graph generation; file paths identify overview nodes. */
export type CodeMapRoot = { id: string; label: string; path: string; relative: string };
export type CodeMapSymbolLocation = { id: number; qualified_name: string; name: string; path: string; start_line: number };
export type CodeMapSymbol = {
  id: number; name: string; qualified_name: string; kind: string;
  start_line: number; end_line: number; snippet_start_line: number;
  snippet: string; snippet_truncated: boolean; syntax_errors: boolean;
};
export type CodeMapEdge = {
  from: number; to: number | null; target: string; line: number;
  kind: string; resolution: string; confidence: string; ambiguous: boolean;
};
export type CodeMapGraph = {
  generation: number; roots: CodeMapSymbolLocation[];
  files: { path: string; symbols: CodeMapSymbol[] }[];
  edges: CodeMapEdge[]; path: number[]; impact: CodeMapSymbolLocation[];
  affected_tests: string[]; ambiguous: boolean; truncated: boolean;
};
export type CodeMapSnapshot = {
  roots: CodeMapRoot[]; root_id: string;
  status: { generation: number; last_indexed: number | null; files: number; symbols: number; references: number };
  overview: {
    generation: number;
    files: { path: string; language: string; symbols: number }[];
    edges: { from: string; to: string; count: number; uncertain: boolean }[];
    truncated: boolean;
  };
  result: CodeMapGraph | null;
};
export type CodeMapRequest = { id: string; root?: string; query?: string; path?: string; line?: number; force?: boolean };
