---
type: Contract
title: "Organizations in Prometeu Cloud"
description: "Operations on organizations, invitations, catalogs and access to the relay."
status: stable
---

# Organizations in Prometeu Cloud

The app obeys this contract. [ADR 0021](../decisions/0021-cloud-organizations.md)
records the decision.

## Ownership and access

An organization is the collaboration scope that had the name "team" before. The
Cloud owns the name, the unique slug, the owner, the members and the
invitations. Teams cannot contain teams. A person can be a member of more than
one organization. The desktop keeps one organization active. The owner and the
administrators change the organization, invite members and edit its catalog.
Only the owner can promote administrators, control other administrators and
delete the organization. Members can go out of an organization, but the owner
cannot. Before a person deletes an account, the person must delete the
organizations that the account owns.

Action Mailer sends the invitations in Portuguese or English. They expire after
7 days. The link opens a review page. A GET request does not give access. To
accept, these items are necessary:

- A POST with the cookie and CSRF.
- An authenticated account.
- An email that the Cloud examined, and that is the same as the email of the
  invited person.

After the registration, the verification and the login, the
person goes back to the invitation. The signed token includes a nonce, the
recipient, the role and the consumption state. When the Cloud sends an
invitation again, revokes it or accepts it, the previous links stop.

When a person accepts, the Cloud makes the membership and uses the invitation in the same SQLite
transaction. An organization can have a maximum of 64 members. This is the
limit of the collaboration protocol. After an SMTP failure, the invitation stays
open, and the Cloud shows an error. Thus, a person can send it again. Tests use
only delivery in memory.

The catalog of the organization uses the same `Catalog` model, with only one
owner: `user_id` or `organization_id`. A database constraint makes sure of this.
Each document has its own revision, a limit of 256 KB and the same portability
validation as the personal catalog. Members can read the catalog. The owner and
the administrators do the CRUD operations. Access by slug does not replace the
authorization through the membership of the authenticated account.

When a person shares a personal definition with the organization, the Cloud
copies only that item. Members see the definitions directly in the desktop hubs
and select `Install
here`. Installations are local and do not change the catalogs. A copy into the personal
catalog is not necessary. The Cloud continues to let a person copy items
between catalogs. Subsequent changes do not go to the other catalog. Collisions
return 409, and the Cloud does not replace the item. The app does not enable
received code automatically. See
[ADR 0039](../decisions/0039-organization-catalog-on-desktop.md).

## API and relay

The previous routes for the account and the personal catalog do not change.

| Route | Authentication and response |
| --- | --- |
| `GET /api/organizations?device=` | desktop Bearer. `{ organizations: [{ id, slug, name, member, role }] }`, only accepted memberships. The optional `device` sets `member` (see below) |
| `GET /api/organizations/:id/catalog` | desktop Bearer. `{ catalog, revision }`. A current membership is necessary. Returns 404 without access. The desktop cannot write to the catalog of the organization |
| `POST /api/organizations/:id/relay-ticket` | desktop Bearer. JSON `{ device?, label? }`. Returns `{ ticket, relay }`. A current membership is necessary. Returns 422 for an incorrect `device` |
| `POST /api/relay/authorize` | JSON `{ ticket, organization }`. A capability for one use. Returns `{ organization, member, name, expires_at, members: [{ id, name, person? }] }` or 401 |
| `POST /orgs/:slug/companion-ticket` | Cookie and CSRF. JSON `{ companion, label? }`. Returns `{ ticket, relay }`. A current membership is necessary. Returns 422 for an incorrect ID or for an ID of a different person |
| `DELETE /companions/:id` | Cookie and CSRF. Removes the device of the person and revokes its tickets |
| `GET /app` | Cookie. The phone page with the canonical origin, the user and the memberships in `data-*`. `connect-src` includes the relay |
| `GET /app/manifest` | Public. The installation manifest with `start_url` `/app` |

