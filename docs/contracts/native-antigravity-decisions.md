---
type: Contract
title: "Native Antigravity decision boundary"
description: "Verified decision channels and the remaining boundary for an embedded Antigravity terminal."
status: stable
---

# Native Antigravity decision boundary

The first terminal version intentionally leaves Antigravity approvals, questions
and plan decisions in its own TUI. Prometeu has no attached decision transport
for this provider. Recreating these controls is outside the accepted first-version
scope; other terminal integration and conformance requirements still apply.
Chat retains its existing NDJSON capabilities.

Application request cards and notch controls cannot answer these native requests. Shared
views receive `decision_surface: "terminal"` inside the encrypted announcement,
show that the owner must answer, and hide response actions even for members with
Control. Viewer, owner and native backend independently reject unsupported
responses. Messages, verified interruption, notes and native keyboard input keep
their existing behavior. Parameterized tests in `conversation-capabilities.test.ts`,
`team-owner.test.ts`, `island.test.ts` and
`chat::terminal::tests::native_only_decisions_reject_application_responses_without_disabling_native_input`
cover both Antigravity and Cursor; encrypted and legacy compatibility coverage
remains in the shared viewer, channel and relay tests.

The existing authenticated hook bridge is an observation transport; authentication
and bounded delivery do not create an upstream response API.

## Verified native channels

The [hook contract](https://antigravity.google/docs/hooks) lists `SessionStart`,
`PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation` and `Stop`.
It does not define a parallel `PermissionRequest` event. `PreToolUse` can gate
execution and rewrite arguments. Its documented `allow` continues through later
hooks/default policy; `auto_approve` bypasses a default prompt while preserving
explicit rules. These are policy inputs before execution, not answers to an
identified, already-open native dialog. Holding this hook is not established as
a way to keep local and Prometeu decisions available simultaneously. The hook
schema has no answer result for `ask_question`.

The [ACP server](https://antigravity.google/docs/ide/extensions/acp/) exposes
permissions and elicitation to an editor over stdio. `agy acp` starts that server
as a subprocess; the editor renders its interaction. The documented flags do
not attach it to an existing interactive TUI. The
[headless stream](https://antigravity.google/docs/cli/headless/) explicitly
rejects `control_request` and `control_response` with an error and process exit.

Antigravity does have a parallel native channel:
[interactive Remote Control](https://antigravity.google/docs/remote-control)
keeps the local TUI and Google's web UI synchronized, including approvals and
questions answered from either surface. It uses an opt-in reverse tunnel and
the same Google account; account or organization policy may disable it. These
docs provide a product flow, not a public local attachment API for Prometeu.
Prometeu does not activate that tunnel or depend on its private wire protocol.

## Installed-version evidence

The read-only audit on 2026-11-17 inspected the installed macOS arm64 `agy`
version `1.5.10`, SHA-256
`780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798`.
`--help` exposes `acp`, `--remote-control` and the remote-control daemon, but no
local TUI attach endpoint. The embedded Go symbol table includes
`hookcaller.CallPreToolHook`, `hooks.NewPreToolHookConverter`,
`store.(*Manager).clearRemotelyResolvedInteractions` and
`acp/server.(*Server).requestPermissionFromClient`. These corroborate separate
hook, remote synchronization and ACP paths; symbol names alone do not prove
their ordering or a supported third-party API.

The binary's embedded hook guide differs from the current web guide about the
meaning of `allow`. No response bridge relies on that ambiguity. No permission
or question inference, tunnel connection, configuration mutation or private
session read was performed for this audit. A public attachment contract, or a
separately authorized product change, is still required before adding Prometeu
response buttons for native Antigravity sessions.
