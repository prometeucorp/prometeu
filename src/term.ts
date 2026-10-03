import { invoke } from "./ipc";
import { onConnectionRestored } from "./connection";
import { terminalSnapshot, type TerminalOutput } from "./terminal-output";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

/// An xterm view for backend PTYs; agent conversations use chat.ts.
/// Measure visible hosts on the next frame so layout changes do not clip terminal rows.

export type Skin = {
  fontSize: number;
  foreground: string;
  scrollback: number;
};

/// Route input to the local PTY or its remote owner.
export type Sink = (key: string, data: string) => void;

const BACKGROUND = "#141110";

/// WebKitGTK reports an input method commit without preedit (fcitx dead keys) as a compositionend
/// with no compositionstart. xterm already sends that text from its keydown 229 path, and would
/// resend the textarea contents since its previous composition, so such lone ends are dropped.
export function loneCompositionEnds() {
  let composing = false;
  return (type: string) => {
    if (type === "compositionstart") {
      composing = true;
      return false;
    }
    const lone = !composing;
    composing = false;
    return lone;
  };
}

export class Term {
  private term: Terminal;
  private fit = new FitAddon();
  private decoder = new TextDecoder("utf-8");
  private host!: HTMLElement;
  private pending = 0;
  private attachVersion = 0;
  /// Ignore output from PTYs that are not attached to this view.
  private key: string | null = null;
  private sink: Sink = () => {};
  private held: { bytes: number[]; seq: number }[] | null = null;
  private sequence = -1;
  private stopRecovery = () => {};
  private acceptsInput = false;

  constructor(skin: Skin) {
    this.term = new Terminal({
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      fontSize: skin.fontSize,
      theme: {
        background: BACKGROUND,
        foreground: skin.foreground,
        cursor: "#d8c2b3",
        selectionBackground: "#373533",
      },
      allowProposedApi: true,
      scrollback: skin.scrollback,
    });
  }

  open(host: HTMLElement, sink: Sink) {
    this.host = host;
    this.sink = sink;
    this.term.loadAddon(this.fit);
    this.term.open(host);
    // Capture on the host so the check runs before xterm's listeners on its textarea.
    const lone = loneCompositionEnds();
    for (const type of ["compositionstart", "compositionend"]) {
      host.addEventListener(type, (event) => { if (lone(event.type)) event.stopPropagation(); }, true);
    }
    this.term.onData((data) => {
      if (this.key && this.acceptsInput) this.sink(this.key, data);
    });
    new ResizeObserver(() => this.refit()).observe(host);
    listen<[string, number[], number]>("pty", ({ payload: [session, bytes, seq] }) => {
      if (session !== this.key) return;
      if (this.held) this.held.push({ bytes, seq });
      else this.write(bytes, seq);
    });
    this.stopRecovery();
    this.stopRecovery = onConnectionRestored(async () => {
      if (this.key) await this.attach(this.key, Promise.resolve(), true);
    });
  }

  /// Invalidate earlier opens before waiting for the process and its retained output.
  async attach(key: string, ready: Promise<unknown>, recovering = false) {
    this.detach();
    const version = this.attachVersion;
    this.key = key;
    this.held = [];
    try {
      await ready;
      if (version !== this.attachVersion) return;
      const snapshot = await terminalSnapshot(key);
      if (version !== this.attachVersion) return;
      this.restore(snapshot);
      this.refit(true);
    } catch (error) {
      if (version !== this.attachVersion) return;
      if (!recovering) this.detach();
      throw error;
    }
  }

  /// Show retained output without attaching input to an exited process.
  async show(key: string) {
    this.detach();
    const version = this.attachVersion;
    const snapshot = await terminalSnapshot(key);
    if (version !== this.attachVersion) return;
    this.restore(snapshot);
    this.refit();
  }

  detach() {
    this.attachVersion++;
    this.key = null;
    this.acceptsInput = false;
    this.held = null;
    this.sequence = -1;
    this.decoder = new TextDecoder("utf-8");
    this.term.reset();
  }

  private write(bytes: number[], seq: number) {
    if (seq <= this.sequence) return;
    this.sequence = seq;
    this.term.write(this.decoder.decode(new Uint8Array(bytes), { stream: true }));
  }

  private restore(snapshot: TerminalOutput) {
    this.term.write(this.decoder.decode(new Uint8Array(snapshot.data), { stream: true }));
    this.sequence = snapshot.seq ?? -1;
    const held = this.held ?? [];
    this.held = null;
    // Legacy byte buffers cannot identify overlap; keep live bytes rather than lose newer output.
    for (const frame of held) this.write(frame.bytes, frame.seq);
    if (snapshot.running === false) this.key = null;
    this.acceptsInput = this.key !== null;
  }

  current() {
    return this.key;
  }

  focus() {
    this.term.focus();
  }

  dims() {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  /// Force a resize after switching PTYs even when the view dimensions stay the same.
  refit(force = false) {
    cancelAnimationFrame(this.pending);
    this.pending = requestAnimationFrame(() => {
      if (!this.host.clientHeight || !this.host.clientWidth) return;
      const before = `${this.term.cols}x${this.term.rows}`;
      this.fit.fit();
      const changed = `${this.term.cols}x${this.term.rows}` !== before;
      if (this.key && (force || changed)) {
        invoke("pty_resize", { session: this.key, cols: this.term.cols, rows: this.term.rows });
      }
    });
  }
}
