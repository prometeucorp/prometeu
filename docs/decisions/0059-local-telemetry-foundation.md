---
type: Decision
title: "ADR 0059 — Local telemetry with canonical events in SQLite"
description: "Local telemetry in SQLite, with shared columns and typed JSON payloads."
status: stable
---

# ADR 0059 — Local telemetry with canonical events in SQLite

Date: 2026-09-25

## Context

Prometeu sees the product selections of the person. It also sees how work
operates across providers, conversations, workspaces and pull requests. The
board gives the current state. Transcripts contain content, and they are not
a correct data set for analytics. Some execution metadata gets to the adapters
at this time. But there is no durable history without content that has
canonical usage rules.

The first delivery must record data, send queries by period and by workspace,
calculate simple totals, export and delete. Cloud synchronization and a full
Insights page were not in the first delivery. At this time, the local Usage
screen uses these queries. Cloud synchronization stays out of this decision.

## Options considered

1. **JSONL event files.** It is easy to add records. But queries, the removal of
   duplicates and deletion make more application code necessary.
2. **SQLite with shared columns and typed JSON payloads.** One event table gives
   indexed queries, unique constraints and transactions. It also accepts
   different event types.
3. **Different relational tables for each domain.** The schemas are strong and
   specialized. But they make more migrations and relations necessary before the event
   vocabulary is stable.

## Decision

Use option 2, in a private `<root>/telemetry.sqlite3` that the Rust backend
owns. Write the SQL directly in one telemetry module. Consumers call application
queries through typed IPC. They do not use SQLite or provider payloads directly.
JSONL stays an export format. It is not the primary store.

The [contract](../contracts/telemetry.md) gives the event formats, the
lifecycle, privacy, coverage and the criteria that the implementation must satisfy.
These are its primary rules:

- A telemetry turn is one message that Prometeu accepts for execution. The
  completion of the primary agent does not wait for background children.
  Responses to requests are not new turns. This does not change the rule for
  the completion of a conversation in
  [ADR 0056](0056-background-tasks-hold-completion.md).
- Record execution intervals. Thus queries can return the sum of their times and
  their union. These values do not measure how long a model calculates.
- Measure the time until a person answers only for explicit requests. A
  stable request ID identifies each request. Do not use the time between turns
  as human work or as a wait.
- Input totals include cache tokens. Output totals include reasoning tokens.
  The breakdowns are subsets. They are not more consumption. An unknown value
  is null.
- Keep the selected model apart from the models that Prometeu sees. Record
  usage for each model only when the provider reports a breakdown that
  Prometeu can trust. A delta for the full tree that Prometeu examined has an explicit
  `wholeTree` scope. If not, keep the scope of the primary agent and keep
  useful evidence that is not full.
- Connect workspaces to many repositories and PRs. Do not divide consumption
  between them automatically. The totals of related PRs do not add across PRs.
- Store opaque work identifiers. Do not store project names, branch names,
  titles, repository URLs or paths. Prometeu can store provider and model
  identifiers and PR numbers as metadata.
- Tell that Prometeu saved an event only after the commit. A capture failure does not
  stop agent work. Log diagnostics without private data. Show an
  incomplete-history status in the summary and the export. Do not show a toast
  or a modal for each event.
- Give each event a UUID and a different stable occurrence key. Retries keep the
  two values. A transcript replay and remote copies do not record the event
  again.
- Keep archive, PR association and merge as three different facts. Do not make a
  general "work completed" event from them. A merge is not a deployment.
- Keep incomplete executions and known measurements. Do not add an end time or
  an outcome that Prometeu did not see to make totals look complete.
- Put the first summary for the user in Settings. It has filters for period
  and workspace, coverage, export and full deletion. Use the i18n catalogs and
  the shared UI again.
- Use this store for the reply footers and the workspace insights of issue 131.
  Do not make a different usage ledger. New canonical completions can contain
  normalized usage. The historical top-level `costUsd` stays in its legacy
  scope. Prometeu does not add it to totals or use it as a footer fallback.
- Use cumulative deltas only with a new baseline that Prometeu examined, or a
  baseline from the same process. A previous saved terminal event cannot prove if resumed
  external work changed the counter. When a counter starts again from zero, or when
  Prometeu cannot make sure of a resumed baseline, it keeps the evidence that is not full.
  Prometeu does not add consumption, and it does not use 0 for missing values.
- Get the reply metadata of a native transcript again through an accurate local
  lookup. The lookup uses a hash of the assistant ID in the scope of the
  conversation. Do not connect records by timestamp or by turn sequence. Do not
  add data to transcripts from before this change. Historical anchors that are missing stay unknown.
- Keep app calls as different facts in pairs, with a closed vocabulary of
  sources. When a person gives a name to a workspace, the call has a workspace
  scope. The creation of a
  global plugin does not. The provenance of actions and delegations uses opaque
  aliases, and it does not copy commands.
