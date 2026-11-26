---
type: Decision
title: "ADR 0041 — The members list shows people, not devices"
description: "The Organization tab shows one item for each person, not one item for each device."
status: stable
---

# ADR 0041 — The members list shows people, not devices

Date: 2026-09-10

This ADR specifies how Settings shows the device model of
[ADR 0027](0027-companion-devices.md).

## Context

The Organization tab showed one chip for each member of the roster, with a
"Security code" button adjacent to each chip. Companions are members. Thus, the
same person was in the list as three or four items: "(iPhone)", "(MacBook Air)",
"(you)". Also, because of the buttons between the chips, the items of the list did not align. The
reader of the screen must know who is in the organization and who is available
at this time. It is not necessary for the reader to see the devices of each person.

The permanent security code controls also added a second task to a list for
presence. The current trust policy accepts key changes of peers automatically,
as [ADR 0042](0042-automatic-key-rotation.md) specifies.

## Options considered

1. Keep one chip for each device and only correct the layout.
2. Group the list by person. Keep the security code of each device in an
   area that the person can expand.
3. Group the list by person and remove the permanent security code.

## Decision

Use option 3. The list uses `people()`, with one chip for each person.
Companions only give their online state. A person is online when one or more of
the devices of the person are connected. The code button of each member goes
away. The dialog that compares codes and the `code()` method of `TeamSecurity` also go
away.

There is no review of changed keys. ADR 0042 keeps keys and accepts them
automatically for each device, independently of the person in the list. Input
from a changed or new device waits for the answer of the owner. The application
shows this question above the conversations of the workspace that got the
input, not in this list. That notice also shows the name of the person
([ADR 0090](0090-approved-remote-input.md)).

## Consequences

After this change, the Organization tab answers "who is in the organization and who is
online". The application does not have the optional fingerprint check before the first
contact.
There is no interface to compare keys manually.

The groups in the list do not change a stored format, IPC, the relay protocol or
the trust boundary of the encrypted channel. The roster and the keys stay
specified for each device. Thus, a format compatibility test does not apply.

## Evidence

- [Collaboration core](../../src/team-channel.test.ts): companions go into the
  person for the audience, the mentions and the box recipients.
- [Settings](../../src/settings.ts): one chip for each person, with one online
  state from all the devices of the person.
- [Channel security](../../src/team-security.test.ts): the application can use a
  new peer key only after the application keeps it automatically. A
  replacement of the identity of the application continues to be a failure.
