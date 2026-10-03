# Portable application core

Status: implemented board, conversation, process, session, terminal and private subprocess boundaries.
Decisions: [ADR 0063](../decisions/0063-portable-board-core.md),
[ADR 0064](../decisions/0064-portable-conversation-stream.md),
[ADR 0065](../decisions/0065-injected-process-supervision.md),
[ADR 0066](../decisions/0066-injected-session-coordination.md),
[ADR 0067](../decisions/0067-injected-terminal-and-private-processes.md) and
[ADR 0068](../decisions/0068-bounded-command-and-query-ports.md).

`src-tauri/crates/core` contains the board and its referenced action, delegation,
issue, PR and telemetry identity models, legacy revival, selection composition,
workspace tool-selection use case, ordered board publisher and conversation
stream rules, process-supervision policy, session coordination, terminal byte
retention, private subprocess sessions and bounded command/query ports. It has no Tauri,
filesystem, subprocess, socket or desktop dependency. It is a library within the
Cargo workspace, not a separately deployed runtime.

## Ports and composition

`BoardStore` loads an unreconciled board and saves an immutable snapshot. Its
implementation owns serialization, backup recovery, permissions and atomic
replacement. The existing `FileBoardStore` receives its root explicitly from
`main.rs`; it retains the current missing/corrupt-primary backup behavior and
defaults only when neither file can be read. The application calls `revive` only
when restoring after process loss, then prepares telemetry identities.

`BoardEvents` receives an immutable board snapshot and returns a delivery
result. It must not reenter publication or persistence while delivering it.
The desktop adapter preserves the `board` Tauri event and payload. Disconnected
delivery does not cancel the queued save. The port is passed explicitly to the
publisher; no ambient emitter or service lookup exists in the core.

`BoardPublisher` receives an `Arc<dyn BoardStore>`. A single queue and
publication mutex order snapshots, delivery and durable barriers. Clones share
the same queue and mutex. Disk operations and event delivery release the board
mutation lock. `persist` prepares identities and waits for acknowledgment on
that queue; a live worker's storage failure is returned without an implicit
second write. A terminated worker permits a synchronous fallback because it can
no longer overwrite that result. There is no timeout that starts a competing
write while the worker might still be saving.

Terminal port assignment releases its board lock before calling this same
publisher's durable barrier. It no longer performs a separate `Board::save`
that an older pending snapshot could overwrite. Counts publish after the save
attempt, preserving the existing failure logging and UI behavior.

The core owns persisted types. Launch construction and delegation observation
remain application functions at the desktop edge, so models do not import
session, provider, clock or Tauri adapters to perform those operations.
Existing UUID identity generation remains in the core; this boundary does not
claim that all its functions are deterministic or free of scheduling effects.

## Workspace catalog

The core also owns a workspace catalog through `workspaces::Workspaces`,
reusing `Board`, `Workspace`, `Project` and `Tab`. `CatalogStore` and
`WorkspaceFolders` inject persistence and directory validation; the composing host
supplies provider selection. `WorkspaceWorktrees` injects new-branch checkout
preparation before registration, keeping Git commands out of the catalog. The service validates catalog identities and stages
and saves mutations before committing its local snapshot. Native execution stays
in the host's context factory. See the [workspace contract](wsl-workspaces.md) and
[ADR 0083](../decisions/0083-wsl-workspace-catalog.md).

## Conversation stream

`conversation::stream::Lines` owns accepted-input events, transcript retention,
transport numbering, telemetry filtering and `{ text, seq }` snapshot creation.
`conversation::work::Work` owns settlement of a turn and its observed background
tasks. All providers use these same rules through the desktop integration.

The host supplies these ports explicitly:

| Port | Responsibility | Desktop implementation |
| --- | --- | --- |
| `ConversationInput` | send a canonical command with the current replay buffer and return local echoes | `Chat`, using existing provider adapters |
| `TranscriptStore` | load one conversation and append retained public lines | `FileTranscriptStore` or `ProviderTranscriptStore` |
| `ConversationEvents` | deliver one public line with its transport sequence | `DesktopConversationEvents` preserving `chat(session, text, seq)` |
| `Clock` | Unix milliseconds for acceptance and snapshot events | `SystemClock` |

There is no generic method-name dispatcher, native path or provider selection in
these core ports. Canonical commands/events retain the existing V1 JSON shape;
this extraction does not introduce a new bridge wire protocol.

The host holds the same per-conversation mutex across command sending, local
recording and live publication. A successful message records `session.state`
busy, then `user.message`, then adapter echoes before concurrent output can
overtake them. A rejected command records nothing and is never retried by the
stream. Request responses close their request without starting a new execution.
The recording callback may update ordered in-memory observations; it must use
the supplied buffer and must not reenter command delivery. Event/storage ports
must not acquire that conversation lock again.

