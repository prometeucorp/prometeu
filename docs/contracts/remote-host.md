---
type: Contract
title: "Remote host protocol"
description: "Installation, SSH attachment, NDJSON methods and events of prometeu-host."
status: stable
---

# Remote host protocol

Decisions: [ADR 0101](../decisions/0101-remote-machines.md); for the registry
and agent clients, [ADR 0103](../decisions/0103-agents-on-remote-machines.md); for
several computers attached at once,
[ADR 0104](../decisions/0104-several-computers-on-one-machine.md).
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
~/.prometeu-remote[-dev]-agents/   default agent directory, 0700
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
exec "$HOME/<root>/host/<hash>/prometeu-host" attach --root "$HOME/<root>" \
  --device '<device id>' --label '<computer name>' --release '<app version>'
```

`attach` starts `serve --release <app version>` with `setsid` when the socket
does not answer, then sends
`{"attach":{"protocol":1,"device":"…","label":"…"}}` and relays stdio. Desktops
before ADR 0104 send neither field.

### Several computers

The device matches `[A-Za-z0-9_-]{1,128}`; another value counts as none. The
label is cut to 64 characters without control characters. The host serves at
most 8 attachments. A new attachment replaces those with the same device,
including the empty one, with `replaced`; attachments from other devices remain.
Beyond the limit it answers `{"event":"error","error":"busy"}` and closes. An
attachment with more than 16 MiB of frames waiting to be written is closed; file
streaming waits below half of that instead.

Every attachment that called `terminal_attach` for a terminal receives its
`output` and `exit`, and every attachment receives `registry`. Hooks other than
permission requests are kept per terminal (the last 512) with the set of them
each device confirmed. Desktops send `hook_reply` for every hook, and for these
hooks it is that confirmation; each reply confirms only its own hook, whatever
order replies arrive in. `terminal_attach` sends a device every kept hook it has
not confirmed. The first time a terminal serves a device, even with no hooks
kept yet, the hooks another device already confirmed count as confirmed for it,
and every later hook is owed to it. A hook queued to an attachment that was
already dead is therefore sent again when its desktop comes back. A desktop that
handled a hook but lost the connection before its confirmation arrived can
receive it twice. A permission request is sent to every attachment following the
terminal and is not kept. The first `hook_reply` with output is printed; the
others that received it get `hook_resolved`. An empty reply, or an attachment
ending, abstains, and when every recipient abstained the hook prints nothing.
When the wait ends without a decision, recipients also get `hook_resolved`.

The PTY takes the size of the attachment that opened the terminal or last wrote
to it. `terminal_resize` from another attachment is recorded and applied when
that one writes. With no such attachment present, any resize applies.
`terminal_forget` from an attachment while another still follows the exited
terminal only stops this one following it; the terminal is dropped when the last
follower forgets it or ends. Exit status 127 or
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
| `ready` | `protocol`, `version`, `executable`, `pid`, `home`, `user`, `root`, `os`, `arch`, `linger`, `scopes`, `clients`, `release` | first frame of every attachment; `clients` is true when several computers may attach, `release` is the app version that started the host or null |
| `replaced` | none | the same computer attached again, or another computer disconnected this one; this one is detached |
| `hook_resolved` | `hook` | another attachment decided that permission request, or nobody did in time; withdraw it |
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
| `terminal_open` | `id`, `workspace?`, `argv`, `cwd`, `env?`, `unset?`, `cols`, `rows`, `limits?` | `{ pid, unit }`; rejects with `terminal_exists` an `id` that is running or whose process another open is starting, so concurrent opens start one process |
| `terminal_attach` | `id`, `seq` | `{ data, reset, seq, running, code, pid }`, then live `output`/`exit` and queued `hook` frames |
| `terminal_write` | `id`, `data` (base64) | `{}` |
| `terminal_resize` | `id`, `cols`, `rows` (1–1000) | `{}` |
| `terminal_signal` | `id`, `signal`: `interrupt`, `terminate`, `kill` or `hangup` | `{}` |
| `terminal_stop` | `id`, `instance` | `{}`; kills only the listed process instance; rejects a replaced or ended process with `terminal_changed` |
| `terminal_forget` | `id` | `{}`; drops an exited terminal and `sessions/<id>`; rejects a running terminal |
| `terminal_list` | none | `[{ id, instance, workspace, pid, unit, started, running, code, seq }]`; older hosts omit `instance` |
| `watch` | `id`, `dir` (absolute), `depth?` (default 3, at most 6), `offsets?` | `{}`; streams `file` events every 250 ms while this attachment lives |
| `watch_flush` | `id` | `{}` after every byte the watch had not sent is on the stream |
| `hook_reply` | `hook`, `stdout` (base64) | `{}`; empty `stdout` abstains |
| `clients_list` | none | `[{ device, label, connected, current }]`, `connected` in Unix seconds, `current` for the asking attachment |
| `client_detach` | `device` | `{}`; sends `replaced` to that device's attachment; `unknown_client` for none or the asking one |
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

## Desktop process inspection

Settings exposes **Running processes** on each connected machine. The additive
desktop IPC commands `machine_terminals { id }` and
`machine_terminal_stop { id, terminal, instance }` use `terminal_list` and
`terminal_stop`. They require a connected machine and only address terminals
owned by its host, never arbitrary PIDs. Each terminal launch receives a fresh
UUID in `instance`, retained across attachments. The confirmation carries that
exact instance through IPC to the host, which checks it and signals the same
process handle without looking up the terminal ID again. Reusing a conversation
ID or PID cannot retarget an earlier confirmation; the process control also
serializes signaling with reaping to prevent signaling a reused OS PID.

The new host method and list field are additive within protocol 1, with no
persisted format change. Old desktops still use `terminal_signal`. Hosts without
`instance` remain inspectable, but their stop buttons are disabled with guidance
to end work from the conversation or machine terminal before upgrading. A stop
sent after reconnecting to an old host fails as an unknown method; there is no
fallback to an unguarded signal. Adding an optional identity to `terminal_signal`
would be unsafe because older hosts silently ignore unknown parameters.

The list uses the host's `running` flag, including idle conversations, sign-in
terminals and terminals absent from the desktop board. Board names are resolved
only within that machine. It shows the conversation or terminal ID, workspace,
PID and start time, refreshes every three seconds while open, and clears stale
controls when a query fails. Ending a process requires confirmation and kills
its process group without removing its workspace or saved transcript. Normal
exit handling still drains and mirrors its output. The pending-update notice
explains that idle processes also block replacement; remaining host requests
can still delay replacement after the list becomes empty.

`src/machine-processes.test.ts` covers host liveness, machine-scoped labels,
confirmation races, legacy hosts, targeted stopping and polling lifetime.
`stopping_a_listed_process_never_kills_a_replacement_under_the_same_id` verifies
that stale or missing instances leave a replacement running and its current
instance can stop it. The existing
`terminals_survive_detachment_and_replay_the_gap` host test verifies list and
kill compatibility, including the listed state after exit.

## Desktop project selection

Settings > Projects lists registered folders filtered by computer. The launcher
uses the same exact `Project.host` filter; origin aliases only group the sidebar.
Remote registration and cloning use the existing `remote_list_dir` and
`remote_add_project` desktop commands in separate dialogs. They do not change
the host protocol or persisted project IDs. Existing backend origin resolution
remains compatible with callers that pass a local project ID.

`src/machines.test.ts` covers local, selected-host, other-host and aliased clones.
`src/settings-navigation.test.ts` preserves the saved `projects` destination.

## Connected computers

The additive desktop IPC commands `machine_clients { id }` and
`machine_client_detach { id, device }` use `clients_list` and `client_detach` on a
connected machine. Settings offers them from the machine's menu only when
`ready.clients` is true, lists this computer first and asks before
disconnecting another one. A disconnected desktop shows **In use by another
computer** and stops reconnecting, as after a replacement. An older host answers
`clients_list` as an unknown method, which the desktop reports as needing a host
update. `src/machine-clients.test.ts` covers ordering and confirmation.

The desktop also passes `--release` and keeps an idle host from another build
when `ready.release` is newer than its own version, so computers on different
releases never retire each other's host. A host without `release` is replaced
when idle, as before.

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

`agents setup` creates missing socket directories with mode 0700 and leaves
existing directory permissions unchanged. It never opens the home or other
parent folders to additional users. Setup and the host's bind refuse a directory
not owned by the host's user or writable by its group or others, and a parent
owned by anyone other than that user or root, or writable by group or others
without the sticky bit. Setup also refuses a directory holding a `prometeu-mcp`
it did not record, and a socket path taken by something other than a socket.

Setup records the socket and `<directory>/prometeu-mcp` in `agents.json`, removes
a previously recorded client elsewhere, copies itself there (0755), then creates
or rotates the token. The host refreshes the recorded copy atomically, only when
it differs, every time it binds the socket. `agents disable` deletes the token
and the recorded copy. For compatibility, setup preserves the `opened` list
written by older versions without changing those folders. Disabling still
removes `o+x` from those legacy folders, skips only folders confirmed gone,
retains records it could not check or close, and fails naming them. The removed
`--open-home` flag fails before any setup changes.

`agents status --json` prints `{ enabled, socket, serving, error }`: whether
something answers at the socket, and the last bind error, stored in
`agents/bind-error` and removed once the host serves or agents are disabled.

`agents setup` prints a local stdio MCP entry:
`{ command: "<directory>/prometeu-mcp", args: ["mcp", "--socket", "<socket>"], env }`.
Both paths are resolved against setup's working directory when `--root` is
relative, so the copied configuration works from another directory. Resolution
does not require the daemon's socket to exist or rewrite stored paths. Setup
validates the emitted address against the platform's Unix socket path limit
before writing configuration or tokens; an invalid address is rejected with
guidance to choose another path using `--socket`.
`env` holds `PROMETEU_AGENT_TOKEN`,
`PROMETEU_AGENT_NAME: "agent"` and `PROMETEU_AGENT_HOST_UID`. The expected peer ID
is the setup user's actual UID, or the explicit `--client-uid` override. Setup
makes no assumption about user namespaces. With `--json` it prints
`{ token, socket, client, moved_from, server }`.

The desktop's `machine_agents_enable { id, rotate }` and `machine_agents_disable`
run these commands through `exec` with the installed build they attach with,
even when an older daemon is still running. Enabling waits up to 8 seconds for
`serving` or `error` before publishing status. It returns
`{ config, preview, moved_from }` to Settings: the pretty-printed
`{"mcpServers": {"prometeu": server}}` with the token for copying, and a masked
preview for display. `MachineView.agents` carries status read at each attachment.
The desktop IPC and mock change together; the setup output no longer contains
mounts, namespace hints or a permission-consent outcome.

Any MCP client that can execute the command and reach the socket can use this
configuration. The default configuration targets the same machine and Unix user
as the daemon. Containers, volumes, remote command execution, network access and
permission changes belong to the integrator. No HTTP listener is added. An
integration that maps paths or user identities must adapt the command, socket
path and expected peer UID without removing token or peer authentication.
Existing client configurations, recorded socket paths and directory permissions
remain usable across upgrades; generating a new configuration produces local
paths even for a previously customized installation.

An existing socket entry is replaced only when a non-blocking connection is
refused. An answering socket or a full queue is kept; otherwise the host logs
the refusal once and retries at every check. When the path changes, the old
listener stops and its socket is removed.

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
that user ID, as the client's user namespace sees it. `agents setup` always
adds the variable using the local user by default (see above); a missing or
non-numeric `--client-uid` fails the command before anything is written. It answers `initialize` with
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
rest. `the_generated_local_config_lists_tools_and_relays_calls` executes the
emitted command, arguments and environment without deployment-specific edits.
`a_relative_root_generates_an_mcp_config_usable_from_another_directory` covers
relative roots before the socket exists and actual tool calls from another directory.
`setup_rejects_an_oversized_absolute_socket_before_saving` covers rejection
without enabling agents and recovery with a short explicit socket path.
`setup_under_a_private_home_never_opens_parent_permissions` covers private homes;
`setup_preserves_legacy_access_and_disable_cleans_it_up` covers existing paths,
tokens, directory modes and legacy permission cleanup. The desktop test
`setup_output_becomes_a_masked_local_mcp_config` verifies token masking.

## Compatibility

A protocol change that older desktops or hosts cannot ignore bumps `PROTOCOL`,
which renames the socket. Fields are additive within a protocol. The attachment
identity, `hook_resolved`, `clients_list` and `client_detach` are additive: an
older host ignores the identity and keeps replacing attachments, and an older
desktop never asks for hooks by device, so it receives what its device missed. The agent socket
has its own `AGENT_PROTOCOL`, carried in its hello frame. The registry carries its
own `version`. Tests:
`src-tauri/crates/host/tests/host.rs` and the `remote` unit tests.
