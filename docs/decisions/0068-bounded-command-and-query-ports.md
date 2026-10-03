# ADR 0068 — Bounded command and query ports

Date: 2026-09-27
Status: Accepted

## Context

Model discovery, workspace naming, background GitHub polling and preparation
fetches each owned subprocess loops in desktop features. Discovery needs
interactive private lines and one deadline across all pages; finite commands
need raw stdout/stderr and input closure. Reusing authentication's blocking
writes would weaken discovery's existing deadline guarantee.

Catalog teardown unconditionally signaled a PID after successful exit polling
could already have reaped it. Naming waited before draining stdout, risking pipe
backpressure, and its timeout killed without reaping. Background `gh` joined
reader threads after waiting for the leader, allowing inherited pipes to extend
an otherwise bounded call indefinitely.

## Decision

Add application-owned `QueryLauncher<Request>`/`QueryProcess` and
`CommandRunner<Request>` ports in `prometeu-core::command`. Query policy declares
one timeout and total stdout limit. Finite-command policy declares a timeout and
independent stdout/stderr capture, discard or inheritance, plus explicit input
bytes that are closed after sending. A nonzero exit is a result, separate from
spawn, timeout, output-bound and I/O failures. Native command builders remain
local adapter requests and do not cross IPC.

`prometeu-process` supplies Unix implementations. Queries keep bounded UTF-8
lines and deadline-aware nonblocking writes; finite commands progress both
nonblocking output pipes and input together. Their deadline covers input,
output and exit. A finite runner retains the unreaped group leader while any
captured pipe remains open, allowing timeout cleanup to reach descendants that
hold those pipes. Both adapters establish a cleanup owner immediately after
spawn. Reaping retires the signal target; drop never signals an already reaped
leader. Detached descendants remain outside the guarantee.

`main.rs` injects both implementations. Catalog commands receive the query
launcher; naming and action monitoring receive the command runner; workspace
preparation passes it through branch/worktree helpers into fetch. Provider
protocol parsing, account setup, command arguments, result interpretation and
error translation remain at the feature edge. No vendor or platform dispatch is
added to the core.

## Policies and compatibility

- Catalog: 20 seconds for the whole query; 1 MiB stdout across all lines,
  including delimiters; discarded stderr; unchanged pagination, schemas and
  catalog error codes. Deadline checks also cover blocked writes.
- Naming: 60 seconds; Claude receives the prompt on stdin with a new 1 MiB
  stdout bound; stderr is discarded. Codex keeps its existing final-answer file
  and discards both streams. Failures keep the fallback title; manual renames
  still win. The final-answer file is still read by the native naming adapter.
- Background GitHub action queries: 30 seconds and 8 MiB per captured stream.
  Stdout overflow keeps the response-error classification. Stderr overflow now
  rejects with that same classification instead of returning a truncated
  diagnostic. The deadline includes pipe draining, not just leader exit.
- Preparation fetch: 10 seconds, closed stdin and inherited output. A completed
  nonzero fetch still allows the caller to use its existing ref. Timeout keeps
  the existing `err.git.fetchSlow` code; cleanup now kills the group and reaps.

IPC, model descriptors, persisted state and conversation events are unchanged.
Existing provider fixtures and real-Git preparation tests retain compatibility
coverage. Injected runner tests verify fallback and error interpretation.

## Limits and evidence

This change extracts the bounded helper paths above. Ordinary repository Git
operations, authentication, system integrations and other command consumers
retain their existing adapters. It does not make the whole application a
headless service or add WSL transport. Native process creation and kernel-level
termination are not made cancellable by these ports; deadlines bound the
post-spawn I/O/polling loops.

- [Command ports](../../src-tauri/crates/core/src/command.rs).
- [Native finite-command tests](../../src-tauri/crates/process/src/command/tests.rs).
- [Native query tests](../../src-tauri/crates/process/src/query/tests.rs).
- [Catalog compatibility](../../src-tauri/src/agents/catalog_tests.rs).
- `command_port_tests` in `naming.rs` and `github.rs`, and `fetch_port_tests` in `session.rs` verify injected policy and application outcomes.
- [Core contract](../contracts/application-core.md#bounded-commands-and-queries).
