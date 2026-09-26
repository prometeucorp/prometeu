# Local diff review

Implement issue #135: local review notes on a file, line or same-hunk range in
Local files, Stage and Compare; mouse and keyboard, unified and split layouts.
Notes survive restarts, remain separate from Reviewed marks and team comments,
and enter a chosen local conversation as one removable draft tag. Sending is
explicit. Desktop, desk, mobile and shared history render the same text block.

## Boundaries and decisions

- Pure note rules and a strict versioned text codec have no DOM or I/O.
- A localStorage adapter owns `prometeu:review:<workspace>` version 1, with
  200 notes, 8 KiB UTF-8 bodies and bounded excerpts. Unsupported/corrupt stores
  are protected against overwrite; write failures preserve memory and report.
- Original anchors remain immutable. Placement is derived from complete,
  unfiltered scope snapshots; changing scope never mutates a note. Matching
  uses complete selection fingerprints and lengths independently of abbreviated
  display quotes. Equidistant matches stay ambiguous. Only exact text matches.
- Each edit increments a revision. A draft contains immutable note snapshots;
  submitting an old snapshot does not mark newer edits sent. Sent means the
  person submitted the batch, including the existing backend queue path; it
  does not assert provider receipt. Retrying is explicit through Reopen.
- The identified `<prometeu-review v="1">` block contains up to 100 notes and
  256 KiB of encoded UTF-8. Paths are relative to the agent working directory.
  Unknown/malformed blocks remain literal, including beside browser contexts.
- UI components receive snapshots and callbacks. Workspace Changes owns Git
  reads, the store adapter owns persistence, ChatView owns drafts and sending.
  New presentation uses shared controls, English/Portuguese labels and gallery
  states. Remote/conflict/history/binary/metadata/truncated areas cannot author.
- No automatic resolution, publishing, pending-note sharing, provider capability,
  Conversation V1, IPC or relay changes. No invisible default instruction.

## Evidence required

Unit tests cover anchors, scopes, repeated code, truncation, strict decoding,
store failures/versioning/pruning and revision-aware draft merging/submission.
Existing Git keyboard WebKit coverage gains notes, reload and explicit sending;
portable history coverage proves review/browser tags coexist. Full repository
checks run after focused tests. Contracts, ADR and provider matrix stay aligned.
