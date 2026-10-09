# ADR 0041 — The members list shows people, not devices

Date: 2026-09-10
Status: Accepted

Defines the Settings presentation of the device model in
[ADR 0027](0027-companion-devices.md).

## Context

The Organization tab listed one chip per roster member and a "Security code"
button next to each one. Since companions are members, the same person showed
up three or four times — "(iPhone)", "(MacBook Air)", "(you)" — and the
interleaved buttons broke the list's alignment. Whoever reads the screen wants
to know who is in the organization and who is available now, not the inventory
of each person's devices.

Permanent security-code controls also added a second task to a list intended
for presence. The current trust policy adopts peer key changes automatically,
as defined in [ADR 0042](0042-automatic-key-rotation.md).

## Options considered

1. Keep one chip per device and only fix the layout.
2. Group by person and keep the per-device security code in an expandable
   detail.
3. Group by person and remove the permanent security code.

## Decision

Option 3. The list uses `people()`: one chip per person, with companions only
lending their online state — a person is online when at least one of their
devices is connected. The per-member code button goes away, along with the
comparison dialog and the `code()` method of `TeamSecurity`.

There is no changed-key review. Key persistence and automatic adoption remain
per device under ADR 0042, independently of the person shown in the list. Input
from a changed or new device waits for the owner's answer above the
conversations of the workspace where it arrived, never in this list, and that
notice also names the person ([ADR 0090](0090-approved-remote-input.md)).

## Consequences

The Organization tab now answers "who is in the organization and who is
online". The voluntary fingerprint check before the first contact disappears;
there is no manual key-comparison interface.

Grouping the list does not change a persisted format, IPC, the relay protocol
or the encrypted channel's trust boundary: the roster and keys remain per device.
A format compatibility test therefore does not apply.

## Evidence

- [Collaboration core](../../src/team-channel.test.ts): companions fold into the
  person for the audience, mentions and box recipients.
- [Settings](../../src/settings.ts): one chip per person, with aggregated
  online state.
- [Channel security](../../src/team-security.test.ts): a new peer key is usable
  only after automatic persistence; replacing the own identity still fails.
