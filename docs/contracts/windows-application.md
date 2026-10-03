# Native Windows application

Status: integration in progress under [ADR 0084](../decisions/0084-shared-windows-desktop.md).
The target is the existing Prometeu interface in a native Windows window, with
WSL implementations of project files, Git, providers and supporting terminals.
This contract does not claim full application parity yet.

## Composition

The Windows Vite build transforms only the `index.html` entry script to
`src/windows/main.ts`. After WSL connection it injects `IpcTransport` and the
Windows application menu and imports the existing `src/main.ts`. The same HTML,
sidebar, desk, launcher, workspace and ChatView are used. The isolated `wsl.html`
entry remains available for transport diagnostics.

The Windows bundle inlines small images so nested SVG icons do not depend on
WebView2 custom-protocol image lookup. The native CSP permits same-origin connections for embedded application resources;
WSL transport remains native IPC. It does not enable arbitrary remote connections.

The application command registry remains `src/ipc.ts`; neither screens nor callers
choose Windows/WSL implementations. `application_request` is the native transport
command. `ApplicationClient` sends `{ method: "application", command, args }`
through the existing versioned envelope, requiring `application.v1`. Unknown or
unimplemented application commands reject without mutation. Older runtimes reject
at the capability check before an application request is sent.

The current command coverage is:

| Area | Operations |
| --- | --- |
| Discovery/accounts | `agents`, `agent_models`, `accounts`, `account_select`, `account_remove`, external `account_login` |
| Launcher | `create_workspace` with an existing folder or a new-branch worktree |
| Board/session | `load_board`, `set_stage`, `focus_tab`, `look_at`, `set_lang`, `rename_workspace`, `rename_tab`, `pin_workspace`, `set_unread`, `set_tab_choice` |
| Workspace lifecycle | `archive_workspace`, `finish_workspace`, `remove_workspace`, `cleanup_list`, `cleanup_worktree` |
| Conversation | `chat_snapshot`, `chat_send`, `chat_control`, `new_tab`, `close_tab` |
| Projects | `add_project`, `remove_project` through the existing native directory picker |
| Project files | `list_dir`, `read_file`, `read_bytes`, `file_stamp`, `write_file`, `create_path`, `rename_path`, `trash_path`, `find_paths`, `reveal_path` |
| Shell, Setup and Run | `open_dock`, `close_dock`, `dock_state`, `pty_buffer`, `pty_write`, `pty_resize` |
| Git | `workspace_git_status`, `workspace_git_diff`, `workspace_git_action`, `workspace_git_history`, `workspace_git_branches`, `workspace_git_conflict`, `workspace_git_resolve`, `tree_git_status`, `tree_restore` |
| Plugin library | `plugin_look`, `plugin_save`, `plugin_install`, `plugin_update`, `plugin_remove`, `plugin_scrap` |
| Local resources | `mcp_hub`, `mcp_save`, `mcp_remove`, `mcp_logins`, `plugin_hub`, `skill_hub`, `skill_save`, `skill_remove`, local-only `catalog_state` |
| Tool selection | `set_tools_global`, `set_workspace_mcp`, `set_workspace_plugins`, `set_workspace_skills`, `workspace_tools`, `mcp_inherited`, `project_tools`, `project_tools_trust` |
| Attachments | existing file picker and `paste_files`, native drop adaptation, launcher `inject` and message `@path` references |
| Repository declarations | `workspace_scripts`, `workspace_branch`, `list_branches` |

`chat_send` starts/resumes a stopped context and waits for provider readiness
before dispatch. The observed ready status begins at `session.identity`, matching
input admission; an earlier command catalog event does not make the session ready.
Each workspace admits up to 32 conversation tabs with globally
unique IDs; `new_tab` validates the registered provider, saves before launch and
selects the new tab. Closing a tab stops its process and removes the catalog entry,
while retaining its transcript directory. Failed launch leaves the saved tab
available for recovery. Closed sessions are not addressable through application
commands. Launch model and effort belong to the workspace or explicit tab choice.

Directory selection uses the existing Tauri dialog and `add_project` contract.
The Windows host injects `ApplicationPaths`: matching `\\wsl.localhost\distribution`
and `\\wsl$\distribution` selections become Linux paths; drive paths are converted
by the selected distribution's `wslpath`, respecting its configured mounts.
Conversion has a ten-second deadline and bounded output, uses literal process
arguments, and rejects other distributions, network shares and relative paths.
The WSL folder adapter verifies and canonicalizes the existing directory before
saving. Core project registration is shared with desktop; duplicates return the
existing project. Registration does not create a workspace or launch an agent;
removal unregisters the project without deleting files or existing workspaces.
Project-only views support the same file editor and terminal tabs; Setup/Run
remain workspace operations. Board and persisted project models are unchanged.

`ProjectFiles<Path>`, `ProjectEntries<Path>`, `ProjectSearch` and
`RepositorySettings` are injected native adapters. Tree mutations and ranked path
search use the original desktop implementation in `prometeu-files`, with the same
IPC arguments and results. Both workspace and project-only trees can create files
and folders, rename/move entries and send entries to the execution environment's
system trash. WSL uses its Linux trash, not the Windows Recycle Bin; recovery data
and the original path are retained there. Unsupported trash volumes fail without
falling back to permanent deletion. Parent containment, Git metadata refusal,
symlink-entry handling and case-only rename behavior are shared with desktop.
Mutations invalidate Git caches. Path search retains Git ignore handling, loose
folder exclusions, recent-file ranking and the 40-result limit; files-only filtering
happens before candidate trimming. Each injected search owns its bounded cache,
keyed by execution roots/repositories, with the existing thirty-second refresh
policy. Initial indexing and native tree mutations still run in the request handler;
moving potentially slow effects off it remains pending.

