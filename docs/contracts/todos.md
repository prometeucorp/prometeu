---
type: Contract
title: "Standalone to-do list"
description: "The to-do list in Tasks, its file, the computer that keeps it, the IPC commands and the MCP tools of the desktop and of the host."
status: stable
---

# Standalone to-do list

Decision: [ADR 0108](../decisions/0108-standalone-todo-list.md).
Implementation: `src-tauri/crates/core/src/todos.rs` (model and store),
`src-tauri/src/todos.rs` (desktop), `src-tauri/crates/host/src/todos.rs`
(host), `src/todos.ts` and `src/todos-model.ts` (interface).

## Purpose and scope

Tasks has a third tab, **To-dos**, adjacent to Linear and GitHub. The tab shows
one list of standalone to-dos. Agents in Prometeu can add to-dos to the same
list and change their status. The tab operates without a Linear or GitHub
connection.

A to-do has a title and a status. It has no link to a project, a workspace, an
issue, an assignee or an external provider. This version has no rich editor,
no other fields, no deletion and no migration from a different board.

## Model

| Field | Type | Rule |
| --- | --- | --- |
| `id` | string | A UUID v4. It does not change, and the app does not use it again. |
| `title` | string | One line. The app removes the spaces at the start and at the end. Then the title must have 1 to 200 Unicode scalar values and no control characters. |
| `status` | string | `todo`, `doing` or `completed`. A new to-do has `todo`. |
| `created_at`, `updated_at` | number | Unix seconds. A status change updates `updated_at` only when the status is different. |

The interface counts the title with the same rule. It does not use the HTML
`maxLength` attribute, because that attribute counts UTF-16 units. Thus a title
of 101 emoji is correct in the interface and in the store.

## Size of the list

A list has a maximum of 1,000 to-dos. The serialized `items` array also has a
maximum of 512 KiB after a new to-do. If a new to-do goes above one of these
limits, the store refuses it with `todo_list_full`.

The byte limit is necessary because a title of 200 characters can have 800
bytes. A status change can add a maximum of 5 bytes to each to-do. Thus a full
list stays below 768 KiB, which is the response limit of the desktop MCP. The
host MCP has a larger limit. A compile-time check in the core keeps the three
values in agreement.

## File

The computer that keeps the list stores it in `todos.json` in its root:

- The desktop: `<root>/todos.json`, for example `~/.prometeu/todos.json`.
- A remote machine: `<remote root>/todos.json`, for example
  `~/.prometeu-remote/todos.json`.

The format is `{ version: 1, enabled, items: [Todo] }`. Only a host reads
`enabled`. The desktop ignores it in its own file. The file is private (0600).
The writer replaces it with a temporary file and an atomic rename.

`prometeu-core::todos::TodoStore` does each change as one read, change and
write under one lock. Only one process writes each file:

- The desktop app writes its file. The embedded MCP subprocess sends its calls
  through the private socket to the app. It does not write the file.
- The host daemon writes the file of the machine. The attachments of the
  desktops and the agent connections share one store in the daemon.

Thus requests at the same time do not lose a change. A change that has an error
writes nothing. The store refuses a file with a `version` that is larger than 1
(`todos_newer_version`), and it does not write that file again. It also
refuses a damaged file (`todos_unreadable`). The store ignores unknown fields
in a version 1 file, and its next write does not keep them.

## Location of the list

A desktop shows one list. The list is on this computer by default. The person
can select one remote machine instead. The selector in the toolbar of the tab
always shows the selection. `<root>/todos-location.json` stores it as
`{ machine: "<machine id>" }`. A missing file means this computer.

When the person selects a machine, the desktop does these steps:

1. It reads the list of the machine to make sure that the host answers.
2. If a different machine kept the list, it sends `todos_enable` with
   `enabled: false` to that machine.
3. It sends `todos_enable` with `enabled: true` to the new machine.
4. It saves the new location.

