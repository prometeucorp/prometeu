# ADR 0085 — Portable board models and injected publication effects

Date: 2026-09-27
Status: Accepted

## Context

A native Windows desktop with WSL execution needs application services that can
run outside Tauri. The board was coupled to desktop persistence/publication and
referenced types declared inside actions, delegation, Linear and telemetry
adapters. Moving only its struct would preserve those indirect dependencies.
The workspace tool use case already supplied explicit state, but still imported
this desktop-owned board.

## Options considered

- Add platform branches to existing desktop services and port Unix operations
  individually.
- Introduce a complete runtime/transport before isolating its state ownership.
- Extract the board boundary with injected effects and validate it independently
  while retaining the local deployment.

## Decision

Use a Cargo workspace library, `prometeu-core`, for the persisted models and
their portable rules. Move the referenced model types together; keep application
launch construction and process-dependent delegation observation outside them.
Preserve source imports through small desktop re-exports during incremental
extraction.

Define narrow application-owned `BoardStore` and `BoardEvents` ports. The
composition root supplies the file store with its explicit root; a Tauri edge
adapter supplies event delivery. `BoardPublisher` owns the existing serialized
snapshot/queue/publication ordering and coalescing worker. All board writes,
including lazy terminal port allocation, go through that publisher.

Implementation selection belongs to composition and adapters. Core rules cannot
reach Tauri, native system APIs or OS dispatch. Test with injected stores and
event sinks, and build/test the library independently on Linux and Windows.
The current deployment remains in-process, consistent with
[ADR 0050](0050-tested-application-boundaries.md). Future WSL transport and
process ownership require their own implemented contracts and evidence.

## Consequences

Board compatibility and tool-selection tests no longer require GUI libraries.
The desktop uses the same tested models and publisher rather than a parallel
implementation. Publication failures remain independent of persistence; durable
storage failures are returned without silently retrying a live worker's write.
The file adapter preserves backup recovery and private atomic storage.

Moving related types touches several desktop modules but changes no IPC or
persisted formats. This does not make sessions, process lifecycle, accounts or
the rest of the desktop independent of Tauri, and it does not ship Windows
support. The dependency guard has explicit limits; native builds and review
remain necessary.

## Evidence

- [Application-core contract](../contracts/application-core.md).
- [Core library](../../src-tauri/crates/core/src/lib.rs).
- [Publication ports and service](../../src-tauri/crates/core/src/publication.rs).
- [Publication tests](../../src-tauri/crates/core/src/publication/tests.rs).
- [Dependency checks](../../src-tauri/crates/core/tests/boundary.rs).
- [File-store adapter and tests](../../src-tauri/src/board_store.rs).
- [Desktop event adapter](../../src-tauri/src/state.rs).
