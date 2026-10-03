# Portable application core

Status: implemented. Decision: [ADR 0085](../decisions/0085-portable-core.md).

`src-tauri/crates/core` (`prometeu-core`) holds the persisted models, portable
rules and sequencing shared by the desktop and the WSL runtime. It has no Tauri,
filesystem, subprocess, socket or OS dispatch. It is a library in the Cargo
workspace, not a deployed service. This document is a map of its ports and the
obligations on their implementations; it is not a wire format. Formats that
cross processes are in the [IPC](ipc.md), [persistence](persistence.md) and
[WSL runtime](wsl-runtime.md) contracts.

## Ports and adapters

Composition roots choose implementations: `src-tauri/src/main.rs` for the
desktop and the runtime host (`crates/runtime/src/host.rs`) for WSL.

| Area | Core ports and services | Desktop adapter | WSL runtime adapter |
| --- | --- | --- | --- |
| Board | `BoardStore`, `BoardEvents`, `BoardPublisher` | `board_store.rs`, `state.rs` | `CatalogStore` in `runtime::workspaces` |
| Workspace catalog and lifecycle | `Workspaces`, `WorkspaceFolders`, `WorkspaceWorktrees`, `workspace_lifecycle`, `WorktreeCleanup` | `session.rs`, `prometeu-git::cleanup` | `runtime::worktrees`, `runtime::lifecycle`, `prometeu-git::cleanup` |
| Conversation stream | `conversation::stream::Lines`, `Work`, `ConversationInput`, `TranscriptStore`, `ConversationEvents`, `Clock` | `transcript_store.rs`, `chat.rs` | runtime `SessionStore` and event delivery |
| Session coordination | `SessionService`, `SessionReactions`, `SessionOutput`, `SessionHost`, `SessionPump` | `chat/host.rs` | runtime host |
| Launch and workers | `LaunchService`, `ResumePreparation`, `ConversationLauncher`, `ConversationWorkers`, `TaskExecutor` | `session/launch.rs`, `prometeu-process::ThreadExecutor` | runtime provider, `ThreadExecutor` |
| Provider preparation | `ProviderPreparation`, `AgentProtocol`, `AgentInput`, `ProviderDiscovery` | `agent_launch.rs`, `prometeu-protocols` | `runtime::provider`, `runtime::discovery`, `prometeu-protocols` |
| Processes and terminals | `ProcessLauncher`, `ProcessControl`, `ProcessWait`, `TerminalFactory`, `TerminalEvents`, `AuxiliaryLauncher` | `prometeu-process` | `prometeu-process` (responsive PTY factory) |
| Bounded commands | `QueryLauncher`, `CommandRunner` | `prometeu-process` | `prometeu-process` |
| Accounts and login | `AccountRegistry`, `AccountStore`, `LoginService`, `AccountAuthentication`, `LoginEffects` | `prometeu-files::accounts`, `accounts.rs`, `account_login.rs` | `prometeu-files::accounts`, external Codex account |
| Profiles and tools | `ProfileBackend`, `StartupTools`, `PackageBackend`, `tool_resolution` | `prometeu-profiles`, `prometeu-tools`, `tool_materialization.rs` | `prometeu-profiles`, `prometeu-tools`, `runtime::tools` |
| Files, Git and repository | `ProjectFiles`, `ProjectEntries`, `ProjectSearch`, `RepositorySettings`, `RepositoryGit`, `RepositoryReferences` | `prometeu-files`, `prometeu-git` | same crates |

Native request types (a prepared `Command` or PTY `CommandBuilder`), captured
profiles, paths and handles are adapter-local generic parameters. No port is a
generic method dispatcher, and none of these values crosses IPC or the bridge.

## Obligations of implementations

- Event, storage, diagnostic and publication ports must not re-enter the
  operation that called them; several run under the conversation or registry
  lock. The host owns serialization and lock order.
- Stores distinguish "missing" from "empty" and return load failures instead of
  defaults. Private writes use 0700 directories and 0600 files with atomic
  replacement; errors are returned once, already encoded where the port says so.
- A `TaskExecutor` accepts work without running it inline and drops rejected
  jobs before returning.
- Process and terminal waiters own their child: dropping one unreaped kills and
  reaps it. Control operations are best-effort requests; only the waiter
  establishes exit.
- Hosts that share an installed package cache share one preparation gate.

## Board and publication

`BoardPublisher` orders snapshot capture, queueing, delivery and durable
barriers under one publication mutex; clones share it, and disk I/O and delivery
run outside the board lock. `persist` waits for the worker's acknowledgment; a
live worker's storage failure is returned without a second write, and only a
terminated worker permits a synchronous fallback. Terminal port allocation uses
the same barrier. `FileBoardStore` keeps backup recovery and defaults only when
neither file is readable. `revive` runs only after process loss.

## Conversation stream and pump

The host holds one conversation mutex across a command write, local acceptance
events (`session.state` busy, `user.message`, adapter echoes), transcript append
and live delivery. A rejected command records nothing. Every public event takes
one sequence, including ephemeral ones; `telemetry.usage` takes none, and
`telemetry`/`providerDurationMs` fields are stripped before persistence and
delivery. Storage and delivery errors are returned separately without rolling
back the buffer. The replay buffer keeps 4 MiB of whole lines; loading preserves
mixed legacy/V1 bytes and restarts the transport sequence at zero.

