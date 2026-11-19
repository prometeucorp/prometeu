import { beforeEach, expect, it, vi } from "vitest";
import type { Ctx, Info } from "./chat";

const fake = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), terms: [] as any[] }));
vi.mock("./ipc", () => ({ invoke: fake.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: fake.listen }));
vi.mock("./i18n", () => ({ fromBack: String }));
vi.mock("./term", () => ({ Term: class {
  key: string | null = null;
  generation?: string;
  constructor(_skin: unknown, private options: { snapshot: (key: string) => Promise<{ running: boolean; generation?: string }> }) { fake.terms.push(this); }
  open = vi.fn();
  attach = vi.fn(async (key: string) => { const snapshot = await this.options.snapshot(key); this.key = snapshot.running ? key : null; this.generation = snapshot.generation; });
  detach = vi.fn(() => { this.key = null; });
  dispose = vi.fn();
  refit = vi.fn();
  focus = vi.fn();
  paste = vi.fn(async (_text: string) => {});
  canPaste() { return this.key !== null; }
  selection = vi.fn(() => "");
  current() { return this.key; }
  processGeneration() { return this.generation; }
} }));

import { TerminalConversationView } from "./terminal-conversation";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  fake.terms.length = 0;
  fake.listen.mockReset().mockResolvedValue(() => {});
  fake.invoke.mockReset().mockImplementation(async command => command === "agent_terminal_snapshot" ? { data: [], seq: 0, running: true, generation: "process" } : undefined);
});

function open() {
  const view = new TerminalConversationView();
  const ctx: Ctx = { say: vi.fn(), info: () => ({ status: "desligada" } as Info) };
  view.open({ style: { setProperty: vi.fn() } } as unknown as HTMLElement, ctx, vi.fn());
  return { view, ctx };
}

it("automatically resumes a stopped attachment and restores the new process without submitting input", async () => {
  const { view } = open();
  let running = false;
  fake.invoke.mockImplementation(async command => {
    if (command === "agent_terminal_snapshot") return { data: [], seq: 0, running, generation: running ? "resumed-process" : "old-process" };
    if (command === "agent_terminal_resume") {
      running = true;
      fake.listen.mock.calls[0][1]({ payload: ["agent", "resumed-process"] });
    }
  });
  await view.attach("agent");
  expect(fake.invoke.mock.calls).toEqual([
    ["agent_terminal_snapshot", { session: "agent" }],
    ["agent_terminal_resume", { session: "agent" }],
    ["agent_terminal_snapshot", { session: "agent" }],
  ]);
  expect(view.running()).toBe(true);
  expect(view.busy()).toBe(false);
  expect(fake.terms[0].processGeneration()).toBe("resumed-process");
  expect(fake.terms[0].paste).not.toHaveBeenCalled();
  view.dispose();
});

it("uses the live native snapshot instead of stale stopped board status", async () => {
  const { view } = open();
  await view.attach("agent");
  expect(fake.invoke.mock.calls).toEqual([["agent_terminal_snapshot", { session: "agent" }]]);
  expect(view.running()).toBe(true);
  view.dispose();
});

it("coalesces automatic resume when multiple surfaces open the same stopped session", async () => {
  const pending = deferred();
  let running = false;
  fake.invoke.mockImplementation(async command => {
    if (command === "agent_terminal_snapshot") return { data: [], seq: 0, running, generation: running ? "resumed-process" : undefined };
    if (command === "agent_terminal_resume") { await pending.promise; running = true; }
  });
  const first = open().view;
  const second = open().view;
  const attaches = Promise.all([first.attach("agent"), second.attach("agent")]);
  await vi.waitFor(() => expect(fake.invoke.mock.calls.filter(([command]) => command === "agent_terminal_resume")).toHaveLength(1));
  expect(first.busy()).toBe(true);
  expect(second.busy()).toBe(true);
  pending.resolve();
  await attaches;
  expect(fake.invoke.mock.calls.filter(([command]) => command === "agent_terminal_resume")).toHaveLength(1);
  expect(first.running()).toBe(true);
  expect(second.running()).toBe(true);
  first.dispose(); second.dispose();
});

it("leaves a failed automatic resume stopped for an explicit retry", async () => {
  const { view, ctx } = open();
  let attempts = 0;
  let running = false;
  fake.invoke.mockImplementation(async command => {
    if (command === "agent_terminal_snapshot") return { data: [], seq: 0, running, generation: running ? "resumed-process" : undefined };
    if (command === "agent_terminal_resume") {
      if (++attempts === 1) throw "resume failed";
      running = true;
    }
  });
  await view.attach("agent");
  expect(ctx.say).toHaveBeenCalledExactlyOnceWith("resume failed", true);
  expect(view.running()).toBe(false);
  expect(view.busy()).toBe(false);
  view.refresh();
  expect(attempts).toBe(1);
  fake.listen.mock.calls[0][1]({ payload: ["agent", "failed-process"] });
  await vi.waitFor(() => expect(view.busy()).toBe(false));
  expect(attempts).toBe(1);
  await view.resume();
  expect(attempts).toBe(2);
  expect(view.running()).toBe(true);
  expect(view.busy()).toBe(false);
  view.dispose();
});