Each public event consumes a sequence, including ephemeral events excluded from
replay. A retained event is appended once, retained in memory and emitted once;
storage and delivery errors are returned independently in `Delivery`. Neither
error rolls back the buffer or triggers another command send. The desktop logs
storage errors and continues, preserving the prior best-effort behavior.
Private `telemetry.usage` consumes no public sequence; `telemetry` and
`providerDurationMs` fields are stripped before persistence and live delivery.
The original frame remains available for private capture.

The replay buffer keeps its existing 4 MiB limit by removing complete lines.
Loading preserves mixed legacy/V1 bytes, normalizes a missing final newline in
memory only and starts the process's transport sequence at zero. Load failures
are explicit at the port; the desktop retains its empty-buffer fallback.
Snapshots append synthetic busy/ready state without changing the buffer or
consuming a sequence. Native history normalization remains in provider adapters.

The desktop constructs file adapters with explicit seed/log paths. Claude's
provider-owned transcript is read-only to Prometeu; Codex and Antigravity keep
private app-managed V1 logs. The process ports below now cover spawning, pipe
draining and shutdown. Session coordination uses the ports described below;
native account, action, delegation and telemetry implementations stay at the edge. There is no
headless runtime or reconnect guarantee yet. See
[ADR 0064](../decisions/0064-portable-conversation-stream.md).

## Process supervision

The core's `ProcessLauncher<Request>` port performs one typed launch. Its request
type belongs to the execution adapter; the current Unix implementation accepts
the provider's prepared `Command` without reconstructing it from strings.
No subprocess command is sent across IPC by this interface. The provider keeps
ownership of executable discovery, command configuration and environment cleanup.

`StartedProcess` transfers a byte writer, independently drained stdout/stderr
iterators, a `ProcessHandle` and an owning `ProcessWait`. Both drains must start
before returning the input writer, so output cannot block a child that is not
yet reading a large command. Line decoding retains existing behavior: invalid
UTF-8 or a read error ends that pipe's iterator; publication stalls can grow its
unbounded queue. Changing those policies requires separate evidence.

`ProcessHandle` carries an in-process identity and injected `ProcessControl`.
Its numeric system ID is for native process accounting; identity comparisons
never use that ID or a running flag. The desktop compares identities while
holding its conversation-map lock through exit reactions, so a predecessor's
completion cannot clear the successor's readiness or board status. Tokens are
not persisted and do not implement future reconnect epochs.

The host closes provider input before the shutdown grace period. It executes
`ShutdownPolicy::AGENT` on a background thread: wait 2 seconds, request
termination, wait 500 ms and request kill. Control operations are best-effort
requests; the owning waiter establishes exit. The same control port supplies
Antigravity's group interruption. Claude and Codex retain protocol interruption.

`prometeu-process::UnixProcessLauncher` starts a dedicated process group. Its
control and waiter serialize signaling and reaping with the same mutex. Reaping
uses nonblocking attempts and releases the mutex between attempts, allowing
shutdown to proceed while a child is alive. Observed exit clears running state
and retires the signal target under that mutex. Stdout EOF never disables
signaling by itself. Repeated waits return the retained exit code; dropping an
unreaped waiter requests kill and reaps it. A post-spawn pipe-setup failure has
the same cleanup owner.

The group is addressable only until its leader is reaped. Detached children and
descendants that outlive that point are not tracked separately. PTYs and authentication have separate ports in the same crate, described below.
Other native subprocess consumers retain their existing adapters. The crate
has no Tauri or GUI dependency and is independently tested on Unix; it is not a
Windows-native process implementation. See
[ADR 0065](../decisions/0065-injected-process-supervision.md).

## Session coordination

`SessionService` receives explicit board state and four independent ports:
`SessionRuntime` for process state, setup, readiness, stopping, revival and
canonical input; `SessionAccounts` for account change/working observations;
`SessionPublication` for board publication and the currently viewed workspace;
and `SessionDiagnostics` for account and pending-input failures.

Blank input has no effects. Existing pending input is appended in order. An
account change waits for current work or restarts an idle process before new
input. Setup and provider turn barriers retain queued input. An orphaned queue
revives its process. A failed send restores the prompt ahead of concurrently
appended input and pauses its action with the error. Account failures leave the
queue available for recovery. Publication is requested before revival; ordinary
publication does not promise synchronous persistence.

An execution host owns `InputGates`; local, remote and MCP commands share its
per-session gate across admission and delegation reservation. Hosts do not share
an ambient registry. Runtime adapters recheck process identity and idle-only
admission under their process lock. Readiness and background work are ephemeral.

