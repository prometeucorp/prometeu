import { parseContext, type Report } from "./context";
import {
  parseConversationEvent,
  type AnyConversationEventV1,
  type AssistantBlock,
  type BackgroundTask,
  type InputContent,
  type SlashCommand,
} from "./conversation";
import { LegacyConversationAdapter } from "./conversation-legacy";
import type { ConversationUsage as TelemetryMeasurement, TurnMeasurement } from "./conversation";

export type TurnMetadata = { durationMs: number | null; usage?: TelemetryMeasurement };
type ContextState = { used: number; window: number | null };

export type ToolBlock = {
  kind: "tool";
  id: string;
  name: string;
  input: unknown;
  json: string;
  result: string | null;
  error: boolean;
  done: boolean;
  background: boolean;
};

export type Task = { id: string; description: string; toolUseId: string | null };
export type Block = { kind: "text"; text: string } | { kind: "thinking"; text: string } | ToolBlock;

export type Ask = {
  kind: "ask";
  requestKind: Extract<AnyConversationEventV1, { type: "request.opened" }>["kind"];
  ts: number;
  id: string;
  tool: string;
  input: Record<string, unknown>;
  toolUseId: string | null;
  answered: boolean;
};

export type Item =
  | { kind: "user"; ts: number; text: string }
  | { kind: "assistant"; ts: number; msg: string; blocks: Block[]; streaming: boolean; next: number; completion?: TurnMetadata }
  | Ask
  | { kind: "result"; ts: number; error: boolean; text: string; cost: number | null; ms: number | null }
  | { kind: "system"; ts: number; text: string; error: boolean; what?: "compacted" | "summary" | "stderr" | "background"; tokens?: [number, number]; trigger?: "auto" | "manual" }
  | { kind: "context"; ts: number; report: Report };

export type Command = SlashCommand;

/// Derive presentation state exclusively from the conversation contract. The legacy adapter only replays pre-V1 transcripts.
export class Timeline {
  items: Item[] = [];
  busy = false;
  compacting = false;
  tasks = new Map<string, Task>();
  commands: Command[] = [];
  context: ContextState | null = null;

  /// `busy` is the primary turn; background tasks can outlive it. The conversation is only idle
  /// when both have finished, so the person can still interrupt work started by a finished turn.
  get working(): boolean {
    return this.busy || this.tasks.size > 0;
  }

  private legacy = new LegacyConversationAdapter();
  private tools = new Map<string, { item: number; block: number }>();
  private settled: number[] = [];
  private restored = new Map<number, { before: TurnMetadata | undefined; current: TurnMetadata }>();
  private contextPosition = -1;
  private restoredContext?: { before: ContextState | null; position: number; current: ContextState };

  get pending(): Ask[] {
    return this.items.filter((item): item is Ask => item.kind === "ask" && !item.answered);
  }

  load(text: string, now = Date.now()) {
    for (const line of text.split("\n")) {
      if (line.trim()) this.push(line, now);
    }
  }

  /** Replay lookups use the provider's final assistant identity, never turn position or time. */
  restoreUsage(rows: TurnMeasurement[]): number[] {
    const touched: number[] = [];
    const locations = new Map<string, number>();
    this.items.forEach((item, index) => {
      if (item.kind === "assistant") locations.set(item.msg, locations.has(item.msg) ? -1 : index);
    });
    for (const row of rows) {
      const at = locations.get(row.messageId) ?? -1;
      const item = this.items[at];
      if (item?.kind !== "assistant" || item.streaming || item.completion?.usage) continue;
      const before = item.completion;
      item.completion = { durationMs: row.durationMs ?? before?.durationMs ?? null, usage: row.usage };
      this.restored.set(at, { before, current: item.completion });
      this.measuredContext(row.usage, at, true);
      touched.push(at);
    }
    return touched;
  }