`read_bytes` preserves the existing `{ id, rel }` command and `ArrayBuffer` result.
Both hosts use the shared native reader with canonical containment, regular-file
admission and the original 100 MiB limit. Windows reuses the existing image, PDF
and CSV viewers. Its CSP permits their local image and frame blob URLs, matching
the original desktop; no new viewer or remote resource access is introduced.
The private runtime request adds `offset` (default zero) and optional `stamp`;
its `FileBlock` result contains `data` (byte array), total `size` and `stamp`.
Each block is at most 256 KiB, keeping JSON below the existing reply limit.
The bridge requests consecutive blocks and validates exact length, total size and
the existing modification-time/length stamp before returning one raw Tauri byte
response. Interrupted, malformed or changed reads reject without partial results
or automatic replay. Older residents without this application command reject it;
installing a new executable does not replace an already running resident.
The runtime compares the stamp before and after each block;
this detects ordinary changes, not an atomic snapshot or deliberate same-stamp
rewrites. No transfer IDs or retained file handles are introduced. Reads remain
serialized through the current native client and can delay other commands until
completion.

Attachments reuse the original path-reference workflow. `AttachmentPicker` is
selected at bootstrap: the default implementation uses the existing dialog;
Windows translates its starting directory and selected paths through the injected
`ApplicationPaths` service (`application_paths`, a host-only command). Native drive
paths and the connected distribution's UNC paths become WSL paths before reaching
drafts. No file upload, content embedding or prompt rewriting is needed. Other
distributions and unsupported network shares reject as with project import.
Native drops adapt physical coordinates to logical coordinates and publish the
existing `file-drag` pending/received events, retaining the original target during
conversion. File references outside the workspace remain allowed by the original
attachment contract; their access is governed by the WSL user and provider.

`paste_files` composes an injected `AttachmentClipboard` in the Windows shell.
Its arboard adapter reads copied paths or converts clipboard pixels to PNG, with
a 64 MiB pixel/output admission limit. Screenshots are retained under the Windows
user's app-local `attachments/<uuid>/pasted.png`, inheriting its access controls,
then translated to the selected WSL mount. This keeps persisted references valid
across window closure without sending image bytes through the runtime envelope.
Clipboard images must therefore remain reachable through that Windows mount.
No app-managed automatic cleanup is introduced, matching the original attachment
lifetime. Clipboard failures reject; absence of image/files returns an empty list.
Ordinary text paste keeps browser behavior.

`reveal_path` retains its existing `{ id, rel }` arguments and void result in the
webview. The WSL response is a private `FileLocation` descriptor (`path`, `dir`)
from the injected file service after canonical root containment and existence
checks. `NativeApplication` consumes it, translates through the selected
WSL distribution's bounded `wslpath -w`, verifies the reverse conversion and calls
an injected `FileManager`. Explorer opens directories and selects files using
Windows Shell APIs; file reveal never launches the file's default application.
Missing/outside entries fail before a Windows effect. Linux names that Windows
would reinterpret (for example backslashes or trailing dots) reject explicitly.
Paths and provider execution remain in WSL; the native file-manager effect is the
only Windows responsibility here. The desktop and mock command shapes do not
change, and direct diagnostic runtime consumers see the internal descriptor.

The `prometeu-files` crate shares the original desktop's text limits, canonical root
containment, binary rejection, compare-before-write conflict check and complete
repository-settings fallback. Reading declarations does not execute scripts.
The development transport still limits requests to 1 MiB and replies to 8 MiB,
including JSON encoding; oversized writes reject before sending, and an oversized
read result returns an explicit error without disconnecting. Text reads retain
their 2 MiB limit; bounded binary blocks do not enlarge the transport envelope.
`RepositoryReferences` injects bounded Git queries; reference selection policy is
shared with the desktop launcher. `RepositoryGit` injects the existing native
status/diff/index, history, branches, conflict and tree-restore implementation.
Workspace admission and pull/discard protection use core rules; runtime admission
uses observed agent statuses. Saved files invalidate the shared Git caches.
The Windows shell carries these unchanged commands without repository logic.
With `application.operations.v1`, the native application adapter sends Git review,
mutations, tree status/restore and launcher reference queries through deferred
operations. Git retains the original adapter's process lifetime; this change does
not impose a new command timeout or cancel hooks.

Application shell keys retain the desktop `<workspace>:terminal[-N]` format, with
up to 32 shells per workspace. Internal terminal identities remain private. The
host retains shell services independently of the selected conversation and shuts
all of them down on explicit runtime shutdown. Ordinary window closure leaves
resident shells running. The desktop `Term` contract has no output-credit messages;
these services use detached credit semantics, retain bounded scrollback, and rely
on bounded resident delivery to disconnect a slow client. Setup and named Run docks
use the same keys and existing UI. The Windows transport orders writes per terminal
before native worker scheduling. A failed write rejects its already queued suffix,
without replay; other terminals keep independent queues. A stopped dock retains its output until rerun;
opening an already running Run reuses the process and rejects a conflicting name.
`WorkspacePreparation` injects file hydration and port allocation; the native
implementation, environment construction and localized copy/failure notices are
shared with the original desktop. Scripts run through `/bin/sh -lc` in WSL with
persisted port reservations and the existing Prometeu/Conductor variables.

## Deferred application effects

`application.operations.v1` advertises the additive private commands
`application_operation_start {command,args}` and `application_operation_poll {job}`.
They preserve the result/error of the existing Git, reference and `create_workspace`
commands. The shared UI, IPC registry and browser mock retain their original shapes.
The native adapter selects this path before dispatch; an older resident without the
capability receives the original synchronous command. A failed start or poll never
falls back or replays a mutation.

