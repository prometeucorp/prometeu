---
type: Decision
title: "ADR 0027 — companion devices in the relay"
description: "A companion device connects to the relay as one more member that is linked to a person."
status: stable
---

# ADR 0027 — companion devices in the relay

Date: 2026-09-08

This ADR uses an added field in [relay v4](../contracts/relay-v4.md). It adds to
[ADR 0021](0021-cloud-organizations.md) and
[ADR 0022](0022-end-to-end-encryption.md).

## Context

A phone must go into the room of the relay with its own key. The relay stores
one key for each member, and the owner does not encrypt boxes for itself. Thus a
browser with the same membership replaces the key of the Mac and does not
receive content. Also, the identity of the Mac must stay stable. A change to the
ID of the member makes the TOFU links incorrect. Then nobody can read the
previous comments.

## Options considered

1. **More than one key for each member.** This option changes the envelope
   (`boxes` for each key). TOFU then cannot find a replacement without
   cross-signatures between devices.
2. **Each member becomes a device (v5).** This option migrates the identity of
   the Mac. Each device must keep a stable ID after each new login. The storage
   must also change.
3. **A companion device as one more member.** The Mac keeps the membership
   as its identity. Browsers connect as their own members. A `person` field in
   the roster links each browser to the person.

## Decision

Use option 3. The Cloud has a `Companion` model. A companion is a browser or
one more Mac of a person. It has a `relay_id` that the device makes, a label
and `last_seen_at`.

`POST /orgs/:slug/companion-ticket` authenticates with a cookie and CSRF. It
adds or finds the companion and issues a ticket for that companion.
`POST /api/relay/authorize` returns `member` with the ID of the companion. It
also returns the roster with `{ id, name, person }` for each device. The first
Mac identifies the membership. Other Macs use companion tickets that the
desktop authenticates, as [ADR 0036](0036-second-mac-as-companion.md) specifies.
Browsers use a cookie session and CSRF for their companion tickets.

`Member.person` is optional and added in v4. Previous parsers ignore the field
and show the companion as one more member. The relay makes sure that `person`
points to a primary member of the same roster, without chains. The relay stores
the field.

In the client, audiences and mentions continue to name persons. The encrypted
channel changes each person into the devices of that person when it selects the
recipients of the boxes. It does this in the audience that it publishes in the
relay and in the mentions. When the owner validates `watch` and `write`, it
accepts a device through the person of that device. The owner operates the input
of that device only after it gives approval to the key of the device
([ADR 0090](0090-approved-remote-input.md)).

The interface selects the audience and mentions from persons (`people()`). Since
[ADR 0041](0041-members-list-shows-people.md), the members list also shows
persons. The keys stay pinned for each device, because each TOFU link is for one
key.

The roster of the Cloud keeps the limit of 64 members of the relay. Persons
always get a position. Companions get the remaining positions in the sequence of
recent use. Each person keeps a maximum of five companions. When a person
adds one more companion, the Cloud removes the companion with the longest age.

When a person removes the device in the account settings, the Cloud removes it
from the roster. In a maximum of 60 seconds, the clients stop the encryption of
new content for that device. The other members then cannot read the comments
of that device. This is the same as for removed members.

## Consequences

- The desktop does not migrate an identity. The current links, comments and
  audiences stay correct.
- Legacy teams that use a shared secret have no `person`. In these teams, each
  browser is its own membership, as before.
- Each recipient costs one box. An organization of 64 persons has no space for
  companions. To make that limit larger, the content must have a key for each
  message, not one copy for each box. That is a different envelope and a new
  review of the cryptographic protection.
- The TOFU of a companion is for each device. If a person removes the stored
  data of the browser, the browser becomes a new device. The previous device
  stays in the roster until the Cloud removes it because of its age or a person
  removes it.
- The mobile shell uses this identity model through
  [ADR 0028](0028-mobile-web-app.md).

## Evidence

- `relay/src/cloud.test.ts` has tests for a roster with `person`. The tests
  also make sure that the relay rejects links to unknown members and chains, and
  that the reducer stores the field.
- `src/team-channel.test.ts` has tests for a companion. The companion receives
  the share and the mention of the person and sends messages to the owner. A
  third person that is not in the audience continues to get no content.
- `prometeu-cloud/test/integration/organizations_test.rb` has tests for the
  companion ticket, the roster, the isolated data of each person, the removal
  and the limit of the roster.
