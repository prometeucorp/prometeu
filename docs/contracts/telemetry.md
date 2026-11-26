---
type: Contract
title: "Local telemetry and insights"
description: "Local events without content, usage measurements, SQLite storage and insight queries."
status: stable
---

# Local telemetry and insights

This contract specifies the local usage insights and context insights that the
app has. The decision is in
[ADR 0059](../decisions/0059-local-telemetry-foundation.md).

## Scope and ownership

The Rust backend records product transitions and observed executions locally.
The feature includes these items:

- Pages of indexed events.
- Summaries for each period and for each workspace.
- Queries about PRs.
- An accurate lookup of a reply.
- Breakdowns by workspace, model and source.
- A JSONL export.
- The deletion of the full history.

Completed replies and workspace headers show the usage. Settings / **Analytics**
contains the Usage screen. This screen has these items:

- Filters for the period and the workspace.
- The token composition and the totals for each source.
- A list of sessions in sequence of their usage, and session details.
- The observed models. The feature does not
include Cloud sync, billing, prices, rankings of tool calls, organization
analytics or a backfill from transcripts.

- Adapters normalize measurements. Native usage objects do not go across the
  boundary of the store. `usage.updated` stays the account quota data.
  `Tab.tokens` stays an estimate of the context growth. These two values are not
  the historical token consumption.
- `chat.rs` records only the live output of local processes and the application
  commands that it wrote correctly. Replay, remote mirrors and snapshots do not
  record the data again. Remote control operates through the same backend
  command path of the owner. The backend removes internal capture fields before
  it keeps or shares the conversation. The added canonical
  `turn.completed.usage` is an explicit conversation payload. The app can keep
  and share it under the current consent of that conversation.
- `telemetry.rs` owns the typed facts, the validation, SQLite, the capture
  health and the erasure. `telemetry/capture.rs` owns the logical turns, the
  execution and request intervals, and the scope. `telemetry/query.rs` owns the
  queries. Consumers do not send SQL.
- Journey facts start in workspace and tab operations that complete correctly.
  The discovery of GitHub PRs records new relations. An accepted message also
  records all known current PR relations. The app does not make data for
  previous work from transcripts.

## Storage, identities and compatibility

`<root>/telemetry.sqlite3` obeys the release and debug roots and
`PROMETEU_ROOT`. The folder is private (0700), and the database is 0600. The
bundled SQLite has these settings:

- WAL journaling.
- FULL synchronous commits.
- Secure deletion.
- A busy timeout of 250 ms.

The WAL file and the shared-memory file get the permissions of the database.
Each event uses a short transaction. When an append returns without an error,
the commit is complete. No batch queue in memory tells that its data is durable.
`PRAGMA user_version` is the migration version of the database. The event
`schemaVersion` is a different value. The first version is 1 for the two values.
The app keeps future database versions that it does not know, and it shows them
as not available.

One event table contains the columns that all events have and typed JSON
payloads. Unique event IDs and occurrence keys make the capture idempotent. A
retry with the same data keeps the two identifiers, the timestamp, the scope and
the payload. It has no effect. A retry with different data is a capture failure.
If the app sees the same relation between a workspace, a repository and a PR
again, it does not make a second association.

| Envelope field | Meaning |
| --- | --- |
| `sequence` | The local sequence of insertion. It is not a global identity or a sync cursor. |
| `id` | The UUID that the app gives before the append. |
| `occurrenceKey` | The stable identity of the fact. It is different from the event UUID. |
| `schemaVersion` | The canonical event version. At this time, it is 1. |
| `occurredAt` | The time of the first live observation, in UTC Unix milliseconds. |
| `recordedAt` | The time of the local append, in UTC Unix milliseconds. |
| `category` | `journey` or `work`. It comes from the closed event discriminant. |
| `type` | The closed event discriminant. |
| `projectId`, `workspaceId`, `conversationId`, `turnId` | Opaque scope IDs. An unknown value is null. |
| `provider` | `claude`, `codex`, `antigravity`, `cursor`, or null. |
| `payload` | Typed JSON for the event. It does not contain arbitrary native properties. |

