---
type: Decision
title: "ADR 0095 — Cursor CLI through ACP"
description: "Prometeu operates Cursor CLI through ACP with an explicit external account."
status: stable
---

# ADR 0095 — Cursor CLI through ACP

Date: 2026-10-25

## Context

Cursor is one more agent that a person installs locally. It has its own paid
login, native history and model identifiers. Prometeu must control the
lifecycle of the conversation and show interactive permissions. The
presentation must not get Cursor messages.

## Options considered

- Stream JSON in print mode: this mode is good for scripts. But it does not
  give the same contract for two-way approval and for the session lifecycle.
- Cursor ACP through stdio: this mode gives these items:
  - Session creation and load.
  - Streaming and cancellation.
  - Model selection.
  - Permission requests to the client.

## Decision

Use `cursor-agent acp`. This executable name is clear. Thus Prometeu does not
start a different program with the name `agent`. A Tauri-free adapter in
`prometeu-protocols::cursor` changes ACP and the question and plan extensions
of Cursor into the current V1 contract. The desktop composition prepares the
external profile and the application transcript. There is no generic ACP
framework and no more dependencies.

Attach the current CLI identity explicitly as the single external `cursor`
account. The selection is a different action. Do not copy credentials. Do not
promise that Prometeu isolates the managed account. Cursor owns the login and
the native history.

**Quotas.** Read the account quotas independently through the internal
dashboard API of Cursor. Use the same CLI access token. ACP does not have a
quota query.

- Use the current quota scheduler and window format again. Do not calculate the
  usage from conversations.
- The token gives read-only access, and Prometeu keeps it for a short time
  only. Prometeu does not import a browser session. It does not get a new
  token, and it does not add a login.
- Reject redirects. Keep credentials out of logs and IPC.

With this decision, Prometeu uses an endpoint without documentation. As a
result, Prometeu shows the limits that the account really has. When the
response is not available or changes, Prometeu keeps the last value. Unified
weekly plans must not use monthly summaries that are not current.

**Models.** Find the models through an ACP session without a prompt. Use the
accurate model IDs of that session. ACP `session/set_model` does not accept the
`models` aliases that the CLI shows to persons. In the version that the team
examined, Prometeu does not keep empty sessions. Queries have a time limit, and
they stop the process without inference.

**Session load and failures.**

- Stop the native replay while Prometeu loads a session. Prometeu replays its
  own V1 log.
- When a native load has a failure, Prometeu shows an error and keeps the
  history. It does not replace the history with a new conversation.
- When the handshake of the terminal has a failure, Prometeu closes the input.
  It sets the status of the connection to disconnected. Then the next message
  of the person can start the agent again through the shared host lifecycle.
  This occurs also when the previous child process did not stop.
- The current adapters keep their liveness, which comes from the process,
  through the default connected state.
- Ask profiles show permission requests. Usual automatic profiles select the
  one-time allow option that Cursor advertises.
- Prometeu sends a JSON-RPC error for unknown client requests. Thus these
  requests do not stop the operation without a time limit.

## Consequences

Cursor is available on the macOS and Linux desktop. Prometeu puts Hub MCP into
new and resumed sessions through ACP `mcpServers`.

Plugin and skill selections are not available:

- If a workspace has an explicit selection of plugins or skills that is not
  empty, the startup stops with an error.
- If the workspace has no selection on an axis, Prometeu uses an empty hub
  selection. It does not inherit packages that Cursor cannot accept.
- The configuration of Cursor stays active. Cursor adds the servers that the
  client gives to its own servers.
- Prometeu does not advertise an isolated configuration, a token and cost meter
  for each turn, a context report or a compaction control.
- Cursor owns the execution of subagents. Prometeu does not show a background
  lifecycle signal that the team did not examine independently.

Cursor questions accept option IDs. For a free-text reply, Prometeu uses the
field for the skipped cause that the extension supports. Thus the agent
receives the feedback of the person. A new V1 event or IPC command is not
necessary. The current account readers keep an unknown provider as data that
they do not read. Board readers from earlier versions can use Claude as the
default for an unknown agent. Thus an earlier version of Prometeu cannot open
Cursor boards correctly.

## Evidence

- `crates/protocols/src/cursor/tests.rs`: recorded native translation, replay
  suppression, permissions, questions, plans, cancellation, MCP `session/new`
  and `session/load` payload, and failures.
- `crates/tools/src/tests.rs`: ACP encoding of stdio, HTTP and SSE hub servers,
  OAuth header replacement, and refusal of Claude-only connectors.
- `crates/core/src/accounts/tests.rs`: explicit selection, removal, and the
  save and load of the Cursor identity.
- `src/conversation.test.ts` and `src/model-choice.test.ts`: V1 consumption and
  the accurate ACP model IDs that Prometeu keeps.
- `src-tauri/src/cursor/quota/tests.rs`: the weekly and monthly map,
  conservative failures, credential validation and the compatibility of the
  quota serialization. The ignored native quota check sends read-only requests
  without inference.
- The ignored `cursor::tests::live_cursor_round_trip` desktop test runs the
  installed authenticated CLI in a temporary folder. It examines tools, resume,
  plan mode and cancellation. It uses paid inference and is not a default
  gate.

See the [Cursor contract](../contracts/cursor.md) and
[provider matrix](../quality/provider-matrix.md#cursor-verification-boundary).
