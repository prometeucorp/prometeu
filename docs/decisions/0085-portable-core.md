---
type: Decision
title: "ADR 0085 — Portable core with injected effects"
description: "A portable core that receives its effects through ports."
status: stable
---

# ADR 0085 — Portable core with injected effects

Date: 2026-09-27

## Context

The application rules must run, and tests must examine them, without Tauri and
without the process that shows the window. These items used the Tauri state,
Unix effects and desktop singletons directly:

- the board
- the sequence of conversation messages
- the supervision of processes
- the coordination of sessions
- terminals
- accounts
- the preparation of tools

If Prometeu moved only their structs, those dependencies stay through the types
that the structs refer to and through helper calls.

## Options considered

- Add platform branches to the current desktop services, and port each Unix
  operation independently.
- Add a full runtime and transport before Prometeu isolates the ownership of
  state.
- Move each boundary into a portable core. The core receives its effects
  through narrow ports. Validate the core independently and keep the local
  deployment.

## Decision

Use the third option, one boundary at a time.

`prometeu-core` owns the saved models, the portable rules and the sequence of
operations. It has no Tauri, file system, subprocess, socket or OS dispatch
code. Its manifest accepts only serialization and UUID libraries. Each boundary
specifies ports that the application owns. Each port has only the functions
that its callers use. There is no general method dispatcher, service locator, universal
`Platform` object or ambient singleton. The composition roots select the
implementations. These roots are `src-tauri/src/main.rs` and the host for
remote machines of [ADR 0101](0101-remote-machines.md). Native requests, for
example a prepared `Command`, are general parameters that stay in the adapter.
They do not go through IPC.

The table below shows the adapters of the desktop. It includes the crates
without Tauri that use the core. The host for remote machines shares only
`prometeu-core` and `prometeu-process` from the workspace.

| Boundary | Core ports | Desktop adapter |
| --- | --- | --- |
| Board and publication | `BoardStore`, `BoardEvents`, `BoardPublisher` | desktop `board_store.rs`, `state.rs` |
| Conversation stream | `ConversationInput`, `TranscriptStore`, `ConversationEvents`, `Clock` | `transcript_store.rs`, `chat.rs` |
| Processes | `ProcessLauncher`, `ProcessControl`, `ProcessWait`, `TaskExecutor` | `prometeu-process` |
| Terminals and private pipes | `TerminalFactory`, `TerminalEvents`, `AuxiliaryLauncher` | `prometeu-process` |
| Commands with limits | `QueryLauncher`, `CommandRunner` | `prometeu-process` |
| Session host, pump, launch and workers | `SessionHost`, `SessionLifecycle`, `SessionPump`, `LaunchService`, `ConversationWorkers` and their effect ports | desktop `chat/host.rs`, `session/launch.rs` |
| Provider preparation | `ProviderPreparation`, `AgentProtocol`, `AgentInput` | `agent_launch.rs`, `prometeu-protocols` |
| Accounts and login | `AccountStore`, `AccountAuthentication`, `LoginEffects` | `prometeu-files`, `account_login.rs` |
| Profiles and tools | `ProfileBackend`, `StartupTools`, `PackageBackend`, MCP sources and files | `prometeu-profiles`, `prometeu-tools` |
| Files and Git | `ProjectFiles`, `RepositoryGit`, `WorktreeCleanup` | `prometeu-files`, `prometeu-git` |

The [application-core contract](../contracts/application-core.md) gives the
full contract of the ports. The guarantees below are the policies that this
change repaired or kept. A change to one of them must have its own evidence.

### Ordering and ownership

- `BoardPublisher` puts the snapshot capture, the queue and the delivery in
  sequence. Each board write goes through it. This includes the lazy assignment
  of terminal ports. If a failure occurs in the storage of a live worker, the
  publisher returns the failure. It does not start a second write in parallel.
- The host holds one conversation mutex during these operations: a command
  write, the local acceptance events, the transcript append and the live delivery. The host
  tries the save and the delivery one time each. Their failures are diagnostics,
  and the buffer and the sequence continue. Private telemetry does not go into
  the public history.
- Each execution host owns its `InputGates`, its registry, its readiness state
  and its background work. Hosts do not share ambient state. The exit cleanup
  compares an identity token of the process under the registry lock. Thus a
  previous process cannot clear or close the process that replaced it. The
  publication and the reactions run after Prometeu releases the transcript and
  capture locks. Port implementations must not get these locks again.
- Input that waits keeps its sequence during setup, account changes and sends
  that have a failure. If a failure occurs during the launch preparation, the
  current process does not change. If a failure occurs when the process starts,
  the queue stays. A new start does not resume only because a file is there.

### Processes and commands

