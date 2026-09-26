# Local diff review implementation plan

> **For agentic workers:** Use superpowers:executing-plans inline; review the completed change independently.

**Goal:** Annotate local diffs and explicitly submit one review batch to a conversation.
**Architecture:** Pure review rules and codec; local storage adapter; callback-driven Git and chat presentation.
**Tech Stack:** TypeScript, shared DOM components, Vitest, Playwright.
**Spec:** [Local diff review](../specs/2026-09-26-diff-review-design.md).

## Global constraints

English identifiers and tests, English/Portuguese UI. Preserve V1, IPC, relay,
provider boundaries and existing unrelated changes. Use current isolated worktree.

## Review focus

- Scope switches and filters cannot report false removal or rewrite anchors.
- A submitted older revision cannot consume newly edited notes.
- A storage read/write failure cannot silently destroy user writing.
- Queued send failures must not trigger automatic duplicate retries.
- Lazy diff refresh must preserve selection, focus and unchanged patch nodes.

### Task 1: Pure rules, codec and storage

- [x] Write anchor, codec, storage and draft lifecycle tests; run them red.
- [x] Implement `review-comments.ts`, `review-context.ts`, `message-context.ts`
  and `review-store.ts`; run focused tests green.
- Interfaces: `anchorSelection`, `place`, `reviewBatch`, `mergeReviewDraft`,
  `encodeReviewContext`, `splitMessageContexts`, `createReviewStore`.

### Task 2: Git annotation and summary

- [x] Extend keyboard review scenario with note creation and reload; run red.
- [x] Add callback-driven inline notes, range selection and summary components.
- [x] Connect unfiltered Git snapshots and store in Workspace Changes.
- Interfaces: optional DiffSnapshot review callbacks, note summary callbacks;
  components import only portable rules, never the storage adapter or IPC.

### Task 3: Draft, send and portable history

- [x] Extend the representative journey through draft removal, merging and send.
- [x] Integrate revision snapshots in ChatView and workspace conversation picker.
- [x] Render review tags through shared message content; include Actions.
- Interfaces: `attachReview(tab, draft)`; submission updates exact revisions only.

### Task 4: Documentation and verification

- [x] Add gallery examples, contract, ADR, indexes, README and provider matrix.
- [x] Run focused unit/browser checks, then `npm run check`.
- [x] Obtain fresh-context review and fix material findings with regressions.

## Execution record

Analysis approved by the implementation request. Existing branch is isolated.
No commits or external publication are needed to deliver the reviewable change.


- Task 1: complete; model/codec/store regressions passed, including rejected writes.
- Tasks 2–3: keyboard/reload/send and mobile mixed context passed in Chromium and
  WebKit; split pointer selection and merged/removable drafts also passed.
- Independent review found CRLF quote rejection and unfinished-editor loss on
  removal. Both received failing regressions before fixes. Matching remains
  exact while display strips CR; editor drafts now outlive rendered file nodes.
- Ruling: submission records the user's send, not provider receipt, because an
  IPC error can follow durable queueing. Reopen is the explicit retry action.
- Ruling: draft tags and unfinished editors remain window-local; saved notes
  survive restarts. The contract makes this distinction explicit.
- Full web suite initially passed 547 tests. A later complete-check run hit the
  existing relay WebSocket close timeout; all 11 relay integration tests passed
  in isolation. The final run passed 548 web tests and 477 Rust tests (7 ignored),
  along with documentation, architecture, formatting and desktop/mobile builds.
- Full browser verification exposed a patch assertion that included the new
  annotation controls; it now compares patch cells. The next full run passed
  178 scenarios, with a streaming-scroll race in the remaining review scenario.
  Waiting for the simulated response before inspecting replay passed three
  repetitions in each browser (6/6). Clippy passed separately. Every check stage
  passed, although the aggregate command required these targeted follow-ups.

- Greptile follow-up: changed patches now rebuild with retained editor text and
  an explicit original-selection notice. Storage mutations/pruning use Web Locks,
  refresh disk state, and reject stale revisions; incomplete envelopes no longer
  hide later complete contexts. Regressions first reproduced all three findings.
- Final follow-up verification: `npm run check` passed end to end: 552 web tests,
  477 Rust tests (7 ignored), 179 browser scenarios, builds, documentation,
  architecture, formatting and Clippy. A native WKWebView probe at
  `tauri://localhost` also acquired the Web Lock successfully.

- Second Greptile follow-up: refused saves now report an error without closing or
  deleting resumed drafts. Rebuilt active editors retain derived line placement,
  keyboard focus and caret selection. Coordinator and browser regressions cover
  refusal/retry, patch refresh and layout changes. `npm run check` passed with
  553 web tests, 477 Rust tests (7 ignored) and all 179 browser scenarios.
