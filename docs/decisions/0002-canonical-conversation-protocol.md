---
type: Decision
title: "ADR 0002 — Canonical conversation protocol"
description: "Canonical conversation protocol."
status: stable
---

# ADR 0002 — Canonical conversation protocol

Date: 2026-09-03

## Context

The original frontend reduced Claude's stream-json. The Codex adapter converted
JSON-RPC into that same format, allowing the timeline, transcript and sharing to be
reused. The solution proved the usefulness of a common representation.

However, the common format belongs to a vendor, is consumed as dynamic JSON and
mixes conversation facts with Claude protocol details. Features exclusive to
another provider have to imitate external concepts or inject their own subtypes.

## Options considered

1. Keep Claude's stream-json as a permanent contract.
2. Make the frontend know and reduce each protocol separately.
3. Create canonical Prometeu events and per-provider adapters.

## Decision

Adopt `ConversationEventV1` and `ConversationCommandV1` as versioned internal
contracts. Each provider translates input and output at the edge. Timeline,
persistence and collaboration consume only the Prometeu contract.

Legacy transcripts remain readable without rewriting them. New Prometeu logs
contain only V1 events, without a rollback mirror. The contract in force is in
[conversation-events-v1.md](../contracts/conversation-events-v1.md).

## Consequences

Positive:

- a new provider does not require conditionals in the reducer;
- events gain an explicit parser, types and compatibility;
- conformance tests can be shared;
- external changes stay concentrated in the adapter's fixtures.

Negative:

- the legacy reader remains necessary for existing transcripts;
- each new event requires a decision on common semantics;
- translation may lose a provider-specific detail;
- transcripts and snapshot/live need a careful migration.

## Acceptance evidence

- `conversation.test.ts` demonstrates equivalent replay between the legacy
  format and V1;
- reducer tests cover streaming, requests, background and compaction;
- Rust tests cover the stream-json translation and Codex's direct V1
  translation;
- the parser and the adapters discard an unknown event in isolation;
- Codex logs contain V1 events; replay recognizes historical mirror marks
  without duplicating conversation items.

## Detail decisions

- a separate attachment stays outside V1 until real byte transport exists;
- cost stays in the common event as a nullable field;
- slash commands remain text interpreted by the adapter, with explicit discovery
  through `commands.list`;
- a translated, presentable notice appears; an unknown external type is a no-op;
- `prometheusV1Mirror` and `type: "prometheus"` remain read-only compatibility
  tokens for existing transcripts; the app does not emit them.
