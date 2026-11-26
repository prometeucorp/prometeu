import { beforeEach, expect, it, vi } from "vitest";
import type { ITerminalOptions } from "@xterm/xterm";

const fake = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  terminals: [] as {
    output: string; input: (text: string) => void; binary: (text: string) => void; disposed: boolean;
    cols: number; rows: number; options: ITerminalOptions; modes: { bracketedPasteMode: boolean };
    _core: { coreService: { triggerDataEvent(data: string, user?: boolean): void } };
  }[],
  deferWrites: false,
  writes: [] as (() => void)[],
}));
vi.mock("./ipc", () => ({ invoke: fake.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: fake.listen }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: class { fit() {} } }));
vi.mock("@xterm/addon-webgl", () => ({ WebglAddon: class { onContextLoss() {} dispose() {} } }));
vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    cols = 80;
    rows = 24;
    output = "";
    disposed = false;
    options: ITerminalOptions;
    modes = { bracketedPasteMode: false };
    input = (_text: string) => {};
    binary = (_text: string) => {};
    _core = { coreService: { triggerDataEvent: (data: string, _user = false) => this.input(data) } };
    constructor(options: ITerminalOptions) { this.options = options; fake.terminals.push(this); }
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    open() {}
    onData(input: (text: string) => void) { this.input = input; return { dispose: () => { this.input = () => {}; } }; }
    onBinary(input: (text: string) => void) { this.binary = input; return { dispose: () => { this.binary = () => {}; } }; }
    onSelectionChange() { return { dispose() {} }; }
    reset() { this.output = ""; }
    resize(cols: number, rows: number) { this.cols = cols; this.rows = rows; }
    write(text: string, done?: () => void) {
      const write = () => {
        if (this.disposed) return;
        this.output += text;
        if (text.includes("\x1b[6n")) this._core.coreService.triggerDataEvent("\x1b[1;1R");
        done?.();
      };
      if (fake.deferWrites) fake.writes.push(write);
      else write();
    }
    focus() {}
    getSelection() { return ""; }
    dispose() { this.disposed = true; }
  },
}));

import * as dock from "./dock";
import { loneCompositionEnds, Term, wordJump } from "./term";
import type { TerminalOutput } from "./terminal-output";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const bytes = (text: string) => [...new TextEncoder().encode(text)];

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  vi.stubGlobal("requestAnimationFrame", () => 0);
  vi.stubGlobal("cancelAnimationFrame", () => {});
  fake.invoke.mockReset();
  fake.listen.mockReset().mockResolvedValue(() => {});
  fake.deferWrites = false;
  fake.writes.length = 0;
  dock.detach();
  dock.init(new EventTarget() as HTMLElement, new EventTarget() as HTMLElement);
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

const skin = { fontSize: 12, foreground: "#fff", scrollback: 100 };
const receiver = (event: string) => fake.listen.mock.calls.filter(([name]) => name === event).slice(-1)[0][1];

it("opens terminal hyperlinks through the system browser and reports failures", async () => {
  const error = vi.fn();
  const term = new Term(skin, { error });
  const host = new EventTarget() as HTMLElement;
  term.open(host, vi.fn());
  const handler = fake.terminals[fake.terminals.length - 1].options.linkHandler;
  expect(handler).toBeTruthy();
  expect(handler!.allowNonHttpProtocols).not.toBe(true);
  const event = { preventDefault: vi.fn() } as unknown as MouseEvent;
  const range = { start: { x: 1, y: 1 }, end: { x: 5, y: 1 } };
  const url = "https://example.com/review?q=1#changes";
  fake.invoke.mockResolvedValue(undefined);
  handler!.hover!(event, url, range);
  expect(host.title).toBe(url);
  handler!.activate(event, url, range);
  expect(event.preventDefault).toHaveBeenCalledOnce();
  expect(fake.invoke).toHaveBeenCalledExactlyOnceWith("open_external", { url });
  handler!.leave!(event, url, range);
  expect(host.title).toBe("");

  fake.invoke.mockClear();
  for (const rejected of ["javascript:alert(1)", "file:///tmp/report.html", "vscode://file/tmp/report", "#section"]) {
    handler!.activate(event, rejected, range);
  }
  expect(fake.invoke).not.toHaveBeenCalled();
  const failure = new Error("browser unavailable");
  fake.invoke.mockRejectedValueOnce(failure);
  handler!.activate(event, "http://localhost:3000", range);
  await vi.waitFor(() => expect(error).toHaveBeenCalledExactlyOnceWith(failure));
  handler!.hover!(event, url, range);
  term.dispose();
  expect(host.title).toBe("");
});

