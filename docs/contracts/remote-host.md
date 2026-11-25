---
type: Contract
title: "Remote host protocol"
description: "Installation, SSH attachment, NDJSON methods and events of prometeu-host."
status: stable
---

# Remote host protocol

Decisions: [ADR 0101](../decisions/0101-remote-machines.md) and, for the registry
and agent clients, [ADR 0103](../decisions/0103-agents-on-remote-machines.md).
Implementation: `src-tauri/crates/host/` (host) and `src-tauri/src/remote/` (desktop).

## Installation

The desktop ships one Linux x86_64 build. The build is found through
`PROMETEU_REMOTE_HOST`, then `Contents/Resources/remote-host/prometeu-host-x86_64-linux`
in the app bundle, then (debug only) `src-tauri/target/x86_64-unknown-linux-musl/release/prometeu-host`.
Its install directory is the first 16 hex digits of its SHA-256.

```text
~/.prometeu-remote[-dev]/
  host/<hash>/prometeu-host   immutable build, 0700
  current -> host/<hash>       last installed build
  previous -> host/<hash>      build it replaced, kept for rollback
  host-v1.sock                 0600, one per protocol version
  host-v1.lock                 flock held by the serving host
  host.log                     host stderr
  sessions/<tab>/settings.json Claude hook settings, 0600
  worktrees/<repo>/<branch>/   worktrees created by the desktop or the host
  registry.json                the machine's projects and workspaces, 0600
  agents.json                  `{ socket, client, opened }` written by setup, 0600
  agents/bind-error            why the host cannot serve the agent socket, while it cannot
  agents/token                 agent token, 0600; agents are enabled while it exists
~/.prometeu-remote[-dev]-agents/   default agent directory, 0755
  agents-v1.sock               agent socket, 0666
  prometeu-mcp                 client executable the host keeps current, 0755
```

Installation checks that `uname -sm` is `Linux x86_64`, streams the build over
SSH stdin to a temporary file, and verifies its SHA-256 before the rename. It
then updates `current` and `previous`, and removes every other build that no host
process runs and no `sessions/*/settings.json` names (running conversations call
the hook helper of the build that launched them). It then tries
`loginctl enable-linger`. A host deletes `sessions/` when it starts, since it owns
no terminal yet.

## Attachment

The desktop runs, through the system `ssh` with `BatchMode=yes`,
`ControlMaster=no`, `ControlPath=none`, `ServerAliveInterval=5` and
`ServerAliveCountMax=3`:

```text
exec "$HOME/<root>/host/<hash>/prometeu-host" attach --root "$HOME/<root>"
```

`attach` starts `serve` with `setsid` when the socket does not answer, then
sends `{"attach":{"protocol":1}}` and relays stdio. Exit status 127 or
`No such file` before the ready frame means the build is not installed. If the
ready frame's `executable` belongs to another build, the desktop shuts that host
down only when it runs no terminal; otherwise it keeps using it and reports a
pending update. While the update is pending and the attachment lasts, the desktop
checks again when an `exit` event arrives and at least every 60 seconds, and
never while one of its requests is in flight. The host's own `shutdown` check
still decides: `exec`, `file_write` and `terminal_open` delay a non-forced
`shutdown` like the agent requests that change the machine, and fail with
`host_stopping` once one has started.

## Framing

NDJSON, at most 8 MiB per line. Requests are `{ id, method, params }`; replies
are `{ id, result }` or `{ id, error }` with a diagnostic string. Frames with an
`event` field are unsolicited. Requests are handled in the order they arrive,
except `exec`, `stats` and `shutdown`, which run beside the stream, so a client
may send an attach right after its open, or input without waiting for replies. Nothing is retried: a lost reply has an
unknown outcome.

## Events

