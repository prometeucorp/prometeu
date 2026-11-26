# Prometeu — guide for agents

This file is the short map of the repository. The full source of truth is in
`ARCHITECTURE.md` and in `docs/`. Do not copy full documents into this file.

## Before you change code

1. Read `ARCHITECTURE.md` to find the boundary that the change touches.
2. Open only the `docs/` documents that the map gives for that area.
3. Make sure that the tests and the code show the current behavior.
4. Keep the local changes that are not part of the task.

If the code and the documentation do not agree, use the behavior of the code
that you can prove to find the cause. Update the documentation in the same
change. Do not make a silent architectural decision from a difference.

## Documentation map

- `README.md`: the product, the behavior that a person sees and the Quick start.
- `CONTRIBUTING.md`: the contribution procedure and links to recipes for code.
- `ARCHITECTURE.md`: the context, the containers, the responsibilities and the
  primary flows.
- `docs/architecture/`: the flows and the dependency rules, with more information.
- `docs/contracts/`: the formats that go between processes or layers.
- `docs/decisions/`: the architectural decisions. Proposals are not rules.
- `docs/quality/`: the support matrices and the verification strategy.
- `docs/operations/`: development, CI and release.

`docs/` is the knowledge bundle. `AGENTS.md` stays the instructions that always apply.
If you do not know the location, start at [the map](docs/README.md). Then read
the index of the area and the applicable concept. Follow its dependencies. If
you know the path, open it directly.

## Product invariants

- A session is the transcript. The agent process can stop, and Prometeu can
  resume it.
- Claude and Codex are external adaptations. Protocol differences must stay at
  the edge. Vendor payloads must not go into the presentation.
- The status of the tab is the state of the agent that Prometeu sees.
- The shared session continues to run only on the Mac of the owner. The relay
  sends the messages and keeps the ciphertext and the metadata.
- The content uses E2EE v4 with trust on first contact. It uses changed keys
  automatically. ADRs 0022 and 0042 give the limits.
- `relay/src/protocol.ts` is the source of the types and the validation of the
  relay.
- Interface text that a person sees goes through i18n. User data and agent
  output stay in the language that they had.
- UI controls use `src/ui.ts`, `src/menu.ts` and the shared tokens again.
  Before you make or change controls, read the
  [Design System](docs/architecture/design-system.md).
  The Desktop components, the stories and the code that uses them are in
  [`src/components/`](src/components/README.md) and its `catalog.json`.
  For isolated Desktop screens, obey the [composition recipe](docs/architecture/desktop-composition.md).

`docs/architecture/dependency-rules.md` gives all the dependency rules.

## Where each responsibility lives

- `src/agents.ts`: the typed catalog, the relation between models and providers,
  and the capabilities.
- `src/timeline.ts`: a pure reducer from the conversation stream to the items on
  the screen.
- `src/chat.ts`: the presentation of the conversation and the interaction with
  it.
- `src/desk.ts`: the desk, which is the home screen. It has one `ChatView` for
  each conversation that operates.
- `src-tauri/crates/core/`: the portable rules for board models, the order of
  conversations, sessions, launch, accounts and tools. They use ports that the
  caller gives (ADR 0085).
- `src-tauri/crates/process/`, `profiles/`, `tools/`, `files/`, `git/`,
  `protocols/` and `oauth/`: native adapters without Tauri. The desktop composes
  them. `prometeu-host` uses core and process again.
- `src-tauri/src/main.rs`: the composition root of the desktop. `chat.rs`,
  `chat/host.rs` and `session/launch.rs` compose conversations from core
  services.
- `src-tauri/src/claude.rs`, `antigravity.rs` and `crates/protocols/src/codex.rs`:
  the provider adapters. `src-tauri/src/agent_launch.rs` connects the native
  preparation to the launch.
- `src-tauri/src/session.rs`: the desktop operations and the lifecycle of
  workspaces and tabs. `state.rs` and `board_store.rs` adapt board events and
  file storage.
- `src-tauri/src/accounts.rs` and `account_login.rs`: the account facade of the
  desktop and the native authentication.
