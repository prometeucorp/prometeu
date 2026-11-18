import { LIMITS, ProtocolError } from "./protocol.ts";
import { advanceTerminalEscapeBoundary, extractPartialEscapeTail } from "./vendor/terminal-partial-escape-tail.ts";

/** Keep parsing after an oversized incomplete escape, but refuse snapshots until it closes.
 * A truncated CSI/OSC must never become an apparently valid restore point.
 */
export class EscapeTail {
  private tail = "";
  private overflowBoundary = "";
  write(text: string): void {
    if (this.overflowBoundary) {
      // Track a bounded state until the oversized sequence terminates. A later fresh ESC starts
      // a recoverable tail. Scanning a byte at a time here is bounded by the write frame limit.
      for (let i = 0; i < text.length; i++) {
        this.overflowBoundary = advanceTerminalEscapeBoundary(this.overflowBoundary, text[i]);
        if (!this.overflowBoundary) { this.write(text.slice(i + 1)); return; }
      }
      return;
    }
    this.tail = extractPartialEscapeTail(this.tail + text);
    if (this.tail.length > LIMITS.escapeChars) {
      this.overflowBoundary = advanceTerminalEscapeBoundary("", this.tail);
      this.tail = "";
    }
  }
  snapshot(): string {
    if (this.overflowBoundary) throw new ProtocolError("snapshot_unavailable");
    // C0 controls inside an incomplete CSI/ESC have already executed (for example LF).
    // Replaying them would move the cursor a second time. OSC/DCS payload stays verbatim.
    return /^\x1b[\]PX^_]/.test(this.tail) ? this.tail : this.tail.replace(/[\x00-\x1a\x1c-\x1f\x7f]/g, "");
  }
}
