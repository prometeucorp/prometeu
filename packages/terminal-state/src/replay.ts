import type { Screen, Snapshot } from "./types.js";

const ESC = "\x1b";

/** Browser-safe replay into a reset terminal of snapshot.cols/rows, with all input suppressed.
 * Seed its output decoder from pendingUtf8 before accepting bytes after snapshot.seq.
 */
export function replayAnsi(snapshot: Snapshot): string {
  const restorePosition = (s: Screen, origin: boolean): string => {
    const saved = `${ESC}[?6l${ESC}[r${ESC}[${s.savedCursor.y + 1};${s.savedCursor.x + 1}H${s.savedCursor.attributes}${ESC}7`;
    const margins = `${ESC}[${s.scrollTop + 1};${s.scrollBottom + 1}r${ESC}[?6${origin ? "h" : "l"}`;
    const y = s.cursor.y - (origin ? s.scrollTop : 0);
    const position = s.wrapCell
      ? `${ESC}[4l${ESC}[?7h${ESC}[${y + 1};${s.wrapCell.col + 1}H${s.wrapCell.ansi}`
      : `${ESC}[${y + 1};${s.cursor.x + 1}H`;
    return saved + margins + position;
  };
  const colors = [snapshot.appearance.foreground, snapshot.appearance.background, snapshot.appearance.cursor]
    .map((value, i) => `${ESC}]${10 + i};${value}${ESC}\\`).join("");
  let ansi = colors + snapshot.normal.ansi + restorePosition(snapshot.normal, snapshot.active === "normal" && snapshot.modes.originMode);
  if (snapshot.alternate)
    ansi += `${ESC}[?47h${ESC}[H` + snapshot.alternate.ansi + restorePosition(snapshot.alternate, snapshot.modes.originMode);
  return ansi + snapshot.rehydrate + `${ESC}]2;${snapshot.title}${ESC}\\` + snapshot.pendingEscape;
}
