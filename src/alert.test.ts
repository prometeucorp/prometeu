import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Board, Status, Workspace } from "./types";

const { badge, audio, inbox } = vi.hoisted(() => ({
  badge: vi.fn(), audio: vi.fn(), inbox: [] as { id: string }[],
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setBadgeCount: async (n?: number) => badge(n) }),
}));
vi.mock("./team", () => ({ inboxCount: () => inbox.length }));
import { boardChanged, chatChanged, glanced, init, looked, teamChanged, unseen, waiting } from "./alert";

const tab = (id: string, status: Status = "rodando") => ({ id, title: id, status, note: null, tokens: null });
const workspace = (id: string, tabs = [tab(`${id}-t`)], unread = false) =>
  ({ id, tabs, unread }) as Workspace;
const board = (...workspaces: Workspace[]): Board => ({ stages: [], projects: [], workspaces });
const emit = (tab: string, event: Record<string, unknown>) =>
  chatChanged(tab, JSON.stringify({ v: 1, at: 1, ...event }));
const done = (tab = "a-t") => emit(tab, {
  type: "turn.completed", outcome: "ok", message: "", durationMs: null, costUsd: null,
});
const begin = (tab = "a-t") => {
  emit(tab, { type: "session.state", state: "busy" });
  emit(tab, { type: "assistant.started", messageId: "m" });
};
const settle = () => vi.advanceTimersByTime(1_000);
const background = (tasks: string[], tab = "a-t") => emit(tab, {
  type: "background.changed", tasks: tasks.map((id) => ({ id, description: id, toolId: null })),
});
const question = (tab = "a-t") => emit(tab, {
  type: "request.opened", requestId: "q", kind: "question", toolId: null, tool: "AskUserQuestion", input: {},
});
const speak = (tab = "a-t") => emit(tab, { type: "user.message", content: [{ kind: "text", text: "continue" }] });
let focused = false;
const visible = new Set<string>();
const notify = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  vi.useFakeTimers();
  focused = false;
  visible.clear();
  inbox.length = 0;
  vi.stubGlobal("document", { hasFocus: () => focused });
  vi.stubGlobal("window", { addEventListener() {}, removeEventListener() {} });
  vi.stubGlobal("AudioContext", audio);
  init({ visible: (tab) => visible.has(tab), notify });
  boardChanged(board());
});

