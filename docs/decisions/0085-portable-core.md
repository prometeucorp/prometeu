# ADR 0085 — Portable core with injected effects

Date: 2026-09-27
Status: Accepted

## Context

A native Windows desktop with WSL execution needs the application rules to run
outside Tauri and outside the process that draws the window. The board,
conversation ordering, process supervision, session coordination, terminals,
accounts and tool preparation were coupled to Tauri state, Unix effects and
desktop singletons. Moving only their structs would keep those dependencies
through referenced types and helper calls.

## Options considered

- Add platform branches to the existing desktop services and port Unix
  operations individually.
- Introduce a complete runtime and transport before isolating state ownership.
- Extract each boundary into a portable core whose effects are injected through
  narrow ports, validate it independently and keep the local deployment.

## Decision

Use the third option, one boundary at a time.

`prometeu-core` owns persisted models, portable rules and sequencing. It has no
Tauri, filesystem, subprocess, socket or OS dispatch, and its manifest is limited
to serialization and UUID libraries. Each boundary defines application-owned
ports sized for its callers; there is no generic method dispatcher, service
locator, universal `Platform` object or ambient singleton. Composition roots
(`src-tauri/src/main.rs`, the runtime host and the Windows shell) select the
implementations. Native requests such as a prepared `Command` are generic,
adapter-local parameters; they never cross IPC or the WSL bridge.

Native effects live in Tauri-free adapter crates that depend inward on the core,
so the desktop and the WSL runtime inject the same implementations:

| Boundary | Core ports | Native adapter |
| --- | --- | --- |
| Board and publication | `BoardStore`, `BoardEvents`, `BoardPublisher` | desktop `board_store.rs`, `state.rs` |
| Conversation stream | `ConversationInput`, `TranscriptStore`, `ConversationEvents`, `Clock` | `transcript_store.rs`, `chat.rs` |
| Processes | `ProcessLauncher`, `ProcessControl`, `ProcessWait`, `TaskExecutor` | `prometeu-process` |
| Terminals and private pipes | `TerminalFactory`, `TerminalEvents`, `AuxiliaryLauncher` | `prometeu-process` |
| Bounded commands | `QueryLauncher`, `CommandRunner` | `prometeu-process` |
| Session host, pump, launch and workers | `SessionHost`, `SessionLifecycle`, `SessionPump`, `LaunchService`, `ConversationWorkers` and their effect ports | desktop `chat/host.rs`, `session/launch.rs`; runtime host |
| Provider preparation | `ProviderPreparation`, `AgentProtocol`, `AgentInput` | `agent_launch.rs`, `prometeu-protocols` |
| Accounts and login | `AccountStore`, `AccountAuthentication`, `LoginEffects` | `prometeu-files`, `account_login.rs` |
| Profiles and tools | `ProfileBackend`, `StartupTools`, `PackageBackend`, MCP sources/files | `prometeu-profiles`, `prometeu-tools` |
| Files, Git and catalogs | `ProjectFiles`, `RepositoryGit`, `CatalogStore`, `WorktreeCleanup` | `prometeu-files`, `prometeu-git`, runtime |

The detailed port contract is the
[application-core contract](../contracts/application-core.md). The guarantees
below are the policies the extraction fixed or preserved; changing one needs its
own evidence.

### Ordering and ownership

- `BoardPublisher` serializes snapshot capture, queueing and delivery. Every board
  write, including lazy terminal port allocation, goes through it. A live
  worker's storage failure is returned without a competing second write.
- The host holds one conversation mutex across a command write, local acceptance
  events, transcript append and live delivery. Persistence and delivery are each
  attempted once; their failures are diagnostics while the buffer and sequence
  advance. Private telemetry never enters public history.
- Each execution host owns its `InputGates`, registry, readiness and background
  work; hosts share no ambient state. Exit cleanup compares an in-process
  identity token under the registry lock, so a predecessor cannot clear or close
  its replacement. Publication and reactions run after transcript and capture
  locks are released; port implementations must not re-enter them.
