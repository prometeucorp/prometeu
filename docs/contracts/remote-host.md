---
type: Contract
title: "Remote host protocol"
description: "The installation, the SSH attachment, the NDJSON methods and the events of prometeu-host."
status: stable
---

# Remote host protocol

Decisions: [ADR 0101](../decisions/0101-remote-machines.md). For the registry
and the agent clients, see [ADR
0103](../decisions/0103-agents-on-remote-machines.md). For many computers that
attach at the same time, see [ADR
0104](../decisions/0104-several-computers-on-one-machine.md). For Codex, see
[ADR 0105](../decisions/0105-codex-on-remote-machines.md). Implementation:
`src-tauri/crates/host/` (host) and `src-tauri/src/remote/` (desktop).

## Installation

The desktop has one Linux x86_64 build. The desktop looks for the build in
these locations, in this sequence:

1. `PROMETEU_REMOTE_HOST`.
2. `Contents/Resources/remote-host/prometeu-host-x86_64-linux` in the app
   bundle.
3. Only for a debug build:
   `src-tauri/target/x86_64-unknown-linux-musl/release/prometeu-host`.

The installation folder of the build is the first 16 hexadecimal digits of its
SHA-256.

```text
~/.prometeu-remote[-dev]/
  host/<hash>/prometeu-host   immutable build, 0700
  current -> host/<hash>       last installed build
  previous -> host/<hash>      build it replaced, kept for rollback
  host-v1.sock                 0600, one per protocol version
  host-v1.lock                 flock held by the serving host
  host.log                     host stderr
  sessions/<tab>/settings.json Claude hook settings, 0600
  sessions/<tab>/rpc           a Codex engine's listening alias (ADR 0105)
  worktrees/<repo>/<branch>/   worktrees created by the desktop or the host
  registry.json                the machine's projects and workspaces, 0600
  todos.json                   the machine's to-do list, 0600 (todos.md)
  agents.json                  `{ socket, client, opened }` written by setup, 0600
  agents/bind-error            why the host cannot serve the agent socket, while it cannot
  agents/token                 agent token, 0600; agents are enabled while it exists
~/.prometeu-remote[-dev]-agents/   default agent directory, 0700
  agents-v1.sock               agent socket, 0666
  prometeu-mcp                 client executable the host keeps current, 0755
```

The installation does these steps:

1. It makes sure that `uname -sm` is `Linux x86_64`.
2. It streams the build through SSH stdin to a temporary file.
3. It examines the SHA-256 of the file before the rename.
4. It updates `current` and `previous`.
5. It removes all other builds that no host process runs and that no
   `sessions/*/settings.json` file names.
6. It tries `loginctl enable-linger`.

Step 5 keeps the builds that these settings files name, because a
conversation that operates calls the hook helper of the build that started it.
A host deletes `sessions/` when it starts, because it owns no terminal at that
time.

## Attachment

The desktop uses the system `ssh` with `BatchMode=yes`, `ControlMaster=no`,
`ControlPath=none`, `ServerAliveInterval=5` and `ServerAliveCountMax=3` to run
this command:

```text
exec "$HOME/<root>/host/<hash>/prometeu-host" attach --root "$HOME/<root>" \
  --device '<device id>' --label '<computer name>' --release '<app version>'
```

If the socket does not answer, `attach` starts `serve --release <app version>`
with `setsid`. Then it sends
`{"attach":{"protocol":1,"device":"…","label":"…"}}` and relays stdio. Desktops
from before ADR 0104 do not send the two fields.

### Several computers

The device must agree with `[A-Za-z0-9_-]{1,128}`. The host uses no device for
a different value. The host removes control characters from the label and cuts
it to 64 characters. The host serves a maximum of 8 attachments. A new
attachment replaces the attachments with the same device with `replaced`. This
includes the empty device. Attachments from other devices stay.

After the limit, the host answers `{"event":"error","error":"busy"}` and closes
the connection. The host closes an attachment that has more than 16 MiB of
frames that wait for a write. The file stream waits at half of that value. It
does not close the attachment.

Each attachment that called `terminal_attach` for a terminal gets the `output`
and `exit` events of that terminal. Each attachment gets `registry`. The host
keeps the hooks for each terminal, but not the permission requests. It keeps the
last 512 hooks, with the set of devices that confirmed each hook.

Desktops send `hook_reply` for each hook. For the kept hooks, this reply is the
confirmation. Each reply confirms only its own hook, in all sequences of the
replies. `terminal_attach` sends to a device each kept hook that the device did
not confirm. When a terminal serves a device for the first time, the hooks that
a different device confirmed count as confirmed for the new device. This rule
also applies when the terminal has no kept hooks. The terminal owes all
subsequent hooks to the new device.

The host can put a hook in the queue of an attachment that was not connected. In
this condition, the host sends the hook again when the desktop comes back. A
desktop can get a hook two times if it processed the hook but lost the
connection before the confirmation arrived.

