import { describe, expect, it } from "vitest";
import { Session, type Target } from "./session";
import { MockSession } from "./mock";

const target: Target = { distribution: "Ubuntu", executable: "/runtime", root: "/root", workdir: "/project", codex: "/codex" };
async function fixture(port = new MockSession()) {
  const session = new Session(port, () => {});
  await session.connect(target);
  await session.start();
  return { port, session };
}
describe("WSL conversation composition", () => {
  it("handles identity before the start reply and replaces snapshot history without duplicates", async () => {
    const { session } = await fixture();
    expect(session.canSend).toBe(true);
    expect(await session.send("one message")).toBe(true);
    expect(session.timeline.items.filter(item => item.kind === "user")).toHaveLength(1);
    await session.stop();
    expect(session.canSend).toBe(false);
    await session.start();
    expect(session.canSend).toBe(true);
    expect(session.timeline.items.filter(item => item.kind === "user")).toHaveLength(1);
  });
  it("rejects old generations and duplicate frames; a gap disables sending", async () => {
    const { session, port } = await fixture();
    const event = { v: 1 as const, type: "user.message" as const, at: 1, content: [{ kind: "text" as const, text: "hello" }] };
    port.receive({ v: 1, generation: "stale", seq: 2, event });
    expect(session.timeline.items).toHaveLength(0);
    port.receive({ v: 1, generation: "1", seq: 2, event });
    port.receive({ v: 1, generation: "1", seq: 2, event });
    expect(session.timeline.items).toHaveLength(1);
    port.receive({ v: 1, generation: "1", seq: 4, event });
    expect(session.error).toBe("sequence_gap");
    expect(session.canSend).toBe(false);
  });
  it("preserves a rejected draft outcome and never retries a mutation", async () => {
    class LostReply extends MockSession {
      calls = 0;
      override async send(_text: string) { this.calls++; throw new Error("outcome unknown"); }
    }
    const port = new LostReply();
    const { session } = await fixture(port);
    expect(await session.send("keep draft")).toBe(false);
    expect(port.calls).toBe(1);
    expect(session.error).toContain("outcome unknown");
  });
  it("ignores a detached connection and recovers via a new connection", async () => {
    const { session, port } = await fixture();
    const oldReceive = port.receive;
    await session.disconnect();
    await session.connect(target);
    oldReceive({ v: 1, lifecycle: "disconnected", error: "old connection" });
    expect(session.connected).toBe(true);
    expect(session.error).toBe("");
    expect(port.generation).toBe(1);
    expect(session.canSend).toBe(true);
  });
  it("disables sends on transport loss until cleanup and reconnect", async () => {
    const { session, port } = await fixture();
    port.receive({ v: 1, lifecycle: "disconnected", error: "lost" });
    expect(session.canSend).toBe(false);
    expect(session.connected).toBe(true);
    await session.disconnect();
    expect(session.connected).toBe(false);
  });
});
it("reattaches the running generation and reconciles live output during its snapshot", async () => {
  class Resident extends MockSession {
    inject = false;
    override async snapshot() {
      const snapshot = await super.snapshot();
      if (this.inject) this.event({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "during attach" }] });
      return snapshot;
    }
  }
  const port = new Resident();
  const { session } = await fixture(port);
  await session.send("before detach");
  await session.disconnect();
  port.inject = true;
  await session.connect(target);
  expect(port.generation).toBe(1);
  expect(session.running).toBe(true);
  expect(session.ready).toBe(true);
  expect(session.timeline.working).toBe(true);
  expect(session.canSend).toBe(false);
  expect(session.timeline.items.filter(item => item.kind === "user").map(item => item.text)).toEqual(["before detach", "during attach"]);
});
it("does not revive an exited process from a covered identity event during attachment", async () => {
  class Exited extends MockSession {
    override async snapshot() {
      const snapshot = await super.snapshot();
      this.receive({ v: 1, generation: snapshot.generation!, seq: 1, event: { v: 1, type: "session.identity", at: 1, providerSession: "preview" } });
      return { ...snapshot, ready: false };
    }
  }
  const port = new Exited(); await port.start();
  const session = new Session(port, () => {});
  await session.connect(target);
  expect(session.running).toBe(true);
  expect(session.canSend).toBe(false);
});
it("keeps a transport loss during attachment from being overwritten by a late snapshot", async () => {
  class Lost extends MockSession {
    override async snapshot() {
      const snapshot = await super.snapshot();
      this.receive({ v: 1, lifecycle: "disconnected", error: "connection lost" });
      return snapshot;
    }
  }
  const port = new Lost(); await port.start();
  const session = new Session(port, () => {});
  await session.connect(target);
  expect(session.canSend).toBe(false);
  expect(session.error).toContain("connection lost");
});
it("shuts down execution separately from detaching the client", async () => {
  class Closing extends MockSession {
    override async shutdown() {
      await super.shutdown();
      this.receive({ v: 1, lifecycle: "disconnected", error: "runtime connection closed" });
    }
  }
  const { session, port } = await fixture(new Closing());
  await session.disconnect(); expect(port.running).toBe(true);
  await session.connect(target); await session.shutdown();
  expect(session.connected).toBe(false); expect(port.running).toBe(false);
  expect(session.error).toBe("");
});
it("bounds conversation attachment buffering and requires a new snapshot after overflow", async () => {
  class Flood extends MockSession {
    override async snapshot() {
      const snapshot = await super.snapshot();
      this.event({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "x".repeat(4 * 1024 * 1024) }] });
      return snapshot;
    }
  }
  const port = new Flood(); await port.start();
  const session = new Session(port, () => {});
  await session.connect(target);
  expect(session.canSend).toBe(false);
  expect(session.error).toBe("sequence_gap");
});
