---
type: Contract
title: "Workspace supervision"
description: "The links between remote workspaces and supervisors, the durable metadata events and the delivery of signed webhooks through Prometeu Cloud."
status: stable
---

# Workspace supervision

Decision: [ADR 0106](../decisions/0106-workspace-supervision.md). The first
executor is `prometeu-host`. Two items are not in this version: local workspaces
that the desktop makes, and a network transport from the local MCP back to the
Cloud. The external supervisor uses the current remote MCP to read data and to
send instructions. The Cloud does not operate agents.

## Destination and host setup

The Supervisor settings of the Cloud account configure one HTTPS webhook
destination. The settings contain these items:

- The name of the destination.
- The enabled state.
- The event filters.
- The webhook format: `prometeu` (the default) or `openbot`.

When a person makes a Prometeu destination, the Cloud makes a signing secret if
the person does not supply one. For OpenBot, the person must supply the signing
secret of the receiver when they make the destination. The form accepts secrets
of 16 to 1024 characters. If a person saves the form with an empty secret, the
stored value does not change. The Cloud keeps secrets encrypted and does not
show them again in the form.

When a person makes or replaces credentials, the Cloud shows the new ingestion
token one time. In Prometeu mode, it also shows the signing secret. In OpenBot
mode, rotation changes only the host token and keeps the secret of the receiver.
To replace the OpenBot secret, use the destination form. The ingestion token
authenticates the publication from the host. The signing secret lets the
receiver examine the callbacks. The two secrets have different purposes, and
they do not go into MCP results.

On the remote machine, `prometeu-host supervision setup --url <cloud-origin>`
reads the ingestion token from standard input and validates it with the Cloud.
`supervision status` shows the public identity, the publication configuration
and the `pending_events` count. `supervision disable` stops publication for a
time and keeps the events that wait for delivery. The host stores its stable
executor identity and its private configuration in its own root. The host does
not use the desktop credentials for these items.

The remote MCP gives `list_supervisors`. It returns entries with `id`, `name`
and `executor_id`, so that the receiver can find the MCP connection for each
callback. `create_workspace` accepts an optional `supervisor_id`. Without this
field, the host makes the workspace as usual. The host refuses an unknown
supervisor or a supervisor without configuration before the work starts. A
supervised workspace records its link to the supervisor before it starts the
first conversation.

`set_workspace_supervisor { id, supervisor_id }` adds supervision to a current
remote workspace. `remove_workspace_supervisor { id }` removes that link and
sends `workspace.unsupervised`. It does not close the workspace.
`link_workspace_pr { id, repository, number }` links the workspace to a
GitHub PR. These methods use the authority of the current authenticated remote
MCP. They do not use the callback URL as an authorization credential.

## Provisioning with the remote MCP

Settings / Machines / Enable agents shows an optional **Receive workspace
events** option. The app uses the current destination of the account again. If
there is no destination, the person supplies a webhook URL, a format and the
signing secret of the receiver in the same form. The default format is OpenBot.
For a current MCP connection, the app shows **Enable events**. This option does
not replace the MCP token of the connection. A Cloud account is not necessary
for the usual MCP setup.

Supervision stays an explicit link of a workspace that a person selects
through the host MCP. Provisioning does not attach all current or future
workspaces automatically.

The desktop authenticates provisioning with its stored Cloud session. The form
keeps the identity of the account, the ID of the destination and the revision of
the configuration. If the account or the destination changes, the person must
open the form again. A person must enable a disabled destination in the Cloud
settings. When the app makes a destination, it does not replace a destination
that a different client made in the same period.

The desktop gets the stable public ID of the executor through the current SSH
access. Then it requests a credential and sends that credential through a pipe
to `supervision setup` on that machine. The Bearer token of the account and the
signing secret of the receiver do not go to the host. Ingestion credentials do
not go into the webview, the clipboard or MCP responses. The only result that a
person can copy is the current MCP configuration.

Desktop Bearer endpoints use JSON writes. They have the same Origin limit and
the same maximum body dimension as the other desktop endpoints:

- `GET /api/supervisor` returns `{ supervisor: null }` or these public fields of
  the destination: `id`, `revision`, `name`, `url`, `enabled`,
  `webhook_protocol`. It does not return secrets.
