---
type: Decision
title: "ADR 0038 — selected elements as tags in the conversation"
description: "Selected elements as tags in the conversation."
status: stable
---

# ADR 0038 — selected elements as tags in the conversation

Date: 2026-09-10

## Context

The browser from ADR 0037 added HTML, CSS and geometry directly to the editable
text. That JSON took over the composer and the history, making it harder to
write the request. The context must stay complete for the agent and in the
replay.

## Options considered

- Introduce a content type in the conversation protocol: it requires changing
  adapters, the queue, persistence and collaboration for a presentation change.
- Hide the data only in memory: it loses the presentation after a replay and
  prevents peers from recognizing the context the agent received.
- Keep tags in the draft and use an identified textual block on send: it
  preserves the existing transports and allows a compact presentation in the
  replay.

## Decision

Keep elements and their captures in the per-conversation draft, separate from
the text. Show removable tags, with details reachable through a button. On send,
serialize each element in the versioned textual format defined in the
[browser contract](../contracts/browser.md#context-in-the-message-text).

Only the presentation recognizes those blocks. The reducer, the adapters and the
relay neither remove nor reinterpret them. Strict reading bounds the structure,
size and types; invalid blocks stay literal. Page content is always displayed as
text, without executing HTML and without reading local paths from the history.

## Consequences

Desktop, desk and phone show tags for new messages without extending V1 or IPC.
Previous clients keep showing the complete block. Old transcripts are not
rewritten. Removing a tag also removes the associated capture from the send;
standalone captures are still ordinary attachments.

## Evidence

- `src/browser-context.test.ts`: roundtrip and fallback for invalid content.
- `e2e/browser.spec.ts`: draft, removal, sent content, replay and the persisted
  queue.
- `e2e/mobile.spec.ts`: tags and details in the shared history.
