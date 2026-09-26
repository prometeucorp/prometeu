import { rows, type Row } from "./components/git/patch";

export type ReviewScope = "changes" | "staged" | "compare";
export type Span = [number, number] | null;
export type ReviewSide = "both" | "before" | "after";
export type ReviewAnchor = {
  repo: string; path: string; scope: ReviewScope; reference: string; side: ReviewSide; stamp: string;
  hunk: string | null; old: Span; new: Span; excerpt: string[];
  match: { count: number; hash: string } | null;
};
export type ReviewEditorDraft = { anchor: ReviewAnchor; body: string };
export type ReviewNote = {
  id: string; revision: number; anchor: ReviewAnchor; body: string;
  state: "draft" | "sent" | "resolved";
  sent: { batch: string; tab: string; at: number; revision: number; anchor: ReviewAnchor } | null;
  created: number; updated: number;
};
export type Placement = { kind: "attached"; anchor: ReviewAnchor } | { kind: "changed" | "detached" | "ambiguous" | "unknown" };
export type ReviewDraft = { workspace: string; batch: string; items: Array<{ id: string; revision: number; anchor: ReviewAnchor; body: string; file: string }> };
export const REVIEW_ROWS = 2500;
export const bodyValid = (body: unknown): body is string => typeof body === "string" && !!body.trim() && new TextEncoder().encode(body).length <= 8192;

// Two independent accumulators keep full-selection evidence bounded, even for long lines.
function fingerprint(values: string[]): string {
  let a = 2166136261, b = 5381;
  for (const value of values) for (const char of `${value.length}:${value}`) {
    const n = char.codePointAt(0)!;
    a = Math.imul(a ^ n, 16777619); b = Math.imul(b, 33) ^ n;
  }
  return `${(a >>> 0).toString(36)}-${(b >>> 0).toString(36)}`;
}
const selected = (all: Row[], side: ReviewSide) => all.filter(r => r.kind !== "hunk" && (side === "both" || r[side] !== null));
const notation = (r: Row) => (r.kind === "add" ? "+" : r.kind === "del" ? "-" : " ") + r.text;
// Prefix hashes make each candidate range constant-time, instead of rehashing long
// selections at every line. Display truncation never participates in matching.
function windows(all: Row[], side: ReviewSide) {
  const a = [0], b = [0], pa = [1], pb = [1];
  for (const row of all) {
    const [x, y] = fingerprint([side === "both" ? notation(row) : row.text]).split("-").map(v => parseInt(v, 36));
    a.push((Math.imul(a[a.length - 1], 31) + x) >>> 0);
    b.push((Math.imul(b[b.length - 1], 37) + y) >>> 0);
    pa.push(Math.imul(pa[pa.length - 1], 31) >>> 0); pb.push(Math.imul(pb[pb.length - 1], 37) >>> 0);
  }
  return (start: number, count: number) => `${((a[start + count] - Math.imul(a[start], pa[count])) >>> 0).toString(36)}-${((b[start + count] - Math.imul(b[start], pb[count])) >>> 0).toString(36)}`;
}
const evidence = (all: Row[], side: ReviewSide) => windows(all, side)(0, all.length);
const span = (all: Row[], side: "before" | "after"): Span => {
  const numbers = all.flatMap(r => r[side] === null ? [] : [r[side]!]);
  return numbers.length ? [numbers[0], numbers[numbers.length - 1]] : null;
};
function located(base: ReviewAnchor, all: Row[], hunk: string): ReviewAnchor {
  const quote = all.map(row => notation(row).replace(/\r$/, ""));
  const excerpt = (quote.length > 40 ? [...quote.slice(0, 19), "…", ...quote.slice(-20)] : quote).map(line => line.length > 400 ? line.slice(0, 399) + "…" : line);
  return { ...base, hunk, old: base.side === "after" ? null : span(all, "before"), new: base.side === "before" ? null : span(all, "after"), excerpt, match: { count: all.length, hash: evidence(all, base.side) } };
}

/** Indexes address canonical patch rows, not visual split rows. */
export function anchorSelection(repo: string, path: string, scope: ReviewScope, patch: string, start?: number, end = start, side: ReviewSide = "both", reference = ""): ReviewAnchor {
  const all = rows(patch).slice(0, REVIEW_ROWS);
  const base: ReviewAnchor = { repo, path, scope, reference, side, stamp: fingerprint([patch]), hunk: null, old: null, new: null, excerpt: [], match: null };
  if (!all.some(r => r.kind !== "hunk")) throw new Error("review.selection");
  if (start === undefined) return base;
  if (end === undefined || !Number.isInteger(start) || !Number.isInteger(end)) throw new Error("review.selection");
  const lo = Math.min(start, end), hi = Math.max(start, end);
  if (lo < 0 || hi >= all.length || all.slice(lo, hi + 1).some(r => r.kind === "hunk")) throw new Error("review.selection");
  const content = selected(all.slice(lo, hi + 1), side);
  const hunk = all.slice(0, lo).reverse().find(r => r.kind === "hunk");
  if (!content.length || !hunk) throw new Error("review.selection");
  return located(base, content, hunk.text);
}

/** Derived per scope; never rewrite original or submitted anchors during navigation. */
export function place(anchor: ReviewAnchor, patch: string | null): Placement {
  if (patch === null) return { kind: "detached" };
  const stamp = fingerprint([patch]);
  if (stamp === anchor.stamp) return { kind: "attached", anchor };
  if (!anchor.match) return { kind: "attached", anchor: { ...anchor, stamp } };
  const matches: Array<{ chunk: Row[]; header: string; distance: number }> = [];
  const all = rows(patch);
  const origin = anchor.new?.[0] ?? anchor.old?.[0] ?? 0;
  for (let i = 0; i < all.length;) {
    const header = all[i++];
    if (header.kind !== "hunk") continue;
    const content: Row[] = [];
    while (i < all.length && all[i].kind !== "hunk") content.push(all[i++]);
    const candidates = selected(content, anchor.side);
    const hash = windows(candidates, anchor.side);
    for (let n = 0; n + anchor.match.count <= candidates.length; n++) {
      if (hash(n, anchor.match.count) !== anchor.match.hash) continue;
      const chunk = candidates.slice(n, n + anchor.match.count);
      const position = (anchor.side !== "before" ? span(chunk, "after") : null) ?? span(chunk, "before");
      matches.push({ chunk, header: header.text, distance: Math.abs((position?.[0] ?? 0) - origin) });
    }
  }
  matches.sort((a, b) => a.distance - b.distance);
  if (!matches.length) return { kind: "changed" };
  if (matches[1]?.distance === matches[0].distance) return { kind: "ambiguous" };
  return { kind: "attached", anchor: located({ ...anchor, stamp }, matches[0].chunk, matches[0].header) };
}

export function reviewBatch(workspace: string, batch: string, notes: ReviewNote[], path: (anchor: ReviewAnchor) => string): ReviewDraft {
  return { workspace, batch, items: notes.map(note => ({ id: note.id, revision: note.revision, anchor: structuredClone(note.anchor), body: note.body, file: path(note.anchor) })) };
}
export function mergeReviewDraft(previous: ReviewDraft | undefined, next: ReviewDraft): ReviewDraft {
  if (!previous || previous.workspace !== next.workspace) return structuredClone(next);
  const items = new Map(previous.items.map(item => [item.id, item]));
  for (const item of next.items) items.set(item.id, item);
  return structuredClone({ ...previous, items: [...items.values()] });
}
