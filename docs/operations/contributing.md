---
type: Operations
title: "Contributor recipes"
description: "The procedures for changes to providers, IPC, collaboration and Cloud contracts."
status: stable
---

# Contributor recipes

Start with the [setup and validation](development.md) and the
[architecture map](../../ARCHITECTURE.md). These recipes tell you the files and
the evidence that frequent changes must have. It is not necessary to add a new
service, a new directory hierarchy or a new interface for each feature.

## Add a provider capability or adapter

1. Specify the behavior that a person can see in the
   [agent runtime contract](../contracts/agent-runtime.md).
2. Add a capability only when it changes what the application gives. Keep the
   protocol method names in the adapter.
3. Update `AgentCapabilities` in [Rust](../../src-tauri/src/agents.rs) and
   [TypeScript](../../src/agents.ts). Include the bootstrap and mock descriptors.
4. Keep the value `false` for behavior that the agent cannot do or that has no
   evidence.
5. Render the controls through `capabilitiesOf`. Do not compare provider names
   in the presentation code.
6. Write the translation in [Claude](../../src-tauri/src/claude.rs) or
   [Codex](../../src-tauri/src/codex.rs). Keep the external payloads there.
7. Send the current [canonical events](../contracts/conversation-events-v1.md).
   Change that contract only if the shared behavior must have a new event or
   command.
8. For a new provider, extend `ProviderId` in the
   [Rust board model](../../src-tauri/crates/core/src/board.rs) and in the
   [frontend types](../../src/types.ts).
9. For a new provider, add coverage for discovery, accounts, launch, resume and
   shutdown.
10. Examine each match expression that lists all providers, and its callers. If one provider is not
    installed, the other providers must continue to operate.
11. Add sanitized adapter fixtures. Add assertions for the event order, the
    relation of requests to answers and unknown events.
