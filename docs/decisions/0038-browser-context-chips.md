---
type: Decision
title: "ADR 0038 — selected elements as tags in the conversation"
description: "The draft shows selected page elements as tags, and the message sends them in a versioned text block."
status: stable
---

# ADR 0038 — selected elements as tags in the conversation

Date: 2026-09-10

## Context

The browser of ADR 0037 added HTML, CSS and geometry directly to the editable
text. That JSON filled the composer and the history. Thus, it was not easy to write
the request. The agent must get the full context, and the replay must
also show the full context.

## Options considered

- Add a content type to the conversation protocol. This makes changes necessary
  in the adapters, the queue, the storage and the collaboration, only for a
  change to the presentation.
- Keep the data hidden only in memory. The presentation then stops after a
  replay. Also, peers cannot identify the context that the agent got.
- Keep tags in the draft and send an identified text block. This keeps the
  current transports and lets the replay show a compact presentation.

## Decision

Keep the elements and their captures in the draft of each conversation,
apart from the text. Show tags that the person can remove. A button shows the
full data. When the person sends the message, serialize each element in the
versioned text format of the
[browser contract](../contracts/browser.md#context-in-the-message-text).

Only the presentation identifies those blocks. The reducer, the adapters and the
relay do not remove them or change their meaning. A strict parser puts a limit on
the structure, the dimension and the types. An incorrect block stays as literal
text. The application always shows page content as text. It does not operate
HTML, and it does not read local paths from the history.

## Consequences

The desktop, the desk and the phone show tags for new messages without a change
to V1 or IPC. Previous clients continue to show the full block. The application
does not write the transcripts of a previous version again. When a person removes a tag, the
application also removes the related capture from the message. A capture
without an element continues to be a usual attachment.

## Evidence

- `src/browser-context.test.ts`: a roundtrip and the fallback for incorrect
  content.
- `e2e/browser.spec.ts`: the draft, the removal, the sent content, the replay and
  the stored queue.
- `e2e/mobile.spec.ts`: tags and their full data in the shared history.
