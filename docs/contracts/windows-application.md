# Native Windows application

Status: integration in progress under
[ADR 0084](../decisions/0084-shared-windows-desktop.md). The existing Prometeu
interface runs in a native Windows window; project files, Git, providers and
supporting terminals run in WSL through the [WSL runtime](wsl-runtime.md). Only
Codex is registered. This contract does not claim full application parity.

## Composition

The Windows Vite build transforms only the `index.html` entry script to
`src/windows/main.ts`. After connecting, it injects the WSL `IpcTransport`, the
Windows application menu, `AttachmentPicker`, `ConnectionRecovery` and
`TerminalSnapshots`, then imports the existing `src/main.ts`. The same sidebar,
desk, launcher, workspace and ChatView are used; screens never choose a
Windows/WSL implementation. Small images are inlined so nested SVG icons do not
depend on WebView2 custom-protocol lookup. The CSP allows same-origin resources
and the local image/frame blob URLs used by the existing viewers; it enables no
remote connections.

The command registry remains `src/ipc.ts`. `application_request` is the native
transport command: `ApplicationClient` sends
`{ method: "application", command, args }` through the runtime envelope and
requires `application.v1`. Unknown or unimplemented commands reject without
mutation, and older runtimes reject at the capability check before sending.
Host-only commands outside the shared registry are `application_open`,
`application_reconnect` and `application_paths`.

| Area | Operations |
| --- | --- |
| Discovery and accounts | `agents`, `agent_models`, `accounts`, `account_select`, `account_remove`, external `account_login` |
| Launcher | `create_workspace` with an existing folder or a new-branch worktree |
| Board and session | `load_board`, `set_stage`, `focus_tab`, `look_at`, `set_lang`, `rename_workspace`, `rename_tab`, `pin_workspace`, `set_unread`, `set_tab_choice` |
| Workspace lifecycle | `archive_workspace`, `finish_workspace`, `remove_workspace`, `cleanup_list`, `cleanup_worktree` |
| Conversation | `chat_snapshot`, `chat_send`, `chat_control`, `new_tab`, `close_tab` |
| Projects | `add_project`, `remove_project`, `reorder_projects` |
| Project files | `list_dir`, `read_file`, `read_bytes`, `file_stamp`, `write_file`, `create_path`, `rename_path`, `trash_path`, `find_paths`, `reveal_path` |
| Shell, Setup and Run | `open_dock`, `close_dock`, `dock_state`, `pty_buffer`, `pty_write`, `pty_resize` |
| Git | `workspace_git_status`, `workspace_git_diff`, `workspace_git_action`, `workspace_git_history`, `workspace_git_branches`, `workspace_git_conflict`, `workspace_git_resolve`, `tree_git_status`, `tree_restore`, `file_base` |
| Plugin library | `plugin_look`, `plugin_save`, `plugin_install`, `plugin_update`, `plugin_remove`, `plugin_scrap` |
| Local resources | `mcp_hub`, `mcp_save`, `mcp_remove`, `mcp_found`, `mcp_check`, `mcp_login`, `mcp_logins`, `mcp_logout`, `plugin_hub`, `skill_hub`, `skill_save`, `skill_remove`, local-only `catalog_state` |
| Tool selection | `set_tools_global`, `set_workspace_mcp`, `set_workspace_plugins`, `set_workspace_skills`, `workspace_tools`, `mcp_inherited`, `project_tools`, `project_tools_trust` |
| Attachments | file picker, `paste_files`, native drop, launcher `inject` and `@path` references |
| Repository declarations | `workspace_scripts`, `workspace_branch`, `list_branches` |

Requests identify their own session; the runtime never switches a shared
selection first. Events reuse the desktop `board`, `chat`, `chat-closed`, `pty`,
`pty-closed` and `accounts` contracts, and the native shell forwards only those
names. Observed statuses overlay the saved board; work stage stays independent.

## Deadlines and transport limits

- Ordinary requests wait 30 s for a reply; requests are at most 1 MiB and replies
  8 MiB including JSON encoding. Oversized writes reject before sending; an
  oversized read result returns an error without disconnecting.
