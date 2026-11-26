---
type: Contract
title: "Native Antigravity decision boundary"
description: "The examined decision channels and the current limit for an Antigravity terminal in Prometeu."
status: stable
---

# Native Antigravity decision boundary

In the first terminal version, Antigravity approvals, questions and plan
decisions stay in the TUI of Antigravity. This is intentional. Prometeu has no
decision transport that connects to this provider. The accepted scope of the
first version does not include these controls in Prometeu. The other
requirements for the terminal integration and for conformance continue to
apply. Chat keeps its current NDJSON capabilities.

The request cards and the notch controls of the application cannot answer these
native requests. Shared views receive `decision_surface: "terminal"` in the
encrypted announcement. They show that the owner must answer, and they do not
show response actions, also for members with Control. The viewer, the owner and
the native backend each reject a response that they cannot accept. Messages, the
verified interruption, notes and native keyboard input do not change.

Parameterized tests examine Antigravity and Cursor. These tests are in
`conversation-capabilities.test.ts`, `team-owner.test.ts`, `island.test.ts` and
`chat::terminal::tests::native_only_decisions_reject_application_responses_without_disabling_native_input`.
The shared tests for the viewer, the channel and the relay continue to examine
the encrypted and the legacy compatibility.

The current authenticated hook bridge is a transport that only monitors. The
authentication and the delivery with limits do not make an API that sends responses
to Antigravity.

## Verified native channels

The [hook contract](https://antigravity.google/docs/hooks) gives `SessionStart`,
`PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation` and `Stop`.
It does not specify a parallel `PermissionRequest` event. `PreToolUse` can stop
the execution and write the arguments again. The documented `allow` result
continues through the subsequent hooks and the default policy. `auto_approve`
does not show a default prompt, but it keeps the explicit rules. These results
are policy inputs before the execution. They are not answers to a known native
dialog that is open at that time. Nobody has shown that a held hook keeps the
local decision and the Prometeu decision available at the same time. The hook
schema has no answer result for `ask_question`.

The [ACP server](https://antigravity.google/docs/ide/extensions/acp/) gives
permissions and elicitation to an editor through stdio. `agy acp` starts that
server as a subprocess, and the editor renders the interaction. The documented
flags do not connect the server to an interactive TUI that operates at that
time. The [headless stream](https://antigravity.google/docs/cli/headless/)
rejects `control_request` and `control_response`. It shows an error and the
process stops.

Antigravity has a parallel native channel:
[interactive Remote Control](https://antigravity.google/docs/remote-control).
It shows the same state in the local TUI and in the web interface of Google.
This includes approvals and answers to questions from the two surfaces. It uses a
reverse tunnel that the person must enable and the same Google account. A policy
of the account or of the organization can disable it. These documents give a
product flow, not a public local API that Prometeu can connect to. Prometeu does
not enable that tunnel and does not use its private network protocol.

## Installed-version evidence

On 2026-11-17, a read-only audit examined the installed macOS arm64 `agy`
version `1.5.10`, with this SHA-256:
`780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798`.

`--help` shows `acp`, `--remote-control` and the remote-control daemon. It does
not show a local endpoint that connects to the TUI. The Go symbol table in the
binary includes these symbols:

- `hookcaller.CallPreToolHook`
- `hooks.NewPreToolHookConverter`
- `store.(*Manager).clearRemotelyResolvedInteractions`
- `acp/server.(*Server).requestPermissionFromClient`

These symbols agree with three different paths: hooks, remote synchronization
and ACP. But symbol names do not prove the sequence of these paths. They also do
not prove an API that third parties can use.

The hook guide in the binary and the current web guide give different meanings
for `allow`. No response bridge uses this unclear meaning. The audit did not
guess permissions or questions, connect a tunnel, change the configuration or
read a private session. Before Prometeu can add response buttons for native
Antigravity sessions, one of these items is necessary:

- A public contract to connect to the TUI.
- A product change with a different authorization.