Indexes on time, on workspace and time, on turn and on execution and request
make lookups fast. Current version-1 databases get the added indexes and change
to WAL. The app does not write the events again. Event pages use keyset
pagination with a maximum number of events on each page. Queries open a
different read-only connection and transaction after they release the capture
mutex. WAL snapshots let live commits occur during summaries and exports.

SQL puts each turn with its last measurement, or with its latest partial
measurement. It also puts the observed intervals in pairs. Rust reads the rows
in sequence. It does not keep the event history or arrays of intervals. An
export streams JSONL from the same snapshot that its summary uses. SQLite uses a
page cache with a maximum dimension and sorts in temporary files. The workspace
options are the only collection in the summary that becomes larger with the
history. The query duration and the WAL file can become larger with the history
while a reader is active.

Some current project IDs are paths. `Board.telemetry_ids` is a map that is empty
by default. It gives UUID aliases to projects, repository clones and pairs of
repository and branch. The app uses the workspace and conversation UUIDs again.
Legacy IDs that are not UUIDs get aliases. Names and paths stay only in the
board and domain boundary that was there before. They do not go into events or
exports.

The alias maps stay after the removal of board entries. Thus, the identity stays
stable if a person adds a repository to the board again. When a person deletes
the telemetry history, the domain identities do not change. They contain no
execution history.

The board publication prepares aliases before it makes a snapshot. The journey
capture flushes the board before it records these new aliases and the changes of
the archive state. The first message on a process also flushes the board. A
failure shows that the coverage is not complete, but it does not refuse the
agent command.

Logical turn IDs are durable in `turn.started`. A migration of the transcript
format or a historical backfill is not necessary. A process that starts again
keeps the previous turns that are not complete open. It does not give its new
work to them. The archive and finish commands wait for the flush on a Tauri
worker thread. There is no atomic transaction across the JSON board, the
provider process and SQLite.

## Event vocabulary

| Category | Event | Payload and condition |
| --- | --- | --- |
| journey | `workspace.created` | `mode: worktree | repository`. The app added the workspace correctly, before the asynchronous preparation. |
| journey | `workspace.archived`, `workspace.resumed` | An empty payload. An actual change of the archive state. It is not a view that opens or a process that starts again. |
| journey | `conversation.created` | An empty payload. The app published a new logical tab. |
| journey | `provider.selected` | `scope: workspace | conversation`. The first selection at launch, or an explicit selection for a tab that the person committed. |
| work | `pull_request.associated` | `repositoryId`, a nullable `branchId` and `pullRequest`. A relation that the app recorded for the first time. |
| work | `pull_request.observed` | The repository and branch IDs, the PR number and the lifecycle timestamps and state, the snapshot ID, the number of candidates, the start limit of the query and the completeness. It has no title and no branch name. |
| work | `app.call.started`, `app.call.completed` | An opaque `callId`, a closed source and a measurement. The completion adds the outcome and the elapsed duration. |
| work | `turn.started` | The first `measurement` and an `origin` with a default value. The app wrote a message correctly to the ready process. |
| work | `turn.completed` | `outcome: ok | error | interrupted`, `elapsedMs`, a nullable `providerDurationMs` and `measurement`. |
| work | `turn.usage.observed` | `measurement`. It is a snapshot, not an increase of a counter. |
| work | `agent.execution.started` | An opaque `executionId`. An observed main execution or child execution. |
| work | `agent.execution.completed` | The same `executionId` and a monotonic `elapsedMs`. |
| work | `human_input.requested` | An opaque `requestId` and `kind: approval | question | plan`. |
| work | `human_input.received`, `human_input.cancelled` | The same `requestId` and a monotonic `elapsedMs`. An accepted answer or a refusal is different from a cancellation. |
| work | `context.compacted` | A nullable `before` and `after`. Only an explicit observation from the provider. |