`id` and `member` are stable identifiers that have no meaning to the client. They are different from the
slug and the name. Each ticket has 256 random bits and a SHA-256 hash in the
database. It expires after 60 seconds and has a link to the membership and the
desktop session. The consumption is a transaction. After these events, nobody
can use the ticket: the removal of the membership, the deletion of the
organization, or the revocation of the session. The desktop Bearer stays in
Rust and in the Cloud. It does not go to the relay or to the webview.

Rust validates the `relay` origin and returns only a WSS URL with a short
ticket. The frontend connects to `/organization/:id?ticket=…&p=4`. The relay
queries the configured Cloud origin and does not follow redirects. It ignores
the identity and the name that the client supplies. It uses the Durable Object
namespace `organization:<id>`, which is different from the namespace of legacy
teams. An enrollment through a shared secret does not give access here. The
Cloud roster includes offline members. The app uses them in audience selections.

A browser connects as a companion device. `companion` is an `[A-Za-z0-9_-]{16,64}`
ID that the browser makes together with its private identity. The ID is unique
in the Cloud and is connected to the person who added it. The companion
ticket is possible only with a browser session. It makes `member` the ID of the
companion. The roster shows each companion with these values:

- `person` is the membership of the person in that organization.
- `name` is the name of the person, with the label in parentheses.

The roster always has space for the persons. Companions get the remaining
positions, up to 64, by the most recent use. Each person can have a maximum of five
companions. To remove a device, use Settings → Devices. See
[ADR 0027](../decisions/0027-companion-devices.md).

Each Mac sends `device`. This is an ID in the same format that the Mac makes one
time in `device.json`. The ID stays after a logout. The Mac also sends `label`,
with the name of the computer as its value. The first device that queries or
requests a ticket claims the membership (`desktop_id`). This device continues
with `member` equal to the membership ID. Each other Mac of the same person gets
`member` equal to its own `device`. The Cloud records that Mac as a companion
with the label of the Mac. Companion tickets accept a desktop session or a
browser session. Without `device`, the desktop identifies the membership as
before. See [ADR 0036](../decisions/0036-second-mac-as-companion.md).

The claim of the first device refreshes the membership under a database lock.
Concurrent claims see the same stored `desktop_id`. The device that is second
becomes a companion and does not replace the primary identity.
`test/models/membership_test.rb` in the Cloud examines this with claims at the
same time.

The connection has a lease of a maximum of 60 seconds. The expiry of the login
also limits the lease. An alarm closes expired sockets. Entry and exit examine
the deadline also if the alarm is late. After the identity proof, the relay
sends `lease { expires_in }`, in milliseconds. The desktop and the phone request
a new ticket at the half of that period. They send `renew { ticket }` through
the same socket. The relay queries the Cloud again. The organization and the
member must be the same, and the socket must stay open. The confirmation is a
new `lease`. When the roster does not change, the relay does not send the
handshake, the presence data or the snapshots again.

After a removal or a revocation, the relay stops new traffic in 60 seconds or
less, also after a renewal. A new membership uses a different ID and does not
get back previous private audiences. A Cloud failure stops the renewal. Local work
stays available.