`application.initialization.v1` separately negotiates deferred `agent_models`,
`accounts` refresh and `open_dock` with `kind:"setup"`. Discovery uses the same
provider ports and returns the same catalog/error and account shapes. Refresh
retains revision checks so a concurrent account edit is not overwritten.
Older residents receive these commands synchronously, even if they support the
previous Git extension. Normal terminal and Run commands retain their transport.

Manual Setup reads declarations and copies files on a worker, then starts the PTY
on the owner. Reopening a live Setup reuses it without copying or executing again.
Concurrent preparation, conflicting writes, Run/input and stopping that Setup
reject with the localized operation-busy error until the worker settles. No implicit
cancellation or retry is added. A normal terminal can open during preparation without
reading script declarations. Accepted preparation continues after window closure.
The same global worker bounds and shutdown/lifecycle protections apply.

The runtime reuses the bounded MCP job adapter: at most eight application jobs and
eight MCP jobs, with completed results retained for ten minutes after completion.
Poll returns `{done:false}` or consumes `{done:true,result}`; failure consumes the
job too. Unknown, expired and consumed IDs reject. Results must fit the 8 MiB reply
budget. Worker failure releases admission and returns an error. A 100 ms native
poll interval releases the connection mutex between requests. The native wait ends
after ten minutes with an uncertain-outcome error; it does not cancel the effect.

Window closure does not stop accepted work. Jobs are memory-only, and runtime death
loses their results; checkout/commit effects may already exist and are never replayed.
The host commits prepared workspaces and publishes through its existing event loop,
not through a client poll. Retirement refuses running or unclaimed jobs. Explicit
shutdown refuses running workers before stopping any session or releasing root
ownership. Destructive workspace lifecycle operations (archive, remove, cleanup)
refuse while any application effect runs, conservatively protecting shared checkout
paths. Setup preparation and Git writes also guard new application input and competing writes against the
same checkout; native Git's existing mutation lock remains in force.

Compatibility and concurrency evidence lives in `bridge/src/operations.rs`,
`core::workspaces` tests, `runtime/src/jobs.rs` and the resident test
`deferred_git_and_checkout_preserve_responsiveness_and_settle_after_detach`.
That test holds real Git hooks while reading the board and using a terminal, then
verifies a single commit and a detached checkout completion preserving another card.

## Launcher and execution identity

The original launcher now sends its existing `Draft` contract, shared in core.
Admission checks provider, selected account, model/effort, stage and supported
options before checkout effects. In-place work and new-branch worktrees persist
the chosen model, effort, issue metadata and initial prompt. The application
launcher's existing unattended policy is stored explicitly as `Auto`; legacy
preview sessions retain `Ask`. Additional tabs inherit the initial tab's stored
policy. Initial file references use the same core `first_message` formatter as
desktop, including quoted paths with spaces and prompts containing only attachments.
Unsupported multi-repository, existing-branch/switch,
plan and custom-instruction options reject explicitly rather than being ignored.

The created workspace is saved and published before provider startup. A launch
failure is recorded on that workspace with its initial prompt retained. Successful
submission clears the saved prompt. There is no automatic replay after an uncertain
write or reply. Repository setup/copy declarations start a supporting PTY, and
provider initialization may proceed while setup runs. The host event loop releases
the saved first prompt only after setup exits and the provider is ready, including
while detached. As on desktop, failed/stopped setup prefixes the message with its
localized warning and keeps the terminal log. Setup launch errors retain the prompt
and mark the workspace failed. Early manual input/new tabs reject during preparation.
The pending launch is removed before dispatch so publication failure cannot retry
an uncertain send. Runtime restart does not automatically replay saved pending input.

The deferred creation path admits the draft on the catalog owner, then validates
the model and prepares the checkout through injected services on a worker.
Completion commits to the current catalog, preserving unrelated edits; it runs even
without an attached window. Capacity and stage are revalidated before persistence.
Failure or an uncertain save retains the prepared directory and reports its path.
Initial file hydration now runs on that same worker before catalog commit, using
`WorkspacePreparation` and the original copy/report behavior. Setup command/header
are prepared once and consumed by the owner, which reserves the port and starts the
PTY before starting the provider. Existing destination files are retained. Copy
failures keep the original Setup report rather than silently discarding the workspace.
Provider process preparation/start, model validation for new tabs and explicit retry
of interrupted launches remain work; Setup execution/readiness are asynchronous.

Discovery uses registered `ProviderDiscovery` ports. The current WSL registration
is Codex, using its actual executable, official `account/read` identity and live
paginated `model/list` catalog. Model queries use the same bounded implementation
as desktop, with a host-supplied client version. Missing, malformed, timed-out and
no-account results remain failures. Structured application failures travel as
`application-error:<JSON>` through the string-based development error envelope;
only the Windows transport decodes them back to the original IPC object shape.

The account registry is private `<runtime-root>/accounts.json`, using the same
portable registry and native atomic file store as desktop. Removal of the selected
account blocks new application input and launches, including after host restart.
Reattaching the external account does not select it; selection is explicit. The
WSL descriptor advertises external account attachment: login credentials remain
managed by the installed Codex CLI, and only identity metadata crosses IPC. Managed
OAuth profiles and switching between additional accounts are not implemented here.
This does not adopt or change a production desktop registry.

Local hub reads use the same file adapters as desktop. Standalone skill creation,
editing and removal use `prometeu-tools::skills::SkillLibrary`, with an explicit
runtime root and injected private-file/package-registration effects. The existing
Resources screen and IPC types are unchanged. Validation, name-collision checks,
`skills.json`, generated Claude/Codex manifests and retained package files match
the desktop behavior. Both hosts retain the existing write order; a failed registry
write can leave materialized files and is returned as an error, never successful
activation. Catalog reads retain their existing empty-on-missing-or-invalid behavior.

