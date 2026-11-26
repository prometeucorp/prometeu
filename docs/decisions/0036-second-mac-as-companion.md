---
type: Decision
title: "ADR 0036 — a second Mac as a companion device"
description: "Only the first Mac of a person keeps the membership. A second Mac connects as a companion device."
status: stable
---

# ADR 0036 — a second Mac as a companion device

Date: 2026-09-11

This ADR adds to [ADR 0027](0027-companion-devices.md). It also makes the
"one active identity per membership" rule of
[ADR 0022](0022-end-to-end-encryption.md) accurate. It does not change
[relay v4](../contracts/relay-v4.md).

## Context

Two Macs with the same account get the same membership. Thus, they get the same
member ID in the relay. Each Mac makes its private identity. The relay stores one
key for each member, and the last Mac that connects replaces the key of the
other Mac. This causes these problems:

- The replaced Mac sees a different key in the directory, not its key.
- The replaced Mac does not complete the handshake. The share stops with
  "Encrypted sharing failed".
- The two Macs connect again with backoff and replace the key of the other Mac
  again and again.
- The phone sees a change of the key of the person and does not show the
  content.

## Options considered

1. **Copy the private key to each Mac.** The key must go through the
   Cloud or through pairing. Thus, the Cloud starts to touch key material.
2. **Every Mac becomes a device (v5).** This changes the identity of the first
   Mac. It makes TOFU links incorrect, and the application cannot read previous
   comments.
3. **Only the first Mac keeps the membership. The other Macs connect as
   companions.** This uses the model of ADR 0027 again without a migration.

## Decision

Use option 3. Each Mac makes a device ID with a label from the name of the
computer. The device ID is in `device.json`, apart from the login, and it
stays after a logout. Rust sends that ID in `GET /api/organizations?device=` and
in `POST /api/organizations/:id/relay-ticket { device, label }`.

The membership gets `desktop_id`. The first device that sends a query or a request
for a ticket gets the membership. This device keeps `member` equal to the ID of
the membership, as before. Each other Mac of the same person gets `member` equal
to its device ID. The Cloud records that Mac as a `Companion` with the label
of the Mac. The roster shows it with `person` equal to the membership. After this change, companion
tickets accept a desktop session and a browser session. Desktops from a previous version that do
not send `device` continue to identify the membership.

In the client, a companion can own shares. The audience, the mentions and the
admission of `watch` and `write` find the person of each member. The member can
be a membership or a companion. The devices of the same person as the owner get
the share only through remote control (ADR 0030). This also applies to the first
Mac. The audience selector does not show the person of the owner.

## Consequences

- A person who uses one Mac has no identity migration. Links, comments and
  audiences stay correct.
- The first Mac of each organization stays in `desktop_id` and does not change. If
  this Mac is not available after that time, the other Macs continue to operate as
  companions. To give the membership to a different Mac, a person must change
  data in the Cloud. The peers then see the key change procedure.
- A person can update two Macs that replaced the keys of the other Mac. Then
  the first Mac that sends a query to the Cloud keeps the membership. The other Mac gets a new identity
  as a companion. The phone can see one key change.
- A companion Mac counts in the limit of five companions of the person and in
  the relay roster of 64.

## Evidence

- `src/team-channel.test.ts`: a companion that owns a share sends it to the team
  without its devices. With remote control, it also sends it to the first Mac
  and the phone.
- `prometeu-cloud/test/integration/organizations_test.rb`: the first Mac keeps
  the membership. The second Mac becomes a companion with a label. A desktop
  without `device` does not change. The Cloud does not accept an incorrect ID.
