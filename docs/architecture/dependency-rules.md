---
type: Architecture
title: "Dependency rules"
description: "The rules between the presentation, application, domain and adapter layers."
status: stable
---

# Dependency rules

This page gives the rules in force and the direction of future changes.

## Principle

Add an interface when it isolates one of our policies from a technology or a
protocol that can change. Do not add interfaces between internal functions only
to have more layers.

## Conceptual layers

| Layer | Current examples | Can know about |
| --- | --- | --- |
| presentation | `chat.ts`, `workspace.ts`, `workspace-changes.ts`, `sidebar.ts` | View models, use cases and IPC contracts |
| derived domain | `timeline.ts`, `relay/src/logic.ts` | Domain types and pure functions |
| application | `workspace_tools.rs`, `session.rs`, coordination in `main.ts` | The domain and the external ports |
| adapters | `claude.rs`, `codex.rs`, IPC, relay transport, Git/files | External protocols and core contracts |

The current directories do not agree fully with these layers. Use the table
to find the owner and the direction of a dependency when you make changes in
small steps.

## Rules in force

1. `timeline.ts` does not use the DOM, Tauri or the network.
2. `relay/src/logic.ts` does no I/O. `room.ts` does its effects.
3. `relay/src/protocol.ts` does not use APIs that only the app or the Worker
   has.
4. The backend changes the stored state. Then the `board` event publishes the
   state again.
5. The presentation stores two items: the local Git read marks and the
   [review notes](../contracts/diff-review.md). These items do not obey rule 4. Their storage adapters
   stay out of the components.
6. Only the backend accesses the filesystem, Git and processes.
7. The side that owns the authority validates remote input again.
8. Backend errors go through IPC as codes and data. The frontend translates
   them.
9. The presentation uses `AgentCapabilities` to select what to show. Only the
   catalog or the adapters compare provider names.
10. The runtime imports between local TypeScript modules have no cycles. An
    import of types only can refer back to a caller. It does not make a runtime
    dependency.

## Composition and application use cases

Connect features in the caller that controls them at this time. For example,
`settings.ts` supplies the callback that refreshes the plugins and the catalog
after `skills.ts` refreshes its data. `main.ts` gives `issues.ts` the callback
for the live Linear connection. Thus, the two features do not import their
callers or use a global event bus to get those dependencies.

`prometeu-core::workspace_tools` validates the tool selections. It changes one
axis of a workspace with explicit board state. It does not access process
handles and does not change tabs. `session.rs` reads the native IPC body. It
keeps the difference between missing arguments and null arguments. It
translates validation errors and publishes the board.

A person can save a selection during a turn. The processes that operate at that
time keep their tools until they stop. The new selection applies at the next
start or resume of a stopped process. The tests examine the validation and make
sure that the sibling tabs do not change. They operate without `AppHandle`,
`AppState`, a saver or a provider process. Refer to
[ADR 0050](../decisions/0050-tested-application-boundaries.md).

`prometeu-core::workspace_lifecycle` gets a board that it can change for
the finish, archive and restore operations. On archive, it returns the IDs of the
tabs to stop. It also tells if the archived flag changed. `session.rs` continues
to do these operations:

- It runs the archive scripts before the change.
- It stops the processes after it releases the board lock.
- It owns the publication and the telemetry.

