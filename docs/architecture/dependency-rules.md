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
   Local Git reading marks and [review notes](../contracts/diff-review.md) are
   explicit presentation-store exceptions; their storage adapters stay outside components.
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

`src-tauri/crates/core/src/workspace_tools.rs` validates tool selections and changes one
workspace axis using explicit board state. It does not access process handles
or mutate tabs. `session.rs` reads the native IPC body, preserves absent versus
null arguments, translates validation errors and publishes the board.

Saving a selection is allowed during a turn; existing processes keep their
captured tools until they stop. The new selection applies at the next spawn or
resume of a stopped process. Tests exercise validation and preservation of
sibling tabs without `AppHandle`, `AppState`, a saver or a provider process.
The board models and this use case live in `prometeu-core`, independently of
Tauri. `BoardStore` and `BoardEvents` are injected at the publication boundary;
file storage and desktop delivery stay in adapters. Core dependencies and known
native API/OS-dispatch tokens have conservative guards, and CI builds/tests the
core on Linux and Windows without GUI libraries. See the limits in the
[core contract](../contracts/application-core.md) and
[ADR 0063](../decisions/0063-portable-board-core.md).

The WSL workspace catalog reuses those models through injected `CatalogStore` and
`WorkspaceFolders`; provider selection is supplied by composition. Its native
host opens execution contexts through `ContextFactory`. `prometeu-bridge` imports
the portable core catalog type, with no Unix adapter dependency. Frontend
`src/wsl/workspaces.ts` consumes only an injected port and the session coordinator;
the architecture checker protects that portable boundary. See
[ADR 0083](../decisions/0083-wsl-workspace-catalog.md).

The same library owns conversation ordering, replay, snapshots and background
settlement. `ConversationInput`, `TranscriptStore`, `ConversationEvents` and
`Clock` separate these rules from providers, native files, Tauri and host time.
`chat.rs` composes the implementations and preserves the existing lock order;
the session service below coordinates cross-feature reactions. See
[ADR 0064](../decisions/0064-portable-conversation-stream.md).

`ProcessLauncher<Request>`, `ProcessControl` and `ProcessWait` now separate
process effects from the core shutdown policy and process identity. The Unix
implementation lives in `prometeu-process`, depends inward on the core and
never imports Tauri. Its native request is the prepared provider command; it is
not a bridge payload. `main.rs` injects the launcher. See
[ADR 0065](../decisions/0065-injected-process-supervision.md).

`SessionService` owns input queues, account handoffs and board updates through
separate runtime, account, publication and diagnostic ports. `SessionReactions`
adds context, action and usage effects; `SessionOutput` orders execution
observations and telemetry around transcript delivery. `chat/host.rs` composes
the native implementations. No provider or platform dispatch enters these core
services. See [ADR 0066](../decisions/0066-injected-session-coordination.md).

`TerminalFactory`, `TerminalControl`, `TerminalWait` and `TerminalEvents` separate
terminal bytes and lifecycle from native PTYs and Tauri. `AuxiliaryProcess` and
`AuxiliaryLauncher` isolate private authentication pipes from their cancellation
and deadline policy. Implementations live in `prometeu-process`; terminal factory
selection lives in `main.rs`, private launcher selection at the provider edge.
Catalog and finite commands use the separate ports below. See
[ADR 0067](../decisions/0067-injected-terminal-and-private-processes.md).

`QueryLauncher`/`QueryProcess` expose bounded private lines for model discovery;
`CommandRunner` exposes bounded finite command results for naming, preparation
fetches and action-monitor GitHub queries. The desktop composition root injects
both Unix implementations. Feature adapters retain native command construction,
protocols, output policy and error interpretation. Generic request types are
local interfaces, not serialized arbitrary-command APIs. See
[ADR 0068](../decisions/0068-bounded-command-and-query-ports.md).

`SessionHost<C>` owns each host's conversations, admission gates, readiness and
background work. `HostedConversation` supplies native observations and retirement;
`SessionLifecycle` supplies setup, revival, delivery and stop effects. The core
composes `SessionService` using those ports, with no desktop state lookup.
`AppState` owns one host and `chat/host.rs` supplies its effects. See
[ADR 0069](../decisions/0069-portable-session-host.md).

