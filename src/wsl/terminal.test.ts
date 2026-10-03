import { expect, it } from "vitest";
import { MockSession } from "./mock";
import { SupportingTerminal } from "./terminal";

function fixture(port = new MockSession()) {
  const output: number[] = [];
  const terminal = new SupportingTerminal(port, { reset: (data, consumed) => { output.length = 0; output.push(...data); consumed(); }, write: (data, consumed) => { output.push(...data); consumed(); } }, () => {});
  return { port, terminal, output };
}
it("reconciles output before open replies with the snapshot, preserving byte fragments", async () => {
  const { port, terminal, output } = fixture();
  await terminal.open(80, 24);
  expect(new TextDecoder().decode(new Uint8Array(output))).toBe("Preview shell\r\n$ ");
  await port.writeTerminal(terminal.id!, [195]);
  await port.writeTerminal(terminal.id!, [169, 255]);
  expect(output.slice(-3)).toEqual([195, 169, 255]);
  port.terminalEvent({ kind: "output", id: terminal.id!, seq: 3, data: [42] });
  expect(output[output.length - 1]).toBe(255);
});
it("keeps terminal input ordered and bounded without replay after a rejected write", async () => {
  class Rejected extends MockSession {
    calls: number[][] = [];
    override async writeTerminal(id: string, data: number[]) {
      if (data.length < 100) return super.writeTerminal(id, data);
      this.calls.push(data); throw new Error("unknown outcome");
    }
  }
  const port = new Rejected();
  const { terminal } = fixture(port);
  await terminal.open(80, 24);
  terminal.input(new Uint8Array(9000));
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(port.calls.map(data => data.length)).toEqual([4096]);
  expect(terminal.running).toBe(false);
  expect(terminal.error).toContain("unknown outcome");
});
it("drops stale terminals, rejects sequence gaps and keeps closure separate from conversation", async () => {
  const { port, terminal, output } = fixture();
  await terminal.open(80, 24);
  const old = terminal.id!;
  await terminal.close();
  await terminal.open(80, 24);
  const length = output.length;
  port.terminalEvent({ kind: "output", id: old, seq: 2, data: [42] });
  expect(output.length).toBe(length);
  port.terminalEvent({ kind: "output", id: terminal.id!, seq: 4, data: [42] });
  expect(terminal.running).toBe(false);
  expect(terminal.error).toBe("terminal_stream_gap");
});
it("does not carry queued keystrokes into a replacement terminal", async () => {
  const { terminal, output } = fixture();
  await terminal.open(80, 24);
  terminal.input(new TextEncoder().encode("discard"));
  await terminal.close();
  await terminal.open(80, 24);
  expect(new TextDecoder().decode(new Uint8Array(output))).not.toContain("discard");
});
it("ignores a late write failure from a disconnected terminal", async () => {
  let rejectWrite!: (error: Error) => void;
  class Delayed extends MockSession {
    override async writeTerminal(id: string, data: number[]) {
      if (data.length !== 1) return super.writeTerminal(id, data);
      await new Promise<void>((_, reject) => { rejectWrite = reject; });
    }
  }
  const { port, terminal } = fixture(new Delayed());
  await terminal.open(80, 24);
  terminal.input(new Uint8Array([42]));
  await Promise.resolve();
  port.terminalEvent({ kind: "disconnected" });
  await terminal.open(80, 24);
  rejectWrite(new Error("old connection closed"));
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(terminal.running).toBe(true);
  expect(terminal.error).toBe("");
});
it("keeps rendering output after an asynchronous input failure and rejects more input", async () => {
  const { port, terminal, output } = fixture();
  await terminal.open(80, 24);
  port.terminalEvent({ kind: "error", id: terminal.id!, error: "input failed" });
  terminal.input(new Uint8Array([42]));
  await port.writeTerminal(terminal.id!, [43]);
  expect(terminal.running).toBe(false);
  expect(terminal.error).toBe("input failed");
  expect(output.slice(-1)).toEqual([43]);
  expect(output).not.toContain(42);
  await terminal.close();
  expect(terminal.id).toBeNull();
});
it("rejects an oversized paste entirely and invalidates attachment on disconnect", async () => {
  const { port, terminal } = fixture();
  await terminal.open(80, 24);
  terminal.input(new Uint8Array(64 * 1024 + 1));
  expect(terminal.error).toBe("terminal_input_limit");
  port.terminalEvent({ kind: "disconnected" });
  expect(terminal.id).toBeNull();
  expect(terminal.running).toBe(false);
});
it("retains a handle for cleanup when initial output exceeds the replay buffer", async () => {
  class Flood extends MockSession {
    override async openTerminal(cols: number, rows: number) {
      const opened = await super.openTerminal(cols, rows);
      this.terminalEvent({ kind: "output", id: opened.id, seq: 2, data: new Array(512 * 1024).fill(42) });
      return opened;
    }
  }
  const { terminal } = fixture(new Flood());
  await terminal.open(80, 24);
  expect(terminal.id).not.toBeNull();
  expect(terminal.running).toBe(false);
  expect(terminal.error).toBe("terminal_stream_gap");
  await terminal.close();
  expect(terminal.id).toBeNull();
});
it("a disconnect during open cannot attach its late reply", async () => {
  class Detached extends MockSession {
    override async openTerminal(cols: number, rows: number) {
      const opened = await super.openTerminal(cols, rows);
      this.terminalEvent({ kind: "disconnected" });
      return opened;
    }
  }
  const { terminal } = fixture(new Detached());
  await terminal.open(80, 24);
  expect(terminal.id).toBeNull();
  expect(terminal.running).toBe(false);
});
it("acknowledges bytes only after the renderer consumes them", async () => {
  class CreditPort extends MockSession {
    acknowledgements: number[] = [];
    override async acknowledgeTerminal(_id: string, seq: number) { this.acknowledgements.push(seq); }
  }
  const port = new CreditPort();
  const callbacks: (() => void)[] = [];
  const terminal = new SupportingTerminal(port, { reset: (_, done) => callbacks.push(done), write: (_, done) => callbacks.push(done) }, () => {});
  await terminal.open(80, 24);
  expect(port.acknowledgements).toEqual([]);
  callbacks.shift()!();
  await Promise.resolve();
  expect(port.acknowledgements).toEqual([1]);
  await port.writeTerminal(terminal.id!, [42]);
  expect(port.acknowledgements).toEqual([1]);
  callbacks.shift()!();
  await Promise.resolve();
  expect(port.acknowledgements).toEqual([1, 2]);
});
it("reattaches the same terminal and deduplicates output covered by discovery", async () => {
  class Resident extends MockSession {
    override async currentTerminal() {
      await this.writeTerminal("1", [195, 169]);
      return super.currentTerminal();
    }
  }
  const { port, terminal, output } = fixture(new Resident());
  await terminal.open(80, 24);
  const id = terminal.id;
  terminal.disconnected();
  await terminal.attach();
  expect(terminal.id).toBe(id);
  expect(terminal.running).toBe(true);
  expect(output.slice(-2)).toEqual([195, 169]);
  expect(output.filter(byte => byte === 195)).toHaveLength(1);
  await port.closeTerminal(id!);
});