The creation of a workspace does not prove that the preparation was correct. The
app does not make a generic `work.completed` event. A PR association is not the
creation of a PR. An archive is not a completion, and a merge is not a
deployment. Different `pull_request.observed` facts record the lifecycle
evidence of the provider. They also record full snapshots of the branch history
for tenure. If the evidence is missing or not complete, the association stays
related only. The branch identity comes from the actual head of the worktree or
the observed PR head. It does not come from a workspace label that is not
current.

`turn.completed.messageKey` is an optional hash of a reply in the scope of a
conversation. The app uses it to restore the accurate footer of a reply. Older
completions have no key by default.

## Turns and time

A message starts after the app writes a command correctly. Failed sends do not
start turns. Prompts in the queue for setup or a new start also do not start
turns. Provider echoes, responses to requests, replay and internal turn
notifications do not make new message turns. The completion of the main agent
closes its telemetry turn independently of the child agents. The readiness,
Stop, notification and delegation rules of the conversation stay those of
[ADR 0056](../decisions/0056-background-tasks-hold-completion.md).

Each Pump has a mutex for the capture sequence. The core `SessionOutput` and
`capture_command` control the telemetry ports that the composition gives them.
`chat/host.rs` adapts these ports to the current capture service and SQLite. The
output and the accepted input hold that gate through the publication and the
capture. SQLite runs only after the app releases the transcript locks and the
chat locks. Reactions also run after the app releases the capture gate. Thus,
reentrant commands cannot cause a deadlock on it. The current tests for fast
responses and for pipe deadlocks continue to examine the boundary between the
command and the publication.

A provider can accept streaming input while a different message operates. The
current native protocols do not always identify which accepted message a
subsequent terminal completes. In that condition, these rules apply:

- The two starts stay recorded.
- The attribution of the turn completion and the usage stays unknown.
- The observed main execution stops as usual.
- The capture health shows that the coverage is not complete.

Do not replace the older turn with the newer message. Do not make a completion.
Subsequent autonomous activity is an execution without a new message turn.
This is an explicit limit of the coverage. It is not a change to the behavior
of streaming input.

Execution intervals show observed lifetimes. They include tools and possibly
requests that wait. They are not GPU time. Child agents that the app identifies
independently get different opaque intervals. Aggregate task signals do not
make child counts. Queries return the **summed execution duration** and the
**union duration**. For example, two ten-minute intervals at the same time give
twenty minutes in total and ten minutes with active agents. The union of a
parent conversation is not an added execution.

Explicit requests have identities in pairs. The counts of received requests and
cancelled requests stay different. To get the elapsed wait time of the person,
calculate the union of their closed intervals. A period between turns does not
show that a person did work or waited. Wait intervals can overlap the execution.
These clocks do not divide the wall time.

Durations use monotonic `Instant` measurements. UTC puts the intervals in the
history. The duration that the provider reports stays different. The adapter for
Antigravity measures a duration locally, and the app does not show it as the
provider duration. The app does not use these intervals when it calculates the
historical overlap:

- An interval with an end before its start.
- An interval where the wall clock and the monotonic clock have a difference of
  more than one second.

The app counts these intervals as clock anomalies. After a crash, a stop, the
loss of a process or a new start, a start without an observed end stays not
complete. The app does not add an end that it guesses.

## Usage and model attribution

Measurements have `usageScope: mainAgent` or `wholeTree`. The second value is
possible only with verified provider totals for the main agent and the child
agents in that turn. The app does not use the second value only because child
agents are there. `complete` tells that the full input and output totals for the
scope are known. When it is false, the measurement keeps useful partial
observations. Resumed counters alone cannot prove the start baseline of a new
turn.

A measurement has a nullable `selectedModel`, a nullable `observedModels`,
`usage` and nullable `usageByModel` rows (`model`, `usage`). A selection alone
does not prove an observation.

`usage` contains nullable `inputTokens`, `outputTokens`, `cacheReadTokens`,
`cacheWriteTokens`, `reasoningTokens`, `contextUsed`, `contextWindow`,
`peakContext`, `modelCalls`, `compactions`, `cacheRebuilds` and `costUsd`.

