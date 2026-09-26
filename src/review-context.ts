import type { ReviewDraft, Span } from "./review-comments";
import { bodyValid } from "./review-comments";
export type ReviewContext = { batch: string; comments: Array<{ n: number; repo: string; file: string; in: "worktree" | "index" | "HEAD"; old: Span; new: Span; hunk: string | null; excerpt: string[]; body: string }> };
const OPEN = '<prometeu-review v="1">', CLOSE = '</prometeu-review>';
export const REVIEW_BYTES = 256 * 1024;
export const record = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);
export const exactKeys = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).length === keys.length && keys.every(key => Object.prototype.hasOwnProperty.call(value, key));
export const bounded = (value: unknown, limit: number): value is string => typeof value === "string" && value.length <= limit && new TextEncoder().encode(value).length <= limit;
export const safeReviewPath = (value: unknown): value is string => bounded(value, 4096) && !!value && !/^[\/\\]|^[a-zA-Z]:|[\u0000\r\n\\]/.test(value) && value.split("/").every(part => !!part && part !== "." && part !== "..");
export const validSpan = (v: unknown): v is Span => v === null || (Array.isArray(v) && v.length === 2 && v.every(n => Number.isSafeInteger(n) && n > 0) && v[0] <= v[1]);
function valid(value: unknown): value is ReviewContext {
  return record(value) && exactKeys(value, ["batch", "comments"]) && bounded(value.batch, 128) && !!value.batch && Array.isArray(value.comments) && value.comments.length > 0 && value.comments.length <= 100 && value.comments.every((c, i) =>
    record(c) && exactKeys(c, ["n", "repo", "file", "in", "old", "new", "hunk", "excerpt", "body"])
    && c.n === i + 1 && bounded(c.repo, 4096) && !!c.repo && safeReviewPath(c.file)
    && ["worktree", "index", "HEAD"].includes(c.in as string) && validSpan(c.old) && validSpan(c.new)
    && (c.hunk === null || bounded(c.hunk, 4096)) && Array.isArray(c.excerpt) && c.excerpt.length <= 40
    && c.excerpt.every(line => typeof line === "string" && line.length <= 400 && !/[\r\n\0]/.test(line)) && bodyValid(c.body));
}
export function encodeReviewContext(value: ReviewContext): string {
  if (!valid(value)) throw new Error("review.limit");
  const text = `${OPEN}\n${JSON.stringify(value).replace(/</g, "\\u003c")}\n${CLOSE}`;
  if (new TextEncoder().encode(text).length > REVIEW_BYTES) throw new Error("review.limit");
  return text;
}
export function decodeReviewContext(text: string): ReviewContext | null {
  if (text.length > REVIEW_BYTES || new TextEncoder().encode(text).length > REVIEW_BYTES) return null;
  const lines = text.split("\n");
  if (lines.length !== 3 || lines[0] !== OPEN || lines[2] !== CLOSE) return null;
  try { const value: unknown = JSON.parse(lines[1]); return valid(value) ? value : null; } catch { return null; }
}
export function reviewContext(draft: ReviewDraft): ReviewContext {
  return { batch: draft.batch, comments: draft.items.map((item, i) => ({ n: i + 1, repo: item.anchor.repo, file: item.file, in: item.anchor.scope === "changes" ? "worktree" : item.anchor.scope === "staged" ? "index" : "HEAD", old: item.anchor.old, new: item.anchor.new, hunk: item.anchor.hunk, excerpt: item.anchor.excerpt, body: item.body })) };
}