The Windows composition injects `LocalCatalog`, an explicit disconnected sharing
service using the shared desktop response types. Local hubs remain independent;
no Cloud caches or identities are adopted, and a save carrying a Cloud revision
rejects before local writes. Cloud login, publication and project installation
remain unavailable. Local MCP save/remove reuses the desktop validation and private
registry format; `prometeu` stays reserved without advertising its unsupported
built-in service. MCP discovery, checks and local OAuth use the shared native
services described below. Agent-generated plugin creation and Cloud remain pending.
Reading or saving a hub item alone does not activate it.
Successful shared-shell acceptance is not full parity.

Plugin management uses the original Resources dialogs and IPC shapes. The injected
`Plugins` port is implemented by the same `PluginLibrary` as desktop: manifest
inspection, private registration, Git/marketplace import, fast-forward updates,
removal and abandoned-clone cleanup. The Windows application adapter translates
native folder sources for `plugin_look` and `plugin_save` using `ApplicationPaths`;
URLs, Linux paths and tilde sources remain execution-side inputs. Returned sources
are Linux paths. Clone addresses pass to Git unchanged after shared normalization.
A non-null Cloud revision rejects before any local write.

Each Git command has a twenty-second deadline and bounded captured output through
`CommandRunner`; the runtime currently handles it synchronously. No request is
replayed after an uncertain reply. Local divergence is preserved on failed update;
manual folders and shared clone directories are retained on removal, and canonical
containment prevents cleanup/marketplace traversal through outside symlinks.
WSL removes registrations and owned sources but leaves installed Codex cache cleanup
to future integration; selected derived configurations rebuild at the next spawn.
The existing opt-in native journey drives Git import/update/removal through the
original Resources UI and registers a Windows UNC source without deleting that
user-owned source on removal. Shared UI rules retain their existing browser tests.

## Conversation tools

Codex startup receives injected `ToolSelection` and `StartupTools` services. Core
`tool_resolution` shares the original global → approved project → workspace rules,
picker provenance, package union and trust decisions with desktop. The files
adapter shares repository identity and canonical declaration hashing. Existing
selection/trust IPC arguments and persisted board fields are unchanged; absent
patch fields preserve state, null restores inheritance and explicit empty
selections disable app-managed tools. Catalog mutations commit before publication;
a failed write retains the previous selection and approval. Launcher tool presets
are saved on their original axes.

The runtime selection adapter reads the current catalog at each provider spawn.
Running and idle processes retain their captured selection; changing a picker
does not restart them. A stopped resume re-resolves the catalog and project hash.
Only exact approved declarations participate; a stale approval request fails.
Codex still has no discovered CLI MCP base. Its picker shows local hub definitions;
undeclared axes preserve CLI defaults. Other providers are not registered here.

Both hosts compose `prometeu-tools::NativeTools`, `NativePackages` and the same MCP
materializer. WSL supplies private roots, its local catalogs and the installed
Codex executable. The existing reserved `prometeu` marketplace and derived
workspace home preserve personal CLI plugin configuration while controlling
app-selected packages; skills use that same package pipeline. Canonical plugin
and hook IDs pass to the existing protocol handshake. Preparation failures prevent
provider spawn and input submission; earlier materialized files may remain. MCP
header secrets use the child environment and stdio secrets use private environment
files, retaining the original encoding and 0600 permissions. Native package
installation retains its existing blocking CLI calls.

`runtime/tests/lifecycle.rs` verifies exact-hash approval, selection changes across
live/stopped sessions, package preparation failure, retained personal configuration
and restart persistence using a hermetic provider. Core catalog tests cover failed
persistence and launcher presets; desktop selection/provenance compatibility tests
remain in place. The opt-in native journey selects an MCP through the original
picker and has its fixture provider execute the configured WSL command with the
private environment. It proves native delivery, not live model tool use.

Requests identify their own session, independent of the preview's selected
workspace. The runtime emits `{ application: { name, payload } }` frames for the
existing desktop `board`, `chat`, `chat-closed`, `pty`, `pty-closed` and `accounts` events. Observed statuses overlay
the catalog's saved board; work stage remains independent. Transcript sequence
numbers and snapshots keep their existing contracts.

## MCP authentication and checks

The original `mcp_found`, `mcp_check`, `mcp_login`, `mcp_logins` and `mcp_logout`
application contracts are unchanged. Read-only import discovery shares the desktop
Claude configuration scanner; it does not change Codex's inherited selection base.
`prometeu-tools::mcp_probe::Inspection` shares HTTP inspection and uses the existing
injected `QueryLauncher` for local MCP processes, with a 25-second deadline and
1 MiB stdout limit. Dropping the query terminates/reaps its process group. HTTP
requests retain their existing per-request deadlines. No provider is needed for a
connection check.

`prometeu-tools::mcp_auth::Authorization` shares discovery, client registration,
PKCE exchange, refresh and logout with desktop. Private storage is injected and
retains the `mcp-auth.json` format, under the isolated WSL runtime root. Refresh
and logout serialize credential mutations. A failed write does not report login
success. Running provider processes retain their startup token; new preparation
reads/refreshed tokens through the existing `McpSources` port. Cloud catalog
revisions and tool activation remain independent from local authentication.

Windows injects `prometeu-oauth::Consent`: it binds `127.0.0.1:17421/mcp` locally,
opens the system browser using a literal HTTP(S) URL, validates callback state and
passes the code to WSL. The verifier and access/refresh tokens stay in WSL. This
avoids depending on Windows-to-WSL loopback forwarding for the callback. The
callback and authorization state expire after five minutes; invalid-state requests
cannot cancel consent. Completion consumes state once; decline or native failure
cancels it. Closing the app does not replay a token exchange on reconnect.

