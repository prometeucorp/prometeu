---
type: Decision
title: "ADR 0002 — Canonical conversation protocol"
description: "One canonical Prometeu protocol for the events and commands of a conversation."
status: stable
---

# ADR 0002 — Canonical conversation protocol

Date: 2026-09-03

## Context

In the first frontend, a reducer changed the stream-json of Claude into screen
items. The Codex adapter changed JSON-RPC into the same format. Thus, the
timeline, the transcript and the shared sessions used the same code again. This
solution showed that one shared representation is useful.

But the shared format was the format of one provider. The code read it as
dynamic JSON, and it mixed facts of the conversation with information that
only the Claude protocol has. A feature of only one different provider had to copy external
concepts or add its own subtypes.

## Options considered

1. Keep the stream-json of Claude as a permanent contract.
2. Let the frontend know each protocol and change each protocol independently.
3. Make canonical Prometeu events and one adapter for each provider.

## Decision

Use `ConversationEventV1` and `ConversationCommandV1` as versioned internal
contracts. Each provider adapter translates the input and the output at the
edge. The timeline, the storage and the collaboration features use only the
Prometeu contract.

The application can read legacy transcripts, and it does not write them again.
New Prometeu logs contain only V1 events, without a mirror for rollback. The
contract in force is in
[conversation-events-v1.md](../contracts/conversation-events-v1.md).

## Consequences

Positive:

- A new provider does not add conditions to the reducer.
- Events get an explicit parser, types and compatibility.
- Different adapters can use the same conformance tests.
- External changes stay in the fixtures of the adapter.

Negative:

- The legacy reader stays necessary for the current transcripts.
- Each new event makes a decision about the shared meaning necessary.
- A translation can remove information that only one provider has.
- Transcripts, snapshots and live data must have a careful migration.

## Acceptance evidence

- `conversation.test.ts` shows that a replay of the legacy format and a replay
  of V1 give the same result.
- Reducer tests examine streaming, requests, background work and compaction.
- Rust tests examine the translation of stream-json and the V1
  translation that Codex does without an intermediate format.
- The parser and the adapters discard an unknown event and keep the other
  events.
- Codex logs contain V1 events. The replay finds the historical mirror marks
  and shows each conversation item only one time.

## Detail decisions

- An isolated attachment stays out of V1 until there is a transport for bytes.
- The cost stays in the shared event as a nullable field.
- Slash commands stay text that the adapter interprets. The adapter shows the
  available commands explicitly through `commands.list`.
- The timeline shows a notice that the adapter can translate and show. The
  adapter ignores an unknown external type.
- `prometheusV1Mirror` and `type: "prometheus"` stay read-only compatibility
  tokens for the current transcripts. The application does not write them.
