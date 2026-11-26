---
type: Contract
title: "Workspace supervision"
description: "Remote workspace bindings, durable metadata events and generic signed webhook delivery through Prometeu Cloud."
status: stable
---

# Workspace supervision

Decision: [ADR 0105](../decisions/0105-workspace-supervision.md).
The first executor is `prometeu-host`. Desktop-created local workspaces and a
reverse network transport to the local MCP are deferred. The external
supervisor uses the existing remote MCP for reads and instructions; the Cloud
does not execute agents.

## Destination and host setup

The Cloud account's Supervisor settings configure one HTTPS webhook destination,
its name, enabled state and event filters. Creating or rotating credentials
shows an ingestion token and a webhook signing secret once. The ingestion
token authenticates host publication; the signing secret lets the receiver
verify callbacks. They have distinct purposes and never enter MCP results.

On the remote machine, `prometeu-host supervision setup --url <cloud-origin>`
reads the ingestion token from standard input and validates it with the Cloud.
`supervision status` reports the public identity, publication configuration and
`pending_events` count;
`supervision disable` pauses publication while preserving pending events.
The host stores its stable executor identity and private configuration under
its own root, independently of desktop credentials.

The remote MCP exposes `list_supervisors`, returning entries with `id`, `name`
and `executor_id` so the receiver can correlate callbacks with its MCP connection.
`create_workspace` accepts optional
`supervisor_id`; absence preserves ordinary creation. An unknown or unconfigured
supervisor is rejected before work begins. A supervised workspace records its
binding before launching the first conversation.

`set_workspace_supervisor { id, supervisor_id }` assigns supervision to an
existing remote workspace. `remove_workspace_supervisor { id }` removes that
binding and emits `workspace.unsupervised`; it does not close the workspace.
`link_workspace_pr { id, repository, number }` associates the workspace with a
GitHub PR. These methods use the existing authenticated remote MCP authority,
not the callback URL as an authorization credential.

## Host publication

`GET /api/supervision` accepts the ingestion token as Bearer and returns:

```json
{"supervisor":{"id":"20000000-0000-4000-8000-000000000001","name":"Example supervisor"}}
```

`POST /api/supervision/events` accepts JSON and the same Bearer. A host event is:

```json
{
  "version": 1,
  "id": "50000000-0000-4000-8000-000000000002",
  "supervisor_id": "20000000-0000-4000-8000-000000000001",
  "executor_id": "30000000-0000-4000-8000-000000000001",
  "workspace_id": "40000000-0000-4000-8000-000000000001",
  "kind": "execution.completed",
  "occurred_at": 1795608000,
  "conversation_id": "60000000-0000-4000-8000-000000000001",
  "execution_id": "70000000-0000-4000-8000-000000000001",
  "repository": "example/app",
  "branch": "prometeu/fixture-work"
}
```

IDs match `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`. Requests are limited to 16 KiB.
`occurred_at` is Unix seconds. Optional
metadata consists of conversation/execution IDs, `owner/repository`, branch,
`pull_request` number from 1 through 2,147,483,647 and `head_sha` (40–64 lowercase
hexadecimal characters). Branch names are limited to 255 UTF-8 bytes and contain
no whitespace or control characters. No field carries user text or
provider output. The server rejects unknown fields, unsupported versions,
invalid metadata, a mismatched supervisor and reused event IDs with different
content. Repeating an accepted identical event is safe.

Host kinds:

| Kind | Meaning |
| --- | --- |
| `workspace.created` | Supervision begins, including explicit assignment to an existing workspace. |
| `workspace.pr_linked` | Repository and PR association changes. |
| `workspace.closed` | The workspace closes and GitHub watching ends. |
| `workspace.unsupervised` | The supervisor is removed and GitHub watching ends. |
| `execution.completed` | The observed root turn and its known children have finished. |
| `execution.failed` | Execution fails or an unfinished execution is recovered after host restart. |
| `execution.blocked` | The execution needs intervention, once per observed blocking episode. |

Session start, idle snapshots, tokens, tool output and repeated stop signals do
not independently imply completion. Execution observations and pending events
are persisted together. Known children hold completion until they finish.
An interrupted execution never announces success. A completed turn is not proof
that a PR meets merge criteria: the supervisor checks current GitHub state.

The publisher uses a persistent outbox, acknowledges only successful ingestion
and retries with backoff after network failure. New events wake it immediately;
there is no periodic state query. A host restart recovers pending deliveries.
The host cannot keep executing through its own process or machine failure.

Before changing to a different Cloud or supervisor, detach workspaces and let
their pending events reach the previous destination. If that credential was
revoked or the destination was deleted, `supervision reset --discard-pending`
is the explicit administrative recovery: it discards unsent metadata, removes
local supervisor bindings and disables publication while preserving executor
identity, workspaces, worktrees and conversations. It cannot retract events
already accepted by the previous Cloud; remove the old destination there too.

