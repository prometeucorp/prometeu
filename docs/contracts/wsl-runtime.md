# WSL runtime protocol

Status: implemented; experimental. Decisions:
[ADR 0084](../decisions/0084-shared-windows-desktop.md) and
[ADR 0082](../decisions/0082-resident-wsl-attachments.md).

`prometeu-runtime` is the Unix execution host used by the native Windows
application. It builds without Tauri or GUI libraries and runs on Linux, WSL or
macOS. This contract covers its process interface, framing, capabilities,
storage and compatibility. The desktop commands carried by the `application`
method are specified in the [Windows application contract](windows-application.md).
Build and run steps are in [development](../operations/development.md#native-windowswsl-integration).

```text
prometeu-runtime --root DIR --workdir DIR [--codex EXE] [--model MODEL]
  [--shell EXE] [--transport stdio|resident|serve] [--catalog workspace|application]
```

- `--transport stdio` (default) serves one client on stdin/stdout; EOF shuts the
  host down. `resident` is a disposable proxy to a detached `serve` host (see
  [resident attachment](#resident-attachment)).
- `--catalog application` seeds an empty board for the shared desktop;
  `workspace` seeds the initial directory as the primary workspace. The seed
  applies only when no catalog exists and is not part of the attachment handshake.
- `--shell` defaults to `$SHELL`, then `/bin/sh`. Shells run as login shells with
  `TERM=xterm-256color` under the distribution's user.
- Only Codex is registered. It uses the native authentication and configuration
  of the execution environment; approvals default to Ask for diagnostic sessions
  and to the launcher's stored policy for application sessions.

## Framing

Requests and frames are NDJSON, one JSON object per line.

- Request: `{ "v": 1, "id": <number>, "action": { "method": ..., ... } }`.
- Reply: `{ v: 1, id, result }` or `{ v: 1, id, error }`. Schema errors use
  `id: null`. Errors are diagnostic strings; structured application failures
  travel as `application-error:<JSON>` and only the Windows transport decodes them.
- The host reads at most 1 MiB per line. An oversized or invalid UTF-8 line ends
  a stdio host (stopping its children) or disconnects a resident attachment.
  Invalid JSON, schema or version is answered with an error and starts no work.
- Requests run serially on the owner loop. Events may precede the corresponding
  reply and interleave with unrelated replies.
- The bridge sends payloads of at most 1 MiB, accepts lines of at most 8 MiB,
  waits at most 30 s for startup and for each reply, and keeps the last 4 KiB of
  bootstrap stderr. After a failed handshake it stops the child and waits at most
  one more second for that tail. A wrong version, malformed or oversized frame,
  unexpected reply, EOF or timeout invalidates the connection.
- A missing reply has an unknown outcome. No mutation, message or keystroke is
  retried and there are no deduplication receipts.

## Handshake and capabilities

The host announces ownership of its root with
`{ v: 1, lifecycle: "ready", provider: "codex", capabilities: [...] }`.
Capabilities are additive: clients ignore unknown values and treat a missing
value as an unavailable feature, rejecting before sending the request.

| Capability | Adds |
| --- | --- |
| `terminal.v1` | supporting shells for the selected diagnostic context |
| `workspaces.v1` | saved catalog and `workspace_*` methods |
| `worktrees.v1` | `workspace_worktree` (new-branch checkouts) |
| `resident.v1` | resident attachment, snapshot readiness fields, `terminal_current` |
| `retire.v1` | safe replacement at attachment; the ready frame adds `executable` |
| `application.v1` | `application` method carrying desktop IPC commands |
| `application.operations.v1` | deferred Git, reference and checkout effects |
| `application.initialization.v1` | deferred discovery, account refresh and manual Setup |

## Session methods

Diagnostic clients and tests use these methods on the selected context.
Application commands address sessions by ID instead and never change selection.

| Method | Fields | Result |
| --- | --- | --- |
| `start` | none | process created; uses the stored provider thread when present |
| `command` | `frame` (canonical V1 command) | accepted |
| `snapshot` | none | `{ generation, snapshot: { text, seq }, providerSession, running, ready, error }` |
| `stop` | none | agent stopped after observed exit |
| `shutdown` | none | every context stopped, root released |
| `retire` | none | `{ retired }` (see [safe replacement](#safe-replacement)) |
| `application` | `command`, `args` | the desktop command's result |

`start` acknowledges process creation, not readiness or a successful resume; a
second `start` while an entry exists is rejected. Wait for `session.identity`
before the first message. A busy conversation rejects another `message.send`;
there is no implicit queue. `stop` closes input and applies the shared shutdown
policy, waiting at most 5 s after escalation; failure to observe exit is an error
and keeps the entry.

Live frames are `{ v: 1, generation, seq, event }` with an unchanged V1 event. A
UUID generation identifies one agent process; sequences restart with it. Clients
replace snapshots at generation changes and apply only newer sequences.
`{ v: 1, generation, lifecycle: "exited", code, error }` follows stdout
consumption and reaping. Storage failures are reported with the generation and
latch the context to reject further commands. Snapshot `running` means a process
handle is admitted (including an exited one awaiting Stop); `ready` requires a
live process, observed identity and no latched fault. Application events use
`{ application: { name, payload } }` with the desktop `board`, `chat`,
`chat-closed`, `pty`, `pty-closed` and `accounts` payloads.

## Workspace methods

| Method | Arguments | Result |
| --- | --- | --- |
| `workspace_list` | none | catalog |
| `workspace_create` | `title`, `path` | catalog; selection unchanged |
| `workspace_worktree` | `request: { title, path, branch, base }` | catalog; selection unchanged |
| `workspace_select` | `id` | catalog after the context is prepared |
| `workspace_stage` | `id`, `stage` | catalog; execution unchanged |

The catalog is `{ v: 1, active, board }` using the portable board models, with at
most 64 workspaces and 32 tabs per workspace. Stored IDs are `primary` or UUIDs;
the host derives paths from them, so unknown IDs never select a filesystem path.
Folders are canonicalized by the runtime; nothing is copied. Domain errors use
`workspace_*` codes.

Worktree creation resolves the repository root and a locally available base
commit, then creates a new branch under `<root>/checkouts/<uuid>`. It never
fetches, switches the source checkout or copies uncommitted changes; Git refuses
existing branches and destinations. Git runs through the bounded command runner
(20 s, 256 KiB per stream, argument arrays, closed input). Git and catalog writes
are separate commits: on failure, timeout or an uncertain save the error reports
the checkout path, which is kept for inspection or manual registration. Nothing
is rolled back or retried, and no operation deletes a checkout except the
explicit cleanup command.

## Supporting terminals

| Method | Fields | Result |
| --- | --- | --- |
| `terminal_open` | `cols`, `rows` | `{ id }`, a fresh opaque identity |
| `terminal_write` | `id`, `data` (bytes) | `{ accepted: true }` once queued |
| `terminal_resize` | `id`, `cols`, `rows` | `{ resized: true }` |
| `terminal_snapshot` | `id` | `{ id, data, seq, running, code }` |
| `terminal_current` | none | that snapshot or `null` |
| `terminal_acknowledge` | `id`, `seq` | `{ acknowledged: true }` |
| `terminal_close` | `id` | `{ closed: true }` after observed cleanup |

Frames are `{ v: 1, terminal: { kind: "output" | "closed" | "error", id, ... } }`
with raw bytes, including split or invalid UTF-8. Rows and columns are 1–500;
writes carry at most 4 KiB. A missing, closed or replaced ID fails without
touching the current shell. An exited handle stays readable until closed.

- Flow control: at most 64 unacknowledged chunks of at most 8 KiB.
  Acknowledgements are cumulative; lower or duplicate values are harmless. The
  reader waits for credits outside the scrollback lock, so snapshot, acknowledge
  and close stay available. Renderers acknowledge only after parsing.
- Scrollback keeps the latest 512 KiB in memory, never in transcripts or
  telemetry. It can start inside a UTF-8 character or escape sequence.
- Input enters a 16-frame writer queue (at most 64 KiB plus one in-flight frame);
  saturation rejects the whole request. A native write failure latches an error
  frame and later writes fail. Close discards queued input, cancels a blocked
  writer and waits at most 5 s for drain and reaping; a timeout keeps the handle.
- `stop` affects only the agent and `terminal_close` only the shell.

## Resident attachment

`--transport resident` connects to `<root>/resident.sock`, starting a detached
`serve` host when needed and waiting at most 5 s for its endpoint. Only the root
lease owner binds or removes the socket. The root must be 0700 and owned by the
current user, and the socket 0600; the proxy checks this before sending. Same-user
local processes are trusted; there is no TCP listener.

Before ordinary requests the proxy sends one line with `v: 1`, the canonical
`workdir`, `codex`, `model` and resolved `shell`. They must match the resident
exactly or the connection is rejected before any command. The line has a 2 s
read timeout and the 1 MiB bound. Rejected connections receive no ready frame.
Request IDs and replies are scoped to one attachment. A second attached client
is rejected; an attachment racing old-client cleanup may need a retry.

Bounds: at most two queued input frames of 1 MiB; per attachment, at most 64
outbound frames and 8 MiB plus one in-flight frame of 8 MiB; socket writes time
out after 2 s. Overflow or write failure detaches that client; execution and
persistence continue. Detached terminals keep filling scrollback without credits;
at reattachment, credits reset to the last published sequence. The proxy
forwards in explicit 8 KiB reads and writes. The bridge waits up to 10 s for its
proxy to exit; killing the proxy does not stop the host.

Clients buffer events while snapshots are pending (8 MiB of conversation text,
512 KiB per terminal), replace their views and apply only newer sequences. The
application's `pty_buffer` accepts an optional `snapshot: true` argument returning
the terminal snapshot shape or `null`; without it the existing byte array is
returned, which older residents always do.

### Safe replacement

When the installed executable differs from the ready frame's `executable`, the
bridge may send `retire` on attachment. The host admits it in its serialized loop
only when no conversation, live shell (in any context or dock), pending launch,
unclaimed application/MCP job or unexpired browser consent remains; an idle
process still counts, and inspection errors refuse. `{ retired: false }` keeps
the host and attachment. `{ retired: true }` drains the reply, closes the
listener and releases the root; the bridge attaches through the new executable,
retrying for up to 5 s. No application request is sent during this preflight.
Residents without `retire.v1` require explicit shutdown.

## Storage and ownership

| Path under the root | Content |
| --- | --- |
| `runtime.lock` | exclusive advisory lease held for the host's lifetime |
| `runtime.json` | `{ v: 1, workdir, provider: "codex", provider_session }` |
| `transcript.jsonl` | canonical transcript of the primary conversation |
| `workspaces.json` | catalog, at most 8 MiB |
| `workspaces/<uuid>/`, `tabs/<session-uuid>/` | additional contexts with the same metadata, transcript and lease rules |
| `checkouts/<uuid>/` | worktrees created by the runtime |
| `accounts.json`, `mcp-auth.json`, local hubs | registries and credentials in the desktop formats |

Directories are 0700 and files 0600. Metadata and catalogs are written privately,
synced, atomically renamed and directory-synced; provider identity is persisted
before its public event. Unknown desktop roots and roots with a different version,
provider or workdir are rejected before a lease is created. The lease does not
coordinate with older desktop writers. Interrupted initialization may leave a
temporary file to inspect. A store whose directory is missing reopens only when
its saved version, provider and directory still match. Provider-owned rollouts
stay in the native Codex home and are never rewritten.

## Compatibility

- Capabilities and fields are additive; v1 requests, metadata and transcripts are
  unchanged. Old roots become the primary workspace without rewriting history.
- An older runtime can reopen the primary root; it ignores the catalog and child
  contexts. Older readers that require a primary entry reject empty application
  catalogs without overwriting them.
- Older residents reject unknown application commands, which the bridge reports
  without fallback for MCP operations. Clients fall back to synchronous commands
  only when a deferred capability is absent.
- Persisted board, transcripts and catalogs survive runtime replacement; jobs,
  pending consent and terminal scrollback are memory-only.
- Runtime death, WSL shutdown and reboot lose live processes and shells; history
  resumes in a new host. Recovery of orphans after an uncatchable kill is not
  implemented, and native pipe drains keep their unbounded queues.

## Verification

- `npm run test:runtime`: `crates/runtime/tests/lifecycle.rs` (launch, stop,
  restart and resume, root exclusion, private storage),
  `crates/runtime/tests/bridge.rs` (bridge framing, terminals, credits, EOF) and
  `crates/runtime/tests/resident.rs` (detach and reattach, competing clients,
  configuration mismatch, malformed input, catalogs and contexts, replacement).
- `npm run test:bridge`: client framing, path validation and old-host refusals.
- `crates/core/src/workspaces.rs` and `crates/runtime/src/worktrees.rs`: catalog
  rules, failed-save preservation and real Git worktrees.
- Ignored tests `actual_wsl_*` in `bridge.rs` and `resident.rs` run through real
  `wsl.exe` when `PROMETEU_TEST_WSL_DISTRIBUTION` is set; see
  [development](../operations/development.md#native-windowswsl-integration).
