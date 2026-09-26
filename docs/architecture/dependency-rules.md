# Dependency rules

Status: rules in force and direction of evolution.

## Principle

An interface exists when it separates a policy of ours from a technology or
protocol that may vary. Interfaces are not required between internal functions
only to increase the number of layers.

## Conceptual layers

| Layer | Current examples | May know about |
| --- | --- | --- |
| presentation | `chat.ts`, `workspace.ts`, `workspace-changes.ts`, `sidebar.ts` | view models, use cases and IPC contracts |
| derived domain | `timeline.ts`, `relay/src/logic.ts` | domain types and pure functions |
| application | `workspace_tools.rs`, `session.rs`, coordination in `main.ts` | domain and external ports |
| adapters | `claude.rs`, `codex.rs`, IPC, relay transport, Git/files | external protocols and core contracts |

The current directories do not literally represent these layers. The table
serves to decide ownership and dependency direction during incremental changes.

## Rules in force

1. `timeline.ts` does not depend on DOM, Tauri or network.
2. `relay/src/logic.ts` performs no I/O; `room.ts` interprets its effects.
3. `relay/src/protocol.ts` does not depend on APIs exclusive to the app or the Worker.
4. Persisted state is changed in the backend and republished by the `board` event.
5. Filesystem, Git and process access happens in the backend.
6. Remote input is validated again on the side that owns the authority.
7. Backend errors cross IPC as codes/data and are translated in the frontend.
8. Presentation decides visibility through `AgentCapabilities`; provider name
   comparisons stay in the catalog or in the adapters.
9. Runtime imports between local TypeScript modules are acyclic. Type-only
   imports can refer back to a caller without creating a runtime dependency.

## Composition and application use cases

Connect features where their caller already coordinates them. For example,
`settings.ts` supplies the callback that refreshes plugins and the catalog after
`skills.ts` refreshes its own data. `main.ts` supplies `issues.ts` with the live
Linear connection callback. Neither feature needs to import its caller or use
a global event bus to obtain those dependencies.

`src-tauri/src/workspace_tools.rs` validates tool selections and changes one
workspace axis using explicit board state. It does not access process handles
or mutate tabs. `session.rs` reads the native IPC body, preserves absent versus
null arguments, translates validation errors and publishes the board.

Saving a selection is allowed during a turn; existing processes keep their
captured tools until they stop. The new selection applies at the next spawn or
resume of a stopped process. Tests exercise validation and preservation of
sibling tabs without `AppHandle`, `AppState`, a saver or a provider process.
The board types still come from `state.rs`, so this boundary is not yet a
standalone crate. See
[ADR 0050](../decisions/0050-tested-application-boundaries.md).

## Rules in force for agents

1. Claude, Codex or any other vendor protocol appears only in the corresponding
   adapter and in that adapter's fixtures.
2. The core receives `ConversationCommand` and produces `ConversationEvent`,
   both owned by Prometeu.
3. A new provider implements the same port and passes the conformance suite.
4. Unknown events do not take a session down; they stay observable and are
   ignored compatibly until they have an explicit translation.

The legacy transcript reader is an explicit exception to the first rule. It is
isolated in `conversation-legacy.ts` and in the Claude adapter's replay path,
without reaching the timeline or the canonical protocol. Prometeu does not
produce new lines in the legacy format.

## Boundaries that justify interfaces

### Agent runtime

Varies per installation, catalog, protocol, resume and capabilities. It must
expose discovery, start/resume, commands, events and shutdown without leaking
the vendor's payload.

### IPC

Separates TypeScript and Rust. Name, arguments, return value, error and events
form a single contract. The web mock is another adapter of that same contract.
`src/ipc.ts` owns the command argument/result map consumed by frontend callers
and `IpcHandlers` in the mock. Exact command-name parity with Rust is tested;
Rust payload shapes remain manually synchronized. See the
[IPC contract](../contracts/ipc.md).

### Collaboration

`team-transport.ts` abstracts the socket; `team-control.ts` turns frames into
local actions. The protocol and its validation stay shared.

