# Local telemetry and insights

Status: implemented local data foundation; decision in
[ADR 0059](../decisions/0059-local-telemetry-foundation.md).

## Scope and ownership

The Rust backend captures product transitions and observed execution locally.
The feature includes indexed event pages, period/workspace summaries, PR-related
queries, JSONL export and complete history deletion. Settings exposes the first
summary under **Work and team / Local history**. Cloud sync, billing, pricing,
organization analytics, transcript backfill and a full Insights page are absent.

- Adapters normalize measurements. Native usage objects do not cross the store
  boundary. `usage.updated` remains account quota data; `Tab.tokens` remains a
  context-growth estimate. Neither is historical token consumption.
- `chat.rs` captures only live local process output and successfully written
  application commands. Replay, remote mirrors and snapshots do not recapture.
  Remote control executes through the owner's same backend command path. Internal
  measurement fields are stripped before conversation persistence and sharing.
- `telemetry.rs` owns typed facts, validation, SQLite, capture health and erasure.
  `telemetry/capture.rs` owns logical turns, execution/request intervals and scope.
  `telemetry/query.rs` owns queries. Consumers do not issue SQL.
- Journey facts originate in successful workspace/tab use cases. GitHub PR
  discovery captures new relations; an accepted message also observes any known
  current PR relations. Existing work is never reconstructed from transcripts.

## Storage, identities and compatibility

`<root>/telemetry.sqlite3` honors release/debug roots and `PROMETEU_ROOT`.
The directory is private (0700); the database is 0600. Bundled SQLite uses WAL
journaling, FULL synchronous commits, secure deletion and a 250 ms busy timeout.
WAL and shared-memory files inherit the database's permissions. Each event uses
a short transaction; a successful append means commit completed. No in-memory batch
queue claims durability. `PRAGMA user_version` is the database migration version;
event `schemaVersion` is independent. The first version is 1 for both. Unknown
future database versions are preserved and reported unavailable.

One event table contains common columns and typed JSON payloads. Unique event
IDs and occurrence keys enforce capture idempotency. Identical retries retain
both identifiers, timestamp, scope and payload; they are no-ops. Conflicting
retries are capture failures. Repeated observations of the same
workspace/repository/PR relation do not create another association.

| Envelope field | Meaning |
| --- | --- |
| `sequence` | Local insertion order; not a global identity or sync cursor |
| `id` | UUID assigned before the append |
| `occurrenceKey` | Stable identity of the fact, distinct from its event UUID |
| `schemaVersion` | Canonical event version, currently 1 |
| `occurredAt` | First live observation time, UTC Unix milliseconds |
| `recordedAt` | Local append time, UTC Unix milliseconds |
| `category` | `journey` or `work`, derived from the closed event discriminant |
| `type` | Closed event discriminant |
| `projectId`, `workspaceId`, `conversationId`, `turnId` | Opaque scope IDs; unknown is null |
| `provider` | `claude`, `codex`, `antigravity`, or null |
| `payload` | Event-specific typed JSON, never arbitrary native properties |

Time, workspace/time, turn and execution/request indexes support lookups. Existing
version-1 databases acquire the additional indexes and switch to WAL without
rewriting events. Event pages use bounded keyset pagination. Queries open a
separate read-only connection and transaction after releasing the capture mutex;
WAL snapshots allow live commits during summaries and exports. SQL pairs turns
with their final or latest partial measurements and pairs observed intervals;
Rust consumes ordered rows without retaining event history or interval arrays.
Exports stream JSONL from the same snapshot used for their summary. SQLite uses
a bounded page cache and file-backed temporary sorting; workspace choices are
the only history-sized collection returned by the summary. Query duration and
WAL disk growth can still scale with history while a reader remains active.

Some existing project IDs are paths. `Board.telemetry_ids`, a default-empty map,
assigns UUID aliases to projects, repository clones and repository/branch pairs.
Workspace/conversation UUIDs are reused; legacy non-UUID IDs receive aliases.
Names and paths remain only in the pre-existing board/domain boundary, never in
events or exports. Alias maps survive removal of board entries, preserving stable
identity if a repository is registered again. Domain identities are preserved
when telemetry history is deleted; they contain no execution history.