| Event | Fields | Meaning |
| --- | --- | --- |
| `ready` | `protocol`, `version`, `executable`, `pid`, `home`, `user`, `root`, `os`, `arch`, `linger`, `scopes` | first frame of every attachment |
| `replaced` | none | another desktop attached; this one is detached |
| `output` | `id`, `seq`, `data` (base64) | terminal output after the attachment's replay |
| `exit` | `id`, `code`, `seq` | terminal exited, after its output was drained (up to 2 s, in case a background process keeps the PTY open); also sent at attachment for an exited terminal |
| `hook` | `hook`, `session`, `permission`, `env`, `payload` (base64), `at` | a native hook invocation for the terminal `session` |
| `file` | `watch`, `path`, `offset`, `reset`, `data` (base64) | bytes of a watched file from `offset`; `reset` restarts it from zero |
| `registry` | none | an agent request that may add or close registry entries finished; the desktop synchronizes again |

## Methods

| Method | Params | Result |
| --- | --- | --- |
| `ping` | none | `{}` |
| `exec` | `argv`, `cwd?`, `env?`, `stdin?` (base64), `timeout_ms?` (default 30 s, at most 600 s) | `{ code, timeout, stdout, stderr }` (base64, 4 MiB each) |
| `file_write` | `path` (absolute), `data` (base64), `mode?` (default 0600) | `{}`; parents created 0700, atomic rename |
| `file_read` | `path`, `limit?` | `{ data }` |
| `terminal_open` | `id`, `workspace?`, `argv`, `cwd`, `env?`, `unset?`, `cols`, `rows`, `limits?` | `{ pid, unit }`; rejects a running `id` with `terminal_exists` |
| `terminal_attach` | `id`, `seq` | `{ data, reset, seq, running, code, pid }`, then live `output`/`exit` and queued `hook` frames |
| `terminal_write` | `id`, `data` (base64) | `{}` |
| `terminal_resize` | `id`, `cols`, `rows` (1–1000) | `{}` |
| `terminal_signal` | `id`, `signal`: `interrupt`, `terminate`, `kill` or `hangup` | `{}` |
| `terminal_forget` | `id` | `{}`; drops an exited terminal and `sessions/<id>`; rejects a running terminal |
| `terminal_list` | none | `[{ id, workspace, pid, unit, started, running, code, seq }]` |
| `watch` | `id`, `dir` (absolute), `depth?` (default 3, at most 6), `offsets?` | `{}`; streams `file` events every 250 ms while this attachment lives |
| `watch_flush` | `id` | `{}` after every byte the watch had not sent is on the stream |
| `hook_reply` | `hook`, `stdout` (base64) | `{}` |
| `registry_list` | none | the registry, with each conversation's `state`, `state_at` and `notification` |
| `registry_put` | `project?`, `workspace?` (registry records) | `{}`; inserts a project by path; inserts a workspace by ID, or merges into the one held: its title and new conversations, keeping every other field and conversation |
| `registry_close` | `id` | `{}`; marks a workspace closed after the desktop cleaned it |
| `stats` | `workspaces` | per workspace slice `{ memory, memory_max, cpu_usec, cpu_max }`, plus `memory_total`, `memory_available`, `load`, `cpus`, `terminals` |
| `shutdown` | `force?` | `{}`, then the host exits; without `force` it rejects with `host_busy` while a terminal runs, an `exec`, `file_write` or `terminal_open` is in progress, or an agent request that changes the machine is; once it starts, those fail with `host_stopping` |

Terminal and watch IDs match `[A-Za-z0-9_-]{1,128}`. Unknown terminals answer
`unknown_terminal`, which the desktop treats as a process lost with the host.

### Terminal sequence

Every output chunk gets the next sequence number. `terminal_attach` with the last
sequence the client applied replies with the bytes after it. When the retained
2 MiB no longer reaches that sequence, or the sequence is ahead of the host's,
the reply carries the whole buffer with `reset: true`, and the desktop clears the
screen first. Replay, live output and queued hooks are ordered under the
terminal's buffer lock, so they never overlap or leave a gap.

### Limits

With linger enabled and `systemd-run` available (`ready.scopes`), a terminal
opened with `workspace` runs as
`systemd-run --user --scope --slice=prometeu-w<uuid>.slice --unit=prometeu-t<id>-<n>`.
`limits.memory_max`, `memory_high`, `cpu_quota` and `tasks_max` (systemd syntax)
are applied to the slice with `systemctl --user set-property --runtime`. Without
scopes, limits are ignored.

## Hooks