it("fences asynchronous snapshot parsing so replayed terminal queries never reach the child", async () => {
  fake.deferWrites = true;
  const term = new Term(skin, { snapshot: async () => ({ data: bytes("snapshot\x1b[6n"), seq: 1, running: true, generation: "new" }) });
  const sink = vi.fn();
  term.open(new EventTarget() as HTMLElement, sink);
  const attached = term.attach("agent", Promise.resolve());
  await vi.waitFor(() => expect(fake.writes).toHaveLength(1));
  fake.writes.shift()!();
  await vi.waitFor(() => expect(fake.writes).toHaveLength(1));
  receiver("pty")({ payload: ["agent", bytes("live\x1b[6n"), 2, "new"] });
  fake.writes.shift()!();
  expect(sink).not.toHaveBeenCalled();
  await attached;
  fake.writes.shift()!();
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "\x1b[1;1R", "new");
  term.dispose();
});

it("rejects output and closure from an old process even when its sequence is higher", async () => {
  const snapshot = deferred<TerminalOutput>();
  const exited = vi.fn();
  const term = new Term(skin, { snapshot: () => snapshot.promise, exited });
  const sink = vi.fn();
  term.open(new EventTarget() as HTMLElement, sink);
  const attached = term.attach("agent", Promise.resolve());
  receiver("pty")({ payload: ["agent", bytes("old held"), 90, "old"] });
  receiver("pty-closed")({ payload: ["agent", 0, "old"] });
  receiver("pty")({ payload: ["agent", bytes("current held"), 2, "new"] });
  snapshot.resolve({ data: bytes("snapshot"), seq: 1, running: true, generation: "new" });
  await attached;
  const native = fake.terminals[fake.terminals.length - 1];
  receiver("pty")({ payload: ["agent", bytes("old delayed"), 91, "old"] });
  receiver("pty-closed")({ payload: ["agent", 0, "old"] });
  expect(native.output).toBe("snapshotcurrent held");
  expect(exited).not.toHaveBeenCalled();
  expect(term.current()).toBe("agent");
  native.input("current input");
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "current input", "new");
  receiver("pty-closed")({ payload: ["agent", null, "new"] });
  native.input("after exit");
  expect(sink).toHaveBeenCalledTimes(1);
  expect(term.current()).toBeNull();
  expect(exited).toHaveBeenCalledOnce();
  term.dispose();
});

it("disposes pending registration, resize observation and input without leaving attachment promises waiting", async () => {
  const registration = deferred<() => void>();
  fake.listen.mockReturnValue(registration.promise);
  const unlisten = vi.fn();
  const disconnect = vi.fn();
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect = disconnect; });
  const term = new Term(skin, { snapshot: async () => ({ data: [], running: true }) });
  const host = new EventTarget();
  const remove = vi.spyOn(host, "removeEventListener");
  const sink = vi.fn();
  term.open(host as HTMLElement, sink);
  const attached = term.attach("agent", Promise.resolve());
  term.dispose();
  registration.resolve(unlisten);
  await attached;
  expect(unlisten).toHaveBeenCalledTimes(2);
  expect(disconnect).toHaveBeenCalledOnce();
  expect(remove).toHaveBeenCalledWith("compositionend", expect.any(Function), true);
  const native = fake.terminals[fake.terminals.length - 1];
  expect(native.disposed).toBe(true);
  native.input("ignored");
  expect(sink).not.toHaveBeenCalled();
});

