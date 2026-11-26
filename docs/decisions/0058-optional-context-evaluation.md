---
type: Decision
title: "ADR 0058 — Optional context evaluation behind an application-owned port"
description: "An optional context evaluation behind a port that the application owns."
status: stable
---

# ADR 0058 — Optional context evaluation behind an application-owned port

Date: 2026-09-24

## Context

A task can start with a product decision that is not complete, or without
information that changes the implementation. If the person finds this after the
agent starts, more turns and more work are necessary. TypeSafe can put a text
context into classes. In one call, it can answer some narrow, closed questions,
with a confidence value for each answer. Prometeu runs local CLIs and has no
model of its own. Thus each external evaluation is a new trust boundary:

- It must have the credential of the person.
- It sends the draft of the person out of the machine.
- A failure can occur in it independently of the app.

The first consumer is a review of missing context in the launcher for new local
workspaces. Other small feature that puts text into classess can come after it. Thus the
boundary must not agree with only one vendor payload or one screen.

## Options considered

1. **Call TypeSafe from the launcher.** This is the smallest change. But the key
   goes to the webview, and vendor payloads go to the presentation. A second
   feature must copy the transport and the error processing.
2. **A new conversational provider in the agent catalog.** This uses the
   accounts and the runtime port again. But an evaluation is not a
   conversation. It has no process, and it must operate independently of
   the selected coding agent.
3. **An evaluation port that the application owns, with one HTTP adapter.**
   The adapter is at the edge. The feature sends closed questions and applies
   its own rules. The
   adapter controls the credential, the transport, the retries, the response
   validation and the translation of errors.
4. **A general provider marketplace or orchestration layer.** This is not necessary.
   There is only one adapter and one consumer.

## Decision

Prometeu selected option 3.

- `src-tauri/src/evaluation.rs` contains the port. The port includes these
  items:
  - `EvaluationRequest`: a small text or structured JSON context, with a
    maximum of eight closed questions.
  - Answers with an outcome, a confidence value and an optional identity of
    the model that gave the answer.
  - Validation of the request and the response against the closed sets.
  - The configuration generation. Prometeu reads it together with the
    credential. A new generation cancels the results that started under an
    older configuration.
  - The application error codes `disabled`, `auth`, `rate_limited`,
    `unavailable`, `malformed`, `invalid` and `stale`.

  The port has a fake for tests and a disabled path that makes no call.
- `src-tauri/src/typesafe.rs` is the only module that knows TypeSafe. It
  contains the private credential file and the explicit enable flag. It also
  contains the HTTP adapter, with timeouts and a maximum number of retries, and
  the translation of vendor statuses into the codes above.
- `src/evaluation.ts` contains a copy of the IPC types of the port for the
  frontend. It does no I/O.
- A consumer is a pure feature. It makes the context, sends its own closed
  questions and applies its own rules. It does not see the key or a vendor
  payload.
- At the start, Prometeu keeps the integration disabled. When the person saves a key, this
  does not enable the integration. When the person removes the key, this
  disables it. Only an explicit action can send context. The creation of
  workspaces, prompts and agent runs operate without the integration. After a
  failure, Prometeu does not use a different service or provider.

The adapter uses the documented System One HTTP API of TypeSafe, at the pinned
version `jev-1.13.0`. Its request and response format stays isolated in
`typesafe.rs::wire`. The
[contract](../contracts/context-evaluation.md#typesafe-system-one-wire-shape)
gives this format. `PROMETEU_TYPESAFE_URL` can replace the origin of the
endpoint. This value must be an origin only: HTTPS, or loopback HTTP for local
tests. A correction of the wire format changes only that module and its
fixtures.

### Removed consumer

The first consumer was a **Review request** for missing context in the
launcher. It had a TypeSafe section in Settings and confidence thresholds for
each language. For a short time, it also had an optional local calibration
history. No person used it, so Prometeu removed all of it on 2026-10-22. The
port, the adapter, the credential file and the IPC commands stay for a
subsequent small feature that puts text into classes. Settings keeps only the key field
and a **Test key** action that sends one synthetic request that does not change. That feature
must have its own closed questions, selection rules and measurement plan. The
thresholds of the removed review are not evidence for that feature.

## Consequences

- Tests examine the port with a fake. They examine the adapter against a local
  HTTP server with synthetic responses.
- The key stays in a private backend file. It does not go through IPC, the
  board, the relay, transcripts, logs or error messages.
- A new small feature that puts text into classes can use the port and the adapter again.
  It must give its own closed questions and selection rules.
- Without a product consumer, only the key test in Settings uses the port. If no
  feature uses it, remove it and retire this ADR.
- On 2026-09-24, Prometeu compared the wire format with the public API and with
  live Choice requests. One of these requests had eight questions.

## Evidence

- [Port validation, disabled path and expired generations](../../src-tauri/src/evaluation.rs).
- [Credential lifecycle, removal of secrets from text, and adapter failures](../../src-tauri/src/typesafe.rs).
- TypeSafe [models](https://docs.typesafe.ai/models) and [HTTP API](https://docs.typesafe.ai/api), examined on 2026-09-27. The API accepts the pinned version, the identity of the model that answered, and the JSON state. Synthetic fixtures validate the new structured requests. A live quality study did not validate them.
- [Contract](../contracts/context-evaluation.md).