it("does not retry automatically when the resumed process is already stopped again", async () => {
  fake.invoke.mockImplementation(async command => command === "agent_terminal_snapshot" ? { data: [], seq: 0, running: false } : undefined);
  const { view } = open();
  await view.attach("agent");
  expect(fake.invoke.mock.calls.map(([command]) => command)).toEqual([
    "agent_terminal_snapshot", "agent_terminal_resume", "agent_terminal_snapshot",
  ]);
  expect(view.running()).toBe(false);
  expect(view.busy()).toBe(false);
  view.dispose();
});

it("does not resume again when a delayed start event arrives during a stopped post-resume snapshot", async () => {
  const pending = deferred();
  let snapshots = 0;
  fake.invoke.mockImplementation(async command => {
    if (command !== "agent_terminal_snapshot") return;
    snapshots++;
    if (snapshots === 2) await pending.promise;
    return { data: [], seq: 0, running: false, generation: snapshots === 1 ? "old-process" : "resumed-process" };
  });
  const { view } = open();
  const attaching = view.attach("agent");
  await vi.waitFor(() => expect(snapshots).toBe(2));
  fake.listen.mock.calls[0][1]({ payload: ["agent", "resumed-process"] });
  pending.resolve();
  await attaching;
  await vi.waitFor(() => expect(view.busy()).toBe(false));
  expect(fake.invoke.mock.calls.map(([command]) => command)).toEqual([
    "agent_terminal_snapshot", "agent_terminal_resume", "agent_terminal_snapshot", "agent_terminal_snapshot",
  ]);
  expect(view.running()).toBe(false);
  expect(fake.terms[0].processGeneration()).toBe("resumed-process");
  view.dispose();
});

it("refreshes a restarted board status without launching a process that is already stopped again", async () => {
  const { view, ctx } = open();
  await view.attach("agent");
  fake.terms[0].key = null;
  ctx.info = () => ({ status: "pronta" } as Info);
  fake.invoke.mockClear().mockResolvedValue({ data: [], seq: 0, running: false, generation: "stopped-process" });
  view.refresh();
  await vi.waitFor(() => expect(view.busy()).toBe(false));
  expect(fake.invoke.mock.calls).toEqual([["agent_terminal_snapshot", { session: "agent" }]]);
  expect(view.running()).toBe(false);
  view.dispose();
});

it("does not resume a stale stopped snapshot after navigating to another conversation", async () => {
  const pending = deferred();
  fake.invoke.mockImplementation(async (command, { session }) => {
    if (command === "agent_terminal_snapshot" && session === "first") {
      await pending.promise;
      return { data: [], seq: 0, running: false };
    }
    return { data: [], seq: 0, running: true, generation: "second-process" };
  });
  const { view } = open();
  const first = view.attach("first");
  await vi.waitFor(() => expect(fake.invoke).toHaveBeenCalledWith("agent_terminal_snapshot", { session: "first" }));
  await view.attach("second");
  pending.resolve();
  await first;
  expect(fake.invoke.mock.calls.map(([command]) => command)).toEqual(["agent_terminal_snapshot", "agent_terminal_snapshot"]);
  expect(view.current()).toBe("second");
  expect(view.busy()).toBe(false);
  view.dispose();
});

it.each([
  ["navigation", "success"], ["disposal", "success"],
  ["navigation", "failure"], ["disposal", "failure"],
])("ignores automatic resume completion after %s with %s", async (action, outcome) => {
  const pending = deferred();
  fake.invoke.mockImplementation(async (command, { session }) => {
    if (command === "agent_terminal_resume") {
      await pending.promise;
      if (outcome === "failure") throw "resume failed";
      return;
    }
    return { data: [], seq: 0, running: session !== "first", generation: session === "first" ? undefined : "second-process" };
  });
  const { view, ctx } = open();
  const first = view.attach("first");
  await vi.waitFor(() => expect(fake.invoke).toHaveBeenCalledWith("agent_terminal_resume", { session: "first" }));
  if (action === "navigation") await view.attach("second");
  else view.dispose();
  pending.resolve();
  await first;
  expect(fake.invoke.mock.calls.filter(([command, args]) => command === "agent_terminal_snapshot" && args.session === "first")).toHaveLength(1);
  expect(view.current()).toBe(action === "navigation" ? "second" : null);
  expect(view.busy()).toBe(false);
  expect(ctx.say).not.toHaveBeenCalled();
  view.dispose();
});

