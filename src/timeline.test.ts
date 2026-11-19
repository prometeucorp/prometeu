import { describe, expect, it } from "vitest";
import { pieces, Timeline, summary, touched, turnMetadata } from "./timeline";

const j = (o: unknown) => JSON.stringify(o);
const assistant = (id: string, block: unknown, extra = {}) =>
  j({ type: "assistant", message: { id, role: "assistant", content: [block] }, uuid: `u-${Math.random()}`, ...extra });
const ev = (event: unknown) => j({ type: "stream_event", event, parent_tool_use_id: null });
const observed = (state: "working" | "idle" | "unknown", scope = "root") => j({
  v: 1, type: "execution.observed", at: 1, activityId: "native-cycle", scope, state,
  blockId: null, kind: null, detail: null,
});

describe("turn insights", () => {
  const turn = (timeline: Timeline, id: string, at: number, completion = {}) => {
    timeline.push(j({ v: 1, type: "user.message", at, content: [{ kind: "text", text: "Continue" }] }));
    timeline.push(j({ v: 1, type: "assistant.block", at: at + 10, messageId: id, index: 0, block: { kind: "text", text: "First" } }));
    timeline.push(j({ v: 1, type: "assistant.block", at: at + 20, messageId: id, index: 1, block: { kind: "text", text: "Final" } }));
    timeline.push(j({ v: 1, type: "turn.completed", at: at + 50, outcome: "ok", message: "", durationMs: 4000, costUsd: 99, messageId: id, ...completion }));
  };
  it("prefers completion duration and attaches metadata only under the final reply", () => {
    const timeline = new Timeline(); turn(timeline, "one", 100); turn(timeline, "two", 200);
    const speech = pieces(timeline.items).filter(piece => piece.kind === "say");
    expect(turnMetadata(timeline.items, speech[0])).toBeNull();
    expect(turnMetadata(timeline.items, speech[1])).toEqual({ durationMs: 4000 });
    expect(turnMetadata(timeline.items, speech[2])).toBeNull();
    expect(turnMetadata(timeline.items, speech[3])).toEqual({ durationMs: 4000 });
  });
  it("restores usage by exact assistant identity and leaves unmatched replies unchanged", () => {
    const timeline = new Timeline(); turn(timeline, "one", 100); turn(timeline, "two", 200);
    const measurement = { usageScope: "mainAgent" as const, complete: false, selectedModel: null, observedModels: null,
      usage: { inputTokens: 80, outputTokens: null, cacheReadTokens: null, cacheWriteTokens: null, reasoningTokens: null,
        contextUsed: null, contextWindow: null, peakContext: null, modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null }, usageByModel: null };
    expect(timeline.restoreUsage([{ messageId: "one", durationMs: 1234, usage: measurement }, { messageId: "missing", durationMs: 99, usage: measurement }])).toEqual([1]);
    const speech = pieces(timeline.items).filter(piece => piece.kind === "say");
    expect(turnMetadata(timeline.items, speech[1])).toEqual({ durationMs: 1234, usage: measurement });
    expect(turnMetadata(timeline.items, speech[3])).toEqual({ durationMs: 4000 });
    expect(timeline.clearRestoredUsage()).toEqual([1]);
    expect(turnMetadata(timeline.items, speech[1])).toEqual({ durationMs: 4000 });
    expect(turnMetadata(timeline.items, speech[3])).toEqual({ durationMs: 4000 });
  });
  it("retains context updates and lowers occupancy only on a reported compaction result", () => {
    const timeline = new Timeline();
    timeline.push(j({ v: 1, type: "context.updated", at: 1, used: 180000, window: 200000 }));
    expect(timeline.context).toEqual({ used: 180000, window: 200000 });
    timeline.push(j({ v: 1, type: "context.compacted", at: 2, before: 180000, after: 20000 }));
    expect(timeline.context).toEqual({ used: 20000, window: 200000 });
  });
  it("restores the known context window from a canonical completion after replayed compaction", () => {
    const timeline = new Timeline();
    timeline.push(j({ v: 1, type: "context.compacted", at: 1, before: 180000, after: 20000 }));
    turn(timeline, "one", 100, { usage: contextMeasurement });
    expect(timeline.context).toEqual({ used: 24000, window: 200000 });
  });
  it("uses the latest restored context without overwriting a later compaction or retaining erased lookup data", () => {
    const timeline = new Timeline(); turn(timeline, "one", 100);
    const rows = [{ messageId: "one", durationMs: 1200, usage: contextMeasurement }];
    timeline.restoreUsage(rows);
    expect(timeline.context).toEqual({ used: 24000, window: 200000 });
    timeline.clearRestoredUsage();
    expect(timeline.context).toBeNull();
    timeline.push(j({ v: 1, type: "context.compacted", at: 200, before: 24000, after: 2000 }));
    timeline.restoreUsage(rows);
    expect(timeline.context).toEqual({ used: 2000, window: null });
  });
});

