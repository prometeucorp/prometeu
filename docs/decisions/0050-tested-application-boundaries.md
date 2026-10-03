# ADR 0050 — Test application boundaries before extracting services

Date: 2026-09-15
Status: Accepted

## Context

The desktop has useful protocol boundaries, but runtime import cycles and
application functions tied to global Tauri state make smaller pieces hard to
reuse. Direct-import checks miss dependencies hidden behind another module.
Splitting deployments now would carry those dependencies into the new services.

## Options considered

1. Split the desktop into crates or services before changing its dependencies.
2. Add a general service framework, event bus and repository interfaces.
3. Remove concrete cycles, enforce existing dependency rules and extract one
   application use case with explicit state and effects.

## Decision

Use the third option and keep the current deployment model.

Frontend composition supplies dependencies that otherwise point back into a
feature's caller. Settings coordinates refreshes after a skill changes; the
skills module does not import plugins. The main module supplies the issues
feature with a callback for Linear connection state; issues does not import the
settings view. Existing asynchronous ordering and error handling stay intact.

The architecture check builds a local TypeScript dependency graph using the
already installed TypeScript parser. It checks runtime cycles and follows
runtime imports through intermediate modules when enforcing portable
boundaries. Type-only edges do not create runtime cycles. Focused fixtures
exercise both forbidden and allowed dependencies. The existing provider and
canonical-protocol checks remain in place.

`crates/core/src/workspace_tools.rs` owns validation and updates for a workspace's MCP, plugin
or skill selection. Validation accepts JSON null for inheritance or a valid
`Selection` object with identifiers belonging to the requested axis. The
update receives the board mutex, workspace id, axis and validated selection.
It changes only that axis, preserving sibling tabs and unrelated workspaces.
It does not receive `AppHandle` or `AppState`, access processes, translate errors
or publish events. The Tauri commands adapt those responsibilities at the edge
and preserve the distinction between an absent argument and explicit null.

The same boundary applies to finish/archive/restore in `workspace_lifecycle.rs`:
it receives a mutable board and returns stop IDs and the archive transition
flag. It does not execute scripts or stop processes. `session.rs` preserves the
existing sequence: finish chooses the final stage, archive scripts run before
the archive mutation, processes stop outside the board lock, then publication
and transition telemetry run. Repeated archive still requests shutdown without
emitting another transition fact; restore leaves sessions stopped.

The application rule follows [ADR 0045](0045-layered-tool-selection.md) and
[ADR 0047](0047-tool-selection-boundaries.md): saving a selection is allowed
during a turn and never retires an existing process, even an idle one. A change
applies only at the next spawn or resume of a stopped process. Keeping process
access outside this use case prevents selection changes from interrupting
work. Commands publish after validation and the board update succeed.

## Consequences

Contributors can test this use case with an in-memory board, without a Tauri
application, saver thread or installed agent CLI.
Other use cases can adopt the same approach when a change needs that boundary;
there is no requirement to wrap every function in an interface.

[ADR 0085](0085-portable-board-core.md) moves board models and this use case
into an independently tested library with injected storage and publication. The
production desktop deployment remains local and in-process.
[ADR 0079](0079-headless-conversation-slice.md) permits an isolated experimental
headless conversation executable to validate the execution boundary. Process ownership, storage ownership
and transport still need explicit contracts before a later service extraction.

The import graph and selected global-access checks are architectural fitness
checks, not a complete proof of purity. Their exact scope and limits are in the
[dependency rules](../architecture/dependency-rules.md). They use no additional
dependency and run in the existing validation command.

This extraction preserves the layered IPC names, payloads, error codes and
persisted formats. Rolling back the extraction requires no data migration;
the earlier format migration remains governed by
[the persistence contract](../contracts/persistence.md#workspace-migration).

## Evidence

- [Frontend composition](../../src/main.ts) and
  [settings coordination](../../src/settings.ts).
- [Workspace tool use case and validation tests](../../src-tauri/crates/core/src/workspace_tools.rs).
- [Workspace lifecycle rules and preservation tests](../../src-tauri/crates/core/src/workspace_lifecycle.rs).
- [Architecture check](../../scripts/check-architecture.mjs).
- [Agent runtime contract](../contracts/agent-runtime.md).