`SessionReactions` consumes canonical V1 events with injected context, action
completion and usage ports. It updates tab status, notes, unread state, context
and provider identity without changing workspace stage or sibling tabs.
Background settlement uses `Work::observe`, including assistant activity that
invalidates a terminal held by children. Repeated readiness announcements do
not repeatedly release queued input. Provider usage dispatch stays in adapters.

`SessionOutput` receives transcript/event ports and `ExecutionObservation`.
Its memory-only observation runs under the transcript lock before publication.
`SessionTelemetry` sees the original frame after transcript release, inside the
host's capture-order gate. Private usage frames never enter public history.
`CommandTelemetry` and `capture_command` coordinate successful command capture
under that same gate; failed writes do not capture accepted input. The host
prepares scope and relation identities before the gate. Its adapter rejects
stale telemetry generations after history deletion. Execution publication and
reentrant reactions run after releasing the capture gate and transcript lock.

`src/chat/host.rs` supplies desktop effects. Account registries, provider usage,
SQLite, actions and delegation retain their existing implementations; the core
owns their session sequencing. Workspace creation, launch preparation, dock configuration and auxiliary
service composition still require native effects; revival sequencing now uses
the launch service described below. See
[ADR 0066](../decisions/0066-injected-session-coordination.md).

## Conversation pump

`SessionPump<T>` composes output delivery and command recording with a shared
telemetry capture gate. Clones share readiness, capture, transcript sequence and
retirement. The pump has no desktop handle or provider profile. It receives
`PumpReactions`, `PendingInput` and `PumpDiagnostics`, plus the existing output
and clock ports. Reactions run after execution publication and after releasing
the capture/transcript locks. A settled turn schedules pending input only when
a queue exists and setup is not running. The scheduler stays at the edge;
`SessionService` rechecks admission when the scheduled send executes.

For commands, the host prepares telemetry scope/relations and rechecks process
identity and idleness under the registry lock inside `capture`'s closure. The
pump's `command` records canonical echoes under the transcript lock. The closure
releases its process lock before accepted capture; reactions follow capture
release. Failed writes restore the previous turn flag and cause no accepted
capture or reactions. Storage failure preserves live delivery and is reported
through a non-reentrant diagnostic sink, which can run under command locks.

The desktop supplies `ConversationHost` and `TelemetryCapture` implementations.
Its launch configuration, persistence preparation and feature implementations
still depend on native composition. No IPC or persisted format changes here.
Portable tests verify ordering, shared readiness, settlement/setup scheduling,
private usage, retirement and failed writes. See
[ADR 0070](../decisions/0070-portable-conversation-pump.md).

## Terminals and private subprocesses

`TerminalFactory<Request>` creates raw input/output streams, `TerminalControl`
and an owning `TerminalWait`. The desktop injects the Unix factory using its
prepared `CommandBuilder`; shell selection, arguments, working directory and
environment remain at the execution edge. There is no platform selection in
core terminal code and no command builder crosses desktop IPC.

The core `Terminal` delegates input, resize, process state and asynchronous
shutdown to those ports. `TerminalOutput` retains 512 KiB of raw bytes, with one
sequence per header, output chunk or exit notice. Split UTF-8 and ANSI bytes
remain unchanged. A snapshot uses the same buffer lock as numbered delivery;
event sinks must not reenter terminal operations. `exit_code` remains nullable
`u32` with the native PTY exit convention. No bytes or sequences persist to disk.

Retirement suppresses further output and closure events under that same lock,
so a replaced reader cannot close its successor in the UI. The desktop runs
setup completion callbacks after releasing the lock, even after intentional
closure, then publishes closure for a current terminal and updates counts.

The native factory serializes signaling and reaping; running state changes only
on observed exit, never on output EOF. Close requests hangup, then escalation
through `ShutdownPolicy::TERMINAL` (one second, terminate, 500 ms, kill).
The owning waiter reaps once and retains its code. Abandoning it or failing setup
after spawn kills and reaps the child. Group ownership ends when the leader is
reaped; detached descendants are not tracked separately.

`AuxiliaryLauncher<Request>` returns private `AuxiliaryProcess` input/line/exit
operations. `AuxiliarySession` adds cancellation and deadline checks with typed
errors. Authentication provider adapters inject the Unix implementation and map
errors to the same account/i18n codes. Stdout stays private; stderr is discarded.
The adapter keeps the existing 64-line queue and 1 MiB per-line limit, checking
that bound during reading. Oversized or invalid lines close output without
publishing partial frames. Dropping the transport kills and reaps its owned
child. Cancellation does not interrupt an already-blocked native write.