it("resumes without inserting a fabricated prompt and refreshes the native snapshot", async () => {
  const { view } = open();
  await view.attach("agent");
  fake.invoke.mockClear();
  await view.resume();
  expect(fake.invoke.mock.calls).toEqual([
    ["agent_terminal_resume", { session: "agent" }],
    ["agent_terminal_snapshot", { session: "agent" }],
  ]);
  expect(view.busy()).toBe(false);
});

it("does not reattach an earlier terminal when its resume finishes after navigation", async () => {
  const pending = deferred();
  const { view } = open();
  await view.attach("first");
  fake.invoke.mockImplementation(async command => command === "agent_terminal_resume" ? pending.promise : { data: [], seq: 0, running: true });
  const resume = view.resume();
  await view.attach("second");
  pending.resolve();
  await resume;
  expect(fake.terms[0].attach.mock.calls).toEqual([["first", expect.any(Promise)], ["second", expect.any(Promise)]]);
  expect(view.current()).toBe("second");
  expect(view.busy()).toBe(false);
});

it("surfaces a failed resume and permits retry while disposal prevents another attachment", async () => {
  const { view, ctx } = open();
  await view.attach("agent");
  fake.invoke.mockRejectedValueOnce("resume failed");
  await view.resume();
  expect(ctx.say).toHaveBeenCalledWith("resume failed", true);
  expect(view.busy()).toBe(false);
  const pending = deferred();
  fake.invoke.mockReturnValueOnce(pending.promise);
  const resume = view.resume();
  view.dispose();
  pending.resolve();
  await resume;
  expect(fake.terms[0].attach).toHaveBeenCalledOnce();
  expect(fake.terms[0].dispose).toHaveBeenCalledOnce();
  expect(view.current()).toBeNull();
});

it("refreshes a replaced ready process from its snapshot rather than trusting the lifecycle event identity", async () => {
  const { view } = open();
  await view.attach("agent");
  fake.invoke.mockResolvedValue({ data: [], seq: 0, running: true, generation: "actual-current-process" });
  const receive = fake.listen.mock.calls[0][1];
  receive({ payload: ["agent", "delayed-process-event"] });
  await vi.waitFor(() => expect(fake.terms[0].attach).toHaveBeenCalledTimes(2));
  expect(fake.terms[0].processGeneration()).toBe("actual-current-process");
  expect(view.current()).toBe("agent");
  view.dispose();
  receive({ payload: ["agent", "another-process"] });
  expect(fake.terms[0].attach).toHaveBeenCalledTimes(2);
});

it("keeps context insertion bound to its tab and attachment even after returning to the same tab", async () => {
  const { view } = open();
  expect(view.inputTarget()).toBeNull();
  await view.attach("first");
  const target = view.inputTarget()!;
  await target.paste("context for first");
  target.focus();
  expect(fake.terms[0].paste).toHaveBeenCalledExactlyOnceWith("context for first");
  expect(fake.terms[0].focus).toHaveBeenCalledOnce();
  await view.attach("second");
  await expect(target.paste("late context")).rejects.toContain("notReady");
  target.focus();
  await view.attach("first");
  await expect(target.paste("still obsolete")).rejects.toContain("notReady");
  expect(fake.terms[0].paste).toHaveBeenCalledOnce();
  expect(fake.terms[0].focus).toHaveBeenCalledOnce();
  view.dispose();
});

it("does not offer context input while restoring or stopped and rejects a retired process target", async () => {
  const { view } = open();
  const loading = view.attach("agent");
  expect(view.inputTarget()).toBeNull();
  await loading;
  const target = view.inputTarget()!;
  fake.terms[0].generation = "replacement";
  await expect(target.paste("context")).rejects.toContain("notReady");
  fake.terms[0].key = null;
  expect(view.inputTarget()).toBeNull();
  view.dispose();
  expect(view.inputTarget()).toBeNull();
  await expect(target.paste("context")).rejects.toContain("notReady");
  expect(fake.terms[0].paste).not.toHaveBeenCalled();
});

it("leaves a rejected context paste to its caller without a second error toast or resume", async () => {
  const { view, ctx } = open();
  await view.attach("agent");
  const failure = new Error("write failed");
  fake.terms[0].paste.mockRejectedValueOnce(failure);
  fake.invoke.mockClear();
  await expect(view.inputTarget()!.paste("staged context")).rejects.toBe(failure);
  expect(ctx.say).not.toHaveBeenCalled();
  expect(fake.invoke).not.toHaveBeenCalled();
  view.dispose();
});