Private transport operations adapt the long-running use case without changing UI
IPC. `mcp_operation_start {operation}` accepts `check {server}`, `begin {server}`,
`finish {state,code}`, `logins` or `logout {id}` and returns `{job}`. Each operation
runs outside the resident request loop. `mcp_operation_poll {job}` returns
`{done:false}` or consumes `{done:true,result}`; failures consume the job and return
the ordinary error. `mcp_auth_cancel {state}` discards a pending verifier. There
are at most eight retained operations; completed, unclaimed results expire after
ten minutes. Pending authorization entries are bounded at eight and expire after
five minutes. Jobs and pending consent are memory-only and do not survive a
runtime restart. An uncertain mutation is never retried automatically.

The bridge polls through bounded requests and waits for browser consent without
holding the native session mutex. Other commands and live events continue. Each
request verifies the captured target, preventing consent from being completed in
a different WSL root after a connection change. Old runtimes lacking the private
operations reject explicitly; no automatic protocol downgrade changes credentials.

Evidence: shared OAuth tests cover callback-state rejection and timeout; tools
tests cover failed persistence, cancellation, expiry and single-use completion;
bridge tests cover refused consent and no replay after an uncertain exchange.
`runtime/tests/lifecycle.rs` uses a hermetic HTTP/OAuth server to verify PKCE,
registration reuse, refresh, 0600 storage, startup encoding, restart and logout,
plus application responsiveness during a blocked stdio check. The opt-in native
journey exercises the original Resources actions and real Windows browser callback.
It does not authenticate an external personal service.

## Workspace lifecycle

The original menus use the original IPC shapes. Core `workspace_lifecycle`
shares board mutations with desktop; WSL commits catalog changes before publishing.
Rename trims blank input, pin/unread change only metadata, and ordinary stage
changes leave execution alone. Model changes reuse `Workspace::retune`: provider
changes and frozen task profiles reject. Only the selected tab stops, retaining
its transcript and provider identity. Its next message resumes with the saved
model/effort; sibling tabs and shells keep running. A failed catalog save leaves
the saved choice unchanged, though execution may already have stopped.

Archive and Finish stop that workspace's agents, docks and pending launch dispatch.
Finish also selects the final stage. Unarchive only restores the card; it does not
start processes. The archive script uses the existing settings and stable script
environment, runs asynchronously with closed standard streams, and is reaped.
Starting the script must succeed before the archive mutation commits; its eventual
exit does not gate cleanup. Files, branches and transcripts remain until a separate
cleanup confirmation. Remove stops execution and removes only the card.

`WorktreeCleanup` injects the original native Git checks and deletion implementation
into both hosts. Cleanup always requires archiving and refuses the original clone.
Dirty or unmerged work requires explicit force selection in the existing dialog;
force does not bypass those ownership checks. Successful cleanup removes the
checkout and workspace-owned branch, sets `cleaned`, and keeps the card/history.
Saved existing branches remain protected. WSL currently admits single-repository
worktrees only; grouping-root validation remains with the desktop host.
Transcript stores live outside worktrees and reopen after cleanup, including after
runtime restart. Metadata must still bind the same directory and provider. New
input, new tabs and new docks reject cleaned workspaces.

Coverage: `core::workspaces` verifies persistence failure rollback and separation
of stage/status; `runtime/tests/lifecycle.rs` uses actual child processes, PTYs and
Git to verify model resume, sibling isolation, script environment, archive/removal,
force admission and history after cleanup/restart. The original desktop cleanup
suite exercises the extracted adapter's merge and branch-preservation rules.
The existing opt-in native journey also drives Rename, Pin and Finish through the
original menus and confirms dirty-worktree cleanup in its original dialog. It
verifies the native command/board-event boundary and actual WSL deletion/history;
no extra generic mock-browser scenario is introduced.

## First launch and reconnection

The native app opens the existing interface without a connection screen, distro
selector or initial project picker. `application_open` composes injected
`WslEnvironment` and `RuntimeInstaller` ports. Discovery invokes `wsl.exe --exec`
without `--distribution`, so WSL chooses its actual default. A bounded login-shell
probe returns `WSL_DISTRO_NAME`, the Linux home, architecture and installed Codex
executable. It does not install WSL or provider credentials.

The application launcher injects an empty catalog seed. A fresh installation has
no projects or conversations; adding a project and creating work happen through
the original desktop controls. The bootstrap home is only an execution directory,
never an implicitly registered project. Existing catalogs take precedence over
the seed and retain their workspaces and transcripts. Diagnostic launchers keep
their explicit initial workspace.

A complete Windows build embeds its matching x86_64 Linux runtime. Preparation
streams that trusted package through a bounded WSL command, validates its SHA-256,
runs `--help` to verify it can load in the distribution, and atomically publishes
it under `~/.local/share/prometeu-windows/runtime/<digest>/prometeu-runtime` with
private permissions. Failed transfer, checksum or loader checks leave the prior
executable intact. No runtime is downloaded at application startup. WSL needs
`/bin/sh`, `sha256sum`, an installed Codex CLI and a libc compatible with the
packaged executable. x86_64 Ubuntu 24.04 is the current native acceptance target;
ARM and a universal Linux binary remain unverified.

New managed state uses `~/.local/share/prometeu-windows/state`; it never adopts the
Mac/WSLg production root. Existing saved roots are retained when preparing the
same distribution. An explicit `PROMETEU_WINDOWS_RUNTIME_ROOT` environment override
supports isolated installations and acceptance tests. Roots still pass the runtime's
ownership and configuration checks.

