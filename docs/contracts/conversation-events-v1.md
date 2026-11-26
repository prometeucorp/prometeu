---
type: Contract
title: "Conversation Events v1"
description: "The canonical protocol that Prometeu owns."
status: stable
---

# Conversation Events v1

This contract is in force from 2026-09-03.

Claude, Codex, Antigravity and Cursor have different external protocols. The
adapters translate them into events and commands that Prometeu owns. They do
this before the buffer, IPC, new storage or collaboration. The executable
sources of the contract are `src/conversation.ts` and
`src-tauri/src/conversation.rs`.

## Envelope and transport

Each event is a JSON object on one line:

```ts
type EventBase<T extends string> = {
  v: 1;
  type: T;
  at: number; // Unix time in milliseconds
};
```

The session and the sequence are not in the event. IPC and the relay send the
line in the current transport envelope: `[session, line, seq]`. The sequence
connects the snapshot and the live stream, but it is not a durable identity.

The reader ignores unknown fields. An incorrect version, type or mandatory
field discards only that line. An unknown type has no effect and does not stop
the session.

The [serialization checks](ipc.md#executable-serialization-examples) send
synthetic Claude and Codex adapter output through the same TypeScript validator
and timeline. The current Antigravity fixture examines the translation of
recorded NDJSON independently. Canonical fixtures normalize timestamps and
numeric time values. They do not change the event values in production or the
meaning of the durable sequence.

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

The current contract sends attachments to the provider as path mentions in the
text. We keep the image and file metadata for a future implementation that sends
attachments independently. Bytes and local paths do not go into the shared
transcript by inference.

The mentions start the text as a paragraph of their own. An empty line divides
them from the remaining text (`src/mentions.ts`). The presentation reads back
only that first paragraph, and only when it contains only mentions. Images
become numbered tags (`Image #1`, `Image #2`). Other files show their name, with
the path in the tooltip. The transcript and the provider keep the paths without
changes. A mention that a person types in the text stays text.

Browser element contexts use an additive text convention in `text`, without a
new event type or command type. The presentation changes only correct blocks
into tags. The reducer, the adapters and the transcript keep the full string.
See the [browser contract](browser.md#context-in-the-message-text).

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
      outcome: "allowed" | "denied" | "answered" | "cancelled" | "resolved";
    })
  | (EventBase<"turn.completed"> & {
      outcome: "ok" | "error" | "interrupted";
      message: string;
      durationMs: number | null;
      costUsd: number | null;
      usage?: ConversationUsage;
      messageId?: string;
    })
  | (EventBase<"context.compacted"> & {
      before: number | null;
      after: number | null;
      trigger?: "auto" | "manual";
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

`assistant.block` is authoritative for each `(messageId, index)`. Blocks with
the same `messageId` make one visual message. A result for an unknown tool or
the close of an unknown request has no effect. It does not stop the replay. An authoritative block revision can come after a subsequent turn. It
updates the current item and keeps its position, timestamp and completion
state. It does not open historical activity again. Stream drafts stay in the
scope of the current response. `src/timeline.test.ts` examines the position
keys in this condition.

`request.opened.kind` sets the interaction (question, plan or approval) and the
accepted response. The timeline keeps it as `Ask.requestKind`. The rendering
and the validation of remote responses do not guess it from `tool`, because
that field can be unknown or `null`. The adaptation of legacy transcripts supplies
the same canonical kind. `src/timeline.test.ts`,
`e2e/conversation-requests.spec.ts` and the Rust remote-control tests examine
those conditions. They do not change the V1 wire format.

A `system.notice` with `code: "background.completed"` reports a completed
background task. Its `detail` is the summary of the task on one line. When a
subagent returns a report, the report follows as Markdown after an empty line.
Adapters decode the XML character codes that Claude adds to the
`<task-notification>` text. The chat shows only the first line of long notices
until a person opens them.

`request.closed.outcome = "resolved"` tells that the provider closed the
request but did not tell the decision. It closes the card and the notification
state. It does not tell if the provider permitted, denied, answered or cancelled
the request. The current outcomes keep their meaning. The parser and replay
fixture in `codex/fixtures/native-request-resolved.json`,
`src/conversation.test.ts` and `src/alert.test.ts` examine this additive value
and legacy closures. It is possible that older renderers reject unknown outcome
values and keep the card until a different event closes it. The backend
continues to reject the expired response ID.

`turn.completed` stops the turn and all visual compaction. But it does not stop
background tasks. Consumers think that the conversation operates until the
turn stops and `background.changed` reports no task. Only `interrupted` stops
the two at the same time. If a last event comes while tasks continue to operate,
the consumer keeps it and does not discard it. The cost stays in the shared
event as an optional number. Claude can supply it, and Codex can use `null`.
Thus, the history gets no extension for one provider only.

The optional `usage` extension uses the measurement shape that the application
owns, from the [telemetry contract](telemetry.md#usage-and-model-attribution).
It contains these items:

- Nullable counts for all input and output. These counts include the cache
  tokens and the reasoning tokens.
- The rows of the models that the adapter saw.
- An explicit coverage and scope.
- The current context.
- A normalized cost for each turn.

`messageId` identifies the last assistant message when the adapter can prove
it. The presentation attaches this metadata only to the last text block of the
completed turn. Previous completions without these two fields keep their
current behavior. The reader ignores optional usage that is incorrect, and it
keeps a correct completion.

`context.compacted.trigger` is optional. It reports automatic or manual
compaction only when the provider supplies that difference. When the field is
missing or unknown, the app does not guess a label. The native transcript
capture records compaction observations as metadata, also when there is no
model turn. `context.compacted` stays in the append-only history. The replay
and late metadata revisions do not open the execution again, do not send a
completion and do not record telemetry again. `context.compaction` stays
temporary.

The current top-level `costUsd` stays a legacy value that the provider reports.
Its scope can be cumulative. The app keeps it for compatibility. The app does
not add it to other values and does not use it as a fallback for the new
footer. Only `usage.usage.costUsd` is an estimate for one turn. An unknown cost
is null. This includes a cost from a stored session that the app cannot examine.
Prometeu has no price table and gives no guarantee about the invoice. `usage.updated` stays temporary
account quota information. It is not related to the stored consumption of a
turn.

Canonical completion usage follows the current encrypted sharing boundary of
the conversation. This includes the CLI estimate that the legacy event
permitted. The app does not publish the local SQLite history, the app-call
origins or the reply lookup to the relay or the Cloud. When a person removes
the local history, the app does not write transcripts again. It also does not
change copies of conversation events that it shared before.

## Ephemeral events

The app does not write these events to the transcript:

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
  | (EventBase<"execution.observed"> & {
      activityId: string;
      state: "working" | "blocked" | "idle" | "unknown";
      scope: "root" | "background";
      blockId: string | null;
      kind: "permission" | "question" | "auth" | null;
      detail: string | null;
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

Deltas come before the presentation. `assistant.block` replaces the draft of the
same index. Temporary events also get a transport sequence. Thus, the snapshot
and the live stream keep the same sequence.

`execution.observed` is a read-only observation of the native execution. It is
not related to assistant content. `activityId` identifies one activity cycle in
the scope of one process. `blockId` identifies one blocked period and is
mandatory only for `blocked`. The two identifiers are not empty and have a
maximum of 256 characters. Other states must have `blockId: null` and
`kind: null`. The optional kind and detail fields tell about facts that the app saw. They
do not make a request, a path for an answer, a tool call or an assistant
message.

Background observations do not open the root turn again. They do not cancel a
completion that the app holds for those same child tasks. `idle` reports
availability, without a completion or a new unread indication. `unknown` reports
that the evidence is not available. It cancels a completion that waits, before the app
publishes an ambiguous removal of background tasks.

The native execution projector accepts only explicit root changes from the
working or blocked state to the done or error state as possible completions. The host must read all of
the native transcript before it commits one. If a canonical completion comes
during that read, it cancels the possible completion. More completion or
status reports cannot open a closed cycle again. A new accepted submission can
open it again. A different native generation that the host proves can also
open it again.

The first activity can come before the related transcript prompt. Its first
accepted input then claims that temporary cycle, which continues to be open,
and keeps the activity evidence. The host must correlate the input or put it
in sequence before the last publication. If a prompt comes after a
closed cycle, the text alone cannot identify it as the earlier prompt. Replay
does not go into this projector or into the notification monitor. These events
do not tell that a completion was correct:

- Clear.
- Idle.
- The eviction of a record.
- A process exit.
- A stop hook that does not prove the end.

When an agent sends `Error` for its `root` record, `ExecutionProjector` sends
`execution.observed` with `root` scope and `idle` state. This event stops the
indication that the primary agent is active, without a `turn.completed` event. The app keeps
transcript data that comes after the error. This data does not start the
indication again. Each active child task keeps its state until it stops.

The `idle` state of the `root` record does not remove or use the pending
`CompletionTicket` for the error.
The agent can try again after its usage limit without a new prompt. A `Working`
report cancels that `CompletionTicket` and starts the indication again. The app
uses the current V1 fields. It does not record a new event in the transcript or
tell the person that the turn completed.

The event adds to V1. Older readers ignore it. It uses a live transport
sequence but no stored transcript position. These tests examine the compatibility, the
scope, the reads with an unsure result and the delivery of one completion only:
`src/conversation.test.ts`, `src/alert.test.ts`, the core session and work
tests, and the protocol tests in `terminal_execution/tests.rs`.
[ADR 0100](../decisions/0100-conversation-transport-identity.md) tells about the
availability of the native terminal and the control channels of each provider.

In the live local stream, `chat.rs` sends `session.state` with `starting` when
it starts a process. It sends the same event with `busy` after the process
accepts the `message.send` write. That `busy` comes before `user.message`, local
echoes and concurrent responses, under the same publication lock. If the write has a
failure, this event does not occur. Answers to requests and provider echoes are not a new accepted
message. The snapshot can make `busy`, `ready` or `stopped` to show the current
state. But it does not start a new execution in the monitor of the Dock.

`stopped` tells that no process operates the conversation. This also applies
after the app starts again. In this condition, the replay shows a turn that the transcript
did not completed as cut off:

- The reply that is not completed has a mark that tells this.
- Open requests close without an answer.
- An `interrupted` notice shows.

`ready` continues to close them as before. The snapshot also puts `stopped`
after each line count in `Tab.cuts`, where the app cut off a turn before the
conversation continued. Older readers read `stopped` as `ready`. These events
stay temporary. The envelope, the stored format and the version of the contract
do not change. Optional local notifications use these same live events and do
not change V1. At the start, the app disables notifications and sound. See the
[notification contract](notifications.md) and
[ADR 0054](../decisions/0054-local-notifications.md).

`usage.updated` identifies the provider, because a quota is account information
and the external payloads do not have sufficient shared meaning. That payload
goes directly to the usage adapter, not to the timeline or the transcript.

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

Slash commands continue to be `message.send` text. The adapter interprets them,
because the availability and the implementation are different for each
provider. The autocomplete list uses `commands.list` and `commands.updated`.

The model, the effort, the MCP servers and the plugins configure the session.
This contract does not include them. The app shows `permission.mode.set` only when the
capability of the provider lets it operate.

## Security

- The Mac that owns the process validates the remote control.
- A remote `request.respond` is correct only for a request that is open in the
  buffer.
- The app makes the approval input again from the initial request. It does not
  accept it from the client.
- Question answers accept only keys that are in the request.
- A remote client cannot enable the mode without permission prompts.
- Events alone do not give access to the file system.

## Persistence and legacy

The app does not write previous transcripts again.
`LegacyConversationAdapter` translates previous lines during the replay,
before the reducer. The CLI continues to own the transcript of Claude. Live events come
normalized.

For Codex, Prometeu writes only the V1 event in the managed log. The reader
continues to ignore projections with the `prometheusV1Mirror` mark. It
translates the legacy `type: "prometheus"` discriminant. Those names come from
the historical format of the previous product, and new logs do not contain them.
ADR 0004 records the change in the rollback policy.

## Evidence

- `src/conversation.test.ts`: the parser, the V1 replay and the equivalence with
  the legacy format.
- `src/timeline.test.ts`: streams, tools, requests, background tasks and
  compaction.
- `claude.rs` tests: the stream-json translation, commands and unknown events.
- `conversation.rs` tests: the V1 envelope.
- `codex.rs` tests: V1 commands, the JSON-RPC protocol and V1 output without a
  translation.
- `chat.rs` tests: storage, sequence and a safe new build of the remote
  control state.

## Local telemetry observations

Adapters can attach normalized `telemetry` measurements and a nullable
`providerDurationMs` to `turn.completed` before the capture of the session output. These internal
fields follow the [telemetry contract](telemetry.md). The capture removes them
before the transcript storage, the live UI publication and the share. The
current timeline parsers read the initial completion fields. At this time, the
`costUsd` of Claude gives an estimate for each turn. It is null when only a
cumulative value that the app cannot examine is available.

The internal `telemetry.usage` observation uses the same adapter envelope and
the same sequence gate of the capture. But it does not go into the
conversation buffer, the transport sequence, the transcript or the relay. Only the local
event store of the owner uses it. The quota event `usage.updated` and the
context event `context.updated` keep their current meanings. Replay cannot
record new telemetry.

Cursor ACP uses the current V1 events for text, the reasoning of the model,
tools, requests and completions. No provider messages go through this boundary.
`src/conversation.test.ts` validates its recorded translation fixture. See
[Cursor](cursor.md).