Claude's settings name `'<host>' hook --root '<root>'` as the command of every
hook. The hook process reads at most 1 MiB of stdin and forwards every
`PROMETEU_TERMINAL_HOOK_*` variable with the payload. Only `PermissionRequest`
waits for a reply (up to 300 s). Every other hook is acknowledged at once, then
delivered, or queued when the terminal has no attached desktop. The desktop
rewrites transcript paths to its mirror, replaces the bridge socket, token and
nonce with its own, and returns any decision the local bridge produced. A failing
hook command prints nothing and exits 0.

## Transcript mirror

After a terminal exits, the desktop calls `watch_flush` on its conversation's
watch, so the conversation reads its last turn before it stops, and then
`terminal_forget`. When it reattaches and finds a finished terminal that belongs
to a conversation on the board, it adopts that terminal without starting Claude,
collects its output, queued hooks and transcript the same way, and forgets it.
Finished terminals that no conversation will read again are forgotten at once.

For a conversation in `<cwd>`, the desktop watches
`<home>/.claude/projects/<slug>` (depth 3, which includes subagents). It writes
the bytes to `<root>/remote/<machine>/mirror/<absolute remote path>` (0700
directories, 0600 files), and sends the mirror's sizes as offsets after every
reattachment. The slug replaces every character outside `[A-Za-z0-9]` with `-`.

Before adopting a terminal, the desktop waits for `watch_flush` to finish. A
Claude `SessionStart` hook may have bound a transcript path before the first
prompt creates that file. When the bound file is still absent and the journal
contains neither native receipts nor prepared input, and the canonical log
contains only status metadata, adoption keeps the existing process, identity
and journal while observing its first prompt as new input. This exception never
launches a replacement process or bypasses verification of existing history.
The next authenticated hook binds observation to the transcript as usual.
`remote_reattachment_before_first_prompt_preserves_identity_and_requires_empty_history`
in `chat/terminal/ingestion/tests.rs` covers this behavior without changing any
persisted format or host protocol.

## Desktop project selection

Settings > Projects lists registered folders filtered by computer. The launcher
uses the same exact `Project.host` filter; origin aliases only group the sidebar.
Remote registration and cloning use the existing `remote_list_dir` and
`remote_add_project` desktop commands in separate dialogs. They do not change
the host protocol or persisted project IDs. Existing backend origin resolution
remains compatible with callers that pass a local project ID.

`src/machines.test.ts` covers local, selected-host, other-host and aliased clones.
`src/settings-navigation.test.ts` preserves the saved `projects` destination.

## Registry

`registry.json` is `{ version: 1, projects, workspaces }`; only the host writes
it, with an atomic rename through a uniquely named temporary file. A registry
with a newer `version` is refused instead of being rewritten without the fields
this host does not know. Records:

- project: `{ path, name, origin?, added_at, added_by }`;
- workspace: `{ id, project, title, branch, base, worktree, created_at,
  created_by, closed, keep_branch, conversations }`;
- conversation: `{ id, title, created_at, permission }`, where `id` is the tab,
  Claude session and terminal ID and `permission` is `auto` or `ask`;
- creator: `{ kind: "desktop" | "agent", name? }`.

Unknown fields are ignored. Times are Unix seconds.

The desktop synchronizes its board with `registry_list` on every attachment
and on every `registry` event (`src-tauri/src/remote/registry.rs`). It adopts
missing projects and open workspaces as remote board entries. Their
conversations become terminal-mode Claude tabs with the same IDs, and the
workspace names an agent creator in `creator`. It adds conversations an agent
opened in a workspace it already shows, and marks workspaces an agent closed as
cleaned and archived. It records with `registry_put` its own remote workspaces
and conversations once their tab is on the board, and the projects added on the
machine. Each synchronization also records the projects, open workspaces and
conversations that the registry lacks, and repeats `registry_close` for a
workspace the desktop cleaned that the registry still lists as open. An entry
whose ID the board already uses for a local workspace, a workspace on another
machine or another tab is never adopted or changed. A conversation an agent
opened before closing its workspace still becomes a tab of the closed
workspace. Synchronization and the terminal reattachment that follows run one
at a time per machine, and `registry` events that arrive while one is queued
join it. It calls `registry_close` after cleaning a workspace. A
workspace removed from the board without cleanup, a conversation whose tab was
closed, or a project removed from the board (until it is added again) stays in
the registry and is listed in
`<root>/remote/<machine>/dismissed.json` on the desktop, which keeps it from
being adopted again. An adopted conversation without a terminal on the
host gets the transcript binding its first hook would have written.