12. Obey the fixture rules in the
    [development guide](development.md#capturing-agent-fixtures).
13. Add reducer and integration coverage for the changed behavior.
14. Add or extend browser coverage only when the change agrees with the
    [E2E scope policy](development.md#e2e-scope).
15. Update the [provider matrix](../quality/provider-matrix.md) with the tests
    that show the support.

The scenario table of the provider matrix is a target. It does not tell that
each scenario has a fixture at this time.

Run these focused checks from the root of the repository:

```sh
npm run test:rust -- codex::tests
npm run test:web -- src/agents.test.ts src/conversation.test.ts src/timeline.test.ts
npm run architecture:check
```

Use the name of the adapter that you changed as the Rust filter. The fixture
tests show that the translation is correct. Examine the installed CLI
independently for the login, the start of the process and the resume.

## Add or change an IPC use case

1. Find the part of the backend that makes the decision. Keep the decisions in
   functions that tests can examine with explicit state and effects.
2. Let the Tauri command change the arguments, the errors and the publication.
3. Use [workspace tool selection](../../src-tauri/crates/core/src/workspace_tools.rs)
   as a small example.

   In this example, `selection` validates an axis. `change` gets the board and
   the validated selection. [The commands](../../src-tauri/src/session.rs) process
   the native arguments, translate `Invalid` and publish.
4. Keep the invariants before you move code.

   The tool example changes only the requested axis. It keeps the sibling tabs
   and does not use process handles at any time. Its tests examine the
   validation of the payload and the state without Tauri.
5. Add a new command to the handler list in [main.rs](../../src-tauri/src/main.rs).
6. Declare its arguments and result in [ipc.ts](../../src/ipc.ts).
7. Write the same behavior for success and for a rejected request in
   [mock.ts](../../src/mock.ts).
8. Keep the Rust serde names and optional fields the same as the TypeScript map.
9. Add focused tests for the use case and for the serialization.

   The current [handler parity test](../../src-tauri/tests/mock.rs) examines the
   command names. It does not show that the Rust and TypeScript payloads agree.
10. For an event, write tests for its emitter and its consumer. Discard the listeners
    when their screen closes.
11. Add the interface text to [Portuguese](../../src/i18n.pt.ts) and
    [English](../../src/i18n.en.ts).
12. Translate the backend codes with `fromBack`.
13. Update the [IPC contract](../contracts/ipc.md).
14. If the change is about storage, update the
    [persistence contract](../contracts/persistence.md).

```sh
npm run test:rust -- workspace_tools
npm run test:web -- src/ipc.test.ts
npm run typecheck
```

The use case for workspace tools and the board models are in the
`prometeu-core` crate, which has no dependency on Tauri. Supply the effects through ports that the application
owns. Keep Tauri at the edge, in the commands. Read the
[core contract](../contracts/application-core.md) and
[ADR 0085](../decisions/0085-portable-core.md) before you extend that limit.

## Connect features or change collaboration

Keep the coordination in a current caller. For example,
[Settings](../../src/settings.ts) supplies the callback that refreshes the
plugins and the catalog after a change to a skill.
[Main](../../src/main.ts) supplies the live state of the Linear connection to
the issues feature. For these dependencies, a feature does not import its
caller. It also does not add a global event bus.

For behavior that the desktop and the mobile browser share, do these steps:

1. Put the shared behavior in the applicable `team-*` feature.
2. Supply the platform effects through the current [ports](../../src/team-ports.ts).
3. Compose the ports in [team.ts](../../src/team.ts) for the desktop or in
   [mobile/main.ts](../../src/mobile/main.ts) for the browser.

   The mobile browser adds the viewer and comments features. The agent
   continues to operate on the Mac of the owner.
4. Keep Tauri and the desktop IPC out of the portable core and the mobile
   modules. This rule also applies to indirect imports.
5. Use `import type` for dependencies on types only. Do not make cycles in the
   runtime imports.

   The [dependency rules](../architecture/dependency-rules.md) specify what the
   architecture check makes sure of.
6. Change the relay messages and their validation in
   [protocol.ts](../../relay/src/protocol.ts).
7. Put the authorization decisions in [logic.ts](../../relay/src/logic.ts). Put
   the effects in the Worker adapters.
8. Examine the audience and the remote-control authority at the owner and also
   at the relay.
9. Extend the nearest core, protocol or relay tests.

   The [browser-core test](../../src/team-member.test.ts) shows composition
   without Tauri.
10. For a change to the network format, update the
    [relay contract](../contracts/relay-v4.md).
11. In the relay contract, tell how the current clients operate during the
    rollout.

```sh
npm run test:web -- src/team-member.test.ts relay/src/protocol.test.ts relay/src/logic.test.ts
npm run architecture:check
npm run build:mobile
```

For the checks of the mobile browser and the handoff of the Cloud bundle, obey
the [development guide](development.md). A satisfactory browser bundle does not
show the behavior of the native process, the Cloud authentication or a
deployed relay.

## Change a Cloud contract

The Rails service is in a different repository and has its own checks. Start
with the applicable contract:
[account](../contracts/cloud-account.md),
[catalog](../contracts/cloud-catalog.md),
[organizations](../contracts/cloud-organizations.md) or
[feedback](../contracts/feedback.md). Then read the contributor guide and the
implementation of the Cloud.

Before you change one of the two sides, specify these items:

- The request and the response.
- The authentication.
- The limits.
- The errors.
- The compatibility.

The browser path with a cookie and CSRF is different from the desktop path with
a Bearer token. Write tests for the Rails producer and also for the Rust or relay consumer. A
browser mock cannot show that an HTTP contract is correct. If the contract
gives default values for missing fields for older clients, keep these defaults.
Write tests for the paths of rejected requests, for example catalog revisions that are not current and expired tickets.

Keep the links between the two related pull requests. If a deployment order is
necessary, write it. Update the contract examples and the compatibility fixtures
together. Use synthetic identities and no credentials. The tests of each
repository must operate without a checkout of the other repository. When you review the
two sides, record which revision of the other side you examined.