`SessionPump<T>` composes command/output capture and post-lock reactions through
`PumpReactions`, `PendingInput` and `PumpDiagnostics`. Provider context and native
feature implementations stay in `chat/host.rs`; pending input uses the injected
executor described below.
The core owns sequencing and scheduling eligibility, not an executor or a
platform selector. See [ADR 0070](../decisions/0070-portable-conversation-pump.md).

`LaunchService` coordinates restart against explicit board and session-host
state. `ResumePreparation`, `ConversationLauncher` and `LaunchEffects` keep
native configuration, provider registration and publication at the edge.
`Launch` settings are shared core types; provider CLI details do not enter them.
New tabs and resumed tabs use the same launcher composed in `session/launch.rs`.
See [ADR 0071](../decisions/0071-injected-session-launch.md).

`ConversationWorkers` owns initialization, translated output consumption and
wait-before-exit ordering. `TaskExecutor` supplies independent background
execution; `WorkerLifecycle` supplies diagnostics and identity-guarded closure.
The native `ThreadExecutor` is injected at desktop composition for readers,
pending input and shutdown. Provider translation/filtering remain at the edge.
See [ADR 0072](../decisions/0072-injected-conversation-workers.md).

`ProviderPreparation` and `AgentInput` keep native configuration and protocol
policies at the edge. `agent_launch.rs` owns provider registration; `main.rs`
injects its implementation. Provider preparation does not receive a desktop
handle or launch the conversation; shared chat code consumes the prepared
command and connected interfaces. Native account/tool modules still require
further extraction. See [ADR 0073](../decisions/0073-injected-provider-preparation.md).

`AccountRegistry` owns recognized account selection, revision lookup and
serialized updates through `AccountStore`. Its models preserve unknown provider
entries without exposing them to account consumers. `account_store.rs` supplies
private files at an explicit root; `accounts.rs` retains native profiles, login
and the desktop singleton facade. Core registry instances share no global state.
See [ADR 0074](../decisions/0074-injected-account-registry.md).

`LoginState` owns per-host login admission and cancellation. `LoginService`
coordinates registration, working-turn checks, revision commits and quota/event
ordering through `AccountAuthentication` and `LoginEffects`. `account_login.rs`
implements native authentication, injected by `main.rs`; the desktop owns its
blocking executor and invokes cleanup after completion or executor failure.
Provider/method validation remains at the adapter edge. See
[ADR 0075](../decisions/0075-injected-account-login.md).

`prometeu-profiles` is a Tauri-free Unix adapter crate. `ProfileBackend` exposes
native profile resolution, preparation and child environment application;
`ProfileFiles` supplies private directory/file writes. `main.rs` injects the
backend into provider startup and native authentication; `account_profiles.rs`
composes roots and the existing file writer. Native paths/commands stay outside
the portable core and any bridge payload. See
[ADR 0076](../decisions/0076-injected-native-profiles.md).

`prometeu-oauth` owns shared PKCE, callback validation and completion-page mechanics,
with a `Consent` port for host browser effects. It has no desktop or execution-host
dependency. `prometeu-tools::mcp_auth` shares authorization with injected private
storage; `mcp_probe` injects `QueryLauncher` for bounded native inspection. Windows
consent runs in the native shell; tokens and the verifier remain execution-side.
Runtime deferred operations and bridge polling are transport composition, not UI
or core policy. See [ADR 0084](../decisions/0084-shared-windows-desktop.md).

`prometeu-tools` owns native startup-tool ports, artifacts and MCP encoders.
`StartupTools` is injected into provider preparation; `McpSources` and `McpFiles`
supply catalog/token lookup and private writes. Desktop `tool_materialization.rs`
and runtime `tools.rs` compose the shared `NativeTools` with native package and
MCP source/file adapters. Core `tool_resolution` owns pure selection/provenance/trust;
`ToolSelection` supplies current choices at WSL spawn. Repository declaration
reads and hashing belong to `prometeu-files::tool_declarations`. Provider
startup must not import desktop MCP/plugin modules directly. Vendor fields,
paths and derived homes stay in native adapters, outside the portable core.
See [ADR 0077](../decisions/0077-injected-startup-tools.md).