## Observed state

The host reads the event name of every hook it serves and keeps, per
conversation, `state` and `state_at`: `idle` after `SessionStart`, `Stop` or
`StopFailure`; `working` after `UserPromptSubmit` or a tool hook; `permission`
after `PermissionRequest`. `Notification` records its `message` as
`notification` until the next working hook. A conversation whose terminal is
not running reports `ended`; a running one with no hook yet reports `starting`.

## Agent socket

Agents connect to the path in `agents.json`. Setup records `agents-v1.sock` in
the default agent directory `<root>-agents` (`~/.prometeu-remote-agents`) unless
`--socket` names another. Without a record, which is an install from before that
default, the path stays `<root>/agents/agents-v1.sock`; the next setup moves it
and reports `moved_from`. The host serves it
only while `agents/token` exists. It checks the token and the path every 3
seconds and rebinds, so `prometeu-host agents setup [--socket PATH] [--rotate]
[--client-uid ID] [--json]` and `agents disable` apply without a restart. When
agents are disabled, the listener stops and its socket is removed.

`agents setup` creates the socket's directory 0755 when missing. Setup and the
host's bind refuse a directory not owned by the host's user or writable by its
group or others, and one with a parent folder that is owned by another user than
that user or root, or writable by group or others without the sticky bit. Setup
also refuses a directory holding a `prometeu-mcp` it did not record, and a socket
path taken by something other than a socket. When that directory is under
`$HOME`, other users must be able to traverse each folder from the home down to
it. Folders the person owns that lack `o+x` are changed only with
`--open-home`: without it, setup changes nothing, exits 3 and names them, or with
`--json` prints `{ needs_open: [paths] }` and exits 0. With it, setup records them
in `opened`, adds `o+x` and reports them. `agents disable` removes `o+x` from
every recorded folder, skips only those confirmed gone, keeps recorded those it
could not check or close, and fails naming them. It records the
socket and `<directory>/prometeu-mcp` in `agents.json`, removes a previously
recorded client elsewhere, copies itself there (0755), then creates or rotates
the token. The host refreshes the recorded copy, through a temporary file and a
rename and only when it differs, every time it binds the socket. `agents
disable` deletes the token and the recorded copy. `agents status --json`
prints `{ enabled, socket, serving, error }`: whether something answers at the
socket, and the reason the host last gave for not serving it, which it writes to
`agents/bind-error` when a bind fails and removes once it serves or agents are
disabled.

`agents setup` prints a mount (`<directory>:/prometeu:ro`) and an MCP entry
`{ command: "/prometeu/prometeu-mcp", args: ["mcp", "--socket",
"/prometeu/<socket name>"], env }`. `env` holds `PROMETEU_AGENT_TOKEN`,
`PROMETEU_AGENT_NAME: "agent"` and `PROMETEU_AGENT_HOST_UID`. That ID is
`--client-uid` when given, and otherwise `/proc/sys/kernel/overflowuid`
(usually 65534), which the text output explains. With `--json` it prints
`{ token, socket, client, opened, moved_from, client_uid, local_uid, mount: {
source, target, read_only }, server }` instead. The desktop's `machine_agents_enable`
and `machine_agents_disable` commands run these through `exec` with the build
they attach with. Enabling passes `--open-home` only after Settings asks the
person about the folders named in `needs_open`, then waits up to 8 seconds for
`serving` or `error` before publishing the status.
They return the mount and the pretty-printed `{"mcpServers": {"prometeu":
server}}` to Settings only, both as copied (`config`) and with the token masked
for the screen (`preview`), and `MachineView.agents` carries the
status read at each attachment. An existing entry
at the path is replaced only when it is a socket whose non-blocking connection
attempt is refused; one that answers or whose queue is full is kept;
otherwise the host leaves it alone, logs the refusal once and tries again at
every check, so the path is taken as soon as it is freed. When the path changes,
the previous listener stops and its socket file is removed.

