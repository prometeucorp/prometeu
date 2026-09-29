import type {
  AnyConversationEventV1,
  AssistantBlock,
  BackgroundTask,
  SlashCommand,
} from "./conversation";

type Line = Record<string, any>;

/// Read transcripts written before V1 for rollback compatibility. Keep legacy stream-json outside Timeline and presentation dependencies.
export class LegacyConversationAdapter {
  private lastTs = 0;
  private message = "";
  private nextBlock = 0;
  private tools = new Map<string, string>();
  private pendingSkill: string | null = null;
  private tasks = new Map<string, BackgroundTask>();
  private commands: SlashCommand[] = [];
  private terminal = new Set<string>();

  translate(value: unknown, now = Date.now()): AnyConversationEventV1[] {
    if (!value || typeof value !== "object") return [];
    const o = value as Line;
    if (o.prometheusV1Mirror || o.isSidechain || o.parent_tool_use_id) return [];
    const at = this.when(o, now);
    switch (o.type) {
      case "user":
        return this.user(o, at);
      case "assistant":
        return this.assistant(o, at);
      case "stream_event":
        return this.stream(o, at);
      case "control_request":
        return this.request(o, at);
      case "control_response":
        return this.commandList(o, at);
      case "result":
        return [this.turn(o, at)];
      case "system":
        return this.system(o, at);
      // Historical discriminator, retained as read-only data for a future import.
      case "prometheus":
        return this.legacyApp(o, at);
      case "rate_limit_event":
        return [this.event("usage.updated", at, { provider: "claude", usage: o.rate_limit_info })];
      default:
        return [];
    }
  }

  private when(o: Line, now: number): number {
    let at = typeof o.ts === "number" ? o.ts : o.timestamp ? Date.parse(o.timestamp) : NaN;
    if (!Number.isFinite(at)) at = this.lastTs || now;
    this.lastTs = Math.max(this.lastTs, at);
    return at;
  }

  private event<T extends AnyConversationEventV1["type"]>(type: T, at: number, data: object) {
    return { v: 1, type, at, ...data } as Extract<AnyConversationEventV1, { type: T }>;
  }

  private user(o: Line, at: number): AnyConversationEventV1[] {
    const content = o.message?.content;
    if (Array.isArray(content)) {
      const events: AnyConversationEventV1[] = [];
      const texts: string[] = [];
      for (const block of content) {
        if (block?.type === "tool_result") {
          const toolId = String(block.tool_use_id ?? "");
          if (!toolId) continue;
          const output = resultText(block.content);
          events.push(
            this.event("tool.completed", at, {
              toolId,
              output,
              error: !!block.is_error,
              background: [...this.tasks.values()].some((task) => task.toolId === toolId),
            }),
          );
          if (this.tools.get(toolId) === "Skill" && !block.is_error) this.pendingSkill = toolId;
        } else if (block?.type === "text" && typeof block.text === "string") {
          texts.push(block.text);
        } else if (block?.type === "image") {
          texts.push("[imagem]");
        }
      }
      if (texts.length) events.push(...this.spoken(texts.join("\n\n"), o, at));
      return events;
    }
    if (typeof content !== "string" || !content.trim()) return [];
    return this.spoken(content, o, at);
  }

  private spoken(text: string, o: Line, at: number): AnyConversationEventV1[] {
    if (this.pendingSkill && (o.isSynthetic || o.isMeta)) {
      const source = typeof o.sourceToolUseID === "string" ? o.sourceToolUseID : null;
      if (!source || source === this.pendingSkill) {
        const toolId = this.pendingSkill;
        this.pendingSkill = null;
        return [this.event("tool.completed", at, { toolId, output: text, error: false, background: false })];
      }
    }
    if (o.isMeta || /^\s*<(command-name|local-command-stdout|local-command-caveat)>/.test(text)) return [];
    if (o.isCompactSummary || text.startsWith("This session is being continued from a previous conversation")) {
      return [this.event("system.summary", at, { text })];
    }
    const task = /^\s*<task-notification>/.test(text) ? taskNotice(text) : null;
    if (task !== null) return [this.event("system.notice", at, { level: "info", code: "background.completed", detail: task })];
    return [this.event("user.message", at, { content: [{ kind: "text", text }] })];
  }

