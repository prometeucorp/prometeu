---
type: Contract
title: "Reusable actions and agents"
description: "Commands and profiles that a person can use again, the execution of tasks and the PR discovery."
status: stable
---

# Reusable actions and agents

The app obeys this contract. [ADR 0009](../decisions/0009-reusable-actions.md)
records the decision.

## Registry and entries

`Board.actions` contains `profiles`, `commands`, `overrides`, `pr_action` and
`defaults_initialized`. When the app opens an earlier catalog for the first
time, it adds the **Code review** profile and the `/review` command. A person
can edit them. The profile reports bugs and risks. It does not change files and
does not publish a PR. The app records this initialization. Thus, it does not
add again the items that a person removed or changed, and it does not replace
the current names or identities. The source JSON is in
[`action-defaults.json`](../../src/action-defaults.json). The registry is local.
All projects can use it, and the app does not send it to the relay.

- A command has `name`, `description`, `kind: prompt | agent`, `prompt` and an
  optional `profile`. Each name is unique and has 1–64 lowercase ASCII letters,
  digits or hyphens. The app keeps the names `compact` and `context` for its own use.
- `prompt` adds the expanded text to the current box and does not send it. The
  remaining text follows the expansion, and the person can edit it.
- `agent` starts a task in a different tab. The request contains the text of
  the command, the title, the repositories and bases of the workspace, and the
  context that the person wrote.
- `/name` and the **Actions** menu use the same registry. If the provider also
  has a `/name` command, the provider command keeps the name, and the app
  command uses `/prometeu:name`. The explicit namespace also operates when the
  names are different. The suggestions for the commands that the app keeps in its registry
  show the **Prometeu** tag. The commands and skills of the provider do not get
  that tag.
- `pr_action: null` keeps the PR request in the current conversation, and the
  agent continues to prefer the skill of the repository. A configured name
  points to an `agent` command, and the **Open PR / Update PR** button uses
  that command.

## Profile and execution

A profile has an identity, a name, a prompt, `choice` (provider, model and
effort), MCP servers, plugins, skill names and `permission: ask | auto`.
`overrides[project][profile]` replaces all of a profile for that project. It
does not change the global profile.

Each `Tab.task` stores a copy of the resolved profile, the name of the command
and the execution state. If MCP or plugins are `null`, the task uses the
selection of the workspace at the start. An empty list is an explicit
selection. If the workspace value is also `null`, the external configuration of
the provider stays. The catalog contains no secrets. Credentials stay in the
current hubs.

A change to the profile or to the selection of the workspace does not change
that copy. When the agent resumes, it uses the same profile. The app refuses a
change to the model from the footer of a task. To change the model, edit the
profile for the subsequent executions. For Codex plugins and MCP, each task
session gets its own configuration. Thus, the selection of a different
conversation does not change.

Skills are names that the app gives to the agent as instructions. They come
from the installation or from the selected plugins. That list is not an
allowlist, and it does not disable the other skills of the provider. If a skill
is not available, the instruction tells the agent to stop and report it. The app
does not promise to find that text result automatically.

The adapter applies the permissions:

- With `ask`, Claude uses its usual approval mode, and Codex uses
  `approvalPolicy: untrusted`.
- `auto` keeps the current bypass.

These options do not isolate the worktree.

A task is completed when its turn is completed. If a person starts the same
command again before the task is completed, the app returns the current tab. If the
person added context, the app refuses and keeps the draft. The person can then
send the draft in the tab of the task. A new task can start only when no
conversation operates, waits for an answer or holds a message that it did not
send.

## PR discovery

The app does not follow a PR automatically. It does not read the PR comments or
the CI of a task at intervals, and it does not start agent turns itself.

The general PR discovery starts when the app opens. For each clone, one scan
examines all the workspace branches that apply. The app schedules the scan at
these intervals:

- At intervals of three minutes when the app is in the foreground and the
  computer uses AC power.
- At intervals of five minutes when the app is in the foreground and the
  computer uses a battery or the power source is unknown.
- At intervals of 15 minutes when a person cannot see the app or the app does
  not have the focus.

When the app comes back to the foreground, it requests a scan immediately if
the last request occurred one minute ago or before. If not, the schedule continues.

When a person opens a workspace, the app also refreshes the repositories of
that workspace immediately. When a turn in the open workspace becomes stable,
the app refreshes them again. These refreshes occur a maximum of one time in
20 seconds for each workspace. Thus, a PR that the agent made shows quickly.
The app monitors the turn state also when a menu or a rename field stops the
redraws for a short time. The next redraw that the app lets occur does the
refresh that waits, one time.

The general discovery does not include archived workspaces, cleaned
workspaces or workspaces without a branch. Recorded PRs stay available on archived workspaces. If a
second general request comes during a scan, the app puts one more scan in the
queue. Each general `gh pr list` has a deadline of 15 seconds and an output
limit of 2 MiB. The app keeps the known PR metadata in these
conditions:

- The command has a failure.
- The deadline goes by.
- The result is empty or not completed.

## IPC and compatibility

- `actions_save({ catalog }) -> void`: validates the names, the limits and the
  items that the catalog refers to. It stores the catalog in the board and publishes the `board`
  event.
- `action_start({ workspace, name, context }) -> Tab`: resolves the profile,
  makes a local session and starts the request. If the process does not start,
  the error stays in the tab, and a person can examine it.

New fields add to the format and have default values in the stored data.
Boards from earlier versions can contain a profile `watch` and the task cursors
`seen` and `prs` from the removed PR monitor. The app ignores these fields when
it loads the board, and it removes them at the next save. When the app loads the
tasks from those versions (a `checked_at` value that is not 0), it sets them
to done one time. Thus, the same command can start again. A message in the queue
stays in the tab where a person can see it. The app does not send it
automatically. Tasks continue to write `paused`, `turns` and `checked_at` with
the values of an idle task. Thus, earlier versions can read the board. Usual sessions do not
change their launch configuration. The web mock has the registry and the tab
creation. It does not query GitHub and does not run models. Evidence:
[`actions.test.ts`](../../src/actions.test.ts),
[`git-refresh.test.ts`](../../src/git-refresh.test.ts),
[`actions.rs`](../../src-tauri/src/actions.rs),
[`github.rs`](../../src-tauri/src/github.rs),
[`actions.spec.ts`](../../e2e/actions.spec.ts).