The first frame is `{"hello":{"protocol":1,"token":"…","client":"<label>"}}`.
The token is read from `agents/token` and compared in constant time on the hello
frame and again before every request. A mismatch answers
`{"event":"error","error":"unauthorized"}`, an unknown protocol `"protocol"`,
and the connection closes. At most 16 connections may wait for their hello, each
for at most 10 seconds; more are closed without an answer. At most 64
authenticated connections are served; the next gets `"busy"`. Otherwise the
host answers `{"event":"ready","protocol":1,"version"}` and serves `{ id,
method, params }` requests in order with `{ id, result }` or `{ id, error }`.
`client` (at most 64 characters) is recorded as the creator's `name`.
Requests that change the machine (`project_add`, `project_clone`,
`workspace_create`, `conversation_open`, `conversation_send`,
`conversation_interrupt`, `workspace_close`) delay a non-forced `shutdown`. Once a
shutdown has started, they fail with `host_stopping`.

The token authorizes running commands as the host's user, so only that user may
be able to write to the socket's directory: whoever can replace the socket can
collect the token from the next client.

| Method | Params | Result |
| --- | --- | --- |
| `projects_list` | none | projects |
| `project_add` | `path` (absolute, inside a Git repository) | the project registered at the repository's top level |
| `project_clone` | `url`, `path?` (default `~/code/<name>`) | the cloned project |
| `workspace_create` | `project` (path or name), `prompt`, `title?`, `branch?`, `base?` | the workspace with its first conversation |
| `workspace_list` | `all?` | open workspaces, or all of them |
| `workspace_get` | `id` | the workspace |
| `conversation_open` | `workspace`, `prompt` | the conversation and its `workspace` |
| `conversation_read` | `id`, `cursor?`, `limit?` (1–200, default 50) | `{ messages, cursor, more, status }` |
| `conversation_send` | `id`, `text` | `{ resumed }` |
| `conversation_interrupt` | `id` | `{}`; sends Escape |
| `workspace_close` | `id`, `force?` | `{}`, or `{ branch_kept, detail }` when the worktree is gone but the branch could not be deleted |

Workspaces and conversations carry the observed state fields above.

`workspace_create` fetches `origin` (best effort) and picks a fresh
`prometeu/<noun>-<adjective>-<NNNN>` branch when none is given. The base is
`origin/HEAD`, or else `HEAD`. The worktree path is
`<root>/worktrees/<project name>/<flattened branch>`, with symbolic links in the
parent resolved, because Claude names transcript folders after its physical
working directory. Before Git creates anything, the host marks that exact folder
trusted in `~/.claude.json` (`projects[<path>].hasTrustDialogAccepted`) under a
lock shared by its own edits. Claude asks about folder trust even without
permission prompts, and a trusted parent does not count. The host then adds the
worktree and records the workspace; if recording fails, it removes the worktree
and the new branch. Finally it launches:

```text
claude --session-id|--resume <id> --dangerously-skip-permissions \
  --settings <root>/sessions/<id>/settings.json <prompt>
```

If the launch fails, the error names the recorded workspace and conversation:
`conversation_send` retries the launch and `workspace_close` removes the
workspace. `conversation_open` removes its conversation from the registry when
its launch fails.

The terminal is 120×40 with the workspace slice and no limits. Its hook
environment names the session and `claude`, with placeholder token and nonce
that a desktop relay replaces. Variables a parent Claude exports to its children
are unset. `--resume` is used when the transcript already exists. `PROMETEU_HOST_CLAUDE`
replaces the `claude` program, for tests.

