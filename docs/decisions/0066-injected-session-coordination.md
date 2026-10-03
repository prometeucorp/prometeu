# ADR 0066 — Session coordination through injected effects

Date: 2026-09-27
Status: Accepted

## Context

Conversation ordering and native process supervision have portable boundaries,
but input recovery, account handoffs and canonical event reactions still lived
inside the Tauri chat module. Reusing those policies in a WSL execution host
would require duplicating them or depending on the desktop.

## Decision

Move input admission, pending-prompt recovery and board reactions to
`prometeu-core::session`. `SessionService` receives the board plus separate
runtime, account, publication and diagnostic ports. `SessionReactions` receives
that service, the background-work map and context, action and usage ports.
`InputGates` belongs to each execution host; a global registry must not couple
independent roots. Desktop and MCP callers share the host's per-session gate.

`SessionOutput` coordinates transcript delivery with injected execution
observation and telemetry effects. Execution observation is memory-only under
the transcript lock. Telemetry receives the original private frame after that
lock is released, while the capture-order gate is held. Publication and session
reactions run after releasing the gate. `capture_command` keeps successful input
and output capture ordered, committing telemetry only after process and
transcript locks are released. Failed writes neither record nor capture input.

The desktop composes these ports in `chat/host.rs`. Native account lookup,
provider usage translation, action completion, delegation projection and SQLite
capture remain adapters. Core rules contain no platform or provider selection.
Workspace creation, launch configuration and process revival still live in the
native application; this change does not create a standalone runtime.

## Compatibility and consequences

Pending input retains its order across setup, account changes and failed sends;
publication precedes revival but is not a durable save acknowledgment. Tab
status stays separate from workspace stage. IPC, V1 events and persisted fields
are unchanged; existing compatibility fixtures continue to exercise core types.

Canonical assistant activity now reaches `Work::observe` in the actual session
reaction path. This fixes a missing integration: resumed activity invalidates a
held completion, so a later child drain cannot mark the active tab ready or
release queued input. It enforces the existing ADR 0056 rule.

Separate narrow ports avoid a universal host interface and let tests substitute
only the effects they need. The desktop adapter still coordinates real services;
it does not prove a headless host, bridge recovery or Windows desktop support.
The application remains in-process under ADR 0050.

## Evidence

- [Session policies and tests](../../src-tauri/crates/core/src/session/tests.rs).
- [Output ordering and capture tests](../../src-tauri/crates/core/src/session/output.rs).
- [Desktop composition](../../src-tauri/src/chat/host.rs).
- [Session contract](../contracts/application-core.md#session-coordination).
