# Prometeu — guide for agents

This file is the repository's short map. The detailed source of truth lives in
`ARCHITECTURE.md` and in `docs/`; do not copy whole documents here.

## Before changing code

1. Read `ARCHITECTURE.md` to locate the affected boundary.
2. Open only the `docs/` documents pointed to for that area.
3. Confirm the existing behavior in the tests and in the code.
4. Preserve local changes that do not belong to the task.

If code and documentation diverge, use the code's verifiable behavior to
diagnose the situation and update the documentation in the same change. Do not
turn a divergence into a silent architectural decision.

## Documentation map

- `README.md`: product, visible behavior and quick start.
- `CONTRIBUTING.md`: contribution workflow and links to implementation recipes.
- `ARCHITECTURE.md`: context, containers, responsibilities and core flows.
- `docs/architecture/`: details of the flows and dependency rules.
- `docs/contracts/`: formats that cross processes or layers.
- `docs/decisions/`: architectural decisions; proposals are not rules in force.
- `docs/quality/`: support matrices and verification strategy.
- `docs/operations/`: development, CI and release.

Start at `docs/README.md` for the complete index.

## Product invariants

- A session is the transcript; the agent's process may die and be resumed.
- Claude and Codex are external adaptations. Protocol differences must stay at
  the edge, without spreading vendor payloads through the presentation.
- The workspace stage is the person's decision; the tab's status is the agent's
  observed state. Do not mix the two.
- The shared session still runs only on the owner's Mac. The relay coordinates
  and persists ciphertext and metadata; content uses E2EE v4 with trust on first
  contact and automatic adoption of changed keys. The limits are in ADRs 0022
  and 0042.
- The relay's source of types and validation is `relay/src/protocol.ts`.
- Visible interface text goes through i18n. User data and agent output stay in
  their original language.
- UI controls reuse `src/ui.ts`, `src/menu.ts` and the shared tokens.
  See the [Design System](docs/architecture/design-system.md) before creating or
  changing controls.
  Find Desktop components, stories and real consumers in
  [`src/components/`](src/components/README.md) and its `catalog.json`.
  For isolated Desktop screens, follow the [composition recipe](docs/architecture/desktop-composition.md).

The complete dependency rules are in
`docs/architecture/dependency-rules.md`.

## Where each responsibility lives

- `src/agents.ts`: typed catalog, model/provider association and capabilities.
- `src/timeline.ts`: pure reducer from the conversation stream to screen items.
- `src/chat.ts`: presentation and interaction of the conversation.
- `src/desk.ts`: the desk, the home screen — one `ChatView` per running
  conversation.
- `src-tauri/src/chat.rs`: process, transport, buffer, numbering and lifecycle.
- `src-tauri/src/claude.rs`: Claude's stream-json adapter.
- `src-tauri/src/codex.rs`: Codex's JSON-RPC adapter.
- `src-tauri/src/session.rs`: use cases and workspace/tab lifecycle.
- `src-tauri/src/workspace_tools.rs`: tool selection validation and workspace updates,
  with explicit state and effects; Tauri commands stay in `session.rs`.
- `src-tauri/src/state.rs`: the board's persisted state.
- `src/team.ts`: the collaboration desktop shell (team.json, organizations,
  Tauri ports, facade).
- `src/team-member.ts` and `src/team-*.ts`: the portable collaboration core and
  its features; they never import Tauri or IPC (ADR 0026).
- `src/mobile/`: the browser shell over the same core, vendored in the Cloud as
  a bundle (ADR 0028).
- `src-tauri/src/evaluation.rs` and `src-tauri/src/typesafe.rs`: optional
  context-evaluation port and its TypeSafe adapter (ADR 0058);
  `src/context-review.ts` holds the missing-context review rules.
- `relay/src/protocol.ts`: the network contract shared by the app and the
  Worker.
- `relay/src/logic.ts`: the relay's pure rules.

## Contracts and decisions

A change in a persisted format, IPC, the relay, the conversation protocol, a
trust boundary or a dependency between layers requires:

- updating the corresponding contract in `docs/contracts/`;
- adding or updating an ADR when there is a choice with trade-offs;
- including a compatibility test or explaining why it does not apply.

Keep ADRs aligned with the implemented decision. When replacing or removing a
decision, consolidate the guarantees still in force into its replacement and
remove the obsolete document and links in the same change. Git preserves the
history; do not keep superseded instructions in the current documentation.
Keep ADR numbers stable and never reuse retired numbers. See
[the decision lifecycle](docs/decisions/README.md).

## Development and validation

```sh
npm install
npm run app
npm run dev
npm run docs:check
npm run typecheck
npm run test:web
npm run test:rust
npm run test:e2e
npm run check
```

During implementation, run the test closest to the change first. Before
finishing a cross-cutting change, prefer `npm run check`. If a check cannot run,
say exactly which one and why.

Follow the [E2E scope policy](docs/operations/development.md#e2e-scope) before
adding or expanding browser tests. A visible behavior change does not by itself
justify E2E coverage.

The browser uses `src/mock.ts`; the Tauri app uses the Rust backend. New IPC
commands must exist in both paths and in the typed registry in `src/ipc.ts`.

## Commits and release

Commits follow Conventional Commits in English:

```text
type(scope): description
```

`feat`, `fix` and `perf` appear in the changelog. Use a description aimed at
what the person perceives, in lowercase and without a trailing period. Internal
details belong in the body or in `refactor`, `test`, `docs`, `chore`, `ci`,
`build` and `style` commits.

The release is done by `sh scripts/release.sh`; do not create a tag or publish
an artifact without an explicit request. See `docs/operations/release.md`.

## Documentation maintenance

- Write code identifiers, test names, comments and doc comments in English,
  including tests, scripts, stylesheets and configuration files. Author test
  fixtures in English unless a contract or a Unicode regression needs another
  value. Preserve persisted and wire identifiers.
- Run E2E scenarios with the English UI. Do not duplicate scenarios by language
  or add tests whose only purpose is checking translation copy.
- Document the why and the contracts; do not narrate obvious code.
- Comments explain local details. Documents explain flows and decisions.
- Links are relative to the repository and must stay valid.
- A new feature must point to its tests and state the differences between agents
  in the matrix in `docs/quality/provider-matrix.md`.