## GitHub

The existing authenticated GitHub App webhook endpoint also feeds supervision.
The App needs subscriptions for `pull_request`, `pull_request_review`,
`pull_request_review_comment`, `pull_request_review_thread`, `issue_comment`,
`workflow_run`, `check_suite`, `check_run` and `status`, with their corresponding
read permissions. Repository installation and subscription changes are separate
operational steps; code deployment does not enable them automatically.
Only explicitly watched workspaces participate. Repository/branch metadata can
discover a PR; explicit PR association handles known PRs. Registration alone
does not authorize private GitHub metadata: a signed GitHub event must establish
the account's linked GitHub identity as a participant before installation and
PR routing become trusted. A previously existing PR can therefore wait for a
qualifying event after registration. App installation deletion clears routing.

The supervisor receives relevant CI, review/comment and PR changes, including
successful CI, without the human notification feed's actor suppression. Bodies,
titles, logs and code are not persisted or forwarded. GitHub events are wakeups,
not authoritative readiness verdicts; the receiver reads the current head,
required checks and reviews before deciding. Repeated deliveries are deduplicated.

GitHub event kinds are `github.ci_changed`, `github.review_changed` and
`github.pr_changed`. Successful individual `check_run` events are suppressed;
completed suites/workflows and failed checks can wake the supervisor. Legacy
commit status events require a previously associated PR head. The receiver
queries current GitHub state rather than inferring success from the event kind.

Durable recovery covers events accepted by the Cloud. A GitHub delivery that
never reaches ingestion must be redelivered from GitHub's delivery history;
this feature does not periodically scan GitHub to reconstruct missed events.

## Outgoing callbacks

The destination receives a batch scoped to one supervisor, executor and workspace:

```json
{
  "version": 1,
  "id": "80000000-0000-4000-8000-000000000001",
  "supervisor_id": "20000000-0000-4000-8000-000000000001",
  "executor_id": "30000000-0000-4000-8000-000000000001",
  "workspace_id": "40000000-0000-4000-8000-000000000001",
  "events": []
}
```

Actual event batches contain events; the empty array above abbreviates their
shape. `fixtures/cloud-api.json` contains a full executable example. Metadata
events from GitHub use the same envelope. No receiver product is assumed.

The settings test sends `webhook.test`. Its envelope has `executor_id: null`
and `workspace_id: null` because it tests the destination without creating work.
The event includes `version`, `id`, `supervisor_id`, `kind` and `occurred_at`.
Receivers should verify and acknowledge this event without starting supervision.

Headers:

- `Content-Type: application/json`
- `X-Prometeu-Delivery`: the stable delivery ID.
- `X-Prometeu-Timestamp`: the current attempt's Unix timestamp as decimal text.
- `X-Prometeu-Signature`: `sha256=` followed by lowercase hexadecimal
  HMAC-SHA256 of `timestamp + "." + raw_request_body`, keyed by the signing secret.

The secret is the displayed string's UTF-8 bytes; do not base64-decode it.

Receivers verify the signature against the raw bytes with a constant-time
comparison, reject stale timestamps, require `X-Prometeu-Delivery` to match the
signed body's `id`, and deduplicate that signed ID. A retry
retains the ID and body and receives a fresh timestamp/signature. A `2xx`
response acknowledges durable receipt; redirects and other statuses do not.
Store the event before acknowledging it, then process asynchronously.

Cloud groups nearby events and permits one HTTP delivery attempt at a time.
Its worker recovers persistent pending deliveries on startup and wakes on
new work or retry deadlines. This is delivery serialization, not a lock on an
external model: receivers must serialize evaluations by executor/workspace,
retain changes arriving during an evaluation, and read current state before
sending a new instruction. GitHub and host events can arrive out of order.

Destinations require HTTPS, public network addresses, bounded timeouts and no
redirect following. DNS addresses are checked and pinned for each attempt to
prevent private-network requests through DNS changes. Callback bodies, secrets
and arbitrary error responses are not written to delivery logs.

## Verification and compatibility

Old registry records omit supervision and keep their original behavior. MCP
creation arguments are additive. Existing local MCP ownership and provider
support do not change. Future local executors can use the same IDs, events and
binding model, but need their own authenticated reverse transport and grants.

`prometeu-core::supervision` tests check root/child completion, duplicate events,
blocking episodes, interruption, restart and the shared JSON examples. Host
tests exercise private configuration, MCP assignment and persistent HTTP
delivery using synthetic hooks. The Cloud's supervision integration and
service tests cover request authentication, metadata validation, account
isolation, GitHub routing, signatures, grouping, retries and destination
validation. The shared fixture is imported with `bin/contracts` in the Cloud.
These checks do not claim a deployment or a live external supervisor run.
