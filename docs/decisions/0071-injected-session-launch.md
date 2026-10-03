# ADR 0071 — Injected session launch and resume

Date: 2026-09-28
Status: Accepted

## Context

Session services and their pump were portable, but the desktop's resume function
still mixed board snapshots, tool resolution, provider dispatch, process
replacement, publication and readiness. New tabs duplicated provider dispatch.
The launch settings themselves also lived in a Tauri command module.

## Decision

Move `Launch` and its existing deserialization/default/package-union behavior
into `prometeu-core::session::launch`. `LaunchRequest` describes a fresh start or
a resume, session/workspace identity, execution-side worktree and resolved
settings. It is a local application request, not a new IPC or bridge payload.

`LaunchService` snapshots workspace, global tools, trust and delegation
permission under the board lock. `ResumePreparation` resolves native tools,
kickoff metadata and path validity after releasing that lock. A delegation's
permission overrides prepared settings, including an explicitly unset value;
absence of a delegation leaves those settings intact.

After successful preparation the service revokes old access, removes the old
conversation through `SessionHost`, and reports it stopped. The injected
`ConversationLauncher` creates the replacement and reports whether it actually
selected a resume. A missing-skill warning reaches the new conversation before
installation. The service then updates tab status/note, publishes and signals
readiness in that order. Failed preparation leaves the old process intact;
failed spawning leaves the queue and prior board state available for recovery,
without publishing a successful launch.

New tabs call the same launcher through `LaunchService::start`. That method
installs the conversation only; the existing caller publishes the new tab
before releasing its pending prompt. `LaunchEffects` supplies native access,
delegation, warning, publication and readiness effects. Those callbacks and
preparation/spawning run without board or registry locks.

`src/session/launch.rs` is the desktop composition adapter. The injected native
preparation table in `src/agent_launch.rs` owns provider dispatch (ADR 0073). Claude checks for its existing transcript on
resume and starts fresh with the same ID when it is absent. Codex and
Antigravity resume from a saved provider identity, or start fresh when absent.
Fresh starts never acquire resume behavior merely because a file exists.
Retired providers retain the existing structured error.

## Compatibility and limits

No persisted field, IPC argument, provider flag or event payload changes.
Launch settings keep null versus empty tool selections, legacy provider
identity handling and plugin-then-skill package order. Existing provider and
workspace tests now consume the shared launch type. Portable workflow tests
cover effect ordering, failed preparation/spawn, queue retention, new-tab
publication ownership and delegation permission overrides.

The snapshot now observes delegation permission in the same board read as the
workspace and tools. This does not add transaction/retry semantics across native
preparation; existing caller serialization remains necessary. Process reader
activation and installation timing are unchanged. Native provider configuration
and feature composition still use desktop adapters. Worker orchestration now
uses the injected boundary in [ADR 0072](0072-injected-conversation-workers.md). This is not
a headless runtime, and ADR 0050's in-process deployment remains in force.

## Evidence

- [Launch types and service](../../src-tauri/crates/core/src/session/launch.rs).
- [Portable tests](../../src-tauri/crates/core/src/session/launch/tests.rs).
- [Desktop preparation and effects](../../src-tauri/src/session/launch.rs).
- [Provider preparation](0073-injected-provider-preparation.md).
- [Application contract](../contracts/application-core.md#session-launch-and-resume).
