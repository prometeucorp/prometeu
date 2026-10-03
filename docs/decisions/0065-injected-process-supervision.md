# ADR 0065 — Injected process supervision and a reusable Unix adapter

Date: 2026-09-27
Status: Accepted

## Context

The portable conversation stream still depended on `chat.rs` to spawn native
children, drain pipes, signal groups and reap them. A WSL execution host would
have had to duplicate these effects or import Tauri. Shutdown also treated
stdout EOF as process exit before waiting, disabling escalation when a live
child closed stdout. Checking an atomic running flag before signaling did not
serialize that signal with reaping and PID reuse.

## Decision

Define application-owned `ProcessLauncher<Request>`, `ProcessControl` and
`ProcessWait` ports in `prometeu-core`. `StartedProcess` transfers input, two
independently drained output streams, a control handle and the owning waiter.
Keep request construction at the provider edge: the Unix implementation takes
the existing prepared `Command`, preserving arguments, environment removals,
working directory and native setup. The generic request parameter is a local
adapter type, not a generic method dispatcher or a new bridge wire format.

The core owns the existing shutdown policy: close input in the host, wait two
seconds, request termination, wait 500 ms, then request a forced stop. The host
runs it away from the interaction thread. Stable in-process `ProcessIdentity`
tokens distinguish replacements independently of PIDs and running state.

Implement native effects in the separate `prometeu-process` crate without Tauri.
The Unix launcher starts a dedicated process group and drains stdout and stderr
before returning the input writer. The control adapter serializes signals with
nonblocking reap attempts under one mutex. Only observed exit clears running
state; EOF alone does not. A discarded waiter kills and reaps its owned child,
covering a failed host setup or a panicking output worker.

`main.rs` injects the launcher into the desktop. `chat.rs` uses its control port
for Antigravity interruption and the common shutdown policy for all providers.
Provider links consume an injected byte writer. Native command construction,
provider-specific environment cleanup, protocol translation and desktop exit
reactions remain at their existing edges.

## Alternatives and consequences

Extracting functions while keeping native handles in the conversation host
would leave lifecycle ownership implicit. Replacing every provider command with
a new serialized spawn format would introduce unrelated configuration changes
before a bridge contract exists. Typed local launch requests preserve the
current providers while making process ownership and effects substitutable.

Tests use deterministic controls for policy and real Unix subprocesses for
input closure, signals, descendants, pipe backpressure, exit retention and
abandoned-handle cleanup. The crate builds/tests independently on Linux and
macOS CI; the core retains its independent Windows build. PTYs and private
authentication now use the separate ports in
[ADR 0067](0067-injected-terminal-and-private-processes.md). Bounded catalog and
finite-command helpers use [ADR 0068](0068-bounded-command-and-query-ports.md);
other subprocess consumers keep their existing lifecycle.

Pipe queues remain unbounded during publication stalls. Signals target the
owned group only while its unreaped leader anchors the PID; detached processes
and descendants that outlive an already reaped leader are outside this
supervisor's guarantee. The application still runs in-process under ADR 0050.
Session orchestration now uses the ports in
[ADR 0066](0066-injected-session-coordination.md); headless runtime ownership
and WSL transport remain pending. No provider capability, IPC or persisted
format changes.

## Evidence

- [Core process ports and policy tests](../../src-tauri/crates/core/src/process.rs).
- [Unix adapter](../../src-tauri/crates/process/src/lib.rs).
- [Real subprocess tests](../../src-tauri/crates/process/src/tests.rs).
- [Process contract](../contracts/application-core.md#process-supervision).
- [Desktop integration](../../src-tauri/src/chat.rs).
