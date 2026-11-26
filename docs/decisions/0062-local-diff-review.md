---
type: Decision
title: "ADR 0062 — local review notes and explicit batch submission"
description: "Local review notes that the person sends together in one explicit submission."
status: stable
---

# ADR 0062 — local review notes and explicit batch submission

Date: 2026-09-26

## Context

Before this decision, a person read a diff and told an agent to change it. To
do this, the person had to copy the locations into the composer manually. Notes
must stay after the app starts again. They must follow the code carefully. They
must stay private until the person sends them. They are not the shared human
comments that ADR 0007 controls.

## Options considered

- Backend files and new IPC. The ownership is stronger, and external readers
  can use the files in a subsequent version. But the first version becomes
  larger than the current review workflow in the frontend.
- Temporary notes. This is simple, but the text of the person is not kept when
  the app starts again.
- localStorage with protection, together with a versioned envelope in the message
  text. This keeps the current transports. It also uses the portable
  presentation pattern of ADR 0038 again.

## Decision

Use versioned localStorage, with a limit on the quantity of data, for the notes
that wait. Usually the backend owns saved data, but this decision explicitly
does not obey that rule. These rules
apply:

- If a record is not correct or is unknown, Prometeu does not write.
- If a failure occurs during a save, Prometeu keeps the notes in memory and shows an error.
- An origin-wide Web Lock puts changes and removals in sequence across windows.
  Prometeu reads the data again under the lock. It rejects expired revisions of
  a note.

This keeps the v1 record without a backend or an operation log. If the runtime
does not have Web Locks, Prometeu does not write. A write in memory that has a failure
cannot replace changes on the disk from a different window. Do not put the storage
in the isolated components.

Use initial anchors that do not change. Calculate the position for each scope
from them. Keep the evidence of the full selection, with a limit on its length, apart
from the short quotations. Prometeu finds a note position only when the code
agrees accurately. The interface shows ambiguity, missing code and missing files. Prometeu does not resolve notes automatically.

Send a snapshot of one revision, which does not change, to the draft of a
selected conversation as one tag. Only an explicit submission changes the
snapshot into `<prometeu-review v="1">` in usual text. If the person edits a
note after the snapshot, the older snapshot cannot send that edit. The
"submitted" state records the action of the user. It does not tell that the
provider received the notes. The current queue can stay after a failure
when the process starts again. Keep the notes, and let the person try again or open them
again explicitly. The [Local diff review](../contracts/diff-review.md) contract
gives more information.

## Consequences

V1, IPC and the relay do not change. The same text operates for all the
providers that Prometeu supports. Older clients show the text without changes.
The content that the person sent follows the rules for shared transcripts. Notes that the
person did not send have no collaboration channel. The storage of the browser
origin has no copy in the backend, and it can become full. A subsequent
migration to the backend must keep this version and the guarantees for write
failures. It must not discard the data without a message.

## Evidence

`review-comments.test.ts`, `review-context.test.ts`, `review-store.test.ts`, the
Git keyboard journey and the mobile journey with mixed context examine these
boundaries.
