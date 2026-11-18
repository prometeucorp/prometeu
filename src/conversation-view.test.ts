import { beforeEach, expect, it, vi } from "vitest";
import type { Ctx, Info } from "./chat";

const fake = vi.hoisted(() => {
  class Element {
    hidden = false;
    disabled = false;
    textContent = "";
    dataset = {};
    children: Element[] = [];
    attributes = new Map<string, string>();
    classList = { add: vi.fn(), toggle: vi.fn() };
    listeners = new Map<string, (event: unknown) => void>();
    addEventListener(type: string, listener: (event: unknown) => void) { this.listeners.set(type, listener); }
    removeEventListener(type: string) { this.listeners.delete(type); }
    append(...children: Element[]) { this.children.push(...children); }
    setAttribute(name: string, value: string) { this.attributes.set(name, value); }
    remove = vi.fn();
  }
  return {
    Element,
    chats: [] as any[],
    terminals: [] as any[],
    files: new Map<string, string[]>(),
    contexts: new Map<string, unknown[]>(),
    paste: vi.fn(),
  };
});
vi.mock("./util", () => ({ h: () => new fake.Element() }));
vi.mock("./i18n", () => ({ t: (key: string) => key, fromBack: String }));
vi.mock("./paste", () => ({ pasteFiles: fake.paste }));
vi.mock("./icons", () => ({ icon: (name: string) => name }));
vi.mock("./chat", () => ({ ChatView: class {
  key: string | null = null;
  remote = false;
  constructor() { fake.chats.push(this); }
  open = vi.fn();
  attach = vi.fn(async (key: string) => { this.key = key; this.remote = false; });
  attachRemote = vi.fn((key: string) => { this.key = key; this.remote = true; });
  remoteWrite = vi.fn();
  detach = vi.fn(() => { this.key = null; });
  dispose = vi.fn();
  focus = vi.fn();
  refresh = vi.fn();
  showNativeTools = vi.fn();
  pickFile = vi.fn();
  forget = vi.fn();
  selection = vi.fn(() => "canonical selection");
  canAttachFiles() { return !!this.key && !this.remote; }
  fileDropTarget() {
    const key = this.key;
    if (!key || this.remote) return null;
    return { put: (files: string[]) => fake.files.set(key, files), wait: () => () => {} };
  }
  contextTarget() {
    const key = this.key;
    return key && !this.remote ? (context: unknown) => fake.contexts.set(key, [context]) : null;
  }
  quoteSelection = vi.fn(() => true);
  focusAnchor = vi.fn();
} }));
vi.mock("./terminal-conversation", () => ({ TerminalConversationView: class {
  key: string | null = null;
  selected = "native selection";
  constructor() { fake.terminals.push(this); }
  open = vi.fn();
  attach = vi.fn(async (key: string) => { this.key = key; });
  detach = vi.fn(() => { this.key = null; });
  dispose = vi.fn();
  focus = vi.fn();
  refresh = vi.fn();
  refit = vi.fn();
  resume = vi.fn(async () => {});
  running() { return this.key !== null; }
  busy() { return false; }
  selection() { return this.selected; }
  paste = vi.fn(async (_text: string) => {});
  inputTarget() { return this.key ? { paste: this.paste, focus: this.focus } : null; }
} }));

import { ConversationView } from "./conversation-view";

beforeEach(() => {
  fake.chats.length = 0;
  fake.terminals.length = 0;
  fake.files.clear();
  fake.contexts.clear();
  fake.paste.mockReset();
});

function open(modes: Record<string, "chat" | "terminal"> = {}) {
  const view = new ConversationView();
  const comment = vi.fn();
  const context: Ctx = {
    say: vi.fn(), comment, nativeToolsChanged: vi.fn(),
    info: () => ({ conversationMode: modes[view.current() ?? ""] ?? "chat", status: "pronta", team: true } as Info),
  };
  const host = new fake.Element();
  view.open(host as unknown as HTMLElement, context);
  return { view, comment, host, context };
}

