# ADR 0069 — Portable session host ownership

Date: 2026-09-27
Status: Accepted

## Context

The session service already receives injected effects, but its live-session
registry, readiness, input gates and background-work ownership still belonged
to desktop state. Another execution host would have to reconstruct those rules,
including the identity check that prevents an old process exit from closing a
replacement.

## Decision

`prometeu-core::session::host::SessionHost<C>` owns that transient state and
composes `SessionService` against the caller's board and injected effects.
`HostedConversation` exposes identity, liveness, account and turn observations,
plus retirement. `SessionLifecycle` supplies setup observation, revival, native
command delivery and the feature effects after stopping. Publication and
diagnostics retain their separate ports. No platform or provider selection
enters the host.

The desktop owns one `SessionHost<Chat>` in `AppState`. Account checks, resource
accounting, MCP admission, setup completion and chat commands access that same
host. `chat/host.rs` supplies native effects rather than implementing registry
policy. A headless composition can supply different conversation and lifecycle
implementations without a GUI dependency.

Removal marks the conversation retired and clears readiness and background
work under the registry lock, then drops the native transport outside it.
Unlike the old general `chat::kill` path, every removal clears that transient
state, including removal without an account restart. Input gates remain stable
for the lifetime of the host, including across removal and revival.

Exit cleanup compares process identity under the registry lock and retains it
through closure publication. An obsolete or unknown process cannot clear state
or publish closure for its successor. A stopped conversation remains available
for replay. The publication callback must not reenter the registry. Existing
stream retirement checks remain in force; this does not add a barrier for
reactions already in flight.

## Compatibility and limits

No IPC, V1 event, account identifier or persisted field changes. The old
identity-predicate unit test is replaced by portable tests of actual cleanup,
publication exclusion, replay retention, independent hosts and native teardown
outside locks. Composed-service tests exercise queued input recovery and
account handoff using the real registry and injected lifecycle effects.

This extracts session ownership, not the complete execution host. The desktop
launch configuration, telemetry, usage, actions and delegation composition
still need native adapters independent of Tauri before a production headless
runtime can use them. Pump orchestration is now portable under
[ADR 0070](0070-portable-conversation-pump.md). Terminal ownership, root locking and
bridge transport are outside this extraction. Windows composition and packaging
now follow [ADR 0084](0084-shared-windows-desktop.md).
In-process deployment under ADR 0050 continues.

## Evidence

- [Host and ports](../../src-tauri/crates/core/src/session/host.rs).
- [Host tests](../../src-tauri/crates/core/src/session/host/tests.rs).
- [Desktop composition](../../src-tauri/src/chat/host.rs).
- [Application contract](../contracts/application-core.md#session-host-ownership).
