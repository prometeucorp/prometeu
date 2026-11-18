/** Private host ↔ terminal emulator contract. This is not the agent conversation protocol. */
export const VERSION = 1;
export const LIMITS = Object.freeze({
  sessions: 32,
  cols: 500,
  rows: 300,
  scrollback: 5_000,
  cellsPerSession: 1_000_000,
  totalCells: 4_000_000,
  writeBytes: 256 * 1024,
  lineBytes: 512 * 1024,
  snapshotBytes: 16 * 1024 * 1024,
  viewportCharacters: 256 * 1024,
  escapeChars: 4_096,
  titleChars: 4_096,
  queuedRequests: 128,
});

import type { Request, ErrorCode } from "./types.ts";
export type * from "./types.ts";

export class ProtocolError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode) { super(code); this.code = code; }
}
export function requireThat(condition: unknown, code: ErrorCode): asserts condition {
  if (!condition) throw new ProtocolError(code);
}
export function dimensions(cols: unknown, rows: unknown): asserts cols is number {
  requireThat(Number.isInteger(cols) && Number.isInteger(rows), "invalid_request");
  requireThat((cols as number) >= 2 && (cols as number) <= LIMITS.cols
    && (rows as number) >= 1 && (rows as number) <= LIMITS.rows, "limit");
}
export function decodeBase64(data: unknown): Buffer {
  requireThat(typeof data === "string", "invalid_request");
  requireThat(data.length <= Math.ceil(LIMITS.writeBytes / 3) * 4, "limit");
  requireThat(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(data), "invalid_request");
  const bytes = Buffer.from(data, "base64");
  requireThat(bytes.length <= LIMITS.writeBytes && bytes.toString("base64") === data, "invalid_request");
  return bytes;
}
export function parseRequest(input: unknown): Request {
  requireThat(input && typeof input === "object" && !Array.isArray(input), "invalid_request");
  const r = input as Record<string, unknown>;
  requireThat(r.v === VERSION, "invalid_request");
  for (const key of ["id", "session", "generation"])
    requireThat(typeof r[key] === "string" && /^[A-Za-z0-9_.:-]{1,128}$/.test(r[key]), "invalid_request");
  requireThat(["create", "write", "resize", "snapshot", "dispose"].includes(r.op as string), "invalid_request");
  if (r.op === "create" || r.op === "resize") dimensions(r.cols, r.rows);
  if (r.op === "create") requireThat(r.scrollback === undefined || (Number.isInteger(r.scrollback)
    && (r.scrollback as number) >= 0 && (r.scrollback as number) <= LIMITS.scrollback), "limit");
  if (r.op === "write" || r.op === "resize") requireThat(Number.isSafeInteger(r.seq) && (r.seq as number) >= 0, "invalid_request");
  if (r.op === "write") decodeBase64(r.data);
  return r as Request;
}