- `POST /api/supervisor/connections` accepts `executor_id` and one of these
  two items:
  - For a current destination: an optional `supervisor_id` and
    `supervisor_revision`.
  - For a new destination: `destination` with `name`, `url`,
    `webhook_protocol` and `signing_secret`.

  It returns `{ supervisor: { id, name }, ingest_token }` with
  `Cache-Control: no-store`. A revision of the destination that is not current
  gives 409. Incorrect input or a disabled destination gives 422. Browser
  cookies and ingestion credentials do not authorize provisioning.

The `revision` of a destination is a SHA-256 fingerprint. Clients only compare
this value and send it back. The fingerprint uses these items of the
destination:

- The ID, the name and the URL.
- The enabled state, the filters and the format.
- The encrypted signing material.
- The legacy ingestion digest.

The fingerprint does not show secrets as plain text. The resolution of
timestamps does not change it.

The server compares the revision in the request under the supervisor lock before
it changes credentials. For a current destination, a missing revision or a
revision that is not current returns 409 `changed`, and all executor digests
stay the same. When the app does provisioning for a different executor, the
revision does not change. The companion desktop must send back the revision that
it read when the form opened. It must not get a new revision silently when the
person sends the form, because the revision records the consent of the person.
The legacy manual setup and the ingestion endpoints of the host do not change.

The Cloud stores one ingestion digest for each pair of supervisor and executor
in `supervision_credentials`. A new provisioning replaces only the token of that
executor. It keeps the secret of the receiver, the other executors and the
legacy ingestion token. Each new token authorizes ingestion and test receipts
only for its executor. Legacy destination tokens keep their current authority.
When a person disables a destination, the Cloud refuses all its tokens. When a
person deletes a destination, the Cloud deletes its credentials. An explicit
rotation of credentials in the Cloud settings also revokes all executor
credentials.

Provisioning uses the Cloud and SSH without one transaction for the two
systems. If the
SSH step has a failure after the Cloud issues the credential, click
**Enable events** again to repair that executor. Its outbox and workspaces stay
complete. The first setup configures events before it makes the MCP credential.
Thus, a failure of the event setup cannot hide a new MCP token after a rotation.
The rotation of the MCP token stays a different and explicit action.

`supervision identity --root DIR` returns `{ executor_id }`. It does not return
credentials, and it does not enable publication. The daemon keeps this ID in the
private file `supervision-executor`. If a current `supervision.json` has an
identity, the host uses that identity first. Setup, disable and a new start of
the host keep the ID. These added files and commands do not change the version
of the registry or the current MCP arguments.

### Test from the machine

After provisioning, the desktop runs `supervision test --root DIR` on the host.
The host uses its saved credential to send a POST request with
`{ id, executor_id }` to `/api/supervision/tests`. The Cloud keeps a different
`webhook.test` delivery with that executor and a null workspace. The Cloud
does not make a workspace or an execution.

Test IDs are idempotent in the delivery history that the Cloud keeps. The Cloud
does not put tests from different executors or browser tests in one batch.
`GET /api/supervision/tests/:id` returns `{ id, status, last_error }`. The
scope is the authenticated destination and, for new credentials, the executor.
The receiver gets the current signature and envelope.

The host polls this receipt for a diagnostic period with a maximum time. It
stops after the delivery or after a failure report, and it returns only
`{ status, failed }`. The UI shows a connection only after `delivered`. A
delivery that waits, a refused delivery and a receiver that the Cloud cannot
connect to do not show success. If the diagnostic has a failure, the person can
continue to copy the MCP configuration. **Test events from this machine** runs
the diagnostic again independently.

This test proves that the receiver got the event. It does not prove that the
receiver has the MCP installed or that an agent completed its work. The usual
publisher and the Cloud dispatcher do not poll the state of the host or the
agent.

New desktop IPC calls:

- `machine_supervision_options()` returns `{ account, supervisor }`. The two
  values can be null.
- `machine_agents_enable` accepts an optional `supervision: { account,
  supervisor_id?, supervisor_revision?, destination? }`. Without this field, the
  behavior and the output do not change.
- `machine_supervision_enable({ id, setup })` configures events for a current
  MCP.
