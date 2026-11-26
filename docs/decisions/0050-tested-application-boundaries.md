---
type: Decision
title: "ADR 0050 — Test application boundaries before extracting services"
description: "Test the application boundaries before Prometeu moves code into different services."
status: stable
---

# ADR 0050 — Test application boundaries before extracting services

Date: 2026-09-15

## Context

The desktop has useful protocol boundaries. But runtime import cycles and
application functions that use the global Tauri state make it not easy to use
smaller parts again. A check that examines only the imports of a module does not
find dependencies that go through a different module. If Prometeu divides the deployments at this
time, these dependencies go into the new services.

## Options considered

1. Divide the desktop into crates or services before Prometeu changes its
   dependencies.
2. Add a general service framework, an event bus and repository interfaces.
3. Remove the cycles, and make the code obey the current dependency rules.
   Then move one application use case into a module with explicit state and
   effects.

## Decision

Use option 3 and keep the current deployment model.

The frontend composition gives some dependencies to the modules. Without this,
these dependencies point back to the caller of a feature:

- Settings starts the refresh operations after a skill changes. The skills
  module does not import plugins.
- The primary module gives the issues feature a callback for the state of the
  Linear connection. The issues feature does not import the settings view.

The current sequence of asynchronous operations and the error processing do
not change.

The architecture check builds a local graph of TypeScript dependencies. It uses
the TypeScript parser from the current dependencies. It finds runtime cycles.
When it examines portable boundaries, it follows runtime imports through the
modules between them. Imports of types only do not make runtime cycles.
Specified fixtures examine dependencies that the rules prevent and dependencies
that the rules accept. The current checks for providers and for the canonical
protocol stay.

`crates/core/src/workspace_tools.rs` controls the validation and the updates for
the MCP, plugin or skill selection of a workspace:

- The validation accepts JSON null for inheritance. It also accepts a correct
  `Selection` object with identifiers that are in the requested axis.
- The update receives the board mutex, the workspace id, the axis and the
  validated selection. It changes only that axis. Other tabs and other
  workspaces do not change.
- The update does not receive `AppHandle` or `AppState`. It does not use
  processes, change errors into messages or publish events.

The Tauri commands do these operations at the edge. They keep the difference
between a missing argument and an explicit null.

The same boundary applies to finish, archive and unarchive in
`workspace_lifecycle.rs`. This module receives a mutable board. It returns the
IDs of the processes to stop. It also returns a flag that tells if the archive
state changed. It does
not run scripts or stop processes. `session.rs` keeps the current sequence:

1. The archive scripts run before the archive change to the board.
2. The processes stop after Prometeu releases the board lock.
3. Then the publication and the telemetry for the state change run.

When the person archives a workspace again, Prometeu continues to stop its
processes. But it does not record a second state-change fact. An unarchive
operation does not start the sessions again.

The application rule follows [ADR 0045](0045-layered-tool-selection.md) and
[ADR 0047](0047-tool-selection-boundaries.md). A person can save a selection
during a turn. This operation does not stop a current process, also when the
process does no work. A change applies only when Prometeu starts or
resumes a stopped process. This use case does not use processes. Thus a
selection change cannot stop work. The commands publish only after the
validation and the board update complete correctly.

## Consequences

Contributors can test this use case with a board in memory. A Tauri
application, a saver thread and an installed agent CLI are not necessary. Other
use cases can use the same method when a change must have that boundary. It is not
necessary to put each function behind an interface.

[ADR 0085](0085-portable-core.md) moves the board models, this use case and the
other application rules into a library. The callers of the library give it its effects. That library has
its own tests. The production desktop deployment stays local and in one process.

The import graph and the specified checks for global access are architecture
checks. They do not fully prove that the code is pure. The
[dependency rules](../architecture/dependency-rules.md) give their scope and
their limits. These checks add no dependency. They run in the current
validation command.

This change keeps the IPC names of the layers, the payloads, the error codes and
the saved formats. If Prometeu removes this change, a data migration is not
necessary. The previous format migration continues to follow
[the persistence contract](../contracts/persistence.md#workspace-migration).

## Evidence

- [Frontend composition](../../src/main.ts) and
  [settings coordination](../../src/settings.ts).
- [Workspace tool use case and validation tests](../../src-tauri/crates/core/src/workspace_tools.rs).
- [Workspace lifecycle rules and tests that make sure that data stays](../../src-tauri/crates/core/src/workspace_lifecycle.rs).
- [Architecture check](../../scripts/check-architecture.mjs).
- [Agent runtime contract](../contracts/agent-runtime.md).