Board publication prepares aliases before snapshotting. Journey capture flushes
the board before recording these new references and archive-state transitions;
the first message on a process also flushes the board. Failure marks coverage
incomplete without rejecting the agent command. Logical turn IDs are durable in
`turn.started`; no transcript format migration or historical backfill is required.
A restarted process leaves previous unfinished turns open rather than assigning
its new work to them.
Archive and finish commands wait for the flush on a Tauri worker thread.
There is no atomic transaction across the JSON board, provider process and SQLite.

## Event vocabulary

| Category | Event | Payload and condition |
| --- | --- | --- |
| journey | `workspace.created` | `mode: worktree | repository`; the workspace was registered successfully, before asynchronous preparation |
| journey | `workspace.archived`, `workspace.resumed` | Empty payload; an actual archive-state transition, not opening a view or respawning a process |
| journey | `conversation.created` | Empty payload; a new logical tab was published |
| journey | `provider.selected` | `scope: workspace | conversation`; initial launch choice or explicit committed tab choice |
| work | `pull_request.associated` | `repositoryId`, nullable `branchId`, `pullRequest`; a newly captured relation |
| work | `turn.started` | Initial `measurement`; a message successfully written to the ready process |
| work | `turn.completed` | `outcome: ok | error | interrupted`, `elapsedMs`, nullable `providerDurationMs`, `measurement` |
| work | `turn.usage.observed` | `measurement`; a snapshot, not a counter increment |
| work | `agent.execution.started` | Opaque `executionId`; observed main or child execution |
| work | `agent.execution.completed` | Same `executionId`, monotonic `elapsedMs` |
| work | `human_input.requested` | Opaque `requestId`, `kind: approval | question | plan` |
| work | `human_input.received`, `human_input.cancelled` | Same `requestId`, monotonic `elapsedMs`; accepted answer/denial and cancellation are distinct |
| work | `context.compacted` | Nullable `before` and `after`; only an explicit provider observation |

Workspace creation is not proof that preparation succeeded. No generic
`work.completed` is inferred. PR opened/merged/closed timestamps remain a later
extension. PR association is not PR creation, archive is not completion and merge
is not deployment. Branch identity is included only when the observed PR names
its head branch; stale workspace labels are not evidence of the PR's branch.

## Turns and time

A message starts after a successful command write. Failed sends and prompts still
queued for setup/restart do not start turns. Provider echoes, request responses,
replay and internal turn notifications do not create new message turns.
Main-agent completion closes its telemetry turn independently of children.
The conversation's existing readiness/Stop/notification/delegation rules remain
those of [ADR 0056](../decisions/0056-background-tasks-hold-completion.md).

Each Pump has a capture-order mutex. Output and accepted input hold that gate
through publication and capture; SQLite runs only after releasing transcript and
chat locks. Reactions run after releasing the capture gate too, so reentrant
commands cannot deadlock it. Existing fast-response and pipe-deadlock tests still
exercise the command/publication boundary.

Providers can accept streaming input while another message is executing. The
current native protocols do not always identify which accepted message a later
terminal completes. In that case both starts remain recorded, ambiguous
turn completion/usage attribution stays unknown, the observed main execution
still ends, and capture health marks incomplete coverage. Do not overwrite the
older turn with the newer message or invent a completion. Subsequent autonomous
activity is an execution without a new message turn. This is an explicit
coverage limit, not a change to streaming-input behavior.

Execution intervals represent observed lifetimes, including tools and potentially
pending requests; they are not GPU time. Independently identified children get
separate opaque intervals. Aggregate task signals never invent child counts.
Queries return both **summed execution duration** and **union duration**: two
concurrent ten-minute intervals yield twenty accumulated minutes and ten minutes
with active agents. A parent conversation union is not an extra execution.

Explicit requests have paired identities. Received and cancelled request counts
remain separate; union their closed intervals for elapsed human wait time.
No gap between turns implies human work or waiting. Wait intervals can overlap
execution; these clocks do not partition wall time.

Durations use monotonic `Instant` measurements. UTC places intervals historically.
Provider-reported duration stays separate; Antigravity's locally measured adapter
duration is not misreported as provider duration. An end before its start, or a
wall/monotonic disagreement over one second, is excluded from historical overlap
calculations and counted as a clock anomaly. Starts without observed ends remain
incomplete after crashes, stop/process loss or restart; no guessed end is added.

