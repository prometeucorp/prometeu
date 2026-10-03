# ADR 0023 — Serialize snapshot publication and conversation delivery

Date: 2026-09-07
Status: Accepted

## Context

Capturing board snapshots under a mutex did not order their later enqueueing
and emission. A paused publisher could overwrite a newer snapshot. Likewise,
conversation sequence assignment under a mutex did not order emissions made
after unlocking. Receivers could discard a late lower sequence. Writing to an
agent before recording its prompt also allowed an immediate response to appear
first.

## Options considered

1. Add revisions and reject stale board publications at every consumer.
2. Introduce separate publication and conversation delivery queues.
3. Serialize each existing publication path at its owner.

## Decision

The board saver uses one publication mutex across snapshot capture, enqueueing,
and event emission. The board mutex is held only while cloning; disk writes
remain on the existing worker. Explicit flushes use the publication mutex and
keep the worker available for later runtime changes.

Conversation delivery holds the existing per-conversation `Lines` mutex across
recording, sequence assignment, and emission. Command writes hold the same
mutex until their local events are recorded. A failed write records no prompt.
Stdout and stderr each have a dedicated raw reader feeding their existing event
processor. These readers keep draining while command writes hold publication
locks, preventing a full stdin/stdout pipe cycle. EOF drains queued lines before
the existing process cleanup. State reactions execute after conversation locks
are released. Machine status
clones the board before inspecting processes, avoiding the opposite lock order.

## Consequences

No revision field, wire format, or persisted migration is required. Two standard
library channels and reader threads per process separate pipe draining from
event processing.
Publication and command writes serialize within their respective owners.
Callbacks inside these critical sections must not reenter those owners. Slow
child writes delay publication, but not pipe draining. Read-ahead channels are
unbounded: sustained output during a stalled consumer can grow memory usage.
A bounded channel would recreate the pipe deadlock when full. If measured
stalls require a memory ceiling, spool pending output to disk while preserving
prompt-before-response and snapshot/live ordering.

Rollback reverts code without rewriting stored data. Existing board and
transcript compatibility tests remain applicable.

## Evidence

- [Board publication and flush regressions](../../src-tauri/crates/core/src/publication/tests.rs).
- [Concurrent emission, fast response, and failed-write regressions](../../src-tauri/src/chat.rs).
- [Persistence contract](../contracts/persistence.md).
- [Conversation flow](../architecture/conversation-flow.md).
