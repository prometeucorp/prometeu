---
type: Contract
title: "Optional Prometeu account"
description: "The SaaS, the authentication in the browser, the desktop connection and the private storage."
status: stable
---

# Optional Prometeu account

The client and the different `prometeu-cloud` project obey this contract. The
publication of the service is a different operation.

## Boundary

The account is a Prometeu account, not a Claude or Codex account. The SaaS owns
the registration, the authentication, the profile and the login sessions. The
app does not send workspaces, transcripts or provider secrets to the SaaS. [Private feedback](feedback.md) sends only the typed text and the selected
image, with a notice that the Prometeu team keeps it private. To send feedback,
a connected account is necessary
([ADR 0035](../decisions/0035-feedback-requires-account.md)). Without an
account, the panel shows the same device authorization as the sidebar. The
desktop operates without an account. It does not call the account APIs when it
is disconnected.

An account can link one GitHub identity, at the sign-in or after it from the
Cloud settings. The Cloud stores the GitHub user ID and login. It does not store
a GitHub token. Linked accounts receive
[GitHub notifications](github-notifications.md) that contain only metadata.

Optional [workspace supervision](supervision.md) adds the configuration of
destinations, host events with only metadata, and durable webhooks that the
Cloud sends.
Its ingestion credentials are for the supervisor only (and for one executor
when the desktop provisions it). They are different from desktop sessions. The
remote MCP connection flow uses the account to issue an ingestion credential.
It sends the credential directly to the host through SSH. The Cloud stores
ingestion digests and an encrypted secret that signs the callbacks. It does not
store the credentials of the agent providers. The remote host is the first
publisher. Local supervision is not available at this time.

The top of the sidebar keeps the Prometeu logo and name. When the account is
disconnected, the sidebar shows "Create account" on the right, on the same
line. When the account is connected, the sidebar shows the name of the person
below the brand. The email is in the tooltip, and the indication of an offline
account is adjacent to the name.

The `prometeu-cloud` project uses Rails 8.1, SQLite and ERB. Its authentication
comes from the native generator of Rails and `has_secure_password`. It has
these features:

- Registration with an email and a password.
- Email verification.
- A view of the profile and changes to it.
- An email change with a confirmation.
- A password change and a password recovery.
- Logout.
- The revocation of login sessions.
- The deletion of the account with a password.

Production must have SMTP. Local development can operate without email.
Each session stops after 30 days, and this period does not change. The server
can revoke a session. The service does not make the period longer
automatically.

Puma accepts request bodies of a maximum of 7 MiB for feedback images. The
desktop endpoints keep their controller limits: 256 KiB for a catalog PUT and
16 KiB for the remaining desktop API. The production smoke test of the Cloud
examines those routes through Puma. Thus, the transport cannot silently set a
smaller limit for a feature.

The Rails adapter of the Design System renders the controls of the site. Its
local runtime adds menus, a control that shows the password, an optional
revocation confirmation and a protection against a second send of a form. The CSP lets local scripts
operate with a nonce. It does not let `unsafe-inline` or `eval` operate. Forms
continue to send native POSTs with CSRF and the button values. Without
JavaScript, the registration, the login and the Mac authorization continue to
operate. See [ADR 0017](../decisions/0017-executable-design-system.md).

## Connecting the Mac

The backend uses `POST /api/auth/device/code`, with
`client_id=prometeu-desktop`. It opens `/device?user_code=…&mode=signup` in the
system browser. The person makes an account or signs in. Then the person
compares the code with the code on the desktop and gives explicit approval. When
the person clicks "Create account", the browser opens directly, without a
dialog on the desktop. The code stays in the sidebar while the authorization
waits. Its menu lets the person open the browser again or cancel. The app finds
the connection automatically. The desktop sends `POST /api/auth/device/token`
at intervals. It obeys `interval`, `authorization_pending`, `slow_down` and the
expiry. The returned token authenticates `GET /api/auth/get-session` through
Bearer. The app cannot use a code a second time.

Credentials stay only in Rust. The frontend receives only these items: the
profile, the origin, the offline state, the public confirmation code and the
local ID of the login try. The backend makes the approval URL from the configured
origin. It does not follow HTTP redirects. A person types passwords only on the
SaaS page.

`PROMETEU_CLOUD_URL` sets the origin in the app process. The default value is
`https://app.prometeu.co`. HTTPS is necessary for all origins other than
loopback. The app stores the origin together with the token. Thus, after a
change to the configuration, the app does not send a current credential to a
different server.

## IPC

