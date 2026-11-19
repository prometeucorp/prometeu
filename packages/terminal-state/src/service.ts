import { Emulator } from "./emulator.ts";
import { LIMITS, ProtocolError, decodeBase64, parseRequest, requireThat, type Request, type Response, type Result } from "./protocol.ts";

export class TerminalStateService {
  private sessions = new Map<string, Emulator>();
  private queue: Promise<unknown> = Promise.resolve();
  private queued = 0;
  private closed = false;

  /** Serial ordering includes create/dispose and snapshot barriers, not only parser writes. */
  request(input: unknown): Promise<Response> {
    let request: Request;
    try { request = parseRequest(input); }
    catch (error) { return Promise.resolve(this.failure(input, error)); }
    if (this.queued >= LIMITS.queuedRequests) return Promise.resolve(this.failure(input, new ProtocolError("limit")));
    this.queued++;
    const pending = this.queue.then(async (): Promise<Response> => {
      try {
        requireThat(!this.closed, "closed");
        return { v: 1, id: request.id, ok: true, result: await this.apply(request) };
      } catch (error) { return this.failure(request, error); }
      finally { this.queued--; }
    });
    this.queue = pending;
    return pending;
  }
  private failure(input: unknown, error: unknown): Response {
    const id = input && typeof input === "object" && "id" in input
      && typeof input.id === "string" && /^[A-Za-z0-9_.:-]{1,128}$/.test(input.id) ? input.id : null;
    return { v: 1, id, ok: false, error: { code: error instanceof ProtocolError ? error.code : "internal" } };
  }
  private capacity(cells: number, excluding?: Emulator): void {
    requireThat(cells <= LIMITS.cellsPerSession, "limit");
    const total = [...this.sessions.values()].reduce((sum, value) => sum + (value === excluding ? 0 : value.cells()), cells);
    requireThat(total <= LIMITS.totalCells, "limit");
  }
  private async apply(request: Request): Promise<Result> {
    const existing = this.sessions.get(request.session);
    if (request.op === "create") {
      requireThat(!existing, existing?.generation === request.generation ? "already_exists" : "stale_generation");
      requireThat(this.sessions.size < LIMITS.sessions, "limit");
      const scrollback = request.scrollback ?? 1_000;
      this.capacity(request.cols * (request.rows * 2 + scrollback));
      this.sessions.set(request.session, new Emulator(request.session, request.generation, request.cols, request.rows, scrollback));
      return { seq: 0 };
    }
    requireThat(existing, "not_found");
    requireThat(existing.generation === request.generation, "stale_generation");
    switch (request.op) {
      case "write": return existing.write(decodeBase64(request.data), request.seq);
      case "resize":
        this.capacity(existing.cells(request.cols, request.rows), existing);
        existing.resize(request.cols, request.rows, request.seq);
        return { seq: existing.seq };
      case "snapshot": return existing.snapshot();
      case "dispose":
        existing.dispose(); this.sessions.delete(request.session);
        return { seq: existing.seq };
    }
  }
  async dispose(): Promise<void> {
    await this.queue;
    this.closed = true;
    for (const terminal of this.sessions.values()) terminal.dispose();
    this.sessions.clear();
  }
}