- Keep the current rules for related-PR queries. For a more accurate period of
  ownership, Prometeu must have the branch identity, lifecycle timestamps and a
  full candidate snapshot. If the evidence is not complete, the PR is only
  "related". Do not add PR shares across repositories as if each share is workspace
  consumption.
- Show context occupancy apart from consumption. Warning thresholds only give
  advice. Compact, report and new-conversation actions stay explicit.

Canonical usage follows the current contract for the encrypted shared
conversation. The local app-call history, the origins and the lookup queries do not
go across that boundary. The legacy conversation event can contain a CLI estimate
at this time. When the person erases the local history, Prometeu cancels the UI
caches that it filled from that history. It does not change conversation transcripts, shared copies
or exported files.

## Consequences

SQLite is one more desktop dependency. With SQLite, it is not necessary to
write an index, a store that removes duplicates and a query engine for event
files. The first policy is short committed writes. A batch queue in memory is
not the durability boundary.

Read-only WAL snapshots keep summaries and stream exports out of the capture
mutex. SQL joins and the processing of rows in sequence do not load the kept
history into application memory. Thus the WAL file on the disk becomes larger
while readers are active. Deletion waits for readers and exports. Then it does
a vacuum and truncates the WAL before it reports success. Prometeu can read the
current version-1 events. New indexes only add to the schema.

Telemetry stays apart from the retention of the board and the transcripts.
Archive and worktree cleanup must not remove it. An explicit deletion is more
important than the append-only retention. It must also prevent callbacks that
wait from writing erased data again.

No atomic transaction includes a provider process, the JSON board and SQLite.
The foundation promises that Prometeu saves each recorded fact one time, also
after retries. It does not promise that Prometeu sees all external activity one
time and only one time. When Prometeu can find crashes and storage failures,
it shows them as incomplete coverage.

Global IDs and event versions can help a subsequent consumer. But this change
does not add a sync worker, a cloud schema, organization access or a second
taxonomy. A subsequent design for the Cloud must include consent, deletion and
recipients.

## Implementation and evidence

The Usage screen adds coverage for each conversation to `telemetry_insights`.
It does not send the event history to the webview. Prometeu calculates the
new counts during the current cohort query. Thus the query keeps its
"final or latest" rules, and a second read of the full history is not
necessary. A migration of the stored events from before this change is not
necessary. Older query responses
stay usable with unknown coverage. Settings continues to own the filters, the
refresh, the export and the erasure. The component gallery also uses the
isolated presentation. This change does not add provider capture or prices.

- [Store and types](../../src-tauri/src/telemetry.rs),
  [capture](../../src-tauri/src/telemetry/capture.rs) and
  [queries](../../src-tauri/src/telemetry/query.rs) contain the local boundary.
- [Regression tests](../../src-tauri/src/telemetry/tests.rs) examine
  persistence, privacy, retries, incomplete executions, overlap, query cohorts
  and erasure.
- [Claude](../../src-tauri/src/claude.rs) and
  [Codex](../../src-tauri/src/codex.rs) test native normalization and the scope
  of counters. [Antigravity](../../src-tauri/src/antigravity.rs) tests the
  usage of each step that Prometeu sees and the cumulative boundaries against
  its recorded fixtures.
- [Settings](../../src/telemetry-settings.ts), [IPC types](../../src/telemetry.ts)
  and [mock tests](../../src/telemetry.test.ts) examine the first consumer.
  The current IPC parity test examines the native registration and the browser
  handlers.
- [Usage presentation tests](../../src/usage-presentation.test.ts),
  [workspace refresh tests](../../src/workspace-usage.test.ts),
  [timeline tests](../../src/timeline.test.ts) and
  [browser focus and draft coverage](../../e2e/usage-insights.spec.ts) prevent
  regressions in the conversation and workspace consumers. Synthetic Claude fixtures declare
  their provenance. They do not claim that they come from a recorded CLI run.

The [contract](../contracts/telemetry.md) records the payloads of the
implementation and the coverage limits. The usage scope is explicit. Prometeu
records the time of child tasks independently. It does not give tokens to child
tasks if the provider does not report them. If Prometeu cannot connect the
terminal event of stream input to an accepted message, that input stays
incomplete. Normalization does not guess its owner. The cost stays an estimate
of the provider client. The measurement and capture coverage shows these limits.

Boards from before this change get an empty opaque identity map by default. Current transcripts stay
readable, and Prometeu does not add data to them. Prometeu keeps the data of
unknown newer SQLite versions. Provider fixtures and the current lifecycle tests
make the regression behavior stable. This decision does not claim a live
run for each installed CLI version that shows agreement with the contract.