const contextMeasurement = { usageScope: "mainAgent" as const, complete: true, selectedModel: null, observedModels: null,
  usage: { inputTokens: 24000, outputTokens: 1000, cacheReadTokens: null, cacheWriteTokens: null, reasoningTokens: null,
    contextUsed: 24000, contextWindow: 200000, peakContext: 24000, modelCalls: null, compactions: null, cacheRebuilds: null, costUsd: null }, usageByModel: null };

describe("Timeline", () => {
  it("settles native activity on explicit root idle and keeps late source revisions quiet", () => {
    const t = new Timeline();
    t.push(observed("idle"));
    t.push(observed("working"));
    t.push(j({ v: 1, type: "assistant.started", at: 1, messageId: "native" }));
    t.push(j({ v: 1, type: "assistant.block.started", at: 2, messageId: "native", index: 0, block: { kind: "text", text: "" } }));
    t.push(j({ v: 1, type: "assistant.delta", at: 3, messageId: "native", index: 0, kind: "text", delta: "Partial" }));
    expect(t.working).toBe(true);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: true, blocks: [{ text: "Partial" }] });

    expect(t.push(observed("idle"))).toEqual([0]);
    expect(t.busy).toBe(false);
    expect(t.working).toBe(false);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: false });
    const keys = pieces(t.items).map(piece => piece.key);
    expect(t.push(j({ v: 1, type: "assistant.block", at: 4, messageId: "native", index: 0, block: { kind: "text", text: "Partial reply before interruption" } }))).toEqual([0]);
    expect(pieces(t.items).map(piece => piece.key)).toEqual(keys);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: false, blocks: [{ text: "Partial reply before interruption" }] });
    expect(t.items[0]).not.toHaveProperty("completion");

    t.push(j({ v: 1, type: "assistant.started", at: 5, messageId: "late-native" }));
    t.push(j({ v: 1, type: "assistant.block", at: 6, messageId: "late-native", index: 0, block: { kind: "text", text: "Source catchup" } }));
    expect(t.items.map(item => item.kind)).toEqual(["assistant", "assistant"]);
    expect(t.items[1]).toMatchObject({ kind: "assistant", streaming: false, blocks: [{ text: "Source catchup" }] });
    expect(t.items[1]).not.toHaveProperty("completion");
    expect(t.working).toBe(false);
  });

  it.each([
    ["accepted input", j({ v: 1, type: "user.message", at: 2, content: [{ kind: "text", text: "Continue" }] })],
    ["observed activity", observed("working")],
    ["busy session", j({ v: 1, type: "session.state", at: 2, state: "busy" })],
  ])("does not keep source content quiet after %s", (_name, event) => {
    const t = new Timeline();
    t.push(observed("idle"));
    t.push(event);
    t.push(j({ v: 1, type: "assistant.block", at: 3, messageId: "next", index: 0, block: { kind: "text", text: "Continuing" } }));
    expect(t.busy).toBe(true);
    expect(t.items[t.items.length - 1]).toMatchObject({ kind: "assistant", streaming: true });
  });

  it("does not infer new root activity or a stop from lost observation", () => {
    const t = new Timeline();
    t.push(observed("working"));
    t.push(j({ v: 1, type: "assistant.block", at: 1, messageId: "native", index: 0, block: { kind: "text", text: "Partial" } }));
    t.push(observed("unknown"));
    expect(t.busy).toBe(true);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: true });

    t.push(j({ v: 1, type: "background.changed", at: 2, tasks: [{ id: "child", description: "Child task", toolId: null }] }));
    t.push(observed("idle"));
    t.push(observed("unknown"));
    t.push(j({ v: 1, type: "assistant.block", at: 3, messageId: "native", index: 0, block: { kind: "text", text: "Partial reply before interruption" } }));
    expect(t.busy).toBe(false);
    expect(t.items[0]).toMatchObject({ kind: "assistant", streaming: false, blocks: [{ text: "Partial reply before interruption" }] });
    expect(t.working).toBe(true);
    expect(t.tasks.has("child")).toBe(true);
  });

  it("preserves background work and answerable requests when the native root becomes idle", () => {
    const t = new Timeline();
    t.push(observed("working"));
    t.push(j({ v: 1, type: "background.changed", at: 1, tasks: [{ id: "child", description: "Child task", toolId: null }] }));
    t.push(j({ v: 1, type: "request.opened", at: 2, requestId: "question", kind: "question", toolId: null, tool: null, input: {} }));
    t.push(observed("idle", "background"));
    expect(t.busy).toBe(true);
    t.push(observed("idle"));
    expect(t.busy).toBe(false);
    expect(t.working).toBe(true);
    expect(t.tasks.has("child")).toBe(true);
    expect(t.pending).toMatchObject([{ id: "question", answered: false }]);
    t.push(j({ v: 1, type: "background.changed", at: 3, tasks: [] }));
    expect(t.working).toBe(false);
    expect(t.pending).toMatchObject([{ id: "question", answered: false }]);
  });

  it.each([
    ["question", "custom_question"],
    ["question", null],
    ["plan", "custom_plan"],
    ["plan", null],
    ["approval", "AskUserQuestion"],
    ["approval", null],
  ] as const)("preserves canonical request kind %s with tool %s", (requestKind, tool) => {
    const timeline = new Timeline();
    timeline.push(j({
      v: 1, type: "request.opened", at: 1, requestId: "request", kind: requestKind,
      toolId: null, tool, input: {},
    }));
    expect(timeline.pending).toEqual([{
      kind: "ask", requestKind, ts: 1, id: "request", tool: tool ?? "",
      input: {}, toolUseId: null, answered: false,
    }]);
  });

  it("combines response blocks with the same message ID into one item", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "hi" }, timestamp: "2026-08-27T22:12:36.835Z" }));
    expect(t.busy).toBe(true);
    t.push(assistant("m1", { type: "thinking", thinking: "hmm" }));
    t.push(assistant("m1", { type: "text", text: "hello" }));
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Bash", input: { command: "ls" } }));
    expect(t.items.map((i) => i.kind)).toEqual(["user", "assistant"]);
    const a = t.items[1];
    if (a.kind !== "assistant") throw new Error();
    expect(a.blocks.map((b) => b.kind)).toEqual(["thinking", "text", "tool"]);
    expect(t.items[0].ts).toBe(Date.parse("2026-08-27T22:12:36.835Z"));
  });

  it("updates an earlier authoritative block without moving anchors or reopening completed turns", () => {
    const t = new Timeline();
    for (const id of ["first", "second"]) {
      t.push(j({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "Repeat" }] }));
      t.push(j({ v: 1, type: "assistant.block", at: 2, messageId: id, index: 0, block: { kind: "text", text: "Partial" } }));
      t.push(j({ v: 1, type: "turn.completed", at: 3, outcome: "ok", message: "", durationMs: null, costUsd: null, messageId: id }));
    }
    const keys = pieces(t.items).map(piece => piece.key);
    const count = t.items.length;
    expect(t.busy).toBe(false);
    expect(t.push(j({ v: 1, type: "assistant.block", at: 4, messageId: "first", index: 0, block: { kind: "text", text: "Final text" } }))).toEqual([1]);
    expect(t.items).toHaveLength(count);
    expect(pieces(t.items).map(piece => piece.key)).toEqual(keys);
    expect(t.items[1]).toMatchObject({ kind: "assistant", ts: 2, streaming: false, blocks: [{ text: "Final text" }] });
    expect(t.busy).toBe(false);
  });

  it("places tool results inside their tool block instead of a message", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Bash", input: { command: "ls" } }));
    const touched = t.push(
      j({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "a.txt\n" }] } }),
    );
    expect(touched).toEqual([0]);
    expect(t.items).toHaveLength(1);
    const a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "tool") throw new Error();
    expect(a.blocks[0].result).toBe("a.txt\n");
    expect(a.blocks[0].done).toBe(true);
  });

  it("replaces the streaming draft with the authoritative block when it arrives", () => {
    const t = new Timeline();
    t.push(ev({ type: "message_start", message: { id: "m1" } }));
    t.push(ev({ type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }));
    t.push(ev({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "hel" } }));
    t.push(ev({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "lo" } }));
    let a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "text") throw new Error();
    expect(a.blocks[0].text).toBe("hello");
    expect(a.streaming).toBe(true);

    t.push(ev({ type: "content_block_start", index: 1, content_block: { type: "tool_use", id: "tu1", name: "Write", input: {} } }));
    t.push(ev({ type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: '{"file_path": "a' } }));
    t.push(ev({ type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: '.txt"}' } }));
    t.push(ev({ type: "content_block_stop", index: 1 }));
    a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[1].kind !== "tool") throw new Error();
    expect(a.blocks[1].input).toEqual({ file_path: "a.txt" });

    // Complete lines arrive in order as one item with two blocks.
    t.push(assistant("m1", { type: "text", text: "hello!" }));
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Write", input: { file_path: "a.txt", content: "x" } }));
    expect(t.items).toHaveLength(1);
    a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "text" || a.blocks[1].kind !== "tool") throw new Error();
    expect(a.blocks[0].text).toBe("hello!");
    expect(a.blocks[1].input).toEqual({ file_path: "a.txt", content: "x" });

    t.push(j({ type: "result", subtype: "success", is_error: false, duration_ms: 10 }));
    expect(t.busy).toBe(false);
    expect((t.items[0] as { streaming: boolean }).streaming).toBe(false);
    // Successful turns add no error row.
    expect(t.items).toHaveLength(1);
  });

  it("new input closes the streaming message and reports the change", () => {
    const t = new Timeline();
    t.push(ev({ type: "message_start", message: { id: "m1" } }));
    t.push(ev({ type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }));
    expect((t.items[0] as { streaming: boolean }).streaming).toBe(true);
    const touched = t.push(j({ type: "user", message: { role: "user", content: "another" } }));
    expect(touched).toEqual([0, 1]);
    expect((t.items[0] as { streaming: boolean }).streaming).toBe(false);
  });

  it("preserves thinking deltas when the authoritative block is empty", () => {
    const t = new Timeline();
    t.push(ev({ type: "message_start", message: { id: "m1" } }));
    t.push(ev({ type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "" } }));
    t.push(ev({ type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "considered" } }));
    t.push(assistant("m1", { type: "thinking", thinking: "", signature: "x" }));
    const a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "thinking") throw new Error();
    expect(a.blocks[0].text).toBe("considered");
  });

  it("discards deltas without an open item when joining midstream", () => {
    const t = new Timeline();
    expect(t.push(ev({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "x" } }))).toEqual([]);
    expect(t.items).toHaveLength(0);
  });

  it("turns permission requests into cards and closes them on tool results", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "ExitPlanMode", input: { plan: "# P" } }));
    t.push(
      j({
        type: "control_request",
        request_id: "r1",
        request: { subtype: "can_use_tool", tool_name: "ExitPlanMode", input: { plan: "# P" }, tool_use_id: "tu1" },
      }),
    );
    expect(t.pending.map((a) => a.id)).toEqual(["r1"]);
    expect(t.pending[0].requestKind).toBe("plan");
    // Overlapping snapshot and live delivery must produce one card.
    t.push(
      j({ type: "control_request", request_id: "r1", request: { subtype: "can_use_tool", tool_name: "ExitPlanMode", input: {} } }),
    );
    expect(t.items.filter((i) => i.kind === "ask")).toHaveLength(1);
    t.push(j({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "ok" }] } }));
    expect(t.pending).toEqual([]);
  });

  it("closes the request card immediately after a local response", () => {
    const t = new Timeline();
    t.push(j({ type: "control_request", request_id: "r1", request: { subtype: "can_use_tool", tool_name: "Bash", input: {} } }));
    expect(t.answer("r1")).toEqual([0]);
    expect(t.pending).toEqual([]);
  });

  it("turns errors and interruptions into timeline rows", () => {
    const t = new Timeline();
    t.push(j({ type: "result", subtype: "error_during_execution", is_error: true, errors: ["interrupted"] }));
    expect(t.items[0]).toMatchObject({ kind: "result", error: true, text: "interrupted" });
  });

  it("excludes metadata, subagents, noise and malformed input", () => {
    const t = new Timeline();
    t.push(j({ type: "user", isMeta: true, message: { role: "user", content: "<local-command-stdout>x</local-command-stdout>" } }));
    t.push(j({ type: "assistant", parent_tool_use_id: "tu9", message: { id: "sub", content: [{ type: "text", text: "I am a subagent" }] } }));
    t.push(j({ type: "system", subtype: "hook_started" }));
    t.push(j({ type: "attachment", attachment: {} }));
    t.push("{partial line");
    expect(t.items).toEqual([]);
  });

  it("shows a compaction notice while running and a boundary afterward", () => {
    const t = new Timeline();
    t.push(j({ type: "system", subtype: "status", status: "compacting" }));
    expect(t.compacting).toBe(true);
    t.push(j({ type: "system", subtype: "status", status: null }));
    expect(t.compacting).toBe(false);
    t.push(j({ type: "system", subtype: "compact_boundary" }));
    expect(t.items[0]).toMatchObject({ kind: "system", text: "compacted" });
  });

  it("settles streaming items at the buffer end without an active turn", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "hi" } }));
    t.push(assistant("m1", { type: "text", text: "hello" }));
    t.push(j({ type: "control_request", request_id: "r1", request: { subtype: "can_use_tool", tool_name: "Bash", input: {} } }));
    expect(t.busy).toBe(true);
    expect(t.push(j({ type: "prometheus", subtype: "state", busy: false }))).toEqual([1, 2]);
    expect(t.busy).toBe(false);
    expect((t.items[1] as { streaming: boolean }).streaming).toBe(false);
    expect(t.pending).toEqual([]);
    // Preserve an existing turn association.
    t.push(assistant("m2", { type: "text", text: "again" }));
    t.push(j({ type: "prometheus", subtype: "state", busy: true }));
    expect(t.busy).toBe(true);
  });

  it("shows process stderr as an error", () => {
    const t = new Timeline();
    t.push(j({ type: "prometheus", subtype: "stderr", text: "No conversation found" }));
    expect(t.items[0]).toMatchObject({ kind: "system", error: true, text: "No conversation found" });
  });

  it("combines consecutive stderr lines into one error", () => {
    const t = new Timeline();
    for (const detail of ["fn example() {", "  fail();", "}"]) {
      t.push(j({ v: 1, type: "system.notice", at: 1, level: "error", code: "provider.stderr", detail }));
    }

    expect(t.items).toHaveLength(1);
    expect(t.items[0]).toMatchObject({
      kind: "system",
      error: true,
      what: "stderr",
      text: "fn example() {\n  fail();\n}",
    });
  });

  it("keeps timestamps monotonic and inherits missing timestamps", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "a" }, ts: 1000 }));
    t.push(assistant("m1", { type: "text", text: "b" }));
    expect(t.items[1].ts).toBe(1000);
    t.push(j({ type: "user", message: { role: "user", content: "c" }, timestamp: new Date(500).toISOString() }));
    expect(t.items[2].ts).toBe(500);
  });

  it("initialize returns slash commands and init removes terminal-only commands", () => {
    const t = new Timeline();
    expect(t.commands).toEqual([]);
    const answer = {
      type: "control_response",
      response: {
        subtype: "success",
        request_id: "initialize",
        response: {
          commands: [
            { name: "compact", description: "Free up context", argumentHint: "<instructions>" },
            { name: "color", description: "Set the color" },
            { name: "release", description: "Publish a release" },
            { bogus: true },
          ],
        },
      },
    };
    expect(t.push(j(answer))).toEqual([]);
    expect(t.items).toEqual([]);
    expect(t.commands).toEqual([
      { name: "compact", description: "Free up context", hint: "<instructions>" },
      { name: "color", description: "Set the color", hint: "" },
      { name: "release", description: "Publish a release", hint: "" },
    ]);
    // Initialization follows the first prompt and identifies terminal commands.
    t.push(j({ type: "system", subtype: "init", slash_commands: ["compact", "color", "release"], terminal_slash_commands: ["color"] }));
    expect(t.commands.map((c) => c.name)).toEqual(["compact", "release"]);
    // A restarted process preserves command metadata from initialization.
    t.push(j(answer));
    expect(t.commands.map((c) => c.name)).toEqual(["compact", "release"]);
    // Unrelated responses such as permission acknowledgements change nothing.
    t.push(j({ type: "control_response", response: { subtype: "success", request_id: "x", response: {} } }));
    expect(t.commands.length).toBe(2);
  });

  it("keeps skill content inside its card instead of creating a message", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Skill", input: { skill: "release" } }));
    t.push(j({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "Launching skill: release" }] } }));
    t.push(j({ type: "user", isSynthetic: true, message: { role: "user", content: [{ type: "text", text: "You will publish a release" }] } }));
    const a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "tool") throw new Error();
    expect(a.blocks[0].result).toBe("You will publish a release");
    expect(t.items).toHaveLength(1);
  });

  it("does not swallow the next message after a background skill", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Skill", input: { skill: "code-review" } }));
    t.push(j({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "Launching skill: code-review" }] } }));
    t.push(assistant("m2", { type: "text", text: "I invoked the skill" }));
    t.push(j({ type: "user", isMeta: true, message: { role: "user", content: "<local-command-stdout>x</local-command-stdout>" } }));
    t.push(j({ type: "user", message: { role: "user", content: "any update?" } }));
    if (t.items[0].kind !== "assistant" || t.items[0].blocks[0].kind !== "tool") throw new Error();
    expect(t.items[0].blocks[0].result).toBe("Launching skill: code-review");
    expect(t.items[2]).toMatchObject({ kind: "user", text: "any update?" });
  });

  it("keeps background task cards active until notification and reports their count", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Agent", input: { description: "explore", run_in_background: true } }));
    t.push(j({ type: "system", subtype: "task_started", task_id: "bg1", tool_use_id: "tu1", description: "explore", is_backgrounded: true }));
    t.push(j({ type: "user", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "tu1", content: "Agent started" }] } }));
    t.push(j({ type: "system", subtype: "background_tasks_changed", tasks: [{ task_id: "bg1", description: "explore" }] }));
    t.push(j({ type: "result", subtype: "success" }));
    const a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "tool") throw new Error();
    expect(a.blocks[0].done).toBe(true);
    expect(a.blocks[0].background).toBe(true);
    expect([...t.tasks.values()].map((k) => k.description)).toEqual(["explore"]);
    expect(t.busy).toBe(false);
    // The turn ended, but the child it started has not: the conversation is still working.
    expect(t.working).toBe(true);
    const touched = t.push(
      j({ type: "system", subtype: "task_notification", task_id: "bg1", tool_use_id: "tu1", status: "completed", summary: 'Agent "explore" finished' }),
    );
    expect(touched).toEqual([0, 1]);
    expect(a.blocks[0].background).toBe(false);
    expect(t.tasks.size).toBe(0);
    expect(t.working).toBe(false);
    expect(t.items[1]).toMatchObject({ kind: "system", text: 'Agent "explore" finished', error: false, what: "background" });
  });

  it("stops working when an interruption ends the turn and the children it started", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "tu1", name: "Agent", input: { description: "explore", run_in_background: true } }));
    t.push(j({ type: "system", subtype: "task_started", task_id: "bg1", tool_use_id: "tu1", description: "explore", is_backgrounded: true }));
    t.push(j({ type: "system", subtype: "background_tasks_changed", tasks: [{ task_id: "bg1", description: "explore" }] }));
    expect(t.working).toBe(true);
    const touched = t.push(j({ type: "result", subtype: "error_during_execution", is_error: true }));
    expect(t.tasks.size).toBe(0);
    expect(t.working).toBe(false);
    const a = t.items[0];
    if (a.kind !== "assistant" || a.blocks[0].kind !== "tool") throw new Error();
    expect(a.blocks[0].background).toBe(false);
    expect(touched).toContain(0);
  });

  it("normalizes task notifications from user XML into the same system row", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "<task-notification>\n<task-id>x</task-id>\n<summary>Agent \"explore\" finished</summary>\n</task-notification>" } }));
    expect(t.items[0]).toMatchObject({ kind: "system", text: 'Agent "explore" finished', what: "background" });
    expect(t.busy).toBe(false);
  });

  it("appends the unescaped subagent report to its task notification", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "<task-notification>\n<task-id>x</task-id>\n<summary>Agent \"explore\" completed</summary>\n<result>## Findings\n\n`a &amp;&amp; b` returns 2&gt;1</result>\n</task-notification>" } }));
    expect(t.items[0]).toMatchObject({ kind: "system", text: 'Agent "explore" completed\n\n## Findings\n\n`a && b` returns 2>1', what: "background" });
  });

  it("shows compaction progress, final size and folded summary while excluding command echoes", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "/compact" } }));
    t.push(j({ type: "system", subtype: "status", status: "compacting" }));
    expect(t.compacting).toBe(true);
    t.push(j({ type: "system", subtype: "status", status: null, compact_result: "success" }));
    expect(t.compacting).toBe(false);
    t.push(j({ type: "system", subtype: "compact_boundary", compact_metadata: { pre_tokens: 23978, post_tokens: 3132 } }));
    t.push(j({ type: "user", isCompactSummary: true, message: { role: "user", content: "This session is being continued from a previous conversation…" } }));
    t.push(j({ type: "user", message: { role: "user", content: "<local-command-stdout>Compacted </local-command-stdout>" } }));
    t.push(j({ type: "user", message: { role: "user", content: "<command-name>/compact</command-name>" } }));
    t.push(j({ type: "result", subtype: "success" }));
    expect(t.items.map((i) => i.kind)).toEqual(["user", "system", "system"]);
    expect(t.items[1]).toMatchObject({ what: "compacted", tokens: [23978, 3132] });
    expect(t.items[2]).toMatchObject({ what: "summary" });
    expect(t.busy).toBe(false);
  });

  it("shows failed compaction as an error", () => {
    const t = new Timeline();
    t.push(j({ type: "system", subtype: "status", status: "compacting" }));
    t.push(j({ type: "system", subtype: "status", status: null, compact_result: "failed", compact_error: "Not enough messages to compact." }));
    expect(t.compacting).toBe(false);
    expect(t.items[0]).toMatchObject({ kind: "system", error: true, text: "Not enough messages to compact." });
  });

  it("loads a complete buffer and skips truncated input", () => {
    const t = new Timeline();
    t.load(`ssage":{"content":"truncated"}}\n${j({ type: "user", message: { role: "user", content: "complete" } })}\n`);
    expect(t.items.map((i) => i.kind)).toEqual(["user"]);
  });
});

