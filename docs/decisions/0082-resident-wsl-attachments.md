# ADR 0082 — Resident WSL execution with disposable attachments

Date: 2026-09-29
Status: Accepted

## Context

Restarting the stdio host restores a transcript but loses the live turn and shell.
The native preview needs to detach without changing execution ownership, while
keeping Unix transport details outside portable clients and presentation.

## Decision

The Windows composition injects `ResidentWslLauncher`. It runs the same Linux
executable as a disposable stdio proxy, which starts or attaches to a detached
host through a private Unix socket. The original `WslLauncher` and default stdio
mode remain available for disposable execution and compatibility tests.

A shared `Host` dispatches requests to the same injected conversation and terminal
services. `DeliveryHub` implements the existing runtime event port. It discards
unattached live delivery and detaches slow consumers rather than blocking or
failing transcript persistence. Only one client can attach at a time. The root
lease reserves execution ownership; transport attachment never takes it over.
Launch configuration must match before requests can be admitted.
The listener uses nonblocking acceptance; each accepted stream explicitly uses
blocking worker I/O and the contract's read/write deadlines. Socket mode is set
at the Unix adapter boundary rather than relying on platform-specific inheritance.

`AttachmentClient::disconnect` releases only the connection. Explicit shutdown
stops both children and the host; Stop remains scoped to the conversation.
Snapshots report current readiness and generation, and terminal discovery returns
the current ID and byte snapshot. Presentation buffers events during attachment,
replaces snapshots, then applies only newer sequence numbers. No mutation or
keystroke is retried automatically. Provider payloads remain at the adapter edge.

Detached terminals keep draining into bounded scrollback. Reattachment resets
output credits at the last published sequence; the renderer acknowledges its
replacement snapshot before continuing live consumption. Neither the renderer
nor conversation controller selects an OS, launcher or transport implementation.

## Trade-offs and limits

A resident host intentionally consumes resources after the window closes. The
preview exposes a separate translated shutdown control; there is no idle expiry.
A second client is rejected rather than silently taking control. Lost replies
still have an unknown outcome; operation receipts and transparent retries are
not introduced. Socket paths have the native Unix length limit.

This survives client/proxy loss, not runtime death, WSL shutdown or machine reboot.
The transcript survives; a shell cannot be recovered after host death. Cleanup
of descendants after an uncatchable host kill remains outside this development
slice. Bundled updates use capability-negotiated retirement at attachment: the
serialized host admits replacement only without retained conversations, live
shells or pending launches/application/MCP work/consent. Busy hosts defer replacement; older
hosts without retirement support still require explicit shutdown. This avoids
abandoning live execution when the content-addressed executable changes, at the
cost of retaining an older build until a later safe attachment. Persisted state
survives; stopped PTY scrollback remains memory-only. The exact compatibility
contract is in the resident contract.
Native Windows GUI validation and full application parity remain separate gates.

## Evidence

- [Resident contract](../contracts/resident-runtime.md): ownership, framing, bounds,
  permissions, compatibility and shutdown behavior.
- [Executable tests](../../src-tauri/crates/runtime/tests/resident.rs): detach in
  a turn, same generation and provider process, shell state and offline output,
  competing attachment, configuration mismatch, malformed input, proxy death and
  explicit shutdown and replacement admission across retained agents and both
  terminal types. The same reconnect fixture runs through actual `wsl.exe` on opt-in.
- [Delivery test](../../src-tauri/crates/runtime/src/resident.rs): saturated
  consumer detaches without failing execution delivery.
- [Session tests](../../src/wsl/session.test.ts) and
  [terminal tests](../../src/wsl/terminal.test.ts): snapshots racing events,
  readiness, stale identities and terminal reattachment.
- The existing [preview browser scenario](../../e2e/wsl-preview.spec.ts) verifies
  DOM replacement and keyboard/draft continuity across attachment.
