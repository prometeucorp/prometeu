import antigravityFixture from "../src-tauri/src/antigravity/fixtures/canonical-events.json?raw";
import { describe, expect, it } from "vitest";
import { parseConversationEvent } from "./conversation";
import { LegacyConversationAdapter } from "./conversation-legacy";
import { Timeline } from "./timeline";
import type { TelemetryMeasurement } from "./telemetry";

const line = (value: unknown) => JSON.stringify(value);

describe("ConversationEventV1", () => {
  it("keeps old completions and ignores malformed optional usage without losing settlement", () => {
    const completion = { v: 1, type: "turn.completed", at: 1, outcome: "ok", message: "", durationMs: 100, costUsd: 99 };
    const usage: TelemetryMeasurement = { usageScope: "mainAgent", complete: true, selectedModel: null,
      observedModels: ["example"], usageByModel: null, usage: {
        inputTokens: 100, outputTokens: 20, cacheReadTokens: 80, cacheWriteTokens: 0, reasoningTokens: 5,
        contextUsed: 100, contextWindow: 200000, peakContext: 100, modelCalls: 1,
        compactions: null, cacheRebuilds: null, costUsd: 0,
      } };
    expect(parseConversationEvent(completion)).toEqual(completion);
    expect(parseConversationEvent({ ...completion, usage, messageId: "reply" })).toMatchObject({ usage, messageId: "reply" });
    for (const invalid of [null, {}, { ...usage, usage: { ...usage.usage, inputTokens: -1 } },
      { ...usage, usage: { ...usage.usage, cacheReadTokens: 101 } },
      { ...usage, usage: { ...usage.usage, costUsd: Infinity } },
      { ...usage, usageByModel: [{ model: "m", usage: { ...usage.usage, outputTokens: -1 } }] }]) {
      const parsed = parseConversationEvent({ ...completion, usage: invalid });
      expect(parsed?.type).toBe("turn.completed");
      expect(parsed).not.toHaveProperty("usage");
    }
  });

  it("discards invalid versions, types and required fields without breaking replay", () => {
    expect(parseConversationEvent({ v: 2, type: "user.message", at: 1, content: [] })).toBeNull();
    expect(parseConversationEvent({ v: 1, type: "provider.surprise", at: 1 })).toBeNull();
    expect(parseConversationEvent({ v: 1, type: "assistant.block", at: 1, messageId: "m", index: -1 })).toBeNull();

    const timeline = new Timeline();
    expect(timeline.push(line({ v: 1, type: "commands.updated", at: 1 }))).toEqual([]);
    expect(timeline.items).toEqual([]);
  });

  it("ignores the legacy mirror retained only for rollback", () => {
    const timeline = new Timeline();
    timeline.push(
      line({
        type: "user",
        prometheusV1Mirror: true,
        message: { role: "user", content: "do not duplicate" },
      }),
    );
    expect(timeline.items).toEqual([]);
  });

  it("reduces canonical events without knowing provider envelopes", () => {
    const timeline = new Timeline();
    timeline.push(line({ v: 1, type: "user.message", at: 1, content: [{ kind: "text", text: "hi" }] }));
    timeline.push(line({ v: 1, type: "assistant.started", at: 2, messageId: "m1" }));
    timeline.push(
      line({ v: 1, type: "assistant.block.started", at: 3, messageId: "m1", index: 0, block: { kind: "text", text: "" } }),
    );
    timeline.push(line({ v: 1, type: "assistant.delta", at: 4, messageId: "m1", index: 0, kind: "text", delta: "hello" }));
    timeline.push(
      line({ v: 1, type: "assistant.block", at: 5, messageId: "m1", index: 0, block: { kind: "text", text: "hello!" } }),
    );
    timeline.push(line({ v: 1, type: "turn.completed", at: 6, outcome: "ok", message: "", durationMs: 10, costUsd: null }));

    expect(timeline.items.map((item) => item.kind)).toEqual(["user", "assistant"]);
    expect(timeline.items[1]).toMatchObject({ kind: "assistant", streaming: false, blocks: [{ kind: "text", text: "hello!" }] });
    expect(timeline.busy).toBe(false);
  });

  it("produces the same timeline whether legacy transcripts are adapted before or during reading", () => {
    const legacy = [
      { type: "control_response", ts: 1, response: { response: { commands: [{ name: "compact", description: "Compact", argumentHint: "" }] } } },
      { type: "system", subtype: "init", ts: 2, terminal_slash_commands: [] },
      { type: "user", ts: 3, message: { content: "do it" } },
      { type: "stream_event", ts: 4, event: { type: "message_start", message: { id: "m1" } } },
      { type: "stream_event", ts: 5, event: { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } } },
      { type: "stream_event", ts: 6, event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "working" } } },
      { type: "assistant", ts: 7, message: { id: "m1", content: [{ type: "text", text: "working" }] } },
      { type: "assistant", ts: 8, message: { id: "m1", content: [{ type: "tool_use", id: "tool-1", name: "Agent", input: { description: "explore" } }] } },
      { type: "control_request", ts: 9, request_id: "request-1", request: { subtype: "can_use_tool", tool_name: "Agent", tool_use_id: "tool-1", input: { description: "explore" } } },
      { type: "system", subtype: "task_started", ts: 10, task_id: "task-1", tool_use_id: "tool-1", description: "explore" },
      { type: "user", ts: 11, message: { content: [{ type: "tool_result", tool_use_id: "tool-1", content: "started" }] } },
      { type: "system", subtype: "background_tasks_changed", ts: 12, tasks: [] },
      { type: "system", subtype: "task_notification", ts: 13, task_id: "task-1", status: "completed", summary: "exploration finished" },
      { type: "system", subtype: "status", ts: 14, status: "compacting" },
      { type: "system", subtype: "status", ts: 15, status: null, compact_result: "success" },
      { type: "system", subtype: "compact_boundary", ts: 16, compact_metadata: { pre_tokens: 100, post_tokens: 20 } },
      { type: "result", ts: 17, is_error: false, duration_ms: 50 },
    ];
    const during = new Timeline();
    const before = new Timeline();
    const adapter = new LegacyConversationAdapter();
    for (const value of legacy) {
      during.push(line(value));
      for (const event of adapter.translate(value)) before.push(line(event));
    }

    expect(before.items).toEqual(during.items);
    expect(before.busy).toBe(during.busy);
    expect(before.compacting).toBe(during.compacting);
    expect([...before.tasks]).toEqual([...during.tasks]);
    expect(before.commands).toEqual(during.commands);
  });
});

it("accepts canonical usage for Antigravity without accepting unknown provider names", () => {
  const event = { v: 1, type: "usage.updated", at: 1, provider: "antigravity", usage: { windows: [] } };
  expect(parseConversationEvent(event)).toEqual(event);
  expect(parseConversationEvent({ ...event, provider: "unknown" })).toBeNull();
});


it("accepts and reduces every event emitted by the Antigravity adapter fixture", () => {
  const fixture: { events: unknown[] } = JSON.parse(antigravityFixture);
  const timeline = new Timeline();
  for (const event of fixture.events) {
    expect(parseConversationEvent(event), JSON.stringify(event)).not.toBeNull();
    timeline.push(JSON.stringify(event));
  }
  const assistants = timeline.items.filter(item => item.kind === "assistant");
  expect(assistants.length).toBeGreaterThan(0);
  expect(assistants.every(item => !item.streaming)).toBe(true);
  expect(assistants.flatMap(item => item.blocks)).toEqual(expect.arrayContaining([
    expect.objectContaining({ kind: "tool", name: "run_command" }),
  ]));
  expect(timeline.items.filter(item => item.kind === "ask")).toHaveLength(0);
});
