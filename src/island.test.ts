import { describe, expect, it, vi } from "vitest";
import type { Board, Status, Workspace } from "./types";

vi.mock("./ipc", () => ({ invoke: vi.fn() }));
import { snapshot, track } from "./island";

const tab = (id: string, status: Status) => ({ id, title: "", status, note: null, tokens: null });
const workspace = (id: string, tabs: ReturnType<typeof tab>[], extra: Partial<Workspace> = {}) =>
  ({ id, title: id, agent: "claude", model: "opus", tabs, archived: false, cleaned: false, preparing: false, ...extra }) as Workspace;
const board = (...workspaces: Workspace[]): Board => ({ stages: [], projects: [], workspaces });
const line = (event: Record<string, unknown>) => JSON.stringify({ v: 1, at: 1_000, ...event });

describe("notch island", () => {
  it("follows prompt, tool and open requests until the turn ends", () => {
    const store = new Map();
    track(store, "a", line({ type: "user.message", content: [{ kind: "text", text: "fix the auth bug" }] }));
    track(store, "a", line({ type: "assistant.block", messageId: "m", index: 0, block: { kind: "tool", id: "t", name: "Edit", input: { file_path: "src/auth/middleware.ts", old_string: "x".repeat(5_000), new_string: "y" } } }));
    track(store, "a", line({ type: "request.opened", requestId: "r", kind: "approval", toolId: "t", tool: "Edit", input: { file_path: "src/auth/middleware.ts", old_string: "x".repeat(5_000), new_string: "y" } }));
    const [open] = snapshot(board(workspace("ws", [tab("a", "querendo")])), store, []).tabs;
    expect(open).toMatchObject({ title: "ws", prompt: "fix the auth bug", since: 1_000, activity: { tool: "Edit", target: "middleware.ts" } });
    expect(open.request).toMatchObject({ id: "r", requestKind: "approval", toolUseId: "t" });
    expect((open.request!.input.old_string as string).length).toBeLessThan(2_100);

    expect(track(store, "a", line({ type: "request.closed", requestId: "r", outcome: "allowed" }))).toBe(true);
    expect(snapshot(board(workspace("ws", [tab("a", "rodando")])), store, []).tabs[0].request).toBeNull();
    track(store, "a", line({ type: "request.opened", requestId: "q", kind: "question", toolId: null, tool: "AskUserQuestion", input: { questions: [] } }));
    track(store, "a", line({ type: "turn.completed", outcome: "ok", message: "", durationMs: null, costUsd: null }));
    expect(snapshot(board(workspace("ws", [tab("a", "pronta")])), store, []).tabs[0]).toMatchObject({ request: null, activity: null });
    expect(track(store, "a", "not json")).toBe(false);

    // Question text keys the answers, so it must reach the overlay unchanged.
    const long = "q".repeat(5_000);
    track(store, "a", line({ type: "request.opened", requestId: "long", kind: "question", toolId: null, tool: "AskUserQuestion", input: { questions: [{ question: long, options: [] }] } }));
    const [asking] = snapshot(board(workspace("ws", [tab("a", "querendo")])), store, []).tabs;
    expect((asking.request!.input.questions as { question: string }[])[0].question).toBe(long);
  });

  it("lists local active tabs, most urgent first, and drops empty quota", () => {
    const store = new Map();
    const result = snapshot(board(
      workspace("idle", [tab("i", "pronta")]),
      workspace("busy", [tab("b", "rodando")]),
      workspace("ask", [tab("q", "querendo")]),
      workspace("off", [tab("o", "desligada")]),
      workspace("gone", [tab("g", "rodando")], { archived: true }),
      workspace("shared", [tab("s", "rodando")], { remote: {} } as unknown as Partial<Workspace>),
    ), store, [{ agent: "claude", windows: [{ label: "5h", pct: 11, resets: 1 }] }, { agent: "codex", windows: [] }]);
    expect(result.tabs.map(item => item.id)).toEqual(["q", "b", "i"]);
    expect(snapshot(board(workspace("idle", [tab("i", "pronta")])), store, [], tab => tab === "i").tabs[0].unseen).toBe(true);
    expect(result.usage.map(item => item.agent)).toEqual(["claude"]);
  });
});