  /** History deletion removes only the values restored from the local ledger. */
  clearRestoredUsage(): number[] {
    const touched: number[] = [];
    for (const [at, restored] of this.restored) {
      const item = this.items[at];
      if (item?.kind !== "assistant" || item.completion !== restored.current) continue;
      item.completion = restored.before; touched.push(at);
    }
    this.restored.clear();
    if (this.restoredContext && this.context === this.restoredContext.current) {
      this.context = this.restoredContext.before; this.contextPosition = this.restoredContext.position;
    }
    this.restoredContext = undefined;
    return touched;
  }

  private measuredContext(measurement: TelemetryMeasurement, at: number, restored = false) {
    const { contextUsed: used, contextWindow: capacity } = measurement.usage;
    if (used === null || (restored && at < this.contextPosition)) return;
    const current = { used, window: capacity };
    if (restored) {
      const previous = this.restoredContext && this.context === this.restoredContext.current ? this.restoredContext : { before: this.context, position: this.contextPosition };
      this.restoredContext = { ...previous, current };
    } else this.restoredContext = undefined;
    this.context = current; this.contextPosition = at;
  }

  push(line: string, now = Date.now()): number[] {
    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch {
      return [];
    }
    const canonical = parseConversationEvent(value);
    const events = canonical ? [canonical] : this.legacy.translate(value, now);
    this.settled = [];
    const touched = events.flatMap((event) => this.reduce(event));
    return [...new Set([...this.settled, ...touched])];
  }

  private reduce(event: AnyConversationEventV1): number[] {
    switch (event.type) {
      case "user.message": {
        const text = inputText(event.content);
        if (!text.trim()) return [];
        this.busy = true;
        return [this.add({ kind: "user", ts: event.at, text })];
      }
      case "assistant.started": {
        if (this.findAssistant(event.messageId) !== -1) return [];
        this.busy = true;
        return [
          this.add({
            kind: "assistant",
            ts: event.at,
            msg: event.messageId,
            blocks: [],
            streaming: true,
            next: 0,
          }),
        ];
      }
      case "assistant.block.started":
        return this.startBlock(event.messageId, event.index, event.block);
      case "assistant.delta":
        return this.appendDelta(event.messageId, event.index, event.kind, event.delta);
      case "tool.input.delta":
        return this.appendToolInput(event.messageId, event.index, event.delta);
      case "assistant.block":
        return this.commitBlock(event.messageId, event.index, event.block, event.at);
      case "tool.completed":
        return this.completeTool(event.toolId, event.output, event.error, event.background);
      case "request.opened":
        return this.openRequest(event);
      case "request.closed":
        return this.answer(event.requestId);
      case "turn.completed":
        return this.completeTurn(event);
      case "context.compaction":
        this.compacting = event.state === "started";
        return event.state === "failed"
          ? [this.add({ kind: "system", ts: event.at, text: event.detail || "compact failed", error: true })]
          : [];
      case "context.compacted": {
        this.compacting = false;
        if (event.after !== null) {
          this.context = { used: event.after, window: this.context?.window ?? null };
          this.contextPosition = this.items.length; this.restoredContext = undefined;
        }
        const tokens: [number, number] | undefined =
          event.before !== null && event.after !== null ? [event.before, event.after] : undefined;
        return [
          this.add({
            kind: "system",
            ts: event.at,
            text: "compacted",
            error: false,
            what: "compacted",
            tokens,
            ...(event.trigger ? { trigger: event.trigger } : {}),
          }),
        ];
      }
      case "background.changed":
        return this.changeBackground(event.tasks);
      case "system.notice": {
        if (!event.detail) return [];
        const error = event.level === "error";
        if (event.code === "provider.stderr") {
          const index = this.items.length - 1;
          const previous = this.items[index];
          if (previous?.kind === "system" && previous.what === "stderr" && previous.error === error) {
            previous.text += `\n${event.detail}`;
            return [index];
          }
        }
        return [
          this.add({
            kind: "system",
            ts: event.at,
            text: event.detail,
            error,
            what: event.code === "provider.stderr" ? "stderr" : event.code === "background.completed" ? "background" : undefined,
          }),
        ];
      }
      case "system.summary":
        return [this.add({ kind: "system", ts: event.at, text: event.text, error: false, what: "summary" })];
      case "context.reported": {
        const report = parseContext(event.markdown);
        return report ? [this.add({ kind: "context", ts: event.at, report })] : [];
      }
      case "session.state":
        if (event.state === "busy" || event.state === "waiting") {
          this.busy = true;
          return [];
        }
        return event.state === "ready" || event.state === "stopped" ? this.idle() : [];
      case "commands.updated":
        this.commands = event.commands;
        return [];
      case "context.updated":
        this.context = { used: event.used, window: event.window };
        this.contextPosition = this.items.length; this.restoredContext = undefined;
        return [];
      case "session.identity":
      case "usage.updated":
        return [];
    }
  }

