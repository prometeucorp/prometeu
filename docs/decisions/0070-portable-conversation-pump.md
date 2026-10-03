# ADR 0070 — Portable conversation pump

Date: 2026-09-28
Status: Accepted

## Context

Transcript ordering and session reactions already have portable services, but
`chat.rs::Pump` still joined them through a desktop handle. Sharing that flow
with a headless runtime required extracting the orchestration itself, including
its capture gate and the delayed release of queued input.

## Decision

`prometeu-core::session::pump::SessionPump<T>` owns the shared capture gate,
readiness latch, output coordination, clock and injected effect ports. Clones
share the same transcript, sequence, retirement state, capture and readiness.
The telemetry type remains generic; native account/profile context is not a
field of the pump.

`PumpReactions` applies canonical reactions outside capture and transcript
locks and reports settlement. `PendingInput` observes the queue and setup and
schedules sending on the host's executor. Only a settled turn with pending input
and no running setup schedules a send. Actual admission is rechecked by the
session service when that work runs. `PumpDiagnostics` reports storage errors
without turning them into failed provider commands.

The pump also owns command recording and turn-state rollback. Its `capture`
method invokes a host closure that validates process identity and idleness under
the process registry lock, writes through `command`, then releases that lock.
Accepted telemetry follows process/transcript release; execution publication and
reactions follow capture release. Failed admission or writes produce no accepted
capture or local reactions, and failed writes restore the previous turn flag.

The desktop injects `ConversationHost` for reactions, pending-input scheduling
and diagnostics. Account/profile lookup, persistence preparation and process
identity checks remain native. Its scheduler starts a thread as before; core
code neither creates threads nor selects platforms. Telemetry, delegation,
action and usage implementations remain at the edge.

## Compatibility and limits

IPC, conversation V1, event sequencing, transient-versus-persisted events and
private telemetry stripping are unchanged. Storage failures still permit live
delivery. Private usage retains capture and internal reactions without entering
public history. Retirement checks retain their existing semantics and do not
cancel reactions already in flight. Diagnostic sinks may run under command
locks and must not reenter conversation operations.

This is executable shared orchestration, not a production headless runtime.
Native feature composition, launch configuration, terminal ownership and the
runtime/desktop transport still need extraction. The deployment decision under
ADR 0050 stays in force.

## Evidence

- [Pump and ports](../../src-tauri/crates/core/src/session/pump.rs).
- [Ordering, scheduling, privacy and failure tests](../../src-tauri/crates/core/src/session/pump/tests.rs).
- [Desktop effects](../../src-tauri/src/chat/host.rs).
- [Application contract](../contracts/application-core.md#conversation-pump).
