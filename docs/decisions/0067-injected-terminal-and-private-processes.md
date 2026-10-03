# ADR 0067 — Injected terminals and private authentication processes

Date: 2026-09-27
Status: Accepted

## Context

Supporting shells and setup/Run scripts still combined native PTY ownership,
byte retention and Tauri delivery. Authentication subprocesses depended on a
PTY signal helper despite having private pipes and no terminal. These effects
would otherwise need duplication in a headless execution host.

The PTY reader also marked a child stopped at EOF, before waiting for exit.
A live child that closed its descriptors could therefore disable shutdown.
Checking an atomic flag before signaling did not serialize signals with reaping.

## Decision

Define `TerminalFactory<Request>`, `TerminalControl`, `TerminalWait` and
`TerminalEvents` in `prometeu-core`. Requests remain native command builders at
the adapter boundary, not a bridge protocol. The core owns byte retention,
sequence numbers, output retirement and terminal handle behavior. The host
supplies its event sink and reader task; translated notices and setup callbacks
remain in desktop composition.

`prometeu-process::terminal::UnixTerminalFactory` implements PTY creation and
resizing using portable-pty. `main.rs` injects this factory. A single child owner
serializes group signals and nonblocking reaping under the same mutex. EOF does
not change running state. Closing requests SIGHUP, then the shared terminal
shutdown policy waits one second before SIGTERM and 500 ms before SIGKILL.
Dropping an unreaped waiter kills and reaps the child, including partial startup.

Core output holds the scrollback mutex through retention and event delivery.
Retirement takes that same lock, preventing an old reader from publishing output
or `pty-closed` after replacement. Setup callbacks still run outside that lock,
including after deliberate closure, preserving pending-prompt release.

Authentication uses a separate `AuxiliaryLauncher<Request>` and
`AuxiliaryProcess` port: private input, line polling and exit polling. The core
`AuxiliarySession` owns cancellation/deadline checks and outcome classification.
Provider adapters inject `UnixAuxiliaryLauncher`, retain their protocol and map
errors to existing i18n codes. Native pipes, process groups, bounded line reads
and drop cleanup live in the Tauri-free process crate. Private output never
uses conversation or terminal publication.

## Alternatives and limits

A common process interface for every command would hide meaningful differences:
terminal bytes must remain raw, authentication lines are private, and catalog
queries already have different total-output and nonblocking-write policies.
Keep those responsibilities explicit. Catalog queries and bounded Git/naming
helpers now use [ADR 0068](0068-bounded-command-and-query-ports.md). Ordinary
repository operations and system integration retain their adapters. Dock script selection,
workspace environment construction and setup reactions still use native services.

A native terminal child anchors its group only until reaped. Detached processes
and descendants surviving that point are outside this supervisor's guarantee.
Authentication cancellation and deadlines are checked between operations; native
writes remain blocking, so this does not guarantee interrupting a blocked write.
Output retains the existing 64-line queue and 1 MiB line limit, now checked before
unbounded allocation. Oversized/invalid lines end private output without a
partial frame. No terminal persistence, durable cursor, WSL daemon, reconnect
epoch or Windows desktop is introduced.

## Compatibility and evidence

IPC names, terminal event tuples, raw bytes, 512 KiB retention, exit-code shape,
script environments and persisted fields are unchanged. The deliberate fixes
are shutdown after EOF and suppression of retired terminal close events.

- [Portable terminal tests](../../src-tauri/crates/core/src/terminal/tests.rs): raw split UTF-8, snapshots, retention, retirement and shutdown delegation.
- [Real PTY tests](../../src-tauri/crates/process/src/terminal/tests.rs): input, resize, output, exit, EOF, descendants and abandoned ownership.
- [Private session policy](../../src-tauri/crates/core/src/auxiliary.rs) and [native tests](../../src-tauri/crates/process/src/auxiliary/tests.rs).
- Existing `accounts.rs` cancellation and `codex/account.rs` protocol fixtures exercise the injected native transport.
- [Core contract](../contracts/application-core.md#terminals-and-private-subprocesses).