  private add(item: Item): number {
    if (item.kind === "user" || item.kind === "assistant") this.settle();
    this.items.push(item);
    return this.items.length - 1;
  }

  private settle() {
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i];
      if (item.kind === "assistant" && item.streaming) {
        item.streaming = false;
        this.settled.push(i);
      } else if (item.kind === "assistant") {
        break;
      }
    }
  }

  private idle(): number[] {
    this.busy = false;
    this.compacting = false;
    const touched: number[] = [];
    this.items.forEach((item, index) => {
      if (item.kind === "assistant" && item.streaming) {
        item.streaming = false;
        touched.push(index);
      }
      if (item.kind === "ask" && !item.answered) {
        item.answered = true;
        touched.push(index);
      }
    });
    return touched;
  }

  private findAssistant(messageId: string): number {
    for (let index = this.items.length - 1; index >= 0; index--) {
      const item = this.items[index];
      if (item.kind === "assistant") return item.msg === messageId ? index : -1;
      if (item.kind === "user" || item.kind === "result") return -1;
    }
    return -1;
  }

  private assistant(messageId: string, at: number): { index: number; item: Extract<Item, { kind: "assistant" }> } {
    let index = this.findAssistant(messageId);
    if (index === -1) {
      index = this.add({
        kind: "assistant",
        ts: at,
        msg: messageId,
        blocks: [],
        streaming: true,
        next: 0,
      });
    }
    return { index, item: this.items[index] as Extract<Item, { kind: "assistant" }> };
  }

  private startBlock(messageId: string, index: number, source: AssistantBlock): number[] {
    const found = this.findAssistant(messageId);
    if (found === -1) return [];
    const item = this.items[found];
    if (item.kind !== "assistant" || !item.streaming || index < item.next) return [];
    const block = viewBlock(source);
    item.blocks[index] = block;
    if (block.kind === "tool") this.tools.set(block.id, { item: found, block: index });
    this.busy = true;
    return [found];
  }

  private appendDelta(messageId: string, index: number, kind: "text" | "thinking", delta: string): number[] {
    const found = this.findAssistant(messageId);
    if (found === -1) return [];
    const item = this.items[found];
    if (item.kind !== "assistant" || !item.streaming || index < item.next) return [];
    const block = item.blocks[index];
    if (!block || block.kind !== kind) return [];
    block.text += delta;
    return [found];
  }

  private appendToolInput(messageId: string, index: number, delta: string): number[] {
    const found = this.findAssistant(messageId);
    if (found === -1) return [];
    const item = this.items[found];
    if (item.kind !== "assistant" || !item.streaming || index < item.next) return [];
    const block = item.blocks[index];
    if (!block || block.kind !== "tool") return [];
    block.json += delta;
    block.input = tryJson(block.json) ?? block.input;
    return [found];
  }

  private commitBlock(messageId: string, index: number, source: AssistantBlock, at: number): number[] {
    const found = this.assistant(messageId, at);
    const block = viewBlock(source);
    const draft = found.item.blocks[index];
    if (block.kind === "thinking" && !block.text && draft?.kind === "thinking") block.text = draft.text;
    found.item.blocks[index] = block;
    found.item.next = Math.max(found.item.next, index + 1);
    found.item.streaming = true;
    if (block.kind === "tool") this.tools.set(block.id, { item: found.index, block: index });
    this.busy = true;
    return [found.index];
  }

  private completeTool(toolId: string, output: string, error: boolean, background: boolean): number[] {
    const location = this.tools.get(toolId);
    if (!location) return [];
    const item = this.items[location.item];
    if (item.kind !== "assistant") return [];
    const tool = item.blocks[location.block];
    if (!tool || tool.kind !== "tool") return [];
    tool.result = output;
    tool.error = error;
    tool.done = true;
    tool.background = background;
    const touched = [location.item];
    this.items.forEach((candidate, index) => {
      if (candidate.kind === "ask" && candidate.toolUseId === toolId && !candidate.answered) {
        candidate.answered = true;
        touched.push(index);
      }
    });
    return touched;
  }

  private openRequest(event: Extract<AnyConversationEventV1, { type: "request.opened" }>): number[] {
    if (this.items.some((item) => item.kind === "ask" && item.id === event.requestId)) return [];
    this.busy = true;
    return [
      this.add({
        kind: "ask",
        requestKind: event.kind,
        ts: event.at,
        id: event.requestId,
        tool: event.tool ?? "",
        input: event.input,
        toolUseId: event.toolId,
        answered: false,
      }),
    ];
  }

  answer(id: string): number[] {
    const index = this.items.findIndex((item) => item.kind === "ask" && item.id === id);
    if (index === -1) return [];
    const ask = this.items[index] as Ask;
    if (ask.answered) return [];
    ask.answered = true;
    return [index];
  }

  private completeTurn(event: Extract<AnyConversationEventV1, { type: "turn.completed" }>): number[] {
    this.busy = false;
    this.compacting = false;
    // An interruption ends the children the turn started, whether or not the provider reports it.
    const touched: number[] = event.outcome === "interrupted" ? this.changeBackground([]) : [];
    let finalAssistant = false;
    for (let index = this.items.length - 1; index >= 0; index--) {
      const item = this.items[index];
      if (item.kind === "assistant" && !finalAssistant) {
        finalAssistant = true;
        if (!event.messageId || event.messageId === item.msg) {
          item.completion = { durationMs: event.durationMs, ...(event.usage ? { usage: event.usage } : {}) };
          if (event.usage) this.measuredContext(event.usage, index);
          touched.push(index);
        }
      }
      if (item.kind === "assistant" && item.streaming) {
        item.streaming = false;
        touched.push(index);
      }
      if (item.kind === "ask" && !item.answered) {
        item.answered = true;
        touched.push(index);
      }
      if (item.kind === "user") break;
    }
    if (event.outcome !== "ok" || event.message) {
      touched.push(
        this.add({
          kind: "result",
          ts: event.at,
          error: event.outcome !== "ok",
          text: event.message,
          cost: event.costUsd,
          ms: event.durationMs,
        }),
      );
    }
    return touched;
  }

  private changeBackground(tasks: BackgroundTask[]): number[] {
    const next = new Map(
      tasks.map((task) => [
        task.id,
        { id: task.id, description: task.description, toolUseId: task.toolId },
      ]),
    );
    const touched: number[] = [];
    for (const [id, task] of this.tasks) {
      if (!next.has(id)) touched.push(...this.mark(task.toolUseId, false));
    }
    for (const task of next.values()) touched.push(...this.mark(task.toolUseId, true));
    this.tasks = next;
    return [...new Set(touched)];
  }

  private mark(toolId: string | null, background: boolean): number[] {
    const location = toolId ? this.tools.get(toolId) : undefined;
    if (!location) return [];
    const item = this.items[location.item];
    if (item.kind !== "assistant") return [];
    const tool = item.blocks[location.block];
    if (!tool || tool.kind !== "tool" || tool.background === background) return [];
    tool.background = background;
    return [location.item];
  }
}

