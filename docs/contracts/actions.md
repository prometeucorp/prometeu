# Reusable actions and agents

Status: implemented; decision in [ADR 0009](../decisions/0009-reusable-actions.md).

## Registry and entries

`Board.actions` contains `profiles`, `commands`, `overrides`, `pr_action` and
`defaults_initialized`. On first open, earlier catalogs receive the editable
**Code review** profile and the `/review` command. The profile reports bugs and
risks without modifying files or publishing a PR. The initialization is recorded
so later removals and customizations are respected; existing names or identities
are not overwritten. The source JSON is in
[`action-defaults.json`](../../src/action-defaults.json). The registry is local,
reusable across projects, and is not sent to the relay.

- A command has `name`, `description`, `kind: prompt | agent`, `prompt` and an
  optional `profile`. Names are unique, with 1–64 lowercase ASCII letters,
  digits or hyphens; `compact` and `context` are reserved.
- `prompt` expands text in the current box without sending it. The remaining
  text follows the expansion and stays editable.
- `agent` starts a task in another tab. The request contains the command text,
  the title, the workspace's repositories/bases and the context written by the
  person.
- `/name` and the **Actions** menu use the same registry. If the provider also
  exposes `/name`, its command keeps the name and the app's command uses
  `/prometeu:name`. The explicit namespace also works without a collision.
  Suggestions for commands registered in the app show the **Prometeu** tag;
  the provider's commands and skills do not get that tag.
- `pr_action: null` keeps the PR request in the current conversation, including
  the preference for the repository's skill. A configured name points to an
  `agent` command and serves the **Open PR / Update PR** button.

## Profile and execution

A profile has identity, name, prompt, `choice` (provider/model/effort), MCP,
plugins, skill names, `permission: ask | auto` and an optional `watch`.
`overrides[project][profile]` fully replaces a profile for that project. It does
not change the global profile.

Each `Tab.task` stores a copy of the resolved profile, the command name and the
execution state. `null` MCP/plugins inherit the workspace selection at the
start; empty lists are explicit selections. If the workspace is also `null`, the
provider's external configuration remains. The catalog contains no secrets:
credentials stay in the existing hubs.

Changing the profile or the workspace selection does not change that copy.
Resuming uses the same profile. Changing the model through a task's footer is
refused; edit the profile for future executions. Codex plugins and MCP use a
configuration derived per task session, so the selection of another conversation
is not changed.

Skills are names instructed to the agent, available in the installation or in
the selected plugins. That list is not an allowlist and does not disable the
provider's other skills. If a skill is not available, the instruction is to stop
and report. The app does not promise automatic detection of that textual result.

Permissions are materialized by the adapter: Claude uses its normal approval
mode under `ask`; Codex uses `approvalPolicy: untrusted`. `auto` keeps the
existing bypass. These options do not constitute worktree isolation.

An untracked task finishes when its turn finishes. Tracking is optional and does
not change the workspace's manual stage. Repeating the same command with a task
still open returns the existing tab. If there is additional context, the app
refuses and preserves the draft so it can be sent in the task's tab. Starting
another task requires that no conversation is working, waiting for an answer or
holding a pending message.

## PR tracking

`watch` defines an interval in seconds (30–86400), comments, CI results and a
limit of automatic turns (1–100). The shipped example uses 60 seconds and 10
turns. PRs are discovered by branch and then pinned to the number per
repository. Repositories without a PR do not prevent the found PRs from
completing.

A backend thread queries `gh`; there is no model turn while waiting. The queries
are sequential and have a 30-second deadline per process. The interval is a
minimum, not a guarantee of real-time delivery. General comments, reviews and
line comments use pagination. Comments from the authenticated account are
ignored to avoid feedback loops. CI results include success, failure, error,
timeout, cancellation and action required; pending states do not wake the agent.
The identifier includes the commit and the check run when available.

The execution stores `seen` (SHA-256 hashes of the events), PRs, the query
timestamp, the turn count, `paused`, `done` and the last error. Repeated events
do not generate a turn; comment edits do. While some conversation is working or
waiting for an answer, news stays unconfirmed and is grouped into the next
query. The cursor and the pending message are written together before sending.
A pending message survives a restart and can be resumed. There is no
exactly-once guarantee in the window between the CLI accepting the message and
the persistence of the transcript.

External data does not grant permissions: it arrives identified as comments or
CI results. Each comment body is limited to 12000 characters; the URL
accompanies the text for full inspection. `gh` responses above 8 MiB are
refused. The turn limit pauses the execution; resuming it renews the limit.

Closing every found PR completes the tracking. Pausing, archiving or cleaning
the workspace suspends the queries; closing the tab removes the task. Pausing
does not interrupt a turn already in progress. A closed app or a suspended Mac
does not query; the next open reconciles the news. Query failures stay visible
and preserve the cursors; a turn failure or interruption pauses the tracking.

General PR discovery is separate from this explicit task monitor. It starts
when the app opens, scans one clone once for all of its eligible workspace
branches, and refreshes at most every three minutes on foreground AC, five
minutes on foreground battery or unknown power, and 15 minutes while hidden or
unfocused. Returning to the foreground requests a scan immediately. Opening a
workspace also refreshes its own repositories immediately. Archived, cleaned
and branchless workspaces are excluded from general discovery; recorded PRs
remain available on archived workspaces. A second general request during a
scan queues one follow-up. Each general `gh pr list` has a 15-second deadline
and a 2 MiB output cap. Failed, timed-out, empty or incomplete results preserve
known PR metadata. The monitor keeps its configured interval, 30-second query
deadline, pagination, cursors and pending delivery. Its richer PR, comment and
CI queries cannot reuse the general listing's field set or freshness guarantee.

## IPC and compatibility

- `actions_save({ catalog }) -> void`: validates references, names and limits;
  persists to the board and publishes the `board` event.
- `action_start({ workspace, name, context }) -> Tab`: resolves the profile,
  creates a local session and starts the request. A spawn error stays in the tab
  for inspection.
- `action_pause({ session, paused }) -> void`: pauses or resumes tracking.

New fields are additive with defaults in persistence. Ordinary sessions do not
change their launch configuration. The web mock implements the registry and tab
creation, but does not query GitHub and does not run models. Evidence:
[`actions.test.ts`](../../src/actions.test.ts),
[`actions.rs`](../../src-tauri/src/actions.rs),
[`github.rs`](../../src-tauri/src/github.rs),
[`actions.spec.ts`](../../e2e/actions.spec.ts).
