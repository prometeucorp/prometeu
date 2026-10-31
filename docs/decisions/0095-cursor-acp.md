---
type: Decision
title: "ADR 0095 — Cursor CLI through ACP"
description: "Cursor CLI through ACP and an explicit external account."
status: stable
---

# ADR 0095 — Cursor CLI through ACP

Date: 2026-10-25

## Context

Cursor is another locally installed agent, with its own paid login, native
history and model identifiers. Prometeu needs conversation lifecycle and
interactive permissions without exposing Cursor messages to presentation.

## Options considered

- Print-mode stream JSON: useful for scripts, but does not provide the same
  bidirectional approval and session lifecycle contract.
- Cursor ACP over stdio: supplies session creation/loading, streaming,
  cancellation, model selection and client permission requests.

## Decision

Use `cursor-agent acp`. The unambiguous executable avoids launching an unrelated
program named `agent`. A Tauri-free adapter in `prometeu-protocols::cursor`
translates ACP and Cursor's question/plan extensions to the existing V1 contract.
Desktop composition prepares the external profile and application transcript.
There is no generic ACP framework or additional dependency.

Attach the existing CLI identity explicitly as the single external `cursor`
account. Selection remains a separate action. Do not copy credentials or promise
managed account isolation. Cursor owns login and native history.

Read account quotas separately through Cursor's internal dashboard API using the
same CLI access token. ACP has no quota query. Reuse the existing quota scheduler
and window format instead of estimating usage from conversations. The token is
read-only and ephemeral; no browser session import, token renewal or additional
login is introduced. Reject redirects and keep credentials out of logs and IPC.
This trades dependence on an undocumented endpoint for real account limits;
unavailable or changed responses preserve the previous reading. Unified weekly
plans must never fall back to stale monthly summaries.

Discover models through an unprompted ACP session and use its exact model IDs.
The CLI's human-readable `models` aliases are not accepted by ACP
`session/set_model`. Empty sessions are ephemeral in the verified version;
queries are bounded and terminate the process without inference.

Suppress native replay while loading a session; Prometeu replays its own V1 log.
A failed native load reports an error and keeps history, never silently replacing
it with a new conversation. Terminal handshake failures close input and mark the
connection disconnected, allowing the next user message to restart through the
shared host lifecycle even if the old child has not exited. Existing adapters
retain their process-based liveness through the default connected state. Ask
profiles surface permission requests; ordinary automatic profiles choose the advertised one-time allow option. Unknown client
requests receive a JSON-RPC error rather than blocking indefinitely.

## Consequences

Cursor is available on the macOS/Linux desktop. Windows/WSL still registers only
Codex. Hub MCP/plugin/skill selections are unavailable and explicit nonempty
workspace selections fail startup. Absent workspace axes resolve to empty hub
selections rather than inheriting global or project tools that Cursor cannot
accept; this applies on both startup and resume. Cursor's own configuration
remains active. No isolated configuration, per-turn token/cost meter, context report or compaction control is
advertised. Cursor owns subagent execution; no independently verified background
lifecycle signal is exposed.

Cursor questions accept option IDs. Free-text replies use the extension's
supported skipped reason field so the agent receives the user's feedback.
No new V1 event or IPC command is needed. Existing account readers preserve an
unrecognized provider opaquely; older board readers may default an unknown agent
to Claude, so opening Cursor boards in an older version is not supported.

## Evidence

- `crates/protocols/src/cursor/tests.rs`: recorded native translation, replay
  suppression, permissions, questions, plans, cancellation and failures.
- `crates/core/src/accounts/tests.rs`: explicit selection, removal and persisted
  Cursor identity round trips.
- `src/conversation.test.ts` and `src/model-choice.test.ts`: V1 consumption and
  preservation of exact ACP model IDs.
- `src-tauri/src/cursor/quota/tests.rs`: weekly/monthly mapping, conservative
  failures, credential validation and quota serialization compatibility; the
  ignored native quota check makes read-only requests without inference.
- The ignored `cursor::tests::live_cursor_round_trip` desktop test exercises the
  installed authenticated CLI in a temporary directory, including tools, resume,
  plan mode and cancellation. It uses paid inference and is not a default gate.

See the [Cursor contract](../contracts/cursor.md) and
[provider matrix](../quality/provider-matrix.md#cursor-verification-boundary).
