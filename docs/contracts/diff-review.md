---
type: Contract
title: "Local diff review notes"
description: "Notes on a diff, their position, their storage and the explicit submission of a batch."
status: stable
---

# Local diff review notes

The implementation is complete. See [ADR 0062](../decisions/0062-local-diff-review.md).

## Ownership and scope

Review notes are local text that a person writes for an agent. They are
different from the Reviewed marks of Git and from the transcript comments
between persons. A person can write notes in Local files, Stage and Compare in
a workspace of the owner that the person can use. The notes apply to text
hunks in the unified layout and in the split layout (two columns). These items do not have a
control to write notes:

- History and merge conflicts.
- Remote workspaces.
- Binary patches and patches that change only metadata.
- Rows after the limit of 2,500 rendered rows.

A file note also must have a text patch.

`review-comments.ts` owns the pure rules for the selection, the position and
the draft snapshots. `review-store.ts` owns the local storage.
`workspace-review.ts` connects those rules with the Git reads, the dialogs and the
callbacks from Workspace Changes. The components in `components/git/` receive
data and callbacks. They do not receive storage or IPC.

The line-number button selects a line. Shift-click or a drag makes the
selection larger in one hunk. When the diff has the focus, these keys operate:

- The arrow keys move the selection.
- Shift and an arrow key make the selection larger.
- C or Enter opens the editor.
- Escape removes the selection or cancels the edit.
- Left and Right select the side of the previous version or of the new version
  in the split layout.
- Command-Enter or Control-Enter saves the note.

Saved notes show in the diff, in the file counts and in a summary of the
workspace. Each saved note has the edit, delete, resolve and reopen actions.
A person can continue to read resolved notes. The summary keeps the notes that
have no position in the current diff (detached notes) and their quotes. It keeps
them also when the current diff is empty. A quote is the selected text that a
note shows.

The window memory keeps the text of an editor that is not saved. This memory
does not use the diff nodes. A refresh, a filter or a layout change cannot
discard the text. The summary shows **Continue editing** also when the file is
not there. Save keeps the text. Cancel discards it.

The view always renders a patch after a refresh, also when an editor is open.
If the patch changed, the editor that the view shows again gives a notice and
the initial quote. The save keeps the initial anchor and calculates the
position against the current patch. A line editor that the view keeps follows
the accurate calculated position in the two layouts. The editors in the diff and the saved
cards look for endpoints only in the calculated anchor hunk.

When the view makes an editor with the focus again, it puts the focus and the
text selection back. It does not scroll the application shell. If the anchor
does not agree with the patch, the editor stays at the start of the file with
its initial quote. The workspace can become read-only before a save. Then the
save reports an error and keeps the draft and the editor open for a retry. The
text of an editor that is not saved does not stay after the app starts again.
Saved notes stay.

## Anchors and placement

Each note has these fields:

- An id and a revision.
- The initial anchor.
- The body.
- A draft, sent or resolved state.
- The time of creation and the time of the last update.
- Optional metadata about the latest submission.

An anchor records these items:

- The identity of the repository.
- The path relative to the repository.
- The source scope and the Git reference.
- The selected side.
- The inclusive spans in the previous version and in the new version.
- The hunk and the fingerprint of the patch.
- A display quote and the evidence for the full selection.

Display quotes remove the CRLF carriage returns. The evidence that finds the
selection keeps them. A file note does not have spans, a hunk or selection evidence.

The initial anchors do not change. The view calculates the position again for
each full scope snapshot that has no filter. A filter or a change of scope does
not remove the position of a note or change the note. A note also shows in a
different scope when that scope contains its selection. The view gives these
results:

- If the file is missing, the note becomes detached.
- If the selected code is missing, the note becomes changed.
- If a Git read has a failure or did not load, the position stays unknown. The
  view does not think that the code is not there.

If the selected text is accurately at more than one location, the view uses
the location nearest to the previous location. If two locations have the same
distance, the position stays ambiguous. The view does not use a whitespace or
an approximate fallback, and it does not resolve notes automatically. A changed
excerpt shows that the code changed. It does not show that the agent did the
request.

A quote contains a maximum of 40 rows of 400 UTF-16 code units. A long
selection keeps its full spans and two fingerprints of the full
selection. The shorter quote shows the first and the last rows with an
explicit elision. The spans and the fingerprints do not use the quote. These hashes are not cryptographic. They help to find the
local position. They do not guarantee integrity or identity. File notes use the
fingerprint of the patch to find changes after the submission.

## Persistence and lifecycle

`prometeu:review:<workspace>` stores `{ "v": 1, "comments": [...] }` in
localStorage. A workspace can have a maximum of 200 notes. Each body that is not
empty has a maximum of 8 KiB of UTF-8. The store fully validates the records and
returns copies of the snapshots.

The store does not change unknown versions or records that are malformed or
that it cannot read. These records stop all writes. If a write has a failure,
the store reports an error and keeps the edit in memory. If the person keeps the window
open and edits again, the store tries the write again. This storage is local to
the browser origin. It is not a copy in the backend. When a person removes a
workspace from the board, the store deletes its notes. When a person closes a
conversation, the store does not delete the notes.

