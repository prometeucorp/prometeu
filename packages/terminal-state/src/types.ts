/** Browser-safe wire types. Runtime validation belongs to the private Node helper. */
import type { Appearance } from "./appearance.js";
type Envelope = { v: 1; id: string; session: string; generation: string };
export type Request = Envelope & (
  | { op: "create"; cols: number; rows: number; scrollback?: number }
  | { op: "write"; data: string; seq: number }
  | { op: "resize"; cols: number; rows: number; seq: number }
  | { op: "snapshot"; view?: boolean }
  | { op: "dispose" }
);

export type Modes = {
  applicationCursorKeysMode: boolean;
  applicationKeypadMode: boolean;
  bracketedPasteMode: boolean;
  insertMode: boolean;
  originMode: boolean;
  reverseWraparoundMode: boolean;
  sendFocusMode: boolean;
  wraparoundMode: boolean;
  mouseTrackingMode: "none" | "x10" | "vt200" | "drag" | "any";
  mouseEncoding: "DEFAULT" | "SGR" | "SGR_PIXELS";
  cursorHidden: boolean;
  cursorStyle: "block" | "underline" | "bar";
  cursorBlink: boolean;
};
export type Cursor = { x: number; y: number; wrapPending: boolean };
export type Screen = {
  ansi: string;
  cursor: Cursor;
  scrollTop: number;
  scrollBottom: number;
  savedCursor: { x: number; y: number; origin: boolean; attributes: string };
  wrapCell: { col: number; ansi: string } | null;
};
export type OscLink = {
  buffer: "normal" | "alternate";
  row: number;
  start: number;
  end: number;
  uri: string;
  id?: string;
};
export type TerminalColor = { mode: "default" | "palette" | "rgb"; value: number };
export type ViewportStyleRun = {
  /** Zero-based columns; end is exclusive. Runs cover a complete row without gaps. */
  start: number;
  end: number;
  dim: boolean;
  inverse: boolean;
  foreground: TerminalColor;
  background: TerminalColor;
};
export type Snapshot = {
  v: 1;
  session: string;
  generation: string;
  seq: number;
  cols: number;
  rows: number;
  scrollback: number;
  /** Current active viewport, excluding scrollback and trailing unoccupied cells. */
  viewportLines: string[];
  /** For each Unicode scalar in viewportLines, its zero-based xterm cell column. */
  viewportCharacterColumns: number[][];
  viewportStyles: ViewportStyleRun[][];
  appearance: Appearance;
  active: "normal" | "alternate";
  normal: Screen;
  alternate: Screen | null;
  modes: Modes;
  rehydrate: string;
  title: string;
  titleTruncated: boolean;
  links: OscLink[];
  /** Apply after all screen/mode restoration; these bytes are included in seq. */
  pendingEscape: string;
  /** Seed the raw output decoder without displaying it; included in seq. */
  pendingUtf8: string;
};
export type Result = { seq: number; replies?: string[] } | Snapshot;
export type ErrorCode = "invalid_request" | "limit" | "not_found" | "stale_generation"
  | "sequence" | "already_exists" | "snapshot_unavailable" | "closed" | "internal";
export type Response = { v: 1; id: string | null } & (
  | { ok: true; result: Result }
  | { ok: false; error: { code: ErrorCode } }
);