- `src-tauri/crates/host/` and `src-tauri/src/remote/`: the SSH daemon for the
  remote machine, its desktop connection, the installer and the remote
  workspaces (ADR 0101).
- `src/team.ts`: the desktop shell for collaboration (team.json, organizations,
  Tauri ports, facade).
- `src/team-member.ts` and `src/team-*.ts`: the portable core of collaboration
  and its features. They do not import Tauri or IPC (ADR 0026).
- `src/mobile/`: the browser shell on the same core. The Cloud contains a copy of
  it as a bundle (ADR 0028).
- `src-tauri/src/evaluation.rs` and `src-tauri/src/typesafe.rs`: an optional port
  that examines the context, and its TypeSafe adapter (ADR 0058). At this time, no
  code uses them. `src/typesafe-settings.ts` saves and tests the key.
- `src-tauri/src/analytics.rs` and `src/analytics.ts`: PostHog events for the
  use of features, without content, and explicit identities of controls
  (ADR 0093).
- `relay/src/protocol.ts`: the network contract that the app and the Worker
  share.
- `relay/src/logic.ts`: the pure rules of the relay.

## Contracts and decisions

These changes have special requirements: a change to a stored format, IPC,
the relay, the conversation protocol, a trust boundary or a dependency between
layers. For each of these changes, do these steps:

- Update the applicable contract in `docs/contracts/`.
- If there are alternatives with trade-offs, add or update an ADR.
- Include a compatibility test, or tell why it does not apply.

Keep ADRs in agreement with the decision in the code. When you replace or
remove a decision, move the guarantees that continue to apply into its
replacement. In the same change, remove the obsolete document and its links. Git
keeps the history. Do not keep replaced instructions in the current
documentation. Keep ADR numbers stable, and do not use retired numbers again.
See [the decision lifecycle](docs/decisions/README.md).

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

While you write the code, first run the test that is nearest to the change.
Before you complete a change that touches many areas, run `npm run check`. If
you cannot run a check, tell which check and why.

Before you add or expand browser tests, obey the
[E2E scope policy](docs/operations/development.md#e2e-scope). If only the
behavior that a person sees changes, E2E coverage is not necessary.

The browser uses `src/mock.ts`. The Tauri app uses the Rust backend. New IPC
commands must be in the two paths and in the typed registry in `src/ipc.ts`.

## Commits and release

Commits obey Conventional Commits in English:

```text
type(scope): description
```

`feat`, `fix` and `perf` show in the changelog. Write a description about what
the person sees, in lowercase and without a period at the end. Put internal
information in the body, or use `refactor`, `test`, `docs`, `chore`, `ci`,
`build` and `style` commits.

`sh scripts/release.sh` makes the release. Do not make a tag or publish an
artifact if the user did not tell you to do it. See `docs/operations/release.md`.

## Documentation maintenance

- Write all new or changed technical documentation in ASD-STE100 Simplified
  Technical English, Issue 9. Use the project terms and the review in the
  [technical writing guide](docs/operations/technical-writing.md). Do not copy
  the text of the specification into the repository.
- A new concept in `docs/`, or a concept with large changes, has the YAML
  `type`, `title`, `description` and `status`. It also has a line in the
  `README.md` of its folder. Folder indexes and files that are not in `docs/`
  do not have this envelope.
- Do not mix draft proposals with the current guidance. When an
  implementation plan is complete, it becomes a current contract or ADR, or you
  remove it. Git keeps the history.
- Write code identifiers, test names, comments and doc comments in English.
  This rule also applies to tests, scripts, stylesheets and configuration files.
- Write test fixtures in English. A contract or a Unicode regression can make a
  different value necessary. Keep the identifiers in stored data and in network messages.
- Run E2E scenarios with the English UI. Do not copy a scenario for each
  language. Do not add tests that only examine translated text.
- In documents, tell why, and give the contracts. Do not tell what clear code does.
- Comments give information about local code. Documents give flows and decisions.
- Links are relative to the repository and must stay correct.
- A new feature must give links to its tests. It must also give the differences
  between agents in the matrix in `docs/quality/provider-matrix.md`.