The restore operation does not start a process at any time. The tests of these rules
operate without Tauri state or effects. The limits of the Rust crates are in
[Rust crates](#rust-crates).

## Rules in force for agents

1. The protocol of Claude, Codex or a different vendor is only in its adapter
   and in its adapter fixtures.
2. The core gets `ConversationCommand` and makes `ConversationEvent`. Prometeu
   owns the two types.
3. A new provider uses the same port. It must operate correctly with the
   conformance suite.
4. An unknown event does not stop a session. The application can see the event
   and ignores it in a compatible manner. It does this until the event has an
   explicit translation.

The legacy transcript reader does not obey the first rule. It is only in
`conversation-legacy.ts` and in the replay path of the Claude adapter. It does not go to the timeline or to the canonical protocol. Prometeu
does not write new lines in the legacy format.

## Boundaries that justify interfaces

### Agent runtime

The agent runtime changes with the installation, the catalog, the protocol, the
resume and the capabilities. It must give discovery, start and resume, commands,
events and shutdown. The payload of the vendor must not go out of the adapter.

### IPC

IPC isolates TypeScript from Rust. The name, the arguments, the return value,
the error and the events are one contract. The web mock is a different adapter
of that contract. `src/ipc.ts` owns the map of command arguments and results.
The frontend callers and `IpcHandlers` in the mock use this map.

A test makes sure that the command names agree fully with Rust. The structure
of the Rust payloads stays the same as TypeScript only through manual changes.
Serialization fixtures compare representative payloads with the TypeScript
types and reducers. Refer to the [IPC contract](../contracts/ipc.md).

### Collaboration

`team-transport.ts` hides the socket. `team-control.ts` changes frames into local
actions. The protocol and its validation stay shared.

The core is `team-member.ts` with the features `team-owner.ts`,
`team-viewer.ts` and `team-comments.ts`. The shell gives it the ports from
`team-ports.ts`: `Membership`, `SecurityStore` and `OwnerHost`. The features are
hooks on the member. The composition root selects their order. `src/team.ts` is
the desktop shell. No `team-*.ts` file imports `@tauri-apps`, `./ipc`, `./mock`
or `./team`. Refer to [ADR 0026](../decisions/0026-portable-collaboration-core.md).

### Context evaluation

The context evaluation changes with the external service and its network
format. Features send closed questions through the port in
`evaluation.rs`/`evaluation.ts` and apply their own rules. The credential, the
transport, the retries and the vendor validation stay in `typesafe.rs`. A
consumer imports only the port types. In tests, it operates with a fake port.
Refer to [ADR 0058](../decisions/0058-optional-context-evaluation.md).

### Local system

Git, files, PTY, processes and the embedded browser are external effects. You
must be able to write tests for the rules that select when to start these
effects. These tests must operate without the effects.

## Isolated Desktop presentation

`src/components/resource-view.ts` uses snapshots with types, translated labels
and callbacks. It can import only these items:

- Its model.
- The Settings search logic.
- The Desktop compositions.
- The current UI and DOM facades.
- The shared component package.

The checker examines all the imports through the chain. It rejects hubs, storage
adapters, IPC and Tauri, also through a facade. The gallery gives fake data to
the same view. `src/settings-resources.ts` gives the hub projections.
`src/components/compositions.ts` can import only its stylesheet, the UI and
menu facades and the shared package. It cannot use feature models or search
rules.

All production modules in `src/components/` obey an allowlist of shared controls
and portable presentation helpers. The checker rejects app controllers, IPC,
Tauri, stories and network or storage effects in the module. Domain components can use
the canonical model types and the i18n adapter. Business effects stay out of
the components. `gallery.ts` and `stories.ts` are composition roots, not
production components.

The unit test of the catalog makes sure that each declared production consumer
gets to its component source through the imports of the product. This includes the
compatibility facades. Refer to
[ADR 0060](../decisions/0060-isolated-desktop-presentation.md) and the
[presentation contract](../contracts/desktop-presentation.md).

## Feature-based organization

When you divide a large file, move a full responsibility with its types and
tests. Do not divide a file only because it is large. A feature can contain
these files:

```text
feature/
  model.ts        state and pure rules
  service.ts      use-case coordination
  view.ts         DOM and interaction
  contract.ts     types that cross the boundary, if any
  *.test.ts
```

It is not necessary to change the full project to this tree at one time. Make a
new module in this tree only when the change makes the boundary necessary.

## How to verify

Review, focused tests and small fitness functions keep these rules in force:

- `npm run architecture:check` runs the fixtures of the dependency checker. It
  examines the source graph below and keeps the provider and protocol checks.
- Types for `ProviderId` and for the canonical events. The compiler makes sure
  that the code includes all their variants.
- A parity test between the IPC commands, the Rust handlers and the mock.
- Conformance fixtures for each adapter.

Do not add a dependency analysis tool before there is a specified rule that the
tool can examine.

### Executable import rules

The checker parses the production TypeScript and JavaScript files in `src/`,
`relay/src/`, `packages/design-system/src/` and `packages/terminal-state/src/`,
with all their subdirectories. It does not parse tests and declaration files. It
uses the TypeScript parser that the builds install.

- Static imports, re-exports, literal dynamic imports and `require` calls go
  into the graph. Relative source paths resolve to their local module. This
  includes `.js` imports of TypeScript sources and directory index files.
- A runtime cycle causes a failure. Use explicit `import type` or `export type`
  for dependencies on types only. An import of values and types together keeps
  a runtime edge.
- The collaboration core and the mobile modules cannot use the desktop
  dependencies that the rules do not let them use. This applies also through a helper
  or a barrel. An import of a shell type that is not permitted also causes a
  failure. The checker does not follow
  type-only targets to find runtime effects.
- The dependencies of the design system stay in that package. This includes
  nested files.
- The terminal-state helper cannot import application modules. Browser code can
  use only its pure `types.ts`, `appearance.ts` and `replay.ts` modules. These
  modules cannot get Node, emulator or ambient runtime dependencies.
- Some modules cannot import external runtime packages. They also cannot use
  selected ambient names for the DOM, storage, the network, Worker APIs, timers,
  `process` or `console`. This rule applies to these modules and to their
  runtime dependencies:
  - `timeline.ts`.
  - The review rules and message codecs: `review-comments.ts`,
    `review-context.ts` and `message-context.ts`.
  - The relay `logic.ts` and the relay `protocol.ts`.
  - The browser modules of terminal-state.
- An import path that the code calculates, or a relative source import that does not resolve
  causes a failure. Thus, no import goes out of the graph without a warning.
  Asset imports do not make source execution edges.

These checks do not examine the internal code of dependency packages. They do
not show that all code is pure. The checks of ambient names are conservative
syntax checks, not a semantic scope analysis. TypeScript, focused behavior tests
and review continue to be necessary. The rule for provider names applies only to
the seven presentation modules in `scripts/check-architecture.mjs`. It does not
prevent all code that selects a provider. The fixtures in
`scripts/architecture-dependencies.test.mjs` show the permitted and the
rejected dependency structures.

## Rust crates

The application rules are in `prometeu-core`. The core owns ports, and the
composition roots supply the effects through these ports
([ADR 0085](../decisions/0085-portable-core.md),
[core contract](../contracts/application-core.md)). The dependencies point to
the core:

| Crate | Responsibility | Can use |
| --- | --- | --- |
| `prometeu-core` | Models, portable rules, sequence rules and ports | `serde`, `serde_json`, `uuid` only |
| `prometeu-oauth` | PKCE, loopback callback and `Consent` port | No workspace crate |
| `prometeu-protocols` | Shared Codex, Cursor ACP and model-catalog protocols | core |
| `prometeu-files`, `prometeu-git` | Project files, search, scripts, account storage. Native Git. | core |
| `prometeu-process`, `prometeu-profiles` (Unix) | Processes, PTYs, pipes, commands with limits. Account profiles. | core |
| `prometeu-tools` (Unix) | MCP, packages, skills, plugins, MCP credentials | core, profiles, oauth |
| `prometeu-host` (Unix) | Remote-machine daemon ([ADR 0101](../decisions/0101-remote-machines.md)) | core, process, protocols |
| `prometeu` (`src-tauri`) | macOS and Linux desktop | core, adapters. Unix crates under `cfg(unix)`. |

Rules:

- Only the desktop crate (`src-tauri`) links Tauri.
- Only the composition roots select the implementations. The composition roots
  are `src-tauri/src/main.rs`, the remote host and `src/main.ts`.
- The application rules and the presentation do not select code for an
  operating system. This also applies to equivalent `match` chains. Code for
  one target stays in adapter modules and in Cargo dependencies for that target.
- Native requests are generic parameters in the adapter. They are not IPC
  payloads at any time. Native requests include prepared commands, PTY builders,
  captured profiles and paths.

These checks make sure that the crates obey the rules:

- `crates/core/tests/boundary.rs` has an allowlist for the core manifest. It
  rejects known native API tokens and tokens that select an operating system.
- `npm run architecture:check` examines `cargo tree`. Thus, portable crates do
  not use Tauri.

These checks are conservative. They are not a semantic analysis. Aliases,
macros and the internal code of dependencies must also have a review.
