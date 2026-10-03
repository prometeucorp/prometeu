# ADR 0084 — Shared desktop interface with an injected WSL application transport

Date: 2026-09-30
Status: Accepted

## Context

The isolated WSL preview validated execution, attachment survival and native
WebView2 transport. Expanding its separate screens duplicates the existing
Prometeu interface and postpones product integration. The required outcome is
the existing desktop interface in a native Windows window, with execution in WSL.

## Decision

The Windows build uses the existing `index.html` and imports `src/main.ts` after
connecting to the runtime. A Vite entry transformation selects Windows bootstrap
without copying the HTML or feature screens. `IpcTransport` preserves the existing
typed commands; the bootstrap injects its WSL implementation. The existing Tauri
transport remains the default desktop and browser-mock composition.

Window-local functionality remains native. Native menu composition receives the
application-menu implementation, avoiding unsupported macOS predefined items on
Windows without platform checks in feature screens. Other host-specific effects
will use the same composition principle as they are integrated. Directory selection
uses the existing dialog plugin. An injected `ApplicationPaths` adapter at the
Windows command boundary translates native selections before the existing
`add_project` request crosses WSL. Only the connected distribution is accepted;
Windows drive mounting is delegated to its bounded `wslpath` command rather than
assuming `/mnt/<drive>`. Canonical folder validation remains with the WSL host.
`NativeApplication` composes native application effects through `ApplicationPaths`
and `FileManager`. Existing `reveal_path` requests first resolve and validate in
WSL, returning a private typed location; the native adapter consumes that location
and preserves the frontend's void result. Reverse path conversion remains in
`wslpath`, while Explorer uses the Windows Shell APIs to open directories and
select files. This avoids reproducing file-manager UI or sending execution paths
to a command shell. Windows-incompatible Linux names fail rather than select a
different entry. See Microsoft's [file selection API](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shopenfolderandselectitems).
The shared project registration rules preserve the existing board shape and keep
project removal independent of workspace and file lifetime.

`ApplicationClient` adds the `application.v1` runtime capability and a bounded
command envelope. The runtime explicitly validates supported application commands
and addresses conversation contexts by their stable session IDs. Requests must
not switch a shared active workspace before executing: multiple ChatViews on the
desk require independent commands and streams. The original selected-workspace
preview protocol remains available for diagnostic acceptance.

Application events retain the production `board`, `chat` and lifecycle contracts.
The native shell forwards only allowed event names to its local webview. Unsupported
commands reject; missing services cannot be implemented as success placeholders.

Connection recovery is another injected effect. Shared views subscribe to the
portable `ConnectionRecovery` port; Windows replaces failed attachments and asks
views to restore snapshots without replacing the document or its drafts. The
default desktop source is inert. `TerminalSnapshots` preserves the original byte
buffer adapter while Windows requests an optional sequence-bearing snapshot.
This keeps WSL commands and retry policy out of feature screens. Only connection
establishment and read-only restoration retry; uncertain mutations are never
replayed. Runtime replacement uses the resident's atomic idle admission (ADR 0082).

File access also uses injection: `ProjectFiles` selects native I/O while the
`prometeu-files` implementation is shared with the original desktop. Repository
settings and Git reference-selection rules are extracted from their existing
implementations, preserving fallback and conflict behavior. Tree mutations and
path search also use injected core ports backed by the existing native algorithms.
Recoverable deletion is a separate `EntryTrash` effect; the WSL
composition selects Linux system trash because execution paths belong to that
filesystem. It does not promise Windows Recycle Bin integration or fall back to
permanent deletion. Search caches belong to the injected service rather than a
process-global workspace-ID map, preventing roots from sharing an index.
Binary reads share the same native reader and original 100 MiB admission limit.
The bridge assembles stateless 256 KiB blocks over the existing application
envelope, then the native shell returns the original raw-byte IPC response.
This preserves existing viewers without enlarging every transport message or
adding another transfer channel. Exact block sizes and the existing file stamp
detect incomplete transfers and ordinary changes, but do not provide an atomic
snapshot. Full reads still occupy the native client until completion; asynchronous
transfer scheduling remains outside this integration.
Supporting shell docks are retained independently of conversation selection; only
their native factory is host-specific. No shared screen tests the operating system.

