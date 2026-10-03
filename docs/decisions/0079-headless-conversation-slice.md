# ADR 0079 — Deliver a headless conversation slice

Date: 2026-09-28
Status: Accepted

## Context

Repeated boundary extraction had not demonstrated a working execution host.
The Windows/WSL effort needs an observable conversation before extending every
feature or finalizing a production bridge.

## Decision

Add the experimental Unix `prometeu-runtime` executable. It composes the existing
process launcher, shutdown policy, conversation lines, output/pump and workers
with injected provider preparation, session storage, event delivery and task
execution. It owns one conversation and accepts typed development requests over
stdio. A private root lease and persisted native thread identity support stop
and resume across host restarts without adopting desktop state.

Move the existing Codex protocol and its fixtures to `prometeu-protocols` and
consume that same implementation from desktop and the headless provider.
Language selection and client version are supplied by the host; desktop retains
its dynamic language callback and application version. Canonical usage model
types move into the portable conversation core, retaining serde shapes and
validation; telemetry storage stays on desktop.

This is a narrow executable delivery milestone. Production desktop remains
in-process under ADR 0050. The development request envelope is explicitly not
the final Windows/WSL transport contract. Other providers and optional features
remain unsupported by this executable until composed through their interfaces.

## Compatibility and limits

Desktop IPC, provider JSON-RPC, persisted board/transcript formats and protocol
translation remain unchanged. Headless metadata introduces its own versioned
format in a separate root. No existing root migration is performed. No new
platform branches enter application rules; the Unix executable and portable
protocol crate have separate build targets.

The initial adapter uses native Codex authentication/configuration and Ask
approvals. It does not inject Prometeu account/tool selections. Root exclusivity,
per-process event generations and observed shutdown are implemented; resident
reconnection, orphan recovery, outbound backpressure, complete command parity,
Windows shell and installation are still required before production use.

## Evidence

- [Runtime and injected dependencies](../../src-tauri/crates/runtime/src/lib.rs).
- [Shared protocol](../../src-tauri/crates/protocols/src/codex.rs).
- [Executable lifecycle tests](../../src-tauri/crates/runtime/tests/lifecycle.rs).
- [Wire, storage and operation contract](../contracts/headless-runtime.md).
- Real Codex conversation and successful same-thread recall after runtime restart
  were verified on Linux in addition to deterministic fixtures.