| Command | Arguments | Return |
| --- | --- | --- |
| `cloud_status` | `{ refresh: boolean }` | `{ user, origin, offline }` |
| `cloud_login_start` | `{ signup: boolean }` | `{ id, user_code, url, interval }` |
| `cloud_login_poll` | `{ id: string }` | connected status, or `null` while the authorization waits |
| `cloud_login_cancel` | `{ id: string }` | empty |
| `cloud_logout` | none | disconnected status |

`user` is `null` or `{ id, name, email }`. `refresh=false` reads only the local
cache. A temporary failure keeps the identity and sets `offline`. A revoked or
expired session removes the credential. The catalog synchronization during that
refresh is not mandatory. A short Cloud failure does not cause a failure of
`cloud_status` and does not show a toast. Error pages that are not JSON (for
example, HTML from a 502) are network failures, not incorrect responses. The UI
refreshes when the window gets the focus again. It also refreshes at intervals
of 60 seconds while a person can see it.

At logout, the app first revokes the session on the service. After a network
failure, the account stays connected and the app shows an error. The app does
not show that the revocation occurred. A cancellation cancels the login try. A
late response does not replace the current login try. If a login was completed
before the cancellation, it is a connected session, and the person can stop it
from the menu.

## Persistence and compatibility

`<root>/cloud.json` contains `{ origin, token, user }`, with atomic writes and
`0600` permissions in a `0700` folder. When the file is not there, the app
operates only locally. Transcripts and provider accounts keep their formats.
The board and `team.json` get optional fields for the consent and the
organization selection. The app can continue to read the previous data. See the
[organizations contract](cloud-organizations.md). When a person deletes the
account in the SaaS, the SaaS revokes the login sessions. It does not delete
local data. The mock simulates the flow without a network and stores only a
fictional profile in localStorage.

Rails replaced the Node prototype. It kept the four routes of the desktop,
their payloads, and the logout through `POST /api/auth/sign-out` with Bearer
and a JSON body. An incorrect session returns JSON `null` in `get-session` and 401
on logout. The prototype had no production data. Rails does not use its
database again. Test tokens from the prototype must connect again. Rails forms with CSRF
replaced the internal routes of the site. The desktop did not use those routes.
See [ADR 0015](../decisions/0015-cloud-rails.md).

At this time, the account is the repository of the catalog of plugins, MCP servers and
Actions. See [`cloud-catalog.md`](cloud-catalog.md). Transcripts in the cloud
are not part of this stage. Collaboration uses [E2EE v4](relay-v4.md), which is
different from the account credential. Organizations use the account identity
to authorize the collaboration in the relay. See the
[organizations contract](cloud-organizations.md).


## Evidence

- [`fixtures/cloud-api.json`](../../fixtures/cloud-api.json): synthetic session
  and signed-out payloads. The Rails endpoints in
  `prometeu-cloud/test/integration/desktop_contract_test.rb` and the production
  profile decoder in `src-tauri/src/cloud.rs` examine them. See the
  [shared fixture workflow](../operations/development.md#shared-cloud-api-fixtures).
- `src-tauri/src/cloud.rs`: the origin validation, and that the status does not
  contain the token.
- `e2e/cloud.spec.ts`: the connection, the storage, the logout, the offline
  account, the revocation and the conversations that stay, through the browser
  mock.
- `prometeu-cloud/test/integration/accounts_test.rb`: Rails requests, CRUD,
  isolated data, the desktop contract, the approval, the revocation, CSRF, rate
  limits and email.
- `prometeu-cloud/test/models/device_grant_test.rb`: a code that concurrent
  requests can use only one time.
- `prometeu-cloud/test/browser/accounts.spec.js`: actual HTTP and forms with
  CSRF in Chromium and WebKit, in a mobile viewport.

The mock does not prove that Tauri opens the browser. It does not prove the
delivery through an actual SMTP server.

## Personal catalog

The same account gives a web editor at `/catalog`, and `GET/PUT /api/catalog`
with Bearer on the desktop. `GET /api/organizations/:id/catalog` gives the
catalog of the organization to its current members. A personal copy is not
necessary before the desktop installs an item. The connection does not publish
local items automatically. The [catalog contract](cloud-catalog.md) specifies
the explicit sharing, the revisions, the formats and the compatibility. When
the account changes, the app forgets the previous links and keeps the local
files.

## Organizations

`cloud_organizations` and `cloud_relay_ticket` are additive IPC commands. The
[organizations contract](cloud-organizations.md) specifies them. The second
command returns a short ticket, not the desktop credential. After a revocation of
the account, the app also cannot extend the access to the organizations.