The host sends a permission request to each attachment that follows the
terminal. The host does not keep permission requests. The hook prints the first
`hook_reply` with output. The other attachments that got the request get
`hook_resolved`. An empty reply, or an attachment that ends, abstains. When all
the attachments abstain, the hook prints nothing. If the wait ends without a
decision, the attachments also get `hook_resolved`.

The PTY uses the size of the attachment that opened the terminal or wrote to it
last. The host records a `terminal_resize` from a different attachment. It
applies the resize when that attachment writes. If no such attachment is
connected, all resizes apply.

An attachment can send `terminal_forget` while a different attachment follows
the stopped terminal. Then only this attachment stops to follow it. The host
removes the terminal when the last follower forgets it or ends.

Exit status 127 or `No such file` before the ready frame tells that the build
is not installed. The `executable` of the ready frame can belong to a different
build. In this condition, the desktop stops that host only when it runs no
terminal. If it runs a terminal, the desktop continues to use it and shows an
update that waits.

While the update waits and the attachment stays, the desktop examines the host
again in these conditions:

- When an `exit` event arrives.
- At intervals of 60 seconds or less.

The desktop does not do this check while one of its requests waits for a reply.
The `shutdown` check of the host continues to make the decision. `exec`,
`file_write` and `terminal_open` delay a `shutdown` that is not forced, as the
agent requests that change the machine do. After a shutdown starts, these
requests return the error `host_stopping`.

## Framing

The protocol uses NDJSON, with a maximum of 8 MiB for each line. A request is
`{ id, method, params }`. A reply is `{ id, result }` or `{ id, error }` with a
diagnostic string. Frames with an `event` field are not replies to requests.

The host processes requests in the sequence in which they arrive. `exec`,
`stats` and `shutdown` are exceptions: they operate in parallel with the stream.
Thus, a client can send an attach immediately after its open, or send input
without a wait for replies. There are no retries. If a reply is lost, the
outcome is unknown.

## Events