`SessionPump` shares readiness, capture order, sequence and retirement across
clones. Accepted telemetry is captured after the process and transcript locks are
released; reactions run after the capture gate. A settled turn schedules pending
input only when a queue exists and setup is not running, and admission is
rechecked when the send runs. Failed writes restore the previous turn flag and
produce no capture or reactions.

## Processes, terminals and commands

`StartedProcess` hands over input, two independently drained output streams, a
`ProcessHandle` with a stable in-process identity and the owning `ProcessWait`;
both drains start before input is returned. The core owns the agent and terminal
shutdown policies; the Unix adapter starts a process group, serializes signals
with nonblocking reaping and clears running state only on observed exit.

`TerminalOutput` keeps 512 KiB of raw bytes with one sequence per header, chunk
or exit notice; snapshots use the same lock, and retirement suppresses late
output and closure. `Terminal::write_bytes` accepts raw input; `write` keeps its
UTF-8 behavior. `AuxiliarySession` adds cancellation, deadlines and typed errors
to private authentication pipes; stdout stays private and stderr is discarded.

`QueryPolicy` fixes one deadline and total stdout limit; `CommandPolicy` sets a
deadline, per-stream capture/discard/inherit and input bytes closed after
sending. Results separate a nonzero exit from timeout, unavailable executable,
output-bound and I/O failures. Current per-caller limits are listed in
[ADR 0085](../decisions/0085-portable-core.md#processes-and-commands).

## Sessions, launch and workers

`SessionService` receives the board plus runtime, account, publication and
diagnostic ports. Blank input has no effect; pending input appends in order and
survives setup, account changes and failed sends (a failed send restores the
prompt ahead of later input). Publication precedes revival and is not a durable
save. `SessionReactions` updates tab status, notes, unread state, context and
provider identity, never workspace stage. `SessionHost` owns one host's registry,
`InputGates`, readiness and background work; exit cleanup compares identity under
the registry lock and keeps a stopped conversation for replay.

`LaunchService` snapshots the workspace, tools, trust and delegation permission
under one board lock, prepares outside it, and only then retires the old process.
A failed spawn keeps the queue and prior board state. Success warns about missing
kickoff content, installs the conversation, updates status and note, publishes
and signals readiness, in that order. `start` installs only; the caller publishes
the new tab. Claude resumes when its transcript exists; Codex and Antigravity
resume from a saved identity; fresh starts never infer resume.

`ConversationWorkers` emits `session.state: starting`, requests `commands.list`,
then schedules stdout and stderr consumers. Nonblank stderr becomes
`provider.stderr`. Stdout EOF waits for exit and releases the waiter before
`WorkerLifecycle::exited`. `ProviderPreparation` may write files and run bounded
probes but never spawns the conversation; `AgentInput` carries closure and
turn-wait policy so shared code never matches a provider enum.

## Accounts and profiles

`AccountRegistry` keeps the existing serde shape, external defaults, optional
selection and opaque unknown or retired provider entries. Only missing storage
imports default external accounts. Updates clone, apply, validate, save if
changed, then replace memory; reads return copies. One instance serializes its
own updates; separate instances sharing a file are not coordinated.

`LoginService` reserves one pending login per host, refreshes quotas before the
active-turn check, publishes pending state before authentication and checks
cancellation before committing identity and revision. The host calls `finish`
after native work stops, including on executor failure; stale cleanup cannot clear
a later attempt. Authentication returns identity metadata only.

`ProfileBackend` resolves, prepares and applies native profiles from explicit
roots. Claude and Codex profiles share history and resources but never inference
credentials; external Antigravity rejects managed preparation. Preparation is not
a multi-file transaction.

## Tools and packages

`tool_resolution` owns global → approved project → workspace resolution,
provenance and exact-hash project trust. `StartupTools` supplies Claude
configuration and plugin arguments and Codex plugin and MCP artifacts; Codex
profile preparation precedes plugin materialization, and its environment precedes
MCP. `None` selections invoke no source; explicit empty lists produce strict
empty configuration; missing selected MCP IDs fail. Codex remote MCP secrets use
child environment variables and stdio secrets private environment files, never
command arguments. `NativePackages` keeps derived homes, revision 2 cache
versions, hashes, manifests and hook trust; installation failure attempts removal
of that canonical ID and propagates the original error. `PluginLibrary` and
`SkillLibrary` share local registration, Git import/update and removal.

## Compatibility

These boundaries change no persisted field, provider identity, IPC command, event
payload or conversation protocol. Desktop modules re-export moved types so source
imports keep working, and existing provider, IPC and file fixtures run against the
shared definitions.

## Verification

- `npm run test:core`, `test:process`, `test:profiles` and `test:tools` run
  without Tauri, GUI libraries or installed agents; `npm run test:rust` includes
  them.
- Portable tests live beside each module under `src-tauri/crates/core/src/`
  (`publication/tests.rs`, `conversation/stream/tests.rs`, `session/*/tests.rs`,
  `accounts/tests.rs`, `accounts/login/tests.rs`, `terminal/tests.rs`).
- Native tests: `crates/process/src/{tests.rs,terminal/tests.rs,command/tests.rs,query/tests.rs}`,
  `crates/profiles/src/tests.rs`, `crates/tools/src/{tests.rs,packages/tests.rs}`.
- `crates/core/tests/boundary.rs` checks the core manifest and known native
  tokens; `npm run architecture:check` checks the crate graph. Both are
  conservative and do not replace review.
