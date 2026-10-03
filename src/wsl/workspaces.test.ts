import { describe, expect, it } from "vitest";
import { MockSession } from "./mock";
import { Session, type Target } from "./session";
import { Workspaces } from "./workspaces";

const target: Target = { distribution: "Test", executable: "/runtime", root: "/root", workdir: "/project", codex: "/codex" };
async function setup(port = new MockSession()) {
  const session = new Session(port, () => {});
  const workspaces = new Workspaces(port, session, () => {});
  await session.connect(target); await workspaces.load();
  await session.start(); await session.send("First workspace");
  return { port, session, workspaces };
}
describe("workspace selection", () => {
  it("registers an isolated checkout without switching the current conversation and never retries failures", async () => {
    class Port extends MockSession {
      calls = 0;
      override async createWorktree(request: Parameters<MockSession["createWorktree"]>[0]) {
        this.calls++;
        if (this.calls > 1) throw new Error("workspace_worktree_unsaved: /preserved/checkout");
        return super.createWorktree(request);
      }
    }
    const port = new Port();
    const { session, workspaces } = await setup(port);
    const request = { title: "Isolated", path: "/project", branch: "feature/test", base: "main" };
    expect(await workspaces.createWorktree(request)).toBe(true);
    expect(workspaces.catalog!.active).toBe("primary");
    expect(session.ready).toBe(true);
    const isolated = workspaces.catalog!.board.workspaces[1];
    expect(isolated.worktree).not.toBe(isolated.repo);
    expect(isolated.branch).toBe("feature/test");
    expect(await workspaces.createWorktree(request)).toBe(false);
    expect(port.calls).toBe(2);
    expect(workspaces.catalog!.board.workspaces).toHaveLength(2);
    expect(workspaces.error).toContain("/preserved/checkout");
  });
  it("ignores catalog replies from a detached connection and scopes drafts to the runtime target", async () => {
    class Deferred extends MockSession {
      delay = false;
      release: () => void = () => {};
      override async listWorkspaces() {
        const result = await super.listWorkspaces();
        if (this.delay) { this.delay = false; await new Promise<void>(resolve => { this.release = resolve; }); }
        return result;
      }
    }
    const port = new Deferred();
    const { workspaces } = await setup(port);
    workspaces.attach("first root");
    port.delay = true;
    const stale = workspaces.load();
    await Promise.resolve();
    workspaces.detach("Private first draft");
    workspaces.attach("second root");
    await workspaces.load();
    await workspaces.create("New", "/new");
    port.release(); await stale;
    expect(workspaces.catalog!.board.workspaces).toHaveLength(2);
    expect(workspaces.draft()).toBe("");
    expect(workspaces.pending).toBe(false);
  });
  it("isolates drafts, history and generations while preserving a live session", async () => {
    const { port, session, workspaces } = await setup();
    await workspaces.create("Second", "/second");
    const second = workspaces.catalog!.board.workspaces[1].id;
    expect(await workspaces.select(second, "First draft")).toBe("");
    expect(session.ready).toBe(false);
    expect(session.timeline.items).toHaveLength(0);
    await session.start(); await session.send("Second workspace");
    expect(await workspaces.select("primary", "Second draft")).toBe("First draft");
    expect(session.ready).toBe(true);
    expect(session.timeline.items.filter(i => i.kind === "user").map(i => i.text)).toEqual(["First workspace"]);
    // A delayed event from the previously selected generation cannot contaminate this transcript.
    port.receive({ v: 1, generation: "2", seq: 50, event: { v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "Stale second event" }] } });
    expect(session.error).toBe("");
    expect(await workspaces.select(second, "Revised first draft")).toBe("Second draft");
    expect(session.timeline.items.filter(i => i.kind === "user").map(i => i.text)).toEqual(["Second workspace"]);
  });
  it("blocks sending during selection and buffers events while the new snapshot is read", async () => {
    class DuringSnapshot extends MockSession {
      inject = false;
      override async snapshot() {
        const result = await super.snapshot();
        if (this.inject) this.event({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "During selection" }] });
        return result;
      }
    }
    const port = new DuringSnapshot();
    const { session, workspaces } = await setup(port);
    await workspaces.create("Second", "/second");
    const second = workspaces.catalog!.board.workspaces[1].id;
    await workspaces.select(second, ""); await session.start();
    port.inject = true;
    const selection = workspaces.select("primary", "");
    expect(session.canSend).toBe(false);
    expect(session.switching).toBe(true);
    await selection;
    expect(session.switching).toBe(false);
    expect(session.timeline.items.filter(i => i.kind === "user").map(i => i.text)).toEqual(["First workspace", "During selection"]);
  });
  it("does not retry a failed selection or move its catalog to an unconfirmed workspace", async () => {
    class Failure extends MockSession {
      calls = 0;
      override async selectWorkspace(_id: string): Promise<never> { this.calls++; throw new Error("outcome unknown"); }
    }
    const port = new Failure();
    const { session, workspaces } = await setup(port);
    expect(await workspaces.select("second", "Keep draft")).toBeNull();
    expect(port.calls).toBe(1);
    expect(workspaces.catalog!.active).toBe("primary");
    expect(session.canSend).toBe(false);
    expect(workspaces.error).toBe("outcome unknown");
    await session.disconnect(); await session.connect(target); await workspaces.load();
    expect(session.canSend).toBe(true);
  });
});