- `machine_supervision_test({ id })` returns `{ status, failed }`.

`src/mock.ts` gives the same calls with synthetic state. The tests in
`remote/supervision.rs` and the host supervision tests examine the native
provisioning and how the host parses receipts. `machine-supervision.test.ts`
examines consent, the protection of the current MCP and the display of the
receipt. The Cloud test `supervision_connections_test.rb` examines these items:

- The accounts and executors stay isolated from each other.
- The compatibility of credentials.
- The acknowledgement of a signed callback.

Deploy the added Cloud migration and endpoints before the desktop and host
update. A rollback of the Cloud must keep the schema. Earlier Cloud code cannot
authenticate the new executor tokens. Thus, keep compatible Cloud code while
those hosts publish.

## Host publication

`GET /api/supervision` accepts the ingestion token as Bearer and returns:

```json
{"supervisor":{"id":"20000000-0000-4000-8000-000000000001","name":"Example supervisor"}}
```

`POST /api/supervision/events` accepts JSON and the same Bearer. A host event
is:

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

IDs agree with `[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}`. The maximum dimension of a
request is 16 KiB. `occurred_at` is in Unix seconds. The optional metadata is
these items:

- The conversation ID and the execution ID.
- The repository as `owner/repository`.
- The branch.
- The `pull_request` number, from 1 to 2,147,483,647.
- The `head_sha`, with 40 to 64 lowercase hexadecimal characters.

A branch name has a maximum of 255 UTF-8 bytes. It does not contain whitespace
or control characters. No field contains text from a person or output from a
provider. The server refuses these items:

- Unknown fields.
- Versions that it does not know.
- Incorrect metadata.
- A supervisor that is not the supervisor of the token.
- An event ID that the server got before with different content.

The host can send an accepted event again with the same content without a
problem. If the optional branch metadata is more than these limits, the host
does not send it. This includes Unicode whitespace. The host keeps the initial
branch name of the workspace locally.

A host event that the Cloud did not get before must have occurred in the last 90
days. It must not be more than five minutes in the future. For an older new
event, the Cloud returns HTTP 410 with `expired_event`. The Cloud refuses a
future event that is more than that clock tolerance. The Cloud examines known
receipts before the age. A known receipt continues to return success for the
same event, or a 409 response for changed content. The Cloud keeps receipts for
91 days after it accepts them. Thus, an expired event that the Cloud gets again
cannot make a workspace active again after the Cloud deletes the receipt.

Event types from the host:

| Type | Meaning |
| --- | --- |
| `workspace.created` | Supervision starts. This includes an explicit link to a current workspace. |
| `workspace.pr_linked` | The relation between the repository and the PR changes. |
| `workspace.closed` | The workspace closes and the Cloud stops to monitor GitHub for it. |
| `workspace.unsupervised` | A person removes the supervisor and the Cloud stops to monitor GitHub for the workspace. |
| `execution.completed` | The root turn that the host saw and its known child turns are complete. |
| `execution.failed` | The execution has a failure. Also, after a new start of the host, the host finds an execution that was not complete. |
| `execution.blocked` | A person must help the execution. The host sends this event one time for each period of blocked work that it sees. |

These items alone do not show completion:

- The session start.
- The snapshots when the agent does not operate.
- Tokens.
- Tool output.
- Stop signals that occur again. The host
keeps the execution observations and the events that wait for delivery together.
Known child turns stop the completion until they are complete. If an execution
stops before it is complete, it does not show success. A completed turn does not
prove that a PR obeys the merge criteria. The supervisor examines the current
GitHub state.

The publisher uses a durable outbox. It acknowledges only a correct
ingestion, and after a network failure it tries again with a backoff. A new
event starts the publisher immediately. There is no periodic query of the state.
After a new start of the host, the host loads the deliveries that wait and
sends them. The
host cannot continue to operate if its own process or machine has a failure.

After more than 90 days without an accepted publication, an event in the queue
that the Cloud did not get can expire at the Cloud. The host keeps it and shows
HTTP 410 in its log. `supervision status` continues to include it in
`pending_events`. An operator can examine and keep the metadata in the queue.
Then the operator can use the explicit reset command below and attach
supervision again. The host does not silently acknowledge expired metadata or
remove it from its outbox.