- Text reads keep their 2 MiB limit. `read_bytes` keeps its `{ id, rel }`
  arguments, `ArrayBuffer` result and 100 MiB limit: the bridge requests
  consecutive `FileBlock`s (`data`, total `size`, `stamp`) of at most 256 KiB with
  `offset` and optional `stamp`, validates length, size and the stamp, then
  returns one raw Tauri byte response. Interrupted, malformed or changed reads
  reject with no partial result; this detects ordinary changes but is not an
  atomic snapshot. Reads occupy the native client until complete.
- Path conversion through `wslpath` has a 10 s deadline and bounded output.
- Plugin Git commands have 20 s and 256 KiB per stream and run synchronously.
- MCP inspection of local processes has 25 s and 1 MiB stdout.
- Deferred jobs are polled every 100 ms; the native wait gives up after
  10 minutes with an uncertain-outcome error and does not cancel the effect.
- Reconnection retries attachment and read-only restoration with delays from
  250 ms to 5 s.

## Paths and host effects

`ApplicationPaths` translates native selections before they reach shared
drafts or `add_project`: `\\wsl.localhost\<distribution>` and
`\\wsl$\<distribution>` paths of the connected distribution become Linux paths;
drive paths go through that distribution's `wslpath`, respecting its mounts.
Literal process arguments are used. Other distributions, network shares and
relative paths reject. The WSL folder adapter canonicalizes and verifies the
directory; duplicates return the existing project, registration creates no
workspace and removal deletes no files or workspaces. Returned sources are Linux
paths; URLs, Linux paths and `~` sources pass through unchanged.

`reveal_path` keeps its `{ id, rel }` arguments and void result. WSL returns a
private `FileLocation` (`path`, `dir`) after containment and existence checks;
`NativeApplication` converts it with `wslpath -w`, verifies the reverse
conversion and asks the injected `FileManager` to open the directory or select
the file through Shell APIs. It never launches a file's default application.
Linux names Windows would reinterpret (backslashes, trailing dots) reject.

Attachments keep the `@path` contract. Native drops convert physical to logical
coordinates and publish the existing `file-drag` events. `paste_files` reads
copied paths or converts clipboard pixels to PNG (64 MiB limit) under the user's
app-local `attachments/<uuid>/pasted.png`, then returns its WSL path. The
Windows mount must remain reachable; there is no automatic cleanup. Files outside
the workspace stay allowed, governed by the WSL user and provider.

## Files, Git and terminals

`ProjectFiles`, `ProjectEntries`, `ProjectSearch` and `RepositorySettings` use the
desktop implementations in `prometeu-files`: canonical containment, binary
rejection, compare-before-write conflicts, Git metadata refusal, case-only renames
and the 40-result ranked search with a per-service 30 s refresh cache. Trash uses
the Linux system trash with recovery metadata; an unsupported volume rejects and
never falls back to permanent deletion. Mutations invalidate Git caches.

`RepositoryGit` and `RepositoryReferences` use `prometeu-git` with the desktop
caches, mutation lock and admission rules; pull and discard refuse while an agent
runs. Git keeps its original process lifetime, without a new timeout or hook
cancellation.

Dock keys keep the desktop `<workspace>:terminal[-N]` format, up to 32 shells per
workspace. Shells survive window closure and stop on explicit runtime shutdown.
The transport orders `pty_write` per terminal; a failed write rejects its queued
suffix without replay. A stopped dock keeps its output until rerun; reopening a
running Run reuses it and a conflicting name rejects. Setup and Run use the shared
hydration, port reservation and script environment through `/bin/sh -lc`.

## Deferred application effects

`application.operations.v1` adds private `application_operation_start
{command,args}` and `application_operation_poll {job}` for Git, reference queries
and `create_workspace`. `application.initialization.v1` adds deferred
`agent_models`, `accounts` refresh and `open_dock` with `kind: "setup"`. MCP checks
and authentication use `mcp_operation_start {operation}` (`check`, `begin`,
`finish`, `logins`, `logout`), `mcp_operation_poll {job}` and
`mcp_auth_cancel {state}`. The shared UI, registry and mock keep their shapes;
only the native adapter selects this path.

