# Windows desktop with an injected WSL runtime

Date: 2026-09-27
Status: In progress. The portable board foundation is implemented in
[ADR 0085](../../decisions/0085-portable-board-core.md), and conversation ordering,
replay, snapshots and background settlement now use injected ports under
[ADR 0064](../../decisions/0064-portable-conversation-stream.md). Process supervision
now uses core ports and the Tauri-free Unix adapter under
[ADR 0065](../../decisions/0065-injected-process-supervision.md). Session
queues, reactions and observation ordering now use injected effects under
[ADR 0066](../../decisions/0066-injected-session-coordination.md). Terminals and
private authentication subprocesses use injected ports under
[ADR 0067](../../decisions/0067-injected-terminal-and-private-processes.md).
Bounded catalog queries and finite command helpers use
[ADR 0068](../../decisions/0068-bounded-command-and-query-ports.md).
Session registry ownership and service composition now use
[ADR 0069](../../decisions/0069-portable-session-host.md). The conversation pump
uses injected reactions and scheduling under
[ADR 0070](../../decisions/0070-portable-conversation-pump.md). Shared launch
settings and restart coordination use
[ADR 0071](../../decisions/0071-injected-session-launch.md). Initialization and
output workers now use an injected executor under
[ADR 0072](../../decisions/0072-injected-conversation-workers.md).
An experimental headless conversation and a separate native WSL shell with an
injected bridge are implemented under ADRs 0079 and 0080. Full Windows support
is not yet claimed; native Windows UI acceptance passed on 2026-09-30 with a
synthetic provider. Packaging and full application parity remain pending.

## Intended result and scope

Run Prometeu's Tauri interface directly on Windows. Keep projects, worktrees,
Git, agent CLIs, supporting terminals and their credentials inside a selected
WSL2 distribution. The Windows desktop owns windows, clipboard, dialogs,
notifications, external links and embedded webviews. WSLg is no longer part of
this interface path.

The first release targets Windows x64 with one selected WSL2 distribution and
one runtime root per desktop instance. Existing macOS and Linux installations
keep local execution. Native Windows agent execution and simultaneous boards
from multiple distributions are subsequent implementations, not prerequisites.
No WSL installation, distribution replacement or project relocation happens
silently. An unavailable selected runtime produces a recoverable error.

The architecture supports another execution implementation without changing
application rules. It does not promise native Windows execution before that
implementation and its conformance tests exist.

## Evidence and existing constraints

- [Dependency rules](../../architecture/dependency-rules.md) already require
  interfaces at variable technologies and explicit composition.
- [ADR 0050](../../decisions/0050-tested-application-boundaries.md) requires
  tested application boundaries before service extraction. Its present
  deployment decision remains in force until implementation changes it.
- `src-tauri/src/main.rs::AppState` combines board, saver, processes, terminals
  and presentation observations. `chat.rs::Pump`, `state.rs::publish`, actions
  and delegation use Tauri directly.
- `platform.rs`, process termination, account setup and embedded MCP contain
  Unix assumptions. Their relocation requires dependency changes, not merely
  adding Windows branches or a packaging target.
- [Persistence](../../contracts/persistence.md),
  [agent runtime](../../contracts/agent-runtime.md),
  [IPC](../../contracts/ipc.md) and
  [embedded MCP](../../contracts/embedded-mcp.md) define guarantees to retain.