function inputText(content: InputContent[]): string {
  return content
    .map((part) => {
      if (part.kind === "text") return part.text;
      if (part.kind === "image") return "[imagem]";
      return "[arquivo: " + part.name + "]";
    })
    .filter(Boolean)
    .join("\n\n");
}

function viewBlock(block: AssistantBlock): Block {
  switch (block.kind) {
    case "text":
      return { kind: "text", text: block.text };
    case "thinking":
      return { kind: "thinking", text: block.text };
    case "tool":
      return {
        kind: "tool",
        id: block.id,
        name: block.name,
        input: block.input,
        json: "",
        result: null,
        error: false,
        done: false,
        background: false,
      };
  }
}
function tryJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/// Summarize tool input for collapsed cards using the command, file, or search pattern, matching backend activity.
export function summary(_name: string, input: unknown, json = ""): string {
  const i = (input ?? {}) as Record<string, unknown>;
  const pick = SUMMARY_KEYS.map((k) => i[k]).find((v) => typeof v === "string" && v.trim()) as string | undefined;
  if (pick) return pick.split("\n")[0];
  // Partial input may expose a readable string before its JSON finishes streaming.
  for (const k of SUMMARY_KEYS) {
    const m = new RegExp(`"${k}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)`).exec(json);
    if (m?.[1]) return m[1].replace(/\\n[\s\S]*/, "").replace(/\\(.)/g, "$1");
  }
  return "";
}