- At most eight application jobs and eight MCP jobs; completed results stay ten
  minutes. Poll returns `{ done: false }` or consumes `{ done: true, result }`;
  failures consume the job. Unknown, expired and consumed IDs reject.
- Jobs and pending consent are memory-only. Window closure does not stop accepted
  work; runtime death loses results, while checkout or commit effects may remain.
- Only the owner loop commits catalog changes, against the current catalog,
  including while detached. Capacity and stage are revalidated before saving.
- Without the capability the adapter sends the synchronous command. A failed start
  or poll never falls back or replays. Old residents reject MCP operations.
- Shutdown and retirement refuse running or unclaimed jobs. Archive, remove and
  cleanup refuse while any application effect runs; Setup preparation and Git
  writes reject competing input and writes on the same checkout.

Manual Setup copies files on a worker and starts the PTY on the owner; reopening
a live Setup reuses it.

## Launcher and execution identity

The launcher sends the core `Draft`. Admission checks provider, account,
model/effort, stage and options before checkout effects; existing-branch,
multi-repository, plan and custom-instruction options reject. Application
sessions store the launcher's unattended policy as `Auto`; additional tabs
inherit it. The first message uses the shared `first_message` formatter.

The workspace is saved and published before provider startup. Deferred creation
prepares the checkout and hydration on a worker and commits on the owner; failure
keeps the prepared directory and reports its path. Setup starts before the
provider, and the host releases the first prompt only after Setup exits and the
provider is ready, with the desktop's Setup warning. Launch failure marks the
workspace failed with its prompt retained. The pending launch is removed before
dispatch, so nothing is replayed after an uncertain send or a restart.

`chat_send` starts or resumes a stopped context and waits for readiness, which
begins at `session.identity`. `new_tab` saves before launch (32 tabs per
workspace); closing a tab stops it and keeps its transcript directory.

Discovery uses `ProviderDiscovery`: the real Codex executable, `account/read`
identity and paginated `model/list` through the shared bounded queries. The
account registry is `<runtime-root>/accounts.json` in the desktop format.
Removing the selected account blocks new input and launches; attaching the
external account again does not select it. Managed OAuth profiles are not
implemented.

## GitHub discovery

The GitHub inbox commands (`github_identity`, `github_issues`, `github_repositories`,
`github_issue_open`, `github_projects`, `github_prepare`) currently have desktop
and browser-mock adapters only. The WSL runtime reports them unsupported; this
does not claim native Windows GitHub discovery or PR launch support.

## Conversation tools

Codex startup reads the current catalog through `ToolSelection` at each spawn and
uses the shared `tool_resolution`, declaration hashing, `NativeTools`,
`NativePackages` and MCP materializer. Running and idle processes keep their
captured selection; a stopped resume re-resolves. Only exactly approved project
declarations participate. Selection and trust mutations commit before
publication. Codex has no discovered CLI MCP base. The reserved `prometeu`
marketplace and derived workspace home preserve personal CLI configuration;
preparation failure prevents spawn and may leave earlier files.

Local hubs use the desktop file adapters and `SkillLibrary`/`PluginLibrary`.
`LocalCatalog` reports the disconnected sharing state; a save carrying a Cloud
revision rejects before writing. `prometeu` stays reserved without advertising
its built-in service. WSL leaves installed Codex cache cleanup to the next
derived-home rebuild. Saving a hub item does not activate it.

## MCP authentication and checks