Previous clients ignore `lease` and connect again when the deadline goes by. New
clients extend the lease only when the relay tells that it has that capability. A previous
relay keeps the path where the client connects again. After an actual
connection loss, the client gets a new ticket with backoff and gets the
snapshots again. TOFU and receipts do not change. Because the lease time is
relative, the clocks of the devices can be different.
[ADR 0034](../decisions/0034-mobile-pairing-continuity.md) records the
decision. The alarm uses the
[native Durable Objects API](https://developers.cloudflare.com/durable-objects/api/alarms/).

Processes and transcripts stay on the Mac of the owner. The relay sends the
encrypted content to the devices. It keeps encrypted comments and metadata.
Rails receives only the identity, the memberships and the portable definitions.
[Relay v4](relay-v4.md) adds E2EE with TOFU and local identities that are not
connected to the tickets.
[ADR 0022](../decisions/0022-end-to-end-encryption.md) gives the limits. For
example, there is no forward secrecy.

## IPC, consent and compatibility

- `cloud_organizations`: returns `{ user, origin, organizations }`. It does not
  use the network while the account is disconnected.
- `cloud_relay_ticket`: receives `{ organization, user, expectedOrigin }`. The
  identity and the origin must be the same as the local credential. It returns
  the temporary URL. Rust adds the device ID and label (`device.json`) to the
  two IPC commands.
- `team_config_set`: continues to store private JSON. The configuration of an
  organization keeps the legacy fields, with empty `secret` and `credential`.
  It adds `cloud: { user, origin, slug, name }`. The app does not store tickets.
- `set_shared`: receives the `remoteControl` argument, the optional `team`
  argument and the optional `rights` argument. It stores `remote_control`,
  `share_team` and `rights` in the workspace. An organization uses
  `organization:<id>:<member>`. A legacy team uses `team:<id>`. When previous boards
  do not have these fields, the app lets only the legacy path operate. A missing
  `remote_control` means `false`. When `rights` is missing, persons can only see
  and comment ([ADR 0090](../decisions/0090-approved-remote-input.md)).
- `set_confirm_messages`: receives `id` and `confirm`, and stores
  `confirm_messages` in the workspace. It does not change with `set_shared`. A
  missing field means `true`: the messages of team members wait for the owner.

When a person has only one accepted membership, the desktop makes it active
automatically when it shows the organizations. When the person goes out of the
organization, the app stores that decision in `localStorage`
(`prometeu:organizacao-saida`). This is a local preference for each machine. It
disables the automatic selection until the next explicit selection. When a
person selects a different organization or account, the app does not show the
previous shares in that new scope. A snapshot that waits records the connection
where it started. The app discards it after a change of connection or a
revocation of the share. Previous frames do not change the state of the new
connection. The selection of an organization does not publish workspaces.

Legacy teams keep their credentials, but the updated desktop must have v4. The
relay reads and writes the collaboration data under `v4:`. The v3 data does not
change, and the new client does not show it. The app does not change v3
comments and does not go back to v3. The desktop UI does not contain the
creation of teams, the invitation codes and the member edits.

The first time that an organization replaces a legacy configuration, the app
keeps a private copy in `<root>/team-legacy-<uuid>.json`. The app cannot find
the emails from the previous anonymous identities. Make the organization, invite the
members by email, and share each workspace again by selection. Local
conversations and legacy storage do not change.

## Publication and rollback

1. Publish the Cloud and its additive migration. Keep the SQLite snapshot.
2. Publish the relay with `CLOUD_URL` that points to that Cloud (default
   `https://app.prometeu.co`).
3. In the Cloud, set `RELAY_URL` to the published relay (the current default of
   the product).
4. Supply desktop v4 after relay v4. v3 clients must use the previous relay. The
   updated relay refuses v3 connections.

For local operation, use these values:

- The Cloud uses `RELAY_URL=http://127.0.0.1:8787`.
- The relay uses `CLOUD_URL=http://127.0.0.1:3100` in `relay/.dev.vars`.
- The desktop uses `PROMETEU_CLOUD_URL=http://127.0.0.1:3100`.

Only loopback accepts HTTP and WS.

A code rollback keeps the expanded schema.

> **CAUTION:** Do not run `db:rollback` when actual organizations are in the
> database. This command removes the new data.

Keep the organization, invitation and catalog data during the compatibility
period. A previous desktop can explicitly put back the copy of the legacy
`team.json`. Do not change the consent of an organization into a legacy share.
The local validation does not include a deploy or an actual email delivery. A
rollback to v3 does not keep E2EE. It keeps the v4 namespace, but it puts back
the previous protocol, where the relay can read the content.

## Evidence

- Cloud: `test/integration/organizations_test.rb` and
  `test/browser/organizations.spec.js`.
- Desktop: `src/team-organizations.test.ts`, `e2e/organizations.spec.ts` and the
  `cloud.rs` tests.
- Relay: `relay/src/cloud.test.ts` and `relay/src/worker.integration.test.ts`
  with an actual Worker, a local authorization service, isolated data, identity,
  audience, messages and expiry.
- The current suites for the account, the catalog, the protocol, the comments
  and the remote control examine legacy clients and the two providers.
