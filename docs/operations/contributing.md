# Contributor recipes

Start with [setup and validation](development.md) and the
[architecture map](../../ARCHITECTURE.md). These recipes identify the files and
proof needed for common changes. They do not require a new service, directory
hierarchy or interface for every feature.

## Add a provider capability or adapter

1. Define the user-visible behavior in the
   [agent runtime contract](../contracts/agent-runtime.md). Add a capability only
   when it changes what the application offers; protocol method names stay in
   the adapter.
2. Update `AgentCapabilities` in [Rust](../../src-tauri/src/agents.rs) and
   [TypeScript](../../src/agents.ts), including bootstrap and mock descriptors.
   Unsupported or unproven behavior stays `false`. Render controls through
   `capabilitiesOf`, without provider-name comparisons in presentation code.
3. Implement translation in [Claude](../../src-tauri/src/claude.rs) or
   [Codex](../../src-tauri/src/codex.rs). Keep external payloads there. Emit the
   existing [canonical events](../contracts/conversation-events-v1.md); change
   that contract only when the common behavior needs a new event or command.
4. For a new provider, extend `ProviderId` in [Rust board model](../../src-tauri/crates/core/src/board.rs)
   and [frontend types](../../src/types.ts), then cover discovery, accounts,
   launch/resume and shutdown. Follow exhaustive matches through their callers;
   an unavailable installation must not disable other providers.
5. Add sanitized adapter fixtures and assertions for event order, request/answer
   correlation and unknown events. Follow the fixture rules in the
   [development guide](development.md#capturing-agent-fixtures). Add reducer and
   integration coverage for changed behavior; add or expand browser coverage only
   when it meets the [E2E scope policy](development.md#e2e-scope).
6. Update the [provider matrix](../quality/provider-matrix.md) with the exact
   tests proving support. Its conformance table is a target, not a claim that
   every scenario already has a fixture.

Useful focused checks, run from the repository root:

```sh
npm run test:rust -- codex::tests
npm run test:web -- src/agents.test.ts src/conversation.test.ts src/timeline.test.ts
npm run architecture:check
```

Use the adapter you changed as the Rust filter. Fixture tests prove translation;
check the installed CLI separately for login, real process startup and resume.

## Add or change an IPC use case

1. Locate the authority in the backend. Keep decisions testable with explicit
   state and effects; the Tauri command adapts arguments, errors and publication.
   [Workspace tool selection](../../src-tauri/crates/core/src/workspace_tools.rs) is a small
   example: `selection` validates an axis and `change` receives the board and
   validated selection, while [the commands](../../src-tauri/src/session.rs)
   handle native arguments, translate `Invalid` and publish.
2. Preserve invariants before moving code. The tool example changes only the
   requested axis, keeps sibling tabs intact and never accesses process handles.
   Its tests check payload validation and state preservation without Tauri.
3. Register a new command in [main.rs](../../src-tauri/src/main.rs), declare its
   arguments and result in [ipc.ts](../../src/ipc.ts), and implement equivalent
   success and rejection behavior in [mock.ts](../../src/mock.ts). Keep Rust
   serde names and optional fields consistent with the TypeScript map.
4. Add focused use-case and serialization tests. The existing
   [handler parity test](../../src-tauri/tests/mock.rs) checks command names;
   it does not prove Rust/TypeScript payload compatibility. For an event, test
   its emitter and consumer and dispose listeners with their screen.
5. Add UI copy to [Portuguese](../../src/i18n.pt.ts) and
   [English](../../src/i18n.en.ts); translate backend codes with `fromBack`.
   Update the [IPC contract](../contracts/ipc.md) and, if affected, the
   [persistence contract](../contracts/persistence.md).

```sh
npm run test:rust -- workspace_tools
npm run test:web -- src/ipc.test.ts
npm run typecheck
```

The workspace tool use case and board models live in the independent
`prometeu-core` crate. Supply effects through application-owned ports and keep
Tauri at the command edge. See the [core contract](../contracts/application-core.md)
and [ADR 0085](../decisions/0085-portable-board-core.md) before extending that boundary.

## Connect features or change collaboration

Keep coordination in an existing caller. [Settings](../../src/settings.ts)
supplies the callback that refreshes plugins and the catalog after a skill
change. [Main](../../src/main.ts) supplies the issues feature with live Linear
connection state. A feature does not need to import its caller or introduce a
global event bus for these dependencies.

For shared desktop/mobile behavior:

1. Put shared behavior in the relevant `team-*` feature. Supply platform effects
   through the existing [ports](../../src/team-ports.ts). Compose them in
   [team.ts](../../src/team.ts) for desktop or
   [mobile/main.ts](../../src/mobile/main.ts) for the browser. Mobile registers
   viewer and comments features; the owner's Mac retains agent execution.
2. Keep Tauri and desktop IPC out of the portable core and mobile modules,
   including indirect imports. Use `import type` for type-only dependencies;
   keep runtime imports acyclic. The
   [dependency rules](../architecture/dependency-rules.md) define what the
   architecture check enforces.
3. Change relay messages in [protocol.ts](../../relay/src/protocol.ts), including
   validation. Put authorization decisions in [logic.ts](../../relay/src/logic.ts)
   and effects in the Worker adapters. Check audience and remote-control
   authority at the owner as well as the relay.
4. Extend the closest core, protocol or relay tests. The
   [browser-core test](../../src/team-member.test.ts) demonstrates composition
   without Tauri. For a wire change, update the
   [relay contract](../contracts/relay-v4.md) and explain how existing clients
   behave during rollout.

```sh
npm run test:web -- src/team-member.test.ts relay/src/protocol.test.ts relay/src/logic.test.ts
npm run architecture:check
npm run build:mobile
```

Follow the [development guide](development.md) for mobile browser checks and
the Cloud bundle handoff. A successful browser bundle does not prove native
process behavior, Cloud authentication or a deployed relay.

## Change a Cloud contract

The Rails service is a separate repository with its own checks. Start with the
relevant [account](../contracts/cloud-account.md),
[catalog](../contracts/cloud-catalog.md),
[organizations](../contracts/cloud-organizations.md) or
[feedback](../contracts/feedback.md) contract, then read the Cloud's contributor
guide and implementation.

Specify request, response, authentication, limits, errors and compatibility
before changing either side. Browser cookie/CSRF and desktop Bearer paths are
distinct. Test both the Rails producer and the actual Rust or relay consumer;
a browser mock cannot prove an HTTP contract. Preserve missing-field defaults
for older clients where the contract requires them, and test rejection paths
such as stale catalog revisions and expired tickets.

Keep paired PRs linked and state any required deployment order. Update contract
examples and compatibility fixtures together, using synthetic identities and no
credentials. Each repository must remain testable without a sibling checkout;
record which revision of the other side was checked when reviewing both.
