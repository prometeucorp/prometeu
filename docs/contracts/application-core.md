---
type: Contract
title: "Portable application core"
description: "The ports of the core, their desktop adapters and the obligations of each implementation."
status: stable
---

# Portable application core

The app obeys this contract. [ADR 0085](../decisions/0085-portable-core.md)
records the decision.

`src-tauri/crates/core` (`prometeu-core`) holds the stored models, the portable
rules and the sequences that the desktop and the host of the remote machine use
(`prometeu-host`, [ADR 0101](../decisions/0101-remote-machines.md)). It does not
contain Tauri, file system access, subprocesses, sockets or calls to the
operating system. It is a library in the Cargo workspace, not a deployed service. This
document is a map of its ports and of the obligations of their implementations.
It is not a format for data that goes between processes. The formats that go between processes are in the
[IPC](ipc.md), [persistence](persistence.md) and
[remote host](remote-host.md) contracts.

## Ports and adapters

The composition root `src-tauri/src/main.rs` selects the implementations of the
desktop.

| Area | Core ports and services | Desktop adapter |
| --- | --- | --- |
| Board | `BoardStore`, `BoardEvents`, `BoardPublisher` | `board_store.rs`, `state.rs` |
| Workspace supervision | `supervision::Binding`, `Event`, `Execution` and normalized `Observation` | remote host `supervision.rs`. Local execution is not available at this time |
| Workspace lifecycle | `workspace_lifecycle`, `WorktreeCleanup` | `session.rs`, `prometeu-git::cleanup` |
| Conversation stream | `conversation::stream::Lines`, `Work`, `ConversationInput`, `TranscriptStore`, `ConversationEvents`, `Clock` | `transcript_store.rs`, `chat.rs` |
| Session coordination | `SessionService`, `SessionReactions`, `SessionOutput`, `SessionHost`, `SessionPump` | `chat/host.rs` |
| Launch and workers | `LaunchService`, `ResumePreparation`, `ConversationLauncher`, `ConversationWorkers`, `TaskExecutor` | `session/launch.rs`, `prometeu-process::ThreadExecutor` |
| Provider preparation | `ProviderPreparation`, `AgentProtocol`, `AgentInput` | `agent_launch.rs`, `prometeu-protocols` |
| Processes and terminals | `ProcessLauncher`, `ProcessControl`, `ProcessWait`, `TerminalFactory`, `TerminalEvents`, `AuxiliaryLauncher` | `prometeu-process` |
| Bounded commands | `QueryLauncher`, `CommandRunner` | `prometeu-process` |
| Accounts and login | `AccountRegistry`, `AccountStore`, `LoginService`, `AccountAuthentication`, `LoginEffects` | `prometeu-files::accounts`, `accounts.rs`, `account_login.rs` |
| Profiles and tools | `ProfileBackend`, `StartupTools`, `PackageBackend`, `tool_resolution` | `prometeu-profiles`, `prometeu-tools`, `tool_materialization.rs` |
| Files, Git and repository | `ProjectFiles`, `ProjectEntries`, `ProjectSearch`, `RepositorySettings`, `RepositoryGit` | `prometeu-files`, `prometeu-git` |

Native request types (a prepared `Command` or a PTY `CommandBuilder`), recorded
profiles, paths and native handles are generic parameters that stay in the
adapter. No
port is a generic method dispatcher, and none of these values goes through IPC.

## Obligations of implementations

Supervision stores the execution observations and the outbox events that they
cause in one atomic operation, before the delivery. Provider hooks and HTTP
stay in the executor adapter. The core does not call the Cloud. The workspace, the executor and the supervisor each have an independent
identity.
See the [contract for the messages](supervision.md).

- Event, storage, diagnostic and publication ports must not enter again the
  operation that called them. Some of them operate under the lock of the
  conversation or of the registry. The host owns the serialization and the
  sequence of the locks.
- Stores make a difference between "missing" and "empty". They return load
  failures and not default values. Private writes use 0700 folders and 0600
  files with an atomic replacement. The stores return each error one time. If
  the port tells that the store must encode the error, the store returns it
encoded.
- A `TaskExecutor` accepts work and does not run it inline. It discards the
  rejected jobs before it returns.
