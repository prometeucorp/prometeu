import type { Ctx } from "./chat";
import { fromBack } from "./i18n";
import { invoke } from "./ipc";
import { Term } from "./term";
import { listen } from "@tauri-apps/api/event";
import { NATIVE_TERMINAL_APPEARANCE } from "../packages/terminal-state/src/appearance";

const resuming = new Map<string, Promise<void>>();

function resumeSession(session: string): Promise<void> {
  const current = resuming.get(session);
  if (current) return current;
  // Desk and workspace surfaces can open the same session together.
  const pending = invoke("agent_terminal_resume", { session }).finally(() => {
    if (resuming.get(session) === pending) resuming.delete(session);
  });
  resuming.set(session, pending);
  return pending;
}

/** Local native screen and input; Prometeu controls remain outside the provider's interface. */
export class TerminalConversationView {
  private term!: Term;
  private ctx!: Ctx;
  private key: string | null = null;
  private version = 0;
  private loading = false;
  private disposed = false;
  private status: ReturnType<Ctx["info"]>["status"] = null;
  private changed = () => {};
  private listening: Promise<unknown> = Promise.resolve();
  private unlisten = () => {};
  private restartPending = false;

  open(host: HTMLElement, ctx: Ctx, changed: () => void) {
    this.ctx = ctx;
    this.changed = changed;
    host.style.setProperty("--native-terminal-background", NATIVE_TERMINAL_APPEARANCE.background);
    this.term = new Term({
      fontSize: 14, foreground: NATIVE_TERMINAL_APPEARANCE.foreground, scrollback: 6000,
      appearance: {
        fontFamily: '"SF Mono", Menlo, Monaco, "Cascadia Mono", Consolas, "DejaVu Sans Mono", "Liberation Mono", monospace',
        fontWeight: 500, fontWeightBold: 700, lineHeight: 1,
        theme: {
          ...NATIVE_TERMINAL_APPEARANCE,
          cursorAccent: NATIVE_TERMINAL_APPEARANCE.background,
          selectionBackground: "#5a7898", selectionForeground: "#ffffff",
          black: "#1d1f21", red: "#cc6666", green: "#b5bd68", yellow: "#f0c674",
          blue: "#81a2be", magenta: "#b294bb", cyan: "#8abeb7", white: "#c5c8c6",
          brightBlack: "#666666", brightRed: "#d54e53", brightGreen: "#b9ca4a", brightYellow: "#e7c547",
          brightBlue: "#7aa6da", brightMagenta: "#c397d8", brightCyan: "#70c0b1", brightWhite: "#eaeaea",
        },
      },
    }, {
      snapshot: session => invoke("agent_terminal_snapshot", { session }),
      error: error => ctx.say(fromBack(error), true),
      selectionChanged: changed,
      exited: changed,
    });
    this.term.open(host, (session, data, generation) => {
      return invoke("pty_write", { session, generation, ...(typeof data === "string" ? { data } : { bytes: data }) });
    });
    this.listening = listen<[string, string]>("agent-terminal-started", ({ payload: [session, generation] }) => {
      if (this.disposed || session !== this.key || generation === this.term.processGeneration()) return;
      // Retuning or account handoff may replace a ready process without changing the tab status.
      // The event is only an invalidation: the new attachment obtains identity from its snapshot.
      this.restartPending = true;
      this.reconcileRestart();
    }).then(unlisten => {
      if (this.disposed) unlisten();
      else this.unlisten = unlisten;
    });
    void this.listening.catch(error => ctx.say(fromBack(error), true));
  }

  attach(key: string) { return this.attachScreen(key, true); }

  private async attachScreen(key: string, resumeStopped: boolean) {
    if (this.disposed) return;
    const version = ++this.version;
    this.key = key;
    this.restartPending = false;
    this.status = this.ctx.info().status;
    this.loading = true;
    this.changed();
    try {
      await this.listening;
      if (version !== this.version || this.disposed) return;
      await this.term.attach(key, Promise.resolve());
      if (version !== this.version || this.disposed) return;
      if (resumeStopped && !this.term.current()) await this.resumeAttachment(key, version);
    } finally {
      if (version === this.version && !this.disposed) {
        this.loading = false;
        this.changed();
        this.reconcileRestart();
      }
    }
  }

  async resume() {
    if (!this.key || this.loading || this.disposed) return;
    const key = this.key;
    const version = this.version;
    this.loading = true;
    this.changed();
    try {
      await this.resumeAttachment(key, version);
    } finally {
      if (version === this.version && !this.disposed) {
        this.loading = false;
        this.changed();
        this.reconcileRestart();
      }
    }
  }

  /** Shared or queued input may have resumed the stopped native process. */
  refresh() {
    const status = this.ctx.info().status;
    const restarted = this.status === "desligada" && status !== null && status !== "desligada";
    this.status = status;
    if (restarted && this.key && !this.loading && !this.term.current()) {
      void this.attachScreen(this.key, false).catch(error => this.ctx.say(fromBack(error), true));
    }
    this.term.refit();
  }

  detach() {
    this.version++;
    this.key = null;
    this.loading = false;
    this.status = null;
    this.restartPending = false;
    this.term?.detach();
  }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.detach();
    this.unlisten();
    this.unlisten = () => {};
    this.term?.dispose();
  }

  current() { return this.key; }
  busy() { return this.loading; }
  running() { return this.term.current() !== null; }
  focus() { this.term.focus(); }
  refit() { this.term.refit(true); }
  selection() { return this.term.selection(); }

  /** Bind delayed context insertion to this attachment and native process. */
  inputTarget(): { paste(text: string): Promise<void>; focus(): void } | null {
    const key = this.key;
    const version = this.version;
    const generation = this.term?.processGeneration();
    const current = () => !this.disposed && !this.loading && this.key === key
      && this.version === version && this.term.current() === key
      && this.term.processGeneration() === generation && this.term.canPaste();
    if (!key || !generation || !current()) return null;
    return {
      paste: async text => {
        if (!current()) throw 'i18n:{"code":"err.terminalAgent.notReady"}';
        await this.term.paste(text);
      },
      focus: () => { if (current()) this.term.focus(); },
    };
  }

  private reconcileRestart() {
    if (!this.restartPending || this.loading || !this.key || this.disposed) return;
    // Lifecycle invalidation observes the current process; only opening or retrying may launch one.
    void this.attachScreen(this.key, false).catch(error => this.ctx.say(fromBack(error), true));
  }

  private async resumeAttachment(key: string, version: number) {
    try {
      await resumeSession(key);
      if (version !== this.version || this.disposed) return;
      // A start event received during resume is covered by the fresh snapshot below.
      this.restartPending = false;
      await this.term.attach(key, Promise.resolve());
    } catch (error) {
      if (version !== this.version || this.disposed) return;
      this.restartPending = false;
      this.ctx.say(fromBack(error), true);
    }
  }
}
