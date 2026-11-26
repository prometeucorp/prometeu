---
type: Decision
title: "ADR 0049 — Native MCP for client-owned delegation"
description: "A native MCP server for delegations that a client owns."
status: stable
---

# ADR 0049 — Native MCP for client-owned delegation

Date: 2026-09-18

## Context

An agent must be able to delegate work through Prometeu and monitor that
work. A workspace is an environment, and it can contain conversations that
are not related to the delegation. If Prometeu used all the tabs in a new
workspace as subordinate agents, the coordinator can control conversations that
the person opened after the delegation. The identity of a process also cannot
tell the owner, because a transcript continues to be available after its
process stops.

## Decision

Include a virtual `prometeu` MCP server in all installations. Do not add it to
a default selection. The current selectors make it active. In the first
version, each delegation makes one agent and one conversation for one task, in
an isolated workspace. Prometeu saves the owner as an authenticated client
identity together with the conversation that it made. The context of a
conversation is optional. The current internal conversation IDs continue to be
correct client identities. External registrations use their own
`client:<uuid>` identities. Each of these identities applies only to the source
projects that the registration names. The ownership does not go to other
workspaces, other tabs or the descendants of the descendants. Workers start
with empty tool selections. A worker can delegate again only after a person
selects the tools.

Use a stdio mode of the current executable. This mode connects to the open
desktop app through a private Unix socket and a client credential:

- An internal credential is correct only while the provider process operates.
- An external credential continues to be correct after the app starts again. The person can
  revoke it through local administration.
- A private discovery file lets external stdio hosts connect again without a
  change to their configuration.

The desktop continues to own the worktrees, the processes and the state. The
provider adapters keep their current MCP configuration step. The built-in row
of the registry contains no credential. Prometeu does not edit a native
configuration of a provider. It does not add a relay or Cloud API.

These modules have these responsibilities:

- `mcp_access.rs` controls the client identity, the project scope and the
  administration of credentials.
- `delegation.rs` controls the authorization of resources and the use-case logic.
- `embedded_mcp.rs` contains the protocol and the transport that the two callers use.

The use-case logic and the clients do not use a provider protocol or a
conversation in the frontend. Internal callers and external callers go into the
same dispatcher. The optional context of a conversation gives the defaults for
the repository, the model and the permissions. An external call selects a
permitted project ID. The local CLI registration prints a standard definition of
a stdio server. It does not edit the settings of a different application. Agents
cannot manage credentials through a tool.

The code that makes a workspace stays in `session.rs`. Add an optional owner
record to its internal path. Give an ID to the worker conversation before the
preparation. Save the owner together with the new workspace. Execution IDs stay
different from conversation IDs.

Prometeu monitors the canonical events while it holds their publication lock.
At that time, it changes the board only in memory. Then it publishes the
changes after it releases the conversation locks. This decision changes the previous rule
that all board changes occurred after Prometeu released the locks. Prometeu
must record an execution in the correct sequence, before a subsequent response
that comes quickly. The lock sequence stays chats → buffer → board. While
Prometeu holds the board lock, it does no process or file operations.

Workers operate on Claude, Codex or Antigravity, and `delegate` must receive
`provider`. A model does not select the provider, and the provider of the
coordinator does not select it. Coordinators also have native subagents that
cannot change provider. Thus Prometeu does these steps:

- The descriptions of the tools and of the server tell when a Prometeu worker
  applies.
- A read-only `list_agents` tool shows the live account catalog of each
  provider.
- Prometeu validates a provider, a model or an effort that is different from
  the values of the conversation. It validates them against that catalog before
  it makes a workspace.

When Prometeu rejects a value, the error gives the nearest IDs of the selected
provider. Prometeu does not guess a different provider. If the selected model
does not list an inherited effort, Prometeu uses the CLI default and does not
reject the request. The cause is that a caller cannot send an empty `effort` to
remove it.

Prometeu rejected these alternatives:

- Use the provider of the coordinator. This sent models of other providers to
  the CLI of the coordinator. It also let Cursor coordinators start Cursor
  workers.
