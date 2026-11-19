import { invoke } from "./ipc";
import { onConnectionRestored } from "./connection";
import { terminalSnapshot, type TerminalOutput } from "./terminal-output";
import { terminalInputOrigin } from "./terminal-input";
import { replayAnsi } from "../packages/terminal-state/src/replay";
import { listen } from "@tauri-apps/api/event";
import { FitAddon } from "@xterm/addon-fit";
import { Terminal, type ITerminalOptions } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";

/// An xterm view for backend PTYs, including native agent conversation terminals.
/// Measure visible hosts on the next frame so layout changes do not clip terminal rows.

export type Skin = {
  fontSize: number;
  foreground: string;
  scrollback: number;
  appearance?: Pick<ITerminalOptions, "fontFamily" | "fontWeight" | "fontWeightBold" | "lineHeight" | "theme">;
};

/// Route input to the local PTY or its remote owner.
export type Sink = (key: string, data: string | number[], generation?: string) => void | Promise<void>;

export type TermOptions = {
  snapshot?: (key: string) => Promise<TerminalOutput>;
  error?: (error: unknown) => void;
  selectionChanged?: () => void;
  exited?: () => void;
};

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
  private held: { bytes: number[]; seq: number; generation?: string }[] | null = null;
  private heldClosed: { generation?: string }[] = [];
  private generation: string | undefined;
  private sequence = -1;
  private stopRecovery = () => {};
  private acceptsInput = false;
  private repliesOwnedByHost = false;
  private origin: ReturnType<typeof terminalInputOrigin> | undefined;
  private disposed = false;
  private mountVersion = 0;
  private cleanup: (() => void)[] = [];
  private listening: Promise<unknown> = Promise.resolve();
  private resizing: Promise<unknown> = Promise.resolve();
  private writing: Promise<void> | null = null;
  private writingVersion: number | null = null;
  private defaultScrollback: number;
  private parsing = new Set<() => void>();

  constructor(skin: Skin, private options: TermOptions = {}) {
    this.defaultScrollback = skin.scrollback;
    this.term = new Terminal({
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      fontSize: skin.fontSize,
      theme: {
        background: "#141110",
        foreground: skin.foreground,
        cursor: "#d8c2b3",
        selectionBackground: "#373533",
      },
      allowProposedApi: true,
      scrollback: skin.scrollback,
      ...skin.appearance,
    });
  }

  open(host: HTMLElement, sink: Sink) {
    if (this.disposed) throw new Error("Cannot reopen a disposed terminal");
    if (this.host) this.detach();
    this.release();
    const mount = ++this.mountVersion;
    this.host = host;
    this.sink = sink;
    this.term.loadAddon(this.fit);
    this.term.open(host);
    if (this.term.textarea) this.term.textarea.dataset.track = "terminal.input";
    // Capture on the host so the check runs before xterm's listeners on its textarea.
    const lone = loneCompositionEnds();
    const composition = (event: Event) => { if (lone(event.type)) event.stopPropagation(); };
    for (const type of ["compositionstart", "compositionend"]) {
      host.addEventListener(type, composition, true);
      this.cleanup.push(() => host.removeEventListener(type, composition, true));
    }
    const input = this.term.onData((data) => {
      if (this.repliesOwnedByHost && !this.origin?.isUser(data)) return;
      this.send(data);
    });
    this.cleanup.push(() => input.dispose());
    this.origin = terminalInputOrigin(this.term);
    this.cleanup.push(() => { this.origin?.dispose(); this.origin = undefined; });
    const binary = this.term.onBinary(data => this.send(Array.from(data, char => char.charCodeAt(0))));
    this.cleanup.push(() => binary.dispose());
    const selection = this.term.onSelectionChange(() => this.options.selectionChanged?.());
    this.cleanup.push(() => selection.dispose());
    const size = new ResizeObserver(() => this.refit());
    size.observe(host);
    this.cleanup.push(() => size.disconnect());
    const retain = (unlisten: () => void) => {
      if (this.disposed || mount !== this.mountVersion) unlisten();
      else this.cleanup.push(unlisten);
    };
    this.listening = Promise.all([listen<[string, number[], number, string?]>("pty", ({ payload: [session, bytes, seq, generation] }) => {
      if (session !== this.key) return;
      if (this.held) this.held.push({ bytes, seq, generation });
      else this.write(bytes, seq, generation);
    }).then(retain), listen<[string, number | null, string?]>("pty-closed", ({ payload: [session, , generation] }) => {
      if (session !== this.key) return;
      if (this.held) { this.heldClosed.push({ generation }); return; }
      if (generation !== this.generation) return;
      this.acceptsInput = false;
      this.key = null;
      this.options.exited?.();
    }).then(retain)]);
    // Attachment also awaits registration; report an idle registration failure once.
    void this.listening.catch(error => this.options.error?.(error));
    this.stopRecovery();
    this.stopRecovery = onConnectionRestored(async () => {
      if (this.key) await this.attach(this.key, Promise.resolve(), true);
    });
  }

  /// Invalidate earlier opens before waiting for the process and its retained output.
  async attach(key: string, ready: Promise<unknown>, recovering = false) {
    if (this.disposed) return;
    this.detach();
    const version = this.attachVersion;
    this.key = key;
    this.held = [];
    try {
      await Promise.all([ready, this.listening]);
      if (version !== this.attachVersion) return;
      const snapshot = await (this.options.snapshot ?? terminalSnapshot)(key);
      if (version !== this.attachVersion) return;
      await this.restore(snapshot, version, key);
      if (version !== this.attachVersion) return;
      this.refit(true);
    } catch (error) {
      if (version !== this.attachVersion) return;
      if (!recovering) this.detach();
      throw error;
    }
  }

  /// Show retained output without attaching input to an exited process.
  async show(key: string) {
    if (this.disposed) return;
    this.detach();
    const version = this.attachVersion;
    await this.listening;
    if (version !== this.attachVersion) return;
    const snapshot = await (this.options.snapshot ?? terminalSnapshot)(key);
    if (version !== this.attachVersion) return;
    await this.restore(snapshot, version, key);
    if (version !== this.attachVersion) return;
    this.refit();
  }

  detach() {
    this.attachVersion++;
    for (const cancel of [...this.parsing]) cancel();
    this.key = null;
    this.acceptsInput = false;
    this.repliesOwnedByHost = false;
    this.held = null;
    this.heldClosed = [];
    this.generation = undefined;
    this.sequence = -1;
    this.decoder = new TextDecoder("utf-8");
    this.term.reset();
  }

  private release() {
    this.stopRecovery();
    this.stopRecovery = () => {};
    cancelAnimationFrame(this.pending);
    this.pending = 0;
    for (const stop of this.cleanup.splice(0)) stop();
  }

  dispose() {
    if (this.disposed) return;
    this.detach();
    this.disposed = true;
    this.mountVersion++;
    this.release();
    this.term.dispose();
  }

  private write(bytes: number[], seq: number, generation?: string) {
    if (generation !== this.generation) return;
    if (seq <= this.sequence) return;
    this.sequence = seq;
    this.term.write(this.decoder.decode(new Uint8Array(bytes), { stream: true }));
  }

  private send(data: string | number[], reportError = true, validate?: () => boolean) {
    if (!this.key || !this.acceptsInput) return;
    const key = this.key;
    const generation = this.generation;
    const version = this.attachVersion;
    const sink = this.sink;
    const write = async () => {
      if (this.disposed || !this.acceptsInput || version !== this.attachVersion || key !== this.key || generation !== this.generation || (validate && !validate()))
        throw 'i18n:{"code":"err.terminalAgent.notReady"}';
      await sink(key, data, generation);
    };
    const precedingVersion = this.writingVersion;
    const pending = this.writing ? this.writing.then(write, error => {
      if (precedingVersion === version) throw error;
      return write();
    }) : write();
    this.writing = pending;
    this.writingVersion = version;
    void pending.catch(error => {
      if (reportError && version === this.attachVersion) this.options.error?.(error);
    }).finally(() => {
      if (this.writing === pending) { this.writing = null; this.writingVersion = null; }
    });
    return pending;
  }

  /** Insert reviewed context into the native draft without submitting it. The caller keeps
   * its staged content until this write resolves and owns reporting a rejected paste. */
  async paste(text: string): Promise<void> {
    const normalized = text.replace(/\r\n/g, "\n");
    if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(normalized))
      throw 'i18n:{"code":"err.terminalAgent.input"}';
    if (!this.canPaste()) throw 'i18n:{"code":"err.terminalAgent.notReady"}';
    await this.send(`\x1b[200~${normalized}\x1b[201~`, false, () => this.canPaste());
  }

  canPaste() {
    return !this.disposed && !!this.key && this.acceptsInput && this.term.modes.bracketedPasteMode;
  }

  private async restore(snapshot: TerminalOutput, version: number, key: string) {
    // A write callback fences xterm's asynchronous parser. Old queued writes must finish before
    // resetting, and replayed device queries must never become input to the current process.
    await this.parsed("");
    if (this.disposed || version !== this.attachVersion) return;
    this.term.reset();
    this.repliesOwnedByHost = snapshot.queryReplies === "host";
    if (this.repliesOwnedByHost && !this.origin?.supported) throw 'i18n:{"code":"err.terminalAgent.screen"}';
    if (snapshot.screen) {
      const screen = snapshot.screen;
      if (screen.v !== 1 || screen.session !== key || screen.generation !== snapshot.generation)
        throw 'i18n:{"code":"err.terminalAgent.screen"}';
      this.term.options.scrollback = screen.scrollback;
      this.term.resize(screen.cols, screen.rows);
      this.decoder = new TextDecoder("utf-8", { ignoreBOM: true });
      await this.parsed(replayAnsi(screen));
      if (this.disposed || version !== this.attachVersion) return;
      const tail = Uint8Array.from(atob(screen.pendingUtf8), char => char.charCodeAt(0));
      if (tail.length > 3 || this.decoder.decode(tail, { stream: true })) throw 'i18n:{"code":"err.terminalAgent.screen"}';
    } else {
      this.term.options.scrollback = this.defaultScrollback;
      await this.parsed(this.decoder.decode(new Uint8Array(snapshot.data), { stream: true }));
    }
    if (this.disposed || version !== this.attachVersion) return;
    this.sequence = snapshot.seq ?? -1;
    this.generation = snapshot.generation;
    const held = this.held ?? [];
    this.held = null;
    if (snapshot.running === false || this.heldClosed.some(event => event.generation === this.generation)) this.key = null;
    this.heldClosed = [];
    this.acceptsInput = this.key !== null;
    // Legacy byte buffers cannot identify overlap; keep live bytes rather than lose newer output.
    for (const frame of held) this.write(frame.bytes, frame.seq, frame.generation);
  }

  private parsed(text: string) {
    return new Promise<void>(done => {
      const finish = () => { this.parsing.delete(finish); done(); };
      this.parsing.add(finish);
      this.term.write(text, finish);
    });
  }

  current() {
    return this.key;
  }

  processGeneration() {
    return this.generation;
  }

  focus() {
    this.term.focus();
  }

  selection() {
    return this.term.getSelection();
  }

  dims() {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  /// Force a resize after switching PTYs even when the view dimensions stay the same.
  refit(force = false) {
    if (this.disposed || !this.host) return;
    cancelAnimationFrame(this.pending);
    this.pending = requestAnimationFrame(() => {
      if (!this.host.clientHeight || !this.host.clientWidth) return;
      const before = `${this.term.cols}x${this.term.rows}`;
      this.fit.fit();
      const changed = `${this.term.cols}x${this.term.rows}` !== before;
      if (this.key && this.acceptsInput && (force || changed)) {
        const session = this.key;
        const version = this.attachVersion;
        const { cols, rows } = this.term;
        const generation = this.generation;
        this.resizing = this.resizing.catch(() => {}).then(async () => {
          if (this.disposed || version !== this.attachVersion || session !== this.key) return;
          await invoke("pty_resize", { session, cols, rows, ...(generation ? { generation } : {}) });
        });
        void this.resizing.catch(error => this.options.error?.(error));
      }
    });
  }
}