## Usage and model attribution

All measurements carry `usageScope: mainAgent`. Child consumption is not allocated
to a message turn. `complete` indicates known complete input/output totals for
that main turn; false preserves useful partial observations. A measurement has
nullable `selectedModel`, nullable `observedModels`, `usage` and nullable
`usageByModel` rows (`model`, `usage`). Selection alone never proves observation.

`usage` contains nullable `inputTokens`, `outputTokens`, `cacheReadTokens`,
`cacheWriteTokens`, `reasoningTokens`, `contextUsed`, `contextWindow`, `peakContext`,
`modelCalls`, `compactions`, `cacheRebuilds` and `costUsd`.

- Input totals include cache read/write; output totals include reasoning. Do not
  add subsets again. Context occupancy/capacity is not consumption.
- Unknown stays null. Validation rejects negative/non-finite costs, impossible
  subsets, invalid IDs/providers and unrecognized payload fields.
- `modelCalls` counts observed distinct model messages, not blocks or tools.
  Compactions require explicit signals. Cache rebuilds remain unknown.
- Usage observations are snapshots. Queries use the final measurement, or the
  latest partial one when no terminal exists; they never sum both.
- Per-model input rows can be partial. They are not added to the aggregate and
  cannot justify distributing unknown output or cost percentages.

| Provider | Normalization and limits |
| --- | --- |
| Claude | Result `usage` covers the current main turn. Input combines noncached input, cache read and cache creation only when all are known. Distinct assistant message IDs provide partial input, observed models, call counts and per-model input; output placeholders are ignored until result. Session-cumulative cost uses a verified same-session baseline, or zero only for a known fresh process. Restored/reset/overlapping-child spend remains unattributed. Duplicate result UUIDs are ignored. |
| Codex | Thread totals are cumulative; subtract a known start baseline. New threads establish zero; resumed threads need a prior observation before a turn can have a verified baseline. Reset invalidates complete attribution and retains previously known partial usage. `last.totalTokens` is context only. Cache/reasoning remain subsets. Native duplicate terminals are ignored. Cost, call counts and observed model breakdown remain null without verified evidence. |
| Antigravity | Lifecycle and local elapsed time are captured. Native cumulative token/cost fields remain unknown. Interactive request and independent child visibility remain unavailable. |

