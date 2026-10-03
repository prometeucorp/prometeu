import { Timeline } from "../timeline";
import type { AnyConversationEventV1, RequestResponse } from "../conversation";

export type Target = { distribution: string; executable: string; root: string; workdir: string; codex: string };
export type Snapshot = { generation: string | null; snapshot: { text: string; seq: number }; providerSession: string | null; running?: boolean; ready?: boolean; error?: string | null };
export type Frame = {
  v: 1; generation?: string; seq?: number; event?: AnyConversationEventV1;
  lifecycle?: "exited" | "disconnected"; error?: string | null;
};
export interface SessionPort {
  connect(target: Target, receive: (frame: Frame) => void): Promise<void>;
  start(): Promise<{ generation: string; resuming: boolean }>;
  snapshot(): Promise<Snapshot>;
  send(text: string): Promise<void>;
  respond(requestId: string, response: RequestResponse): Promise<void>;
  stop(): Promise<void>;
  disconnect(): Promise<void>;
  shutdown(): Promise<void>;
}

/** Owns stream ordering and UI availability, with no platform or IPC knowledge. */
export class Session {
  timeline = new Timeline();
  connected = false;
  running = false;
  ready = false;
  pending = false;
  switching = false;
  targetKey = "";
  error = "";
  private generation: string | null = null;
  private seq = 0;
  private buffered: Frame[] | null = null;
  private bufferedBytes = 0;
  private overflowed = false;
  private connection = 0;
  private lost = false;
  private snapshotReadiness = false;
  constructor(private port: SessionPort, private changed: () => void) {}

  get canSend() { return this.connected && this.ready && !this.pending && !this.timeline.working; }

  async connect(target: Target) {
    await this.perform(async () => {
      this.targetKey = JSON.stringify(target);
      const connection = ++this.connection;
      this.lost = false;
      this.ready = false;
      this.buffered = []; this.bufferedBytes = 0; this.overflowed = false;
      try {
        await this.port.connect(target, frame => {
          if (connection !== this.connection) return;
          this.receive(frame);
        });
        this.connected = true;
        const snapshot = await this.port.snapshot();
        if (this.lost) throw new Error(this.error || "runtime connection closed");
        if (this.overflowed) throw new Error("sequence_gap");
        this.restore(snapshot);
        const frames = this.buffered;
        this.buffered = null;
        for (const frame of frames) this.receive(frame);
      } finally { this.buffered = null; }
    });
  }
  async start() {
    await this.perform(async () => {
      this.ready = false;
      this.buffered = []; this.bufferedBytes = 0; this.overflowed = false;
      try {
        const started = await this.port.start();
        this.generation = started.generation;
        this.running = true;
        const snapshot = await this.port.snapshot();
        if (this.overflowed) throw new Error("sequence_gap");
        this.restore(snapshot);
        const frames = this.buffered;
        this.buffered = null;
        for (const frame of frames) this.receive(frame);
      } finally { this.buffered = null; }
    });
  }
  async send(text: string): Promise<boolean> {
    if (!this.canSend || !text.trim()) return false;
    return this.perform(() => this.port.send(text));
  }
  async select(change: () => Promise<void>): Promise<boolean> {
    if (this.pending || !this.connected) return false;
    this.switching = true;
    const result = await this.perform(async () => {
      this.ready = false;
      this.buffered = []; this.bufferedBytes = 0; this.overflowed = false;
      try {
        await change();
        const snapshot = await this.port.snapshot();
        if (this.lost) throw new Error(this.error || "runtime connection closed");
        if (this.overflowed) throw new Error("sequence_gap");
        this.restore(snapshot);
        const frames = this.buffered; this.buffered = null;
        for (const frame of frames) this.receive(frame);
      } finally { this.buffered = null; }
    });
    this.switching = false; this.changed();
    return result;
  }
  async respond(id: string, response: RequestResponse) {
    await this.perform(() => this.port.respond(id, response));
  }
  async stop() {
    await this.perform(async () => {
      await this.port.stop();
      this.ready = this.running = false;
      this.restore(await this.port.snapshot());
    });
  }
  async disconnect() {
    await this.perform(async () => {
      ++this.connection;
      try { await this.port.disconnect(); }
      finally { this.connected = this.running = this.ready = false; }
    });
  }
  async shutdown() {
    await this.perform(async () => {
      await this.port.shutdown();
      ++this.connection;
      try { await this.port.disconnect(); }
      finally { this.connected = this.running = this.ready = false; }
      this.error = "";
    });
  }
  private restore(snapshot: Snapshot) {
    this.running = snapshot.running ?? this.running;
    this.snapshotReadiness = snapshot.ready !== undefined;
    this.ready = snapshot.ready ?? this.ready;
    this.error = snapshot.error ?? this.error;
    this.generation = snapshot.generation;
    this.seq = snapshot.snapshot.seq;
    this.timeline = new Timeline();
    this.timeline.load(snapshot.snapshot.text);
    if (!this.running) { this.timeline.busy = false; this.timeline.tasks.clear(); }
  }
  private receive(frame: Frame) {
    if (frame.lifecycle === "disconnected") {
      this.lost = true;
      this.ready = this.running = false;
      // Keep connected until explicit cleanup releases the backend client.
      this.error = frame.error ?? "";
      this.changed();
      return;
    }
    if (this.buffered) {
      this.bufferedBytes += JSON.stringify(frame).length * 2;
      if (this.bufferedBytes > 8 * 1024 * 1024) { this.overflowed = true; this.buffered.length = 0; this.ready = false; this.error = "sequence_gap"; return; }
      this.buffered.push(frame); return;
    }
    if (frame.generation !== this.generation) return;
    if (frame.lifecycle === "exited" || frame.error) {
      this.ready = false;
      this.error = frame.error ?? "";
      // The runtime retains the stopped handle until stop acknowledges cleanup.
    }
    if (frame.event?.type === "session.identity" && (!this.snapshotReadiness || (frame.seq ?? 0) > this.seq)) this.ready = true;
    if (frame.event && frame.seq !== undefined && frame.seq > this.seq) {
      if (frame.seq !== this.seq + 1) {
        this.ready = false;
        this.error = "sequence_gap";
      } else {
        this.seq = frame.seq;
        this.timeline.push(JSON.stringify(frame.event));
      }
    }
    this.changed();
  }
  private async perform(run: () => Promise<unknown>): Promise<boolean> {
    if (this.pending) return false;
    this.pending = true;
    this.error = "";
    this.changed();
    try { await run(); return true; }
    catch (error) { this.error = error instanceof Error ? error.message : String(error); return false; }
    finally { this.pending = false; this.changed(); }
  }
}
