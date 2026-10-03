# ADR 0064 — Portable conversation ordering with injected effects

Date: 2026-09-27
Status: Accepted

## Context

After extracting the board, conversation ordering still lived beside Tauri and
Unix process handling. A future execution host must preserve accepted-input
ordering, snapshot sequences, private telemetry filtering and background-task
settlement without reproducing those rules in a bridge adapter.

## Decision

Move canonical event primitives, the `Lines` buffer, snapshot construction and
`Work` settlement into `prometeu-core::conversation`. Inject four narrow ports:
`ConversationInput`, `TranscriptStore`, `ConversationEvents` and `Clock`.
The existing desktop uses these implementations directly; there is no parallel
headless implementation or platform dispatch inside the core.

Keep the host's existing lock order. It holds one conversation mutex across a
command write, local acceptance events, transcript append and live delivery.
The explicit recording callback allows delegation to project execution state
in that order. Reactions and telemetry commits retain their existing positions
outside the transcript lock. Independent native pipe readers keep draining
while command delivery holds the lock.

Keep providers and files at the edge. `FileTranscriptStore` reads the supplied
seed and privately appends app-managed V1 logs; `ProviderTranscriptStore` reads
provider-owned history and never writes mirrors into it. The Tauri event adapter
retains the existing tuple payload and suppresses delivery from a replaced
process. The desktop supplies the system clock.

Both persistence and event delivery are attempted once. Their failures are
returned as delivery diagnostics while the live buffer and sequence advance.
The desktop retains its existing best-effort transcript policy: log storage
errors and continue, without retrying accepted input. Loading reports errors to
the caller; the desktop keeps its existing empty-buffer fallback.

## Alternatives and consequences

Moving the entire pump immediately would pull board reactions, telemetry,
accounts, actions and delegation into this change. Leaving ordering inside the
desktop would require duplicating it when building the execution host. The
narrow stream boundary lets these policies run without Tauri now while keeping
the current deployment and lifecycle explicit.

The host remains responsible for serialization and non-reentrant effect ports.
This preserves existing lock order but means the core type alone does not
prevent a caller from assembling an unsafe host. Concurrency and real pipe
tests verify the current integration. This decision does not provide transport
deduplication, reconnect epochs, process ownership or a WSL daemon. Those still
need separate contracts before deployment changes under ADR 0050.

No IPC, V1, board or transcript format changes. Existing mixed histories stay
byte-preserved, and transport sequences still reset when a process is restored.

## Evidence

- [Core contract](../contracts/application-core.md#conversation-stream).
- [Conversation stream and ports](../../src-tauri/crates/core/src/conversation/stream.rs).
- [Portable stream tests](../../src-tauri/crates/core/src/conversation/stream/tests.rs).
- [Settlement and tests](../../src-tauri/crates/core/src/conversation/work.rs).
- [File adapters and compatibility tests](../../src-tauri/src/transcript_store.rs).
- [Desktop integration and pipe regression](../../src-tauri/src/chat.rs).
