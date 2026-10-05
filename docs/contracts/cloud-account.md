# Optional Prometeu account

Status: implemented in the client and in the separate `prometeu-cloud` project;
publishing the service is an independent operational step.

## Boundary

The account belongs to Prometeu, not to Claude or Codex. The SaaS owns
registration, authentication, profile and login sessions. No workspace,
transcript or provider secret is synchronized with the SaaS.
[Private feedback](feedback.md) sends only the typed text and the chosen image,
with a notice that the Prometeu team handles it privately. Sending requires a
connected account ([ADR 0035](../decisions/0035-feedback-requires-account.md));
without an account, the panel offers the same device authorization as the
sidebar. The desktop works without an account and does not call the account APIs
while disconnected.
An account may link one GitHub identity, at sign-in or later from the Cloud
settings; the Cloud stores the GitHub user ID and login, never a GitHub token.
Linked accounts receive metadata-only [GitHub notifications](github-notifications.md).

The top of the sidebar keeps the Prometeu logo and name. When disconnected, it
offers "Create account" on the right, on the same line. When connected, it shows
the person's name below the brand.
The email is in the tooltip, and the offline-account indication accompanies the
name.

The `prometeu-cloud` project uses Rails 8.1, SQLite and ERB, with authentication
based on Rails' native generator and `has_secure_password`. It offers
registration by email/password, email verification, reading and editing the
profile, email change with confirmation, password change/recovery, logout,
revocation of login sessions and account deletion with a password. SMTP is
required for production; local development can run without email. Sessions have
a fixed 30-day validity, with server-side revocation; there is no automatic
renewal.

Puma permits request bodies up to 7 MiB for feedback images. Desktop endpoints
retain their controller limits: 256 KiB for a catalog PUT and 16 KiB for the
remaining desktop API. The Cloud's production smoke test checks those routes
through Puma so the transport cannot silently impose a smaller feature limit.

The site's controls are rendered by the Design System's Rails adapter. Its local
runtime adds menus, password reveal, optional revocation confirmation and
double-submit blocking. The CSP allows local scripts with a nonce and does not
allow `unsafe-inline` or `eval`. Forms still send native POSTs with CSRF and
preserved button values. Without JavaScript, registration, login and Mac
authorization keep working. See
[ADR 0017](../decisions/0017-executable-design-system.md).

## Connecting the Mac

The backend uses `POST /api/auth/device/code`, with
`client_id=prometeu-desktop`. It opens `/device?user_code=…&mode=signup` in the
system browser. The person creates an account or signs in, checks the code shown
on the desktop and approves it explicitly. Clicking "Create account" opens the
browser directly, without a dialog on the desktop. The code stays in the sidebar
while authorization is pending; its menu allows reopening the browser or
cancelling. The connection is detected automatically. The desktop polls
`POST /api/auth/device/token` respecting `interval`, `authorization_pending`,
`slow_down` and expiration. The returned token authenticates
`GET /api/auth/get-session` through Bearer. A consumed code cannot be reused.

Credentials stay exclusively in Rust. The frontend receives only the profile,
origin, offline state, public confirmation code and the attempt's local ID. The
backend builds the approval URL from the configured origin and does not follow
HTTP redirects. Passwords are typed only on the SaaS page.

`PROMETEU_CLOUD_URL` configures the origin in the app process; the
default is `https://app.prometeu.co`. HTTPS is required outside loopback. The
origin is persisted alongside the token so that an existing credential is never
forwarded to another server after a configuration change.

## IPC

| Command | Arguments | Return |
| --- | --- | --- |
| `cloud_status` | `{ refresh: boolean }` | `{ user, origin, offline }` |
| `cloud_login_start` | `{ signup: boolean }` | `{ id, user_code, url, interval }` |
| `cloud_login_poll` | `{ id: string }` | connected status or `null` while pending |
| `cloud_login_cancel` | `{ id: string }` | empty |
| `cloud_logout` | none | disconnected status |

`user` is `null` or `{ id, name, email }`. `refresh=false` reads only the local
cache. A transient failure preserves the identity and marks `offline`; a revoked
or expired session removes the credential. The UI refreshes when the window
regains focus and every 60 seconds while visible. Logout revokes on the service
first: a network failure leaves the account connected and shows an error,
without pretending the revocation happened. Cancelling invalidates the attempt.
A late response does not replace the current attempt. A login completed before
the cancellation is already a connected session and can be ended from the menu.

## Persistence and compatibility

`<root>/cloud.json` contains `{ origin, token, user }`, with atomic writes and
`0600` permissions in a `0700` directory. Its absence means local-only use.
Transcripts and provider accounts keep their formats. The board and `team.json`
receive optional fields for consent and organization selection, preserving
reading of the old data; see the
[organizations contract](cloud-organizations.md). Deleting the account in the
SaaS revokes the login sessions; it does not delete local data. The mock
simulates the flow without a network and stores only a fictional profile in
localStorage.

Replacing the Node prototype with Rails preserves the desktop's four routes,
their payloads and logout through `POST /api/auth/sign-out` with Bearer and a
JSON body. An invalid session returns JSON `null` in `get-session` and 401 on
logout. The prototype had no production data. Its database is not reused by
Rails; old test tokens require a new connection. The site's internal routes were
replaced with Rails forms with CSRF; they were not consumed by the desktop.
See [ADR 0015](../decisions/0015-cloud-rails.md).

The catalog of plugins, MCP and Actions now has the account as its repository;
see [`cloud-catalog.md`](cloud-catalog.md). Transcripts in the cloud stay
outside this stage. Collaboration uses [E2EE v4](relay-v4.md), separate from the
account credential. Organizations use the account identity to authorize
collaboration in the relay; see the
[organizations contract](cloud-organizations.md).


## Evidence

- [`fixtures/cloud-api.json`](../../fixtures/cloud-api.json): synthetic session
  and signed-out payloads checked by the real Rails endpoints in
  `prometeu-cloud/test/integration/desktop_contract_test.rb` and the production
  profile decoder in `src-tauri/src/cloud.rs`. See the
  [shared fixture workflow](../operations/development.md#shared-cloud-api-fixtures).
- `src-tauri/src/cloud.rs`: origin validation and absence of the token in the
  status.
- `e2e/cloud.spec.ts`: connection, persistence, logout, offline
  account, revocation and preservation of conversations over the browser mock.
- `prometeu-cloud/test/integration/accounts_test.rb`: Rails requests, CRUD,
  isolation, desktop contract, approval, revocation, CSRF, rate limiting and
  email.
- `prometeu-cloud/test/models/device_grant_test.rb`: concurrent single
  consumption.
- `prometeu-cloud/test/browser/accounts.spec.js`: real HTTP and forms with CSRF
  active in Chromium and WebKit, in a mobile viewport.

The mock does not prove that Tauri opens the browser, nor real SMTP delivery.

## Personal catalog

The same account offers web authoring at `/catalog` and `GET/PUT /api/catalog`
with Bearer on the desktop. `GET /api/organizations/:id/catalog` offers
institutional reading authorized by the current membership, without requiring a
personal copy before installing on the desktop. Connecting does not publish
local items automatically. The [catalog contract](cloud-catalog.md) defines
explicit sharing, revisions, formats and compatibility. Switching accounts
forgets previous links, keeping the local files.

## Organizations

`cloud_organizations` and `cloud_relay_ticket` are additive IPC commands
described in the [organizations contract](cloud-organizations.md). The second
returns a short ticket, never the desktop credential. Revoking the account also
prevents renewing access to the organizations.