Provider semantics were checked against the official
[Claude cost/usage contract](https://code.claude.com/docs/en/agent-sdk/cost-tracking)
and [Codex token protocol](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs).
Claude's cost is a provider-client estimate, not authoritative billing.
Adapter fixtures are deterministic regression evidence; they do not claim a live
CLI conformance run for every installed version.

## Relations and queries

Workspaces may relate to multiple repository/PR pairs. A later association makes
existing workspace history queryable without rewriting events. A PR query returns
**related workspace consumption**, not exclusive PR cost. Each workspace/turn
contributes once within a query; different PR query totals can overlap.
Archive, worktree cleanup and removal from the board do not remove event history.

Typed IPC and the browser mock expose:

- `telemetry_summary({ filter })`: counts, known sums, interval clocks, workspace
  IDs and capture/measurement coverage.
- `telemetry_events({ filter, cursor? })`: at most 500 events, nullable next cursor
  and health. Cursor is `(occurredAt, sequence)`.
- `telemetry_export({ filter, path })`: writes a private `.jsonl` file selected by
  the native save dialog. Streaming uses a private temporary file beside the
  destination; only a complete, flushed and synced export replaces it by atomic
  rename. Read, decode or write failures preserve the previous file and remove
  the temporary file. Destination checks reject symlinks and non-regular files;
  rename never follows a symlink introduced after the final check. Parent
  permissions remain unchanged. Success also requires syncing the containing
  directory after rename. If that final sync fails, export reports an error
  although the complete replacement may already be visible; its crash durability
  has not been confirmed.
- `telemetry_clear()`: deletes **all** local telemetry, irrespective of UI filters.

Filter fields are optional `from`, `to`, `workspaceId`, or a paired
`repositoryId`/`pullRequest`. Dates use half-open UTC `[from,to)`; UI date inputs
convert local calendar days to those bounds, including the selected final day.
Event counts/pages/exported rows use occurrence time. Turn usage uses the start
cohort, so a terminal outside the period still belongs to its originating turn.
Closed execution/wait intervals are clipped to the period. Unknown open intervals
before the period's upper bound contribute only to incomplete counts.

Summary fields distinguish null from measured zero, complete from partial token
coverage, starts from completions, received from cancelled waits and clock
anomalies. Settings resolves current workspace names from the board, with a
localized fallback for missing entries. It offers period/workspace filters,
refresh, export and confirmed deletion without a large dashboard. `workspaceIds`
contains every retained workspace matching the PR relation, independent of the
period and workspace selection. Refresh reloads names and options while keeping
the selected workspace, including a fallback when it has disappeared.

## Privacy, retention and deletion

Events contain opaque IDs, provider/model identifiers, PR numbers, timestamps,
measurements and closed enums. They exclude prompts, replies, source, diffs,
file contents, commands, arguments, answers, raw errors, task descriptions,
paths, repository URLs and branch/project/workspace/conversation names.
Native request/child IDs are mapped to UUIDs; raw IDs stay only in process memory.
No Cloud, relay or analytics upload is added.

Retention is indefinite until deletion. JSONL starts with versioned metadata
(`exportVersion: 1`), filters, health, query semantics and summary, then canonical
events. A period export may omit a start/completion outside its event window;
metadata states the start-cohort distinction. Export without filters for the
complete retained dataset. Transcripts and board files are never exported here.

An abrupt process exit or power loss can leave a private (0600)
`.prometeu-telemetry-*.tmp` sibling in the selected export directory. Ordinary
errors remove it, but crash recovery does not track or sweep external export
directories. These incomplete exports may contain metadata and can be removed
manually when no export is running. Automatic recovery would need ownership and
active-export tracking; it is outside the current deletion boundary, like
completed exports and system backups.

Failed capture never rejects an agent command. Diagnostics contain only a fixed
message. A small private `telemetry-health.json` stores failure count and time;
settings and exports expose incomplete history without event toasts or modals.
An unavailable store stays unavailable until app restart. If all persistence
fails, health can only survive in memory; no alternative event log is written.

Clear waits for active snapshot readers/exports, serializes with capture,
advances the capture generation, securely deletes rows, vacuums retained pages,
truncates the WAL and removes capture-health history. A successful clear cannot
leave an older snapshot still exporting erased history. Old generation
callbacks cannot restore events; known old child/request identities remain only
as in-memory suppression markers. Journey/preparation/PR-discovery operations
also carry their originating generation. New accepted activity can start fresh
history; replay never backfills it. If a message arrives before an erased main
run ends, its start remains incomplete and old usage/terminal observations are
discarded until that run ends; no erased execution end is republished.
If erasure fails, report failure rather than claim success. Board state and
transcripts stay intact. Exported files and system
backups are outside this boundary; no forensic media erasure is promised.

## Verification

- `src-tauri/src/telemetry/tests.rs`: commit/reopen, deduplication/conflicts,
  future-version preservation, token cohorts, overlap/clipping, unknown durations,
  waits/cancellation, clock anomalies, content exclusion, PR relations, partial
  snapshots, private permissions, symlink rejection, atomic export replacement
  and preservation on late decode/write failures, deletion generations/pages,
  post-clear late terminals, concurrent snapshot reads/commits, streaming export,
  WAL erasure, existing-database compatibility and capture failures.
- `claude.rs::telemetry_main_usage_deduplicates_steps_and_excludes_restored_cost`
  and `codex.rs::telemetry_uses_thread_deltas_not_context_and_invalidates_resets`:
  provider normalization, deduplication and reset/resume boundaries.
- Existing `chat.rs` command/publication, failed-send, request-response and
  background settlement tests; existing board and transcript compatibility tests.
- `src/telemetry.test.ts`: local calendar bounds, mock start-cohort filtering,
  paging/export and persistent deletion. `src/telemetry-settings.test.ts` checks
  refreshed workspace choices and preserved selection. `src-tauri/tests/mock.rs`
  checks IPC parity.

Follow the [E2E scope policy](../operations/development.md#e2e-scope): aggregation
and persistence are checked below the browser; settings reuses already-tested
shared controls and confirmation dialogs. Provider differences also appear in the
[provider matrix](../quality/provider-matrix.md).