- Process and terminal waiters own their child process. If the app discards a
  waiter before the waiter reaps the child process, the waiter kills and reaps
  the child. Control operations are only requests. Only the waiter can tell that
  the process stopped.
- Hosts that share an installed package cache share one preparation gate.

## Board and publication

`BoardPublisher` puts the snapshot capture, the queue, the delivery and the
durable barriers in sequence under one publication mutex. Clones share this
mutex. The disk I/O and the delivery operate when the board lock is free. `persist`
waits for the acknowledgment of the worker. If a live worker has a storage
failure, the method returns the failure and does not write a second time. A
synchronous fallback is possible only after the worker stopped. The allocation
of terminal ports uses the same barrier. `FileBoardStore` can read the second copy of the file when the primary file is
damaged. It
uses default values only when it cannot read the two files. `revive` operates
only after the process stopped unexpectedly.

## Conversation stream and pump

The host holds one conversation mutex for these operations:

- A command write.
- The local events that accept the input (`session.state` busy,
  `user.message` and adapter echoes).
- The transcript append.
- The live delivery.

A rejected command does not record data. Each public event gets one sequence number, also temporary events.
`telemetry.usage` gets no number. The host removes the `telemetry` and
`providerDurationMs` fields before storage and delivery. The host returns the
storage errors and the delivery errors independently, and it does not roll back
the buffer. The replay buffer keeps 4 MiB of full lines. When it loads, it keeps
mixed bytes of the earlier format and of V1, and it starts the transport
sequence again at 0.

`SessionPump` shares the readiness, the capture sequence, the sequence numbers
and the retirement with all its clones. It records accepted telemetry after
it releases the locks of the process and of the transcript. Reactions operate
after the capture gate. When a turn becomes stable, it schedules
input from the queue only when a queue is there and the setup does not operate.
When the send operates, it examines the admission again. If a write has a
failure, it puts back the previous turn flag and makes no capture or reactions.

`feed_committed` is the ingestion port for a host that must record a native
occurrence in a journal before the publication. Under the same capture and
stream locks, it gives the sanitized public line and the absolute line offset
to an injected commit callback. `Appended` publishes that line without a second
append to the store. `AlreadyCommitted` and an error cause no effects on the
buffer, the execution, the telemetry, the notifications or the queue. Temporary
events do not go through the durable commit. They keep their usual sequence
rules on this port.

`feed_observed_committed` uses the same internal path for reduction and
publication. But it calls the callback for each native fact. For a temporary
event, its public line is `None`. Thus, a receipt can control the capture of
context or of private usage, and the history does not keep that payload. The
host owns the source identities, the journal paths, the repair after a crash
and the decision to stop after a commit failure. It must load the history from the
journal before it accepts live ingestion. This port adds to the API and keeps
the default `feed` behavior. That behavior includes the live delivery after a
storage failure.

`with_commit` installs the same durable policy for all usual output and app
notices in a native terminal session. `command_committed` stages the full
canonical input batch. It commits the batch before it publishes the busy state,
the accepted input or the telemetry. The PTY write occurs one time. If the
commit has a failure, the host gets the error. The host then retires that
process and does not send the input again. `accept_committed` records a native
prompt that the agent accepted before, without a new write to the process.
`capture_committed` does not record accepted telemetry and reactions for a
receipt that it got before.

`revise_committed` publishes an authoritative content block, tool result, or
kept summary, notice or report. It causes no effects on the execution, the
telemetry or the reactions. The host must prove the same native occurrence, a
new version and a turn that stopped before. Usual chat keeps its current policy
and does not install the optional port. `session/pump/tests.rs` examines the two
policies, the removal of private fields, the suppression of duplicates and
the durable commit before reactions that enter again.

## Processes, terminals and commands

`StartedProcess` gives these items:

- The input.
- Two output streams that the app reads independently.
- A `ProcessHandle` with a stable identity in the process.
- The `ProcessWait` that owns the child.

The two output readers start before
the input returns. The core owns the stop policies of the agent and of the
terminal. The Unix adapter starts a process group. It sends the signals in sequence, and
it reaps the child without a block. It removes the running state only when it
sees the exit.