Real PTY and private-pipe tests run independently of Tauri in `test:process`;
portable policies and event ordering run in `test:core`. Existing account and
Codex protocol tests retain native-adapter coverage. Bounded catalog, naming
and Git helper paths use the command/query ports below. Other repository and
system integration commands retain their existing adapters.
This is not a headless host or a WSL transport. See
[ADR 0067](../decisions/0067-injected-terminal-and-private-processes.md) and
[ADR 0068](../decisions/0068-bounded-command-and-query-ports.md).

## Bounded commands and queries

`QueryLauncher<Request>` produces a private `QueryProcess` with send, line,
input-close and finish operations. `QueryPolicy` fixes one timeout and total
stdout byte limit for the whole query. The catalog adapter receives this port,
keeps provider correlation and pagination, and maps typed errors to existing
catalog codes. It retains 20 seconds and 1 MiB, including line delimiters.
Callers drain stdout before requesting normal exit; queries that stop after an
expected response drop their transport, which kills and reaps the server.

`CommandRunner<Request>` runs one finite command with explicit input and
`CommandPolicy`: timeout plus capture/discard/inherit for each output stream.
Its result separates exit success from stdout/stderr bytes. Timeout, unavailable
executable, output bound and I/O failures remain distinct. `main.rs` injects
Unix implementations into catalog commands, naming, action monitoring and
workspace preparation. Prepared native requests never cross IPC.

The finite adapter advances nonblocking stdin, stdout and stderr together,
closes stdin after the provided bytes and checks its deadline throughout. It
retains the unreaped leader while captured pipes remain open; a descendant
holding those pipes cannot extend the call indefinitely. Query writes also
respect the shared deadline, while a reader bounds total stdout before queuing
UTF-8 lines. A cleanup owner exists immediately after spawn. Only that owner
polls/reaps/signals; retained exit status prevents signals after PID retirement.
Detached descendants are outside the owned-group guarantee.

Claude naming captures at most 1 MiB in 60 seconds; Codex keeps its final-answer
file and discarded streams. Errors retain the fallback title. Background `gh`
action queries use 30 seconds and 8 MiB per stream; either overflow is a response
error. Preparation fetch uses 10 seconds, closed stdin and inherited output;
a completed nonzero exit still permits the existing-ref fallback. These policies
bound post-spawn I/O and polling, not kernel-level process creation or teardown.

The extraction does not migrate all repository Git operations, alter model
schemas or add retries. Tests cover native pipe backpressure, blocked writes,
output bounds, held-open descendant pipes and cleanup, plus feature-level fake
runners for error/fallback compatibility. See
[ADR 0068](../decisions/0068-bounded-command-and-query-ports.md).

## Session host ownership

`SessionHost<C>` owns the live-conversation registry, input gates, readiness and
background work for one execution host. `HostedConversation` provides native
identity, liveness, account/turn observations and retirement;
`SessionLifecycle` provides setup observation, revival, command delivery and
stop effects. `with_service` composes the existing `SessionService` against an
explicit board, publication and diagnostics. Each host has independent state,
even when session identifiers coincide.

The desktop injects `Chat` and its native lifecycle effects. Its commands, MCP
admission, account checks, resource accounting and setup completion share the
same host. Low-level command capture still takes the conversation registry
lock through identity validation and transcript writes. These locks and native
handles are in-process implementation details, never bridge payloads.

Removal retires output and clears readiness/work while excluding replacement,
then releases the registry lock before native transport teardown. All removal
paths now clear transient state; the transcript and persisted tab remain owned
by their existing services. Admission gates remain stable across removal.
Exit cleanup compares process identity and holds the registry lock through its
publication callback, which must not reenter the registry. Stale exits do
nothing; current exits retain the conversation for replay. Stream checks do not
cancel reactions already in flight.

Portable host tests cover isolation, stale exits, lock ordering, replay,
recovery through readiness and account handoffs. Existing provider and desktop
fixtures retain their wire/IPC coverage; no serialized format changes here.
The pump now uses the portable orchestration above; native feature
implementations still require desktop composition. See [ADR 0069](../decisions/0069-portable-session-host.md).

## Session launch and resume

`Launch` and its existing deserialization/package rules live in the core.
`LaunchRequest` carries session/workspace identity, an execution-side worktree,
resolved settings and fresh/resume intent. `ConversationLauncher<C>` returns a
conversation and whether the adapter actually resumed. These types are local
interfaces, not serializable bridge requests or native shell commands.

`LaunchService::resume` snapshots the workspace, global tools, trust and
optional delegation permission under one board lock. `ResumePreparation` runs
outside that lock to resolve tools/kickoff and validate native paths. The core
applies a delegation permission override, distinguishing no delegation from a
delegation with unset permission. Only successful preparation revokes access,
retires the old process and attempts spawning. A failed spawn preserves pending
input and prior board fields without publication/readiness effects. Success
warns about missing kickoff content, installs the replacement, updates status
and note, publishes, then signals readiness. Workspace stage remains unchanged.

