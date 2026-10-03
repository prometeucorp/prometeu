# ADR 0059 — Local telemetry with canonical events in SQLite

Date: 2026-09-25
Status: Accepted

## Context

Prometeu sees both product choices and the execution of work across providers,
conversations, workspaces and pull requests. The board describes current state;
transcripts contain content and are not an appropriate analytics dataset.
Some execution metadata already reaches the adapters, but there is no durable,
content-free history with canonical usage semantics.

The first delivery needs capture, queries by period and workspace, simple
aggregates, export and deletion. Cloud synchronization and a full Insights page
are outside this delivery.

## Options considered

1. **JSONL event files.** Straightforward append, but queries, deduplication and
   deletion need additional application machinery.
2. **SQLite with common columns and typed JSON payloads.** One event table
   supports indexed queries, uniqueness constraints and transactions while
   accommodating different event types.
3. **Separate relational tables for every domain.** Strong specialized schemas,
   but more migrations and relations before the event vocabulary is established.

## Decision

Use option 2, in a private `<root>/telemetry.sqlite3` owned by the Rust backend.
Use direct SQL and a concrete telemetry module. Consumers call application
queries through typed IPC; they do not access SQLite or provider payloads.
JSONL remains an export format, not the primary store.

The [contract](../contracts/telemetry.md) owns event shapes, lifecycle,
privacy, coverage and the implementation acceptance criteria. Its core choices
are:

- A telemetry turn is one message accepted for execution. Its main-agent
  completion does not wait for background children. Request responses are not
  new turns. This does not change the conversation settlement rule in
  [ADR 0056](0056-background-tasks-hold-completion.md).
- Record execution intervals so queries can return both their summed duration
  and their union. Neither is a measurement of model compute time.
- Measure human response waits only for explicit requests, identified by a
  stable request ID. Do not interpret gaps between turns as human work or delay.
- Input totals include cache tokens; output totals include reasoning tokens.
  The breakdowns are subsets, not additional consumption. Unknown is null.
- Keep the selected model separate from observed models. Attribute usage per
  model only when the provider reports a trustworthy breakdown. A verified
  whole-tree delta carries explicit `wholeTree` scope; otherwise retain the
  main-agent scope and useful partial evidence.
- Associate workspaces with multiple repositories and PRs without allocating
  consumption automatically. Related PR totals are not additive across PRs.
- Store opaque work identifiers, not project names, branch names, titles,
  repository URLs or paths. Provider/model identifiers and PR numbers are
  permitted metadata.
- Acknowledge event persistence only after commit. Failed capture does not
  block agent work; log sanitized diagnostics and expose incomplete-history
  status in the summary and export, without per-event toasts or modals.
- Give each event a UUID and a separate stable occurrence key. Retries retain
  both; transcript replay and remote mirrors do not capture the event again.
- Keep archive, PR association and merge as independent facts. Do not infer a
  generic work-completed event or treat merge as deployment.
- Preserve incomplete executions and known measurements. Never invent their
  end time or outcome to make aggregates look complete.
- Put the first user-facing summary in settings, with period/workspace
  filters, coverage, export and complete deletion. Reuse i18n and shared UI.
- Extend this store for issue 131's reply footers and workspace insights, rather
  than creating an independent usage ledger. New canonical completions may carry
  normalized usage; historical top-level `costUsd` remains legacy scope and is
  never summed or used as a footer fallback.
- Use cumulative deltas only with a verified fresh or same-process baseline.
  A previous persisted terminal cannot prove whether external resumed work
  changed the counter. Resets and unverified resumed baselines retain partial
  evidence instead of inventing consumption or treating missing values as zero.
- Restore native-transcript reply metadata through exact local lookup of a
  conversation-scoped assistant-ID hash. Do not join by timestamp or turn order,
  and do not backfill transcripts. Missing historical anchors remain unknown.
- Keep app calls as separate paired facts with a closed source vocabulary.
  Workspace naming has workspace scope; global plugin creation does not.
  Action/delegation provenance uses opaque aliases and never copies commands.
- Retain existing related-PR query semantics. More precise tenure requires
  actual branch identity, lifecycle timestamps and a complete retained candidate
  snapshot; incomplete evidence is related-only. Never sum PR shares across
  repositories as independent workspace consumption.
- Show context occupancy separately from consumption. Warning thresholds are
  advisory; compact, report and new-conversation actions remain explicit.

Canonical usage follows the existing encrypted conversation sharing contract;
local app-call history, origins and lookup queries do not cross that boundary.
The legacy conversation event already permits a CLI estimate. Erasing local
history invalidates restored UI caches without rewriting conversation
transcripts, shared copies or exported files.

## Consequences

SQLite is an additional desktop dependency. It avoids implementing an index,
deduplication store and query engine around event files. Short committed writes
are the initial policy; an in-memory batch queue is not the durability boundary.

Read-only WAL snapshots keep summaries and streaming exports outside the capture
mutex. SQL joins and ordered row processing avoid loading retained history into
application memory. This adds WAL disk growth while readers are active; deletion
waits for readers and exports, then vacuums and truncates the WAL before success.
Existing version-1 events remain readable; additional indexes are additive.

Telemetry stays independent of board and transcript retention. Archive and
worktree cleanup must not remove it. Explicit deletion overrides append-only
retention and must also prevent pending callbacks from restoring erased data.

There is no atomic transaction spanning a provider process, the JSON board and
SQLite. The foundation promises idempotent persistence of captured facts, not
exactly-once observation of all external activity. Crashes and storage failures
remain visible as incomplete coverage where detectable.

Global IDs and event versions support a future consumer, but this change does
not introduce a sync worker, cloud schema, organization access or a second
taxonomy. A later Cloud design must address consent, deletion and recipients.

## Implementation and evidence

- [Store and types](../../src-tauri/src/telemetry.rs),
  [capture](../../src-tauri/src/telemetry/capture.rs) and
  [queries](../../src-tauri/src/telemetry/query.rs) implement the local boundary.
- [Regression tests](../../src-tauri/src/telemetry/tests.rs) cover persistence,
  privacy, retries, incomplete execution, overlap, query cohorts and erasure.
- [Claude](../../src-tauri/src/claude.rs) and
  [Codex](../../src-tauri/src/codex.rs) test native normalization and counter scope.
  [Antigravity](../../src-tauri/src/antigravity.rs) tests observed step usage and
  cumulative boundaries against its recorded fixtures.
- [Settings](../../src/telemetry-settings.ts), [IPC types](../../src/telemetry.ts)
  and [mock tests](../../src/telemetry.test.ts) cover the first consumer.
  The existing IPC parity test checks native registration and browser handlers.
- [Usage presentation tests](../../src/usage-presentation.test.ts),
  [workspace refresh tests](../../src/workspace-usage.test.ts),
  [timeline tests](../../src/timeline.test.ts) and
  [browser focus/draft coverage](../../e2e/usage-insights.spec.ts) protect the
  conversation and workspace consumers. Synthetic Claude fixtures declare
  their provenance rather than claiming a recorded CLI run.

The [contract](../contracts/telemetry.md) records the implemented payloads and
coverage limits. Usage scope is explicit; child time remains independently
observed, without invented child token allocation. Streaming input
whose terminal cannot be correlated to an accepted message remains incomplete;
normalization never guesses its owner. Cost remains a provider-client estimate.
These limits are visible in measurement/capture coverage.

Old boards default the additive opaque identity map. Existing transcripts remain
readable and are never backfilled. Unknown future SQLite versions are preserved.
Provider fixtures and existing lifecycle tests establish regression behavior;
no live conformance run for every installed CLI version is claimed.