Window-local storage caches the target to preserve its data root when it belongs
to the current default distribution. An injected runtime-root reader resolves the
original working directory from WSL's existing `runtime.json` before attachment;
that persisted identity takes precedence over an absent or stale window cache.
The reader accepts the existing v1 Codex metadata and performs no writes. Invalid
or incompatible metadata fails without resetting the root. The runtime still owns
locking, permissions and strict identity validation. With no persisted metadata,
only a cache for that same root may supply the directory; otherwise it uses the
discovered Linux home. Every new
window discovers the system default again; changing the default selects that
distribution's own private root and leaves the old data intact. Runtime installation
prepares the bundled executable before attachment. Reloading the webview reuses
its existing connection. Failures appear in the normal application status area;
there is no setup form. Catalog initialization does not change the resident handshake. Existing matching
residents remain attachable; no live process is replaced implicitly.

An interrupted attachment now recovers in the same native window. The native
`application_reconnect` command replaces only a failed connection to its captured
target. It does not rediscover the default distribution, resend input, restart
providers or open terminals. The bootstrap injects `ConnectionRecovery` and
`TerminalSnapshots`; shared ChatView/Term restore their existing snapshots, and
the main composition refreshes board/dock status. File editors and composer drafts
stay in the existing document. Disposed and remote ChatViews do not restore.
Completing workspace entry also respects a shell selected while its conversation
snapshot was pending; late loading cannot replace that shell with the composer.
Explicit diagnostic disconnect/shutdown does not request recovery. Failures while
reconnecting use the existing translated status area and bounded retry delays.

Bundled runtime replacement is negotiated only at attachment using `retire.v1`.
An older host accepts replacement only when it owns no retained execution or
pending application/MCP work or consent. Otherwise the current process continues and the update
is deferred until a later attachment. Residents predating this capability need
explicit shutdown. See the [resident contract](resident-runtime.md) for the private
handshake, terminal snapshot extension, compatibility behavior and limits.