`NativePackages` implements `PackageBackend` with explicit roots/namespace and
injected `PackageCatalog`, `PackageFiles`, `PackageInstaller` and shared preparation
gate. Native manifests, cache versions and derived homes belong to the tools
crate. `PluginLibrary` shares source inspection, catalog mutation and repository
import/update through explicit roots, catalog, file and command ports; desktop
commands retain Cloud publication, cache cleanup and agent-generated creation. File errors are raw causes at this port; installer
errors are already encoded. See [ADR 0078](../decisions/0078-injected-native-packages.md).

`prometeu-protocols` holds the shared Codex protocol adapter, accepting host
language selection and client version. It depends only on the portable core and
JSON encoding. Canonical usage types live in `core::conversation::usage`;
telemetry database effects remain desktop-owned. The Codex architecture check
follows the shared implementation.

`prometeu-runtime` composes one experimental conversation through provider
preparation, `SessionStore`, runtime events, process launcher and task interfaces.
It has no Tauri dependency and uses an isolated leased root. This executable
validates execution; it is not the production WSL transport or complete backend.
See [ADR 0079](../decisions/0079-headless-conversation-slice.md).

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
- `timeline.ts`, the review rules/message codecs (`review-comments.ts`,
  `review-context.ts`, `message-context.ts`), relay `logic.ts` and relay `protocol.ts`, plus their runtime
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

## Experimental native WSL shell

`prometeu-bridge` exposes typed `SessionClient` operations and injected
`RuntimeConnector`, `RuntimeLauncher` and event delivery. It links no Tauri or
Unix execution crate. `prometeu-wsl-desktop` composes the WSL adapter;
`src/wsl/session.ts` receives `SessionPort` and has no platform dispatch or IPC
imports. The native entry injects Tauri, the browser preview injects a mock.
Only the launch adapter has target-specific process flags. See
[ADR 0080](../decisions/0080-native-wsl-conversation-preview.md) and the
[preview contract](../contracts/wsl-preview.md).

The preview's `TerminalClient` and `TerminalPort` are separate from conversation
ports. Only connection composition combines them. Runtime terminal preparation,
factory, scheduling and publication are injected; the pure presentation
controller receives `TerminalScreen` and never imports xterm or Tauri.
The native shell still links no Unix execution crates. See
[ADR 0081](../decisions/0081-wsl-supporting-terminal.md).

Resident execution uses the same request `Host` with injected runtime events.
`ResidentWslLauncher` selects the disposable proxy at native composition,
`AttachmentClient` separates detach from shutdown, and Unix socket ownership and
bounded delivery remain in the runtime transport adapter. Presentation restores
snapshots through its existing narrow ports. See
[ADR 0082](../decisions/0082-resident-wsl-attachments.md).

