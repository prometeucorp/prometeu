---
type: Decision
title: "ADR 0094 — Leaving a share, enforced by the relay"
description: "A viewer leaves a share for all of their devices; the relay keeps them out until the owner readmits them."
status: stable
---

# ADR 0094 — Leaving a share, enforced by the relay

Date: 2026-10-23

Complements the audience rules of [ADR 0022](0022-end-to-end-encryption.md),
companion devices in [ADRs 0027](0027-companion-devices.md) and
[0036](0036-second-mac-as-companion.md), and remote control in
[ADR 0030](0030-remote-control.md).

## Context

Only the owner decides who receives a share. A viewer had no way out: a share
from a teammate they no longer follow, or one whose owner disappeared, stayed in
"Shared" for good. The relay keeps an offline owner's shares listed until that
owner reconnects, and only the owner may `unshare`. A Mac that was formatted, or
a membership whose first Mac is gone, therefore leaves shares that nobody can
remove, including on the owner's own companion devices.

## Options considered

1. **Hide locally.** The viewer keeps a list of hidden shares. No protocol change,
   but the owner keeps encrypting content for that person and nothing tells the
   owner they left.
2. **Let anyone cancel the share.** Simple, but a viewer would remove the share
   for every other viewer.
3. **Leave for one's own person, enforced by the relay.** The relay drops the
   person's devices from the share's audience and keeps them out of later
   announcements until the owner readmits them. The owner learns who left and
   stops encrypting for them.

## Decision

Option 3.

- **Leaving.** `leave { ws }` comes from any device in the audience other than
  the owner. The relay removes every device of that person (`Member.person` or the
  device itself) from the stored audience, adds the person to the entry's `left`
  list, detaches them, sends `unshare` and drops the share's inbox items, exactly
  as when the owner narrows the audience. The owner does not need to be online.
- **Staying out.** While a person is in `left`, the relay filters their devices
  out of every `share` announcement. An owner that was offline re-announces its
  stored audience on reconnection, and that stale audience must not bring the
  person back.
- **The owner learns.** `Shared.left` reaches the owner with its own share. The
  owner's channel then leaves those people out of the recipients it encrypts for,
  and the owner discards their watch, input and snapshots. The board audience is
  not rewritten: an organization-wide share stays organization-wide, and people
  who join later still receive it.
- **Readmission.** Only the owner sends `readmit { ws, person }`. The relay drops
  the person from `left`; the next announcement brings them back. The share menu
  marks people who left; picking one readmits them, and an explicit audience
  gains them if the owner had removed them in the meantime. For the owner's own
  person, turning remote control on again readmits the owner's devices.
- **Lifetime.** `left` lives in the relay entry. Stopping the share deletes the
  entry and the list: sharing again is a new decision and includes everyone it
  names. The roster drops people who left the organization from `left`, keeping
  it within the 64-member limit.

## Consequences

- A viewer can always clean up "Shared", even from a share whose owner will never
  return. Leaving one's own share from another device of the same person removes
  it from all of that person's companions.
- Trust does not change. A malicious relay could already withhold content from
  anyone; a forged `left` only makes the owner stop encrypting for someone, and
  the relay cannot readmit anyone the owner removed. Content already received is
  not revocable.
- `left` and the two frames are additive in protocol v4. Older clients ignore
  `Shared.left` and an older relay answers `leave` with a `bad` error, so the
  Worker ships before the desktop, as usual.
- The mobile shell does not offer leaving yet.

## Evidence

- `relay/src/logic.test.ts` ("leaving"): audience, detachment, inbox, stale
  announcements, readmission, companion devices, refusal and persistence.
- `relay/src/protocol.test.ts`: parsing of `leave`, `readmit` and `Shared.left`.
- `src/team-channel.test.ts`: the owner stops encrypting for a person who left
  and resumes after readmitting them, through the real relay reducer.
- `src/team-owner.test.ts`: input from a person who left is discarded before its
  receipt, and readmission restores it or widens an explicit audience.
