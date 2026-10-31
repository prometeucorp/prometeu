---
type: Decision
title: "ADR 0058 — Optional context evaluation behind an application-owned port"
description: "Optional context evaluation behind an application-owned port."
status: stable
---

# ADR 0058 — Optional context evaluation behind an application-owned port

Date: 2026-09-24

## Context

A task can start with an unresolved product decision or missing information
that changes the implementation. Discovering that after the agent has started
costs clarification turns and rework. TypeSafe can classify a text context and
answer several narrow, closed questions in one call with a confidence per
answer. Prometeu runs local CLIs and has no model of its own; any external
evaluation is a new trust boundary: it needs the person's own credential,
sends their draft off the machine and can fail independently of the app.

The first consumer is a missing-context review in the launcher for new local
workspaces. Other bounded classification features may follow, so the boundary
must not be shaped around one vendor payload or one screen.

## Options considered

1. **Call TypeSafe from the launcher.** Smallest change, but the key would
   reach the webview, vendor payloads would reach the presentation, and a second
   feature would copy the transport and error handling.
2. **A new conversational provider in the agent catalog.** Reuses accounts and
   the runtime port, but evaluation is not a conversation, has no process and
   must stay independent of the chosen coding agent.
3. **An application-owned evaluation port with one HTTP adapter at the edge.**
   The feature asks closed questions and applies its own rules; the adapter owns
   the credential, transport, retries, response validation and error
   translation.
4. **A general provider marketplace or orchestration layer.** Premature: one
   adapter and one consumer exist.

## Decision

Adopt option 3.

- `src-tauri/src/evaluation.rs` owns the port: `EvaluationRequest` (bounded
  text or structured JSON context plus at most eight closed questions), answers with outcome,
  confidence and optional effective model identity, request and response validation against the closed sets, the
  configuration generation, read together with the credential, that
  invalidates results started under an older configuration, and the
  application error codes `disabled`, `auth`, `rate_limited`, `unavailable`,
  `malformed`, `invalid` and `stale`. It has a fake for tests and a disabled
  path that makes no call.
- `src-tauri/src/typesafe.rs` is the only place that knows TypeSafe: the private
  credential file, the explicit enable flag, the HTTP adapter with timeouts and
  bounded retries, and the translation of vendor statuses into the codes above.
- `src/evaluation.ts` mirrors the port's IPC types for the frontend without I/O.
- A consumer is a pure feature: it builds the context, asks its own closed
  questions and applies its own rules. It never sees the key or a vendor
  payload.
- The integration starts disabled. Saving a key never enables it; removing the
  key disables it. Only an explicit action may send context. Creating
  workspaces, sending prompts and running agents never depend on it, and a
  failure never falls back to another service or provider.

The adapter uses TypeSafe's documented System One HTTP API pinned to `jev-1.13.0`.
Its request and response shape stays isolated in `typesafe.rs::wire`, described
in the [contract](../contracts/context-evaluation.md#typesafe-system-one-wire-shape).
The endpoint origin can be overridden with `PROMETEU_TYPESAFE_URL` (an origin
only: HTTPS, or loopback HTTP for local testing). A wire correction changes
only that module and its fixtures.

### Removed consumer

The first consumer was a missing-context **Review request** in the launcher,
with a TypeSafe section in Settings, per-language confidence thresholds and,
briefly, an opt-in local calibration history. Nobody used it, so all of it was
removed on 2026-10-22. The port, the adapter, the credential file and the IPC
commands stay for a future bounded classification feature; Settings keeps only
the key field and a **Test key** action that sends a fixed synthetic request. That feature must bring its own closed questions, selection rules and
measurement plan; the removed review's thresholds are not evidence for it.

## Consequences

- The port is tested with a fake and the adapter against a local HTTP server
  with synthetic responses.
- The key stays in a private backend file and never crosses IPC, the board, the
  relay, transcripts, logs or error messages.
- A new bounded classification feature can reuse the port and the adapter by
  supplying its own closed questions and selection rules.
- Without a product consumer, the port serves only the Settings key test. If no
  feature adopts it, remove it and retire this ADR.
- The wire shape was checked against the public API and live Choice requests,
  including one with eight questions, on 2026-09-24.

## Evidence

- [Port validation, disabled path and stale generations](../../src-tauri/src/evaluation.rs).
- [Credential lifecycle, secret redaction and adapter failures](../../src-tauri/src/typesafe.rs).
- TypeSafe [models](https://docs.typesafe.ai/models) and [HTTP API](https://docs.typesafe.ai/api), checked 2026-09-27: pinned version, effective identity and JSON state are supported. The new structured requests are validated with synthetic fixtures, not a live quality study.
- [Contract](../contracts/context-evaluation.md).