Discovery, inspection and OAuth share `prometeu-tools` with the desktop;
credentials stay in `mcp-auth.json` under the runtime root. Windows injects
`prometeu-oauth::Consent`: it binds `127.0.0.1:17421/mcp`, opens the system
browser with a literal HTTP(S) URL, validates state and passes the code to WSL,
where the verifier and tokens stay. Callback and state expire after five minutes
(at most eight pending); invalid state cannot cancel consent. The loopback reader
keeps the request line (8 KiB) and drains the complete HTTP header, which shared
loopback cookies can enlarge (256 KiB, 2 s or the earlier consent deadline).
State is consumed once; nothing is replayed after reconnecting. The bridge polls
without holding the session mutex and verifies the captured target on each
request.

## Workspace lifecycle

Core `workspace_lifecycle` is shared with the desktop; WSL commits before
publishing. Model changes reuse `Workspace::retune`: provider changes and frozen
task profiles reject, and only the selected tab stops and resumes later with the
saved choice. Archive and Finish stop the workspace's agents, docks and pending
launch; the archive script must start before the mutation commits. Unarchive
starts nothing. `WorktreeCleanup` uses the desktop checks: cleanup requires an
archived, non-original checkout, dirty or unmerged work needs explicit force, and
saved existing branches stay protected. WSL admits single-repository worktrees
only. Transcripts live outside checkouts and reopen after cleanup; cleaned
workspaces reject new input, tabs and docks.

## First launch, reconnection and packaging

`application_open` composes `WslEnvironment` and `RuntimeInstaller`. Discovery runs
`wsl.exe --exec` without `--distribution`, so WSL picks its default; a bounded
login-shell probe returns `WSL_DISTRO_NAME`, home, architecture and Codex path.
Nothing is installed in WSL. The application catalog starts empty; existing
catalogs win over the seed.

The build embeds a matching x86_64 Linux runtime. Installation streams it through
a bounded WSL command, checks its SHA-256, runs `--help` to verify loading and
atomically publishes it under
`~/.local/share/prometeu-windows/runtime/<digest>/prometeu-runtime`; failures keep
the previous executable. WSL needs `/bin/sh`, `sha256sum`, Codex and a compatible
libc; x86_64 Ubuntu 24.04 is the acceptance target. State defaults to
`~/.local/share/prometeu-windows/state` and never adopts a WSLg root;
`PROMETEU_WINDOWS_RUNTIME_ROOT` overrides it. The persisted `runtime.json`,
read through `RuntimeRoots` without writes, decides the working directory over
WebView storage; with no metadata only a cache for the same root applies,
otherwise the Linux home. A changed default distribution uses its own root.

`application_reconnect` replaces only a failed connection to the captured target.
It never rediscovers, resends input, restarts providers or opens terminals. Views
restore through `ConnectionRecovery`; drafts and editors stay in the document.
Explicit disconnect or shutdown suppresses recovery. Runtime replacement follows
[safe replacement](wsl-runtime.md#safe-replacement).

`npm run package:windows` builds Tauri's per-user NSIS installer with identifier
`co.prometeu.desktop` and the root package version; see
[packaging](../operations/release.md#windows-installer). Reinstallation keeps the
WebView profile and never touches WSL state or projects. Updates require the next
installer.

## Verification

- Bridge and transport: `crates/bridge/src/` tests (`bootstrap.rs`,
  `wsl_command.rs`, `paths.rs`, `application.rs`, `files.rs`, `operations.rs`,
  `mcp.rs`), `src/windows/*.test.ts` and `src/ipc.test.ts`.
- Runtime over the application transport: `crates/runtime/tests/resident.rs`
  (addressing, files, docks, deferred Git, catalogs, recovery) and
  `lifecycle.rs` (tools, plugins, MCP OAuth with a hermetic server, lifecycle).
- Shared adapters keep their desktop suites in `crates/files`, `crates/git`,
  `crates/tools` and `crates/core`.
- Opt-in native journey: `npm run test:windows:native -- CONFIG.json` drives the
  original UI through WebView2, native dialogs, clipboard, Explorer and WSL;
  `scripts/fixtures/windows-install.ps1` covers NSIS installation. See
  [development](../operations/development.md#native-windowswsl-integration).

Not validated: missing-WebView2 installation, a clean Windows VM, public signing,
automatic updates, managed login, other providers and model image interpretation.
