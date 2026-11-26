---
type: Decision
title: "ADR 0030 — remote control independent of sharing"
description: "Remote control is a personal permission and it is not a share with other persons."
status: stable
---

# ADR 0030 — remote control independent of sharing

Date: 2026-09-08

This ADR adds to [ADR 0027](0027-companion-devices.md) and
[ADR 0028](0028-mobile-web-app.md). It does not change
[relay v4](../contracts/relay-v4.md).

## Context

The phone belongs to the person who operates the workspace on the Mac. But the
share menu shows only other persons or the full organization. Thus, to get
access to the workspace from the phone, the person had to share it with a
different person.

The person must also be able to remove the personal access. This removal must
not change the persons who got the workspace through the collaboration of the
team.

## Options considered

1. Show the person in the audience menu. This gives exclusive access. But it
   mixes personal devices with collaboration. Also, the person cannot disable
   the phone when the audience is the full organization.
2. Make a different type of share in the relay. This keeps the concepts
   apart. But it makes a second copy of the storage, the comments and the
   stream.
3. Keep a personal permission in the workspace and use the current share again.

## Decision

Use option 3. `Workspace.remote_control` records a consent apart from the
audience. When a
Cloud organization is active, the footer of the conversation shows **Controle
remoto** in Portuguese and **Remote control** in English.

The workspace continues to have one announcement. Before the owner encrypts it,
the owner adds the owner's companion devices to the recipients. The owner does
this only when `remote_control` is active. The audience of the team does not
change. A workspace with remote control and no team audience uses
`audience: []` locally. In the outer frame, the explicit audience contains only
the approved devices.

## Consequences

- When the owner enables remote control, the application does not share the
  workspace with other persons.
- When the owner enables remote control, the application gives approval to the input
  of the current devices of the owner. A device that the owner adds or changes
  after that time waits for the answer of the owner on the Mac
  ([ADR 0090](0090-approved-remote-input.md)).
- Remote control gives the devices of the owner the three rights of ADR 0090:
  view and comment, send messages and control. The owner gives the last two
  rights to each teammate separately. Many teammates can have the same right at
  the same time.
- When the owner disables remote control, the application removes the personal
  devices. The audience of the team does not change.
- Boards from a previous version get `remote_control: false` as the default value.
- The Mac continues to operate the agent. Prometeu must stay open on the Mac.
- The relay gets the same v4 frames. The relay continues to have no data about
  the local permission.

## Evidence

- `src/team-channel.test.ts`: the device of the owner does not get the share
  before the permission, and it gets the share after the permission.
- `src/team-organizations.test.ts`: the application keeps the permission
  independently and encrypts the boxes only for the owner and the device of the
  owner.
