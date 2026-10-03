# Experimental headless conversation runtime

Status: implemented development slice. This document describes disposable stdio
execution; the preview selects the [resident transport](resident-runtime.md).
Decision: [ADR 0079](../decisions/0079-headless-conversation-slice.md).

`prometeu-runtime` builds on Unix without Tauri or GUI libraries. It owns one
Codex conversation per runtime directory and exposes development NDJSON over
stdin/stdout. The desktop continues to run in-process. Claude, Antigravity,
board operations, account switching, hub selections, sharing and
telemetry storage are not exposed by this executable yet.

## Run

Build from the repository root, then run the binary on Linux, WSL or macOS:

```sh
cargo build --manifest-path src-tauri/Cargo.toml -p prometeu-runtime --locked
src-tauri/target/debug/prometeu-runtime --root /tmp/prometeu-headless-demo --workdir "$PWD"
```

`--catalog application` selects an injected empty board for the shared desktop;
`--catalog workspace` (the default) retains the diagnostic primary workspace.
Existing catalogs always take precedence over initialization.

Use an empty directory on first run. A desktop root is rejected. `--codex`
selects an executable for composition/testing; `--model` selects the provider
model. Codex uses its existing native authentication and configuration from the
execution environment. This slice injects no Prometeu hub selection or managed
account. Approvals use the existing Ask policy and can be answered through V1
`request.respond`; the adapter retains the desktop sandbox policy.

Send one JSON request per line. Each request has `v: 1`, a numeric `id`, and a
typed `action`. Input is limited to 1 MiB per line; an oversized or invalid UTF-8
line terminates the host and shuts down its child. Invalid JSON/schema and
unsupported versions receive errors without starting work.

```json
{"v":1,"id":1,"action":{"method":"start"}}
```

Wait for the `session.identity` event before sending the first message:

```json
{"v":1,"id":2,"action":{"method":"command","frame":{"v":1,"type":"message.send","text":"Reply with hello. Do not use tools."}}}
{"v":1,"id":3,"action":{"method":"snapshot"}}
{"v":1,"id":4,"action":{"method":"stop"}}
{"v":1,"id":5,"action":{"method":"start"}}
{"v":1,"id":6,"action":{"method":"shutdown"}}
```

Wait for each relevant response/event rather than piping the entire example at
once: `stop` ends the process even if its turn is still running. `start` uses
the stored provider thread ID automatically, including after restarting the
runtime executable. A second start while an entry exists is rejected; use stop
first, including after an unexpected provider exit. Busy conversations reject
another `message.send`; no implicit queue or retry is provided. Other commands
use the shared adapter's canonical command validation.

