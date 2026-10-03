# ADR 0072 — Conversation workers through an injected executor

Date: 2026-09-28
Status: Accepted

## Context

The pump and launch workflow were portable, but initialization and the stdout,
stderr and exit workers still lived in the desktop chat module. Shutdown and
pending input also created threads directly. A headless composition needed the
same ordering and process ownership without depending on Tauri.

## Decision

`prometeu-core::session::workers::ConversationWorkers` owns initialization and
both output-consumer jobs. Provider adapters supply line translation and stderr
filtering. The workers publish canonical events through `SessionPump`, whose
injected clock also timestamps startup and stderr notices.

Initialization emits `session.state: starting`, requests `commands.list`, and
feeds local echoes before scheduling readers. Initialization failure is a
lifecycle diagnostic and does not disable output. Stderr filtering preserves
nonblank accepted lines as `provider.stderr` notices. Stdout drains translated
frames, waits for termination, releases the waiter and only then invokes
`WorkerLifecycle::exited` with the process identity and wait result. The desktop
callback retains `SessionHost`'s identity check before closure publication.

`TaskExecutor` accepts independent background jobs without executing inline or
waiting for completion. Rejection must drop the job before returning, releasing
captured resources. `prometeu-process::ThreadExecutor` implements it with native
threads and reports thread-creation errors. Desktop composition injects one
executor for conversation readers, pending-input dispatch and agent shutdown.
Native pipe drains are already running before these jobs are scheduled.

Failure to schedule either reader fails launch through the existing spawn error
mapping. The owned waiter is released and cleans up the child, including when
the other reader was already accepted. Shutdown closes provider input first and
schedules the existing grace/escalation policy. If scheduling fails, it requests
immediate kill through `ProcessControl`; the owning waiter still reaps. Failed
pending-input scheduling reports a diagnostic and leaves the queue intact.

## Compatibility and limits

IPC, V1 frames, stderr filtering, initialization commands and normal shutdown
delays are unchanged. Thread-creation failures now follow explicit cleanup paths
instead of thread-spawn panics. Wait errors reach the lifecycle port; the desktop
retains its prior closure behavior after waiter cleanup and does not invent an
exit code. Unknown/stale identities still cannot close a replacement.

This is shared worker orchestration, not an executor for every application job.
The native pipe queues retain their existing unbounded-drain behavior. Scheduling
is not a transaction across both jobs; an accepted stderr job can finish while a
failed launch cleans up. Reader activation still precedes registry installation,
and retirement does not cancel reactions already in flight. Native provider
configuration now uses [an injected preparation boundary](0073-injected-provider-preparation.md).
Its account/tool dependencies, feature composition, terminal ownership and
transport remain before a production headless runtime. ADR 0050's deployment decision remains.

## Evidence

- [Workers and lifecycle port](../../src-tauri/crates/core/src/session/workers.rs).
- [Controlled worker tests](../../src-tauri/crates/core/src/session/workers/tests.rs).
- [Executor port](../../src-tauri/crates/core/src/tasks.rs).
- [Shutdown scheduling and failure test](../../src-tauri/crates/core/src/process.rs).
- [Native executor and process ownership](../../src-tauri/crates/process/src/lib.rs).
- [Application contract](../contracts/application-core.md#conversation-workers).