Before you change to a different Cloud or supervisor, remove the workspaces from
supervision. Then let their events in the queue go to the previous destination.
If the Cloud revoked that credential or a person deleted the destination,
`supervision reset --discard-pending` is the explicit administrative recovery.
It deletes the metadata that the host did not send and removes the local links
to the supervisor. It also disables publication. It keeps the executor identity,
the workspaces, the worktrees and the conversations. It cannot cancel events
that the previous Cloud accepted. Also remove the previous destination in that
Cloud.

## GitHub

The current authenticated GitHub App webhook endpoint also sends data to
supervision. The App must have subscriptions for `pull_request`,
`pull_request_review`, `pull_request_review_comment`,
`pull_request_review_thread`, `issue_comment`, `workflow_run`, `check_suite`,
`check_run` and `status`, with the related read permissions. The installation on
a repository and the changes to subscriptions are different steps for the
operator. A code deployment does not enable them automatically.

Supervision includes only the workspaces that a person explicitly monitors. The
repository and branch metadata can find a PR. An explicit link to a PR gives the
known PRs. A registration alone does not authorize private GitHub metadata.
First, a signed GitHub event must show that the linked GitHub identity of the
account is a participant in the PR. After that, the Cloud trusts the
installation and the PR routing. Thus, a PR that was there before can wait for a
related event after the registration. When a person deletes the App
installation, the Cloud deletes the routing.

The supervisor gets the related changes to CI, reviews, comments and PRs. This
includes CI that has no failures. The supervisor gets these changes also when
the actor is the person. The notification list of the person does not show these
changes. The Cloud does not keep or send bodies, titles, logs or code. GitHub
events only tell the receiver to look again. They do not give the last decision
about readiness. The receiver reads the current head, the necessary checks and
the reviews before it makes a decision. The Cloud removes duplicate deliveries.

The GitHub event types are `github.ci_changed`, `github.review_changed` and
`github.pr_changed`. The Cloud does not send an individual `check_run` event
without a failure. Completed suites, completed workflows and checks with a
failure can start the supervisor. For a legacy commit status event, the Cloud
must have a PR head that a link recorded before. The receiver queries the
current GitHub state. It does not use the event type as proof that the work has
no failures.

The durable recovery applies only to the events that the Cloud accepted. If a
GitHub delivery does not go to ingestion, a person must send it again from the
delivery history of GitHub. This feature does not examine GitHub at intervals to
find events that it did not get.

## Outgoing callbacks

The destination gets a batch for one supervisor, one executor and one workspace:

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

Actual event batches contain events. The empty array above shows only the shape.
`fixtures/cloud-api.json` contains a full example that a test can run. The
metadata events from GitHub use the same envelope. The contract does not specify
a receiver product.

The settings test sends `webhook.test`. Its envelope has `executor_id: null`
and `workspace_id: null`, because it tests the destination and does not make
work. The event includes `version`, `id`, `supervisor_id`, `kind` and
`occurred_at`. We recommend that receivers examine and acknowledge this event
without the start of supervision.

Headers for the default `prometeu` format:

- `Content-Type: application/json`
- `X-Prometeu-Delivery`: the stable delivery ID.
- `X-Prometeu-Timestamp`: the Unix timestamp of the current request as decimal
  text.
- `X-Prometeu-Signature`: `sha256=` and then the lowercase hexadecimal
  HMAC-SHA256 of `timestamp + "." + raw_request_body`. The key is the signing
  secret.

The secret is the UTF-8 bytes of the string that the Cloud shows. Do not decode
it from base64.

Receivers must do these steps:

1. Examine the signature against the raw bytes. Compare in constant time.
2. Refuse expired timestamps.
3. Make sure that `X-Prometeu-Delivery` is the same as the `id` of the signed
   body.
4. Remove duplicates of that signed ID.

A retry keeps the ID and the body. It gets a new timestamp and a new signature.
A `2xx` response acknowledges a durable receipt. Redirects and other statuses
do not acknowledge it. Store the event before you acknowledge it. Then process
it asynchronously.

### OpenBot format