The shared dispatcher also admits `retire` for the resident's negotiated upgrade
preflight. Unlike `shutdown`, it refuses replacement while execution or pending
application/MCP work or consent is retained. Only residents advertise `retire.v1`; callers must
negotiate it before use. See [safe runtime replacement](resident-runtime.md#safe-runtime-replacement).

## Output and ordering

The additive `workspaces.v1` capability enables a saved catalog and selected
execution context. Existing conversation/terminal methods apply to the selected
workspace; `shutdown` stops all contexts. The initial directory remains the primary
workspace, preserving old root metadata and transcripts. See the
[workspace methods and compatibility](wsl-workspaces.md).

Replies carry `{ v: 1, id, result }` or `{ v: 1, id, error }`; schema errors use
`id: null`. Errors are development diagnostics, not the final bridge's typed
error contract. Requests are processed serially; negotiated application operations
admit native effects to bounded workers and settle catalog changes on this same
owner loop (see [deferred application effects](windows-application.md#deferred-application-effects)).
Explicit shutdown refuses running workers without stopping sessions.
Provider events may precede
the corresponding reply and may interleave with unrelated replies.

Live frames carry `{ v: 1, generation, seq, event }`, with an unchanged V1 event.
A UUID generation identifies one agent-process lifetime. Sequence numbers are
scoped to it and restart on the next start. Snapshot returns
`{ generation, snapshot: { text, seq }, providerSession }`; stopped snapshots
have no generation. Clients must replace snapshots at generation changes rather
than compare sequence numbers across processes. A `start` result acknowledges
process creation, not provider readiness or successful resume. The existing
adapter reports failed resume and may create a new thread as on desktop.

`{ v: 1, lifecycle: "ready", provider: "codex" }` means the host owns its root.
`{ v: 1, generation, lifecycle: "exited", code, error }` follows provider stdout
consumption and reaping. Stderr draining is independent, matching the shared
workers. Storage failures are reported with the generation and latched to reject
further commands. Private telemetry fields remain excluded from public history.

## Storage and ownership

The primary root contains `runtime.lock`, `runtime.json` and `transcript.jsonl`,
plus the additive `workspaces.json` catalog and UUID child contexts described in
the [workspace storage contract](wsl-workspaces.md#storage-and-compatibility).
`runtime.json` has `v: 1`, the canonical `workdir`, `provider: "codex"` and nullable
`provider_session`. Existing roots with a different version/provider/workdir are
rejected. Unknown desktop directories are rejected before creating a lease.
Metadata is privately written, synced, atomically renamed and directory-synced.
Provider identity is persisted before its public event. The canonical transcript
uses the shared retention rules, appends privately and syncs each retained line.
Directories use 0700 and files use 0600. Provider-owned rollouts remain in the
native Codex home; this runtime never rewrites them.

An exclusive advisory lock prevents a second runtime owner of the same root.
It does not coordinate with old desktop writers, which is why adoption is
rejected. The lock is released by closing the host; interrupted initialization
may leave a temporary metadata file that requires inspection before reuse.

In default stdio mode, `stop`, clean stdin EOF and `shutdown` close agent input, then apply the shared
process-group termination/escalation policy. Stop waits for the output worker to
observe exit, with a five-second post-escalation bound. Failure to observe exit
is an error and retains the entry. The disposable host does not keep conversations alive
across a disconnected client. [Resident mode](resident-runtime.md) adds retained
ownership, snapshot recovery and bounded client delivery. Recovery of an agent
orphaned by an uncatchable host kill remains outside both modes. Native pipe drains
retain their existing unbounded queues. No automatic request retry is added.

## Evidence

`npm run test:runtime` runs the shared adapter fixtures and executable tests for
launch/message/stop, persisted replay, restart/resume, root exclusion and private
storage. A synthetic app-server needs Python 3, not a provider subscription.
The shared protocols run independently on Linux/Windows CI; the executable is
tested on Linux/macOS. Real-provider validation additionally ran two Codex turns
across separate runtime processes and checked that the resumed conversation
recalled the first turn's marker. This validates Linux execution, not Windows UI
or WSL launch/transport behavior.

## Native preview consumer

The [isolated WSL shell](wsl-preview.md) now consumes this development envelope
without changing v1 or metadata. It now selects a [resident attachment](resident-runtime.md)
that preserves the live process across disconnects. This development transport
does not establish complete Windows support.

## Supporting terminal

[ADR 0081](../decisions/0081-wsl-supporting-terminal.md) adds one supporting shell
beside the conversation. The v1 `ready` frame now includes
`"capabilities":["terminal.v1"]`. Missing capabilities mean chat only. Unknown
capabilities are ignored. Existing v1 conversation requests and metadata are
unchanged; old clients may ignore the new handshake property.

`--shell EXECUTABLE` optionally selects the shell. The composition otherwise
uses `$SHELL`, falling back to `/bin/sh`. The injected `LoginShell` passes `-l`,
sets `TERM=xterm-256color` and uses the canonical runtime project directory.
The shell and its startup files run under the selected distribution's user.
No Windows paths or command strings are interpreted by the desktop.

Each request retains `{v:1,id,action}` and ordinary correlated replies:

| Action method | Fields | Result |
| --- | --- | --- |
| `terminal_open` | `cols`, `rows` | `{id}` with a fresh terminal identity |
| `terminal_write` | `id`, `data` (byte array) | `{accepted:true}` |
| `terminal_resize` | `id`, `cols`, `rows` | `{resized:true}` |
| `terminal_snapshot` | `id` | `{id,data,seq,running,code}` |
| `terminal_acknowledge` | `id`, `seq` | `{acknowledged:true}` |
| `terminal_close` | `id` | `{closed:true}` after observed cleanup |

Rows/columns must be 1–500. Writes contain at most 4 KiB. IDs are required for
all operations after open. A missing, closed or replaced ID fails without touching
the current shell. A naturally exited handle remains readable until close, and
another open is rejected until that handle is retired.

Asynchronous frames have an independent terminal envelope:

```json
{"v":1,"terminal":{"kind":"output","id":"opaque-id","seq":1,"data":[195,169,13,10]}}
{"v":1,"terminal":{"kind":"closed","id":"opaque-id","code":0}}
```

Data is raw bytes, including split UTF-8, invalid UTF-8 and escape/control codes.
Each output chunk advances the sequence; the final empty chunk records exit
status before the closure event. `code` can be null when waiting fails.
Snapshots retain the latest 512 KiB and the covered sequence. They replace the
view; apply only later live chunks. Bytes are in memory only and never enter
conversation transcripts or telemetry.

At most 64 chunks of up to 8 KiB may be published without acknowledgement.
`terminal_acknowledge` accepts a cumulative sequence no greater than the last
published sequence; duplicate/lower acknowledgements are harmless. The reader
waits for credits outside the scrollback lock. Snapshot consumption should
acknowledge its sequence, too. The browser acknowledges only after xterm parses
the supplied bytes. It buffers at most 512 KiB while initial snapshot attachment
is pending. A gap or overflow disables terminal input and requires close/reopen.
The renderer's retained screen is separately bounded to 4,000 scrollback lines.

Input is serialized in a UI queue capped at 64 KiB, split into 4 KiB requests.
Oversized pastes are rejected as a whole. A failed write stops further queued
input and is never replayed automatically. Closing or disconnecting invalidates
queued input for the old ID. The host admits input to a separate 16-frame writer
queue (at most 64 KiB plus one in-flight frame); `accepted` means queued, not consumed by the shell. Queue
saturation rejects the complete request. A native write failure latches an error
and emits `{v:1,terminal:{kind:"error",id,error}}`; later writes fail. Native
writes occur on the injected worker, leaving the request loop available for
output credits and close even while the PTY cannot accept input. Close discards
queued input and waits for both the reader/reaper and writer. The runtime injects
`ResponsiveTerminalFactory`, which uses nonblocking PTY descriptors and cancellation-aware read/write adapters. Close cancels a blocked
writer before waiting for native process termination. The production desktop
continues to inject the original Unix terminal factory.

`stop` still stops only the agent. `terminal_close` requests native hangup and
escalation, then waits up to five seconds for drain/reap completion. Timeout
retains the handle and returns an error. Close releases a reader waiting on
credits; no acknowledgement is needed to clean up. `shutdown`, stdin EOF and
host teardown close both execution resources. The bridge's ten-second transport
cleanup budget remains in effect. Detached sessions are outside the existing
native process-group guarantee. In default stdio mode, neither shell nor scrollback survives host replacement.
Resident mode retains them across client detach and adds `terminal_current` discovery;
its ownership, detached credits and snapshot fields are defined in the
[resident contract](resident-runtime.md).
