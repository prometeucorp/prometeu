# Personal remote MCP

Status: implemented, opt-in. Decision: [ADR 0095](../decisions/0095-personal-remote-mcp.md).

## Product and trust

Settings / Work / Remote access binds selected project IDs and source paths to
the current Cloud account. The app must remain open and the Mac awake. There is
no daemon, inbound port, wake service or offline execution queue. Disabling
locally rejects subsequent tool dispatch; accepted work continues in the app.

Manage integrations opens the Cloud browser UI. Connect an OAuth client or create
a manual token for selected projects. The endpoint is `RELAY_URL/mcp`. Manual
tokens expire after 30 days and appear once. OAuth access tokens last one hour;
rotating refresh tokens last 30 days. Revoking a grant prevents new requests.
Deleting its desktop login removes its device, grants and receipts, not local work.

Rails owns registrations, consent, token digests, tickets and webhook receipts.
The existing Worker hosts a separate `RemoteDevice` Durable Object per device.
Organization membership and collaboration's E2EE protocol confer no authority.
Rust initiates WSS with a single-use ticket in the Authorization header, renewable
every 25 seconds with a maximum 60-second lease. The desktop Bearer only goes to
Rails. Reconnect uses bounded backoff. The backend connection outlives UI navigation.

The gateway terminates TLS and can read forwarded tool arguments and results.
Desktop settings and browser consent disclose this. This is not collaboration
E2EE. Provider credentials stay on the Mac. Rails receives no prompts, transcripts
or file contents; its webhook endpoint accepts only a closed metadata schema.
The authorized external agent receives the content returned by its allowed tools.
Project scopes govern MCP operations, not an OS sandbox around the CLI.

## Protocol and authorization

Streamable HTTP supports MCP `2025-06-18`, JSON responses to POST, and GET SSE for
resource notifications. Clients need that revision or its compatibility fallback.
Experimental Tasks and vendor-specific wake APIs are not advertised. Automated
tests do not certify individual vendor applications or their background behavior.

`/.well-known/oauth-protected-resource/mcp` identifies Rails as the authorization
server. Rails exposes authorization-server metadata, public-client registration,
browser consent and token exchange. Redirect URIs match registrations exactly.
PKCE S256 and `resource=RELAY_URL/mcp` are required at authorization and exchange.
Consent keeps CSRF and a ten-minute signed intent bound to the logged-in user;
login preserves the authorization return path. Redirects require HTTPS except
loopback HTTP. No client-metadata URL fetching or token passthrough is implemented.

Scopes are `prometeu:read` and `prometeu:write`. Each grant selects one device and
project subset. Every public HTTP request is introspected in Rails. The Mac checks
local opt-in, account, pinned project paths and operation scope before dispatch.
Caller arguments cannot override identity. Cookies and desktop Bearers cannot
authenticate the public MCP endpoint.

Both transports share `embedded_mcp::dispatch`: schema validation, serialization,
ownership and response bounds. Remote owners use `remote:<grant UUID>`; existing
external local owners retain `client:<UUID>`. All existing
[delegation tools](embedded-mcp.md#tools) remain available within scope. The added
`list_models` queries the selected local Claude/Codex account without inference.
Provider/model/effort selection, workspace creation and live app presentation reuse
existing use cases. Questions and approvals remain for the person in Prometeu.

`get_device` returns `online` and `last_seen_at` (Unix milliseconds, last authorized
connection/renewal). `read_events` accepts a nonnegative `after` cursor and returns
up to 100 authorized events, `next_cursor` and `gap`. Offline tools return
`isError` with `device_offline`; metadata, cached discovery and retained events
remain available. A heartbeat lease may remain apparently online for up to 60
seconds after suspension. Pending calls time out after 25 seconds with an unknown
outcome. Reconcile using the original `request_key`; never invent another key for
an uncertain retry. The gateway does not queue offline work or invent completion.

## Events and delivery

The Mac writes execution ID, state, outcome, workspace availability/preparation
state, `input_required` and request IDs to a
private durable outbox after ordered publication, outside conversation locks.
Persisted owner/account mappings prevent cross-account uploads. Reconnect also
reconciles current delegation state: this is a state-change feed, not an archive
of every intermediate provider event. No question text or transcript is in events.

Events have stable UUID keys, client/project IDs and bounded metadata. The gateway
deduplicates keys and retains up to 1,000 events for 30 days. Cursors are durable
and monotonic; `gap` requires reconciliation. Event `at` means gateway receipt time.
The local outbox holds at most 10,000 unconfirmed events; at capacity, later state
is reconciled after backlog drainage. Removal requires gateway persistence and
Cloud webhook-ingestion acknowledgment.

Clients can subscribe to `prometeu://events`. GET SSE delivers standard
`notifications/resources/updated`; read the resource or poll `read_events` to get
metadata. Streams close within 30 seconds so reconnect rechecks revocation.
Cursor polling recovers missed notifications.

Each integration may have one optional HTTPS callback. Cloud persists metadata
receipts and delivers at least once, with a five-minute retry lease. Schedule
`bin/rails remote:deliver` every minute to recover interrupted jobs and prune
receipts after 30 days. Without the schedule, process interruption can delay
delivery indefinitely. Revocation or removal of the URL stops delivery.

Headers: `Prometeu-Event-Id` and
`Prometeu-Signature: t=<Unix seconds>,v1=<hex HMAC-SHA256>`. Signatures cover
`timestamp + "." + raw_body` using the integration secret shown after saving its
callback. Receivers must reject old timestamps, compare signatures securely and
deduplicate IDs. Only HTTPS port 443 without userinfo/query/fragment is accepted.
Every delivery rejects private/reserved DNS answers, pins the validated IP while
preserving TLS hostname verification, disables proxies and redirects, and bounds
timeouts and response size. Tests replace delivery transport; no real callback runs.

Notifications cannot force a closed third-party chat to resume. The receiver must
maintain a subscription, poll periodically, or provide a callback that resumes its
own orchestration.

## Operation, compatibility and evidence

Deploy Cloud migrations/routes first, Worker with its additive `v2` migration
second, then desktop. Set Worker's `CLOUD_URL` and Rails's `RELAY_URL` consistently;
OAuth resource and metadata URLs must agree. Use HTTPS outside development loopback.
Configure the recurring delivery task before offering reliable callbacks.

Rollback disables remote access but retains additive tables, private files and
delegation owners for re-upgrade. Do not drop tables containing live grants or
receipts. Worker collaboration rooms and local stdio remain independent.
Local private files: `remote-access.json`, `remote-owners.json`, `remote-events.json`.
The WebSocket dependency reuses the existing rustls stack.

Tests: Rust `remote_access` covers path pinning/gateway validation; common MCP and
delegation tests cover dispatch/ownership. `relay/src/remote.integration.test.ts`
runs real Worker HTTP/WebSocket/SSE paths with fake Cloud authentication. Rails
`remote_access_test.rb` covers PKCE, isolation, one-use tickets and revocation;
`remote_webhooks_test.rb` covers receipts, signatures, retries and network ranges.
