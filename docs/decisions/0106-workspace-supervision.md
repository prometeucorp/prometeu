---
type: Decision
title: "ADR 0106 — Workspace supervision through Cloud webhooks"
description: "Execution hosts publish durable metadata events to the Cloud, which routes signed webhooks to an external supervisor."
status: stable
---

# ADR 0106 — Workspace supervision through Cloud webhooks

Date: 2026-11-25

## Context

External clients can create remote workspaces through the host's MCP
([ADR 0103](0103-agents-on-remote-machines.md)). They previously had to query the
host and GitHub periodically to know when to inspect a result or request a fix.
The user wants event-driven supervision while the desktop is asleep, with a
generic webhook receiver rather than an integration tied to one bot product.

The Cloud already receives GitHub App webhooks. Its notification feed is a
human inbox: it suppresses some actors, reports CI failures rather than all
relevant CI changes, and the desktop polls it. It is not an automation queue.

## Decision

An optional supervisor binding belongs to a workspace, independently of its
creator. The first executor is the remote host. The portable
`prometeu-core::supervision` module owns metadata types and execution observation
rules; the host adapts Claude hooks, stores state and sends HTTP requests.

The Cloud keeps one configurable supervisor destination per account in this
version. It owns the HTTPS callback URL, event filters, signing secret and
revocable ingestion credential. Hosts keep the ingestion credential privately.
Selecting a configured supervisor by ID never lets an MCP client submit an
arbitrary callback URL or read credentials.

The host persists the binding before starting supervised work, and persists
observations together with pending events. Its publisher wakes on changes or
retry deadlines; it does not query GitHub or periodically scan executions. The
Cloud acknowledges ingestion only after persistence and deduplicates event IDs.
Its durable delivery queue groups events per executor/workspace and wakes an
in-process worker on changes, retry deadlines and application startup. No
model runs to filter or deliver events.

The existing single-process Rails deployment gains one delivery worker, not a
new queue service. SQLite stores pending deliveries and receipts. Network I/O
runs outside database transactions; attempts use bounded timeouts. This extends
the no-job-process baseline in [ADR 0015](0015-cloud-rails.md): a persistent queue
is now justified by delivery retries. Multiple Puma processes or replicas need
a separate dispatch ownership decision.

Callbacks use documented JSON and HMAC signatures. Any receiver implementing
the contract can consume them. A successful callback acknowledges receipt, not
completion of a model turn. The receiver deduplicates deliveries, serializes
decisions per workspace, reads current state through MCP/GitHub, and decides
whether to request corrections or notify the person. Merge remains manual.

### GitHub routing and trust

The host registers repository/branch metadata and can explicitly associate a PR
through MCP. Signed GitHub events route to watched workspaces through the Cloud,
independently of desktop notification preferences. A claimed repository name
alone grants no access to another account's private repository metadata: the
Cloud first requires a signed GitHub event establishing the linked user's
participation before binding repository, PR and installation. Installation
removal invalidates that binding.

Cloud storage remains metadata-only: no prompts, transcripts, code, review
bodies or provider credentials. Supervisor signing secrets are integration
credentials, encrypted at rest; ingestion credentials are stored as digests.
Outgoing URLs are validated against private-network destinations on each
delivery. Redirects are not followed. This adds an authenticated host-to-Cloud
boundary and an outbound webhook boundary, separate from collaboration E2EE.

### Future local execution

Executor identity is separate from workspace and supervisor identity. Assigning
supervision is also separate from workspace creation: MCP creation is only one
entry point. These rules admit a later desktop adapter and explicit assignment
from the launcher without changing event semantics.

This version adds neither local supervisor selection nor a reverse network
path to the desktop's MCP. Local MCP ownership rules are unchanged. The remote
host's existing machine-token authority is also unchanged: a supervisor binding
routes events; it is not a new isolated MCP client identity. A future desktop
adapter must add a revocable workspace-scoped grant and an authenticated path
back to its local MCP before it can offer equivalent supervision.

## Alternatives

- Faster polling would reduce latency but still couple supervision to repeated
  queries. The requested flow uses event delivery from its first version.
- Sending directly from each host to the callback duplicates destination
  configuration and cannot receive GitHub updates while an executor is offline.
- Routing everything through the desktop makes remote work depend on the Mac.
- Running an LLM supervisor in the Cloud would add provider credentials and
  transcript handling. The external supervisor already owns that responsibility.

## Compatibility and evidence

Unsupervised workspaces retain existing behavior. Host registry additions use
defaults and version 1 records migrate on write to version 2, which older hosts
refuse to rewrite; downgrade must preserve that file and a compatible host.
Existing MCP calls need no new arguments. Removing supervision stops
routing without closing the workspace. Retries provide at-least-once delivery;
they do not promise exactly-once model execution.

The [supervision contract](../contracts/supervision.md) defines wire formats,
setup, filtering, signatures and recovery. Core observation tests prove held
completion and restart reconciliation. Host tests use synthetic hooks and a
local HTTP peer; Cloud tests cover authentication, account isolation,
deduplication, GitHub routing and delivery. Shared
`fixtures/cloud-api.json` examples check the producer/consumer boundary.
