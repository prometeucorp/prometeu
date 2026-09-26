# ADR 0062 — local review notes and explicit batch submission

Date: 2026-09-26
Status: Accepted

## Context

Reading a diff and asking an agent to change it previously required manually
copying locations into the composer. Notes must survive app restarts, follow
code conservatively and remain private until the person sends them. They are
not the shared human comments governed by ADR 0007.

## Options considered

- Backend files and new IPC: stronger ownership and future external readers,
  but expands the first version beyond the existing frontend review workflow.
- Ephemeral notes: simple, but loses the person's writing on restart.
- Protected localStorage plus a versioned message-text envelope: preserves the
  current transports and reuses ADR 0038's portable presentation pattern.

## Decision

Use bounded, versioned localStorage for pending notes, an explicit exception to
backend-owned persistence. Invalid/unknown records block writes; failed saves
retain memory and show an error. Keep storage outside isolated components.

Use immutable original anchors and derived per-scope placement, with bounded
full-selection evidence separate from abbreviated quotes. Exact matching is
conservative; ambiguity, missing code and missing files remain visible. Never
automatically resolve notes.

Send an immutable revision snapshot to a chosen conversation draft as one tag.
Only explicit submission serializes `<prometeu-review v="1">` into ordinary
text. A later edit cannot be consumed by sending an older snapshot. Submitted
records the user's action, not provider receipt: the existing queue may survive
a failed process restart. Preserve notes and require explicit retry/reopening.
The detailed contract is [Local diff review](../contracts/diff-review.md).

## Consequences

V1, IPC and relay are unchanged; the same text works for all supported providers.
Older clients show raw text. Submitted content follows transcript sharing;
unsent notes have no collaboration channel. Browser-origin storage lacks backend
backup and can run out of space. A future backend migration must preserve this
version and the write-failure guarantees, not silently discard it.

## Evidence

`review-comments.test.ts`, `review-context.test.ts`, `review-store.test.ts`, the
Git keyboard journey and mobile mixed-context journey cover the boundaries.