`StartedTerminal` gives a `ProcessHandle` for the same PTY child, together with
its resize and input ports. Thus, a conversation can keep the current process
identity, accounting and stop rules. It does not own a second process or reaper.
`TerminalWait` stays the single owner of the child. When a person closes a
conversation explicitly, the app calls `TerminalControl::disconnect`. This call
releases all PTY master descriptors and cancels the readers and writers that
wait. It does not schedule a second stop policy. The dock `close` keeps its
hangup and escalation policy.

Native IO does operations that do not wait, under short shared locks. Thus, a reader
or a writer that cannot continue does not stop the disconnection. If the app
discards a waiter before the waiter reaps the child, the waiter disconnects
first.
Then it kills and reaps the child. After a usual exit, the output stays
available to read. Native tests in `crates/process/src/terminal/tests.rs`
examine the shared exit state and the kept native handles during too much
output. They also examine the cancellation and the output after a usual exit. These Rust ports add
to the API and do not change stored formats or IPC formats.

`TerminalOutput` keeps 512 KiB of raw bytes. Each header, chunk or exit notice
gets one sequence number. Snapshots use the same lock. After retirement, the
output and the closure that come late do not show. `Terminal::write_bytes`
accepts raw input. `write` keeps its UTF-8 behavior. `AuxiliarySession` adds
cancellation, deadlines and typed errors to the private authentication pipes.
stdout stays private, and the session discards stderr.

`QueryPolicy` sets one deadline and one limit for all of stdout.
`CommandPolicy` sets a deadline, a capture, discard or inherit rule for each
stream, and the input bytes that it closes after it sends them. The results
show these conditions as different items:

- An exit code that is not 0.
- A timeout.
- An executable that is not available.
- An output limit.
- I/O failures.

