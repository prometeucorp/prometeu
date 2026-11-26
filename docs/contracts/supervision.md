---
type: Contract
title: "Workspace supervision"
description: "Remote workspace bindings, durable metadata events and generic signed webhook delivery through Prometeu Cloud."
status: stable
---

# Workspace supervision

Decision: [ADR 0106](../decisions/0106-workspace-supervision.md).
The first executor is `prometeu-host`. Desktop-created local workspaces and a
reverse network transport to the local MCP are deferred. The external
supervisor uses the existing remote MCP for reads and instructions; the Cloud
does not execute agents.

## Destination and host setup

The Cloud account's Supervisor settings configure one HTTPS webhook destination,
its name, enabled state, event filters and webhook format (`prometeu` by default,
or `openbot`). Creating a Prometeu destination generates a signing secret unless
one is supplied. OpenBot requires the receiver's signing secret on creation.
The form accepts secrets of 16 to 1024 characters; editing with an empty secret
keeps the stored value. Secrets remain encrypted and are never prefilled.

Creating or rotating credentials shows the new ingestion token once. In Prometeu
mode it also shows the signing secret; in OpenBot mode rotation changes only the
host token and preserves the receiver's secret. Replace the OpenBot secret through
the destination form. The ingestion token authenticates host publication; the
signing secret lets the receiver verify callbacks. They have distinct purposes
and never enter MCP results.

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

## Provisioning with the remote MCP

Settings / Machines / Enable agents offers an optional **Receive workspace events**
choice. The account's existing destination is reused; if none exists, the person
supplies a webhook URL, format and the receiver's signing secret in the same form.
The default format is OpenBot. Existing MCP connections offer **Enable events**
without replacing their MCP token. Ordinary MCP setup requires no Cloud account.
Supervision remains an explicit workspace binding selected through the host MCP;
provisioning does not attach all existing or future workspaces automatically.

The desktop authenticates provisioning with its stored Cloud session. The form
retains the account identity and destination ID; a changed account or destination
requires reopening it. A disabled destination must be enabled in Cloud settings.
Creation does not overwrite a destination another client created in the meantime.
The desktop obtains the executor's stable public ID over the existing SSH access,
requests a credential, then pipes that credential to `supervision setup` on that
machine. Neither the account Bearer nor the receiver's signing secret reaches the
host. Ingestion credentials stay out of the webview, clipboard and MCP responses.
The one copyable result remains the existing MCP configuration.

Desktop Bearer endpoints (JSON writes, same Origin and size guards as the other
desktop endpoints):

- `GET /api/supervisor` returns `{ supervisor: null }` or public destination
  fields `id`, `name`, `url`, `enabled`, `webhook_protocol`; never secrets.
- `POST /api/supervisor/connections` accepts `executor_id`, optional
  `supervisor_id` for an existing destination, or `destination` with `name`, `url`,
  `webhook_protocol` and `signing_secret` for a new one. It returns
  `{ supervisor: { id, name }, ingest_token }` with `Cache-Control: no-store`.
  A stale destination returns 409; invalid input or a disabled destination returns
  422. Browser cookies and ingestion credentials do not authorize provisioning.

Cloud stores one ingestion digest per supervisor/executor in
`supervision_credentials`. Reprovisioning replaces only that executor's token;
it preserves the receiver secret, other executors and the legacy ingestion token.
Each new token authorizes ingestion and test receipts only for its executor.
Legacy destination tokens retain their existing authority. Disabling a destination
rejects all its tokens; deleting it removes its credentials. Explicit credential
rotation in Cloud settings revokes all executor credentials too.

Provisioning spans Cloud and SSH, without a distributed transaction. If the SSH
step fails after credential issuance, repeat **Enable events** to repair that
executor. Its outbox and workspaces remain intact. Initial setup configures events
before creating the MCP credential, so a failed event setup cannot hide a newly
rotated MCP token. MCP token rotation remains a separate explicit action.

`supervision identity --root DIR` returns `{ executor_id }` without credentials or
enabling publication. The daemon persists this ID in the private
`supervision-executor` file; existing `supervision.json` identities take precedence.
Setup, disabling and restarting preserve it. These additive files and commands do
not change the registry version or existing MCP arguments.

### Test from the machine

After provisioning, the desktop runs `supervision test --root DIR` on the host.
The host uses its own saved credential to POST `{ id, executor_id }` to
`/api/supervision/tests`. Cloud persists a separate `webhook.test` delivery with
that executor and a null workspace; no workspace or execution is created. Test
IDs are idempotent within retained delivery history. Tests never batch across
executors or with browser tests. `GET /api/supervision/tests/:id` returns
`{ id, status, last_error }`, scoped to the authenticated destination and, for new
credentials, executor. The receiver uses the existing signature and envelope.

The host polls this receipt for a bounded diagnostic window, stopping on delivery
or a reported failure, and returns only `{ status, failed }`. The UI says connected
only after `delivered`; pending, rejected and unreachable paths never become success.
The copyable MCP configuration stays available if the diagnostic fails. **Test
events from this machine** retries the diagnostic independently. This proves
receipt by the receiver, not MCP installation there or an agent's completion.
The ordinary publisher and Cloud dispatcher still do not poll host or agent state.

Desktop IPC additions:

- `machine_supervision_options()` returns `{ account, supervisor }` (both nullable).
- `machine_agents_enable` accepts optional `supervision: { account,
  supervisor_id?, destination? }`; omission preserves existing behavior and output.
- `machine_supervision_enable({ id, setup })` configures events for an existing MCP.
- `machine_supervision_test({ id })` returns `{ status, failed }`.

`src/mock.ts` implements the same calls with synthetic state. Native provisioning
and receipt parsing are covered in `remote/supervision.rs` and host supervision
tests; `machine-supervision.test.ts` covers consent, existing MCP preservation and
receipt presentation. Cloud's `supervision_connections_test.rb` covers account and
executor isolation, credential compatibility and a signed callback acknowledgement.
Deploy the additive Cloud migration and endpoints before the desktop/host update.
A Cloud rollback must retain the schema; old code cannot authenticate newly issued
executor tokens, so keep compatible Cloud code while those hosts publish.

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

The host omits optional branch metadata that exceeds these limits, including
Unicode whitespace, while preserving the original workspace branch locally.

An unseen host event must have occurred within the last 90 days and no more than
five minutes in the future. An older unseen event returns HTTP 410 with
`expired_event`; a future event outside that clock tolerance is invalid. Known
receipts still return success for identical events or a conflict for changed
content before checking age. Receipts are retained for 91 days after acceptance,
so forgetting an old receipt cannot reactivate a workspace through replay.

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

After more than 90 days without accepted publication, an unseen queued event can
expire at the Cloud boundary. The host retains it and reports HTTP 410 in its
log; `supervision status` still includes it in `pending_events`. An operator can
inspect and preserve the pending metadata, then use the explicit reset below and
reattach supervision. Expired metadata is never silently acknowledged or removed
from the host's outbox.

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

Headers for the default `prometeu` format:

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

### OpenBot format

Selecting `openbot` changes only the outbound callback boundary. Host ingestion,
MCP supervisor identities, metadata filtering and batching remain unchanged.
The body is `{"type":"prometeu.events","data":<delivery envelope above>}`.
Individual event kinds remain in `data.events`, including `webhook.test` for the
settings test. The receiver routes `prometeu.events` and inspects those events.

Headers:

- `Content-Type: application/json`
- `X-OpenBot-Timestamp`: the attempt's Unix timestamp as decimal text.
- `X-OpenBot-Delivery-Id`: the stable delivery ID, equal to `data.id`.
- `X-OpenBot-Signature`: `sha256=` followed by lowercase hexadecimal HMAC-SHA256
  of `timestamp + "." + delivery_id + "." + raw_request_body`.

Use the supplied secret's literal UTF-8 bytes, including any `whsec_` prefix;
do not base64-decode it. The signature covers the exact wrapped JSON body sent.
Retries retain the delivery ID and body and receive a fresh timestamp/signature.
The same acknowledgement, deduplication and timestamp checks apply.

Editing the format, URL or secret affects subsequent attempts, including queued
retries; a request already sent cannot be changed. A format change re-encodes the
same stored metadata envelope using the selected format. Existing destinations
keep `prometeu` after the additive migration. Before rolling Cloud back to a
version without OpenBot support, disable OpenBot destinations; older code only
sends the Prometeu format. Preserve the new column, encrypted secrets and queue.

Cloud browser tests cover create/edit, blank preservation, validation, secret
filtering and host-token rotation. Delivery tests verify the OpenBot signature,
wrapped metadata, retries and conversion of a queued Prometeu test delivery.
The existing producer/consumer fixtures still cover the unchanged Prometeu
format; desktop and host clients require no update for this receiver option.

Cloud groups nearby events and permits one HTTP delivery attempt at a time.
Its worker recovers persistent pending deliveries on startup and wakes on
new work or retry deadlines. This is delivery serialization, not a lock on an
external model: receivers must serialize evaluations by executor/workspace,
retain changes arriving during an evaluation, and read current state before
sending a new instruction. GitHub and host events can arrive out of order.

Delivered and cancelled callback history is retained for 30 days; pending and
sending deliveries are never removed by retention. The worker performs local
database maintenance at startup and daily without querying hosts or GitHub.

Destinations require HTTPS, public network addresses, bounded timeouts and no
redirect following. DNS addresses are checked and pinned for each attempt to
prevent private-network requests through DNS changes. Callback bodies, secrets
and arbitrary error responses are not written to delivery logs.

## Verification and compatibility

Old registry records omit supervision and keep their original behavior. MCP
creation arguments are additive. Existing local MCP ownership and provider
support do not change. Future local executors can use the same IDs, events and
binding model, but need their own authenticated reverse transport and grants.

Host registry writes use version 2 and migrate version 1 records on write.
Older hosts refuse that version instead of rewriting away bindings and pending
events; a downgrade must retain a compatible host and preserve the registry.

`prometeu-core::supervision` tests check root/child completion, duplicate events,
blocking episodes, interruption, restart and the shared JSON examples. Host
tests exercise private configuration, MCP assignment and persistent HTTP
delivery using synthetic hooks. The Cloud's supervision integration and
service tests cover request authentication, metadata validation, account
isolation, GitHub routing, signatures, grouping, retries and destination
validation. The shared fixture is imported with `bin/contracts` in the Cloud.
These checks do not claim a deployment or a live external supervisor run.