it("keeps legacy and remote conversations on the canonical view without opening a local terminal", async () => {
  const { view } = open({ remote: "terminal" });
  await view.attach("legacy");
  view.focus();
  expect(fake.chats[0].attach).toHaveBeenCalledWith("legacy");
  expect(fake.chats[0].focus).toHaveBeenCalledOnce();
  view.attachRemote("remote", new Uint8Array([1]));
  view.remoteWrite("remote", new Uint8Array([2]));
  expect(fake.terminals).toHaveLength(0);
  expect(fake.chats[0].attachRemote).toHaveBeenCalledWith("remote", new Uint8Array([1]));
  expect(fake.chats[0].remoteWrite).toHaveBeenCalledWith("remote", new Uint8Array([2]));
  expect(view.fileDropTarget()).toBeNull();
  expect(view.current()).toBe("remote");
});

it("keeps native conversations on the terminal while retaining shared product controls", async () => {
  const { view } = open({ native: "terminal" });
  await view.attach("native");
  view.focus();
  expect(fake.terminals[0].attach).toHaveBeenCalledWith("native");
  expect(fake.chats[0].attach).toHaveBeenCalledWith("native");
  expect(fake.terminals[0].focus).toHaveBeenCalledOnce();
  expect(view.nativeActions().some(item => item !== "sep" && item.track === "conversation.terminal.tools.open")).toBe(true);
  view.focusAnchor("assistant:stable");
  expect(fake.chats[0].focusAnchor).not.toHaveBeenCalled();
  expect(fake.chats[0].focus).not.toHaveBeenCalled();
  expect(fake.terminals[0].focus).toHaveBeenCalledTimes(2);
  expect(view.selection()).toBe("native selection");
});

it("shows manual resume only when automatic attachment has finished without a running terminal", async () => {
  const { view, context } = open({ native: "terminal" });
  context.info = () => ({ conversationMode: "terminal", status: "desligada" } as Info);
  await view.attach("native");
  const terminal = fake.terminals[0];
  const resume = () => view.nativeActions().find(item => item !== "sep" && item.track === "conversation.terminal.resume");
  expect(resume()).toBeUndefined();
  terminal.running = () => false;
  terminal.busy = () => true;
  expect(resume()).toBeUndefined();
  terminal.busy = () => false;
  const action = resume();
  expect(action).toBeDefined();
  if (action && action !== "sep") action.run?.();
  expect(terminal.resume).toHaveBeenCalledOnce();
});

it("opens native tools on demand and reflects pending context without adding a footer", async () => {
  const { view, context: hostContext, host } = open({ native: "terminal" });
  await view.attach("native");
  // The surface owns only terminal and product hosts; controls belong to its parent's chrome.
  expect(host.children[0].children).toHaveLength(2);
  const tools = view.nativeActions().find(item => item !== "sep" && item.track === "conversation.terminal.tools.open");
  if (tools && tools !== "sep") tools.run?.();
  expect(fake.chats[0].showNativeTools).toHaveBeenCalledOnce();
  const context = fake.chats[0].open.mock.calls[0][1] as Ctx;
  context.nativeToolsChanged!(true);
  expect(view.nativeToolsPending()).toBe(true);
  expect(hostContext.nativeToolsChanged).toHaveBeenLastCalledWith(true);
  context.nativeToolsChanged!(false);
  expect(view.nativeToolsPending()).toBe(false);
  await view.attach("legacy");
  expect(view.nativeActions()).toEqual([]);
});

it("keeps native menu actions on their captured conversation after navigation", async () => {
  const { view } = open({ first: "terminal", second: "terminal" });
  await view.attach("first");
  const stale = view.nativeActions();
  await view.attach("second");
  for (const action of stale) if (action !== "sep") action.run?.();
  await view.attach("first");
  for (const action of stale) if (action !== "sep") action.run?.();
  expect(fake.chats[0].showNativeTools).not.toHaveBeenCalled();
  expect(fake.chats[0].pickFile).not.toHaveBeenCalled();
  const attach = view.nativeActions().find(item => item !== "sep" && item.track === "chat.addFile");
  if (attach && attach !== "sep") attach.run?.();
  expect(fake.chats[0].pickFile).toHaveBeenCalledOnce();
});