  private assistant(o: Line, at: number): AnyConversationEventV1[] {
    this.pendingSkill = null;
    const messageId = String(o.message?.id ?? o.uuid ?? "");
    if (!messageId) return [];
    if (o.message?.model === "<synthetic>") {
      const markdown = (Array.isArray(o.message?.content) ? o.message.content : []).find(
        (block: Line) => block?.type === "text",
      )?.text;
      return typeof markdown === "string" ? [this.event("context.reported", at, { markdown })] : [];
    }
    this.selectMessage(messageId);
    const out: AnyConversationEventV1[] = [];
    for (const raw of Array.isArray(o.message?.content) ? o.message.content : []) {
      const block = toBlock(raw);
      if (!block) continue;
      if (block.kind === "tool") this.tools.set(block.id, block.name);
      out.push(this.event("assistant.block", at, { messageId, index: this.nextBlock++, block }));
    }
    return out;
  }

  private stream(o: Line, at: number): AnyConversationEventV1[] {
    const raw = o.event;
    if (!raw || typeof raw !== "object") return [];
    if (raw.type === "message_start") {
      const messageId = String(raw.message?.id ?? "");
      if (!messageId) return [];
      this.message = messageId;
      this.nextBlock = 0;
      return [this.event("assistant.started", at, { messageId })];
    }
    const messageId = this.message;
    const index = Number(raw.index);
    if (!messageId || !Number.isInteger(index) || index < 0) return [];
    if (raw.type === "content_block_start") {
      const block = toBlock(raw.content_block);
      if (!block) return [];
      if (block.kind === "tool") this.tools.set(block.id, block.name);
      return [this.event("assistant.block.started", at, { messageId, index, block })];
    }
    if (raw.type !== "content_block_delta") return [];
    const delta = raw.delta ?? {};
    if (delta.type === "text_delta") {
      return [this.event("assistant.delta", at, { messageId, index, kind: "text", delta: String(delta.text ?? "") })];
    }
    if (delta.type === "thinking_delta") {
      return [this.event("assistant.delta", at, { messageId, index, kind: "thinking", delta: String(delta.thinking ?? "") })];
    }
    if (delta.type === "input_json_delta") {
      const tool = raw.tool_use_id ?? "";
      return [this.event("tool.input.delta", at, { messageId, index, toolId: String(tool), delta: String(delta.partial_json ?? "") })];
    }
    return [];
  }

  private selectMessage(messageId: string) {
    if (this.message === messageId) return;
    this.message = messageId;
    this.nextBlock = 0;
  }

  private request(o: Line, at: number): AnyConversationEventV1[] {
    const request = o.request ?? {};
    const requestId = String(o.request_id ?? "");
    if (request.subtype !== "can_use_tool" || !requestId) return [];
    const tool = typeof request.tool_name === "string" ? request.tool_name : null;
    const kind = tool === "AskUserQuestion" ? "question" : tool === "ExitPlanMode" ? "plan" : "approval";
    return [
      this.event("request.opened", at, {
        requestId,
        kind,
        toolId: request.tool_use_id ? String(request.tool_use_id) : null,
        tool,
        input: request.input && typeof request.input === "object" ? request.input : {},
      }),
    ];
  }

  private commandList(o: Line, at: number): AnyConversationEventV1[] {
    const list: unknown = o.response?.response?.commands;
    if (!Array.isArray(list)) return [];
    this.commands = list
      .filter((command): command is Line => !!command && typeof command.name === "string")
      .map((command) => ({
        name: command.name,
        description: String(command.description ?? ""),
        hint: String(command.argumentHint ?? ""),
      }));
    return [this.commandsEvent(at)];
  }

  private turn(o: Line, at: number): AnyConversationEventV1 {
    const error = !!o.is_error;
    const errors: string[] = Array.isArray(o.errors) ? o.errors.map(String) : [];
    const message = errors.length ? errors.join("\n") : error && typeof o.result === "string" ? o.result : "";
    return this.event("turn.completed", at, {
      outcome: error ? (message ? "error" : "interrupted") : "ok",
      message,
      durationMs: typeof o.duration_ms === "number" ? o.duration_ms : null,
      costUsd: typeof o.total_cost_usd === "number" ? o.total_cost_usd : null,
    });
  }

