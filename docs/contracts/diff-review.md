# Local diff review notes

Status: implemented. See [ADR 0062](../decisions/0062-local-diff-review.md).

## Ownership and scope

Review notes are local writing addressed to an agent. They are independent of
Git's Reviewed marks and the human-to-human transcript comments. Authoring is
available in Local files, Stage and Compare on the owner's usable workspace,
for text hunks in unified and split layouts. History, conflicts, remote
workspaces, binary/metadata-only patches and rows beyond the 2,500-row rendering
limit have no authoring control. A file note also requires a text patch.

`review-comments.ts` owns pure selection, placement and draft snapshot rules.
`review-store.ts` owns local persistence. `workspace-review.ts` composes those
rules with Git reads, dialogs and callbacks from Workspace Changes. Components
under `components/git/` receive data and callbacks, never storage or IPC.

The line-number button selects a line; Shift-click or dragging extends within
one hunk. With focus in the diff, arrows move, Shift-arrows extend, C/Enter opens
the editor and Escape clears selection/cancels editing. Left/Right selects the
old/new side in split layout. Command/Control-Enter saves. Saved notes appear
inline, in file counts and in a workspace summary, with edit/delete/resolve/
reopen actions. Resolved notes remain readable. The summary retains detached
notes and their quotes, including when the current diff is empty. Unfinished
inline writing is kept in window memory independently of diff nodes. Refresh,
filtering or layout changes cannot discard it: the summary offers **Continue
editing** even when its file is absent. Save persists it; Cancel discards it.
A refreshed patch is always rendered, including while an editor is open. If its
patch changed, the restored editor shows a notice and the original quote; saving
keeps that original anchor and computes placement against the current patch.
Retained line editors follow exact derived placement in either layout. Rebuilding
an actively focused editor restores focus and its text selection without scrolling
the application shell. If the anchor no longer matches, the editor stays at the
file start with its original quote. A save attempted after the workspace becomes
non-writable reports an error and keeps the draft and editor open for retry.
Unlike saved notes, unfinished editor text does not survive an app restart.

## Anchors and placement

Each note has an id, revision, original anchor, body, draft/sent/resolved state,
creation/update timestamps and optional latest submission metadata. Anchors
record repository identity, repository-relative path, source scope/reference,
selected side, old/new inclusive spans, hunk, patch fingerprint, a display quote
and full-selection evidence. Display quotes strip CRLF carriage returns;
matching evidence preserves them. A file note has no spans, hunk or selection evidence.

Original anchors are immutable. Placement is derived separately for each full,
unfiltered scope snapshot. Filtering or switching scopes does not detach or
rewrite a note. Notes are also displayed in another scope when that scope
contains their selection. A missing file is detached; missing selected code is
changed. Failed/unloaded Git reads remain unknown, never interpreted as removal.
Exact matches use the nearest old location; equidistant matches remain ambiguous.
There is no whitespace/fuzzy fallback or automatic resolution. A changed excerpt
indicates code changed, not that the agent satisfied the request.

Quotes contain at most 40 rows of 400 UTF-16 code units. Long selections retain
their full spans and two rolling fingerprints independently of the abbreviated
quote (first/last rows with an explicit elision). These non-cryptographic hashes
are local placement heuristics, not integrity or identity guarantees. File notes
use the patch fingerprint to detect changes since submission.

## Persistence and lifecycle

`prometeu:review:<workspace>` stores `{ "v": 1, "comments": [...] }` in
localStorage. The workspace limit is 200 notes; each nonempty body is at most
8 KiB UTF-8. The store strictly validates records and returns copied snapshots.
Unknown versions and malformed/unreadable records are left untouched and block
writes. Failed writes report an error and retain the edit in memory; keeping the
window open and editing again retries persistence. This is browser-origin local
storage, not a backend backup. Removing a workspace from the board prunes its
notes; closing a conversation does not.

Writes and pruning acquire the origin-wide `prometeu:review` Web Lock. Each
mutation rereads storage under the lock; independent additions survive concurrent
windows. Edit, resolve and delete check the revision captured by the control or
editor and report a conflict instead of overwriting a newer or deleted note.
Storage events refresh other windows. A pending failed write remains in memory;
if disk changed meanwhile, retry reports a conflict and leaves both copies intact.
Copy unsaved writing before reopening/reloading after such a conflict. Environments
without Web Locks refuse writes rather than falling back to unsafe persistence.

