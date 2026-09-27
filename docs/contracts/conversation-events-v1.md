# Conversation Events v1

Status: contract in force since 2026-09-03.

Claude, Codex and Antigravity have different external protocols. The adapters translate them
into events and commands owned by Prometeu before the buffer, IPC, new
persistence or collaboration. The contract's executable sources are
`src/conversation.ts` and `src-tauri/src/conversation.rs`.

## Envelope and transport

Each event is a JSON object on a single line:

```ts
type EventBase<T extends string> = {
  v: 1;
  type: T;
  at: number; // Unix time in milliseconds
};
```

Session and sequence are not in the event. IPC and the relay carry the line in
the existing transport envelope: `[session, line, seq]`. The sequence joins the
snapshot and the live stream, but it is not a durable identity.

Unknown fields are ignored. An invalid version, type or required field discards
only that line. An unknown type is a no-op and does not end the session.

The [serialization checks](ipc.md#executable-serialization-examples) feed
synthetic Claude/Codex adapter output through the same TypeScript validator and
timeline. The existing Antigravity fixture separately covers recorded NDJSON
translation. Canonical fixtures normalize timestamps and numeric durations;
they do not change production event values or durable sequence semantics.

## Common content

```ts
type InputContent =
  | { kind: "text"; text: string }
  | { kind: "image"; name: string; mediaType: string }
  | { kind: "file"; name: string };

type AssistantBlock =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tool"; id: string; name: string; input: unknown };

type BackgroundTask = {
  id: string;
  description: string;
  toolId: string | null;
};
```

The current contract sends attachments to the provider as path mentions inside
the text. Image and file metadata are reserved for an implementation that
transports attachments separately; bytes and local paths do not enter the shared
transcript by inference.

The mentions open the text as their own paragraph, separated from the rest by a
blank line (`src/mentions.ts`). The presentation reads back only that opening
paragraph, and only when it consists entirely of mentions: images become
numbered tags (`Image #1`, `Image #2`) and other files show their name, with
the path in the tooltip. The transcript and the provider keep the paths as
they are; a mention typed inside the text stays text.

Browser element contexts use an additive textual convention inside `text`,
without a new event or command type. The presentation turns only valid blocks
into tags; the reducer, the adapters and the transcript preserve the complete
string. See the [browser contract](browser.md#context-in-the-message-text).

## Persistent events

```ts
type ConversationEventV1 =
  | (EventBase<"user.message"> & { content: InputContent[] })
  | (EventBase<"assistant.block"> & {
      messageId: string;
      index: number;
      block: AssistantBlock;
    })
  | (EventBase<"tool.completed"> & {
      toolId: string;
      output: string;
      error: boolean;
      background: boolean;
    })
  | (EventBase<"request.opened"> & {
      requestId: string;
      kind: "approval" | "question" | "plan";
      toolId: string | null;
      tool: string | null;
      input: Record<string, unknown>;
    })
  | (EventBase<"request.closed"> & {
      requestId: string;
      outcome: "allowed" | "denied" | "answered" | "cancelled";
    })
  | (EventBase<"turn.completed"> & {
      outcome: "ok" | "error" | "interrupted";
      message: string;
      durationMs: number | null;
      costUsd: number | null;
    })
  | (EventBase<"context.compacted"> & {
      before: number | null;
      after: number | null;
    })
  | (EventBase<"background.changed"> & { tasks: BackgroundTask[] })
  | (EventBase<"system.notice"> & {
      level: "info" | "warning" | "error";
      code: string;
      detail: string;
    })
  | (EventBase<"system.summary"> & { text: string })
  | (EventBase<"context.reported"> & { markdown: string });
```

`assistant.block` is authoritative per `(messageId, index)`. Blocks with the
same `messageId` form one visual message. A result for an unknown tool or the
closing of an unknown request is a no-op; it never breaks the replay.

`request.opened.kind` determines the question, plan or approval interaction and
the accepted response. The timeline preserves it as `Ask.requestKind`; neither
rendering nor remote response validation infers it from `tool`, which may be
unfamiliar or `null`. Legacy transcript adaptation supplies the same canonical
kind. `src/timeline.test.ts`, `e2e/conversation-requests.spec.ts` and the Rust
remote-control tests cover those cases without changing the V1 wire format.

`turn.completed` ends the turn and any visual compaction, but it does not end
background tasks. Consumers treat the conversation as working until the turn has
ended and `background.changed` reports no task; only `interrupted` ends both at
once. A terminal that arrives with tasks still running is held, never dropped. Cost stays in the common event as an optional number: Claude
may fill it in and Codex may use `null` without introducing a provider extension
into the history.

## Ephemeral events

These are not written to the transcript:

```ts
type ConversationEphemeralV1 =
  | (EventBase<"assistant.started"> & { messageId: string })
  | (EventBase<"assistant.block.started"> & {
      messageId: string;
      index: number;
      block: AssistantBlock;
    })
  | (EventBase<"assistant.delta"> & {
      messageId: string;
      index: number;
      kind: "text" | "thinking";
      delta: string;
    })
  | (EventBase<"tool.input.delta"> & {
      messageId: string;
      index: number;
      toolId: string;
      delta: string;
    })
  | (EventBase<"context.compaction"> & {
      state: "started" | "stopped" | "failed";
      detail: string;
    })
  | (EventBase<"context.updated"> & { used: number; window: number | null })
  | (EventBase<"session.state"> & {
      state: "starting" | "ready" | "busy" | "waiting" | "stopped";
    })
  | (EventBase<"session.identity"> & { providerSession: string })
  | (EventBase<"commands.updated"> & {
      commands: Array<{ name: string; description: string; hint: string }>;
    })
  | (EventBase<"usage.updated"> & {
      provider: "claude" | "codex" | "antigravity";
      usage: unknown;
    });
```

Deltas run ahead of the presentation; `assistant.block` replaces the draft of
the same index. Ephemeral events still receive a transport sequence so that the
snapshot and the live stream keep the same order.

In the live local stream, `chat.rs` emits `session.state` with `starting` when
starting a process and with `busy` after the `message.send` write is accepted.
That `busy` precedes `user.message`, local echoes and concurrent responses,
under the same publication lock; a failed write does not emit it. Answers to
requests and provider echoes do not represent another accepted message. The
snapshot may synthesize `busy` or `ready` to present the current state, but it
does not start a new execution in the Dock's pending tracking. These events stay
ephemeral, with no change to the envelope, the persisted format or the
contract's version.
Optional local notifications use these same live events without changing V1.
Notifications and sound start disabled; see the
[notification contract](notifications.md) and
[ADR 0054](../decisions/0054-local-notifications.md).

`usage.updated` identifies the provider because a quota is account information
and the external payloads do not have enough common semantics. That payload goes
straight to the usage adapter, not to the timeline or the transcript.

## Commands

```ts
type ConversationCommandV1 =
  | { v: 1; type: "message.send"; text: string }
  | {
      v: 1;
      type: "request.respond";
      requestId: string;
      response:
        | { outcome: "allow" }
        | { outcome: "deny"; message: string }
        | { outcome: "answer"; answers: Record<string, string> };
    }
  | { v: 1; type: "turn.interrupt" }
  | { v: 1; type: "permission.mode.set"; mode: "bypass" }
  | { v: 1; type: "commands.list" };
```

Slash commands are still `message.send` text: the interpretation belongs to the
adapter, since availability and implementation vary. The autocomplete list uses
`commands.list` and `commands.updated`.

Model, effort, MCP and plugins configure the session outside this contract.
`permission.mode.set` is offered only when the provider's capability allows it.

## Security

- remote control is validated on the Mac that owns the process;
- a remote `request.respond` is valid only for a request open in the buffer;
- approval input is rebuilt from the original request, never accepted from the
  client;
- question answers accept only keys present in the request;
- unrestricted mode cannot be enabled remotely;
- events do not grant filesystem access by themselves.

## Persistence and legacy

Previous transcripts are not rewritten. `LegacyConversationAdapter` translates
old lines during replay, outside the reducer. Claude's transcript still belongs
to the CLI; live events already arrive normalized.

Prometeu writes only the V1 event in the managed log for Codex. The reader still
ignores projections marked with `prometheusV1Mirror` and translates the legacy
`type: "prometheus"` discriminant; those names belong to the previous product's
historical format and are not emitted in new logs. The change in rollback policy
is recorded in ADR 0004.

## Evidence

- `src/conversation.test.ts`: parser, V1 replay and equivalence with the legacy
  format;
- `src/timeline.test.ts`: streaming, tools, requests, background and compaction;
- `claude.rs` tests: stream-json translation, commands and unknown events;
- `conversation.rs` tests: the V1 envelope;
- `codex.rs` tests: V1 commands, the JSON-RPC protocol and direct V1 output;
- `chat.rs` tests: persistence, sequence and safe reconstruction of remote
  control.

## Local telemetry observations

Adapters can attach normalized `telemetry` measurements and nullable
`providerDurationMs` to `turn.completed` before Pump capture. These internal fields
follow the [telemetry contract](telemetry.md); Pump strips them before transcript
persistence, live UI publication and sharing. Existing timeline parsers read the
original completion fields. Claude's `costUsd` now reflects an attributable
per-turn estimate, or null when only an unverified cumulative value exists.

The internal `telemetry.usage` observation uses the same adapter envelope and
Pump ordering gate, but never enters the conversation buffer, transport sequence,
transcript or relay. It is consumed only by the owner's local event store.
Quota `usage.updated` and context `context.updated` retain their existing meanings.
Replay cannot capture new telemetry.