- The input totals include the cache reads and writes. The output totals include
  the reasoning. Do not add these subsets again. The context occupancy and the
  context capacity are not consumption.
- An unknown value stays null. The validation refuses negative or infinite
  costs, subsets that cannot be correct, incorrect IDs or providers and unknown
  payload fields.
- `modelCalls` counts the different model messages that the app sees. It does
  not count content blocks or tools. A compaction count must have explicit
  signals.
- A cache rebuild must have an observed previous call with context. It must also
  have a minimum of 1,000 cache-write tokens. The writes must be a minimum of
  half of the input context of the previous call. The first observed call is not
  a rebuild. Content blocks that occur again do not make more calls. If the
  provider does not give evidence for each call, the rebuild count stays
  unknown.
- Usage observations are snapshots. Queries use the last measurement, or the
  latest partial measurement when there is no terminal. They do not add the two.
- The input rows for each model can be partial. The app does not add them to the
  aggregate. The app does not use them to divide unknown output or cost as
  percentages.

| Provider | Normalization and limits |
| --- | --- |
| Claude | The result `usage` includes the main turn. The input includes the input that is not in the cache, the cache reads and the cache creation. Verified cumulative `modelUsage` deltas can cover the whole tree and the observed models. If a baseline is missing, reset or resumed, or if child activity goes across
the turn, the coverage stays the main turn. Different assistant IDs give partial
input and observations of calls and rebuilds. The app does not add placeholder
output. The cost uses a verified baseline from the same session or a known new
zero. The app ignores duplicate result UUIDs. |
| Codex | Thread totals are cumulative. The app subtracts a known start baseline. A new thread starts at zero. A resumed thread must have a previous observation before a turn can have a verified baseline. A reset makes the full attribution not correct and keeps the partial usage that the app knew before. `last.inputTokens` is the current input context, not consumption. The cache and reasoning values stay subsets. The app ignores duplicate native terminals. The cost, the call counts and the breakdown by observed model stay null without verified evidence. |
| Antigravity | The usage of the current step, without duplicates, gives partial input and output. This includes tool steps without text. Cumulative result deltas from a known new start or from the same process can make it complete. The app does not count a resumed cumulative result as a new turn. The cost, the window and the breakdown by model stay unknown. Interactive requests are not available. The app cannot see child agents
separately from the turn. |

