# ADR 0082 — Resident WSL execution with disposable attachments

Date: 2026-09-29
Status: Accepted

## Context

A runtime that lives only as long as its stdio client restores the transcript on
restart but loses the live turn and supporting shells. Closing or reloading the
Windows window must not change execution ownership, and Unix transport details
must stay outside portable clients and presentation.

## Decision

The Windows composition injects `ResidentWslLauncher`. It runs the bundled Linux
executable as a disposable stdio proxy that starts or attaches to a detached host
through a private Unix socket in the runtime root. The plain stdio launcher stays
available for disposable execution and tests. Transport selection happens only
in composition.

- The host owns the root lease, every conversation process and every supporting
  shell. Proxy EOF or death, window closure, malformed input and delivery failure
  only detach the client. Explicit shutdown stops all execution and releases the
  root; Stop and terminal close stay scoped to one conversation or shell.
- One client is attached at a time. A second client is rejected rather than
  taking over. The launch configuration must match before any request runs.
- Delivery to a client is bounded. A slow or absent client is detached or
  skipped; execution and persistence never block on it. Reattachment restores
  snapshots of conversations and terminals; presentation buffers events,
  replaces snapshots and applies only newer sequences.
- No mutation, message or keystroke is retried automatically. A lost reply has
  an unknown outcome.
- A changed bundled executable replaces a resident only at attachment and only
  when the host retains no conversation, live shell, pending launch, deferred
  application/MCP work or browser consent. A busy host keeps running and the
  replacement waits for a later attachment. Residents without that negotiation
  require explicit shutdown.

## Trade-offs and limits

A resident host consumes resources after the window closes; there is no idle
expiry, forced takeover or service installation. Deferring replacement avoids
abandoning live execution, at the cost of running an older build until a later
idle attachment. The design survives client and proxy loss, not runtime death,
WSL shutdown or reboot: persisted history can be resumed in a new host, but live
processes, shells and terminal scrollback are lost. Recovery of descendants after
an uncatchable host kill is not implemented. Socket paths have the native Unix
length limit.

## Evidence

- [WSL runtime protocol](../contracts/wsl-runtime.md#resident-attachment):
  handshake, bounds, permissions, snapshots and safe replacement.
- [Resident tests](../../src-tauri/crates/runtime/tests/resident.rs): detach
  during a turn, same generation and provider process, retained shell state and
  offline output, competing attachment, configuration mismatch, malformed input,
  proxy death, shutdown and replacement admission. The same reconnect fixture
  runs through actual `wsl.exe` on opt-in.
- [Delivery test](../../src-tauri/crates/runtime/src/resident.rs): a saturated
  consumer detaches without failing execution delivery.
- `src/windows/recovery.test.ts` and `src/term.test.ts`: snapshot
  reconciliation and stale events after reattachment.