| Event | Fields | Meaning |
| --- | --- | --- |
| `ready` | `protocol`, `version`, `executable`, `pid`, `home`, `user`, `root`, `os`, `arch`, `linger`, `scopes`, `clients`, `release` | The first frame of each attachment. `clients` is true when many computers can attach. `release` is the app version that started the host, or null. |
| `replaced` | none | The same computer attached again, or a different computer disconnected this one. This attachment is detached. |
| `hook_resolved` | `hook` | A different attachment made a decision for that permission request, or no attachment made a decision in time. Remove the request. |
| `output` | `id`, `seq`, `data` (base64) | Terminal output after the replay of the attachment. |
| `exit` | `id`, `code`, `seq` | The terminal stopped, after the host sent all its output. The host waits a maximum of 2 s, because a background process can keep the PTY open. The host also sends this event at attachment for a stopped terminal. |
| `hook` | `hook`, `session`, `permission`, `env`, `payload` (base64), `at` | A native hook call for the terminal `session`. |
| `file` | `watch`, `path`, `offset`, `reset`, `data` (base64) | Bytes of a watched file from `offset`. `reset` starts the file again from zero. |
| `registry` | none | An agent request that can add or close registry entries is complete. The desktop synchronizes again. |
| `todos` | none | The to-do list of the machine changed. Refer to the [to-do contract](todos.md#host-methods). |

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `ping` | none | `{}` |
| `exec` | `argv`, `cwd?`, `env?`, `stdin?` (base64), `timeout_ms?` (default 30 s, at most 600 s) | `{ code, timeout, stdout, stderr }` (base64, a maximum of 4 MiB each) |
| `file_write` | `path` (absolute), `data` (base64), `mode?` (default 0600) | `{}`. The host makes the parent folders with 0700 and uses an atomic rename. |
| `file_read` | `path`, `limit?` | `{ data }` |
| `terminal_open` | `id`, `workspace?`, `argv`, `cwd`, `env?`, `unset?`, `cols`, `rows`, `limits?` | `{ pid, unit }`. Refuses with `terminal_exists` an `id` that operates, or an `id` whose process a different open starts. Thus, opens at the same time start one process. |
| `terminal_attach` | `id`, `seq` | `{ data, reset, seq, running, code, pid }`, then live `output`/`exit` frames and the `hook` frames in the queue. |
| `terminal_write` | `id`, `data` (base64) | `{}` |
| `terminal_resize` | `id`, `cols`, `rows` (1–1000) | `{}` |
| `terminal_signal` | `id`, `signal`: `interrupt`, `terminate`, `kill` or `hangup` | `{}` |
| `terminal_stop` | `id`, `instance` | `{}`. Kills only the specified process instance. Refuses a replaced or stopped process with `terminal_changed`. |
| `terminal_forget` | `id` | `{}`. Removes a stopped terminal and `sessions/<id>`. Refuses a terminal that operates. |
| `terminal_list` | none | `[{ id, instance, workspace, pid, unit, started, running, code, seq }]`. Older hosts do not send `instance`. |
| `watch` | `id`, `dir` (absolute), `depth?` (default 3, at most 6), `offsets?` | `{}`. Streams `file` events each 250 ms while this attachment is connected. |
| `watch_flush` | `id` | `{}`, after all the bytes that the watch did not send are on the stream. |
| `hook_reply` | `hook`, `stdout` (base64) | `{}`. An empty `stdout` abstains. |
| `clients_list` | none | `[{ device, label, connected, current }]`. `connected` is in Unix seconds. `current` marks the attachment that asks. |
| `client_detach` | `device` | `{}`. Sends `replaced` to the attachment of that device. Returns `unknown_client` for no attachment or for the attachment that asks. |
| `registry_list` | none | The registry, with the `state`, `state_at` and `notification` of each conversation. |
| `registry_put` | `project?`, `workspace?` (registry records) | `{}`. Adds a project by path. Adds a workspace by ID, or merges into the workspace that is there: its title and new conversations. It keeps all other fields and conversations. |
| `registry_close` | `id` | `{}`. Marks a workspace as closed after the desktop cleaned it. |
| `todos_list`, `todos_create`, `todos_set_status`, `todos_enable` | refer to the [to-do contract](todos.md#host-methods) | The to-do list of the machine. Only the attachment can call `todos_enable`. |
| `stats` | `workspaces` | For each workspace slice, `{ memory, memory_max, cpu_usec, cpu_max }`, and also `memory_total`, `memory_available`, `load`, `cpus`, `terminals`. |
| `shutdown` | `force?` | `{}`, then the host stops. Without `force`, it refuses with `host_busy` while a terminal operates or while an `exec`, a `file_write`, a `terminal_open` or an agent request that changes the machine operates. After it starts, those requests return the error `host_stopping`. |

Terminal IDs and watch IDs must agree with `[A-Za-z0-9_-]{1,128}`. For an
unknown terminal, the host answers `unknown_terminal`. The desktop processes
this answer as a process that stopped with the host.

### Terminal sequence

Each output chunk gets the next sequence number. `terminal_attach` with the last
sequence that the client applied replies with the bytes after that sequence. The
host keeps 2 MiB of output. In two conditions, the reply contains the full
buffer with `reset: true`. In the first condition, the buffer does not go back
to that sequence. In the second condition, the sequence is after the sequence of
the host. The desktop then clears the screen first. The replay, the live output
and the hooks in the queue are in sequence under the buffer lock of the
terminal. Thus, they do not overlap and no bytes are missing.

### Limits

If linger is enabled and `systemd-run` is available (`ready.scopes`), a terminal
opened with `workspace` runs as
`systemd-run --user --scope --slice=prometeu-w<uuid>.slice --unit=prometeu-t<id>-<n>`.
The host applies `limits.memory_max`, `memory_high`, `cpu_quota` and `tasks_max`
(systemd syntax) to the slice with `systemctl --user set-property --runtime`.
Without scopes, the host ignores the limits.

## Hooks

The Claude settings name `'<host>' hook --root '<root>'` as the command of each
hook. The hook process reads a maximum of 1 MiB of stdin. It sends each
`PROMETEU_TERMINAL_HOOK_*` variable with the payload. Only `PermissionRequest`
waits for a reply, for a maximum of 300 s. The host acknowledges all other
hooks immediately. Then it sends them, or puts them in a queue when no desktop
is attached to the terminal.

The desktop changes the transcript paths to its mirror. It replaces the bridge
socket, the token and the nonce with its own values. It returns the decision of
the local bridge, if there is one. If the hook command has a failure, it prints
nothing and stops with status 0.

## Codex engines

A remote Codex conversation uses three terminals. The desktop opens them with
the usual methods:

- The engine `<tab>-codex`: `codex app-server --listen
  unix://<root>/sessions/<tab>/rpc`, with the workspace and its limits.
- The TUI `<tab>`: `codex --remote unix://… resume <thread>`.
- One relay for each desktop incarnation, `<tab>-rpc-<12 alphanumerics>`. The
  relay runs this command:

```text
<root>/host/<hash>/prometeu-host codex-relay --socket <root>/sessions/<tab>/rpc --pid <engine pid>
```

The relay changes its PTY to raw mode. It waits a maximum of 20 seconds for the
socket. Then it examines the socket as the local engine client of the desktop
does:

- The alias names the SHA-256 endpoint of Codex.
- The folder of the endpoint is 0700.
- The socket has no group bits and no other bits.
- The user owns the folder and the socket.
- The peer of the socket is the engine process or a descendant of it.

If a check finds a problem, the relay stops with status 1 and writes nothing to
stdout. If not, it prints `{"prometeu":"codex-relay","ready":1}`. Then it relays
one JSON object on each line in the two directions. The maximum is 64 MiB from
the engine and 1 MiB to the engine. The relay ignores an input line that is not
a JSON object. The relay stops when the WebSocket closes or its input ends.

The desktop reads relay lines only after the announcement. It stops the link in
these conditions:

- A line is malformed.
- After the first attachment, the host replies to `terminal_attach` with
  `reset`.

The desktop writes to the relay with `terminal_write` requests. If a reply is
missing, the desktop thinks that the request was not sent. The desktop does not
kill the relay of a different incarnation. Before it opens its TUI, it stops
only an engine that it opened. When the TUI stops, the desktop terminates the
engine. At each attachment, the desktop terminates an engine if its TUI does not
operate and the engine started 60 seconds or more before.

## Transcript mirror

After a terminal stops, the desktop calls `watch_flush` on the watch of its
conversation. Thus, the conversation reads its last turn before it stops. Then
the desktop calls `terminal_forget`.

When the desktop attaches again, it can find a stopped terminal that belongs to
a conversation on the board. The desktop adopts that terminal and does not start
Claude. It collects the output, the hooks in the queue and the transcript in the
same way. Then it forgets the terminal. The desktop forgets immediately the
stopped terminals that no conversation will read again.

For a conversation in `<cwd>`, the desktop watches
`<home>/.claude/projects/<slug>`, with depth 3, which includes subagents. It
writes the bytes to `<root>/remote/<machine>/mirror/<absolute remote path>`. The
folders are 0700 and the files are 0600. After each new attachment, the desktop
sends the sizes of the mirror files as offsets. The slug replaces each
character that is not in `[A-Za-z0-9]` with `-`.

Before the desktop adopts a terminal, it waits for `watch_flush` to complete. A
Claude `SessionStart` hook can bind a transcript path before the first prompt
makes that file. The adoption keeps the current process, identity and journal,
and it records the first prompt as new input, when all these conditions are
true:

- The bound file is not there.
- The journal contains no native receipts and no prepared input.
- The canonical log contains only status metadata.

This exception does not start a replacement process. It does not skip the
verification of the current history. The next authenticated hook binds the
observation to the transcript as usual.
`remote_reattachment_before_first_prompt_preserves_identity_and_requires_empty_history`
in `chat/terminal/ingestion/tests.rs` examines this behavior. The behavior does
not change a persisted format or the host protocol.

## Desktop process inspection

Settings shows **Running processes** on each connected machine. The added
desktop IPC commands `machine_terminals { id }` and
`machine_terminal_stop { id, terminal, instance }` use `terminal_list` and
`terminal_stop`. They operate only on a connected machine. They operate only on
the terminals that its host owns. They do not operate on arbitrary PIDs.

Each terminal launch gets a new UUID in `instance`. The host keeps it across
attachments. The confirmation sends that accurate instance through IPC to the
host. The host examines it and signals the same process handle. It does not look
for the terminal ID again. If a conversation ID or a PID is used again, an
earlier confirmation cannot apply to a different process. The process control
also serializes the signals with the reaping. This prevents a signal to an OS
PID that the system used again.

The new host method and list field only add to protocol 1. There is no change to
a persisted format. Old desktops continue to use `terminal_signal`. A person can
examine the processes of hosts without `instance`. But their stop buttons are
disabled, with an instruction to stop the work from the conversation or the
machine terminal before the upgrade. A stop request after a new connection to an
older host gets the error for an unknown method. There is no fallback to a
signal without this protection. An optional identity in `terminal_signal` is not
safe, because older hosts silently ignore unknown parameters.

The list uses the `running` flag of the host. This includes idle conversations,
sign-in terminals and terminals that are not on the board of the desktop. The
desktop finds board names only on that machine. The list shows the conversation
or terminal ID, the workspace, the PID and the start time. While the list is
open, it refreshes each three seconds. If a query has an error, it removes the
controls that are not current.

To stop a process, the person must confirm. The desktop kills the process group.
It does not remove the workspace or the saved transcript. The usual exit
procedure continues to send and mirror the output. The notice about the update
that waits tells that idle processes also prevent the replacement. Other host
requests can delay the replacement after the list becomes empty.

`src/machine-processes.test.ts` examines these items: host liveness, labels for
each machine, confirmation races, legacy hosts, stops of specified processes and
the lifetime of the polling.
`stopping_a_listed_process_never_kills_a_replacement_under_the_same_id` proves
that an expired or missing instance does not stop a replacement. It also proves
that the current instance can stop the replacement. The current
`terminals_survive_detachment_and_replay_the_gap` host test examines the
compatibility of the list and the kill operation. This includes the listed state
after the exit.

## Desktop project selection

Settings > Projects shows the registered folders with a filter by computer. The
launcher uses the same `Project.host` filter. Origin aliases only make
groups in the sidebar. The remote registration and the clone operation use the
current `remote_list_dir` and `remote_add_project` desktop commands in different
dialogs. They do not change the host protocol or the persisted project IDs. The
current backend resolution of the origin stays compatible with callers that send
a local project ID.

`src/machines.test.ts` examines local clones, clones on the selected host,
clones on a different host and clones with aliases.
`src/settings-navigation.test.ts` keeps the saved `projects` destination.

## Connected computers

The added desktop IPC commands `machine_clients { id }` and
`machine_client_detach { id, device }` use `clients_list` and `client_detach` on
a connected machine. Settings shows them in the menu of the machine only when
`ready.clients` is true. The list shows this computer first. The desktop asks
the person before it disconnects a different computer. A disconnected desktop
shows **In use by another computer** and stops its connection attempts, as after
a replacement. An older host answers `clients_list` as an unknown method. The
desktop then tells that a host update is necessary.
`src/machine-clients.test.ts` examines the sequence and the confirmation.

The desktop also sends `--release`. It keeps an idle host from a different
build when `ready.release` is newer than its own version. Thus, computers on
different releases do not stop the host of each other. The desktop replaces an
idle host without `release`, as before.

## Registry

`registry.json` is `{ version: 2, projects, workspaces, supervision }`. Only the
host writes it, with an atomic rename through a temporary file with a unique
name. The host can read version 1 records. It changes them to version 2 at the
next correct write. The host refuses a registry with a newer `version`. It does
not write that registry again without the fields that it does not know.
Records:

- project: `{ path, name, origin?, added_at, added_by }`.
- workspace: `{ id, project, title, branch, base, worktree, created_at,
  created_by, closed, keep_branch, conversations, supervision? }`.
- conversation: `{ id, title, created_at, permission }`. The ID is the tab ID,
  the Claude session ID and the terminal ID. `permission` is `auto` or `ask`.
- creator: `{ kind: "desktop" | "agent", name? }`.

The host ignores unknown fields. Times are in Unix seconds.

Version 2 protects the supervisor links and the durable outbox from older
hosts. Without it, these hosts delete unknown fields. For a downgrade, keep a
host that can read this registry version. Do not change the file label to
version 1. Do not replace the file to start an older host.

The desktop synchronizes its board with `registry_list` at each attachment and
at each `registry` event (`src-tauri/src/remote/registry.rs`). It adopts
missing projects and open workspaces as remote board entries. Their
conversations become Claude tabs in terminal mode with the same IDs. The
workspace names an agent creator in `creator`. The desktop adds the
conversations that an agent opened in a workspace that the desktop shows. It
marks the workspaces that an agent closed as cleaned and archived.

The desktop uses `registry_put` to record its own remote workspaces and
conversations when their tab is on the board. It also records the projects that
a person added on the machine. Each synchronization also records the projects,
open workspaces and conversations that are not in the registry. It sends
`registry_close` again for a workspace that the desktop cleaned and that the
registry continues to show as open.

The board can use the ID of an entry for a local workspace, a workspace on a
different machine or a different tab. The desktop does not adopt or change
such an entry. A
conversation that an agent opened before it closed its workspace becomes a tab
of the closed workspace. The synchronization and the subsequent terminal
attachment operate one at a time for each machine. `registry` events that
arrive while a synchronization waits in the queue join it. The desktop calls
`registry_close` after it cleans a workspace.

These items stay in the registry:

- A workspace that a person removed from the board without cleanup.
- A conversation whose tab a person closed.
- A project that a person removed from the board, until the person adds it
  again.

The desktop lists these items in `<root>/remote/<machine>/dismissed.json`. This
file prevents a new adoption of the items. An adopted conversation without a
terminal on the host gets the transcript link that its first hook writes in the
usual condition.

## Observed state

The host reads the event name of each hook that it serves. It keeps `state` and
`state_at` for each conversation:

- `idle` after `SessionStart`, `Stop` or `StopFailure`.
- `working` after `UserPromptSubmit` or a tool hook.
- `permission` after `PermissionRequest`.

`Notification` records its `message` as `notification` until the next hook that
sets the working state. A conversation whose terminal does not operate shows
`ended`. A conversation that operates and has no hook shows `starting`.

## Agent socket

Agents connect to the path in `agents.json`. Setup records `agents-v1.sock` in
the default agent folder `<root>-agents` (`~/.prometeu-remote-agents`), if
`--socket` does not name a different path. If there is no record, the
installation is from before that default. Then the path stays
`<root>/agents/agents-v1.sock`. The next setup moves it and shows `moved_from`.

The host serves the socket only while `agents/token` is there. It examines the
token and the path each 3 seconds and binds again. Thus,
`prometeu-host agents setup [--socket PATH] [--rotate]
[--client-uid ID] [--json]` and `agents disable` apply without a new start.
When a person disables agents, the listener stops and the host removes its
socket.

`agents setup` makes missing socket folders with mode 0700. It does not change
the permissions of current folders. It does not open the home folder or other
parent folders to more users. Setup and the bind operation of the host refuse
these folders:

- A folder that the user of the host does not own.
- A folder that its group or other users can write to.
- A parent folder that an owner other than that user or root owns.
- A parent folder that its group or other users can write to, without the
  sticky bit.

Setup also refuses a folder with a `prometeu-mcp` that it did not record. It
refuses a socket path that an item other than a socket uses.

Setup records the socket and `<directory>/prometeu-mcp` in `agents.json`. It
removes a client that it recorded before in a different location. It copies
itself to the new location (0755). Then it makes or rotates the token. Each
time that the host binds the socket, it updates the recorded copy with an
atomic operation, but only when the copy is different. `agents disable` deletes
the token and the recorded copy.

For compatibility, setup keeps the `opened` list that older versions wrote. It
does not change those folders. Disable continues to remove `o+x` from those
legacy folders. It skips only the folders that it confirms are not there. It
keeps the records that it cannot examine or close, and it shows an error with
their names. The removed `--open-home` flag stops with an error before setup
changes anything.

`agents status --json` prints `{ enabled, socket, serving, error }`. These
fields tell if a process answers at the socket, and give the last bind error.
The host stores the bind error in `agents/bind-error`. It removes the error
when the host serves the socket or when a person disables agents.

`agents setup` prints a local stdio MCP entry:
`{ command: "<directory>/prometeu-mcp", args: ["mcp", "--socket", "<socket>"], env }`.
When `--root` is relative, setup resolves the two paths against its working
folder. Thus, the copied configuration operates from a different folder. The
resolution does not use the socket of the daemon, and it does not change stored
paths.

Before setup writes the configuration or the tokens, it validates the address
against the Unix socket path limit of the platform. Setup refuses an address
that is not valid. The error tells the person to select a different path with
`--socket`.

`env` contains `PROMETEU_AGENT_TOKEN`, `PROMETEU_AGENT_NAME: "agent"` and
`PROMETEU_AGENT_HOST_UID`. The expected peer ID is the actual UID of the setup
user, or the explicit `--client-uid` value. Setup does not make assumptions
about user namespaces. With `--json`, it prints
`{ token, socket, client, moved_from, server }`.

The desktop commands `machine_agents_enable { id, rotate }` and
`machine_agents_disable` run these commands through `exec`. They use the
installed build that the desktop attaches with, also when an older daemon
operates. Before the enable command publishes the status, it waits a maximum of
8 seconds for `serving` or `error`. It returns `{ config, preview, moved_from }`
to Settings. The first value is the formatted
`{"mcpServers": {"prometeu": server}}` with the token, for a copy. The second
value is a masked version for the display. `MachineView.agents` contains the
status that the desktop reads at each attachment. The desktop IPC and the mock
change together. The setup output does not contain mounts, namespace hints or a
permission-consent outcome.

Each MCP client that can run the command and connect to the socket can use this
configuration. The default configuration is for the same machine and Unix user
as the daemon. Containers, volumes, remote command execution, network access and
permission changes are the responsibility of the integrator. There is no HTTP
listener. An integration that maps paths or user identities must adapt the
command, the socket path and the expected peer UID. It must keep the token
authentication and the peer authentication.

The current client configurations, the recorded socket paths and the folder
permissions stay usable after upgrades. A new configuration contains local
paths, also for an installation that a person customized before.

The host replaces a current socket entry only when a non-blocking connection is
refused. The host keeps a socket that answers or that has a full queue. In the
other conditions, the host writes the refusal to the log one time and tries
again at each check. When the path changes, the previous listener stops and the
host removes its socket.

The first frame is `{"hello":{"protocol":1,"token":"…","client":"<label>"}}`.
The host reads the token from `agents/token`. It compares the token in constant
time on the hello frame and again before each request. If the token is not the
same, the host answers `{"event":"error","error":"unauthorized"}` and closes the
connection. For an unknown protocol, it answers `"protocol"` and closes the
connection.

A maximum of 16 connections can wait for their hello frame, each for a maximum
of 10 seconds. The host closes more connections without an answer. The host
serves a maximum of 64 authenticated connections. The next connection gets
`"busy"`. In the other conditions, the host answers
`{"event":"ready","protocol":1,"version"}`. Then it serves `{ id,
method, params }` requests in order with `{ id, result }` or `{ id, error }`.
The host records `client` (a maximum of 64 characters) as the `name` of the
creator.

Requests that change the machine delay a `shutdown` that is not forced. These
requests are `project_add`, `project_clone`, `workspace_create`,
`conversation_open`, `conversation_send`, `conversation_interrupt`,
`workspace_close`, `todos_create` and `todos_set_status`. After a shutdown starts, they return the error
`host_stopping`.

The token lets a client run commands as the user of the host. Thus, only that
user must have write permission for the folder of the socket. A person who can
replace the socket can get the token from the next client.

| Method | Params | Result |
| --- | --- | --- |
| `projects_list` | none | The projects. |
| `project_add` | `path` (absolute, in a Git repository) | The project, registered at the top level of the repository. |
| `project_clone` | `url`, `path?` (default `~/code/<name>`) | The cloned project. |
| `workspace_create` | `project` (path or name), `prompt`, `title?`, `branch?`, `base?` | The workspace with its first conversation. |
| `workspace_list` | `all?` | The open workspaces, or all workspaces. |
| `workspace_get` | `id` | The workspace. |
| `conversation_open` | `workspace`, `prompt` | The conversation and its `workspace`. |
| `conversation_read` | `id`, `cursor?`, `limit?` (1–200, default 50) | `{ messages, cursor, more, status }` |
| `conversation_send` | `id`, `text` | `{ resumed }` |
| `conversation_interrupt` | `id` | `{}`. Sends Escape. |
| `workspace_close` | `id`, `force?` | `{}`, or `{ branch_kept, detail }` when the worktree is removed but the host cannot delete the branch. |
| `todos_list`, `todos_create`, `todos_set_status` | refer to the [to-do contract](todos.md#host-methods) | The to-do list of the machine, while a desktop selected this machine for the list. |

Workspaces and conversations contain the observed state fields above.

`workspace_create` fetches `origin`, if it can. If no branch is given, it
selects a new `prometeu/<noun>-<adjective>-<NNNN>` branch. The base is
`origin/HEAD`, or `HEAD` if the remote has no default head. The worktree path is
`<root>/worktrees/<project name>/<flattened branch>`. The host resolves the
symbolic links in the parent, because Claude names transcript folders from its
physical working folder.

Before Git makes a file, the host marks that folder as trusted in
`~/.claude.json` (`projects[<path>].hasTrustDialogAccepted`). It does this under
a lock that its own edits also use. Claude asks about folder trust also without
permission prompts, and a trusted parent folder does not count. Then the host
adds the worktree and records the workspace. If the record operation has an
error, the host removes the worktree and the new branch. Then it starts:

```text
claude --session-id|--resume <id> --dangerously-skip-permissions \
  --settings <root>/sessions/<id>/settings.json <prompt>
```

If the launch has an error, the error names the recorded workspace and
conversation. `conversation_send` tries the launch again, and `workspace_close`
removes the workspace. If the launch of `conversation_open` has an error, the
host removes its conversation from the registry.

The terminal is 120×40, with the workspace slice and no limits. Its hook
environment names the session and `claude`. It has a placeholder token and
nonce that a desktop relay replaces. The host removes the variables that a
parent Claude exports to its child processes. The host uses `--resume` when the
transcript is there. For tests, `PROMETEU_HOST_CLAUDE` replaces the `claude`
program.

`conversation_read` parses `~/.claude/projects/<slug>/<id>.jsonl`. A message is
`{ role, kind: text | tool_use | tool_result, text, tool?, error?, at }`. The
host cuts tool text to 2,000 characters and message text to 20,000 characters.
It skips meta lines and subagent lines. Without a cursor, it returns the last
`limit` messages from the last 8 MiB of the file. With a cursor, it returns the
messages after that byte offset, and it reads a maximum of 8 MiB. The `cursor`
of the reply always stops at the end of a line.

The host skips a record that is longer than 8 MiB. The cursor moves after it,
and a `{ role: "system", kind: "skipped" }` message gives its size. A reply
contains a maximum of 4 MiB of serialized messages. A page stops before the
record that makes it larger than this limit. If one record alone is larger than
this limit, the reply keeps the messages that fit. A `skipped` message then
counts the remaining messages. A tail read removes its oldest messages to fit.

A cursor that does not follow a newline points into a record. A tail read
returns such a cursor when its window contains no line end. A read from this
cursor waits for the end of that record. Then it continues after the record
with a `skipped` message.

`conversation_send` writes the text as a bracketed paste. After 150 ms, it
writes Enter. It holds the terminal input during the two writes, so that
messages at the same time do not merge. If the conversation stopped, the host
starts it again with the text as its prompt. This does not apply to a
conversation whose `permission` is not `auto`. The host starts conversations
only without permission prompts, so the desktop must resume such a
conversation. `conversation_interrupt` and all other conversation methods
accept only IDs in the registry. They do not accept other terminals of the
host.

Without `force`, `workspace_close` refuses a worktree with `git status
--porcelain` output (`uncommitted_changes`). It also refuses with
`unpushed_commits` in this condition: the `HEAD` of the worktree has commits
that are not on the base or on a remote. The host resolves the base in the main
checkout. The same rule applies to the branch when the host will delete it. A
kept branch counts as a branch that holds its commits.

The host does not examine a branch if Git confirms that the recorded name is not
there (the branch was renamed or deleted). It examines only `HEAD`. If the
lookup has an error, the host refuses the close. Without `force`, the host
deletes only a branch that the checks examined. The host does the check before
it stops the terminals of the conversations, and again after they stop. If the
second check refuses, the conversations stay stopped. `conversation_send`
resumes the conversations whose `permission` is `auto`. The other conversations
resume from the desktop.

While a workspace closes, launches and messages to it return the error
`workspace_closed`. A second close returns the error `workspace_closing`. Then
the host forgets the terminals. It removes the worktree with
`git worktree remove`. It adds `--force` only when `force` is set. If
`keep_branch` is not set, it deletes the branch. Then it marks the workspace as
closed. If the branch that the host tried to delete stays, the result names it
in `branch_kept` with the `detail` from Git. This occurs when the lookup had an
error without `force`, or when `git branch -D` had an error.

### MCP server

Remote workspaces can use [workspace supervision](supervision.md). The host
accepts a configured `supervisor_id` when it makes a workspace. It gives tools
to show the configured destination, to add or remove supervision and to link a
GitHub PR. The publication does not depend on a desktop attachment. The
registry adds an optional workspace `supervision` field. It also adds a root
supervision state, with default values, for execution observations, PR
associations and the durable outbox. Desktop merges keep the supervisor link
of the host. Credentials stay in the private `supervision.json`. They do not go
into the registry or MCP responses. Current callers stay without supervision.

`prometeu-host mcp [--socket PATH]` speaks MCP through stdio, with JSON-RPC
messages on separate lines. It answers `initialize`, `ping`, `tools/list` and
`tools/call`. Notifications get no reply. It reads `PROMETEU_AGENT_TOKEN` and,
as the client label, `PROMETEU_AGENT_NAME`. It connects only when it must send a
request.

When `PROMETEU_AGENT_HOST_UID` is set, the server sends the token only to a
listener whose peer credentials give that user ID. The user namespace of the
client gives this ID. `agents setup` always adds the variable, with the local
user by default (see above). If `--client-uid` is missing or is not a number,
the command stops with an error before it writes anything.

The server answers `initialize` with the protocol version of the client when it
is 2025-06-18, 2025-03-26 or 2024-11-05. For other versions, it answers with
2025-06-18. If the connection has an error before the server sends a request,
the server connects again and sends the request one more time. If the connection
is lost after the server sent the request, the server sends again only the
read-only methods (`projects_list`, `workspace_list`, `workspace_get`,
`conversation_read`, `todos_list`). For all other methods, it tells that the outcome is
unknown.

Each tool agrees with one agent method: `list_projects`,
`add_project`, `clone_project`, `create_workspace`, `list_workspaces`,
`get_workspace`, `open_conversation`, `read_conversation`, `send_message`,
`interrupt_conversation`, `close_workspace`, `list_todos`, `create_todo` and
`set_todo_status`. The results are formatted JSON text. Host errors set `isError`.

Tests: `src-tauri/crates/host/tests/agents.rs` operates the agent socket and the
MCP server against the actual binary with a substitute Claude. The unit tests in
`agents.rs`, `registry.rs`, `transcript.rs`, `naming.rs` and `mcp.rs` examine
the remaining code. These tests also apply:

- `the_generated_local_config_lists_tools_and_relays_calls` runs the command,
  the arguments and the environment that setup prints, without changes for a
  deployment.
- `a_relative_root_generates_an_mcp_config_usable_from_another_directory`
  examines relative roots before the socket is there, and actual tool calls from
  a different folder.
- `setup_rejects_an_oversized_absolute_socket_before_saving` examines the
  refusal without the enable operation, and the recovery with a short explicit
  socket path.
- `setup_under_a_private_home_never_opens_parent_permissions` examines private
  home folders.
- `setup_preserves_legacy_access_and_disable_cleans_it_up` examines current
  paths, tokens, folder modes and the cleanup of legacy permissions.
- The desktop test `setup_output_becomes_a_masked_local_mcp_config` examines the
  token mask.

## Compatibility

If older desktops or hosts cannot ignore a protocol change, the change
increases `PROTOCOL`. This changes the name of the socket. In one protocol,
changes only add fields. The attachment identity, `hook_resolved`,
`clients_list` and `client_detach` only add to the protocol. An older host
ignores the identity and continues to replace attachments. An older desktop
does not ask for hooks by device, so it gets the hooks that its device did not
get. The agent socket has its own `AGENT_PROTOCOL`, in its hello frame. The
registry has its own `version`. Tests:
`src-tauri/crates/host/tests/host.rs` and the `remote` unit tests.

## Event delivery during MCP setup

The MCP connection form of the desktop can do the provisioning of the
supervisor of the remote host. It uses the current Cloud account and SSH
access. The added `supervision identity` command of the host gives its stable
public executor ID before setup. `supervision test` examines the publication up
to the delivery receipt of the receiver. Ingestion credentials go only to the
standard input of setup. They do not go into the copied MCP configuration.
Current connections can enable events without a rotation of their MCP token.
See the [supervision contract](supervision.md) for the endpoints, the IPC, the
recovery after failures, the compatibility and the tests.