`scripts/wsl-app.mjs` builds the Linux package before the Windows shell when run
on Linux. Other builders supply `PROMETEU_WSL_RUNTIME` as an absolute path to a
matching Linux ELF binary. CI transfers the Linux job's package into the Windows
build. The build validates the ELF architecture and embeds its checksum and bytes;
Windows dependency-only builds without a package remain buildable, but automatic
installation explicitly rejects. `npm run package:windows` requires the package
and builds Tauri's standard per-user NSIS installer. Its application version comes
from the root `package.json`. The stable `co.prometeu.wsl-preview` identifier retains
the existing WebView profile. Installation and reinstallation replace Windows
application files; neither runs cleanup against the WSL state or project roots.
Runtime upgrades still follow the resident negotiation above. Updates currently
require running the next installer; automatic updates and remaining application
services are pending. See [packaging](../operations/release.md#windows-codex-installer).

## Verification

- `src/windows/file-input.test.ts`: picker translation, physical drop coordinates,
  retained pending identities and failure without partial attachment results.
  Bridge tests inject clipboard/path effects without contacting a runtime.
  `core/src/workspace_draft.rs` and workspace/resident tests verify attachment-only
  input, quoted paths and persistence/deferred first input with attachments.
  Native acceptance exercises real file dialogs and the Windows clipboard; its
  synthetic WSL provider reads an attached Windows file. This validates transport
  and filesystem access, not a model's interpretation of image contents. Native
  pointer-driven OLE dragging is not automated; adapter tests cover its event path.
- `files/src/bytes.rs` and `bridge/src/files.rs`: byte preservation, empty files,
  size/offset limits, containment, changed stamps and incomplete/malformed transfers.
  `runtime/tests/resident.rs` transfers a 9 MiB binary through the real resident
  and native application adapter, then verifies subsequent commands still work.
- `tools/src/skills.rs`, `tools/src/catalog.rs`, `runtime/src/resources.rs` and
  `runtime/tests/resident.rs`: shared validation/manifests and catalog wire shapes,
  private writes, collision refusal, local IPC revision admission, host restart and
  removal without deleting files. Existing Cloud browser scenarios cover the shared
  skill editor; no additional browser gate is needed for these backend adapters.
- `src/ipc.test.ts` and `src/windows/transport.test.ts`: unchanged command typing,
  arguments and structured failures; per-terminal byte ordering and rejection of
  queued suffixes after an uncertain write.
- `core/src/git.rs` and `crates/git/src/tests.rs`: workspace/agent admission and the
  original 21 real-repository tests for partial staging, stale commit tokens, literal
  paths, local remotes, conflicts, cache invalidation and deleted-file restoration.
- `bridge/src/bootstrap.rs` and `wsl_command.rs`: default-distribution discovery,
  native package checksum/failure preservation, retained state roots, bounded output
  and cancellation of blocked stdin. Compatibility cases retain v1 roots initially
  bound to `/` across missing/stale WebView storage, reject malformed metadata and
  prevent unrelated root overrides from inheriting cached directories.
  Native acceptance with `bootstrap: true`
  covers direct startup, the empty desk, ordinary project import and reconnection.
  Its optional `existingWorkdir` seeds the earlier root identity before creating
  a fresh Windows profile, then checks discovery and real Codex conversation recall.
- `bridge/src/application.rs` and `paths.rs`: reveal admission/order, private descriptor
  adaptation to the existing void result, unrepresentable-path refusal and native
  effect injection. Resident tests verify root/file resolution and symlink containment.
- `bridge/src/paths.rs` and `core/src/workspaces.rs`: selected-distribution path
  admission, injected drive mounts, project deduplication and failed-save rollback.
- `runtime/tests/resident.rs`: project-only file/shell use and catalog restoration; addressed background conversations and independent
  tabs; persisted account removal/selection, file conflicts and containment; two
  independent shells; detached Setup and deferred first input; Run name conflicts
  and retained logs; Git commands over the real application transport, including
  pull/discard refusal using observed running-agent status.
- `scripts/test-windows-application.mjs`: the original desk/ChatView, editor,
  terminal, launcher, Setup/Run and Changes panel through actual Windows WebView2,
  native IPC and WSL. The journey verifies source-checkout preservation,
  ignored-file hydration, setup ordering, reserved script ports, staged-only
  commit and retained later edits on WSL disk. It also closes/reopens the native
  window with the same conversation/shell and observes explicit runtime shutdown.
  Project import drives the real Windows folder dialog, then the original editor
  and project-only terminal. It also checks actual drive-path conversion, duplicate
  registration, rejection of another distribution, and removal without file or
  workspace deletion. This native-dialog boundary is not covered by mock browser
  tests: the concrete risk is native selection/navigation and path translation
  reaching a different directory from the user's choice. The same opt-in native
  journey verifies the original file menu selects its WSL file in Explorer, project
  reveal opens its directory and mounted-drive files select their original Windows
  location. Shell automation inspects the real folder and selected item, then closes
  only the acceptance run's unique temporary location. This native shell boundary
  has a concrete risk of opening an incorrect path or launching a file association;
  no mock browser scenario can verify those effects. The same journey opens the
  original image, CSV and PDF viewers and verifies a 9 MiB raw `ArrayBuffer`.
  This covers the native Tauri binary response and WebView2 blob/CSP boundary,
  which the browser mock cannot exercise; it adds no generic viewer browser gate.
- `scripts/fixtures/windows-install.ps1`: opt-in native NSIS installation and
  reinstallation, per-user registration and Start menu shortcut target. It refuses
  a different existing installation location or an open native app. The live
  Codex configuration accepts `installer` alongside `executable`; when supplied,
  the journey reinstalls that package between the two windows and verifies the
  same conversation and next Codex reply afterwards. Run only against an explicitly
  chosen local installation; normal application acceptance does not install software.
- `files/src/entries.rs` and `files/src/search.rs` retain the desktop filesystem
  and ranking suites; injected-trash failure preserves the original entry and each
  search host owns its cache. `runtime/tests/resident.rs` exercises unchanged tree
  IPC, collisions, case-only moves, project-only search and Linux trash recovery
  metadata/contents in an isolated XDG data directory. UI menus and moved drafts
  retain their existing unit coverage; no secondary browser gate was added.
- Original desktop file/script suites cover the shared native extraction; IPC
  parity and existing Chromium/WebKit scenarios keep the default host contract.

The default native acceptance uses real files, shells and Git with a synthetic
provider and isolated roots. A separate opt-in `codex` configuration now verifies
two actual inference turns with the installed, authenticated WSL Codex: the first
reply appears in the existing ChatView, and a reply after native window reconnection
recalls the first marker. It passed on 2026-09-30 without webview exceptions or
console diagnostics. This covers external-account inference and window reattachment,
not managed login, provider restart recovery or remaining services. A metadata-only
probe also returned the selected account and nine models without exposing credentials.
Local resource libraries now load through injected services; Cloud synchronization,
built-in delegation and agent-generated plugin creation remain integration gaps.
The development acceptance is not a complete release gate.

Default startup passed native acceptance on 2026-10-01: the original empty desk
opened using the actual default WSL with no setup dialog, the original project
picker and launcher created the first conversation, authenticated Codex answered,
and reopening the native window preserved the transcript and recalled the previous
reply. This uses isolated data, the embedded Linux runtime and the installed CLI.
The native regression also passed the original editor, retained shell, launcher,
Setup/Run, Git review/commit and project import/removal journey on the same build.
It does not imply complete application parity.

The file-tree, Explorer, binary-reader and attachment integration passed the complete
`npm run check` on 2026-10-01: documentation, architecture, formatting,
builds/typechecking, release checks, 588 web unit tests, 648 Rust tests, all
180 browser scenarios and workspace Clippy. Eleven opt-in Rust tests remain
ignored by the ordinary suite. This run used `CI=true` with one browser worker
to reduce local load.

The Windows cross-build and native regression also passed without webview
exceptions or console diagnostics. The original image viewer decoded a 9 MiB PNG,
the CSV viewer showed its cells, and the existing PDF frame rendered the fixture
(also inspected in the captured screenshot). A native IPC assertion verified the
complete large-file `ArrayBuffer`. The original file menu
selected the WSL file in Explorer, project reveal opened its folder, and a mounted
Windows-drive file was selected in its original Windows location. Real resident
integration verifies tree creation, collisions, moves, search and recoverable
Linux trash contents/metadata in an isolated XDG data directory. Existing unit
tests cover the shared menus and moved drafts. These checks do not establish
complete application parity.

Attachment acceptance also passed on the native Windows build: the original
picker selected a Windows file containing spaces and Unicode in its name, and
the synthetic provider read its actual contents from WSL. Native Ctrl+V attached
and sent a clipboard PNG without prompt text, and copied-file paste delivered a
WSL-readable path. The original launcher retained its selected attachment through
Setup and included it in the first provider message. Clipboard contents were
restored after each case. This uses the existing text/path provider adaptation;
no additional claim of live model image interpretation or native OLE pointer
automation is made.

Workspace lifecycle acceptance passed on 2026-10-01 with the updated Windows
cross-build. The native menus renamed, pinned and finished a real WSL workspace;
the existing cleanup dialog kept its dirty worktree unselected until explicit
confirmation, then removed it while retaining its conversation. The complete
native regression finished with no webview exceptions or console diagnostics.
The Rust workspace suite passed 651 tests (11 opt-in tests ignored), and Clippy
passed with warnings denied. All 180 existing Chromium/WebKit scenarios passed
with one browser worker. All 588 web tests passed with `--maxWorkers=1`.
The aggregate `npm run check` stopped at the existing relay lease-renewal test's
timeout under normal parallelism; that test also passed alone. These results do
not establish full application parity or an automatic resident-runtime upgrade.

Tool activation acceptance also passed on 2026-10-01 with the updated Windows
cross-build. The original picker selected a local MCP, and the fixture provider
executed its configured command with the private environment inside WSL. The
complete native regression finished without webview exceptions or console
diagnostics. Runtime integration separately verifies local plugin and standalone
skill activation, exact-hash trust, failed preparation and selection changes across
process restarts. Live model tool use, managed MCP OAuth and built-in delegation
are outside this acceptance.

Plugin library acceptance passed on the native Windows build on 2026-10-01.
The existing Resources dialogs imported and updated an isolated Git repository,
removed its owned registration, then inspected and saved a Windows UNC folder
source. Removing that manual registration preserved the source manifest. The
complete native regression finished without webview exceptions or console
diagnostics. Runtime tests additionally cover multi-plugin marketplaces,
fast-forward refusal, restart persistence and imported-package activation.

MCP authentication acceptance passed on the native Windows build on 2026-10-01.
The original Resources actions checked an isolated HTTP server, opened the actual
Windows browser and completed the native loopback callback. WSL exchanged the
PKCE code, privately persisted and refreshed credentials, checked the remote MCP,
and signed out. A concurrent board request completed while consent was held open.
The complete native regression finished with no webview exceptions or console
diagnostics. This uses a hermetic OAuth service and does not authenticate an
external personal account. No additional generic browser scenario was introduced for OAuth;
the native browser/callback boundary is covered by the opt-in Windows journey.

Attachment recovery acceptance also passed on 2026-10-01. The native journey
started a synthetic provider reply, left an unsent draft and a waiting shell,
killed only the isolated WSL proxy, and released output while detached. The
same document recovered automatically, retained the draft and shell environment,
rendered exactly one completed reply and sent the input only once. The remaining
native regression, including window reopening, finished without webview exceptions
or console diagnostics. It also held a real transcript response until the person
selected the terminal, then verified that completing workspace entry preserved
terminal visibility and focus before typing. Resident integration tests separately prove update deferral
for retained conversations and both terminal types, then replacement of an idle
host with persisted board state. Legacy-capability tests forbid forced retirement.

The aggregate `npm run check` passed: 666 Rust tests (11 opt-in tests ignored),
593 web tests, 180 Chromium/WebKit scenarios, documentation/dependency checks,
builds and Clippy with warnings denied. The focused transport/terminal tests and
Windows cross-build also passed after the final composition adjustments.
Five focused Chromium/WebKit navigation, file-draft and conversation scenarios
passed after the workspace-entry focus adjustment. The embedded build also passed
automatic default-WSL setup and two authenticated Codex turns across native window
reopening, in a separate isolated root.

Deferred Git/checkout acceptance passed on 2026-10-02 with the rebuilt native
Windows executable. The existing 20-step journey passed Git review/commit,
launcher worktree creation, Setup/Run, cleanup and attachment recovery. Its launcher
assertion now waits for actual catalog creation: closing the launcher is submission,
and a responsive `load_board` may finish before checkout. The default-WSL journey
also passed with installed, authenticated Codex and conversation recall after window
reopening. `npm run check` passed (671 Rust, 593 web and 180 browser tests; 11 opt-in
Rust cases ignored). The separate cross-build and both native journeys passed.
Those checks cover execution; installer evidence is recorded below. Automatic
updates and other-provider parity remain outside this coverage.

Initialization acceptance also passed on 2026-10-02 after rebuilding the Windows
executable: `npm run check` passed with 674 Rust, 593 web and 180 browser tests
(11 opt-in Rust cases ignored). The native journey passed 21 steps, including the
existing Setup rerun button preserving a locally edited copied file. Its isolated
fixture handoff waits only for explicit busy-before-effects shutdown refusals;
uncertain transport outcomes are not retried. The default-WSL journey passed with
real Codex discovery, conversation and recall after reopening the native window.

NSIS installation acceptance passed on 2026-10-03 with the optimized 0.18.2
Windows x64 package and its embedded Linux runtime. Per-user installation and
reinstallation registered the app and its Start menu shortcut. The installed
executable passed the 21-step native regression and a seven-step default-WSL
journey with authenticated Codex, including reinstallation between replies and
conversation recall afterwards. Both journeys completed without JavaScript errors
or console diagnostics. The fixture dismisses the existing release-notes dialog
through its ordinary Close button before continuing. Documentation, dependency
boundaries, release-script tests and the packaging build also passed. The earlier
full check above covers the unchanged application implementation. This machine
already had WebView2 and Ubuntu 24.04 WSL; missing-WebView2 installation, a clean
Windows VM, public signing and automatic updates were not validated.

Existing-root startup regression passed on 2026-10-03 after an installed app
failed to reopen earlier state bound to `/` with a fresh Windows profile. Bootstrap
now reads the persisted directory through `RuntimeRoots`; it does not rewrite the
identity to the Linux home. The 23 bridge tests and scoped Clippy passed. The
installed package passed an eight-step native journey with that legacy metadata,
real Codex replies and reinstallation/reopening without diagnostics. Opening the
machine's actual earlier root also restored its saved project and workspace.
This adds existing-data coverage to the clean-profile installation checks above.
