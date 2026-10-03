import { beforeEach, expect, it, vi } from "vitest";

const fake = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  terminals: [] as { output: string; input: (text: string) => void }[],
}));
vi.mock("./ipc", () => ({ invoke: fake.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: fake.listen }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    output = "";
    input = (_text: string) => {};
    constructor() { fake.terminals.push(this); }
    loadAddon() {}
    open() {}
    onData(input: (text: string) => void) { this.input = input; }
    reset() { this.output = ""; }
    write(text: string) { this.output += text; }
    focus() {}
  },
}));

import * as dock from "./dock";
import { loneCompositionEnds } from "./term";
import { useTerminalSnapshots, type TerminalOutput } from "./terminal-output";
import { useConnectionRecovery } from "./connection";
import { Recovery } from "./windows/recovery";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const bytes = (text: string) => [...new TextEncoder().encode(text)];

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} });
  vi.stubGlobal("requestAnimationFrame", () => 0);
  vi.stubGlobal("cancelAnimationFrame", () => {});
  fake.invoke.mockReset();
  fake.listen.mockClear();
  useTerminalSnapshots({ read: async session => ({ data: await fake.invoke("pty_buffer", { session }) }) });
  useConnectionRecovery({ subscribe: () => () => {} });
  dock.detach();
  dock.init(new EventTarget() as HTMLElement, new EventTarget() as HTMLElement);
});

it("restores a retained terminal with ordered bytes, without duplicating concurrent output or reopening it", async () => {
  const recovery = new Recovery(async () => true, () => {});
  useConnectionRecovery(recovery);
  dock.init(new EventTarget() as HTMLElement, new EventTarget() as HTMLElement);
  const receive = fake.listen.mock.calls[fake.listen.mock.calls.length - 1][1];
  useTerminalSnapshots({ read: async () => ({ data: bytes("before"), seq: 1, running: true }) });
  fake.invoke.mockResolvedValue("first:terminal");
  await dock.open("first", "terminal");
  const snapshot = deferred<TerminalOutput>();
  useTerminalSnapshots({ read: () => snapshot.promise });
  fake.invoke.mockClear();
  const pending = recovery.recover();
  await Promise.resolve();
  await Promise.resolve();
  fake.terminals[1].input("must not send while restoring");
  receive({ payload: ["first:terminal", bytes("included"), 2] });
  receive({ payload: ["first:terminal", [0xc3], 3] });
  receive({ payload: ["first:terminal", [0xa9], 4] });
  snapshot.resolve({ data: bytes("beforeincluded"), seq: 2, running: true });
  await pending;
  receive({ payload: ["first:terminal", bytes("duplicate"), 4] });
  expect(fake.terminals[1].output).toBe("beforeincludedé");
  expect(fake.invoke).not.toHaveBeenCalled();
  fake.terminals[1].input("fresh input");
  expect(fake.invoke).toHaveBeenCalledWith("pty_write", { session: "first:terminal", data: "fresh input" });
});

it("disables input when a restored terminal exited while disconnected", async () => {
  useTerminalSnapshots({ read: async () => ({ data: bytes("finished"), seq: 8, running: false }) });
  fake.invoke.mockResolvedValue("first:terminal");
  await dock.open("first", "terminal");
  expect(fake.terminals[1].output).toBe("finished");
  expect(dock.currentKey("shell")).toBeNull();
  fake.invoke.mockClear();
  fake.terminals[1].input("ignored");
  expect(fake.invoke).not.toHaveBeenCalled();
});

it("keeps terminal input in the current workspace when an earlier open finishes late", async () => {
  const first = deferred<string>();
  fake.invoke.mockImplementation((command, args) => command === "open_dock"
    ? args.id === "first" ? first.promise : Promise.resolve(`${args.id}:${args.kind}`)
    : command === "pty_buffer" ? Promise.resolve(bytes(args.session)) : Promise.resolve());
  const old = dock.open("first", "terminal");
  dock.detach();
  await dock.open("second", "terminal");
  first.resolve("first:terminal");
  await old;

  expect(dock.currentKey("shell")).toBe("second:terminal");
  expect(fake.terminals[1].output).toBe("second:terminal");
  fake.terminals[1].input("pwd\r");
  expect(fake.invoke).toHaveBeenLastCalledWith("pty_write", { session: "second:terminal", data: "pwd\r" });
});