Writes and deletions get the `prometeu:review` Web Lock for the full origin.
Each change reads the storage again while it has the lock. Thus, when
different windows add notes at the same time, all the notes stay. Edit, resolve and
delete compare the revision that the control or the editor recorded. If that
revision is not current, they report a conflict. They do not replace a newer
note or a deleted note. Storage events refresh the other windows.

A write with a failure that is not complete stays in memory. If the storage changed
during that time, the retry reports a conflict and keeps the two copies. After
such a conflict, copy the text that is not saved before you reopen or reload
the window. If an environment does not have Web Locks, the store refuses
writes. It does not use a storage method that is not safe.

Usually the backend owns the storage of the application. This storage is an
explicit different rule. It is like the Reviewed marks, but it validates its
reads and it shows write failures. Notes that are not sent do not go into the
board, the Cloud, the relay or a new IPC command. The current transport can send
a text batch that a person sent in its message or in its stored queue.

## Drafts and submission

The summary selects the draft notes. It can also select the unresolved notes
from previous reviews. A person can select individual notes to make a batch that
is in the limits. Send to agent opens the conversation menu of the workspace.
The conversations that the person selected recently show first. If there is no
conversation, the menu lets the person make one with the workspace defaults.

Send to agent only adds a tag to the current draft of that conversation. A busy
conversation uses the current queue. If the person adds a note a second time,
the merge uses the note id and replaces the snapshot of that item. If a person removes the tag, the notes do
not change. Draft tags are local to the window, like browser context tags.
Saved notes stay after a reload, and a person can add them again.

Each item records the revision, the body, the anchor and the path of the note.
The path is relative to the folder where the conversation
operates. Subsequent
edits do not change that tag without a notice. When the person sends the
message, the revisions that agree with the tag become sent. A newer revision or a resolved revision keeps its current
state. The latest batch, the conversation, the time and the sent anchor stay
available. Agent Actions send the same context to the conversation that they
make. Prompt Actions keep the context in the draft.

The sent state means that the person sent the batch. It does not mean that the
provider received it. The native send can store a queue before the recovery of
the process has a failure. Do not try the send again automatically. If IPC
rejects the send, do not think that the send has a failure. The notes stay
available for an explicit Reopen and for a different review. There is no hidden localized instruction and no
automatic send.

## Text envelope and compatibility

The transport is usual message text. The block goes after the file mentions
and the browser contexts, and before the typed text:

```text
<prometeu-review v="1">
{"batch":"example","comments":[{"n":1,"repo":"api","file":"api/src/cart.ts","in":"worktree","old":null,"new":[40,42],"hunk":"@@ -31,9 +31,14 @@","excerpt":["+return tax;"],"body":"Use the configured tax rate."}]}
</prometeu-review>
```

The payload has these keys and types and no others. Each `<` in the JSON line
changes to `\u003c`. The numbers start at 1 and increase by 1. `in` is
worktree, index or HEAD for Local files, Stage or Compare. It identifies the
version of the new side. The paths come from the repositories that the workspace
added in `Workspace.worktree`.

The block refuses these path items:

- Absolute paths.
- Traversal, dot or empty segments.
- Backslashes and disk drive prefixes.
- NUL and line breaks.

The block accepts 1 to 100 notes and a maximum of 256 KiB of encoded UTF-8. This
limit includes the escape characters and the envelope. The block refuses a batch
that is too large. It does not cut the batch without a notice.

`message-context.ts` keeps the sequence of the browser and review blocks and
each byte of the text around them. Unknown, malformed and nested closed
envelopes stay as literal text. A prefix that is not complete stays literal.
The complete blocks after it that are not nested continue to render as tags. A
parent block that is not complete cannot use the remaining part of the
message.

`components/chat/review-context.ts` shows the note data only as text. It does not
open files or do markup from a transcript. Desktop, the desk, mobile and
shared viewers use the same renderer. Older clients show the raw block. V1, the
provider adapters and the relay do not remove or change it. No protocol
migration is necessary. Notes that are not sent stay private. Sent text
follows the usual transcript sharing and E2EE.

## Evidence

- `src/review-comments.test.ts`: the selection and the position that does not
  change with the scope. Moved, changed, duplicate and long selections. Merged
  draft snapshots that do not change.
- `src/review-context.test.ts`: the strict format, the paths, the limits, mixed
  envelopes and the literal fallback. `src/browser-context.test.ts` keeps the
  compatibility of the browser context.
- `src/workspace-review.test.ts`: a refused save keeps a resumed draft for a
  retry.
- `src/review-store.test.ts`: the reload, the limits, unknown or corrupt data,
  write failures, the deletion of workspace notes and the submission that
  compares revisions.
- `e2e/git.spec.ts`: notes from the keyboard, the reload, the draft and the
  sent history in the current WebKit review journey. The mock proves the
  connection of the UI, not the native delivery.
- `e2e/mobile.spec.ts`: mixed browser and review context through the encrypted
  local peer. The note data, the focus and the mobile layout in Chromium and
  WebKit.
- The component catalog includes notes, the editor, the summary, the review tag
  and an interactive state of the diff editor with production components.
