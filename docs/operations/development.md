# Development and testing

## Prerequisites

- Node and npm at the versions declared in `package.json`;
- the Rust toolchain declared in `src-tauri/rust-toolchain.toml`;
- Python 3 for the release validation tests;
- Playwright's Chromium and WebKit for E2E tests;
- Claude Code and/or Codex installed to test real sessions.

On Linux, also install the WebKitGTK packages listed in [Linux](linux.md).

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

`npm run app` starts Tauri through `scripts/app.sh` and isolates the port and
the state root per worktree. `npm run dev` opens only the frontend over
`src/mock.ts`, useful for UI work and for the Playwright-driven tests.

The mock does not prove process lifecycle, filesystem behavior or Rust
serialization. The Tauri app is not driven by Playwright on macOS because
WKWebView does not expose CDP.

`npm run app:bundle` builds the app in debug mode as a `.app` and opens it
through LaunchServices. It is the only way to test dictation on macOS: TCC reads
`NSSpeechRecognitionUsageDescription` only from a bundle the app launched
itself, and a binary run by `tauri dev` (or executed directly from inside the
`.app`) is aborted on the first recognition request. That is why the microphone
button stays hidden under `npm run app`; there is no hot reload in this mode.

System notification banners and their macOS authorization prompt also require
`npm run app:bundle`; the unbundled development binary cannot use
UserNotifications. Notification Settings shows that limitation next to its
master switch. Notch and sound work without notification permission in either
native run mode.

The development bundle is ad-hoc signed by Tauri and verified before opening.
Its signing identifier must match `CFBundleIdentifier`: the linker-only
signature identifies the executable as `Prometeu-<hash>`, which makes
UserNotifications reject authorization for `co.prometeu.desktop` with
`UNErrorDomain` code 1 before showing a permission prompt. Development signing
needs no release certificate or private notification entitlement.

The mock's interactive preview uses a controlled page in an iframe and the app's
own selection script. Selected preview scenarios cover Chromium and WebKit;
the native PNG and AppKit gestures stay outside that proof. See the
[browser contract](../contracts/browser.md).

## Validation commands

```sh
npm run docs:check
npm run architecture:check
npm run typecheck
npm run build
npm run build:mobile
npm run test:release
npm run test:web
npm run test:rust
npm run test:core
npm run test:profiles

npm run test:contracts
npm run test:e2e
npm run format:check
npm run lint:rust
npm run check
```

`npm run check` runs documentation and architecture checks, Rust formatting,
desktop and mobile builds/typecheck, the whole test suite and Clippy with
warnings as errors. It is the same main validation as CI.
`npm run architecture:check` first runs its
dependency-checker fixtures with Node's test runner, then checks the repository.

The phone app is a separate bundle: `npm run build:mobile` generates
`dist-mobile/` from `src/mobile/`; in `prometeu-cloud`,
`bin/mobile <path to prometeu>` vendors the files in `vendor/mobile/assets`
with a hash manifest and `bin/mobile --check` flags a divergence. The Cloud does
not need Node to serve it. See
[ADR 0028](../decisions/0028-mobile-web-app.md).

The desktop CI builds the mobile entry on every run. The Cloud CI verifies its
vendored mobile hashes and tests the Rails page with that pinned bundle. These
checks do not claim the Cloud bundle matches the latest desktop source. When
updating the mobile client, build it in the intended desktop checkout, then run
`bin/mobile ../prometeu` and `bin/mobile --check ../prometeu` in the Cloud. Commit
the assets and manifest together. A desktop build does not publish or replace
the Cloud's bundle.

During development, run the smallest suite that covers the change first. Use
`npm run check` before finishing a cross-cutting change or opening a PR.

`npm run test:core` tests the portable board models, tool selection and injected
publication service without Tauri or GUI libraries. It also runs as part of
`test:rust`; independent Linux/Windows CI guards portability. This does not
exercise a native Windows desktop or a WSL execution bridge.

## What each level proves

- Vitest: reducers, pure presentation, parsing, the relay's protocol and the
  TypeScript adapters.
- Rust tests: lifecycle, Codex translation, state, paths, Git, internal IPC and
  processes.