`start` installs a new conversation without publishing or releasing input; its
caller owns tab creation and publication. Both paths use the same launcher.
The desktop composes preparation and `LaunchEffects` in `src/session/launch.rs`,
with provider configuration injected from `src/agent_launch.rs`. Claude resume uses transcript existence; Codex and
Antigravity use saved provider identity. Missing history falls back to a fresh
provider session as before. Native configuration stays at the edge; the shared
workers below do not add a retry or installation barrier.

Existing provider fixtures use the shared launch settings. Portable tests cover
ordering, failures, queue preservation, initial publication ownership, explicit
permission clearing and legacy/default tool selection formats. See
[ADR 0071](../decisions/0071-injected-session-launch.md).

## Conversation workers

`ConversationWorkers` initializes the provider input adapter, then schedules
independent stdout and stderr consumers through `TaskExecutor`. Starting state
and initialization echoes precede reader scheduling. Provider adapters retain
translation and stderr filtering; nonblank accepted stderr becomes canonical
`provider.stderr` notices. All frames use the shared pump, with startup/notice
timestamps from its injected clock. Initialization errors remain diagnostics.

Stdout EOF does not publish closure: the worker first waits for process exit and
releases its owning waiter, then calls `WorkerLifecycle::exited` with identity
and wait result. Desktop effects preserve the current-host identity check and
closure publication. Wait failure still triggers waiter cleanup before that
callback; the desktop keeps its prior stopped-state behavior.

An executor must accept work without running inline or waiting for completion,
and allow blocking reader jobs to progress independently. On rejection it must
drop the job before returning. A rejected reader therefore releases the waiter
and its child-cleanup obligation, even when the other reader was accepted.
`ThreadExecutor` in `prometeu-process` supplies native threads. `main.rs` injects
it for chat readers, pending input and shutdown; other backend jobs retain their
existing adapters. Reader scheduling failure maps to the existing spawn error.

Agent shutdown closes input before scheduling its existing policy. Scheduling
failure requests immediate kill, leaving reaping to the owning waiter. Pending
input scheduling failure logs a diagnostic and preserves the queue. Native pipe
drains already exist before initialization; unbounded queues, activation before
registry installation and in-flight retirement limits remain unchanged. No wire
or persisted format changes. Tests exercise initialization order, translated
output, stderr filtering, wait/drop/exit ordering, retirement, reader rejection
and shutdown fallback. See [ADR 0072](../decisions/0072-injected-conversation-workers.md).

## Provider preparation and input

`ProviderPreparation<Prepared>` accepts the shared `LaunchRequest`. Preparation
may materialize account/tool files and run existing bounded probes, but must not
spawn the conversation. The native implementation in `agent_launch.rs` registers
provider functions; desktop composition injects it in `AppState`. Prepared native
commands, transcript stores, captured profiles and protocol factories stay at
the execution edge. They are neither serialized nor shared with a Windows shell.

The native launcher starts the prepared command through `ProcessLauncher`, then
connects the factory to stdin and `ProcessControl`. It receives `AgentProtocol`:
a canonical `AgentInput` plus output translation. Shared chat code delegates
command handling, input closure and turn-wait policy through that interface;
it does not match a provider enum. Antigravity uses injected process interruption
and requires idle turns; Claude and Codex keep their protocol commands and permit
input under the existing policy. Closing Codex input releases its writer while
reader-owned protocol state may still consume late output.

Claude seeds read-only provider history. Codex and Antigravity seed and append
app-managed canonical logs. Resume selection, CLI arguments, account capture,
stderr filtering and inherited Claude-variable cleanup retain their behavior.
Cleanup preserves explicitly supplied environment values, including credentials.
Preparation errors keep their existing launch error path and ordering.

Compatibility tests in `agent_launch.rs` prepare Claude/Codex without available
agent binaries, check fresh/resumed selection and transcript ownership, and
reject a retired provider. Each provider tests input through the trait, including
Antigravity’s signal and completion outcome and Codex’s shared writer closure.
Existing protocol fixtures cover native frames. No live model is required.

Native configuration still depends on account/tool modules in the desktop crate;
this boundary alone cannot build a headless executable. Feature composition,
terminal ownership and activation after host installation remain pending. See
[ADR 0073](../decisions/0073-injected-provider-preparation.md).

## Account registry

`prometeu-core::accounts` owns `Identity`, `Account`, `Registry` and
`AccountRegistry`. These preserve the existing serde shape, validation, external
profile defaults, optional selection and opaque unknown/retired provider entries.
Native profile paths and credentials are absent from these types.