const SUMMARY_KEYS = ["command", "file_path", "pattern", "path", "url", "query", "skill", "description", "prompt"];

/// Collect recently read or written files without duplicates for @ completion. Exclude tools whose path denotes a search directory rather than a file.
export function touched(items: Item[], most = 12): string[] {
  const out: string[] = [];
  for (let at = items.length - 1; at >= 0 && out.length < most; at--) {
    const item = items[at];
    if (item.kind !== "assistant") continue;
    for (let k = item.blocks.length - 1; k >= 0 && out.length < most; k--) {
      const block = item.blocks[k];
      if (block.kind !== "tool" || !FILE_TOOLS.has(block.name)) continue;
      const file = (block.input as Record<string, unknown> | null)?.["file_path"];
      if (typeof file === "string" && file && !out.includes(file)) out.push(file);
    }
  }
  return out;
}
const FILE_TOOLS = new Set(["Read", "Edit", "Write", "NotebookEdit", "MultiEdit"]);

/* Conversation display pieces. */

/// Locate a block by item and block index.
export type BlockRef = { at: number; block: number };

/// Group consecutive agent work into one piece while keeping speech, user input, requests, and turn results separate. Stable starting-item keys preserve DOM identity, selection, and collapsed state across append-only updates.
export type Piece =
  | { kind: "item"; key: string; at: number }
  | { kind: "say"; key: string; at: number; block: number }
  | { kind: "work"; key: string; refs: BlockRef[] };

/** Only the final speech of a settled turn owns its usage and copy footer. */
export function turnMetadata(items: Item[], piece: Extract<Piece, { kind: "say" }>): TurnMetadata | null {
  const item = items[piece.at];
  if (item?.kind !== "assistant" || item.streaming || piece.block !== item.blocks.length - 1) return null;
  for (let i = piece.at + 1; i < items.length; i++) {
    if (items[i].kind === "assistant") return null;
    if (items[i].kind === "user") break;
  }
  if (item.completion?.durationMs !== null && item.completion?.durationMs !== undefined) return item.completion;
  for (let i = piece.at - 1; i >= 0; i--) {
    if (items[i].kind === "user") return { ...item.completion, durationMs: Math.max(0, item.ts - items[i].ts) };
  }
  return item.completion ?? null;
}

export function pieces(items: Item[]): Piece[] {
  const out: Piece[] = [];
  let work: Extract<Piece, { kind: "work" }> | null = null;
  items.forEach((item, at) => {
    if (item.kind !== "assistant") {
      work = null;
      out.push({ kind: "item", key: `i${at}`, at });
      return;
    }
    item.blocks.forEach((block, k) => {
      if (!block) return;
      if (block.kind === "text") {
        work = null;
        out.push({ kind: "say", key: `s${at}.${k}`, at, block: k });
        return;
      }
      if (!work) out.push((work = { kind: "work", key: `w${at}.${k}`, refs: [] }));
      work.refs.push({ at, block: k });
    });
  });
  return out;
}
