---
type: Decision
title: "ADR 0106 — Workspace supervision through Cloud webhooks"
description: "Execution hosts publish durable metadata events to the Cloud. The Cloud sends signed webhooks to an external supervisor."
status: stable
---

# ADR 0106 — Workspace supervision through Cloud webhooks

Date: 2026-11-25

## Context

External clients can make remote workspaces through the MCP of the host
([ADR 0103](0103-agents-on-remote-machines.md)). Before this decision, they had
to query the host and GitHub periodically. Thus they knew when to examine a
result or to request a repair. The user wants supervision that events start
while the desktop sleeps. The user wants a generic webhook receiver, not an
integration for one bot product.

The Cloud receives GitHub App webhooks at this time. Its notification feed is
an inbox for persons:

- It does not show some actors.
- It reports CI failures, not all CI changes that are related.
- The desktop polls it.

It is not a queue for automation.

## Decision

An optional supervisor binding is part of a workspace. The creator of the
workspace has no effect on the binding. The first executor is the remote host.
The portable `prometeu-core::supervision` module owns the metadata types and the
rules for the observation of the execution. The host adapts Claude hooks and
Codex RPC events, stores the state and sends HTTP requests.

In this version, the Cloud keeps one supervisor destination for each account,
and the person can configure it. The Cloud owns the HTTPS callback URL, the
event filters, the signature secret and the ingestion credential that the
person can revoke. Hosts keep the ingestion credential privately. An MCP client
selects a configured supervisor by ID. Thus the client cannot send a callback
URL that it selects, and it cannot read credentials.

**Persistence and delivery.** The host keeps the binding before it starts
supervised work. It keeps observations together with pending events. Its
publisher starts when a change occurs or when a retry time comes. It does not
query GitHub or scan executions periodically. The Cloud acknowledges the
ingestion only after it keeps the data, and it removes duplicate event IDs. Its
durable delivery queue puts events in groups for each executor and workspace.
The queue starts a worker in the process on changes, retry times and the start
of the application. No model operates to filter or send events.

The current Rails deployment with a single process gets one delivery worker. It
does not get a new queue service. SQLite stores pending deliveries and receipts.
Network I/O runs outside database transactions. Attempts have time limits. This
decision changes the baseline without a job process in
[ADR 0015](0015-cloud-rails.md). The delivery retries make a persistent queue
necessary now. More than one Puma process or replica needs a separate decision
about the ownership of the dispatch.

**Callbacks.** Callbacks use documented JSON and HMAC signatures. All receivers
that implement the contract can use them. A correct callback acknowledges the
receipt. It does not acknowledge the completion of a model turn. The receiver
does these steps:

1. It removes duplicate deliveries.
2. It puts the decisions for each workspace in sequence.
3. It reads the current state through MCP and GitHub.
4. It decides if it requests corrections or tells the person.

The merge stays manual.

### Provisioning in the MCP connection flow

The remote MCP setup shows the event delivery in the same form. A current MCP
can add events, and its MCP token does not change. The desktop uses its account
session and its SSH access again to prepare the selected host. Users do not move
ingestion tokens between the Cloud and a VPS now. The credentials keep separate
authorities:

- The account manages destinations.
- The MCP token authorizes agent actions.
- The host ingestion token publishes metadata.

The signature secret is part of the receiver and the Cloud. It is not part of
the host or of the MCP configuration at any time.

A digest for each supervisor and executor lets the person add or repair one
machine without a revocation of other machines. The current destination token
stays valid for manual installations. An explicit rotation of the destination
credential revokes the two forms of the credential. An executor keeps a stable
ID before its first setup. Pairing uses the current SSH access. Prometeu does
not add an HTTP listener on the host, and it does not give shell access to the
Cloud.

**Failures and stale forms.** Provisioning is not atomic across the Cloud and
SSH. A failure after the Cloud issues a token can stop the publication of that
executor until the user tries again. Durable events stay in its outbox.

- The form binds its intent to the account and the destination that it shows,
  and to their configuration revision. It rejects stale replacements before it
  issues a token.
