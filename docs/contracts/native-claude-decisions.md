---
type: Contract
title: Native Claude decisions
description: Explicit responses to Claude PermissionRequest hooks for one process, while the native dialog stays available.
status: stable
---

# Native Claude decisions

The macOS terminal transport uses the `PermissionRequest` hook of Claude Code as
one more decision channel. The terminal continues to own the native dialog.
The verified CLI is **2.1.326**. A person can select the terminal mode in
Settings / General. The proofs of the native channel in this document do not
prove the full conformance of the Tauri app or of shared control.

## Ownership and identity

`terminal_agent_hooks.rs` authenticates the helper with these current items:

- The private Unix socket.
- The launch nonce.
- The app session.
- The link to the native session.

Only a Claude permission hook can
go into `terminal_agent_decisions.rs`. Each invocation gets a new UUID for the
generation of the process. The reply goes back through the same socket that the
callback used. Tool names, hashes of arguments and FIFO sequence do not connect a
response to a native request.

The PermissionRequest input of Claude does not include `tool_use_id`. Thus the
canonical `request.opened` has `toolId: null`, but it keeps its input and its
tool name. PostToolUse can independently give a tool ID from Claude. That subsequent
observation does not identify the permission invocation after the event.

Only the live process can own these request IDs. When the process stops, the
backend removes all the entries that wait for an answer. A response to a
withdrawn ID, or to an ID of a previous generation, gives an error. When
Prometeu opens a transcript again, it does not make a hook connection or a
decision channel again.

## Explicit responses

The usual `request.respond` path keeps the current authorization of the
application for local control and for shared control. The native edge changes
only explicit commands:

| Canonical response | Native PermissionRequest decision |
| --- | --- |
| Allow a regular tool | `behavior: allow`, without changed arguments |
| Allow ExitPlanMode | `behavior: allow` with the initial `updatedInput`, without changes |
| Answer AskUserQuestion | `behavior: allow` with the initial input and `answers` |
| Deny | `behavior: deny` with the supplied message |

The answers must include all the initial question strings without changes,
with string values. An empty allow cannot replace an answer. No response writes
`updatedPermissions`, changes the configured permission mode or adds a native
interruption. Claude keeps its own validation, deny rules and permission policy.
For example, some MCP tools must have an interaction with the user. This hook
cannot remove the native prompt for these tools. An allow that the hook sends
does not make sure that the prompt goes away.

The listener does not wait for the answer of a person. It starts a worker with
limits for that invocation, and it continues to receive the usual lifecycle
hooks. These limits apply:

- A maximum of 16 decisions that wait for an answer.
- A queue of canonical events with a maximum length.
- A maximum of 1 MiB for a native payload.
- A period of 300 seconds for an answer.

If there are too many decisions, Prometeu does not change the subsequent native
dialogs. The private native hook has a timeout of 305 seconds. The usual hooks
that only monitor keep their timeout of three seconds.

## Races, cancellation and acknowledgment

Claude runs PermissionRequest hooks at the same time as its native dialog. The
first applicable decision is the decision that Claude uses. The hook helper sends an acknowledgment
only after it writes and flushes the JSON decision to stdout. Then the host
sends `request.closed(outcome: resolved)`. This event means that the native hook
received the decision. It does not mean that Claude used the decision of the hook, that the tool ran
or that the native policy accepted an allow.

These conditions remove the control from the app:

- A timeout.
- A disconnection of the helper.
- The end of the process.
- A root Stop, SessionEnd or StopFailure.

They return an empty native response and close the app request as `cancelled`.
They do not make an allow or a deny.

After a local native selection, its hook can stay alive for a short time. Thus
the hook can supply a subsequent explicit reply from the app after that
selection. Before the verified native implementation applies the reply, it
makes sure that the initial dialog is active. This reply cannot go to a subsequent invocation.

Here, Prometeu uses the root Stop only to stop a decision channel that is
complete. It is not a new proof that the conversation is complete. The transcript
publication continues to use the different completion fence in the
[agent runtime](agent-runtime.md).

## Evidence and compatibility

The team examined the
[official hook contract](https://code.claude.com/docs/en/hooks#permissionrequest)
and the installed 2.1.326 implementation for these items:

- The execution of the dialog and the hook at the same time.
- The liveness of the initial request.
- The replacement of answers.
- A new analysis of the native rules.

Prometeu does not change the global native settings or the permission rules.

`terminal_agent_decisions/tests.rs` examines these items:

- How Prometeu changes a response into a native decision.
- The question keys, without changes.
- The identity for each invocation and each generation.
- The requests with limits.
- The withdrawal and the responses that are not current.

`terminal_agent_hooks/decisions_tests.rs` uses authenticated sockets. A question
that waits for an answer does not stop other observations. An explicit response
goes only to its own socket. A shutdown and a disconnection do not make a
decision.

The optional `terminal_agent_decisions/smoke.rs` uses the selected current
account in a scratch session that it owns. The two question scenarios were correct on
2.1.326:

- An explicit hook answer gave Blue.
- A local Blue selection stayed Blue after the hook supplied Green. The card closed
  as resolved.

A different Bash scenario did these steps:

1. It waited for the native permission dialog.
2. It examined the command, without changes,
   `printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt`.
3. It sent one explicit allow through the hook.

Native PostToolUse identified the same command and a tool ID from Claude. The scratch
file contained only the marker. The closed request rejected a second reply.
These scenarios got to a subsequent root Stop observation and stopped the PTYs that
they owned. The sanitized result fixtures are in
`terminal_agent_decisions/fixtures/`. This is evidence for the native channel.
It is not the full conformance of the application or of shared control.

When Ask and Plan operate together, Prometeu uses `--permission-mode plan`. It
does not enable the option that makes bypass available. Without this, the
pinned CLI opens a bypass consent screen before the prompt, also when the person selected Plan. Regression tests for new and resumed commands keep the manual policy.
A bounded ExitPlanMode run got to its native dialog for an empty plan. After the
native ToolSearch discovery, it sent the canonical plan request. An offline
regression test examines the selector for the dialog of the empty plan. Nobody
has verified the delivery of a deny or the behavior of an allow for
ExitPlanMode. Nobody approved a plan implementation.

This contract adds no IPC method and no stored provider payload. Canonical
requests use V1 again, with the additive `resolved` outcome in the
[conversation contract](conversation-events-v1.md). These proofs do not include
more combinations of native permission rules or ExitPlanMode responses after Prometeu
changes them into native decisions. The [provider matrix](../quality/provider-matrix.md) records the
current verification of the application.
