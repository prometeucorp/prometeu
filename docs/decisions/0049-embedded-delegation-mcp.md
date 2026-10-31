---
type: Decision
title: "ADR 0049 — Native MCP for client-owned delegation"
description: "Native MCP for client-owned delegation."
status: stable
---

# ADR 0049 — Native MCP for client-owned delegation

Date: 2026-09-18

## Context

An agent should be able to delegate work through Prometeu and inspect progress.
A workspace is an environment and may contain unrelated conversations. Treating
all tabs in a created workspace as subordinate agents would give the coordinator
control over conversations the person opened later. Process identity also
cannot express ownership because transcripts outlive their processes.

## Decision

Ship a virtual `prometeu` MCP in every installation, without adding it to any
default selection. Existing selectors activate it. The first version creates
one task-bound agent/conversation and an isolated workspace per delegation.
Ownership is persisted against an authenticated client identity and the exact
created conversation. Conversation context is optional. Existing internal
conversation IDs remain valid client identities; external registrations use
independent `client:<uuid>` identities scoped to explicit source projects. It does not extend to other workspaces, other tabs or
transitive descendants. Workers explicitly start with empty tool selections;
further delegation requires a person's tool selection.

Use a stdio mode of the existing executable, bridged to the open desktop app
through a private Unix socket and a client credential. Internal credentials
follow the provider process lifetime; external credentials survive app restarts
and can be revoked through local administration. A private discovery file lets
external stdio hosts reconnect without changing their configuration. The desktop remains
the owner of worktrees, processes and state. Provider adapters keep their
existing MCP materialization; the built-in registry row contains no credential.
No provider-native config is edited and no relay or Cloud API is added.

`mcp_access.rs` owns client identity, project scope and credential administration;
`delegation.rs` owns resource authorization and use cases; `embedded_mcp.rs` owns
the common protocol/transport. Neither use cases nor clients depend on a
provider protocol or frontend conversation. Both internal and external callers
enter the same dispatcher. Optional conversation context supplies repository,
model and permission defaults; external calls select an allowed project ID.
Local CLI registration prints a standard stdio server definition and does not
edit another application's settings. Credential management is not an agent tool. Workspace creation remains in `session.rs`. Add an optional
owner record to its internal creation path; allocate the worker conversation ID
before preparation and persist ownership together with the new workspace.
Execution IDs remain separate from conversation IDs. Observe canonical events
under their publication lock with only an in-memory board mutation, then
publish outside the conversation locks. This explicitly extends the previous
rule that all board reactions happened after unlocking: ordered execution
observation needs to precede a fast subsequent response. Lock order remains
chats → buffer → board, with no process/file operations while holding the board.

Creation and messages have request keys to make ordinary retries idempotent.
New MCP messages reject busy targets. Native pending-message recovery remains;
there is no exactly-once promise across provider/persistence crashes. Main turn
completion and observed background activity remain separate facts. Workspace
stage remains the person's decision.

Ordinary input accepted while an execution is running keeps that execution's
identity until the next observed turn completion. V1 has no correlation between
accepted messages and provider turns, so counting every overlapping message as
an independent running execution would leave IDs without a matching terminal.
Later provider activity uses the existing observed-continuation rule. MCP
messages remain idle-only, preserving their separate request-key identities.
This changes no persisted fields or conversation events.

Expose configured setup/Run execution, bounded dock logs and explicit preview
opening for the same owned agent IDs. Reuse the existing dock, PTY, script
configuration and workspace navigation instead of adding a shell-command API
or another process supervisor. Setup still runs automatically on creation;
explicit calls allow retrying it. Active scripts are reused under the shared
PTY lock, while a different active Run must be stopped before replacement.
Runtime status and exit codes remain ephemeral, with no invented success after
restart. Starting a process does not certify service readiness.

Preview opening deliberately changes the person's visible workspace. A local
event carries only the authorized workspace/conversation IDs; the desktop owns
navigation, layout and the URL derived from the reserved port. The MCP result
acknowledges the request, not page-load completion. No remote preview route,
arbitrary URL, JavaScript execution or new IPC command is added.

## Alternatives and consequences

A generic workspace CRUD/IPC bridge was rejected because it exposes unrelated
state and couples agents to presentation commands. A standalone daemon would
create a second lifecycle owner; an HTTP server adds an unnecessary network
boundary for the initial local-only feature. Pure stdio without a bridge could
not coordinate the already-running desktop state safely.

The shared executable avoids installation dependencies. It requires the desktop
to stay open. Internal callers require a fresh process credential after restart;
external registrations retain identity and rediscover the new socket. A daemon,
HTTP listener and separate distribution are unnecessary for local composition.
The socket credential authorizes the MCP API but is not a sandbox against local
processes with the user's filesystem permissions.

Persisted delegation records and explicit execution IDs add compatibility and
storage responsibilities. No new scheduler, automatic manager agent, hierarchy
UI, deletion tool, approval proxy or distributed execution is introduced.
Later support for several delegated agents sharing an environment can reuse the
separate identities without reinterpreting every existing workspace as an agent.

## Contract and verification

See [embedded MCP](../contracts/embedded-mcp.md) and the
[provider matrix](../quality/provider-matrix.md). Tests cover ownership of exact
conversations, restart/migration, MCP framing/lifecycle, tool selection and both
provider materialization paths, independent external clients, project scope,
credential revocation and socket rediscovery from an external stdio subprocess.
The integration test uses a socket fixture; live model execution remains a
separate manual check. The implementation does not require a new IPC
command: existing hub and board responses gain additive data.