File attachments keep the existing `@path` contract. An injected picker and native
drop adapter translate paths before they enter shared drafts; the launcher uses
the original first-message formatter extracted into core. Files already reachable
through WSL mounts are not copied. Clipboard reading is a separate native port;
the Windows implementation uses [arboard](https://docs.rs/arboard/3.6.1/arboard/struct.Get.html)
for copied files and image pixels, and retains PNGs in the user's app-local cache.
Mapping those paths through the same WSL service avoids adding an upload protocol,
at the cost of requiring the Windows cache mount to remain accessible. It retains
the original path-based attachment semantics and lifetime, including access to
explicitly selected files outside the project. It does not promise embedded
multimodal provider input or a snapshot of later-edited source files.

Git review and actions follow that same composition: `RepositoryGit` and its IPC
models live in core, `prometeu-git` owns the original native commands, caches and
mutation guard, and both hosts inject it. Desktop facades retain Tauri admission;
the runtime resolves the same repository indices from its isolated catalog. No
board or IPC migration is introduced. Native Git configuration and hooks retain
their original behavior, including the existing unbounded process lifetime; moving
slow operations off the request handler remains a follow-up integration requirement.

Workspace lifecycle follows the same split: pure board mutations live in core,
while hosts stop their own contexts and publish through their existing stores.
The original cleanup checks and Git effects are extracted behind `WorktreeCleanup`
and injected into both hosts. Grouping-root ownership remains a host concern;
Windows admits only its currently supported single-repository worktrees. This
preserves existing menus, persisted fields and explicit force confirmation without
a parallel workspace model. Transcript storage remains separate from checkout
lifetime and permits reopening a missing directory only when existing metadata
binds that same path. Retuning replaces provider preparation in the stopped runtime,
retaining its store and identity instead of creating another conversation.

Discovery follows the same boundary: portable descriptors and `ProviderDiscovery`
ports, shared bounded model query protocols, and host-owned executable/profile
selection. The WSL composition initially registers the installed Codex external
account, with an isolated persistent registry; managed login profiles remain
pending. External attachment is advertised as a capability and never silently
reactivates a removed selection. This reuses existing credentials without copying
them to Windows, at the cost of managing login in the WSL CLI for now.

The existing launcher `Draft` is shared in core. Supported creation modes preserve
the chosen configuration and initial prompt before startup. Unsupported options
reject. Setup uses the existing script/environment/copy implementation through an
injected preparation port and terminal factory. The runtime event loop waits for
setup completion and provider readiness without blocking application requests,
then sends the retained first message. Failed setup uses the original desktop's
warning policy; failed process creation retains the prompt and workspace. Run
uses the same independent dock lifecycle and retains stopped logs. This intermediate coverage does not waive full
application integration; see the contract's remaining gaps.

Startup follows the same dependency boundary. The Windows composition injects
default WSL environment discovery, bounded command I/O, path import and runtime installation.
The Windows binary embeds a matching Linux executable supplied by the build; the
installer verifies content and loader compatibility before atomic publication in
its private user directory. This avoids runtime downloads and developer-entered
paths, at the cost of distributing one Linux architecture/libc build. There is no connection or distribution-selection screen. WSL itself selects its
default distribution; an injected empty catalog seed leaves project selection in
the original desktop flow. Existing catalogs and matching-distribution data roots
are preserved; a changed default uses its own private state. Linux host processes and credentials never move into the Windows UI.

The persisted WSL root, rather than the WebView cache, owns its original working
directory. Bootstrap reads that identity through an injected `RuntimeRoots` port
before attachment. This adds one bounded native read but keeps installations and
fresh Windows profiles compatible with earlier roots, including those bound to
`/`. It neither migrates metadata nor relaxes the runtime store's strict identity
and ownership checks; malformed or unsupported metadata still fails explicitly.

Windows packaging reuses Tauri's standard per-user NSIS installer. The app takes
its version from the root package and retains its existing bundle identifier, so
reinstallation keeps the WebView profile. The installer owns Windows files and
shortcuts; it does not manage WSL project or state directories. The initial package
uses manual installer updates instead of adding another updater path. Public
signing and automatic updates remain separate distribution work. Native acceptance
reinstalls the package between two real Codex turns and checks conversation recall.

Local resource libraries follow the same boundary: the desktop and WSL hosts
share skill validation, package materialization and catalog response shapes from
`prometeu-tools`; file roots and registry effects are injected. Windows composes an
explicit disconnected sharing source until Cloud is integrated, so local authoring
works without implying synchronization. Saving a library item does not activate it
in a provider. Existing desktop Cloud publication and cache-cleanup effects stay
in its adapter, preserving their ordering and compatibility.

Tool activation composes the existing startup services rather than defining a
Windows tool protocol. Pure selection/provenance/trust rules live in
`core::tool_resolution`; repository declaration identity and hashing live in the
shared files adapter. A runtime `ToolSelection` reads the persisted catalog at
each spawn, and injected `StartupTools` reuses native package and MCP preparation.
This keeps changes to an active conversation deferred until its next process
start, at the cost of a catalog read during preparation. Both hosts use the same
reserved marketplace namespace so deselection disables prior managed packages
without changing personal CLI configuration. Existing IPC and persisted formats
are retained. WSL initially supplies local catalogs and external Codex profiles;
built-in delegation and agent-generated plugin creation remain separate work.

Plugin import and management also compose the original implementation through
`prometeu-tools::plugins::Plugins`. `PluginLibrary` receives roots, read catalog,
private writes and a bounded command runner. The desktop facade retains Cloud
publication and cache cleanup; WSL supplies local services and handles the existing
IPC shapes. Windows path conversion stays in `NativeApplication`, with no platform
branches in the Resources UI. Git commands are bounded at twenty seconds to fit the
existing bridge reply deadline; the trade-off is explicit failure on long downloads
and synchronous handling while Git runs. Plugin Git commands do not use deferred operations.
Canonical ownership checks now protect removal and marketplace discovery in both
hosts; manual sources and shared directories are retained. Formats are unchanged;
real-Git runtime tests, original desktop fixtures and native UI acceptance provide
compatibility evidence.

MCP authentication follows the same composition rule. `prometeu-oauth` extracts
existing PKCE, callback validation and completion-page mechanics without desktop
dependencies. `prometeu-tools` shares authorization, private credential persistence,
CLI discovery and connection checks; inspection receives the existing bounded
query port. The desktop keeps its browser adapter; Windows injects native browser
consent and owns the loopback listener, while WSL retains the verifier and tokens.
This avoids reliance on WSL loopback forwarding for authentication callbacks.

Consent and multi-request HTTP discovery can exceed the ordinary thirty-second
transport deadline. The runtime therefore exposes a bounded, in-memory deferred
operation adapter for MCP checks/authentication; the bridge preserves the existing
application command result and releases its connection lock between requests.
No general task system or alternate UI is added. The trade-offs are polling,
explicit rejection by older residents, and loss of unfinished consent on runtime
restart. State is consumed once and uncertain exchanges are never replayed.
Compatibility tests exercise the original persisted format, shared desktop
facades and a real runtime with hermetic HTTP and subprocess effects.

The same bounded adapter now carries Git operations and launcher checkout preparation,
negotiated through `application.operations.v1`. The core separates admitted immutable
drafts, native preparation and catalog commit; only the owner commits, against the
current catalog, including while detached. This avoids cloning mutable host state
or placing a long-held mutex around it. Older residents retain synchronous commands;
an uncertain start/poll never triggers fallback. The trade-offs are bounded in-memory
results, polling and explicit busy errors for destructive lifecycle operations while
work runs. Shutdown refuses running effects so normal retirement cannot release root
ownership under them. Initial hydration also uses the worker and produces a prepared
Setup command/header consumed once by the host. `application.initialization.v1`
adds deferred model/account discovery and manual Setup through the same adapters,
keeping old-resident command selection explicit. Copying remains the shared native
implementation; normal terminals no longer read script declarations. Provider-start
and new-tab model validation effects remain on the owner.

## Consequences

The runtime's application command coverage must grow to match the real desktop
flows. Opening the shared shell is not feature parity. The existing desktop,
browser mock, runtime and native Windows journeys remain separate validation
boundaries. The isolated preview stops receiving product screens; its tests
remain useful for execution/transport diagnostics.

See the [Windows application contract](../contracts/windows-application.md).