afterEach(() => {
  expect(audio).not.toHaveBeenCalled();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("Dock notifications", () => {
  it("requires accepted input and completion; status and synthetic messages do not create notifications", () => {
    boardChanged(board(workspace("a")));
    for (let i = 0; i < 5; i++) {
      boardChanged(board(workspace("a", [tab("a-t", i % 2 ? "pronta" : "rodando")])));
      speak();
      emit("a-t", { type: "assistant.started", messageId: "m" });
      question();
      emit("a-t", { type: "request.closed", requestId: "q", outcome: "answered" });
      done();
      settle();
    }
    expect(waiting()).toBe(0);
    begin();
    question();
    settle();
    expect(waiting()).toBe(1);
    emit("a-t", { type: "request.closed", requestId: "q", outcome: "answered" });
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("tools and streaming do not create notifications; completion updates the Dock", () => {
    boardChanged(board(workspace("a")));
    begin();
    for (let i = 0; i < 10; i++) {
      emit("a-t", { type: "assistant.block", messageId: "m", index: i, block: { kind: "text", text: "working" } });
      emit("a-t", { type: "tool.completed", toolId: "tool", output: "ok", error: false, background: false });
      emit("a-t", { type: "context.updated", used: i, window: 100 });
      settle();
    }
    expect(waiting()).toBe(0);
    done();
    emit("a-t", { type: "context.compaction", state: "stopped", detail: "" });
    done();
    settle();
    expect(waiting()).toBe(1);
    expect(badge).toHaveBeenLastCalledWith(1);
  });

  it("hovering the notch island acknowledges finished conversations but keeps unread workspaces", () => {
    boardChanged(board(workspace("a"), workspace("b", [tab("b-t")], true)));
    begin();
    done();
    settle();
    expect(unseen("a-t")).toBe(true);
    expect(waiting()).toBe(2);
    glanced();
    expect(unseen("a-t")).toBe(false);
    expect(waiting()).toBe(1);
  });

  it("holds the completion while background tasks remain and delivers it when they drain", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["one", "two"]);
    done();
    settle();
    expect(waiting()).toBe(0);
    background(["two"]);
    settle();
    expect(waiting()).toBe(0);
    background([]);
    settle();
    expect(waiting()).toBe(1);
    expect(notify).toHaveBeenCalledWith("done", "a-t");
  });

  it("draining background tasks without a terminal from the main agent completes nothing", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["one"]);
    background([]);
    settle();
    expect(waiting()).toBe(0);
    emit("a-t", { type: "assistant.started", messageId: "still working" });
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("the main agent answering again discards the terminal its background tasks held", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["one"]);
    done();
    emit("a-t", { type: "assistant.started", messageId: "continued" });
    background([]);
    settle();
    expect(waiting()).toBe(0);
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("looking at the conversation consumes the completion its background tasks hold", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["one"]);
    done();
    focused = true;
    visible.add("a-t");
    looked();
    focused = false;
    background([]);
    settle();
    expect(waiting()).toBe(0);
    expect(notify).not.toHaveBeenCalled();
  });

  it("an interruption while background tasks run never becomes a notice", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["one"]);
    emit("a-t", { type: "turn.completed", outcome: "interrupted", message: "", durationMs: null, costUsd: null });
    background([]);
    settle();
    expect(waiting()).toBe(0);
  });

  it("resuming cancels pending completion without inferring completion from silence", () => {
    boardChanged(board(workspace("a")));
    begin();
    done();
    vi.advanceTimersByTime(500);
    emit("a-t", { type: "assistant.started", messageId: "continued" });
    vi.advanceTimersByTime(30_000);
    expect(waiting()).toBe(0);
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("viewing, replying and receiving synthetic messages do not rearm an already notified execution", () => {
    boardChanged(board(workspace("a")));
    begin();
    done();
    settle();
    expect(waiting()).toBe(1);
    focused = true;
    visible.add("a-t");
    looked();
    expect(waiting()).toBe(0);
    focused = false;
    speak();
    question();
    emit("a-t", { type: "request.closed", requestId: "q", outcome: "answered" });
    emit("a-t", { type: "assistant.started", messageId: "automatic" });
    done();
    settle();
    expect(waiting()).toBe(0);
    begin();
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("consumes viewed completion so leaving the tab does not create a later notification", () => {
    boardChanged(board(workspace("a")));
    focused = true;
    visible.add("a-t");
    begin();
    done();
    settle();
    focused = false;
    done();
    settle();
    expect(waiting()).toBe(0);
    begin();
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("viewing completion during the waiting period cancels notification even after leaving", () => {
    boardChanged(board(workspace("a")));
    begin();
    done();
    focused = true;
    visible.add("a-t");
    looked();
    focused = false;
    settle();
    expect(waiting()).toBe(0);
    done();
    settle();
    expect(waiting()).toBe(0);
  });

  it("completes tabs independently; the Dock counts workspaces and preserves unread state", () => {
    boardChanged(board(workspace("a", [tab("a-t"), tab("b-t")], true)));
    begin();
    begin("b-t");
    done();
    done("b-t");
    settle();
    expect(waiting()).toBe(1);
    boardChanged(board());
    expect(waiting()).toBe(0);
  });

  it("archiving or removing a tab cancels its notification; remote tabs stay out of the Dock", () => {
    boardChanged(board(workspace("a"), { ...workspace("b"), remote: {} } as Workspace));
    begin();
    done();
    begin("b-t");
    done("b-t");
    boardChanged(board({ ...workspace("a"), archived: true }));
    settle();
    expect(waiting()).toBe(0);
  });

  it("restarting a process discards old background tasks; new input in the same process preserves them", () => {
    boardChanged(board(workspace("a")));
    begin();
    background(["old"]);
    begin();
    done();
    settle();
    expect(waiting()).toBe(0);
    emit("a-t", { type: "session.state", state: "starting" });
    begin();
    done();
    settle();
    expect(waiting()).toBe(1);
  });

  it("ignores invalid data, local commands and interruptions; terminal errors notify", () => {
    boardChanged(board(workspace("a")));
    chatChanged("a-t", "invalid json");
    emit("a-t", { type: "turn.completed" });
    emit("a-t", { type: "session.state", state: "busy" });
    emit("a-t", { type: "context.reported", markdown: "context" });
    done();
    emit("a-t", { type: "assistant.started", messageId: "automatic" });
    done();
    settle();
    expect(waiting()).toBe(0);
    begin();
    emit("a-t", { type: "turn.completed", outcome: "interrupted", message: "", durationMs: null, costUsd: null });
    done();
    settle();
    expect(waiting()).toBe(0);
    emit("a-t", { type: "session.state", state: "busy" });
    emit("a-t", { type: "turn.completed", outcome: "error", message: "failed", durationMs: null, costUsd: null });
    settle();
    expect(waiting()).toBe(1);
  });
});

it("comments update the Dock silently, including after reconnecting", () => {
  inbox.push({ id: "comment" });
  teamChanged();
  teamChanged();
  expect(badge).toHaveBeenLastCalledWith(1);
  inbox.length = 0;
  teamChanged();
  expect(badge).toHaveBeenLastCalledWith(undefined);
  inbox.push({ id: "comment" });
  teamChanged();
  expect(waiting()).toBe(1);
  expect(badge).toHaveBeenLastCalledWith(1);
});

it("delivers each live completion and request once without changing Dock eligibility", () => {
  boardChanged(board(workspace("a")));
  begin();
  question(); question();
  expect(notify).toHaveBeenCalledTimes(1);
  expect(notify).toHaveBeenLastCalledWith("approval", "a-t");
  emit("a-t", { type: "request.closed", requestId: "q", outcome: "answered" });
  done(); done(); settle(); done(); settle();
  expect(notify).toHaveBeenCalledTimes(2);
  expect(notify).toHaveBeenLastCalledWith("done", "a-t");
  begin();
  emit("a-t", { type: "turn.completed", outcome: "error", message: "failed", durationMs: null, costUsd: null });
  settle();
  expect(notify).toHaveBeenLastCalledWith("error", "a-t");
});

it("does not deliver viewed, cancelled, interrupted, or replayed completions", () => {
  boardChanged(board(workspace("a")));
  done(); settle();
  focused = true; visible.add("a-t");
  begin(); question(); done(); settle();
  focused = false;
  begin(); done();
  emit("a-t", { type: "assistant.started", messageId: "continuation" });
  settle();
  emit("a-t", { type: "turn.completed", outcome: "interrupted", message: "", durationMs: null, costUsd: null });
  done(); settle();
  expect(notify).not.toHaveBeenCalled();
});