If step 3 has an error, the desktop enables the previous machine again. It does
not save the location. If step 4 has an error, the desktop does the steps again
from the new location to the saved location. Thus the hosts agree with the saved
location, and the next selection can find the host that serves the list.

A removed machine does not block a change of location. The desktop refuses to
move the list away from a machine that is not connected
(`todos_leave_offline`). The host of that machine cannot get the change. Thus it
must not continue to serve the list to its agents without a desktop that shows
the list.

The desktop never uses a different list as a replacement. While the selected
machine is offline, the tab shows the error and the list area stays empty. The
input and the status controls are disabled. MCP calls on the desktop return
`machine_offline`.

The local list and the list of a machine are different files. A change of
location does not copy or merge to-dos. The previous list stays in its file,
and the person can select its computer again to see it.

## Connection changes

When the connection to the selected machine changes, the tab reads the list
again. The `machines` event gives the phase of each machine. Thus the tab shows
these results:

- After a disconnection, the tab shows the error. It does not show the last
  items.
- After a new attachment, the desktop also emits the `todos` event. The tab
  shows the changes that the agents of the machine made while the desktop was
  away.

If the tab is not visible, it reads the list when the person opens it.

## Desktop IPC

The commands are in `src/ipc.ts` and in `src/mock.ts`.

| Command | Arguments | Result |
| --- | --- | --- |
| `todos` | none | `{ machine, items, error }`. `machine` is null for this computer. When the list is not available, `items` is null and `error` is a translatable message. |
| `todo_create` | `title` | the new to-do |
| `todo_set_status` | `id`, `status` | the changed to-do |
| `todos_locate` | `machine` (string or null) | the same object as `todos`, after the change |

The errors are translatable `todos.err.<code>` messages. The desktop emits the
`todos` event with a `null` payload after each change that it makes. It also
emits the event when the host of the selected machine sends its `todos` event.
The tab reads the list again when it is visible.

The tab sends one write at a time. It ignores a write that starts during a
different write, and it reports that write as ignored, not as complete. The
person can type in the title field while the tab sends a new to-do. When the
create is complete, the tab clears the field only if it contains the text that
the tab sent. Thus a person does not lose a new draft.

## Host methods

The attachment of the desktop and the agent socket accept these methods. The
methods add to protocol 1 and to agent protocol 1.

| Method | Params | Result | Callers |
| --- | --- | --- | --- |
| `todos_list` | none | `{ items }` | attachment, agents |
| `todos_create` | `title` | the new to-do | attachment, agents |
| `todos_set_status` | `id`, `status` | the changed to-do | attachment, agents |
| `todos_enable` | `enabled` (boolean) | `{ enabled }` | attachment only |

While `enabled` is false, the first three methods return `todos_not_here`. Thus
an agent on a machine cannot fill a list that no desktop shows. A new machine
has no file, and its list is disabled. After each change, the host sends
`{"event":"todos"}` to each attachment.

`todos_create`, `todos_set_status` and `todos_enable` delay a `shutdown` that
is not forced. This rule applies to the attachment and to the agent socket.
After a shutdown starts, these methods return `host_stopping`. Thus an update of
the host does not stop the host during a write of the list.

## MCP tools

The embedded MCP of the desktop and the `prometeu-host mcp` server give the
same three tools with the same descriptions:

| Tool | Inputs | Result |
| --- | --- | --- |
| `list_todos` | none | `{ items }` |
| `create_todo` | `title` (1 to 200 characters) | the new to-do |
| `set_todo_status` | `id`, `status` (`todo`, `doing` or `completed`) | the changed to-do |

`list_todos` is read-only. The host MCP sends it again after a lost
connection. A status change to the current status changes nothing. Thus a
client can send the same status change again. A `create_todo` that loses its
connection has an unknown outcome. The client must read the list before it
tries again.

The tools return errors as `isError` results with a stable code:

- `title_required`, `title_too_long` and `invalid_title`.
- `invalid_status` and `todo_not_found`.
- `todo_list_full`.
- `todos_not_here` and `todos_not_allowed`.
- `machine_offline`, `machine_unknown` and `host_update_required`.