- To stop an agent, Prometeu closes the input and waits 2 s. Then it sends
  SIGTERM, waits 500 ms, and then kills the process. To close a terminal,
  Prometeu sends hangup and waits 1 s. Then it sends SIGTERM, waits
  500 ms, and then kills the process.
- Signals and reaping use the same mutex. Only an exit that Prometeu sees
  removes the running state. EOF does not remove it. If Prometeu discards a waiter
  that did not reap its child, the waiter kills and reaps the child. This also
  applies after a start that is not complete.
- A PTY waiter releases each master descriptor before a cleanup with force. This
  includes streams that the host keeps. Thus the kernel exit does not wait on an
  open PTY. When a conversation closes, Prometeu disconnects the
  terminal IO. It does not start the different shutdown policy of the dock. When
  a process stops usually, Prometeu can continue to read its output.
  Descendants that continue after the reaped leader, or that disconnect from
  it, are not in the guarantee. Native pipe drains keep their queues without a limit.
- `TaskExecutor` does not run a job inline. If it rejects a job, it discards the
  job before it returns, and thus it releases the waiter of the job. If a
  failure occurs in the reader schedule, the launch stops with an error and
  Prometeu removes the child. If a failure occurs in the shutdown schedule, Prometeu sends a request to kill the process
  immediately.
- A query has one deadline and one total limit for stdout. The deadline also
  includes blocked writes.
- A finite command declares these items: a timeout, the capture, discard or
  inherit mode for each stream, and explicit input. The command closes the input
  after it sends it. Its deadline includes the input, the output and the exit.
  While a captured pipe stays open, the command does not reap the leader.
- A nonzero exit is a result. It is different from a failure of the start, a
  timeout, an output limit or an I/O failure. Deadlines apply to the I/O after
  the start. They do not apply to the creation of the process.
- These are the current policies:

  | Operation | Policy |
  | --- | --- |
  | Model catalog | 20 s and 1 MiB |
  | Naming | 60 s and 1 MiB |
  | GitHub action polling | 30 s and 8 MiB for each stream. A stderr overflow is a response error. |
  | Preparation fetch | 10 s, with stdin closed and inherited output. After a nonzero exit, Prometeu can use the current ref. A timeout keeps `err.git.fetchSlow`. |
  | MCP inspection | 25 s and 1 MiB |
  | Plugin and worktree Git | 20 s and 256 KiB for each stream |

- Private authentication pipes keep a queue of 64 lines and 1 MiB for each line.
  Prometeu examines these limits before it gives memory to a line. Prometeu examines
  cancellation and deadlines between operations. They do not stop a native
  blocked write. Terminals keep 512 KiB of raw bytes. A retired
  terminal does not publish late output or a late closure.

### Accounts and tools

- Prometeu imports the default external accounts only when the account storage
  is missing. Load and validation failures stay errors. They do not set the storage
  back to empty or replace it. An update clones the data, applies the change, validates
  it and saves it if it changed. Then it replaces the data in memory. Different
  instances that share a file do not operate together.
- The login operation does these steps in this sequence. It reserves
  admission. It refreshes the quotas before the check for an active turn. It
  publishes the pending state before the authentication. It examines
  cancellation before it saves the identity. The host completes only after the
  native work stops. A cleanup of an expired try cannot remove a subsequent
  try.
- The preparation of profiles, MCP servers and packages keeps 0700 directories
  and 0600 files. It does not add a transaction across many files or a
  rollback. Hosts that share an installed package cache share one preparation
  gate. Prometeu captures the roots during composition.

## Consequences

These changes did not change an IPC command, a saved field, a provider flag or
a V1 event. Desktop re-exports keep the source imports stable. These were the
intentional repairs:

- shutdown after stdout EOF
- no closure event from a retired terminal
- the completion of resumed assistant activity through `Work::observe`
- cleanup with limits for naming, fetch and GitHub polling

The production desktop continues to run in one process, as
[ADR 0050](0050-tested-application-boundaries.md) specifies. The host stays
responsible for the serialization and for the correct composition of
non-reentrant effects. The core types alone do not prevent a dangerous
composition. The manifest allowlist of the core, the source-token checks and
the crate-graph guard in `npm run architecture:check` are careful checks. They
do not replace a review.

## Evidence

- [Application-core contract](../contracts/application-core.md) and
  [dependency rules](../architecture/dependency-rules.md).
- [Core boundary checks](../../src-tauri/crates/core/tests/boundary.rs).
- Portable tests adjacent to each module in
  [`crates/core/src`](../../src-tauri/crates/core/src/lib.rs).
- Tests with actual subprocesses, PTYs and pipes in
  [`crates/process`](../../src-tauri/crates/process/src/tests.rs).
- Adapter test suites in `crates/profiles`, `crates/tools`, `crates/files` and
  `crates/git`. The current desktop fixtures keep the IPC and format
  compatibility.