describe("summary", () => {
  it("summarizes the tool target in one line", () => {
    expect(summary("Bash", { command: "ls -la\necho x", description: "list" })).toBe("ls -la");
    expect(summary("Read", { file_path: "/a/b.rs" })).toBe("/a/b.rs");
    expect(summary("Task", { description: "find bugs", prompt: "x" })).toBe("find bugs");
    expect(summary("Foo", {})).toBe("");
  });

  it("reads the available string prefix from partial JSON", () => {
    expect(summary("Agent", {}, '{"description": "Find gaps in test c')).toBe("Find gaps in test c");
    expect(summary("Bash", {}, '{"command": "git diff\\nls", "descr')).toBe("git diff");
    expect(summary("Bash", {}, '{"command": "echo \\"a\\" ')).toBe('echo "a" ');
    expect(summary("Bash", {}, '{"comm')).toBe("");
  });
});

describe("pieces", () => {
  const work = (t: Timeline, id: string, tool: string, cmd: string) => {
    t.push(assistant(id, { type: "thinking", thinking: "hmm" }));
    t.push(assistant(id, { type: "tool_use", id: `tu-${id}`, name: tool, input: { command: cmd } }));
  };

  it("groups consecutive work into one piece across messages", () => {
    const t = new Timeline();
    t.push(j({ type: "user", message: { role: "user", content: "hi" } }));
    work(t, "m1", "Bash", "ls");
    work(t, "m2", "Bash", "pwd");
    work(t, "m3", "Read", "x");
    const p = pieces(t.items);
    expect(p.map((x) => x.kind)).toEqual(["item", "work"]);
    const w = p[1];
    if (w.kind !== "work") throw new Error();
    expect(w.refs.length).toBe(6);
    expect(w.key).toBe("w1.0");
  });

  it("keeps agent messages outside work groups and splits work around them", () => {
    const t = new Timeline();
    work(t, "m1", "Bash", "ls");
    t.push(assistant("m2", { type: "text", text: "found it" }));
    work(t, "m3", "Bash", "pwd");
    expect(pieces(t.items).map((x) => x.kind)).toEqual(["work", "say", "work"]);
  });

  it("splits work at pending requests so request cards remain outside work cards", () => {
    const t = new Timeline();
    work(t, "m1", "Bash", "rm -rf /");
    t.push(
      j({
        type: "control_request",
        request_id: "r1",
        request: { subtype: "can_use_tool", tool_name: "Bash", input: { command: "rm -rf /" } },
      }),
    );
    work(t, "m2", "Bash", "ls");
    expect(pieces(t.items).map((x) => x.kind)).toEqual(["work", "item", "work"]);
  });

  it("lists edited files from newest to oldest without duplicates", () => {
    const t = new Timeline();
    t.push(assistant("m1", { type: "tool_use", id: "t1", name: "Read", input: { file_path: "/ws/a.rb" } }));
    t.push(assistant("m2", { type: "tool_use", id: "t2", name: "Bash", input: { command: "ls /ws/nothing.rb" } }));
    t.push(assistant("m3", { type: "tool_use", id: "t3", name: "Grep", input: { path: "/ws/app" } }));
    t.push(assistant("m4", { type: "tool_use", id: "t4", name: "Edit", input: { file_path: "/ws/b.rb" } }));
    t.push(assistant("m5", { type: "tool_use", id: "t5", name: "Read", input: { file_path: "/ws/a.rb" } }));
    // Bash and Grep do not identify files; repeated reads retain only the most recent position.
    expect(touched(t.items)).toEqual(["/ws/a.rb", "/ws/b.rb"]);
    expect(touched(t.items, 1)).toEqual(["/ws/a.rb"]);
  });

  it("keeps piece keys stable as the conversation grows", () => {
    const t = new Timeline();
    work(t, "m1", "Bash", "ls");
    const before = pieces(t.items).map((x) => x.key);
    work(t, "m2", "Bash", "pwd");
    t.push(assistant("m3", { type: "text", text: "done" }));
    expect(pieces(t.items).map((x) => x.key).slice(0, before.length)).toEqual(before);
  });
});

