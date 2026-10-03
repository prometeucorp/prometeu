# ADR 0081 — Supporting terminal over the WSL bridge

Date: 2026-09-28
Status: Accepted

## Context

The isolated WSL preview can run a conversation but cannot open a supporting
shell beside it. Terminal bytes, resizing and process ownership differ from
conversation commands and must not be represented as agent messages. Terminal
output can also outpace IPC and the browser renderer by a large margin.

## Decision

The execution host composes a single `TerminalService` from the existing
`TerminalFactory`, `TerminalOutput`, terminal control/waiter and `TaskExecutor`
ports, plus injected shell preparation and event delivery. Its login shell runs
in the runtime's canonical project directory. Shell selection happens in the
Unix composition root. The service is independent of the conversation lifecycle.

Expose a separate typed `TerminalClient` beside `SessionClient`; only their
connection owner combines them as `RuntimeClient`. Presentation consumes a
separate `TerminalPort` and injected `TerminalScreen`. The native adapter and
browser mock implement both narrow ports. xterm receives raw byte arrays rather
than strings decoded independently per frame.

Advertise the additive `terminal.v1` capability on the development v1 handshake.
Each terminal has a fresh opaque ID and ordered byte chunks. Snapshot replay and
live delivery use the same sequence, and stale IDs cannot address a replacement.
The host allows at most 64 unacknowledged chunks of at most 8 KiB. It waits for
credits outside the scrollback lock so snapshot, acknowledgement and close
commands remain available. xterm returns cumulative credits only after parsing
bytes. Closing wakes a credit-blocked reader to permit draining and reaping.

Input uses bounded byte frames, a bounded, ordered UI queue and a separate
bounded runtime writer queue, keeping credit and close requests available;
failures do not replay keystrokes. Resize and close are explicit operations. Close waits for
observed cleanup before retiring a handle; a timeout retains it for recovery.
Shutdown closes both the agent and shell. EOF closes them in disposable stdio
mode; resident attachments follow [ADR 0082](0082-resident-wsl-attachments.md). Conversation stop affects only the
agent, and terminal close affects only the shell.

## Compatibility and limits

Production desktop terminal IPC and persistence are unchanged. `Terminal::write`
retains its UTF-8 behavior through a new raw `write_bytes` method. Existing core
and Unix adapters are reused. The isolated preview adds typed IPC commands and
mock counterparts in its own registry. Old headless hosts remain usable for chat;
terminal controls require the advertised capability.

This does not introduce terminal persistence, multiple shell tabs or full Windows
support. Resident reattachment is governed by [ADR 0082](0082-resident-wsl-attachments.md). Scrollback remains bounded to 512 KiB and
can begin inside a multibyte character or escape sequence after truncation.
Flow control bounds in-flight terminal output, not the conversation stream or
all possible IPC callers. The runtime selects a responsive Unix PTY factory
with nonblocking descriptors and cancellation-aware streams; the original desktop factory remains unchanged.
Input admission and asynchronous failure reporting stay at the service boundary.
Detached process groups remain outside the existing PTY supervision guarantee.

## Evidence

- [Headless terminal contract](../contracts/headless-runtime.md#supporting-terminal).
- [Runtime/bridge integration tests](../../src-tauri/crates/runtime/tests/bridge.rs):
  raw bytes, working directory, resize, high-volume credits, exit status, stale
  IDs, conversation isolation and EOF while waiting for acknowledgements with
  a saturated input queue.
- [Presentation rules](../../src/wsl/terminal.test.ts): initial replay, split
  bytes, queue limits, failed writes, stale IDs, attachment races and renderer credits.
- The existing [preview browser scenario](../../e2e/wsl-preview.spec.ts) now checks
  xterm keyboard routing and preservation of the chat draft. Those focus and DOM
  behaviors require a real browser; protocol parsing remains covered below E2E.