This is an explicit exception to backend-owned application persistence, like
Reviewed marks but with protected reads and visible write failures. No pending
notes enter the board, Cloud, relay or new IPC. Existing transport can of course
carry a submitted textual batch in its message or persisted queue.

## Drafts and submission

The summary selects draft notes and optionally unresolved notes from previous
rounds. Individual selection permits batches below the limits. Send to agent
opens the workspace's conversation menu (recently selected conversations first);
without a conversation it offers creation with workspace defaults. It only adds
a tag to that conversation's existing draft. Busy conversations use the existing
queue. A second addition merges by note id and replaces that item's snapshot.
Removing the tag leaves the notes unchanged. Draft tags are window-local, like
browser context tags; saved notes survive reload and can be added again.

Each item captures the note's revision, body, anchor and working-directory-relative
path. Later edits do not silently change that tag. Sending marks matching
revisions submitted; a newer or resolved revision keeps its current state. The
latest batch, conversation, timestamp and sent anchor remain available. Agent
Actions carry the same context to their resulting conversation; prompt Actions
retain it in the draft.

Submitted means the person sent the batch, not provider receipt. The native send
may persist a queue before process recovery fails. Do not automatically retry or
infer failure from IPC rejection. Notes remain available for explicit Reopen and
another round. There is no localized hidden instruction or automatic send.

## Text envelope and compatibility

The transport is ordinary message text, after file mentions/browser contexts and
before typed text:

```text
<prometeu-review v="1">
{"batch":"example","comments":[{"n":1,"repo":"api","file":"api/src/cart.ts","in":"worktree","old":null,"new":[40,42],"hunk":"@@ -31,9 +31,14 @@","excerpt":["+return tax;"],"body":"Use the configured tax rate."}]}
</prometeu-review>
```

The payload has exactly these keys/types. Each `<` in the JSON line is escaped
as `\u003c`. Numbering is consecutive from 1. `in` is worktree, index or HEAD for
Local files, Stage or Compare, respectively; it identifies the new-side version.
Paths are derived from registered repositories under `Workspace.worktree`.
Absolute paths, traversal/dot/empty segments, backslashes, drive prefixes, NUL
and line breaks are refused. The block accepts 1–100 notes and at most 256 KiB
of encoded UTF-8, including escaping and envelope. Oversized batches are refused,
never silently truncated.

`message-context.ts` preserves browser/review block order and every surrounding
text byte. Unknown, malformed and nested closed envelopes stay literal. An incomplete
prefix stays literal while subsequent complete, non-nested blocks still render
as tags; incomplete ancestors cannot consume the rest of the message.
`components/chat/review-context.ts` shows details only as text. It does not open
files or execute markup from a transcript. Desktop, desk, mobile and shared
viewers use the same renderer. Older clients display the raw block. V1, provider
adapters and relay neither remove nor reinterpret it; no protocol migration is
needed. Pending notes stay private; submitted text follows ordinary transcript
sharing and E2EE.

## Evidence

- `src/review-comments.test.ts`: selection, scope-independent placement, moved,
  changed, repeated and long selections; immutable merged draft snapshots.
- `src/review-context.test.ts`: strict format, paths, bounds, mixed envelopes and
  literal fallback; `src/browser-context.test.ts` retains browser compatibility.
- `src/workspace-review.test.ts`: a refused save keeps a resumed draft for retry.
- `src/review-store.test.ts`: reload, bounds, unknown/corrupt data, write failure,
  workspace pruning and revision-aware submission.
- `e2e/git.spec.ts`: keyboard annotation, reload, draft and sent history in the
  existing WebKit review journey; the mock proves UI wiring, not native delivery.
- `e2e/mobile.spec.ts`: mixed browser/review context over the encrypted local peer,
  details, focus and mobile layout in Chromium/WebKit.
- The component catalog includes notes, editor, summary, review tag and an
  interactive diff authoring state using production components.