[ADR 0085](../decisions/0085-portable-core.md#processes-and-commands) gives the
current limits for each caller.

## Sessions, launch and workers

Native queued input uses the `NativeDispatch` board value and the
`SessionRuntime::send_dispatch` port. These items add to the formats. The core
first stores one item that does not change. Then it sends the item. It removes
the item only after `Sent` or `AlreadyAccepted`, through a second durable board
barrier. The second value tells that an accurate receipt is there. It causes no
new process write and no accepted-input effects. After an error, the item stays
different from the text that a person adds to the queue after it. `SessionPublication::persist` gives
the barrier. The default implementations refuse this native path. Usual chat
keeps the current `send` path. The injected host owns the recovery when the
app does not know if the native delivery occurred. Core session tests examine these items:

- The storage occurs before the write.
- The identity stays stable after a revival.
- The queue that comes after the item stays.
- Duplicates cause no output.

`SessionService` receives the board and the ports for the runtime, the account,
the publication and the diagnostics. Empty input has no effect. Input that waits
goes to the end of the queue in sequence. It stays after setup, account changes
and send failures. After a send failure, the prompt goes back before the
subsequent input. Publication
occurs before revival, and it is not a durable save. `SessionReactions` updates
the tab status, the notes, the unread state, the context and the provider
identity. It does not change the workspace lifecycle. `SessionHost` owns the
registry, the `InputGates`, the readiness and the background work of one host.
The exit clean-up compares the identity under the registry lock and keeps a
stopped conversation for replay.

`execution.observed` updates the read-only native activity through those same
reactions. The work state keeps a block that the app sees in the native activity different
from the requests that a person can answer. Root activity cancels a completion that waits. Background
activity keeps it. Unknown evidence cancels the completion before an ambiguous
task drain. When the agent becomes available, the status changes. But the app
does not show a new unread indication and does not call completion actions.
SessionOutput puts these events in sequence. But Lines does not store them and
does not give them transcript positions. The app makes no assistant message for
them and no path for a response. The [conversation contract](conversation-events-v1.md) specifies
the event.

`LaunchService` records the workspace, the tools, the trust and the delegation
permission under one board lock. It prepares the launch after it releases the
lock. Only then does it retire the previous process. If the spawn has a failure, the queue and the
previous board state stay. After a correct spawn, the service does these
steps in this sequence:

1. It shows a warning about missing kickoff content.
2. It installs the conversation.
3. It updates the status and the note.
4. It publishes and signals the readiness.

`start` only installs. The caller publishes the new tab. Claude resumes when its
transcript is there. Codex and Antigravity resume from a saved identity. A new
start does not guess a resume.

`ConversationWorkers` sends `session.state: starting` and requests
`commands.list`. Then it schedules the stdout and stderr readers. Stderr text
that is not empty becomes `provider.stderr`. At the end of stdout, the worker
waits for the exit and releases the waiter before `WorkerLifecycle::exited`.
`ProviderPreparation` can write files and run bounded queries, but it does not
spawn the conversation. `AgentInput` holds the closure, the connection health
and the turn-wait policy. Thus, shared code does not compare a provider enum.
The connection health is "connected" by default for the current adapters. A
failure of the terminal protocol can disconnect before the process exits. The
desktop liveness then causes the next input to use the usual lifecycle that
starts the process again. That lifecycle stops the process first.

## Accounts and profiles

`AccountRegistry` keeps the current serde shape, the external default values,
the optional selection and the entries of unknown or retired providers. It does
not read these entries. Only when the storage is missing does it import the
default external accounts. An update clones the registry, applies the change,
validates the result, saves it if it changed and then replaces the memory.
Reads return copies. One instance puts its own updates in sequence. Different
instances that share a file do not know about the updates of each other.

`LoginService` keeps one login that is not completed for each host. It does
these steps:

1. It refreshes the quotas before it examines the active turn.
2. It publishes the login state before the authentication.
3. It examines the cancellation before it commits the identity and the revision.

The host calls `finish` after the native work stops, also after an executor
failure. An expired clean-up cannot remove the login that comes after it. The authentication
returns only identity metadata.

`ProfileBackend` resolves, prepares and applies native profiles from explicit
roots. Claude and Codex profiles share history and resources, but they do not
share inference credentials. The external Antigravity profile
rejects managed preparation. The preparation is not a transaction across more
than one file.

## Tools and packages

`tool_resolution` owns the resolution in this sequence: global, approved
project and workspace. It also owns the origin of each item and the project
trust that uses an accurate hash. `StartupTools` supplies these items:

- The Claude configuration and plugin arguments.
- The Codex plugin and MCP artifacts.
- The Cursor ACP `mcpServers`.

The Codex profile preparation occurs before the plugin materialization, and its
environment occurs before MCP. Cursor writes MCP before it resolves the external
profile. `None` selections call no source. Explicit empty lists make a strict
empty configuration. A missing selected MCP ID causes a failure. Codex supplies
remote MCP secrets in the environment of the child process. It supplies stdio
secrets in private environment files. It does not put secrets in command
arguments. For Cursor, the remote headers and the stdio environment
go in the ACP session payload.

`NativePackages` keeps the derived homes, the cache versions of revision 2, the
hashes, the manifests and the hook trust. If an installation has a failure, it
tries to remove that canonical ID and returns the initial error.
`PluginLibrary` and `SkillLibrary` share the local registration, the import and
update from Git, and the removal.

## Compatibility

These limits change no stored field, provider identity, IPC command, event
payload or conversation protocol. Desktop modules export again the moved types.
Thus, source imports continue to operate. The current provider, IPC and file
fixtures run against the shared definitions.

## Verification

- `npm run test:core`, `test:process`, `test:profiles` and `test:tools` run
  without Tauri, GUI libraries or installed agents. `npm run test:rust` includes
  them.
- The portable tests are adjacent to each module under `src-tauri/crates/core/src/`
  (`publication/tests.rs`, `conversation/stream/tests.rs`, `session/*/tests.rs`,
  `accounts/tests.rs`, `accounts/login/tests.rs`, `terminal/tests.rs`).
- Native tests: `crates/process/src/{tests.rs,terminal/tests.rs,command/tests.rs,query/tests.rs}`,
  `crates/profiles/src/tests.rs`, `crates/tools/src/{tests.rs,packages/tests.rs}`.
- `crates/core/tests/boundary.rs` examines the core manifest and the known
  native tokens. `npm run architecture:check` examines the dependencies between crates. These
  two checks are conservative and do not replace a review.