describe("a conversation whose process is gone", () => {
  const unfinished = () => {
    const timeline = new Timeline();
    timeline.push(j({ v: 1, type: "user.message", at: 100, content: [{ kind: "text", text: "Run the migrations" }] }));
    timeline.push(j({ v: 1, type: "assistant.started", at: 110, messageId: "m" }));
    timeline.push(j({ v: 1, type: "assistant.block", at: 120, messageId: "m", index: 0, block: { kind: "text", text: "Two finished; the third" } }));
    timeline.push(j({ v: 1, type: "request.opened", at: 130, requestId: "q", kind: "question", toolId: null, tool: "AskUserQuestion",
      input: { questions: [{ question: "Keep going?", header: "Next", options: [] }] } }));
    return timeline;
  };
  const state = (value: string) => j({ v: 1, type: "session.state", at: 9000, state: value });

  it("keeps a turn it left unfinished visibly cut off", () => {
    const timeline = unfinished();
    timeline.push(state("stopped"));
    expect(timeline.items.find(item => item.kind === "assistant")).toMatchObject({ streaming: false, cut: true });
    expect(timeline.items.find(item => item.kind === "ask")).toMatchObject({ answered: false, closed: true });
    expect(timeline.pending).toEqual([]);
    expect(timeline.items[timeline.items.length - 1]).toEqual({ kind: "system", ts: 130, text: "", error: false, what: "interrupted" });
    expect(timeline.working).toBe(false);
    // The partial reply is not measured as a finished turn, and the question can no longer be answered.
    const speech = pieces(timeline.items).filter(piece => piece.kind === "say");
    expect(turnMetadata(timeline.items, speech[0])).toBeNull();
    expect(timeline.answer("q")).toEqual([]);
  });

  it("keeps a cut turn cut off after the conversation continued past it", () => {
    const timeline = unfinished();
    // Replay places the recorded cut where the history ended, before the continuation.
    timeline.push(state("stopped"));
    timeline.push(j({ v: 1, type: "user.message", at: 200, content: [{ kind: "text", text: "Continue" }] }));
    timeline.push(j({ v: 1, type: "assistant.block", at: 210, messageId: "n", index: 0, block: { kind: "text", text: "Done" } }));
    timeline.push(j({ v: 1, type: "turn.completed", at: 220, outcome: "ok", message: "", durationMs: 20, costUsd: null, messageId: "n" }));
    timeline.push(state("ready"));
    const replies = timeline.items.flatMap(item => item.kind === "assistant" ? [!!item.cut] : []);
    expect(replies).toEqual([true, false]);
    expect(timeline.items.find(item => item.kind === "ask")).toMatchObject({ answered: false, closed: true });
    const speech = pieces(timeline.items).filter(piece => piece.kind === "say");
    expect(turnMetadata(timeline.items, speech[0])).toBeNull();
    expect(turnMetadata(timeline.items, speech[1])).toEqual({ durationMs: 20 });
  });

  it("marks a turn cut before the agent replied", () => {
    const timeline = new Timeline();
    timeline.push(j({ v: 1, type: "user.message", at: 100, content: [{ kind: "text", text: "Run the migrations" }] }));
    timeline.push(state("stopped"));
    expect(timeline.items.map(item => item.kind === "system" ? item.what : item.kind)).toEqual(["user", "interrupted"]);
  });

  it("leaves a settled conversation as it was", () => {
    const timeline = new Timeline();
    timeline.push(j({ v: 1, type: "user.message", at: 100, content: [{ kind: "text", text: "Hi" }] }));
    timeline.push(j({ v: 1, type: "assistant.block", at: 110, messageId: "m", index: 0, block: { kind: "text", text: "Hello" } }));
    timeline.push(j({ v: 1, type: "turn.completed", at: 120, outcome: "ok", message: "", durationMs: 20, costUsd: null, messageId: "m" }));
    const settled = structuredClone(timeline.items);
    expect(timeline.push(state("stopped"))).toEqual([]);
    expect(timeline.items).toEqual(settled);
  });

  it("still settles a process that is ready for the next message", () => {
    const timeline = unfinished();
    timeline.push(state("ready"));
    expect(timeline.items.find(item => item.kind === "assistant")).toMatchObject({ streaming: false });
    expect(timeline.items.find(item => item.kind === "ask")).toMatchObject({ answered: true });
    expect(timeline.items.some(item => item.kind === "system")).toBe(false);
  });
});