The provider meanings agree with the official [Claude cost/usage
contract](https://code.claude.com/docs/en/agent-sdk/cost-tracking) and the
[Codex token
protocol](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/protocol.rs).
The Claude cost is an estimate from the provider client. It is not the
authoritative billing. Adapter fixtures are deterministic regression evidence.
They do not prove a live CLI conformance run for each installed version.

## Application calls and provenance

`app.call.started` and `app.call.completed` put an opaque `callId` in a pair
with a closed `source: naming | plugin-maker`. They also have measurements and
the observed outcome and duration. They add to the usage and the source groups.
They do not increase the counts of message turns or conversations. The naming
calls belong to their workspace. The global plugin creation has no workspace,
and the app does not make one for it. The app reads the capture generation
before it starts the work. Thus, a completion after the history deletion cannot
restore the deleted call.

`turn.started.origin` contains the nullable opaque `actionId`/`delegatedBy` and
the observed pairs of repository and branch. This is the provenance of the
conversation. A manual follow-up in a conversation that an action made keeps
that provenance. The origin does not identify who started each message in the
queue. The app does not examine a prompt or a command to find the origin.

The source groups divide the conversation, naming and plugin-maker measurements
without overlap. The action and delegation origin groups are views of the
conversation usage that can overlap. They are not more consumption. A turn with
the two origins is in the two groups. The app does not add it again to the
workspace total.

## Conversation and workspace presentation

The footer of a completed reply uses the provider duration when it is available.
It shows the known consumption and the normalized CLI cost. It also shows a
token breakdown that includes the subsets, and information about the
measurement scope and the coverage. The legacy top-level `costUsd` is not a
safe fallback for the cost. The native Claude replay restores the accurate
reply metadata from the local history. Remote mirrors do not query the local
history of a different machine.

The current context uses `context.updated` or the latest completion measurement.
It uses the optional window that the tab keeps for the attachment. The meter
shows a warning at 60% or 220,000 tokens, at the first of the two. It shows a
high level at 80%. These limits are only recommendations. They do not guarantee
the quality.

Compact, Context report and New conversation are explicit actions. The
capabilities of the agent control them. A context command keeps the draft. The
desk frames of the desktop use the same ChatView controls. If the window is
unknown, the app does not show the meter. The compaction labels show automatic
or manual only when the provider reports it. An open context panel follows the
current availability of the actions. An update keeps the focus on an available
action. If that action becomes disabled, the focus goes back to the panel.

Workspace headers show one total and a panel. The panel shows the token subsets,
the known cost and the coverage. It also shows the execution and wait clocks,
the observed models, the conversations, the app-call sources and the PR
evidence. Finish and archive include the read-only total without an added
approval step. For missing or removed names, the app shows localized fallbacks
with opaque IDs. Observed model rows keep their provider label when the app
knows it. Thus, the same model ID from different providers stays different.
Providers that are not in the current catalog keep their recorded identifier.
The app does not guess a missing provider.

Local `telemetry-changed` invalidations refresh the current workspace after
these items:

- A turn completion.
- An app-call capture.
- A complete capture of the PR history.

Board changes do not control these invalidations. The main completion can change
the usage while child activity keeps the tab in operation. These events contain
no content. The app ignores responses that come after a navigation or a history
deletion. The turn capture occurs before the current board update.

## Relations and queries

A workspace can have a relation to many pairs of repository and PR. A subsequent
association lets queries find the current workspace history without a new
write of the events. A PR query returns the **related workspace consumption**.
It is not the exclusive cost of the PR. Each workspace and turn adds to a query
one time. The totals of different PR queries can overlap. Archive, worktree
cleanup and removal from the board do not remove the event history.

Insights also mark each PR row as `tenure` or `related`. A tenure row must agree
with these conditions:

- An observed repository and branch on the turn.
- Correct PR lifecycle dates.
- A complete snapshot of the branch history. The query of this snapshot
  started after the turn.

The app must keep all candidates in the snapshot before it can use the snapshot.
A history that has a maximum count or a partial capture stays related only. The
app gives the turn to the first PR with a tenure that did not stop before the
start of the turn. This includes work before the PR opened. Closed or merged PRs
use their observed end time. The app does not divide unknown evidence or
evidence that does not agree. Associations with many repositories can overlap.
The app does not add them to the workspace total as different consumption.

Typed IPC and the browser mock give these calls:

- `telemetry_summary({ filter })`: counts, known sums, interval clocks,
  workspace IDs and the coverage of the capture and the measurements.
- `telemetry_insights({ filter })`: the summary, the known consumption and these
  groups: conversation, observed model, source, origin groups that can
overlap and PR rows with their qualification.
  - The app adds counts one time. The aggregate context occupancy and window are
    null. The peak context is a maximum. A conversation that changes its
    provider stays one conversation.
  - Conversation groups also contain `coverage` with `measuredTurns`,
    `partialTurns`, `unknownTurns`, `mainAgentTurns` and `wholeTreeTurns`. The
    first three values do not overlap. They count turns with full input and
    output totals, turns with some known consumption, and turns where the two
    sides are unknown.
  - The scope counts show the scope of each selected measurement. They do not
    divide the consumption between parent and child agents. Each start adds one
    time through its last or latest measurement. This includes a terminal out of
    the period.
  - Other group types do not have `coverage`. Older backends can also send
    conversations without it. Consumers continue to use those rows, but they do
    not show full coverage or a known scope. This is an added query result.
    There is no stored event and no database migration for it.
- `telemetry_turns({ conversation, messageIds })`: a maximum of 500 requested
  assistant IDs. It returns only completed turns that agree accurately, with the
  duration and
the usage. The store contains the SHA-256 of the conversation UUID, a NUL
  separator and the assistant ID. It does not contain the raw native identifier.
  Events from earlier versions without a key do not get a footer. Reads do not
  record or backfill history.
- `telemetry_events({ filter, cursor? })`: a maximum of 500 events, a nullable
  next cursor and the health. The cursor is `(occurredAt, sequence)`.
- `telemetry_export({ filter, path })`: writes a private `.jsonl` file that the
  person selects in the native save dialog. These rules apply:
  - The stream goes to a private temporary file adjacent to the destination.
    Only a complete export, after the flush and the sync, replaces the
    destination with an atomic rename.
  - If a read, decode or write operation has a failure, the app keeps the
    previous file and removes the temporary file.
  - The destination checks refuse symlinks and files that are not regular files.
    The rename does not follow a symlink that a process adds after the last
    check. The permissions of the parent folder do not change.
  - For a correct export, the app must also sync the folder after the rename. If
    that last sync has a failure, the export shows an error. It is possible that
    the full replacement is there at that time, but its durability after a crash
    is not known.
- `telemetry_clear()`: deletes **all** local telemetry. The UI filters have no
  effect on it.

The filter fields are optional: `from`, `to`, `workspaceId`, or a pair of
`repositoryId`/`pullRequest`. Dates use the half-open UTC range `[from,to)`. The
UI date inputs change local calendar days into those limits. This includes the
last selected day. Event counts, event pages and exported rows use the time of
the occurrence. The turn usage uses the start cohort. Thus, a terminal out of
the period continues to belong to the turn that started it. The app cuts closed
execution and wait intervals to the period. Unknown open intervals before the
end of the period add only to the counts of items that are not complete.

The summary fields show these differences:

- Null and a measured zero.
- Complete and partial token coverage.
- Starts and completions.
- Received waits and cancelled waits.
- Clock anomalies.

Settings gets the current workspace names from the board, with a localized
fallback for missing entries. It reads one insights snapshot. Thus, its known
consumption includes app calls. This includes the global plugin-maker calls
when the person selects no workspace filter. Settings gives filters for the
period and the workspace, a refresh, an export and a deletion with
confirmation.

The Usage screen adds the known input and the known output. It does not add the
cache or the reasoning again. It keeps the values that are not available
different from a measured zero. The Usage screen does not show these items:

- Fractions of collection counts.
- A headline value for the completion coverage.
- Paragraphs of explanation below the token and source panels.
- The previous badges for known values and for values in the total.

Session rows keep the measurement status and a read-only detail dialog.
Observed model rows stay a partial view of the total. The app does not add
origin groups to the source partition. The app gets session names from the
current board tabs, with fallbacks that use opaque IDs.

`workspaceIds` contains all kept workspaces that agree with the PR relation. The
selected period and workspace do not change it. A refresh loads the names and
the options again and keeps the selected workspace. This includes a fallback
when the workspace is not there.

`src/telemetry-settings.ts` owns the live invalidation and the native actions.
`src/usage-history.ts` changes snapshots into a presentation.
`src/components/usage-history.ts` renders the same view in Settings and in the
component gallery. Live updates keep these items:

- The date and workspace controls.
- The scroll position of the table.
- The focused session actions.
- The open detail dialogs.

Filter changes, a deletion and the removal of the Settings section cancel the
open reads. They also close the detail dialogs that are not current. Export
dialogs that wait cannot publish a selection after a person deletes the history.
A capture failure and storage errors stay different from a period that is really
empty.

## Privacy, retention and deletion

Events contain opaque IDs, provider and model identifiers, PR numbers,
timestamps, measurements and closed enums. They do not contain these items:

- Prompts, replies, source code or diffs.
- File contents, commands, arguments or answers.
- Raw errors or task descriptions.
- Paths or repository URLs.
- The names of branches, projects, workspaces or conversations.

The app changes native request IDs and child IDs into UUIDs. The raw IDs stay
only in the memory of the process. The reply lookup stores only a hash in the
scope of a conversation. The action and delegation provenance uses UUID aliases.
The names and the MCP client details stay in the current domain boundary. The
app does not add an upload to the Cloud, the relay or analytics.

The app keeps the data until a deletion. A JSONL file starts with versioned
metadata (`exportVersion: 1`), the filters, the health, the query rules and the
summary. Then it contains the canonical events. An export of a period can not
include a start or a completion out of its event window. The metadata tells
about the start-cohort difference. To get the full dataset that the app keeps,
export without filters. This command does not export transcripts or board files.

If the process stops suddenly or the electrical power stops, a private (0600)
`.prometeu-telemetry-*.tmp` file can stay in the selected export folder. The app
removes it after usual errors. But the crash recovery does not monitor or clean
external export folders. These exports that are not complete can contain
metadata. A person can remove them manually when no export operates. For
automatic recovery, ownership records and a record of active exports are
necessary. This recovery is not in the current deletion boundary. Completed
exports and system backups are also not in this boundary.

A capture failure does not refuse an agent command. Diagnostics contain only one
message that does not change. A small private `telemetry-health.json` stores the
failure count and the time. Settings and exports show that the history is not
complete. They do not show event toasts or modals. A store that is not available
stays not available until the app starts again. If all writes to the disk have a
failure, the health can stay only in memory. The app does not write a different
event log.

The clear operation does these steps:

1. It waits for the active snapshot readers and exports.
2. It serializes with the capture.
3. It increases the capture generation.
4. It deletes the rows with secure deletion.
5. It vacuums the kept pages.
6. It truncates the WAL.
7. It removes the history of the capture health.

After a correct clear, an older snapshot cannot continue to export erased
history. Callbacks from a previous generation cannot restore events. Known child
and request identities from the previous generation stay only as suppression
markers in memory. The journey, preparation and PR discovery operations also
have the generation in which they started. New accepted activity can start a new
history. The replay does not backfill it. A confirmed deletion also cancels the
open history reads of the frontend and deletes the restored footer caches.

The canonical conversation usage that the app keeps is transcript data. This
command does not remove it. If a message comes before an erased main run stops,
its start stays not complete. The app ignores the previous usage and terminal
observations until that run stops. It does not publish an erased execution end
again. If the erasure has a failure, the app reports the failure. It does not
show success. The board state and the transcripts do not change. Exported files
and system backups are not in this boundary. The app does not promise a forensic
erasure of the media.

## Verification

- `src-tauri/src/telemetry/tests.rs` examines these items:
    - Commit and reopen, duplicates and differences, and the protection of
      future
    versions.
    - Token cohorts, overlap and the cut to the period, unknown durations, waits
      and
    cancellation, and clock anomalies.
  - The exclusion of content, PR relations and partial snapshots.
  - Private permissions, the refusal of symlinks, the atomic replacement of
    exports and the protection of the file after late decode or write failures.
  - Deletion generations and pages, and late terminals after a clear.
  - Snapshot reads and commits at the same time, streaming export and WAL
    erasure.
  - The compatibility with current databases and capture failures.
- `claude.rs::telemetry_main_usage_deduplicates_steps_and_excludes_restored_cost`
  and
  `codex.rs::telemetry_uses_thread_deltas_not_context_and_invalidates_resets`
  examine the provider normalization, the removal of duplicates and the reset
  and resume boundaries.
- The current `chat.rs` tests examine the command and the publication, failed
  sends, requests and responses, and the background completion. The current
  board tests and transcript tests examine the compatibility.
- `src/telemetry.test.ts` examines the local calendar limits, the start-cohort
  filter of the mock, the pages, the export and the durable deletion.
  `src/telemetry-settings.test.ts` examines the refreshed workspace options and
  the kept selection. `src-tauri/tests/mock.rs` examines the IPC parity.

Obey the [E2E scope policy](../operations/development.md#e2e-scope). Tests below
the browser examine the aggregation and the storage. Settings uses shared
controls and confirmation dialogs again, and other tests examine them before.
The differences between providers are also in the
[provider matrix](../quality/provider-matrix.md).

The canonical usage and measurement types come from
`prometeu-core::conversation::usage`. The desktop keeps the ownership of the
database and of the stored formats.