`conversation_read` parses `~/.claude/projects/<slug>/<id>.jsonl`. A message is
`{ role, kind: text | tool_use | tool_result, text, tool?, error?, at }`. Tool
text is cut to 2,000 characters and message text to 20,000. Meta and subagent
lines are skipped. Without a cursor, it returns the last `limit` messages from
the last 8 MiB of the file. With a cursor, it returns the messages after that
byte offset, reading at most 8 MiB. The reply's `cursor` always ends at a line
boundary. A single record longer than 8 MiB is skipped: the cursor moves past
it, and a `{ role: "system", kind: "skipped" }` message reports its size. A
reply carries at most 4 MiB of serialized messages: a page ends before the
record that would exceed it, and a record that alone exceeds it keeps the
messages that fit, followed by a `skipped` message counting the rest. A tail
read drops its oldest messages to fit. A cursor that does not follow a newline
points inside a record, which a tail read returns when its window holds no line
boundary. Reading from it waits for that record to end, then continues after
it with a `skipped` message.

`conversation_send` writes the text as a bracketed paste and, 150 ms later,
Enter, holding the terminal's input across both so that concurrent messages
never merge. It relaunches an ended conversation with the text as its prompt,
except one whose `permission` is not `auto`: the host launches only without
prompts, so such a conversation must be resumed from the desktop.
`conversation_interrupt` and every other conversation method accept only IDs in
the registry, never other terminals of the host.

`workspace_close` refuses, unless `force`, a worktree with `git status
--porcelain` output (`uncommitted_changes`). It also refuses
(`unpushed_commits`) when the worktree's `HEAD`, or the branch when it is to be
deleted, has commits on neither the base, resolved in the main checkout, nor
any remote. A kept branch counts as holding its commits. A branch that Git confirms no longer exists under its recorded name (renamed or
deleted) is not checked, and only `HEAD` is. When the lookup itself fails, the
close is refused. Without `force`, only a branch the checks examined is
deleted. It checks before ending the
conversations' terminals and again after they exit. When the second check
refuses, the conversations stay ended: `conversation_send` resumes those whose
`permission` is `auto`, and the others resume from the desktop.
While a workspace is closing, launches and messages into it fail with
`workspace_closed`, and a second close fails with `workspace_closing`. It then
forgets the terminals, removes the worktree (`git worktree remove`, with
`--force` only when `force` is set) and, unless `keep_branch`, deletes the
branch. Finally it marks the workspace closed. When the branch it meant to
delete survives (the lookup failed without `force`, or `git branch -D` failed),
the result names it in `branch_kept` with Git's `detail`.

### MCP server

`prometeu-host mcp [--socket PATH]` speaks MCP over stdio, newline-delimited
JSON-RPC, and answers `initialize`, `ping`, `tools/list` and `tools/call`.
Notifications get no reply. It reads `PROMETEU_AGENT_TOKEN` and, as the client
label, `PROMETEU_AGENT_NAME`, and connects lazily. When `PROMETEU_AGENT_HOST_UID`
is set, it sends the token only to a listener whose peer credentials report
that user ID, as the client's user namespace sees it. A rootless container
usually sees the host's user as 65534. `agents setup` always adds the variable
(see above); a missing or non-numeric `--client-uid` fails the command before
anything is written. It answers `initialize` with
the client's protocol version when it is 2025-06-18, 2025-03-26 or 2024-11-05,
and with 2025-06-18 otherwise. When the connection fails before a request is
sent, it reconnects and sends the request once more. When the connection is lost
after sending, it repeats only the read-only methods (`projects_list`,
`workspace_list`, `workspace_get`, `conversation_read`). For any other method, it
reports that the outcome is unknown. Tools map one to one to the agent methods: `list_projects`,
`add_project`, `clone_project`, `create_workspace`, `list_workspaces`,
`get_workspace`, `open_conversation`, `read_conversation`, `send_message`,
`interrupt_conversation` and `close_workspace`. Results are pretty-printed
JSON text, and host errors set `isError`.

Tests: `src-tauri/crates/host/tests/agents.rs` drives the agent socket and the
MCP server against the real binary with a stand-in Claude, and the unit tests in
`agents.rs`, `registry.rs`, `transcript.rs`, `naming.rs` and `mcp.rs` cover the
rest.

## Compatibility

A protocol change that older desktops or hosts cannot ignore bumps `PROTOCOL`,
which renames the socket. Fields are additive within a protocol. The agent socket
has its own `AGENT_PROTOCOL`, carried in its hello frame. The registry carries its
own `version`. Tests:
`src-tauri/crates/host/tests/host.rs` and the `remote` unit tests.
