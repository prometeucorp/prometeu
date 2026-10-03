# Resident preview runtime

Status: implemented experimental transport under
[ADR 0082](../decisions/0082-resident-wsl-attachments.md).
This extends the [headless v1 contract](headless-runtime.md); production desktop
IPC and persisted metadata remain unchanged.

## Composition and lifecycle

`prometeu-runtime --transport resident` is a disposable stdin/stdout proxy.
It connects to `<root>/resident.sock`, starting `--transport serve` when needed.
The host starts in its own native session, with detached stdin/stdout. Startup
waits at most five seconds for the endpoint. Concurrent startups rely on the
existing exclusive root lease: only its owner can bind or remove a stale socket.
The Unix path must fit the platform socket limit; use a short preview root.
The default `--transport stdio` retains EOF-as-shutdown behavior for existing
clients and tests. Transport selection happens only in composition.

The resident host owns the root lease and each opened workspace's provider process
and supporting shell. The [workspace extension](wsl-workspaces.md) retains these
contexts independently and persists the selected workspace.
Proxy EOF, proxy death, desktop closure, malformed input or delivery failure only
detach the client. They do not stop execution or release the root. A second
attached client receives a rejection; an attachment racing old-client cleanup
may need an explicit retry. There is no forced takeover, idle expiry or service
installation. Bundled executable paths are immutable and content-addressed;
attachment may retire an older idle host as described below.

`stop` terminates the conversation process; `terminal_close` terminates the shell.
These operations apply to the selected workspace. `shutdown` stops every context,
delivers its reply, removes the endpoint and releases the
lease. A failed shutdown retains the host for recovery. The native preview has
separate Disconnect and Shut down runtime controls. The bridge waits up to ten
seconds for its proxy to exit; terminating that proxy does not terminate the host.

## Local attachment boundary

The root is private (0700) and owned by the current Unix user. Its socket is 0600.
The proxy verifies root ownership and permissions before sending anything. Local
processes of the same user remain trusted; no TCP listener or network service is
introduced. Existing foreign/desktop roots are still rejected by the store.

Before ordinary requests, the proxy sends one JSON line containing `v:1`,
canonical `workdir`, `codex`, `model` and resolved `shell`. These must match the
resident configuration exactly. The host rejects mismatches without executing a
command. The internal attachment line has a two-second timeout for each socket read and the
same 1 MiB bound as requests. Rejected connections never receive a ready frame.
The provider's inherited environment is fixed at host creation; matching paths
do not reload credentials, environment, executable contents or configuration.
`--catalog` selects initialization only when spawning a resident; it is not part
of the handshake, so existing residents remain attachable with unchanged settings.

An accepted handshake adds `resident.v1` alongside `terminal.v1` and `workspaces.v1`. The ready frame
is queued before live events. Subsequent request IDs, v1 envelopes and correlated
replies remain scoped to the attachment. A fresh attachment starts fresh request
numbering. Frames from a retired connection cannot execute in its replacement.
There are no replayed mutations, deduplication receipts or automatic command retries.

## Snapshot recovery

The shared Windows application reconnects failed attachments automatically through
`application_reconnect`. It retains the same target and never repeats an application
mutation, message or terminal write. Explicit disconnect/shutdown suppress recovery.
The injected presentation recovery source restores board, local ChatViews and docks
in the existing document. Drafts remain owned by their views; remote Cloud sessions
do not participate. Only attachment and read-only restoration retry, with delays
from 250 ms to five seconds. Native request timeout remains thirty seconds.

For application docks, the private `pty_buffer` argument `snapshot: true` requests
`{id,data,seq,running,code}` or `null` when no dock remains. Omitting it retains the
existing byte-array response. Older application residents ignore the optional
argument and return their byte array, which the Windows adapter still accepts;
they cannot provide sequence-based reconciliation or offline exit observation;
overlapping legacy snapshot/live bytes can still appear twice.
New snapshots and held/live output use sequence cutoffs, including split UTF-8.
No snapshot opens or restarts a terminal. A stopped or missing dock disables input.

## Safe runtime replacement