it("cancels an attachment while xterm has queued an unparsed snapshot", async () => {
  fake.deferWrites = true;
  const term = new Term(skin, { snapshot: async () => ({ data: bytes("pending"), running: true }) });
  term.open(new EventTarget() as HTMLElement, vi.fn());
  const attached = term.attach("agent", Promise.resolve());
  await vi.waitFor(() => expect(fake.writes).toHaveLength(1));
  term.dispose();
  await attached;
  expect(term.current()).toBeNull();
});

it("serializes resizes and drops queued dimensions after the attachment changes", async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal("requestAnimationFrame", (frame: FrameRequestCallback) => { frames.push(frame); return frames.length; });
  const blocked = deferred<void>();
  fake.invoke.mockImplementation(command => command === "pty_resize" ? blocked.promise : Promise.resolve());
  const snapshot = deferred<TerminalOutput>();
  const term = new Term(skin, { snapshot: () => snapshot.promise });
  const host = Object.assign(new EventTarget(), { clientWidth: 800, clientHeight: 600 });
  term.open(host as unknown as HTMLElement, vi.fn());
  const attached = term.attach("agent", Promise.resolve());
  term.refit(true);
  frames.shift()!(0);
  await Promise.resolve();
  expect(fake.invoke).not.toHaveBeenCalled();
  snapshot.resolve({ data: [], seq: 0, running: true, generation: "first-process" });
  await attached;
  frames.shift()!(0);
  await vi.waitFor(() => expect(fake.invoke).toHaveBeenCalledOnce());
  term.refit(true);
  frames.shift()!(0);
  term.detach();
  blocked.resolve();
  await Promise.resolve();
  await Promise.resolve();
  expect(fake.invoke).toHaveBeenCalledExactlyOnceWith("pty_resize", { session: "agent", cols: 80, rows: 24, generation: "first-process" });
  term.dispose();
});

function screenSnapshot(): NonNullable<TerminalOutput["screen"]> {
  return {
    v: 1, session: "agent", generation: "current", seq: 500, cols: 40, rows: 10, scrollback: 80,
    viewportLines: Array(10).fill(""), viewportCharacterColumns: Array.from({ length: 10 }, () => []),
    viewportStyles: Array.from({ length: 10 }, () => [{ start: 0, end: 40,
      dim: false, inverse: false, foreground: { mode: "default", value: -1 }, background: { mode: "default", value: -1 } }]),
    appearance: { foreground: "#d8c2b3", background: "#141110", cursor: "#d8c2b3" },
    active: "normal", normal: { ansi: "faithful screen", cursor: { x: 0, y: 0, wrapPending: false },
      scrollTop: 0, scrollBottom: 9, savedCursor: { x: 0, y: 0, origin: false, attributes: "" }, wrapCell: null },
    alternate: null, modes: { applicationCursorKeysMode: false, applicationKeypadMode: false,
      bracketedPasteMode: false, insertMode: false, originMode: false, reverseWraparoundMode: false,
      sendFocusMode: false, wraparoundMode: true, mouseTrackingMode: "none", mouseEncoding: "DEFAULT",
      cursorHidden: false, cursorStyle: "block", cursorBlink: false },
    rehydrate: "", title: "", titleTruncated: false, links: [], pendingEscape: "", pendingUtf8: "ww==",
  };
}

it("restores screen dimensions and pending UTF-8 while deduplicating by outer chunk sequence", async () => {
  const snapshot = deferred<TerminalOutput>();
  const term = new Term(skin, { snapshot: () => snapshot.promise });
  term.open(new EventTarget() as HTMLElement, vi.fn());
  const attached = term.attach("agent", Promise.resolve());
  receiver("pty")({ payload: ["agent", bytes("already included"), 2, "current"] });
  receiver("pty")({ payload: ["agent", [0xa9], 3, "current"] });
  snapshot.resolve({ data: bytes("discard raw tail"), seq: 2, generation: "current", running: true,
    queryReplies: "host", screen: screenSnapshot() });
  await attached;
  const native = fake.terminals[fake.terminals.length - 1];
  expect(term.dims()).toEqual({ cols: 40, rows: 10 });
  expect(native.options.scrollback).toBe(80);
  expect(native.output).toContain("faithful screen");
  expect(native.output.endsWith("é")).toBe(true);
  expect(native.output).not.toContain("discard raw tail");
  expect(native.output).not.toContain("already included");
  term.dispose();
});

