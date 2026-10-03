/** Supporting terminal rules. Neither transport nor rendering is selected here. */
export type TerminalEvent =
  | { kind: "output"; id: string; seq: number; data: number[] }
  | { kind: "error"; id: string; error: string }
  | { kind: "closed"; id: string; code: number | null }
  | { kind: "disconnected" };
export type TerminalSnapshot = { id: string; data: number[]; seq: number; running: boolean; code: number | null };
export interface TerminalPort {
  readonly terminalSupported: boolean;
  subscribeTerminal(receive: (event: TerminalEvent) => void): () => void;
  currentTerminal(): Promise<TerminalSnapshot | null>;
  openTerminal(cols: number, rows: number): Promise<{ id: string }>;
  writeTerminal(id: string, data: number[]): Promise<void>;
  resizeTerminal(id: string, cols: number, rows: number): Promise<void>;
  snapshotTerminal(id: string): Promise<TerminalSnapshot>;
  acknowledgeTerminal(id: string, seq: number): Promise<void>;
  closeTerminal(id: string): Promise<void>;
}
export interface TerminalScreen {
  reset(data: Uint8Array, consumed: () => void): void;
  write(data: Uint8Array, consumed: () => void): void;
}
export class SupportingTerminal {
  id: string | null = null;
  running = false;
  pending = false;
  error = "";
  code: number | null = null;
  private seq = 0;
  private epoch = 0;
  private buffered: TerminalEvent[] | null = null;
  private bufferedBytes = 0;
  private overflowed = false;
  private queue = Promise.resolve();
  private queuedBytes = 0;
  private acknowledged = 0;
  private consumed = 0;
  private acknowledging = false;
  private unsubscribe: () => void;
  constructor(private port: TerminalPort, private screen: TerminalScreen, private changed: () => void) {
    this.unsubscribe = port.subscribeTerminal(event => this.receive(event));
  }
  get supported() { return this.port.terminalSupported; }
  async attach() {
    if (this.pending || this.id || !this.supported) return;
    this.pending = true; this.error = ""; this.code = null;
    this.buffered = []; this.bufferedBytes = 0; this.overflowed = false;
    const epoch = ++this.epoch;
    this.changed();
    try {
      const snapshot = await this.port.currentTerminal();
      if (epoch !== this.epoch || !snapshot) return;
      this.id = snapshot.id;
      this.acknowledged = this.consumed = 0;
      this.restore(snapshot, epoch);
    } catch (error) { if (epoch === this.epoch) this.error = String(error); }
    finally { this.buffered = null; this.pending = false; this.changed(); }
  }
  private restore(snapshot: TerminalSnapshot, epoch: number) {
    this.seq = snapshot.seq; this.code = snapshot.code; this.running = snapshot.running;
    this.screen.reset(new Uint8Array(snapshot.data), () => this.acknowledge(snapshot.seq, epoch));
    const frames = this.buffered ?? [];
    this.buffered = null;
    if (this.overflowed) { this.running = false; return; }
    for (const frame of frames) this.receive(frame);
  }
  async open(cols: number, rows: number) {
    if (this.pending || this.id || !this.supported) return;
    this.pending = true; this.error = ""; this.code = null;
    this.buffered = []; this.bufferedBytes = 0; this.overflowed = false;
    const epoch = ++this.epoch;
    this.changed();
    try {
      const opened = await this.port.openTerminal(cols, rows);
      if (epoch !== this.epoch) return;
      this.id = opened.id;
      this.acknowledged = this.consumed = 0;
      const snapshot = await this.port.snapshotTerminal(opened.id);
      if (epoch !== this.epoch) return;
      this.restore(snapshot, epoch);
    } catch (error) { this.error = String(error); this.running = false; }
    finally { this.buffered = null; this.pending = false; this.changed(); }
  }
  input(data: Uint8Array) {
    if (!this.id || !this.running || this.pending) return;
    if (this.queuedBytes + data.length > 64 * 1024) { this.error = "terminal_input_limit"; this.changed(); return; }
    const id = this.id, epoch = this.epoch;
    for (let offset = 0; offset < data.length; offset += 4096) {
      const chunk = Array.from(data.slice(offset, offset + 4096));
      this.queuedBytes += chunk.length;
      this.queue = this.queue.then(async () => {
        if (epoch !== this.epoch || !this.running) return;
        try { await this.port.writeTerminal(id, chunk); }
        catch (error) { if (epoch === this.epoch) { this.running = false; this.error = String(error); this.changed(); } }
      }).finally(() => { this.queuedBytes -= chunk.length; });
    }
  }
  async resize(cols: number, rows: number) {
    if (!this.id || !this.running || this.pending) return;
    const epoch = this.epoch;
    try { await this.port.resizeTerminal(this.id, cols, rows); }
    catch (error) { if (epoch === this.epoch) { this.error = String(error); this.changed(); } }
  }
  async close() {
    if (!this.id || this.pending) return;
    const id = this.id;
    ++this.epoch; this.consumed = this.acknowledged = 0; this.running = false; this.pending = true; this.changed();
    try { await this.queue; await this.port.closeTerminal(id); this.id = null; this.error = ""; }
    catch (error) { this.error = String(error); }
    finally { this.pending = false; this.changed(); }
  }
  disconnected() {
    ++this.epoch; this.consumed = this.acknowledged = 0; this.id = null; this.running = false; this.buffered = null;
    this.changed();
  }
  dispose() { this.unsubscribe(); this.disconnected(); }
  private acknowledge(seq: number, epoch: number) {
    if (epoch !== this.epoch || !this.id) return;
    this.consumed = Math.max(this.consumed, seq);
    if (this.acknowledging) return;
    this.acknowledging = true;
    const id = this.id;
    void (async () => {
      try {
        while (epoch === this.epoch && this.acknowledged < this.consumed) {
          const through = this.consumed;
          await this.port.acknowledgeTerminal(id, through);
          if (epoch === this.epoch) this.acknowledged = through;
        }
      } catch (error) { if (epoch === this.epoch) { this.error = String(error); this.running = false; this.changed(); } }
      finally {
        this.acknowledging = false;
        if (epoch !== this.epoch && this.id) this.acknowledge(this.consumed, this.epoch);
      }
    })();
  }
  private receive(event: TerminalEvent) {
    if (event.kind === "disconnected") { this.disconnected(); return; }
    if (this.buffered) {
      this.bufferedBytes += event.kind === "output" ? event.data.length : 1;
      if (this.bufferedBytes <= 512 * 1024) { this.buffered.push(event); return; }
      // Retain the opened handle for explicit close, but bound the bootstrap queue.
      this.overflowed = true; this.buffered.length = 0;
      this.error = "terminal_stream_gap"; this.running = false;
      this.changed(); return;
    }
    if (event.id !== this.id) return;
    if (event.kind === "error") { this.running = false; this.error = event.error; }
    if (event.kind === "closed") { this.running = false; this.code = event.code; }
    if (event.kind === "output" && event.seq > this.seq) {
      if (event.seq !== this.seq + 1) { this.running = false; this.error = "terminal_stream_gap"; }
      else { this.seq = event.seq; const epoch = this.epoch; this.screen.write(new Uint8Array(event.data), () => this.acknowledge(event.seq, epoch)); }
    }
    this.changed();
  }
}