  private system(o: Line, at: number): AnyConversationEventV1[] {
    switch (o.subtype) {
      case "init": {
        const terminal = Array.isArray(o.terminal_slash_commands) ? o.terminal_slash_commands : [];
        this.terminal = new Set(terminal.filter((name): name is string => typeof name === "string"));
        return [this.commandsEvent(at)];
      }
      case "status": {
        const state = o.compact_result === "failed" ? "failed" : o.status === "compacting" ? "started" : "stopped";
        return [this.event("context.compaction", at, { state, detail: String(o.compact_error ?? "") })];
      }
      case "compact_boundary": {
        const metadata = o.compact_metadata ?? {};
        return [
          this.event("context.compacted", at, {
            before: typeof metadata.pre_tokens === "number" ? metadata.pre_tokens : null,
            after: typeof metadata.post_tokens === "number" ? metadata.post_tokens : null,
          }),
        ];
      }
      case "task_started": {
        const id = String(o.task_id ?? "");
        if (!id) return [];
        this.tasks.set(id, {
          id,
          description: String(o.description ?? ""),
          toolId: typeof o.tool_use_id === "string" ? o.tool_use_id : null,
        });
        return [this.background(at)];
      }
      case "background_tasks_changed": {
        const next = new Map<string, BackgroundTask>();
        for (const raw of Array.isArray(o.tasks) ? o.tasks : []) {
          const id = String(raw?.task_id ?? "");
          if (!id) continue;
          next.set(id, this.tasks.get(id) ?? { id, description: String(raw.description ?? ""), toolId: null });
        }
        this.tasks = next;
        return [this.background(at)];
      }
      case "task_notification": {
        const id = String(o.task_id ?? "");
        this.tasks.delete(id);
        const output: AnyConversationEventV1[] = [this.background(at)];
        const detail = String(o.summary ?? "").trim();
        if (detail) output.push(this.event("system.notice", at, { level: o.status === "completed" ? "info" : "error", code: "background.completed", detail }));
        return output;
      }
      default:
        return [];
    }
  }

  private legacyApp(o: Line, at: number): AnyConversationEventV1[] {
    switch (o.subtype) {
      case "stderr":
        return [this.event("system.notice", at, { level: "error", code: "provider.stderr", detail: String(o.text ?? "") })];
      case "state":
        return [this.event("session.state", at, { state: o.busy ? "busy" : "ready" })];
      case "tokens":
        return [this.event("context.updated", at, { used: Number(o.tokens) || 0, window: null })];
      case "session":
        return [this.event("session.identity", at, { providerSession: String(o.session ?? "") })];
      case "usage":
        return [this.event("usage.updated", at, { provider: "codex", usage: o.usage })];
      default:
        return [];
    }
  }

  private commandsEvent(at: number) {
    return this.event("commands.updated", at, {
      commands: this.commands.filter((command) => !this.terminal.has(command.name)),
    });
  }

  private background(at: number) {
    return this.event("background.changed", at, { tasks: [...this.tasks.values()] });
  }
}

function toBlock(raw: Line | undefined): AssistantBlock | null {
  if (!raw || typeof raw !== "object") return null;
  switch (raw.type) {
    case "text":
      return { kind: "text", text: String(raw.text ?? "") };
    case "thinking":
      return { kind: "thinking", text: String(raw.thinking ?? "") };
    case "tool_use":
      return { kind: "tool", id: String(raw.id ?? ""), name: String(raw.name ?? ""), input: raw.input ?? {} };
    default:
      return null;
  }
}

/// Claude escapes the notification's XML text; a subagent's report follows its one-line summary in `<result>`.
function taskNotice(text: string): string | null {
  const summary = /<summary>([\s\S]*?)<\/summary>/.exec(text);
  if (!summary) return null;
  const result = /<result>([\s\S]*)<\/result>/.exec(text);
  const report = result ? unescape(result[1].trim()) : "";
  return [unescape(summary[1].trim()), report].filter(Boolean).join("\n\n");
}

function unescape(text: string): string {
  return text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&(#39|apos);/g, "'").replace(/&amp;/g, "&");
}

function resultText(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => (block?.type === "text" ? String(block.text ?? "") : block?.type === "image" ? "[imagem]" : ""))
    .filter(Boolean)
    .join("\n");
}