it("suppresses automatic replies while preserving ordered keyboard, paste, focus and binary input during parsing", async () => {
  const first = deferred<void>();
  const sink = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation: "current", running: true, queryReplies: "host" }) });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  const native = fake.terminals[fake.terminals.length - 1];
  fake.deferWrites = true;
  receiver("pty")({ payload: ["agent", bytes("output\x1b[6n"), 1, "current"] });
  native._core.coreService.triggerDataEvent("typed", true);
  native._core.coreService.triggerDataEvent("\x1b[200~pasted\x1b[201~", true);
  native._core.coreService.triggerDataEvent("\x1b[I");
  native._core.coreService.triggerDataEvent("\x1b[O");
  native.binary("\x1b[M\x80\xff");
  fake.writes.shift()!();
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "typed", "current");
  first.resolve();
  await vi.waitFor(() => expect(sink).toHaveBeenCalledTimes(5));
  expect(sink.mock.calls.map(call => call[1])).toEqual(["typed", "\x1b[200~pasted\x1b[201~", "\x1b[I", "\x1b[O", [27, 91, 77, 128, 255]]);
  term.dispose();
});

it.each([false, true])("drops obsolete queued input and allows a new attachment after the old write rejects=%s", async rejects => {
  const first = deferred<void>();
  const error = vi.fn();
  const sink = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
  let generation = "old";
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation, running: true }), error });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  const native = fake.terminals[fake.terminals.length - 1];
  native.input("in flight");
  native.input("obsolete queued");
  generation = "new";
  await term.attach("agent", Promise.resolve());
  native.input("fresh queued");
  if (rejects) first.reject(new Error("old failure"));
  else first.resolve();
  await vi.waitFor(() => expect(sink).toHaveBeenCalledTimes(2));
  expect(sink.mock.calls).toEqual([["agent", "in flight", "old"], ["agent", "fresh queued", "new"]]);
  expect(error).not.toHaveBeenCalled();
  term.dispose();
});

it("rejects the queued input suffix after a failed write without replaying uncertain bytes", async () => {
  const first = deferred<void>();
  const failure = new Error("transport failed");
  const error = vi.fn();
  const sink = vi.fn().mockReturnValueOnce(first.promise).mockResolvedValue(undefined);
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation: "current", running: true }), error });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  const native = fake.terminals[fake.terminals.length - 1];
  native.input("prefix");
  native.input("queued text");
  native.binary("\xff");
  first.reject(failure);
  await vi.waitFor(() => expect(error).toHaveBeenCalledTimes(3));
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "prefix", "current");
  expect(error.mock.calls.every(([value]) => value === failure)).toBe(true);
  term.dispose();
});

it("pastes multiline context after earlier input and resolves only after its framed write without Enter", async () => {
  const prefix = deferred<void>(), paste = deferred<void>();
  const sink = vi.fn().mockReturnValueOnce(prefix.promise).mockReturnValueOnce(paste.promise);
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation: "current", running: true }) });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  const native = fake.terminals[fake.terminals.length - 1];
  native.modes.bracketedPasteMode = true;
  native.input("existing draft ");
  const settled = vi.fn();
  const pending = term.paste("@file.txt\r\nContext\twith emoji 🧪").then(settled);
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "existing draft ", "current");
  prefix.resolve();
  await vi.waitFor(() => expect(sink).toHaveBeenCalledTimes(2));
  expect(sink).toHaveBeenLastCalledWith("agent", "\x1b[200~@file.txt\nContext\twith emoji 🧪\x1b[201~", "current");
  expect(settled).not.toHaveBeenCalled();
  paste.resolve();
  await pending;
  expect(settled).toHaveBeenCalledOnce();
  expect(sink).toHaveBeenCalledTimes(2);
  term.dispose();
});

