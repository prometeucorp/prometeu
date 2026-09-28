# Reusable actions and agents

Status: implemented; decision in [ADR 0009](../decisions/0009-reusable-actions.md).

## Registry and entries

`Board.actions` contains `profiles`, `commands`, `overrides`, `pr_action`,
`defaults_initialized` and `defaults_revision`. On first open, earlier catalogs
receive the editable **Code review** profile and the `/review` command. The
profile reports bugs and risks without modifying files or publishing a PR. It
runs read-only on a provider other than the builder's when one is available.
The initialization is recorded so later removals and customizations are
respected; existing names or identities are not overwritten. A catalog whose
bundled profile is still identical to an earlier seed receives the current seed
once, at startup and whenever a Cloud catalog is applied; `defaults_revision`
records it, so later edits stay. The source JSON is in
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

A profile has identity, name, prompt, `choice` (provider/model/effort),
`provider_rule: fixed | different_from_builder`, ordered `candidates` (each with
provider, model and effort), `access: default | read_only`, MCP, plugins, skill
names, `permission: ask | auto` and an optional `watch`.
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

An untracked task finishes when its turn finishes. Tracking is optional and does
not change the workspace's manual stage. Repeating the same command with a task
still open returns the existing tab. If there is additional context, the app
refuses and preserves the draft so it can be sent in the task's tab. Starting
another task requires that no conversation is working, waiting for an answer or
holding a pending message.

### Provider rule

`fixed` runs `choice`. `different_from_builder` has one to three candidates with
distinct providers, and `choice` mirrors the first so an older app runs it as a
fixed profile. Builders are the providers of the workspace's ordinary tabs, each
tab's own choice or the workspace default it inherits; without ordinary tabs,
the workspace default counts. At start the backend takes the first candidate
that is installed, signed in and not a builder, then the first usable
candidate, then the first candidate, whose spawn reports why it cannot run.
Usable means the CLI is on the adopted PATH and the active account's last known
identity is signed in; Antigravity needs its version check and an attached
account. The provider is picked before tool resolution. `Tab.task.profile`
freezes the picked candidate as a fixed choice and `Tab.task.same_family`
records that no usable candidate outside the builders existed.

### Access level

Under `default`, `permission` applies: Claude uses its normal approval mode
under `ask`; Codex uses `approvalPolicy: untrusted`; `auto` keeps the existing
bypass. These options do not constitute worktree isolation.

`read_only` requires every provider the profile can start with to advertise
`readOnlyProfile`, MCP and plugin selections that are `null` or empty, and no
skill names; the frozen task has no MCP servers, plugins or skills, whatever the
workspace selects. MCP tools, plugin hooks and servers, and skill tool grants
act outside the envelope. Adapters materialize it and refuse a launch that
carries any of them:

- Claude: `--restricted --permission-mode dontAsk --tools
  Read,Grep,Glob,Bash --add-dir <worktree>` with
  `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` and an inline empty strict
  MCP configuration. Settings files, CLI-enabled plugins and project skills are
  ignored; write tools are absent; `Skill` is absent because skill grants,
  bundled ones included, pre-approve commands; `Task` is absent because its
  worktree isolation writes a git worktree; commands the CLI does not classify
  as read-only are denied without a prompt; bypass is refused.
- Codex: `sandbox: "read-only"` and `approvalPolicy: "never"` on start and
  resume; `--disable` for `plugins`, `hooks`, `apps`, `computer_use` and
  `browser_use`; and `-c mcp_servers.<name>.enabled=false` for every server in
  the session's `config.toml`, because `-c mcp_servers={}` merges into that
  table. A server name `-c` cannot address refuses the start.
- Antigravity: refused before the spawn.

Read-only access limits what injected repository content can do to the review
text. It is not an operating-system sandbox for Claude's shell, and Git
configuration left in the repository still runs for any Git read.

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
known PR metadata. The monitor keeps its configured interval, 30-second query
deadline, pagination, cursors and pending delivery. Its richer PR, comment and
CI queries cannot reuse the general listing's field set or freshness guarantee.

## IPC and compatibility

- `actions_save({ catalog }) -> void`: validates references, names and limits;
  persists to the board and publishes the `board` event.
- `action_start({ workspace, name, context }) -> Tab`: resolves the profile and
  its provider, creates a local session and starts the request. A spawn error stays in the tab
  for inspection.
- `action_pause({ session, paused }) -> void`: pauses or resumes tracking.

New fields are additive with defaults in persistence. Profiles and tasks saved
before the provider rule load as `fixed`, `default` and `same_family: false`. Ordinary sessions do not
change their launch configuration. The web mock implements the registry and tab
creation, but does not query GitHub and does not run models. Evidence:
[`actions.test.ts`](../../src/actions.test.ts),
[`git-refresh.test.ts`](../../src/git-refresh.test.ts),
[`actions.rs`](../../src-tauri/src/actions.rs),
[`github.rs`](../../src-tauri/src/github.rs),
[`actions.spec.ts`](../../e2e/actions.spec.ts).