On the desktop, the tools use the location that the person selected. If that
location is a machine, the desktop sends the host methods through the current
attachment. Thus the tab, the desktop MCP and the host MCP use one list. On a
machine, the host MCP always uses the list of its own host.

## Authorization

Each MCP keeps its current authentication. The list adds no anonymous access
and no new listener.

| Client | Access to the list | Cause |
| --- | --- | --- |
| Conversation in Prometeu (internal credential) | yes | The person selected the built-in `prometeu` MCP for that conversation. The credential stays bound to the live process of the conversation. |
| Delegated worker | no, by default | Workers start without MCP servers. If a person selects the built-in MCP for a worker, the worker is a conversation client. |
| External client of the desktop | only with `--todos` | Registration with only projects does not give access. `register --todos NAME [PROJECT_ID...]` gives it. |
| Agent with the host token | yes | The token gives all host operations as the user of the host. The list does not increase that authority. |

The external credential file stays version 1. It adds `client.todos`, which is
false when it is missing. Thus the files from earlier versions keep their
project scope and do not get the list. A client with `--todos` and no project
is permitted. Earlier app versions refuse such a client, because their check
requires a project. `--prometeu-mcp-client list` shows `todos` for each client.

The list of a host is available to all clients that have the agent token of
that host. The desktop does not give the list to clients that do not have one of
the credentials in the table.

## Compatibility

- Old desktops and hosts do not know the files and ignore them.
- An old host answers the host methods with `unknown method`. The desktop shows
  `host_update_required` and does not change the location.
- A desktop from before this change ignores `todos-location.json`, the `todos`
  event and `client.todos`.
- A downgrade keeps the files. The next upgrade uses them again.

## Limits

- The selection of a machine is for each desktop. If two desktops follow the
  same machine, one desktop can disable the list of that machine for the other
  desktop. The other desktop then shows `todos_not_here`. The **Try again**
  action selects the machine again.
- A host request that has no answer in its time limit returns
  `machine_offline`. The outcome of a write in that condition is unknown. The
  next read shows the result.
- If the save of the location and the return to the previous hosts both have
  an error, the hosts and the saved location can disagree. The next selection
  of a machine corrects that machine.
- The list has no deletion and no order that a person can change.

## Evidence

- `crates/core/src/todos.rs`: the title rules, the initial status, the stable
  ID, the reload, a write that has an error, newer and damaged files, older
  files, the item limit, the byte limit with titles of four-byte characters and
  writers at the same time.
- `crates/host/src/todos.rs`: `todos_not_here`, `todos_enable` only from the
  attachment, input errors and a reload after a new daemon.
- `crates/host/tests/agents.rs`,
  `the_desktop_and_host_mcp_agents_share_one_todo_list`: the real binary with an
  attachment and the stdio MCP from `agents setup`. An item from the host MCP
  shows in the list of the attachment. A status change from each side shows on
  the other side. The test also examines `todos` events, invalid input, 40 writes
  from four connections at the same time, the list after a new daemon and the
  stop of the service after `todos_enable` with false.
- `crates/host/tests/agents.rs`, `an_update_waits_for_a_desktop_todo_write`: a
  FIFO holds a write from one attachment. A shutdown that is not forced from a
  different attachment gets `host_busy` until the write is complete.
- `src-tauri/src/todos.rs`: the local list without a machine, the same list
  through a machine, no local replacement while offline, the location changes,
  the return of the hosts after a save error, the shared response limit and the
  access rules of clients.
- `src-tauri/src/mcp_access.rs`: the explicit `--todos` grant and credentials
  from earlier versions.
- `src-tauri/src/embedded_mcp.rs`: the tool schemas and the refusal of extra or
  invalid arguments.
- `src/todos-model.test.ts`: the order, the screen states, the location
  options, the request sequence, ignored writes, the drafts during a create,
  the Unicode count and the reload after a connection change.
