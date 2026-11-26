---
type: Operations
title: "Development and testing"
description: "The development environments, the validation commands and the scope of each test level."
status: stable
---

# Development and testing

## Prerequisites

- Node and npm at the versions that `package.json` declares.
- The Rust toolchain that `src-tauri/rust-toolchain.toml` declares.
- Python 3 for the release validation tests.
- The Chromium and WebKit browsers of Playwright for the E2E tests.
- Claude Code, Codex or the two agents, for tests of sessions with an installed agent.

On Linux, also install the WebKitGTK packages in [Linux](linux.md).

```sh
npm install
npx playwright install chromium webkit
```

## Run modes

```sh
npm run app
npm run dev
PORT=1421 npm run dev
```

`npm run app` starts Tauri through `scripts/app.mjs`. It isolates the port and
the state root for each worktree. `npm run dev` opens only the frontend with
`src/mock.ts`. Use it for work on the interface and for the tests that
Playwright controls.

On macOS, the native app and build commands also install the locked
dependencies of the private terminal helper. They also build the self-contained
Node SEA of the helper. `npm run build:app` is the entry point for the bundle. It
gives the sidecar and resource overlay to Tauri. In development, the app gets
the explicit path of the helper. A packaged app finds its sibling executable.

On macOS, Settings / General / Conversation mode applies to new conversations.
It does not change the saved mode of a current tab. Linux keeps Chat and rejects
the native conversation mode. The tests of the helper and of the CLI do not
replace a run of the Tauri app. The
[provider matrix](../quality/provider-matrix.md) records the current
results of the app for each provider. Refer to the
[terminal state contract](../contracts/terminal-state.md).

The mock does not show the process lifecycle, the filesystem behavior or the
Rust serialization. Playwright does not control the Tauri app on macOS, because
WKWebView does not give access to CDP.

`npm run app:bundle` builds the app in debug mode as a `.app` and opens it
through LaunchServices. This is the only procedure for tests of dictation on macOS.
TCC reads `NSSpeechRecognitionUsageDescription` only from a bundle that the app
started itself. macOS stops a binary at the first recognition request in these
conditions:

- `tauri dev` runs the binary.
- A person starts the binary directly from the `.app` folder.

Thus, the microphone button stays hidden in `npm run app`. This mode has no hot
reload.

The system notification banners and their macOS authorization prompt also operate only with
`npm run app:bundle`. The development binary without a bundle cannot use
UserNotifications. The notification settings show this limit adjacent to their
primary control. The notch and the sound operate without notification permission
in the two native run modes.

Tauri gives an ad-hoc signature to the development bundle and validates it
before it opens the bundle. The signing identifier must be the same as
`CFBundleIdentifier`. The signature of the linker identifies the executable as
`Prometeu-<hash>`. With that signature, UserNotifications rejects the
authorization for `co.prometeu.desktop` with `UNErrorDomain` code 1. It does
this before it shows a permission prompt. The development signature operates without
a release certificate and without a private notification entitlement.

The interactive preview of the mock uses a controlled page in an iframe and the
selection script of the app. Selected preview scenarios include Chromium and
WebKit. These scenarios do not show the native PNG and the AppKit gestures.
Refer to the [browser contract](../contracts/browser.md).

## Validation commands

```sh
npm run docs:check
npm run architecture:check
npm run typecheck
npm run build
npm run build:mobile
npm run test:release
npm run test:web
npm run test:scripts
npm run test:rust
npm run test:core
npm run test:process
npm run terminal-state:check
npm run test:contracts
npm run test:e2e
npm run format:check
npm run lint:rust
npm run check
```

`npm run check` runs these items:

- The documentation and architecture checks.
- The Rust format check.
- The desktop and mobile builds and the typecheck.
- All the tests.
- Clippy. Clippy warnings are errors.

It is the same primary validation as in CI.

The terminal helper gate runs after the frontend build. It examines the types
and the differential tests of the helper. It builds the pinned SEA and runs it
with an empty PATH. Then it examines the order and restoration between the built
helper and the host. It does not start a provider and does not use external
credentials.