it("preserves delayed attachment destinations across native navigation without opening a chat", async () => {
  const { view } = open({ first: "terminal", second: "terminal" });
  await view.attach("first");
  const captured = view.fileDropTarget()!;
  const context = view.contextTarget()!;
  await view.attach("second");
  captured.put(["/first/file.png"]);
  context({ selection: {} } as Parameters<typeof context>[0]);
  expect(fake.files.get("first")).toEqual(["/first/file.png"]);
  expect(fake.contexts.has("first")).toBe(true);
  expect(view.attachFiles(["/second/file.png"])).toBe(true);
  expect(fake.files.get("second")).toEqual(["/second/file.png"]);
  expect(fake.chats[0].focus).not.toHaveBeenCalled();
  view.focus();
  expect(fake.terminals[0].focus).toHaveBeenCalledOnce();
});

it("quotes selected native text without manufacturing a transcript anchor", async () => {
  const { view, comment } = open({ native: "terminal" });
  await view.attach("native");
  expect(view.quoteSelection()).toBe(true);
  expect(comment).toHaveBeenCalledWith({ tab: "native", anchor: null, quote: "native selection" });
  await view.attach("legacy");
  expect(view.quoteSelection()).toBe(true);
  expect(fake.chats[0].quoteSelection).toHaveBeenCalledOnce();
});

it("keeps delayed clipboard attachments on their native tab and leaves ordinary paste to the terminal", async () => {
  const { view, host } = open({ first: "terminal", second: "terminal" });
  await view.attach("first");
  const terminalHost = host.children[0].children[0];
  const paste = terminalHost.listeners.get("paste")!;
  const event = { defaultPrevented: false, stopPropagation: vi.fn() };
  paste(event);
  expect(event.stopPropagation).not.toHaveBeenCalled();
  const put = fake.paste.mock.calls[0][1];
  await view.attach("second");
  put(["/first/screenshot.png"]);
  expect(fake.files.get("first")).toEqual(["/first/screenshot.png"]);
  expect(fake.files.has("second")).toBe(false);
  event.defaultPrevented = true;
  paste(event);
  expect(event.stopPropagation).toHaveBeenCalledOnce();
  view.dispose();
  expect(terminalHost.listeners.has("paste")).toBe(false);
});

it("routes product insertions to the current native input and keeps chat input for ordinary tabs", async () => {
  const { view } = open({ native: "terminal" });
  await view.attach("native");
  const context = fake.chats[0].open.mock.calls[0][1] as Ctx;
  await context.nativeInput!()!.paste("review context");
  expect(fake.terminals[0].paste).toHaveBeenCalledExactlyOnceWith("review context");
  expect(fake.chats[0].focus).not.toHaveBeenCalled();
  await view.attach("legacy");
  expect(context.nativeInput!()).toBeNull();
  view.focusAnchor("assistant:stable");
  expect(fake.chats[0].focusAnchor).toHaveBeenCalledWith("assistant:stable");
  view.focus();
  expect(fake.chats[0].focus).toHaveBeenCalledOnce();
});

it("detaches the terminal and product controls and disposes both owners when a conversation is removed", async () => {
  const { view } = open({ native: "terminal" });
  await view.attach("native");
  view.detach();
  expect(view.current()).toBeNull();
  expect(fake.terminals[0].detach).toHaveBeenCalled();
  expect(fake.chats[0].detach).toHaveBeenCalled();
  const alive = new Set(["other"]);
  view.forget(alive);
  expect(fake.chats[0].forget).toHaveBeenCalledWith(alive);
  view.dispose(true);
  view.dispose(true);
  expect(fake.terminals[0].dispose).toHaveBeenCalledOnce();
  expect(fake.chats[0].dispose).toHaveBeenCalledExactlyOnceWith(true);
});
