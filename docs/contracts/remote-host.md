---
type: Contract
title: "Remote host protocol"
description: "Installation, SSH attachment, NDJSON methods and events of prometeu-host."
status: stable
---

# Remote host protocol

Decisions: [ADR 0101](../decisions/0101-remote-machines.md) and, for the registry
and agent clients, the proposed [ADR 0103](../decisions/0103-agents-on-remote-machines.md).
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
  agents.json                  optional `{ "socket": "<absolute path>" }`, 0600
  agents/token                 agent token, 0600
  agents/agents-v1.sock        default agent socket, 0666 in a 0700 directory
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
pending update.

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
| `registry_put` | `project?`, `workspace?` (registry records) | `{}`; inserts a project by path, inserts or replaces a workspace by ID |
| `registry_close` | `id` | `{}`; marks a workspace closed after the desktop cleaned it |
| `stats` | `workspaces` | per workspace slice `{ memory, memory_max, cpu_usec, cpu_max }`, plus `memory_total`, `memory_available`, `load`, `cpus`, `terminals` |
| `shutdown` | `force?` | `{}`, then the host exits; without `force` it rejects with `host_busy` while a terminal runs |

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

## Observed state

The host reads the event name of every hook it serves and keeps, per
conversation, `state` and `state_at`: `idle` after `SessionStart`, `Stop` or
`StopFailure`; `working` after `UserPromptSubmit` or a tool hook; `permission`
after `PermissionRequest`. `Notification` records its `message` as
`notification` until the next working hook. A conversation whose terminal is
not running reports `ended`; a running one with no hook yet reports `starting`.

## Agent socket

Agents connect to the path in `agents.json`, or `<root>/agents/agents-v1.sock`.
The host checks that path every 3 seconds and rebinds it, so `prometeu-host
agents setup [--socket PATH] [--rotate]` applies without a restart. That command
creates or rotates the token and prints an MCP configuration. An existing entry
at the path is replaced only when it is a socket that does not answer;
otherwise the host logs the refusal and leaves it alone. When the path changes,
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
| `workspace_close` | `id`, `force?` | `{}` |

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
it, and a `{ role: "system", kind: "skipped" }` message reports its size.

`conversation_send` writes the text as a bracketed paste and, 150 ms later,
Enter. It relaunches an ended conversation with the text as its prompt.

`workspace_close` refuses, unless `force`, a worktree with `git status
--porcelain` output (`uncommitted_changes`). Unless `keep_branch`, it also
refuses one whose `HEAD` has commits on neither the base, resolved in the main
checkout, nor any remote (`unpushed_commits`). It checks before ending the
conversations' terminals and again after they exit. When the second check
refuses, the conversations stay ended and `conversation_send` resumes them.
While a workspace is closing, launches and messages into it fail with
`workspace_closed`, and a second close fails with `workspace_closing`. It then
forgets the terminals, removes the worktree (`git worktree remove`, with
`--force` only when `force` is set) and, unless `keep_branch`, deletes the
branch. Finally it marks the workspace closed.

### MCP server

`prometeu-host mcp [--socket PATH]` speaks MCP over stdio, newline-delimited
JSON-RPC, and answers `initialize`, `ping`, `tools/list` and `tools/call`.
Notifications get no reply. It reads `PROMETEU_AGENT_TOKEN` and, as the client
label, `PROMETEU_AGENT_NAME`, and connects lazily. It answers `initialize` with
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
