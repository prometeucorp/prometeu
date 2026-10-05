# ADR 0009 — reusable commands and agents responsible for tasks

Date: 2026-09-05
Status: Accepted

## Context

The Open PR button sends a request to the current conversation. Prompts and
configurable agents need to be reusable without tying the task to a specific
button. One delivery may review, open a PR and react to comments and CI in the
same session.

## Options considered

1. Skills only: they reuse instructions, but do not represent model selection,
   tools, persisted execution and tracking by the software.
2. One agent per phase and a generic workflow editor: they require coordination
   and handoffs before any need for independent review exists.
3. Commands with two behaviors and reusable profiles: they integrate the chat,
   the menu and the PR button using existing sessions.

## Decision

Adopt the third option. Commands expand prompts or start agents in another tab.
Profiles have a prompt, model/provider/effort, MCP, plugins, skills and
permissions. Projects can override profiles. Executions store the
resolved configuration; the session is still the transcript.

A single session takes responsibility for the delivery. Review, publication and
fixes are profile instructions; they are not deterministic gates of a workflow
engine. A textual result from the agent does not prove review approval. The
example requires checking the published code and does not authorize a merge.

The app does not follow a task's PR on its own: there is no polling of comments
or CI and no automatic agent turn. An earlier optional PR monitor was removed
together with the versioned automation workflows so automation can be redesigned
from scratch; its persisted fields are ignored (see the contract). Adapters keep
materializing provider differences.

## Consequences

The person can start with a prompt or a task and reuse them across projects.
They do not need to create three agents to deliver a PR. A workflow editor and
handoffs between sessions remain a future evolution, without speculative
structures.

Feedback from PR comments and CI reaches a task only when the person sends it.
Skill selection is an instruction, not capability isolation.

The contract is in [actions](../contracts/actions.md). Persistence is additive;
old boards receive the one-time defaults from
[ADR 0010](0010-default-code-review.md), without creating tasks.

## Evidence

- [Commands and expansion](../../src/actions.test.ts).
- [Persistence, validation and deduplication](../../src-tauri/src/actions.rs).
- [GitHub reads and filtering](../../src-tauri/src/github.rs).
- [Registration, expansion and opening in another tab](../../e2e/actions.spec.ts).