- Serialization contracts: real Rust payloads match a checked fixture; literal
  TypeScript checks and production consumers verify the represented wire shapes.
  See [coverage and limits](../contracts/ipc.md#executable-serialization-examples).
- Worker integration: authentication and the real behavior of the local relay.
- Playwright: critical UI flows against the mock.
- Typecheck/build: imports, types, the i18n catalog and the bundle.
- Mobile build: the production browser entry and its portable dependencies
  bundle successfully; it does not exercise Rails authentication or deployment.
- Architecture check: runtime imports remain acyclic, portable boundaries hold
  through intermediate modules, selected pure modules stay free of known
  effects, and presentation/protocol checks stay in force. The exact scope and
  limits are in the [dependency rules](../architecture/dependency-rules.md).
- Clippy/rustfmt: backend discipline.

For an intentional serialization change, run `npm run contracts:update`, review
`fixtures/backend-contract.json`, then run `npm run test:contracts` and
`npm run test:rust`. The update command requires the normal native build
dependencies. Never accept a changed fixture merely to silence a failure.

The controlled Codex subprocess tests run with
`npm run test:rust -- process_transport`. They use Node and real stdio pipes,
with a bounded child lifetime, without starting a provider or opening a window.
Their synthetic peer is separate from recorded CLI conformance fixtures.

Worker integration tests use Wrangler's test harness with HTTP requests sent
directly to workerd, avoiding the development proxy's upstream connection loss
after a streamed request body is canceled. WebSocket and HTTP Upgrade probes
still use the listening server; transport errors and HTTP 500 responses fail
the tests.

## E2E scope

Keep Playwright focused on the core workspace and conversation journey: create
or open a workspace, send a message, follow the response, interrupt and resume,
answer an agent request, and preserve history and drafts. Belonging to this
journey is necessary for ordinary E2E coverage, but is not sufficient: prefer
the smallest unit or integration test that proves the behavior. A new control,
setting or visible behavior does not automatically need a browser test.

Exceptions outside the core need a concrete risk that requires a real browser:
an engine incompatibility, keyboard or focus accessibility, a security boundary
in the UI, or loss of user data during interaction. These labels alone do not
justify E2E coverage; parsing, authorization and persistence rules still belong
in the closest unit or integration tests.

When adding or expanding a browser scenario, explain in the PR or change summary:

- which user journey it protects, or which concrete exception applies;
- which failure depends on browser behavior;
- why unit or integration coverage cannot prove that behavior;
- what it adds beyond the existing browser scenarios.

Use this qualitative justification rather than test-count quotas or a new test
classification framework. Extend an existing scenario when it can protect the
same journey clearly. Run browser tests with the English UI; translation copy
does not justify an E2E case or a locale matrix. Do not multiply providers,
viewports and engines without a specific risk for each extra case. A test that
only calls the mock and asserts its state without exercising UI belongs below
Playwright.

Keep one representative interaction per distinct production path, including
failure recovery when it protects user data. Several controls reaching the same
cancellation or validation logic do not each need an E2E scenario. Shared
component behavior belongs in its browser scenario; product flows should prove
their integration rather than repeat the component's entire matrix.

Chromium runs the entire lean browser suite. WebKit runs only scenarios
explicitly tagged `@webkit`: a representative core journey and checks with a
specific engine risk. A title mentioning a feature is not a reason to run its
whole test group twice. CI keeps one Playwright worker; reduce unnecessary
browser work rather than increasing concurrency. CI captures traces on the first
retry and screenshots on failure; local runs retain traces on failure because
they do not retry. A CI failure that does not recur has a screenshot of the
original attempt and a trace of the retry, not a trace of the original failure.

Secondary plugin creation/installation forms, quota presentation, issue filters,
sidebar grouping, catalog empty states and preview-panel preferences have no
dedicated E2E gate. Tool selection, MCP comparison, plugin parsing, quota grouping,
Code review defaults and silent alerts have Rust or Vitest coverage. These
lower-level checks do not prove the secondary forms or live CLI installation.
Profile editing retains its UI regression. Layout uses English at representative
viewport sizes; sibling-tab state combinations stay in `workspace_tools.rs`,
with one UI journey through the tool selectors.

Account/settings variations, catalog management shortcuts, header styling, tab
visibility preferences, recent-file ranking, the file viewer's find bar and
Command-P quick open also have no dedicated browser gate. Recent-file collection
and ranking remain covered in `timeline.test.ts` and `crates/files/src/search.rs`; in-file
matching, wraparound and marker markup in `find.test.ts`, and the palette's
keyboard behavior in the shared `e2e/search-picker.spec.ts`; native Git/catalog
rules keep their Rust coverage. Feedback keeps representative submission, attachment races and cancellation checks rather
than repeating every replacement control. Removing a UI scenario does not imply
that its presentation or wiring is proven by a backend test.

Keep contract, security, protocol and backend regression coverage in the cheaper
suites on every CI run. Reducing E2E scope does not remove those guarantees. The
browser mock proves UI integration, not native process lifecycle, filesystem
behavior, real provider execution or deployed services; retain the corresponding
Rust, relay and contract tests and focused native checks.

## Shared Cloud API fixtures

[`fixtures/cloud-api.json`](../../fixtures/cloud-api.json) contains synthetic
HTTP payloads shared with the Rails service. Rust tests in `cloud.rs` and
`catalog.rs` feed them through the production profile decoder and catalog
parser. Cloud's `DesktopContractTest` checks real Bearer-authenticated endpoints
against its vendored copy. Each repository runs independently; desktop CI does
not need access to the Cloud repository.

The fixture covers session profiles, signed-out responses, empty/current/legacy
catalogs, revision conflicts and rejection of credential-bearing catalog input.
It does not replace organization, relay, device-login or feedback integration
tests. These are executable examples of the current wire contract, not a new
API version or a complete schema.

For a paired contract change, update the public fixture and consumer tests here,
then import it into the Cloud checkout and run its producer tests:

```sh
# In prometeu:
npm run test:rust -- shared_cloud_contract

# In prometeu-cloud, with prometeu as a sibling checkout:
bin/contracts ../prometeu
bin/contracts --check ../prometeu
bin/rails test test/integration/desktop_contract_test.rb test/contracts_test.rb
```

Commit the fixture, copied fixture, hash manifest and affected tests in their
respective repositories. Cloud CI runs `bin/contracts --check` to verify its
pinned copy. With a source path, the same command additionally checks that both
repositories use identical fixture bytes. Link the paired PRs and record the
tested revisions; a hash check alone does not prove either server behavior or
deployed compatibility.

## Simultaneous instances

Debug and release use different roots. Each development worktree gets its own
configuration through `scripts/app.sh`; that avoids collisions of `board.json`
and the Vite port. Do not replace that initialization with a direct `tauri dev`
without understanding the isolation.

This repository's `.prometeu/settings.toml` offers the app itself and the mock
as dogfooding scripts.

## Capturing agent fixtures

Synthetic boundary scenarios must identify themselves as synthetic and state
what they exercise. They must not claim a provider version or replace recorded
CLI conformance evidence. The requirements below apply to recorded fixtures.

Protocol fixtures must:

- come from real output of the CLI version stated in the test;
- remove personal prompts, user paths, tokens and credentials;
- preserve the ids and ordering the scenario needs;
- contain the smallest set of frames that reproduces the behavior;
- record the provider, the CLI version and the capability proven;
- never depend on the network during the suite.

A CLI update that breaks a fixture is a signal to review the adapter and the
contract, not to delete the assertion until the test passes.

## Interface text

Portuguese is the source catalog in `src/i18n.pt.ts`; English implements the
same keys in `src/i18n.en.ts`. TypeScript uses `t`/`tn`; Rust emits codes that
the frontend translates with `fromBack`.

Agent output, terminal output and text provided by the person are not
translated.

Write test names, helpers, comments and authored fixtures in English. Preserve
contract values and Unicode samples needed to prove parsing or encoding. Tests
cover locale selection, interpolation and structured errors, not translation
copy quality; TypeScript checks that the English catalog implements the source
catalog keys.

## Process adapter checks

`npm run test:process` runs real local Unix subprocess tests for agent spawning,
pipe backpressure, graceful input closure, interruption, shutdown escalation,
exit codes and abandoned-handle cleanup. It requires neither Tauri/GUI libraries
nor installed agent CLIs. It is included in the workspace Rust suite and runs
in an independent Linux/macOS CI job. `npm run test:core` separately checks
shutdown policy with injected controls and keeps its Windows CI coverage.
The same suite now includes real PTYs (input, resize, EOF, exit and group
shutdown) and private authentication pipes (line bounds and cleanup). Its Python 3
PTY fixture explicitly detaches the controlling terminal before closing stdio,
so EOF while the child remains alive does not depend on Linux-only behavior. These
tests do not exercise a Windows shell, WSL bootstrap or live OAuth.

Bounded query/command adapter tests also run in `npm run test:process`. They use
local synthetic children for pipe backpressure, blocked writes, total-output
bounds, nonzero exits and descendants holding pipes. Provider fixtures remain
in the desktop Rust suite; no CLI account or network is needed for these tests.

## Native profile and tool verification

`npm run test:tools` tests native MCP encoding and package preparation through
injected catalogs, private files and installers, without Tauri or installed agents.
It covers manifest/configuration compatibility, cache invalidation, failure cleanup
and shared preparation ordering; a synthetic CLI checks native installer behavior.

`npm run test:profiles` tests the Unix account profile adapter without Tauri,
GUI libraries or installed CLIs. Its fixtures cover shared history, credential
isolation, explicit roots, environment application and preparation failures.
The Linux/macOS native adapter CI job also runs its tests and Clippy. Private
file permissions and injection into startup/login are verified by the desktop
Rust suite; real provider authentication remains a separate manual check.

## Headless conversation verification

`npm run test:runtime` runs the shared Codex adapter and actual headless executable
fixtures without Tauri or a provider subscription. The synthetic provider needs
Python 3.11 or later (`tomllib` is used to inspect provider configuration); CI
selects Python 3.12 explicitly on both Unix hosts. CI matrices retain all platform
results even when a sibling fails. See [run instructions](../contracts/headless-runtime.md#run) for a real
conversation and the current limitations. Real-provider smoke testing uses an
isolated runtime directory and the execution environment's existing Codex login.

## Native Windows/WSL integration

`npm run test:windows:native -- CONFIG.json` exercises the original desk, file
editor, binary viewers and terminal in WebView2 over WSL. Binary acceptance covers
raw Tauri byte responses larger than one WSL message and the local blob CSP.
Attachment acceptance uses real Windows file dialogs and clipboard image/file
data; a synthetic provider reads the chosen file from WSL. The clipboard fixture
restores its previous contents in `finally`, including on timeout. This opt-in
native coverage targets path conversion and OS clipboard boundaries that mocks
cannot validate; shared chips and draft interactions keep their existing tests.
Tool acceptance uses the original picker and has the fixture provider execute a
selected MCP definition's command with its private environment. This proves native
configuration delivery without spending model credits; shared picker/trust browser
tests and runtime selection/resume tests retain responsibility for portable rules.
The default native entry is the shared
interface; `/wsl.html` remains diagnostic coverage for the older preview.
See [current coverage](../contracts/windows-application.md).

The native acceptance config may additionally set `codex` to the absolute Linux
path of an already authenticated Codex executable. This opt-in mode sends two
short live inference requests in an isolated temporary project and checks the
original ChatView before and after window reconnection. Omit `codex` for the
synthetic-provider journey covering projects, scripts and Git. Both modes keep
credentials in WSL and use separate runtime roots; live inference uses the selected
CLI account's service quota. Use a separate `artifacts` directory for each mode. With live `codex`, set
`bootstrap: true` to exercise default WSL startup without a setup screen, ordinary project selection,
embedded runtime installation and saved automatic reconnection. This mode injects
an isolated `PROMETEU_WINDOWS_RUNTIME_ROOT` and reuses its private WebView profile.

Build the Linux runtime in WSL and run `npm run app:wsl` from a Windows checkout.
`npm run build:app:wsl` builds the native executable without an installer or
publishing. Running this command on Linux first builds and embeds the release
runtime. On other build hosts, set `PROMETEU_WSL_RUNTIME` to the matching Linux
artifact; CI supplies it from the Linux job. Without that artifact, automatic startup rejects the incomplete build. Setup, private roots, reconnect semantics and validation limits are
in the [preview contract](../contracts/wsl-preview.md).
`npm run build:wsl` builds only its frontend; `npm run dev` serves the deterministic
`/wsl-preview.html` composition without WSL. `npm run test:bridge` checks the
portable transport, and `test:runtime` includes real-process bridge tests.

`npm run test:wsl:native -- CONFIG.json` runs the opt-in Windows acceptance check
against the built executable, WebView2 and an installed WSL distribution. It
uses a synthetic provider and an isolated root, without a provider subscription.
See the [native acceptance instructions](../contracts/wsl-preview.md#native-windows-acceptance).
It stays outside the default browser suite: hosted CI's Windows build has no
configured WSL distribution, and browser mocks cannot prove native IPC, Windows
argument encoding or execution surviving closure of the actual window.
