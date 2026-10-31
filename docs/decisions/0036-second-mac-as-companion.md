---
type: Decision
title: "ADR 0036 — a second Mac as a companion device"
description: "A second Mac as a companion device."
status: stable
---

# ADR 0036 — a second Mac as a companion device

Date: 2026-09-11

Extends [ADR 0027](0027-companion-devices.md) and makes the
"one active identity per membership" rule of
[ADR 0022](0022-end-to-end-encryption.md) precise, without changing
[relay v4](../contracts/relay-v4.md).

## Context

Two Macs with the same account receive the same membership and therefore the
same member ID in the relay. Each Mac generates its own private identity. The
relay stores one key per member: the last Mac to connect replaces the other's
key. The replaced Mac sees its own key swapped in the directory, never completes
the handshake and fails to share with "Encrypted sharing failed". Both Macs
reconnect with backoff and replace each other in a cycle; the phone sees the
person's key change and blocks the content.

## Options considered

1. **Synchronize the private key between Macs.** It requires transporting the
   key through the Cloud or pairing; the Cloud would start touching key
   material.
2. **Every Mac becomes a device (v5).** It migrates the first Mac's identity,
   invalidates TOFU links and makes old comments unreadable.
3. **Only the first Mac keeps the membership; the others join as companions.**
   It reuses ADR 0027's model without a migration.

## Decision

Option 3. Each Mac generates a device ID (`device.json`, separate from the login
so it survives logout) with a label taken from the computer's name. Rust sends
that ID in `GET /api/organizations?device=` and in
`POST /api/organizations/:id/relay-ticket { device, label }`.

The membership gains `desktop_id`: the first device to query or request a ticket
claims the membership and keeps `member` equal to the membership's ID, exactly
as before. Any other Mac of the same person receives `member` equal to its own
device ID, registered as a `Companion` with the Mac's label, and the roster
lists it with `person` equal to the membership. Companion tickets now accept a
desktop session in addition to a browser session. Old desktops that do not send
`device` keep identifying the membership.

In the client, a companion may own shares. Audience, mentions and the admission
of `watch` and `write` resolve each member to the person it belongs to, whether
it is a membership or a companion. Devices of the same person as the owner,
including the first Mac, receive the share only through remote control
(ADR 0030). The audience selector omits the owner's own person.

## Consequences

- No identity migration for someone who uses a single Mac. Links, comments and
  audiences stay valid.
- Each organization's first Mac stays fixed in `desktop_id`. If it ceases to
  exist, the other Macs keep working as companions; releasing the membership to
  another Mac still requires intervention in the Cloud and goes through the key
  change flow on the peers.
- When updating Macs that were already colliding, the first to query the Cloud
  keeps the membership; the other receives a new identity as a companion and the
  phone may see a key change once.
- A companion Mac counts toward the person's five companion slots and toward the
  relay's roster of 64.

## Evidence

- `src/team-channel.test.ts`: a companion owning a share reaches the team
  without its own devices and, with remote control, the first Mac and the phone.
- `prometeu-cloud/test/integration/organizations_test.rb`: the first Mac keeps
  the membership, the second Mac becomes a companion with a label, a desktop
  without `device` stays the same, an invalid ID is rejected.