`npm run architecture:check` first runs the fixtures of the dependency checker
with the test runner of Node. Then it examines the repository. This includes the
Rust crate graph through `cargo tree`. Refer to
[Rust crates](../architecture/dependency-rules.md#rust-crates).

`npm run docs:check` first runs the regression tests of the documentation
checker. Then it examines these items:

- The local links.
- The coverage of the folder indexes.
- The YAML header of the concepts.
- The line limit of `AGENTS.md`.
- The import in `CLAUDE.md`.

The concept metadata is not necessary in the indexes and in the files out of
`docs/`.
Refer to the [bundle map](../README.md).

The phone app is a different bundle. `npm run build:mobile` makes `dist-mobile/`
from `src/mobile/`. In `prometeu-cloud`, `bin/mobile <path to prometeu>` copies
the files into `vendor/mobile/assets` with a hash manifest.
`bin/mobile --check` shows a difference. The Cloud operates without Node when it
sends the bundle. Refer to [ADR 0028](../decisions/0028-mobile-web-app.md).

The desktop CI builds the mobile entry in each run. The Cloud CI validates the
hashes of its copied mobile files. It runs the tests of the Rails page with that pinned
bundle. These checks do not show that the Cloud bundle agrees with the latest
desktop source. To update the mobile client, do these steps:

1. Build the mobile client in the applicable desktop checkout.
2. In the Cloud checkout, run `bin/mobile ../prometeu`.
3. In the Cloud checkout, run `bin/mobile --check ../prometeu`.
4. Commit the assets and the manifest together.

A desktop build does not publish or replace the bundle of the Cloud.

During development, first run the smallest suite that includes the change.
Run `npm run check` before you complete a change to many areas. Also
run it before you open a pull request.

## What each level proves

- Vitest: the reducers, the pure presentation, parsing, the relay protocol and
  the TypeScript adapters.
- Rust tests: the lifecycle, the Codex translation, the state, the paths, Git,
  the internal IPC and the processes.
- Serialization contracts: Rust payloads from the production code agree with a
  fixture in the repository. Literal TypeScript checks and production consumers
  validate the structure of the network data that the fixture shows. Refer to
  [coverage and limits](../contracts/ipc.md#executable-serialization-examples).
- Worker integration: the authentication and the behavior of the local relay.
- Playwright: the most important interface flows with the mock.
- Typecheck and build: the imports, the types, the i18n catalog and the bundle.
- Mobile build: the production browser entry and its portable dependencies make
  a correct bundle. It does not examine the Rails authentication or the
  deployment.
- Architecture check: the runtime imports have no cycles. The portable limits
  stay correct through the modules between them. Selected pure modules have no
  known effects. The presentation and protocol checks stay in force. The
  [dependency rules](../architecture/dependency-rules.md) give the accurate scope
  and the limits.
- Clippy and rustfmt: the discipline of the backend.

For a serialization change that you want, do these steps:

1. Run `npm run contracts:update`.
2. Review `fixtures/backend-contract.json`.
3. Run `npm run test:contracts`.
4. Run `npm run test:rust`.

The update command uses the usual native build dependencies. Do not accept a
changed fixture only to stop a failure.

The controlled Codex subprocess tests run with
`npm run test:rust -- process_transport`. They use Node and the stdio pipes of the operating system.
The child process has a maximum lifetime. The tests do not start a provider or
open a window. Their synthetic peer is different from the fixtures that we recorded
from the CLI.

The Worker integration tests use the test harness of Wrangler. They send HTTP
requests directly to workerd. Thus, they prevent a problem of the development
proxy. After the cancellation of a streamed request body, the proxy can
disconnect from its upstream server. The WebSocket and HTTP Upgrade tests
continue to use the server that waits for connections. A transport error or an
HTTP 500 response causes a test failure.

## E2E scope

Keep Playwright on the core journey of a workspace and a conversation:

- Make or open a workspace.
- Send a message.
- Follow the response.
- Stop the agent and resume.
- Answer an agent request.
- Keep the history and the drafts.

A scenario must be part of this journey to get usual E2E coverage. But this is
not sufficient. Use the smallest unit or integration test that shows the
behavior. A browser test is not automatically necessary for a new control, a
new setting or a new behavior that a person can see.

A scenario out of the core journey must have a specified risk that only a
browser can show:

- An incompatibility of an engine.
- Keyboard or focus accessibility.
- A security limit in the interface.
- A loss of user data during an interaction.

These categories alone do not justify E2E coverage. The rules for parsing,
authorization and persistence continue to belong in the nearest unit or
integration tests.

When you add or extend a browser scenario, write these items in the pull
request or in the summary of the change:

- The user journey that it keeps safe, or the specified risk that applies.
- The failure that is a result of browser behavior.
- Why unit or integration coverage cannot show that behavior.
- What it adds to the current browser scenarios.

Use this qualitative justification. Do not use quotas of tests or a new
framework to classify tests. If a current scenario can clearly keep the
same journey safe, extend that scenario. Run the browser tests with the English
interface. Translation text does not justify an E2E scenario or a locale matrix.
Do not add more providers, viewports and engines without a specified risk for
each added scenario. A test that only calls the mock and examines its state without
the interface belongs below Playwright.

Keep one representative interaction for each different production path. Include
the recovery from a failure when it keeps user data safe. When many controls go
to the same cancellation or validation logic, an E2E scenario for each control
is not necessary. The browser scenario of a component contains the shared behavior of
the component. A product flow must show its integration. It must not do the
full matrix of the component again.

Chromium runs the full lean browser suite. WebKit runs only the scenarios with
the explicit tag `@webkit`. These scenarios are a representative core journey
and the checks that have a specified engine risk. If the title of a test has the
name of a feature, this is not a cause to run its full test group two times. CI
keeps one Playwright worker. Decrease the browser work that is not necessary. Do not
increase the concurrency.

CI captures traces at the first retry and screenshots after a failure. Local
runs keep traces after a failure, because they do not retry. Thus, for a CI
failure that does not occur again, CI keeps a screenshot of the first run
and a trace of the retry. It does not keep a trace of the first failure.

These items have no dedicated E2E gate:

- The secondary forms that make and install plugins.
- The quota presentation.
- The issue filters.
- The groups in the sidebar.
- The empty states of the catalog.
- The preferences of the preview panel.

Rust or Vitest tests include the tool selection, the comparison of MCP servers,
plugin parsing, quota groups, the defaults of Code review and silent alerts. These
lower-level checks do not show the secondary forms or a live CLI installation.
The profile editor keeps its interface regression test. Layout tests use English
at representative viewport dimensions. The combinations of sibling tab states stay in
`workspace_tools.rs`. One interface journey goes through the tool selectors.

These items also have no dedicated browser gate:

- The variations of accounts and settings.
- The shortcuts for catalog management.
- The header style.
- The preferences for tab visibility.
- The rank of recent files.
- The find bar of the file viewer.
- The Command-P file search and the Command-F file search of the project view.

These items have coverage in other suites:

- The collection and rank of recent files: `timeline.test.ts` and
  `crates/files/src/search.rs`.
- The search in a file, the wraparound and the marker markup: `find.test.ts`.
- The keyboard behavior of the palette: the shared
  `e2e/search-picker.spec.ts`.
- The native Git and catalog rules: their Rust tests.

The feedback tests keep representative checks for the submission, for the race
conditions of attachments and for cancellation. They do not examine each
replacement control again. If you remove an interface scenario, a backend test does
not automatically show its presentation or its connections.

Keep the regression coverage for contracts, security, protocols and the backend
in the faster suites in each CI run. A smaller E2E scope does not remove these
guarantees. The browser mock shows the interface integration. It does not show
the native process lifecycle, the filesystem behavior, the operation of an
installed provider or deployed services. Keep the related Rust, relay and contract tests
and the focused native checks.

## Shared Cloud API fixtures

[`fixtures/cloud-api.json`](../../fixtures/cloud-api.json) contains synthetic
HTTP payloads that the Rails service also uses. Rust tests in `cloud.rs` and
`catalog.rs` send them through the production profile decoder and the catalog
parser. The `DesktopContractTest` of the Cloud examines the production endpoints with
Bearer authentication against its copy of the file. Each repository runs its
tests independently. The desktop CI operates without access to the
Cloud repository.

The fixture includes these items:

- Session profiles.
- Responses for a person who is not signed in.
- Empty, current and legacy catalogs.
- Revisions that do not agree.
- Catalog input with credentials, which the Cloud rejects.

It does not replace the integration tests for organizations, the relay, device
login or feedback. These items are examples of the current network contract that
you can run. They are not a new API version or a full schema.

For a paired contract change, do these steps:

1. Update the public fixture and the consumer tests in this repository.
2. Import the fixture into the Cloud checkout.
3. Run the producer tests of the Cloud.

```sh
# In prometeu:
npm run test:rust -- shared_cloud_contract

# In prometeu-cloud, with prometeu as a sibling checkout:
bin/contracts ../prometeu
bin/contracts --check ../prometeu
bin/rails test test/integration/desktop_contract_test.rb test/contracts_test.rb
```

Commit the fixture, the copied fixture, the hash manifest and the changed tests
in their repositories. The Cloud CI runs `bin/contracts --check` to validate its
pinned copy. If you give a source path, the same command also makes sure that
the two repositories use the same fixture bytes. Link the paired pull requests
and record the revisions of your tests. A hash check alone does not show the
server behavior or the compatibility of a deployment.

## Simultaneous instances

Debug and release use different roots. `scripts/app.mjs` gives each development
worktree its own configuration. This prevents collisions of `board.json` and of
the Vite port. Do not replace that initialization with `tauri dev` alone
before you know how the worktrees stay isolated.

The `.prometeu/settings.toml` file of this repository gives the app and the mock
as scripts. Thus, you can use Prometeu to make changes to Prometeu.

## Capturing agent fixtures

A synthetic scenario must identify itself as synthetic and tell what it
examines. It must not claim a provider version. It must not replace evidence
that we recorded from the CLI. The requirements below apply to recorded fixtures.

A protocol fixture must obey these rules:

- It comes from the output of the CLI version that the test gives.
- It does not contain personal prompts, user paths, tokens or credentials.
- It keeps the ids and the order that are necessary for the scenario.
- It contains the smallest set of frames that shows the behavior again.
- It records the provider, the CLI version and the capability that it shows.
- It does not use the network during the suite.

If a CLI update causes a fixture failure, review the adapter and the contract.
Do not delete the assertion only to remove the failure.

## Interface text

Portuguese is the source catalog in `src/i18n.pt.ts`. English has the same keys
in `src/i18n.en.ts`. TypeScript uses `t`/`tn`. Rust sends codes that the frontend
translates with `fromBack`.

The application does not translate agent output, terminal output and text from
the person.

Write test names, helpers, comments and authored fixtures in English. Keep the
contract values and the Unicode examples that show parsing or encoding. The tests
examine locale selection, interpolation and structured errors. They do not
examine the quality of the translation. TypeScript makes sure that the English
catalog has all the keys of the source catalog.

## Portable crates and native adapters

These suites run without Tauri, GUI libraries, installed agent CLIs or a
network. `npm run test:rust` includes all of them.

- `npm run test:core`: the portable rules and the ports with fake adapters.
- `npm run test:process`: local subprocesses, PTYs and private pipes of the
  operating system. The
  tests include backpressure, blocked writes, output limits, escalation, EOF
  before exit and waiters that the caller discards. Unix only.
- `npm run test:profiles` and `npm run test:tools`: account profiles, MCP
  encoding and package preparation. They use fake files, fake catalogs
  and a synthetic installer CLI.

## Remote machine host

Remote machines ([ADR 0101](../decisions/0101-remote-machines.md)) use the
Linux build of `prometeu-host`. Debug apps read it from
`src-tauri/target/x86_64-unknown-linux-musl/release/prometeu-host`. To build it
on macOS with Zig, run these commands:

```sh
brew install zig cargo-zigbuild
rustup target add x86_64-unknown-linux-musl
cd src-tauri && cargo zigbuild -p prometeu-host --release --target x86_64-unknown-linux-musl
```

`PROMETEU_REMOTE_HOST` tells the app to use a different build. `cargo test -p
prometeu-host` runs the host with local attachments. An optional journey
installs the build on a remote machine and disconnects SSH during the output:

```sh
PROMETEU_REMOTE_TEST_SSH=<ssh destination> cargo test --bin Prometeu vps_journey -- --ignored --nocapture
```