The core (`team-member.ts` and the features `team-owner.ts`, `team-viewer.ts`,
`team-comments.ts`) receives the ports from `team-ports.ts` through the shell:
`Membership`, `SecurityStore` and `OwnerHost`. Features are hooks registered on
the member, in the order chosen by the composition root. `src/team.ts` is the
desktop shell; no `team-*.ts` imports `@tauri-apps`, `./ipc`, `./mock` or
`./team`. See [ADR 0026](../decisions/0026-portable-collaboration-core.md).

### Context evaluation

Varies per external service and its wire format. Features ask closed questions
through the port in `evaluation.rs`/`evaluation.ts` and apply their own rules;
the credential, transport, retries and vendor validation stay in `typesafe.rs`.
`context-review.ts` imports only the port types and runs with a fake port in
tests. See [ADR 0058](../decisions/0058-optional-context-evaluation.md).

### Local system

Git, files, PTY, processes and the embedded browser are external effects. Rules
that choose when to run those effects must stay testable without them.

## Isolated Desktop presentation

`src/components/resource-view.ts` consumes typed snapshots, translated labels and callbacks.
It may reach only its model, Settings matching, Desktop compositions, existing UI/DOM facades and the
shared component package. The transitive import checker rejects hubs, storage
adapters, IPC and Tauri, including through a facade. The gallery supplies fake
data to that same view; `src/settings-resources.ts` supplies hub projections.
`src/components/compositions.ts` may reach only its stylesheet, UI/menu facades and the
shared package; it cannot depend on feature models or matching rules.
All production modules under `src/components/` follow an allowlist of shared
controls and portable presentation helpers. The checker rejects app controllers,
IPC, Tauri, stories and direct network/storage effects. Domain components may
use canonical model types and the i18n adapter; business effects stay outside.
`gallery.ts` and `stories.ts` are composition roots, not production components.
The catalog's unit test verifies every declared production consumer can reach
its component source through real imports, including compatibility facades.
See [ADR 0060](../decisions/0060-isolated-desktop-presentation.md) and the
[presentation contract](../contracts/desktop-presentation.md).

## Feature-based organization

When splitting a large file, extract a complete responsibility, with its types
and tests, instead of splitting by size. A feature may contain:

```text
feature/
  model.ts        state and pure rules
  service.ts      use-case coordination
  view.ts         DOM and interaction
  contract.ts     types that cross the boundary, if any
  *.test.ts
```

The project does not need to adopt this whole tree at once. A new module should
be born in it only when the change already requires the boundary.

## How to verify

The rules are protected by review, focused tests and small fitness functions:

- `npm run architecture:check` runs dependency-checker fixtures, checks the
  source graph described below and retains the provider/protocol checks;
- exhaustive types for `ProviderId` and canonical events;
- a parity test between IPC commands, Rust handlers and the mock;
- conformance fixtures per adapter.

Do not introduce a dependency-analysis tool before a concrete rule exists that
it can actually verify.

### Executable import rules

The checker parses production TypeScript and JavaScript recursively under
`src/`, `relay/src/` and `packages/design-system/src/`. Tests and declaration
files are excluded. It uses the TypeScript parser already installed for builds.

- Static imports, re-exports, literal dynamic imports and `require` calls enter
  the graph. Relative source paths resolve to their local module, including
  `.js` imports of TypeScript sources and directory index files.
- Runtime cycles fail. Use explicit `import type` or `export type` for type-only
  dependencies; mixed value/type imports retain a runtime edge.
- Collaboration core and mobile modules cannot reach their forbidden desktop
  dependencies through a helper or barrel. Direct imports of forbidden shell
  types also fail, but type-only targets are not traversed for runtime effects.
- Design-system dependencies stay inside that package, including nested files.
- `timeline.ts`, relay `logic.ts` and relay `protocol.ts`, plus their runtime
  dependencies, cannot import external runtime packages or use selected ambient
  names for DOM, storage, network, Worker APIs, timers, `process` or `console`.
- Computed import paths and unresolved relative source imports fail rather than
  silently escaping the graph. Asset imports do not create source execution
  edges.

These checks do not inspect dependency package internals or prove all code is
pure. Ambient-name checks are conservative syntax checks, not semantic scope
analysis. TypeScript, focused behavior tests and review remain necessary. The
provider-name restriction still targets the six presentation modules listed in
`scripts/check-architecture.mjs`; it is not a blanket ban on provider dispatch.
Fixtures in `scripts/architecture-dependencies.test.mjs` demonstrate the allowed
and rejected dependency shapes.