Each `AccountRegistry` receives an `AccountStore` and owns its state. `load`
returns `None` only for missing storage; only that case imports the initial
Claude/Codex external registrations. An empty registry remains empty. Load or
validation failure stays observable and rejects updates without attempting to
repair or overwrite storage. The host must recreate the registry to reload it.

An update locks the registry, clones its state, applies the callback, validates,
and saves only when changed. Memory changes only after successful saving. A
callback, validation or storage error leaves memory intact. Callbacks and stores
must not re-enter this registry; reads wait for an in-progress update. Concurrent
updates through one instance cannot lose one another's changes. This is not a
transaction across separate instances or processes using the same storage.

`active` and `find` return captured account copies; `registered` can also check a
revision. Unknown and retired provider records remain outside visible accounts
and selections. Desktop snapshots continue to omit opaque entries and retain
the existing ephemeral login field. No IPC or persisted format changes.

`account_store.rs::FileAccountStore` receives an explicit root and uses the
existing private atomic file writer. A save error keeps memory unchanged; an
error after filesystem replacement can still leave disk ahead of memory, as
before. This boundary adds no filesystem rollback or recovery protocol.
`accounts.rs` composes the store in its existing lazy desktop singleton and
re-exports the models. Login coordination uses the portable service below;
native profiles, credential materialization, quota effects and publication remain
at the desktop edge.
A headless host can create independent registries but still needs those native
feature dependencies extracted.

Portable tests cover missing versus empty storage, load/update/validation/save
failures, no-op writes, concurrent commits, independent hosts and opaque provider
round trips. Native tests cover explicit roots, private permissions and corrupt
or unreadable storage; existing desktop fixtures retain selection, login and
visible snapshot compatibility. See
[ADR 0074](../decisions/0074-injected-account-registry.md).

## Account login lifecycle

`accounts::login::LoginState` owns one pending login per execution host, its
status and cancellation flag. `LoginService` receives that state, the account
registry and `LoginEffects`. Native `AccountAuthentication` is injected at
composition and returns identity metadata only; CLI output and credentials stay
inside the provider adapter. Provider/method validation, installation probes and
ID generation remain at the edge.

Beginning a managed login checks global admission, registers a disconnected
account or validates a reconnection, and reserves the login under the admission
lock. It then refreshes quota scheduling before checking active turns. A busy
account is refused, clears admission, refreshes quotas again and publishes.
Otherwise the pending login is published before the host starts authentication.
No host effects run while the admission or registry lock is held.

Authentication runs once for a live attempt on its owning host's blocking
executor. The native adapter captures its profile, prepares credentials and runs
the existing provider login. After success, cancellation is checked before
committing identity and incrementing revision. Only a successful commit forgets
the old quota. Login never automatically selects the new account. Cancellation
or authentication/storage failure preserves the previous identity, revision and
selection, including the disconnected registration created before login.

The host calls `finish` after the worker stops, including an executor join error.
Cleanup clears only the matching attempt, then refreshes quotas and publishes.
Repeated or stale cleanup cannot clear a later login. Cancellation requests do
not release admission while native work remains active. Cancellation is best
effort once the identity commit has started, preserving the prior behavior.

Selection refuses the account being logged in; removal and external attachment
refuse any pending login. Attachment persists the explicit external account
without authentication or selection. These operations preserve existing effect
ordering and error codes. `LoginStatus` retains its `{ id, provider }` IPC shape.
There are no new commands, stored fields or provider capabilities.

The desktop facade retains its singleton composition and bounded shutdown wait;
it supplies working-turn observations, quota operations, publication and Tauri's
blocking executor. A cancelled outer request or an incorrectly composed host is
not handled by automatic cleanup: the host must finish after native work stops.
Native profile implementation now follows the boundary below; tool materialization,
standalone runtime composition and Windows transport remain pending. See [ADR 0075](../decisions/0075-injected-account-login.md).

## Native account profiles

`prometeu-profiles` is a Tauri-free Unix crate implementing `ProfileBackend` for
profile resolution, preparation and child environment application. `Profile`
contains the existing captured account ID, provider, revision, home and managed
flag. These are local execution-side types, not serialized bridge commands.
`NativeProfiles` registers separate provider implementations and receives the
application root, provider homes, external home and a `ProfileFiles` dependency.

Claude materialization shares the existing resource directories, filters
alternative credentials from derived preferences and copies only MCP/project
trust into local configuration. Codex shares rollouts/resources and the SQLite
index, forces private file authentication and the OpenAI provider, and removes
alternative endpoints/profile selection. Existing credentials are never copied
or replaced. Conflicting resource paths remain errors. Child environment rules
retain configured homes and remove managed profiles' alternative credentials;
external Antigravity keeps its environment and rejects managed preparation.