Resident readiness advertises `retire.v1` and its `executable` path. On attachment,
the bridge compares that identity with the installed target. When they differ it
may send `{method:"retire"}` through the ordinary request envelope. The host checks
and admits retirement in its serialized request loop: no retained conversation,
live supporting terminal (in any context or application dock), deferred launch,
unclaimed application/MCP job or unexpired browser consent may remain. An idle agent process
or shell still counts as retained execution. Inspection errors refuse retirement.

`{retired:false}` keeps the original host and attachment. `{retired:true}` drains
the reply, closes the listener and releases the root; the bridge attaches using the
new executable. Replacement attachment may retry for up to five seconds between
attempts (each handshake keeps its ordinary timeout). No application operation
is sent during this preflight. Its discarded attachment events cannot reach the UI.
The decision is revisited on the next attachment, not periodically during work.
No idle timer, process kill or forced upgrade is introduced.

Old residents without `retire.v1` remain attachable and require explicit shutdown
before replacement. A changed launch configuration still rejects before retirement.
Persisted board/transcripts survive replacement; stopped PTY scrollback and other
memory-only state do not. There is no persisted-format migration in this change.

## Conversation and terminal snapshots

Conversation `snapshot` adds optional `running`, `ready` and `error` fields.
`running` means a process handle remains admitted, including an exited handle
that needs Stop cleanup. `ready` requires a live process, its observed identity
and no latched runtime fault. Busy state remains in the canonical snapshot.
These additive fields let a client reattach without Start or a new identity event.
Clients of older stdio hosts can still use identity-event readiness.

`terminal_current` returns null or the existing `{id,data,seq,running,code}`
snapshot. This discovers a terminal without creating a new one or repeating input.
The preview replaces conversation and terminal views, buffering events while
snapshots are pending and discarding covered sequence numbers. An exited snapshot
cannot be made ready by an older buffered identity. Conversation bootstrap buffering
is capped at 8 MiB of UTF-16 text; overflow requires reconnect/resnapshot.
Terminal bootstrap buffering retains its existing 512 KiB cap.

Detached terminal output continues into its 512 KiB scrollback without waiting
for renderer credits. At attachment, outstanding credits reset to the last sent
sequence; snapshots cover skipped output. Thereafter the usual 64-chunk window
and renderer acknowledgements apply. Admitted input may still execute after a
disconnect. Input not yet admitted by the client is discarded and never replayed.
Raw retained bytes are not a serialized terminal screen; truncation can start
inside a UTF-8 character or escape sequence. A snapshot reconstructs that retained
tail, not an unlimited history or exact alternate-screen state.

## Bounds and failure behavior

The host admits at most two queued input frames, each limited to 1 MiB. Each
attachment's outbound queue has at most 64 frames and 8 MiB of serialized bytes,
plus one in-flight frame of at most 8 MiB. Socket writes time out after two seconds.
Overflow, oversized output or write failure close the attachment. Transcript and
terminal retention continue through the same execution services. Slow clients
must reconnect and obtain snapshots; output is not accumulated indefinitely for
them. Underlying provider pipe-drain queues retain their previous behavior.

The proxy forwards in explicit 8 KiB reads/writes with flushes. Kernel splice
between sockets and pipes stalled handshake delivery in real WSL testing.
Invalid/truncated/oversized requests disconnect only the offending attachment.
Ordinary schema/version errors receive the existing v1 diagnostic replies.
A missing mutation reply remains an unknown outcome: inspect history before
resubmitting. Development errors remain strings, not the final typed error catalog.

Runtime death, WSL termination and reboot are not attachment failures. They lose
live process and shell state; persisted conversation history can be resumed in a
new host. Orphan recovery after uncatchable host death remains unimplemented.
This is not full Windows support or an installer/update contract.

## Verification

`crates/runtime/tests/resident.rs` runs the real daemon/proxy lifecycle with a
synthetic provider. The optional WSL test invokes the same lifecycle through
Windows `wsl.exe`, including a turn completed offline and preserved shell variables:

```sh
PROMETEU_TEST_WSL_DISTRIBUTION=Ubuntu-24.04 \
  cargo test --manifest-path src-tauri/Cargo.toml -p prometeu-runtime \
  --test resident actual_wsl_resident_reconnect -- --ignored
```

It requires no provider subscription. Passing it proves the transport and Unix
process lifetime, not a Windows-built native GUI.
