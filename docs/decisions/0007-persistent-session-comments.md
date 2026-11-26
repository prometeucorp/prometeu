---
type: Decision
title: "ADR 0007 — Persistent comments alongside the session"
description: "Comments are threads in a side panel, with an anchor in the conversation and an open or resolved state."
status: stable
---

# ADR 0007 — Persistent comments alongside the session

Date: 2026-09-04

## Context

At first, the application put collaboration notes into the transcript at the
time that the application got them. While the session continued, the notes moved out of the
visible area. Thus, they did not help the team to follow a decision until it
was complete. To write a note, the person also had to change the primary box
from agent mode to note mode. That change hid the destination of the next
message at the most important time of the interaction.

The team does not want one more message in the chat. The team wants to keep a
conversation between persons about a part of the work. It also wants to find
the open items easily, and to show when an item is complete.

## Options considered

1. Keep notes in the transcript and make their visual markers better.
2. Show comments as usual messages that go to the agent.
3. Keep persistent threads in a side panel, with an anchor in the
   conversation and an explicit state.

## Decision

Comments are in their own side panel. The primary box always sends messages to
the agent.

Each thread has a root, zero or more replies and an open or resolved state. A
thread can be general to the tab. It can also store `tab` and `Piece.key` as
the anchor of a part of the conversation. The quote is context for the reader.
It does not identify the thread. Markers in the transcript open the thread, and
the thread can go back to the quoted part.

Mentions make assignments in **Notifications**. When a person opens an
assignment, the work is not complete. Only the resolution of the root removes
the thread from the inbox of all persons. Each collaborator who continues to
have access to the workspace can resolve the thread.

The v4 relay uses `note`, `note_reply` and `note_resolve`, with encrypted
content and `comments: 1` in the welcome. The client keeps a capability
fallback for a relay without threads. In that fallback, the client hides the
replies and the resolution, and an open action completes an inbox entry. That
fallback does not go around the mandatory v4/E2EE handshake. Stored notes
without thread fields become open general roots. The v4 client keeps the v3
storage, but it does not load or change it. See the
[relay contract](../contracts/relay-v4.md).

## Consequences

Positive:

- Open items stay visible while the session becomes longer.
- When a person writes to the team, the destination of the agent box does not
  silently change.
- Replies keep the context, and the resolution closes the item for all persons.
- Notes from before the threads get default thread fields. The client does not
  write the local transcripts again.

Negative:

- The relay stores the thread state and the replies.
- An anchor can become unavailable after the retention period or after a change
  to the transcript. In that condition, the quote stays visible.
- v3 comments are not available in the v4 client.
- The resolution does not have more roles. Access to the workspace is the
  authority.

## Evidence

- `src/notes.test.ts` examines the groups, the tab, legacy data and the order
  by activity.
- `src/team.test.ts` examines the capability, the creation, the reply, the
  resolution and the inbox.
- `relay/src/protocol.test.ts` and `relay/src/logic.test.ts` examine the
  validation, the storage, the audience, the assignment and the compatibility.
- `e2e/critical-flows.spec.ts` examines the creation from the context, the
  reply, the resolution, and that the primary box stays on the agent.