- Find the provider from the model. This is an implicit routing rule.
- Use a catalog from a cache. The cache can be older than the account.

This decision has two costs. Callers that did not send `provider` must send it
at this time. Also, Prometeu sends a live CLI query with a time limit in two
conditions:

- for each selection that is different from the values of the conversation
- for each `list_agents` call. This tool sends its queries to the providers in
  parallel.

A request to make a delegation and a message have request keys. Thus, if a caller
sends a usual request again, Prometeu does the operation only one time. New MCP messages reject a target
that is busy. The native recovery of messages that wait continues to operate.
If a provider or the saved data stops because of a crash, Prometeu does not
promise one delivery and only one. The completion of the primary turn and the
background activity that Prometeu sees stay two different facts.

When a person sends usual input while an execution operates, that execution
keeps its identity until Prometeu sees the next turn completion. Version 1 does
not connect accepted messages to provider turns. If Prometeu counted each
message as a new execution, some IDs did not get a terminal state. Subsequent
provider activity uses the current rule for continuations that Prometeu sees.
MCP messages continue to go only to a target that does no work. Thus they keep
their own request-key identities. This decision does not change saved fields or
conversation events.

For the same agent IDs that the client owns, let the client do these
operations:

- Start the configured setup and Run scripts.
- Read dock logs, with a limit on the quantity of text.
- Open a preview when the caller sends a request for it.

Use the current dock, PTY, script configuration and workspace navigation. Do not
add an API for shell commands or a second supervisor for processes. Setup
continues to start automatically when Prometeu makes the workspace. A caller can
start it again with a call. If a script operates at that time, Prometeu uses it
again under the shared PTY lock. If a different Run script operates, the caller
must stop it before Prometeu replaces it. The runtime status and the exit codes
are not saved. After the app starts again, Prometeu does not report a success
that it did not see. When Prometeu starts a process, this does not prove that
the service can receive requests.

When Prometeu opens a preview, it changes the workspace that the person sees.
This is intentional. A local event contains only the approved IDs of the
workspace and of the conversation. The desktop controls the navigation, the
layout and the URL that it makes from the reserved port. The MCP result tells
that Prometeu received the request. It does not tell that the page completed
its load. Prometeu does not add remote routing for previews, a free URL, JavaScript
execution or a new IPC command.

## Alternatives and consequences

Prometeu rejected a general CRUD or IPC bridge for workspaces. That bridge
shows state that is not related, and it connects agents to presentation
commands. A standalone daemon makes a second owner of the lifecycle. An HTTP
server adds a network boundary that is not necessary for the first local
feature. A pure stdio server without a bridge cannot safely control the
state of the desktop that operates at that time.

The shared executable does not add installation dependencies. The desktop must
stay open. After the app starts again, internal callers must get a new process
credential. External registrations keep their identity and find the new socket
again. For local composition, a daemon, an HTTP listener and a different
package are not necessary. The socket credential gives access to the MCP API. It is not a
sandbox against local processes that have the file system permissions of the
user.

Saved delegation records and execution IDs add responsibilities for
compatibility and storage. This decision does not add a new scheduler, an
automatic manager agent, a deletion tool, an approval proxy or execution on
many machines. [ADR 0099](0099-sub-workspaces.md) adds a navigation projection with
one level. It also reserves one conversation for each delegated workspace.
More conversations from before this change stay available. At a subsequent
time, many delegated agents can share one environment. That change can use the
same identities. It will not be necessary to make all current workspaces into
agents.

## Contract and verification

See [embedded MCP](../contracts/embedded-mcp.md) and the
[provider matrix](../quality/provider-matrix.md). The tests examine these items:

- the ownership of the conversations that a client made
- a new start of the app and the migration
- the MCP message frames and lifecycle
- the tool selection and the configuration paths of the two providers
- many external clients and the project scope
- credential revocation
- how an external stdio subprocess finds the socket again

The integration test uses a socket fixture. A live execution of a model stays a
different manual check. The implementation does not add an IPC command. The
current hub and board responses get more data.