`ProfileFiles` owns private directory creation and atomic file writing; it returns
already encoded application errors. Native profile code owns reads, parsing and
Unix links. The desktop implementation delegates to the existing `paths.rs`
writer, retaining 0700/0600 permissions and error behavior. Preparation is not a
transaction across every link/configuration file and adds no rollback semantics.

`main.rs` creates one profile backend after adopting the login PATH and injects
it into `NativeProviders` and `NativeAuthentication`. Those operations capture
selected account metadata and resolve/prepare it through the interface. Errors
prevent further command configuration or authentication. The startup backend
captures its roots at composition; environment changes require recomposition.
The account facade re-exports `Profile` and composes the same implementation for
catalog/naming/plugin consumers, preserving their current call-time discovery.

`npm run test:profiles` runs without GUI libraries or installed providers. Moved
Claude/Codex fixtures retain history and credential guarantees; added fixtures
cover independent roots/revisions, environment removal, file failures and
conflicts. Desktop tests verify private writes and injected startup/login
failures. Linux/macOS native CI tests and lints both process and profile crates.

Account selection, home discovery and private-file composition still have desktop
facades. Tool catalog/installation adapters and provider protocols retain desktop composition;
this adds neither a complete headless executable nor Windows profile execution.
Windows will use the planned execution boundary rather than Unix filesystem
adapters in the shell. See [ADR 0076](../decisions/0076-injected-native-profiles.md).

## Native startup tools