it("rejects paste control characters and a terminal without bracketed paste before writing", async () => {
  const sink = vi.fn(), error = vi.fn();
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation: "current", running: true }), error });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  await expect(term.paste("first\nsecond")).rejects.toContain("notReady");
  const native = fake.terminals[fake.terminals.length - 1];
  native.modes.bracketedPasteMode = true;
  for (const text of ["\0", "\x07", "\x1b[201~\r", "\r", "\x7f", "\x85", "\x9b"]) {
    await expect(term.paste(text)).rejects.toContain("err.terminalAgent.input");
  }
  expect(sink).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
  term.dispose();
});

it("rejects context paste while restoring, after exit and after disposal", async () => {
  const snapshot = deferred<TerminalOutput>();
  const sink = vi.fn();
  const term = new Term(skin, { snapshot: () => snapshot.promise });
  term.open(new EventTarget() as HTMLElement, sink);
  const loading = term.attach("agent", Promise.resolve());
  fake.terminals[fake.terminals.length - 1].modes.bracketedPasteMode = true;
  await expect(term.paste("draft")).rejects.toContain("notReady");
  snapshot.resolve({ data: [], seq: 0, generation: "current", running: true });
  await loading;
  receiver("pty-closed")({ payload: ["agent", 0, "current"] });
  await expect(term.paste("draft")).rejects.toContain("notReady");
  term.dispose();
  await expect(term.paste("draft")).rejects.toContain("notReady");
  expect(sink).not.toHaveBeenCalled();
});

it.each(["replacement", "mode", "failure"])("rejects a queued paste after %s without redirecting or reporting it twice", async change => {
  const prefix = deferred<void>();
  const failure = new Error("transport failed");
  const sink = vi.fn().mockReturnValueOnce(prefix.promise).mockResolvedValue(undefined);
  const error = vi.fn();
  let generation = "first";
  const term = new Term(skin, { snapshot: async () => ({ data: [], seq: 0, generation, running: true }), error });
  term.open(new EventTarget() as HTMLElement, sink);
  await term.attach("agent", Promise.resolve());
  const native = fake.terminals[fake.terminals.length - 1];
  native.modes.bracketedPasteMode = true;
  native.input("prefix");
  const pending = term.paste("context");
  const rejected = expect(pending).rejects.toSatisfy(value => change === "failure" ? value === failure : String(value).includes("notReady"));
  if (change === "replacement") {
    generation = "second";
    await term.attach("other", Promise.resolve());
  } else if (change === "mode") native.modes.bracketedPasteMode = false;
  if (change === "failure") prefix.reject(failure);
  else prefix.resolve();
  await rejected;
  expect(sink).toHaveBeenCalledExactlyOnceWith("agent", "prefix", "first");
  expect(error).toHaveBeenCalledTimes(change === "failure" ? 1 : 0);
  term.dispose();
});

it("keeps Alt+arrow word jumps that xterm 6 no longer sends", () => {
  const key = (key: string, extra: Partial<KeyboardEvent> = {}) => ({ type: "keydown", key, altKey: true, ctrlKey: false, metaKey: false, shiftKey: false, ...extra });
  expect(wordJump(key("ArrowLeft"), true)).toBe("\x1bb");
  expect(wordJump(key("ArrowRight"), true)).toBe("\x1bf");
  expect(wordJump(key("ArrowLeft"), false)).toBe("\x1b[1;5D");
  expect(wordJump(key("ArrowRight"), false)).toBe("\x1b[1;5C");
  expect(wordJump(key("ArrowLeft", { shiftKey: true }), true)).toBeUndefined();
  expect(wordJump(key("ArrowLeft", { type: "keyup" }), true)).toBeUndefined();
  expect(wordJump(key("ArrowUp"), true)).toBeUndefined();
});