Workspace Git preparation is a `WorkspaceWorktrees` port in the portable core.
The Unix runtime injects `GitWorktrees` and a bounded command runner; the typed
bridge and presentation pass requests without filesystem or process imports.
Existing-folder creation and worktree creation have separate operations, avoiding
platform or creation-mode dispatch in the catalog rules. See the
[workspace contract](../contracts/wsl-workspaces.md#git-worktree-creation).

## Shared Windows application

`connection.ts` owns the portable view-restoration subscription port. Its default
implementation is inert; `windows/recovery.ts` composes attachment retries.
`terminal-output.ts` injects snapshot reading: the existing native byte-buffer
adapter remains the default, while Windows consumes the optional runtime sequence
extension. ChatView, Term and the main composition restore through these ports,
without importing Windows modules or resetting the document. The bridge owns
runtime identity comparison; the resident host owns atomic retirement admission.

`runtime::jobs` bounds resident workers and retained results for MCP and application
effects. `runtime::operations` captures injected services and immutable inputs;
completion returns to the owner loop for catalog commit and publication.
`core::workspaces::ApplicationPreparation` has no scheduling or process dependencies.
`bridge::operations` adapts existing commands after capability negotiation; screens
never poll jobs or choose an execution mode. Runtime `PreparedSetup` carries only
prepared script/header data; injected settings and file-copy ports run on workers,
while port reservation and PTY ownership remain on the host. Discovery likewise
uses the existing provider/account ports, without introducing UI scheduling.

`windows::main` opens the default WSL through a native composition command and
loads the original interface. `bridge::bootstrap` receives environment and installer
ports; the application launcher selects an empty `CatalogSeed` at composition. `bridge::wsl_command` owns bounded Windows process I/O.
The shell build embeds a Linux artifact as data; it never links Unix execution
crates. Runtime installation checks content before publishing and keeps prior
state roots independent of executable versions.

`bridge::paths::ApplicationPaths` is injected by the native Windows composition
for directory imports. Shared screens retain the dialog and `add_project` contract;
only the host adapter knows Windows paths and the connected WSL distribution.
`bridge::application::NativeApplication` receives that path port and `FileManager`.
It consumes WSL-validated locations for the existing reveal command; Explorer and
its Shell APIs are implemented only in `wsl-desktop::explorer`. Runtime file access
never starts a Windows process and presentation never selects a platform.
The same application adapter assembles bounded binary blocks through
`ApplicationClient`. Core owns the block type and limits; `prometeu-files` owns
native byte reads and stamp validation; only the Tauri shell converts the complete
result to a raw IPC response. Shared viewers retain their existing commands.
`AttachmentPicker` selects file-input adaptation at Windows bootstrap, while
`AttachmentClipboard` is injected into the native application boundary. The
Windows clipboard adapter and event-coordinate translation stay at that edge;
the existing launcher first-message formatter lives in core for both hosts.
`core::projects` owns registration/removal without file or process effects.

`src/windows/main.ts` selects the WSL `IpcTransport` and application menu, then
imports the existing desktop composition. It does not own feature screens.
`runtime::application` dispatches explicit session/workspace commands through
injected context, file, settings and repository-reference ports. Native text I/O, tree mutations, ranked path search
and repository declarations live in `prometeu-files`, shared with the original
desktop; this crate depends on core and serialization libraries, never Tauri or
runtime. Git reference selection is pure core policy. See
[ADR 0084](../decisions/0084-shared-windows-desktop.md) and the
[coverage contract](../contracts/windows-application.md).

Shared discovery types and the launcher `Draft` are portable core contracts.
`prometeu-protocols::catalog` receives private query commands and launchers; it
never selects accounts or imports Tauri. `prometeu-files::accounts` and `private`
implement the existing atomic persistence shared by execution hosts. The Windows
shell links neither native account files nor protocol/process implementations.

Workspace Setup/Run preparation shares `prometeu-files::scripts` with desktop.
`WorkspacePreparation` injects copies and native port probing; script PTYs enter
through `ContextFactory` and `ShellPreparation`. Pending first-input delivery is
owned by the runtime host event loop and keeps the existing desktop warning policy.
No script selection or operating-system branch enters a shared screen.

Git presentation continues to use the existing typed IPC. `prometeu-core::git`
owns those result/input types, workspace admission, pull/discard admission and the
`RepositoryGit` port. `prometeu-git` owns canonical-path caches, mutation locking,
Git execution and native files. Desktop and WSL composition inject the same adapter;
file writes invalidate it through the port, never through another command handler.

The standalone skill library in `prometeu-tools::skills` receives an explicit root,
private-file writes and a package registry. Desktop and WSL use the same validation
and package generation. Cloud publication stays in the desktop composition; the WSL
host injects a local-only `CatalogSource` with the shared response types. Resource
screens never select a platform implementation or read filesystem paths directly.

`ProjectEntries` and `ProjectSearch` are injected in both desktop and runtime.
`prometeu-files::entries` receives an `EntryTrash` adapter; native recoverable trash
stays at the filesystem edge. `prometeu-files::search` owns the existing ranking,
scan policy and a per-service cache keyed by execution roots. Presentation retains
the existing tree menus, rename handling and quick-open/composer search contracts.

Workspace metadata mutations are shared in `core::workspace_lifecycle`. The
`WorktreeCleanup` port separates Git inspection/removal from catalog persistence
and process shutdown. `prometeu-git::cleanup` contains the original desktop checks;
both hosts inject it. `runtime::lifecycle` composes these effects and retains session
stores, while desktop keeps its grouping-root validation. Script environment
construction is shared in `prometeu-files::scripts::workspace_env`.