- Pending input keeps its order across setup, account changes and failed sends.
  Failed launch preparation leaves the old process intact; failed spawning keeps
  the queue. Fresh starts never infer resume from an existing file.

### Processes and commands

- Agent shutdown closes input, waits 2 s, terminates, waits 500 ms, then kills.
  Terminal close sends hangup, waits 1 s, terminates, waits 500 ms, then kills.
- Signals and reaping share one mutex. Only an observed exit clears running state;
  EOF does not. Dropping an unreaped waiter kills and reaps its child, including
  after partial startup. Descendants that outlive the reaped leader, or detach,
  are outside the guarantee. Native pipe drains keep their unbounded queues.
- `TaskExecutor` never runs a job inline; a rejected job is dropped before
  returning, releasing its waiter. A failed reader schedule fails the launch and
  cleans up the child; a failed shutdown schedule requests an immediate kill.
- A query has one deadline and one total stdout limit; the deadline also covers
  blocked writes. A finite command declares a timeout, capture/discard/inherit
  per stream and explicit input that is closed after sending; its deadline covers
  input, output and exit, and it keeps the leader unreaped while a captured pipe
  is open. A nonzero exit is a result, distinct from spawn, timeout, output-bound
  and I/O failures. Deadlines bound post-spawn I/O, not process creation.
- Current policies: model catalog 20 s and 1 MiB; naming 60 s and 1 MiB; GitHub
  action polling 30 s and 8 MiB per stream (stderr overflow is a response error);
  preparation fetch 10 s with closed stdin and inherited output, where a nonzero
  exit still allows the existing ref and timeout keeps `err.git.fetchSlow`; MCP
  inspection 25 s and 1 MiB; plugin and worktree Git 20 s and 256 KiB per stream.
- Private authentication pipes keep a 64-line queue and 1 MiB per line, checked
  before allocation. Cancellation and deadlines are checked between operations
  and do not interrupt a blocked native write. Terminals retain 512 KiB of raw
  bytes; a retired terminal publishes no late output or closure.

### Accounts and tools

- Only missing account storage imports the default external accounts. Load and
  validation failures stay errors and never reset or overwrite storage. Updates
  clone, apply, validate, save if changed, then replace memory; there is no
  coordination between separate instances sharing a file.
- Login reserves admission, refreshes quotas before the active-turn check,
  publishes the pending state before authenticating and checks cancellation
  before committing identity. The host finishes only after native work stops;
  stale cleanup cannot clear a later attempt.
- Profile, MCP and package preparation keep 0700 directories and 0600 files and
  add no multi-file transaction or rollback. Hosts sharing an installed package
  cache share one preparation gate. Roots are captured at composition.

## Consequences

These extractions changed no IPC command, persisted field, provider flag or V1
event. Desktop re-exports keep source imports stable. Deliberate fixes were
shutdown after stdout EOF, suppression of retired terminal closure, settlement of
resumed assistant activity through `Work::observe` and bounded cleanup for
naming, fetch and GitHub polling.

The production desktop still runs in-process under
[ADR 0050](0050-tested-application-boundaries.md). The host remains responsible
for serialization and for composing non-reentrant effects correctly; the core
types alone do not prevent an unsafe composition. The core's manifest allowlist
and source-token checks, and the crate-graph guard in `npm run architecture:check`,
are conservative and do not replace review. Reusing these ports in the Windows
composition is decided in [ADR 0084](0084-shared-windows-desktop.md).

## Evidence

- [Application-core contract](../contracts/application-core.md) and
  [dependency rules](../architecture/dependency-rules.md).
- [Core boundary checks](../../src-tauri/crates/core/tests/boundary.rs).
- Portable tests beside each module in
  [`crates/core/src`](../../src-tauri/crates/core/src/lib.rs).
- Real subprocess, PTY and pipe tests in
  [`crates/process`](../../src-tauri/crates/process/src/tests.rs).
- Adapter suites in `crates/profiles`, `crates/tools`, `crates/files` and
  `crates/git`; existing desktop fixtures retain IPC and format compatibility.