- The revision is a fingerprint of the destination settings and the encrypted
  secret material. Thus edits in the same timestamp tick also make an open form
  not valid.
- The Cloud examines the revision under the supervisor lock. Thus the token
  issue stays bound to that configuration.

A test starts on the configured host and waits for the callback receipt of the
Cloud. Thus a person cannot think that a webhook test from the browser only is
a connected machine. The test polls with a limit and explicitly. This poll is
separate from the supervision that events start. This decision does not install
MCP into the receiver. It does not change the supervision consent for each
workspace.

### Receiver compatibility

The Cloud destination can select one of two formats:

- The current Prometeu format.
- The `type`/`data` envelope of OpenBot, with a signature of the timestamp,
  the delivery ID and the body.

The person can give or replace a secret that the receiver issued in the account
form. The Cloud keeps this secret encrypted. The OpenBot credential rotation
keeps that secret and changes only the host ingestion. The format selection
stays at the outbound boundary. Thus host events and MCP clients do not depend
on the receiver. An explicit selection keeps the current destinations. A
general editor for headers or templates is not necessary for these two
protocols.

### GitHub routing and trust

The host registers repository and branch metadata. It can explicitly connect a
PR through MCP. The Cloud sends signed GitHub events to watched workspaces.
Desktop notification preferences have no effect on this. A repository name
that an account claims gives no access to the private repository metadata of a
different account. The Cloud first must have a signed GitHub event that shows
the participation of the linked user. Then it binds the repository, the PR and
the installation. When the installation is removed, that binding is not valid.

The Cloud stores only metadata. It does not store prompts, transcripts, code,
review bodies or provider credentials. Supervisor signature secrets are
integration credentials, and the Cloud stores them encrypted. The Cloud stores
ingestion credentials as digests. At each delivery, the Cloud validates the
outgoing URLs against destinations in private networks. It does not follow
redirects. This decision adds an authenticated boundary from the host to the
Cloud and an outbound webhook boundary. These boundaries are separate from the
collaboration E2EE.

### Future local execution

The executor identity is separate from the workspace identity and from the
supervisor identity. The assignment of supervision is also separate from the
creation of a workspace. MCP creation is only one entry point. With these rules,
a subsequent desktop adapter and an explicit assignment from the launcher are
possible. The meaning of the events does not change.

This version does not add a local supervisor selection. It does not add a
reverse network path to the MCP of the desktop. The local MCP ownership rules
do not change. The current machine-token authority of the remote host also does
not change. A supervisor binding sends events. It is not a new isolated MCP
client identity. Before a future desktop adapter can give the same supervision,
it must add two items:

- A grant for one workspace that the person can revoke.
- An authenticated path back to its local MCP.

## Alternatives

- A faster poll decreases the latency. But the supervision continues to depend
  on repeated queries. The requested flow uses event delivery from its first
  version.
- If each host sends directly to the callback, each host has a copy of the
  destination configuration. Also, the host cannot receive GitHub updates while
  an executor is offline.
- If all events go through the desktop, remote work depends on the Mac.
- An LLM supervisor in the Cloud adds provider credentials and the processing
  of transcripts. The external supervisor has that responsibility at this time.

## Compatibility and evidence

Workspaces without supervision keep the current behavior. Additions to the host
registry use defaults. The [current registry format](../contracts/remote-host.md#registry)
protects provider identity, supervision fields and shared archive state. Hosts from earlier
versions refuse a newer format. For a downgrade, keep that file and a compatible host. Current MCP calls
do not need new arguments. When the person removes the supervision, the events
stop, but the workspace stays open. Retries give at-least-once delivery. They
do not promise exactly-once model execution.

The [supervision contract](../contracts/supervision.md) specifies the wire
formats, the setup, the filters, the signatures and the recovery. The core
observation tests prove the held completion and the reconciliation after a new
start. The host tests use synthetic hooks and a local HTTP peer. The Cloud tests
cover authentication, account isolation, deduplication, GitHub routing and
delivery. The shared `fixtures/cloud-api.json` examples examine the boundary
between the producer and the consumer.
