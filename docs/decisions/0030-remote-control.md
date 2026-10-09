# ADR 0030 — remote control independent of sharing

Date: 2026-09-08
Status: Accepted

Extends [ADR 0027](0027-companion-devices.md) and
[ADR 0028](0028-mobile-web-app.md), without changing
[relay v4](../contracts/relay-v4.md).

## Context

The phone belongs to the same person who runs the workspace on the Mac, but the
sharing menu offers only other people or the whole organization. So reaching
one's own workspace from the phone required exposing it to someone else.

Personal access must also be revocable without changing who already received the
workspace through the team's collaboration.

## Options considered

1. Show the person themselves in the audience menu. It solves exclusive access,
   but mixes personal devices with collaboration and does not allow turning the
   phone off when the audience is the whole organization.
2. Create another kind of share in the relay. It separates the concepts, but
   duplicates storage, comments and streaming.
3. Persist a personal permission in the workspace and reuse the existing share.

## Decision

Adopt option 3. `Workspace.remote_control` records separate consent. The
conversation's footer shows **Controle remoto** in Portuguese and **Remote
control** in English when a Cloud organization is active.

The workspace still has a single announcement. Before encrypting, the owner adds
their companion devices to the recipients only when `remote_control` is active.
The team's audience stays independent. A workspace with remote control and no
team audience uses `audience: []` locally; in the outer frame, the explicit
audience contains only the authorized devices.

## Consequences

- Enabling remote control does not share the workspace with other people.
- Enabling remote control approves the input of the owner's devices that exist
  then; a device added or changed later waits for the owner's answer on the Mac
  ([ADR 0090](0090-approved-remote-input.md)).
- Disabling it revokes the personal devices without removing the team's
  audience.
- Old boards receive `remote_control: false` by default.
- The Mac still runs the agent and must stay with Prometeu open.
- The relay receives the same v4 frames and still does not know about the local
  permission.

## Evidence

- `src/team-channel.test.ts`: the owner's device does not receive the share
  before the permission and receives it afterwards.
- `src/team-organizations.test.ts`: independent persistence and boxes encrypted
  only for the owner and their device.