When a person selects `openbot`, only the outbound callback changes. The host
ingestion, the MCP supervisor identities, the metadata filters and the batches
do not change. The body is
`{"type":"prometeu.events","data":<delivery envelope above>}`. The individual
event types stay in `data.events`. This includes `webhook.test` for the
settings test. The receiver sends `prometeu.events` to its handler and examines
those events.

Headers:

- `Content-Type: application/json`
- `X-OpenBot-Timestamp`: the Unix timestamp of the request as decimal text.
- `X-OpenBot-Delivery-Id`: the stable delivery ID. It is the same as `data.id`.
- `X-OpenBot-Signature`: `sha256=` and then the lowercase hexadecimal
  HMAC-SHA256 of `timestamp + "." + delivery_id + "." + raw_request_body`.

Use the literal UTF-8 bytes of the supplied secret. This includes a `whsec_`
prefix if there is one. Do not decode the secret from base64. The signature
applies to the accurate JSON body with the wrapper that the Cloud sends. A retry
keeps the delivery ID and the body. It gets a new timestamp and a new signature.
The same rules for acknowledgement, duplicates and timestamps apply.

A change to the format, the URL or the secret applies to the subsequent
requests. This includes retries in the queue. A request that the Cloud sent
cannot change. A format change encodes the same stored metadata envelope again
in the selected format. After the added migration, current destinations keep
`prometeu`. Before a rollback of the Cloud to a version without OpenBot, disable
OpenBot destinations. Older code sends only the Prometeu format. Keep the new
column, the encrypted secrets and the queue.

The Cloud browser tests cover these items:

- Make and edit operations on the destination.
- An empty secret that keeps the stored secret.
- Validation.
- The removal of secrets from the output.
- The rotation of the host token.

The delivery tests examine the OpenBot signature, the metadata in the wrapper,
the retries and the conversion of a Prometeu test delivery in the queue. The
current producer and consumer fixtures continue to cover the Prometeu format,
which did not change. An update of the desktop and host clients is not necessary
for this receiver option.

The Cloud puts events that occur near the same time in one group. It sends only
one HTTP delivery request at a time. At startup, its worker loads the durable
deliveries that wait. The worker starts again when there is new work or when a
retry is due. This is a serialization of the deliveries. It is not a lock on an
external model.

Receivers must do these steps:

1. Examine the events one batch after the other for each executor and
   workspace.
2. Keep the changes that come while you examine a batch.
3. Read the current state before you send a new instruction.

GitHub events and host events can come in a different sequence.

The Cloud keeps the history of sent and cancelled callbacks for 30 days.
The retention does not remove deliveries that wait or that the Cloud sends at
that time. At startup and one time each day, the worker does maintenance of the
local database. It does not query hosts or GitHub.

Destinations must use HTTPS and public network addresses. The Cloud uses
timeouts with a maximum time and does not follow redirects. For each request,
the Cloud examines the DNS addresses and keeps them for that attempt. This
prevents requests to a private network through DNS changes. The Cloud does not
write callback bodies, secrets or error responses from the receiver to the
delivery logs.

## Verification and compatibility

Registry records from earlier versions do not have supervision fields. They keep
their initial behavior. The MCP creation arguments only add fields. The
ownership of the local MCP and the agents of each provider do not change. Future
local executors can use the same IDs, events and link model. But they must have
their own authenticated transport back from the Cloud and their own grants.

Host registry writes use version 2. When the host writes, it changes version 1
records to version 2. Older hosts refuse that version. They do not write the
registry again without the links and the events in the queue. Before a
downgrade, keep a compatible host and keep the registry.

The `prometeu-core::supervision` tests examine the completion of root and child
turns, duplicate events and periods of blocked work. They also examine
interruptions, a new start and the shared JSON examples. The host tests examine
the private configuration, the MCP link and the durable HTTP delivery with
synthetic hooks. The supervision integration tests and service tests of the
Cloud cover these items:

- The authentication of requests.
- The validation of metadata.
- The accounts stay isolated from each other.
- The GitHub routing.
- The signatures.
- The groups of events.
- The retries.
- The validation of destinations.

The Cloud imports the shared fixture with `bin/contracts`. These checks do not
prove a deployment or a run with a live external supervisor.
