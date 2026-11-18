---
type: Contract
title: Native Claude decisions
description: Process-scoped explicit responses to Claude PermissionRequest hooks while preserving the native dialog.
status: stable
---

# Native Claude decisions

The macOS terminal transport uses Claude Code's `PermissionRequest` hook as an
additional decision channel. The terminal continues to own the native dialog.
The verified CLI is **2.1.326**. Terminal mode is selectable in Settings /
General; the native-channel proofs below do not establish full Tauri app or
shared-control conformance.

## Ownership and identity

`terminal_agent_hooks.rs` authenticates the helper with the existing private
Unix socket, launch nonce, app session and native session binding. Only a Claude
permission hook can enter `terminal_agent_decisions.rs`. Each invocation gets a
new UUID scoped to the process generation. The callback's exact socket carries
its reply; neither tool names, argument hashes nor FIFO ordering correlate a
response to a native request.

Claude's PermissionRequest input does not contain `tool_use_id`. The canonical
`request.opened` therefore has `toolId: null`, while its input and tool name are
preserved. PostToolUse may independently supply a real tool ID; that later
observation does not retroactively identify the permission invocation.

Only the live process can own these request IDs. Retirement removes all pending
entries, and a response to a withdrawn or previous generation's ID fails. A
restored transcript does not recreate a hook connection or a decision channel.

## Explicit responses

The ordinary `request.respond` path retains the application's existing local
and shared-control authorization. The native edge maps only explicit commands:

| Canonical response | Native PermissionRequest decision |
| --- | --- |
| Allow a regular tool | `behavior: allow`, without rewritten arguments |
| Allow ExitPlanMode | `behavior: allow` with the exact original `updatedInput` |
| Answer AskUserQuestion | `behavior: allow` with the original input plus `answers` |
| Deny | `behavior: deny` with the supplied message |

Question answers must cover the exact original question strings with string
values. An empty allow cannot substitute for an answer. No response writes
`updatedPermissions`, changes the configured permission mode, or adds a native
interrupt. Claude retains its own validation, deny rules and permission policy.
In particular, MCP tools marked as requiring user interaction cannot have their
native prompt bypassed by this hook; a delivered allow is not a promise that the
prompt will disappear.

The listener does not wait for a person's answer. It starts a bounded worker
for that invocation and continues receiving ordinary lifecycle hooks. There
are at most 16 pending decisions, a bounded canonical event queue, a 1 MiB
native payload limit and a 300-second answer window. Overflow leaves subsequent
native dialogs alone. The private native hook timeout is 305 seconds; ordinary
observation hooks retain their three-second timeout.

## Races, cancellation and acknowledgment

Claude runs PermissionRequest hooks alongside its native dialog. The first
applicable decision wins. The hook helper acknowledges only after writing and
flushing the JSON decision to stdout. The host then emits
`request.closed(outcome: resolved)`: this means delivery to the native hook,
not that the hook won, the tool ran, or the native policy accepted an allow.

Timeout, helper disconnection, process retirement and root Stop/SessionEnd/
StopFailure withdraw the app's control. They return an empty native response
and close the app request as `cancelled`; they never fabricate allow or deny.
A local native choice may leave its hook alive briefly. A subsequent explicit
app reply can consequently be delivered after that choice; the verified native
implementation checks whether the original dialog is still active before
applying it. This cannot target a later invocation.

Root Stop is used here only to retire an already-ended decision channel. It is
not a new completion proof for the conversation; transcript publication still
uses the separate completion fence in the
[agent runtime](agent-runtime.md).

## Evidence and compatibility

The [official hook contract](https://code.claude.com/docs/en/hooks#permissionrequest)
and the installed 2.1.326 implementation were checked for parallel dialog
execution, original-request liveness, answer replacement and native rule
reevaluation. No global native settings or permission rules are modified.

`terminal_agent_decisions/tests.rs` covers response mapping, exact question
keys, per-invocation/generation identity, bounded requests, withdrawal and stale
responses. `terminal_agent_hooks/decisions_tests.rs` exercises authenticated
sockets: a pending question leaves other observations responsive; explicit
responses reach only their own socket; shutdown and disconnection are neutral.
The opt-in `terminal_agent_decisions/smoke.rs` uses the selected existing account
in an owned scratch session. Both question cases passed on 2.1.326: an explicit
hook answer produced Blue; a local Blue choice remained Blue after the hook
delivered Green and the card closed as resolved. A separate Bash case waited
for the native permission dialog, checked the exact command
`printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt`, and sent one
explicit allow through the hook. Native PostToolUse identified the same command
and a real tool ID; the owned scratch file contained exactly the marker. The
closed request rejected a duplicate reply. These cases reached a subsequent
root Stop observation and reaped their owned PTYs. Sanitized result fixtures
are in `terminal_agent_decisions/fixtures/`; this is native-channel evidence,
not full application or shared-control conformance.

Ask combined with Plan uses `--permission-mode plan` without enabling the
bypass-availability option. The pinned CLI otherwise opens a bypass consent
screen before the prompt, even while Plan is selected. Fresh/resume command
regressions preserve the manual policy. A bounded ExitPlanMode run reached its
native empty-plan dialog and emitted the canonical plan request after native
ToolSearch discovery. The empty-plan dialog selector has an offline regression.
Real deny delivery and allow behavior for ExitPlanMode
remain unverified; no plan implementation was authorized.

No new IPC method or persisted provider payload is introduced. Canonical
requests reuse V1, including the additive `resolved` outcome documented in the
[conversation contract](conversation-events-v1.md). Broader native permission-rule
combinations and ExitPlanMode responses beyond mapping remain separate from
these proofs. Current application verification is recorded separately in the
[provider matrix](../quality/provider-matrix.md).
