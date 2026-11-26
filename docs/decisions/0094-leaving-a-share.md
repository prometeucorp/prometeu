---
type: Decision
title: "ADR 0094 — Leaving a share, enforced by the relay"
description: "A viewer goes out of a share for all of the devices of that viewer. The relay keeps the viewer out until the owner lets the viewer in again."
status: stable
---

# ADR 0094 — Leaving a share, enforced by the relay

Date: 2026-10-23

This ADR adds to these decisions:

- The audience rules of [ADR 0022](0022-end-to-end-encryption.md).
- The companion devices in [ADRs 0027](0027-companion-devices.md) and
  [0036](0036-second-mac-as-companion.md).
- The remote control in [ADR 0030](0030-remote-control.md).

## Context

Only the owner selects the persons who receive a share. A viewer did not have a
procedure to go out of a share. Some shares came from a teammate that the viewer did not follow
at that time. Other shares had an owner that was not there at that time. These
shares stayed in "Shared" without a limit in time.

The relay keeps the shares of an owner that is offline in the list until that
owner connects again. Only the owner can send `unshare`. Thus a Mac that a
person formatted, or a membership without its first Mac, keeps shares that no
person can remove. This also occurs on the companion devices of the owner.

## Options considered

1. **Hide locally.** The viewer keeps a list of hidden shares. The protocol
   does not change. But the owner continues to encrypt content for that person.
   The owner does not get information that the person went out.
2. **Let all persons cancel the share.** This option is simple. But a viewer
   can then remove the share for all other viewers.
3. **Exit that the relay controls.** The relay removes the devices of the person from the audience of the share. It
   keeps these devices out of subsequent announcements until the owner lets the
   person in again. The owner knows which person went out and stops encryption
   for that person.

## Decision

Use option 3.

- **Exit.** A device in the audience that is not the owner sends
  `leave { ws }`. The relay does these steps, as when the owner makes the
  audience smaller:
  - It removes all devices of that person (`Member.person` or the device) from
    the stored audience.
  - It adds the person to the `left` list of the entry.
  - It disconnects the devices of the person.
  - It sends `unshare`.
  - It removes the inbox items of the share.

  It is not necessary that the owner is online.
- **Stay out.** While a person is in `left`, the relay removes the devices
  of that person from each `share` announcement. When an owner that was offline
  connects again, it announces its stored audience again. That audience is not
  current, and it must not put the person back.
- **The owner learns.** `Shared.left` goes to the owner with the share of the
  owner. The channel of the owner then removes those persons from the
  recipients for which it encrypts. The owner ignores the watch requests, input and
  snapshots of those persons. Prometeu does not write the board audience again. A share for all
  of the organization stays a share for all of the organization. Persons that
  become members after this time continue to receive it.
- **Readmission.** Only the owner sends `readmit { ws, person }`. The relay
  removes the person from `left`. The next announcement puts the person back.
  The share menu identifies the persons who went out. When the owner selects one,
  Prometeu lets that person in again. If the owner removed that person from an
  explicit audience during this time, the audience gets the person again. For
  the person of the owner, when the owner enables remote control again, the
  devices of the owner come back in.
- **Lifetime.** `left` is in the relay entry. When the owner stops the share,
  the relay deletes the entry and the list. A new share is a new decision, and
  it includes all persons that it names. The roster removes persons who went
  out of the organization from `left`. Thus the list stays in the limit of 64
  members.

## Consequences

- A viewer can always remove items from "Shared". The viewer can also remove a
  share when the owner will not come back. A person can go out of a share of
  that person from a different device. Then Prometeu removes the share from all
  the companion devices of that person.
- The trust model does not change. A malicious relay can, at this time, keep
  content from all persons. An incorrect `left` only makes the owner stop the
  encryption for a person. The relay cannot let in again a person that the
  owner removed. The relay cannot get back content that a device received
  before.
- Protocol v4 adds `left` and the two frames. Earlier clients continue to operate. Clients from earlier
  versions ignore `Shared.left`. A relay from an earlier version answers `leave`
  with a `bad` error. Thus, as usual, the team deploys the Worker before
  it publishes the desktop app.
- The mobile shell does not let a person go out of a share at this time.

## Evidence

- `relay/src/logic.test.ts` ("leaving"): audience, detachment, inbox, stale
  announcements, readmission, companion devices, refusal and persistence.
- `relay/src/protocol.test.ts`: the parse of `leave`, `readmit` and `Shared.left`.
- `src/team-channel.test.ts`: the owner stops encryption for a person who went
  out. It starts again after the owner lets the person in again. The test uses
  the relay reducer of the production code.
- `src/team-owner.test.ts`: the owner ignores input from a person who went out
  before its receipt. Readmission puts the input back or adds the person to an
  explicit audience.
