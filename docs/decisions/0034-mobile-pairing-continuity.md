---
type: Decision
title: "ADR 0034 — Mobile pairing continuity"
description: "The relay keeps an approved socket open, and the phone shows the messages of its owner as messages of the owner."
status: stable
---

# ADR 0034 — Mobile pairing continuity

Date: 2026-09-10

This ADR gives the renewal for [ADR 0021](0021-cloud-organizations.md). It also
gives the remote authorship for [ADR 0028](0028-mobile-web-app.md) for the
personal devices of [ADR 0030](0030-remote-control.md).

## Context

Leases of 60 seconds closed correct sockets again and again. Each minute, the
phone and the share got a disconnection, a handshake and new snapshots. Personal
pairing also showed the messages of the person as messages from peers. The
mobile form had a minimum height of 112px from a different style. The content of
command lines made the transcript wider.

## Decision

Keep the revocation window of 60 seconds, and renew the lease on the identified
socket:

- The relay sends `lease { expires_in }`.
- At half of the time limit, the client uses the ticket port again and sends
  `renew { ticket }`.
- The Cloud authorizes the socket again. This makes no new API necessary,
  and it puts no permanent credentials in the relay.
- Only the same membership in the same organization can renew a socket that is
  not expired.
- If the roster does not change, the relay sends no presence updates, announcements
  or snapshots.
- Clients and relays from a previous version continue to use the previous path.
- When the connection stops, the client continues to use backoff and
  snapshots.

The authenticated sender can be a device of the same person, with the approval
of the owner on that Mac ([ADR 0090](0090-approved-remote-input.md)). Then the
Mac sends the initial text. The prefix of the team continues to
identify peers. Encryption, the consent for remote control and the protection
against replay stay mandatory.

The mobile shell limits the width of the columns of the transcript. Code blocks
and tables scroll horizontally in their area. The form uses the shared 44px
controls, 16px text and a growth with a limit. The visual viewport keeps the send
button above the keyboard. The shell removes the draft only after the encrypted
message goes out through the socket. This does not tell that the Mac did the
operation.

## Consequences and verification

We do not make the authorization stay for more time to hide
disconnections. The renewal adds two controls to v4. It keeps the revocation
without a webhook. We publish the relay before the clients. Thus, we can enable
the change gradually and move the code back to a previous version without a
migration.

`relay/src/worker.integration.test.ts` examines the renewal, the watchers, the
expiry and the refusal of a ticket. `src/team-organizations.test.ts` examines
the authorship, the continuity and a change of organization.
`e2e/mobile.spec.ts` operates the real shell with an encrypted peer in Chromium
and WebKit. It uses widths of 320/390px, the keyboard and a send failure that
does not remove the draft.