Orca provides evidence for separating desktop and execution environments:
[runtime selection](https://github.com/stablyai/orca/blob/27b823f934f739bc85914dd717b776835f60bcf7/src/shared/project-execution-runtime.ts),
[WSL command handling](https://github.com/stablyai/orca/blob/27b823f934f739bc85914dd717b776835f60bcf7/docs/reference/wsl-command-execution.md)
and [real WSL tests](https://github.com/stablyai/orca/blob/27b823f934f739bc85914dd717b776835f60bcf7/.github/workflows/windows-wsl-e2e.yml).
This proposal uses one resident runtime to keep Linux operations together;
it does not copy Orca's implementation or adopt Electron.

## Dependency direction

```text
Desktop presentation
        |
Typed Tauri command adapters
        |
Injected application-facing ports
        +-- Local client ---------> Application core
        |
        +-- WSL client -- bridge --> Application core in WSL
                                        |
                                 Injected effect ports
                                        |
                              Local system/provider adapters

Desktop commands also receive narrow host ports for native UI effects.
```

The composition roots construct concrete dependencies. Features receive only
the ports they use through constructors or function parameters. No global
service locator, ambient runtime singleton, universal `Platform` object or
generic `execute(method, JSON)` API is exposed to application code.

Platform and runtime implementation selection is confined to composition and
adapter registration. Target-specific modules and Cargo dependencies keep Unix
code out of the Windows desktop build. Application rules and presentation
contain no Windows/WSL/macOS dispatch, including equivalent `match` chains.
Normal input validation and domain decisions remain ordinary control flow.
Capabilities describe available behavior rather than serving as disguised OS
identifiers. Provider capabilities stay separate from desktop capabilities.

## Ports and implementations

These names describe responsibilities; finalize method signatures from real
callers during extraction. Split a port only when its consumers need a separate
responsibility. Do not introduce an interface for every helper.

| Boundary | Application-owned port | Initial implementations |
| --- | --- | --- |
| Workspace lifecycle and board snapshots | `WorkspaceService` | Local service client; WSL service client; test fake |
| Conversation start, send, control, resume and snapshots | `SessionService` | Local service client; WSL service client; test fake |
| Supporting terminal open, input, resize and close | `TerminalService` | Local service client; WSL service client; test fake |
| Git and workspace file operations | `RepositoryService`, `WorkspaceFiles` | Local service clients; WSL service clients; test fakes |
| Accounts, tools and provider discovery | Feature-specific service ports | Local clients; WSL clients; test fakes |
| Core persistence | `BoardStore`, `TranscriptStore` | Existing file formats; in-memory test stores |
| Core event publication | `RuntimeEvents` | Local subscriber delivery; bridge delivery; recording test sink |
| Processes, PTYs and environment discovery | `ProcessLauncher`, `TerminalFactory`, `ExecutionEnvironment` | Unix adapters in the execution host; controlled test processes |
| External agent protocols | Existing agent runtime boundary made explicit | Claude, Codex and Antigravity adapters |
| Native desktop effects | `Clipboard`, `Dialogs`, `Notifications`, `ExternalOpener`, `PowerInhibitor`, `PreviewHost` | macOS, Linux and Windows adapters; explicit unsupported implementations where appropriate |
| Crossing desktop/runtime boundaries | `AttachmentTransfer`, `PreviewEndpointResolver` | Local implementation; WSL implementation |

The service clients expose typed use cases, not the runtime's internal stores,
locks or process handles. The local client calls the same application services
that the bridge server calls. Domain validation runs at the authoritative
runtime even when a caller validated first.

Lower-level ports stay on the runtime side. A Git operation executes beside its
repository; it does not turn every subprocess, stat or file read into a bridge
round trip. Runtime-side services own actions, delegation, usage collection,
tool materialization and telemetry as well as chat.

Extract modules with tests before moving them into crates. The eventual build
graph should separate a Tauri-free core/contracts library, local execution
adapters, bridge client/server adapters, a headless runtime binary and the
existing desktop shell. The Windows desktop must build without linking the
Unix execution adapters. Local desktop builds reuse those adapters directly.

## Ownership, paths and migration

The WSL runtime is the sole writer of its board, transcripts, accounts, tools,
telemetry and execution-related state. Preserve the existing Linux roots,
session IDs, absolute Linux paths, provider homes and transcript formats.
The desktop keeps its own host settings and connection profile; it does not
maintain another mutable copy of the runtime board.

Inventory every existing IPC command and persisted file before moving it.
Assign each one an owner, port, event path and compatibility test. This includes
Cloud credentials, collaboration identity, notification settings, feedback,
Linear login, optional evaluation and actions, not only the primary chat path.
Keep existing durable collaboration identity in its current runtime-root store
through the injected `SecurityStore`; do not silently generate a replacement
identity when moving the interface to Windows. E2EE remains in the desktop
collaboration core and existing audience checks remain in force.

Represent new cross-boundary resources with an execution identity and a
runtime path or opaque file reference. A WSL path is never interpreted with
Windows path rules. Translation to Explorer paths belongs only to the reveal
adapter. Git and normal project reads/writes run inside WSL. Existing persisted
paths acquire their runtime context from the selected root rather than a bulk
rewrite of transcripts. Missing execution metadata on old local installations
resolves to their local implementation.

Windows clipboard images, dropped files and captures are imported atomically
into the runtime's private attachment storage before the draft receives an
agent-readable reference. Failed transfers preserve the draft and never send
a Windows-only path to a Linux agent. Download/export does the inverse through
a user-selected desktop destination. Test Unicode, spaces, drive paths,
symlinks, cancellation and interrupted transfers.

Adopting an existing WSL root requires stopping its old Linux GUI first.
The new runtime acquires an exclusive root lock. Older binaries do not honor
that new lock, so rollback and coexistence instructions must explicitly prohibit
running the old app against the same root concurrently. Back up changed metadata
and test upgrade/rollback against current persisted fixtures.

## WSL bootstrap and bridge

The WSL adapter discovers distributions and their Linux architecture, checks
prerequisites and launches a versioned bundled runtime through `wsl.exe`
with explicit arguments. Use `--exec`; reserve stdout for framed protocol data
and stderr for diagnostics. Discover the shell environment once with bounded,
delimited probes; never let profile banners become protocol frames.

Prefer an stdio bridge into a resident runtime reached over a private Unix
socket inside the distribution. The desktop creates no public listener. The
runtime owns its process tree independently of a particular stdio connection.
Private socket permissions and authenticated attachment bind the bridge to the
selected user/root; credentials do not travel in command-line arguments.

Document version negotiation, runtime identity, request IDs, typed error codes,
bounded frames, cancellation, terminal/file byte transfer and backpressure.
Preserve Conversation Events V1 and existing stream sequence semantics.
Reconnect obtains snapshots and subsequent events without missing the boundary
between them. A runtime epoch invalidates stale process handles after restart.

Mutations need operation identities and deduplication/status lookup. A lost
reply must not automatically resend a prompt or create another worktree.
Document each retryable operation and the bounded retention of its receipt;
unknown outcomes require reconciliation rather than blind replay. Terminal
keystrokes are not replayed on reconnect.

Install runtime binaries into versioned directories using assets included in
the signed desktop distribution. Verify bytes after transfer and switch the
active binary atomically. Negotiate compatibility before attachment; never
replace an incompatible running runtime or interrupt its work during an update.
Start with a documented Linux x64 baseline for the Windows x64 release and
report unsupported distro architectures explicitly.

## Lifecycle and desktop integrations

Connection loss detaches a client; it does not mean the agent completed or the
workspace changed stage. A running WSL runtime retains bounded buffers and
persists history. A desktop crash can reconnect. Runtime death or WSL shutdown
ends live processes; reopening restores transcripts and offers normal resume.
An explicit quit uses a typed shutdown operation with the documented drain and
termination policy. Suspend/resume revalidates the runtime and subscriptions.

Embedded MCP runs alongside agents in WSL and retains its current authorization
and delegation rules. Its executable reference resolves through a runtime-owned
MCP launcher instead of the Windows desktop executable. Windows external MCP
hosts can use a packaged stdio forwarding entry point into that same authority;
they do not receive unrestricted desktop control. Preview/navigation requests
remain validated desktop intents and fail clearly when no desktop is attached.

The Windows preview host owns WebView2 integration. A separate endpoint resolver
makes a WSL development server reachable from that host using verified local
forwarding or a bounded tunnel. Do not assume every WSL network configuration
shares working localhost forwarding. Preserve URL validation, page distrust,
media permission policy, capture bounds and explicit add-to-draft behavior.

Browser OAuth, provider login links, screenshots, sounds, notifications and
keep-awake requests use injected desktop effects. Unsupported optional features
report capabilities and structured errors; they do not silently succeed.
Agent credentials and provider configuration stay with the runtime.

Collaboration continues through its existing injected ports. Execution remains
on the owner's machine; the relay receives no new authority or plaintext.
Extending the current Mac-specific owner guarantee to Windows/WSL requires
updating its contract and ADRs with implementation evidence. Desktop disconnect
does not imply that live sharing survives without its existing frontend owner.

## Implementation sequence and exit criteria

1. **Contracts and ownership.** Complete the command/storage inventory, define
   narrow service ports, root ownership and bridge lifecycle, and add a Proposed
   ADR. Record scope and migration fixtures. Exit: every native command has an
   owner and no ambiguous shared writer remains.
2. **Extract the local application boundary.** Separate board models/storage
   from Tauri publication; inject state, events and effects into workspace and
   conversation services, then terminals, accounts/tools, actions and delegation.
   Keep Tauri commands thin and retain existing local behavior. Exit: these
   services run their meaningful tests without Tauri or an installed agent.
3. **Prove an out-of-process vertical slice on Linux.** Host the same core in
   a headless binary; implement typed bridge clients and server. Exercise load
   board, create workspace, send/control/resume, terminal I/O and snapshots.
   Extend the slice to every inventoried runtime command before shipping.
   Exit: local/bridge conformance and disconnect tests pass on the same fixtures.
4. **Connect a native Windows shell to WSL.** Add target-specific composition,
   bootstrap/install/version negotiation, distribution selection and actionable
   runtime errors. Reuse an existing WSL root and agent accounts. Exit: a real
   Windows build runs the complete core journey without WSLg.
5. **Complete cross-boundary behavior.** Implement native clipboard/drop/dialogs,
   attachment transfer, Explorer/browser opening, login flows, preview routing,
   notifications, power handling, collaboration and MCP forwarding. Exit: the
   capability matrix is accurate and every supported path has native evidence.
6. **Package and validate recovery.** Add Windows installer/updater assets,
   bundled runtime verification, compatible upgrade/rollback, native development
   launcher and CI. Preserve development worktree/root isolation. Exit: clean
   install, update, restart, existing-root adoption and recovery pass on Windows.

Implemented within step 2: board models/publication and the conversation-stream
boundary are used by the local desktop and tested without Tauri. Process
supervision now has core ports/policy and a reusable Unix adapter with real
subprocess tests. Session input/recovery, account handoffs, board reactions and
telemetry/delegation ordering now use portable services with injected effects.
Terminal byte retention/events and native PTY ownership are separated, and
authentication uses private subprocess ports with cancellation/deadline policy.
Catalog queries, naming, background GitHub polling and preparation fetches now
use bounded query/command ports injected by desktop composition.
SessionHost now owns conversations, admission gates, readiness and background
work, and composes SessionService with injected lifecycle effects. Desktop
consumers share that host. SessionPump now owns shared command/output capture,
post-lock reactions and pending-input scheduling eligibility. Desktop context
is supplied through effect ports. LaunchService now owns resume sequencing,
with native preparation and a common provider launcher injected for new and
resumed tabs. ConversationWorkers now own initialization and output/exit ordering
through an injected executor, also used for pending input and shutdown. Native
provider preparation now returns configuration and connection factories without
a desktop handle; shared chat uses canonical input interfaces for provider
policies. Account registry models, selection and serialized updates now live in
the portable core with injected storage and an explicit-root native file adapter.
Login admission, cancellation, revision commits and effect ordering now use core
services with injected native authentication and host effects. Native profile
paths, credential isolation and child environments now live in the Tauri-free
`prometeu-profiles` crate, with a profile backend injected into startup/login and
private file writes supplied by the host. Startup now consumes injected tool
artifacts, and MCP encoding lives in `prometeu-tools` with injected catalog/token
and private-file ports. Package flags, derived homes/manifests and cache coordination
now live in `prometeu-tools` through injected catalog/files/installer ports and a
shared preparation gate. Desktop repository import/update, catalog mutation,
MCP catalog/OAuth/built-in lifecycle, provider validation and home discovery remain.
Delivery checkpoint completed under ADR 0079: `prometeu-runtime` now runs one
headless Codex conversation without Tauri, sharing its protocol with desktop.
Private root leasing, transcript/thread persistence and stop/restart/resume are
implemented. A real provider resumed the same thread and recalled its prior turn.
This is an experimental stdio slice, not the production resident bridge.

ADR 0080 adds the native conversation preview and injected WSL client. Actual
WSL transport and Codex recall are verified; native Windows UI acceptance passed
on 2026-09-30 with a synthetic provider. Resident ownership/reconnect is implemented under ADR 0082; typed transport
errors and operation receipts remain pending. Supporting
terminal access is implemented under ADR 0081. Remaining feature
composition (MCP/OAuth, accounts, telemetry/delegation/actions) then expands parity;
installation/activation boundaries remain required before shipping. Ordinary repository operations remain in their native adapters. The
complete command/storage ownership inventory and deployment contracts remain open.

Each step can span several focused changes. Extraction changes remain separate
from behavioral changes so existing macOS/Linux regressions are attributable.
An early Windows chat demo is a development milestone, not the release gate.

## Validation and architecture enforcement

- Core tests inject fakes for effects; provider fixtures retain V1 behavior.
- Run the same service conformance suite against local and bridge clients.
  Cover queued input, permission requests, ordered board/event publication,
  transcript preservation, cancellation and descendant shutdown.
- Bridge fault tests exercise truncated frames, incompatible versions, unknown
  mutation outcomes, duplicate requests, slow consumers, reconnect races and
  runtime death. No real provider subscription is needed for these tests.
- Build the core/headless runtime without Tauri and the Windows shell without
  Unix adapters. Enforce forbidden dependency edges and platform selection
  outside composition/adapters; document the checker's actual limits.
- Keep existing macOS/Linux CI. Add Windows compilation and a runner that really
  supports WSL2, with explicit participation checks rather than skipped success.
  Real native checks cover clipboard/drop, paths, preview networking, login,
  notifications, installer/update and suspend/resume.
- Follow the existing [E2E scope policy](../../operations/development.md#e2e-scope).
  Browser mocks do not prove WSL processes or native desktop integration. Use
  English for the core native journey and add browser cases only for justified
  browser risks. Run `npm run check` for cross-cutting implementation changes.

Update architecture/dependency rules and the affected IPC, persistence, runtime,
MCP, browser, accounts, collaboration and release contracts with implementation.
Add Windows operations guidance and provider/platform support evidence. Follow
the ADR lifecycle when replacing ADR 0050's deployment decision: retain its
tested-boundary guarantees in the replacement and remove obsolete instructions
and links in the same implementation change. This plan alone changes no ADR.

Release acceptance is a native Windows installation opening an existing WSL
project, preserving its history/accounts, creating a worktree, running an agent,
answering a request, interrupting/resuming, using terminals and Git, importing an
attachment, previewing the app and recovering after desktop/runtime interruption.
Existing macOS/Linux behavior must pass its regression gates as well.

## Native conversation preview checkpoint

Implemented the isolated native shell, typed injected client and WSL launcher,
with shared conversation presentation, explicit reconnect/resume and private
preview roots. The real Linux host passes bridge lifecycle tests; Windows CI
builds the shell without Unix execution dependencies. See
[ADR 0080](../../decisions/0080-native-wsl-conversation-preview.md) and the
[run instructions](../../contracts/wsl-preview.md).

Milestone 2's native Windows UI gate passed on 2026-09-30: embedded WebView2,
native IPC, conversation requests, terminal input, disconnect/reconnect and actual
window closure/reopening were exercised by `scripts/test-wsl-native.mjs` with a
synthetic provider. Actual WSL transport, Codex recall and resident reconnect also
passed their separate checks. ADR 0081 supplies the terminal;
ADR 0082 preserves the live generation and shell across client detach. Full
feature parity and installation/update remain required before release.

Supporting terminal checkpoint: real WSL PTYs now run beside the conversation
through injected terminal ports, with raw bytes, resize, bounded output credits,
exit status and observed cleanup. ADR 0082 adds reattachment while the host remains alive. See
[ADR 0081](../../decisions/0081-wsl-supporting-terminal.md).

Workspace integration checkpoint: ADR 0083 adds saved existing-folder workspaces
using the shared board models and injected catalog/folder/context ports. Each
workspace has an independent conversation and shell; selection preserves running
contexts and per-workspace drafts. Git worktree preparation, multiple tabs and the
remaining production shell features are still open, not implied by this checkpoint.

### New-branch worktree integration (2026-09-30)

The native workspace flow now prepares Git worktrees through `WorkspaceWorktrees`
and an injected Unix command runner. New branches start from a chosen local ref;
source checkout and dirty files remain intact. Catalog saves retain repository,
branch and checkout metadata. Failed or uncertain commits preserve paths for
explicit recovery. The native acceptance flow creates and resumes its second
conversation in that checkout. Existing-branch selection, setup scripts, cleanup
and multiple tabs remain outside this checkpoint.

### Direction correction: reuse the production desktop interface

The user confirmed that modularity means using the existing Prometeu interface
with injected Windows/WSL implementations, rather than rebuilding feature screens
in `src/wsl/`. Freeze expansion of the isolated preview UI; retain it as transport
and lifecycle acceptance coverage until the production interface replaces it.
The completed runtime, bridge, workspace and worktree adapters remain reusable.

The next integration must start from the real `src/main.ts` composition and reuse
`sidebar`, `desk`, `workspace`, `chat`, `launcher`, `dock` and their existing
contracts. `src/ipc.ts` already centralizes typed commands, but imports Tauri
directly; native event subscriptions and file dialogs also reach presentation.
Introduce typed application/host ports at those boundaries and select concrete
implementations once at bootstrap. Windows owns its window, menus, clipboard,
dialogs and notifications; the WSL adapter owns repository files, Git, providers
and PTYs. Browser mock and the existing desktop retain implementations of the
same interfaces. Do not add OS switches to feature screens.

The runtime currently addresses the selected conversation, while production
commands and events identify workspaces and tabs explicitly. Extend the runtime
contract to preserve those identities before connecting the production desk;
a hidden workspace switch before each command would race background conversations.
Native file selection also needs explicit Windows/WSL path conversion at its
adapter boundary. Loading the production HTML alone does not implement its
required services. Missing operations must remain explicit capabilities/errors,
never fabricated successes or another copy of the screen.

Acceptance is the existing desktop workspace/conversation journey running in the
native Windows executable through injected WSL services, with the existing
desktop composition still passing its checks. Feature parity follows through
those shared screens and services, not additional preview screens.

### Shared-interface integration checkpoint

The Windows executable now loads the existing HTML and `src/main.ts` through an
injected command transport/menu. Real WebView2 acceptance has sent a message from
the desk, edited a WSL file in the original editor, and run a shell in the original
terminal tab. Runtime integration proves multiple conversation tabs, isolated
snapshots, close/restart retention, file conflicts/containment and independent
shells. The original desktop file/settings suites pass against the extracted
shared adapters. `wsl.html` remains a diagnostic route, not the application's
window default. User-facing installation artifacts have not been replaced with
this incomplete integration.

The next product gap is the original launcher: connect actual provider/account
and model discovery, then its existing create-workspace contract, using the saved
catalog and worktree ports. Resource hubs and native Windows path/dialog effects
also need their real implementations. No fabricated successful responses have
been added. Full coverage remains explicitly tracked in the Windows application
contract; the active goal is not complete.

### Original launcher checkpoint

Shared provider descriptors, bounded model queries, external Codex identity probes
and a private account registry now serve the original Windows interface. Account
removal/selection and explicit external reattachment persist across runtime restart.
The original launcher creates in-place work or a new-branch Git worktree with its
selected model/effort and initial prompt; the native acceptance exercised that
workflow and verified the source checkout's dirty file was preserved. Initial
launch failure retains a visible failed workspace and saved prompt. Managed OAuth,
additional providers and unsupported launch options remain open. Setup/Run
coverage is described in the following checkpoint.

### Setup and Run integration

The original Setup/Run controls now execute in WSL. Shared native preparation
preserves copy-without-overwrite behavior, stable port allocation and compatible
script environments. Setup and provider readiness defer initial input without
blocking the host request loop; detached hosts continue preparation. Stopped
terminal logs remain available, and named Run conflicts reject explicitly.
Runtime integration covers early-input admission, detached setup failure/warning,
first-message release and named Run lifecycle. Desktop script tests cover the
shared extraction. Native acceptance extends the original launcher journey with
ignored-file hydration, setup ordering and Run start/stop.

Remaining creation work includes asynchronous Git/hydration/catalog effects and
recovery for interrupted first input. Other immediate product gaps are project selection/path conversion, resource hubs,
provider coverage and installation/bootstrap.

### Shared Git integration

Core owns the existing Git IPC models and workspace/agent admission; both desktop
and the WSL host inject `RepositoryGit`. The original native implementation and its
21 real-repository tests moved to `prometeu-git`, retaining caches, mutation locking,
stage/commit semantics, conflict handling and path protections. Runtime acceptance
covers command transport, staged-only commits, stale-index refusal, invalid paths,
history/branches and restoring deleted files. The Windows journey now exercises
the original Changes controls and verifies resulting commits and preserved edits
on WSL disk. Slow Git process execution still needs asynchronous host handling.