it("discards retained output from an earlier attachment", async () => {
  const first = deferred<number[]>();
  fake.invoke.mockImplementation((command, args) => command === "open_dock"
    ? Promise.resolve(`${args.id}:${args.kind}`)
    : args.session === "first:terminal" ? first.promise : Promise.resolve(bytes(args.session)));
  const old = dock.open("first", "terminal");
  await Promise.resolve();
  await dock.open("second", "terminal");
  first.resolve(bytes("old output"));
  await old;

  expect(fake.terminals[1].output).toBe("second:terminal");
  expect(dock.currentKey("shell")).toBe("second:terminal");
});

it("invalidates pending logs and opens when detaching or closing", async () => {
  const buffer = deferred<number[]>();
  fake.invoke.mockReturnValueOnce(buffer.promise);
  const old = dock.show("first", "run");
  dock.detach();
  buffer.resolve(bytes("old log"));
  await old;
  expect(fake.terminals[0].output).toBe("");

  const opening = deferred<string>();
  fake.invoke.mockImplementation(command => command === "open_dock" ? opening.promise : Promise.resolve());
  const pending = dock.open("first", "terminal");
  await dock.kill("first", "terminal");
  opening.resolve("first:terminal");
  await pending;
  expect(dock.currentKey("shell")).toBeNull();
  expect(fake.terminals[1].output).toBe("");
});

it("keeps script and shell attachment lifecycles independent", async () => {
  const opening = deferred<string>();
  fake.invoke.mockImplementation((command, args) => command === "open_dock"
    ? args.kind === "run" ? opening.promise : Promise.resolve(`${args.id}:${args.kind}`)
    : Promise.resolve(bytes(args.session)));
  const run = dock.open("first", "run");
  await dock.open("first", "terminal");
  opening.resolve("first:run");
  await run;
  expect(dock.currentKey("scripts")).toBe("first:run");
  expect(dock.currentKey("shell")).toBe("first:terminal");
  expect(fake.terminals.map(term => term.output)).toEqual(["first:run", "first:terminal"]);
});

it.each(["open_dock", "pty_buffer"])("detaches input when %s fails", async command => {
  const error = new Error("terminal unavailable");
  fake.invoke.mockImplementation(next => next === command ? Promise.reject(error) : Promise.resolve("first:terminal"));
  await expect(dock.open("first", "terminal")).rejects.toBe(error);
  expect(dock.currentKey("shell")).toBeNull();
  fake.invoke.mockClear();
  fake.terminals[1].input("pwd\r");
  expect(fake.invoke).not.toHaveBeenCalled();
});

it("ignores failed older opens without detaching the current shell", async () => {
  const first = deferred<string>();
  fake.invoke.mockImplementation((command, args) => command === "open_dock"
    ? args.id === "first" ? first.promise : Promise.resolve(`${args.id}:${args.kind}`)
    : Promise.resolve(bytes(args.session)));
  const old = dock.open("first", "terminal");
  await dock.open("second", "terminal");
  first.reject(new Error("old open failed"));
  await old;
  expect(dock.currentKey("shell")).toBe("second:terminal");
  expect(fake.terminals[1].output).toBe("second:terminal");
});

it("drops composition ends that no composition start opened", () => {
  const lone = loneCompositionEnds();
  expect(lone("compositionend")).toBe(true);
  expect(lone("compositionstart")).toBe(false);
  expect(lone("compositionend")).toBe(false);
  expect(lone("compositionend")).toBe(true);
});

it("stops lone composition ends at the terminal host before xterm sees them", () => {
  const host = new EventTarget();
  const listen = vi.spyOn(host, "addEventListener");
  dock.init(new EventTarget() as HTMLElement, host as HTMLElement);
  expect(listen).toHaveBeenCalledWith("compositionend", expect.any(Function), true);

  const dispatch = (type: string) => {
    const event = new Event(type);
    const stop = vi.spyOn(event, "stopPropagation");
    host.dispatchEvent(event);
    return stop;
  };
  expect(dispatch("compositionend")).toHaveBeenCalled();
  expect(dispatch("compositionstart")).not.toHaveBeenCalled();
  expect(dispatch("compositionend")).not.toHaveBeenCalled();
});