`prometeu-tools::StartupTools` separates provider startup from desktop MCP and
plugin modules. Claude receives a strict configuration path and plugin arguments;
Codex receives derived-home/canonical-ID/hook-ID artifacts and a separate MCP
TOML table/environment. `NativeProviders` receives the implementation at desktop
composition. Providers still assemble their own commands and protocols.
Desktop and WSL compose the shared `NativeTools` implementation with their own
catalogs, roots and private writes. Core `tool_resolution` shares selection,
provenance and exact-hash project trust. The WSL provider receives `ToolSelection`
to read current saved choices on each spawn; see
[conversation tools](windows-application.md#conversation-tools).

The port receives the resolved plugin and standalone-skill selections together.
Codex receives the captured profile and `config_scope` (or workspace ID as its
fallback), preserving account/workspace isolation. Profile preparation precedes
Codex plugin materialization; profile environment application precedes its MCP
materialization. Claude materializes tools before preparing the profile. A failure
stops subsequent preparation, without starting a conversation process. Antigravity
retains its unsupported-selection validation and does not invoke the tool port.

The crate's `McpMaterializer` receives `McpSources` for catalog/built-in resolution
and bearer refresh, and `McpFiles` for private atomic persistence. `None` does not
invoke either dependency; an explicit empty list still produces strict empty
configuration. Selected missing MCP IDs fail with the existing encoded error.
Claude retains unknown server fields and OAuth headers in its private JSON file.
Codex puts remote header values in process environment variables, replacing
case-insensitive Authorization headers with the refreshed bearer. Local overrides
use a private environment file and the existing `/bin/sh` wrapper; commands without
overrides run directly. Credentials are never inserted into command arguments by
these encoders. File errors pass through without a second encoding.

The desktop file implementation retains existing paths and 0700/0600 permissions.
Catalog discovery and OAuth now share native tools services with injected storage
and host browser consent. Embedded MCP lifecycle stays in desktop `mcp.rs`;
package preparation and cache installation use the native backend described
below; desktop Cloud effects remain in `plugins.rs`, while local plugin mutation
and Git import/update are shared through `prometeu-tools::plugins::PluginLibrary`. Deleted plugin entries retain their existing skip
behavior; explicit empty Codex packages still produce a derived home. These
operations add no transaction or rollback across artifacts.

`npm run test:tools` checks selection defaults, missing IDs, unknown fields,
OAuth placement, stdio quoting and write failures without Tauri or agent binaries.
Desktop fixtures retain filesystem permissions, built-in credential isolation,
plugin installation and native flag coverage. Injected startup tests check merged
packages, frozen scope, derived home, secret environment and failure ordering.
Linux/macOS CI also tests and lints the native crate independently. This boundary
changes no IPC/persisted format and adds no Windows shell or runtime transport.
See [ADR 0077](../decisions/0077-injected-startup-tools.md).

## Native package preparation

`prometeu-tools::packages::PackageBackend` supplies Claude package flags and Codex
package artifacts. `NativePackages` receives workspace-home/user-home roots,
marketplace namespace, catalog, private-file and installer interfaces, plus a
shared preparation gate. `NativeTools` receives the backend instead of invoking
desktop package preparation. The `Plugin` model is re-exported by the desktop;
its persisted shape and absent-field defaults are unchanged.

`FilePackageCatalog` reads an explicit path, retaining empty-on-missing-or-invalid
behavior. `PackageFiles` supplies 0700 directories and private atomic writes;
its errors are raw causes, encoded by the native operation exactly once.
`PackageInstaller` supplies cache listing, installation and best-effort removal.
Its native `CodexInstaller` receives the executable/marketplace, preserves
`CODEX_HOME` and plugin/hook CLI flags, and returns the existing encoded errors.
It retains blocking command execution without new timeout/output limits.

Absent selections skip effects. Explicit empty Codex selections still prepare a
derived home; deleted IDs are skipped and repeated IDs are deduplicated. Native
preparation preserves account/workspace paths, version revision 2, deterministic
hashes, copied manifests, hook IDs, shared credentials/cache links and retained
hook trust. The base configuration remains read-only. Installation failures
attempt removal of the failed canonical ID and stop preparation with the original
error. Cleanup stays best effort; no transaction or rollback is added.

The gate must be shared by instances using a shared cache. Desktop startup and
compatibility facades receive the same gate. Startup roots are captured at
composition while catalog contents are read per explicit preparation; desktop
cleanup/catalog facades keep call-time root discovery. Native filesystem work
uses Unix links and remains outside the portable core and bridge payloads.

Moved fixtures test manifest migration, hook detection, versions, account homes,
configuration preservation and cleanup without following links. Added tests
cover injected cache reuse/invalidation, empty/default selections, write/install
failures, catalog compatibility and the shared gate. A synthetic CLI verifies
native installer arguments, environment, results and errors. The desktop fixture
checks private-file permissions; opt-in real-CLI tests remain opt-in.

Agent-generated plugin creation and Cloud publication remain
desktop use cases. Local MCP catalogs and OAuth compose in WSL; built-in
service lifecycle remains a separate integration. See
[ADR 0078](../decisions/0078-injected-native-packages.md).

## Experimental execution host

`prometeu-runtime` composes the shared conversation lines, output/pump, workers
and native process implementation into a runnable Unix host. Provider
preparation, session storage, live delivery and task execution are injected.
`SessionStore` extends transcript persistence with native thread identity and
working-directory access; its isolated file implementation holds an exclusive
root lease. It deliberately does not adopt a desktop board/root.

The shared Codex protocol now lives in `prometeu-protocols`; the desktop and
headless adapters invoke the same implementation. The host injects the language
callback and client version. Canonical usage/measurement models moved to
`conversation::usage` with identical serde fields and validation; the desktop
telemetry facade re-exports its consumed types. No telemetry database or vendor
payload enters the core.

The [headless contract](headless-runtime.md) defines experimental stdio requests,
per-process generations, private storage and lifecycle limits. Executable tests
exercise launch/message/stop/restart/resume, busy admission, malformed requests,
root exclusion and a child holding inherited output. Shared adapter tests run
without Tauri. A real Codex conversation also resumed and recalled its prior turn
after host restart. This does not establish Windows or production parity.

## Compatibility and limits

This extraction changes no persisted field, provider identity, IPC command,
event payload or conversation protocol. Desktop re-exports retain source-level
imports while pointing at the shared definitions. Existing revival and tool
selection tests run against the actual core types. File-store tests retain
missing/corrupt-primary recovery and verify independent injected roots and
preservation of a valid backup after corruption.

`npm run test:core` runs without Tauri or GUI development libraries. The main
Rust workspace suite also includes it. CI independently tests and lints the
core on Linux and Windows. A manifest dependency allowlist and conservative
source-token checks reject known native APIs and platform selection in the
core. They are not a complete Rust semantic analyzer: aliases, macros and
dependency internals still require review. No Windows desktop build or WSL
transport is implemented by this boundary.

Evidence: `crates/core/src/board.rs`, `selection.rs`, `workspace_tools.rs`,
`publication/tests.rs`, `conversation/stream/tests.rs`, `conversation/work.rs`,
`crates/core/tests/boundary.rs`, `src/board_store.rs`, `src/transcript_store.rs`,
`src/chat.rs`, `src/dock.rs`, `crates/core/src/process.rs` and
`crates/process/src/tests.rs`, `crates/core/src/session/tests.rs`,
`crates/core/src/session/output.rs`, `src/chat/host.rs`,
`crates/core/src/terminal/tests.rs`, `crates/core/src/auxiliary.rs`,
`crates/process/src/terminal/tests.rs` and `crates/process/src/auxiliary/tests.rs`,
all under `src-tauri/`.

`Terminal::write_bytes` accepts raw terminal input without UTF-8 conversion.
The existing string `write` delegates to it, preserving desktop compatibility.
WSL terminal flow control and shell preparation belong to the execution host,
not the portable byte-retention core.
