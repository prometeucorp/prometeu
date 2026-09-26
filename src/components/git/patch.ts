import type { Change, RepoDiff } from "../../types";

export const key = (repo: string, path: string) => `${repo}/${path}`;

export const keys = (repos: RepoDiff[]) => repos.flatMap((r) => r.files.map((f) => key(r.name, f.path)));

export const sum = (files: Change[], of: "added" | "removed") => files.reduce((n, c) => n + c[of], 0);

const stamps = new WeakMap<Change, string>();
export function stamp(c: Change): string {
  let s = stamps.get(c);
  if (s === undefined) {
    let h = 5381;
    for (let i = 0; i < c.patch.length; i++) h = (Math.imul(h, 33) ^ c.patch.charCodeAt(i)) >>> 0;
    s = `${h.toString(36)}:${c.added}:${c.removed}`;
    stamps.set(c, s);
  }
  return s;
}

export type Row = {
  kind: "hunk" | "ctx" | "add" | "del";
  before: number | null;
  after: number | null;
  text: string;
};

/// Each side keeps its own line numbers. Metadata outside hunks is not content; newline markers consume no positions.
export function rows(patch: string): Row[] {
  const out: Row[] = [];
  let before = 0, after = 0, inHunk = false;
  for (const line of patch.split("\n")) {
    if (!line || line.startsWith("\\")) continue;
    if (line.startsWith("@@")) {
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(?: (.*))?$/.exec(line);
      inHunk = !!m;
      if (!m) continue;
      before = Number(m[1]);
      after = Number(m[2]);
      out.push({ kind: "hunk", before: null, after: null, text: line });
      continue;
    }
    if (!inHunk) continue;
    const text = line.slice(1);
    if (line[0] === "+") out.push({ kind: "add", before: null, after: after++, text });
    else if (line[0] === "-") out.push({ kind: "del", before: before++, after: null, text });
    else if (line[0] === " ") out.push({ kind: "ctx", before: before++, after: after++, text });
    else inHunk = false;
  }
  return out;
}

export type SplitRow = { kind: "hunk"; hunk: Row } | { kind: "line"; before: Row | null; after: Row | null };

/// Align each deletion/addition block without crossing context or hunk boundaries. Unmatched sides remain empty without repeated text or line numbers.
export function splitRows(all: Row[]): SplitRow[] {
  const out: SplitRow[] = [];
  for (let i = 0; i < all.length;) {
    const current = all[i];
    if (current.kind === "hunk") { out.push({ kind: "hunk", hunk: current }); i++; continue; }
    if (current.kind === "ctx") { out.push({ kind: "line", before: current, after: current }); i++; continue; }
    const before: Row[] = [], after: Row[] = [];
    while (i < all.length && (all[i].kind === "del" || all[i].kind === "add")) {
      (all[i].kind === "del" ? before : after).push(all[i++]);
    }
    for (let n = 0; n < Math.max(before.length, after.length); n++) {
      out.push({ kind: "line", before: before[n] ?? null, after: after[n] ?? null });
    }
  }
  return out;
}
