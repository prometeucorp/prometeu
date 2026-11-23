---
type: Contract
title: "Remote host protocol"
description: "Installation, SSH attachment, NDJSON methods and events of prometeu-host."
status: stable
---

# Remote host protocol

Decision: [ADR 0101](../decisions/0101-remote-machines.md). Implementation:
`src-tauri/crates/host/` (host) and `src-tauri/src/remote/` (desktop).

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
  worktrees/<repo>/<branch>/   worktrees created by the desktop
```

Installation checks that `uname -sm` is `Linux x86_64`, streams the build over
SSH stdin to a temporary file, and verifies its SHA-256 before the rename. It
then updates `current` and `previous`, removes every other build, and tries
`loginctl enable-linger`.

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
| `exit` | `id`, `code`, `seq` | terminal exited; also sent at attachment for an exited terminal |
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
| `terminal_forget` | `id` | `{}`; rejects a running terminal |
| `terminal_list` | none | `[{ id, workspace, pid, unit, started, running, code, seq }]` |
| `watch` | `id`, `dir` (absolute), `depth?` (default 3, at most 6), `offsets?` | `{}`; streams `file` events every 250 ms while this attachment lives |
| `hook_reply` | `hook`, `stdout` (base64) | `{}` |
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

For a conversation in `<cwd>`, the desktop watches
`<home>/.claude/projects/<slug>` (depth 3, which includes subagents). It writes
the bytes to `<root>/remote/<machine>/mirror/<absolute remote path>` (0700
directories, 0600 files), and sends the mirror's sizes as offsets after every
reattachment. The slug replaces every character outside `[A-Za-z0-9]` with `-`.

## Compatibility

A protocol change that older desktops or hosts cannot ignore bumps `PROTOCOL`,
which renames the socket. Fields are additive within a protocol. Tests:
`src-tauri/crates/host/tests/host.rs` and the `remote` unit tests.
