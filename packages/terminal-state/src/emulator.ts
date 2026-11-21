import type { Terminal as HeadlessTerminal } from "@xterm/headless";
import { LIMITS, type Snapshot, requireThat } from "./protocol.ts";
import { EscapeTail } from "./escape.ts";
import { Utf8Stream } from "./utf8.ts";
import { Terminal, serializeTerminal, terminalColors, viewport, viewportStyles, writeTerminal } from "./xterm-state.ts";

export class Emulator {
  readonly session: string;
  readonly generation: string;
  readonly scrollback: number;
  readonly terminal: HeadlessTerminal;
  seq = 0;
  private utf8 = new Utf8Stream();
  private escape = new EscapeTail();
  private replies: string[] | null = null;
  private title = "";
  private titleTruncated = false;
  private colors: ReturnType<typeof terminalColors>;

  constructor(session: string, generation: string, cols: number, rows: number, scrollback: number) {
    this.session = session;
    this.generation = generation;
    this.scrollback = scrollback;
    this.terminal = new Terminal({ cols, rows, scrollback, allowProposedApi: true, logLevel: "off" });
    this.terminal.onData(data => { this.replies?.push(Buffer.from(data).toString("base64")); });
    this.colors = terminalColors(this.terminal, data => this.replies?.push(Buffer.from(data).toString("base64")));
    this.terminal.onTitleChange(title => {
      this.title = title.slice(0, LIMITS.titleChars);
      this.titleTruncated = title.length > LIMITS.titleChars;
    });
  }
  cells(cols = this.terminal.cols, rows = this.terminal.rows): number {
    return cols * (rows * 2 + this.scrollback);
  }
  async write(bytes: Buffer, seq: number): Promise<{ seq: number; replies: string[] }> {
    requireThat(seq === this.seq + bytes.length, "sequence");
    const text = this.utf8.write(bytes);
    this.replies = [];
    try {
      await writeTerminal(this.terminal, text);
      this.escape.write(text);
      this.seq = seq;
      return { seq, replies: this.replies };
    } finally { this.replies = null; }
  }
  resize(cols: number, rows: number, seq: number): void {
    requireThat(seq === this.seq, "sequence");
    this.terminal.resize(cols, rows);
  }
  /** A view leaves buffer ANSI and links empty: readiness polls read only the viewport, cursor,
   * modes and title, and serializing the whole history on every poll stalls PTY output. */
  snapshot(view = false): Snapshot {
    const pendingEscape = this.escape.snapshot();
    const snapshot: Snapshot = {
      v: 1, session: this.session, generation: this.generation, seq: this.seq,
      cols: this.terminal.cols, rows: this.terminal.rows, scrollback: this.scrollback,
      ...viewport(this.terminal),
      viewportStyles: viewportStyles(this.terminal),
      appearance: this.colors.snapshot(),
      ...serializeTerminal(this.terminal, view), title: this.title, titleTruncated: this.titleTruncated,
      pendingEscape, pendingUtf8: this.utf8.tail(),
    };
    requireThat(Buffer.byteLength(JSON.stringify(snapshot)) <= LIMITS.snapshotBytes, "snapshot_unavailable");
    return snapshot;
  }
  dispose(): void { this.replies = null; this.colors.dispose(); this.terminal.dispose(); }
}
