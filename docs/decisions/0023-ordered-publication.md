---
type: Decision
title: "ADR 0023 — Serialize snapshot publication and conversation delivery"
description: "Each owner publishes board snapshots and conversation events in a constant sequence."
status: stable
---

# ADR 0023 — Serialize snapshot publication and conversation delivery

Date: 2026-09-07

## Context

The board saver captured snapshots under a mutex. But the mutex did not control
the sequence of the queue and send operations after the capture. Thus a
publisher that stopped for a time can write a previous snapshot on top of a
newer snapshot. Also, the conversation gave sequence numbers under a mutex, but
it sent the events after it released the mutex. Thus a receiver can ignore a
late event with a lower sequence number. The app also wrote to an agent before
it recorded the prompt. Thus a fast response can show before the prompt.

## Options considered

1. Add revisions and reject previous board publications at each consumer.
2. Add different queues for publication and conversation delivery.
3. Serialize each current publication path at its owner.

## Decision

The board saver uses one publication mutex for the snapshot capture, the queue
operation and the event emission. The board mutex stays locked only during the
clone. The current worker continues to do the disk write operations. Explicit
flush operations use the publication mutex. They keep the worker available for
runtime changes after the flush.

Conversation delivery keeps the current `Lines` mutex of each conversation
locked during the record, the sequence number and the emission. A command write
keeps the same mutex locked until the app records its local events. If a write
has a failure, the app does not record a prompt.

Stdout and stderr each have a dedicated raw reader. Each reader sends data to
its current event processor. These readers continue to read the pipes while
command writes keep the publication locks. This prevents a cycle where the stdin
and stdout pipes are full. At EOF, the app processes the lines in the queue
before the current process cleanup.

State reactions occur after the app releases the conversation locks. Machine
status makes a clone of the board before it examines processes. Thus the locks
do not occur in the opposite sequence.

## Consequences

No revision field, wire format or stored migration is necessary. Each process
has two standard library channels and reader threads. They keep the read
operations on the pipes isolated from the event processing.

Publication and command writes serialize in their owners. A callback in these
locked sections must not go into those owners again. Slow write operations to a
child process make publication slower, but they do not stop the read operations
on the pipes. The read-ahead channels have no limit. If a consumer stops and the
output continues, the memory usage can increase. A channel with a limit causes
the pipe deadlock again when it is full. If measured stops make a memory limit
necessary, write the output that waits to the disk. That solution must keep the
prompt before the response, and the snapshot before the live events.

A rollback changes the code back and does not write the stored data again. The
current compatibility tests for the board and the transcript continue to apply.

## Evidence

- [Board publication and flush
  regressions](../../src-tauri/crates/core/src/publication/tests.rs).
- [Concurrent emission, fast response, and failed-write
  regressions](../../src-tauri/src/chat.rs).
- [Persistence contract](../contracts/persistence.md).
- [Conversation flow](../architecture/conversation-flow.md).
