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
plugins, skill names and `permission: ask | auto`.
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

A task finishes when its turn finishes. Repeating the same command with a task
still open returns the existing tab. If there is additional context, the app
refuses and preserves the draft so it can be sent in the task's tab. Starting
another task requires that no conversation is working, waiting for an answer or
holding a pending message.

## PR discovery

There is no automatic PR follow-up: the app does not poll a task's PR comments
or CI and does not start agent turns on its own. General PR discovery starts
when the app opens, scans one clone once for all of its eligible workspace
branches, and is scheduled every three minutes on foreground AC, five minutes
on foreground battery or unknown power, and 15 minutes while hidden or
unfocused. Returning to the foreground requests a scan immediately when the
last request is at least one minute old; otherwise the schedule continues.
Opening a workspace also refreshes its own repositories immediately, and a turn
that settles in the open workspace refreshes them again, at most every 20
seconds per workspace, so a PR the agent created appears promptly. Turn state is
observed even while a menu or rename input defers redraws; the next allowed redraw
consumes the pending refresh once. Archived, cleaned and branchless workspaces
are excluded from general discovery; recorded PRs
remain available on archived workspaces. A second general request during a
scan queues one follow-up. Each general `gh pr list` has a 15-second deadline
and a 2 MiB output cap. Failed, timed-out, empty or incomplete results preserve
known PR metadata.

## IPC and compatibility

- `actions_save({ catalog }) -> void`: validates references, names and limits;
  persists to the board and publishes the `board` event.
- `action_start({ workspace, name, context }) -> Tab`: resolves the profile,
  creates a local session and starts the request. A spawn error stays in the tab
  for inspection.

New fields are additive with defaults in persistence. Boards written by earlier
versions may contain a profile `watch` and task `seen`/`prs` cursors from the
removed PR monitor; they are ignored on load and dropped on the next save.
Tasks started by those versions (non-zero `checked_at`) are marked done once on
load, so the same command can start again; a queued message stays visible in the
tab and is never sent automatically. Tasks
still write `paused`, `turns` and `checked_at` (always idle values) so earlier
versions can read the board. Ordinary sessions do not
change their launch configuration. The web mock implements the registry and tab
creation, but does not query GitHub and does not run models. Evidence:
[`actions.test.ts`](../../src/actions.test.ts),
[`git-refresh.test.ts`](../../src/git-refresh.test.ts),
[`actions.rs`](../../src-tauri/src/actions.rs),
[`github.rs`](../../src-tauri/src/github.rs),
[`actions.spec.ts`](../../e2e/actions.spec.ts).
