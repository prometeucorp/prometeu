# ADR 0034 — Mobile pairing continuity

Date: 2026-09-10
Status: Accepted

Defines renewal for [ADR 0021](0021-cloud-organizations.md) and remote authorship for
[ADR 0028](0028-mobile-web-app.md) for the personal devices of
[ADR 0030](0030-remote-control.md).

## Context

60-second leases kept closing valid sockets. The phone and sharing went through
a disconnection, a handshake and new snapshots every minute. Personal pairing
also identified the user's own messages as messages from peers. The mobile form
inherited a 112px minimum height, and the intrinsic content of command lines
widened the transcript.

## Decision

Keep the 60-second revocation window and renew on the identified socket. The
relay announces `lease { expires_in }`; the client reuses the ticket port
halfway through the deadline and answers `renew { ticket }`. The Cloud
authorizes again, without a new API and without permanent credentials in the
relay. Only the same membership in the same organization can renew a socket that
is still valid. An unchanged roster produces no presence, announcements or
snapshots. Old clients and relays keep the previous path. A real connection loss
still uses backoff and snapshots.

The Mac forwards the original text when the authenticated sender is a device
belonging to the person themselves and approved on that Mac
([ADR 0090](0090-approved-remote-input.md)). Peers are still identified by the
team's prefix. Encryption, remote control consent and replay protection stay
mandatory.

The mobile shell limits the width of the transcript's columns and keeps
horizontal scrolling inside code blocks and tables. The form uses shared 44px
controls, 16px text and bounded growth. The visual viewport keeps the send
button above the keyboard. The draft is cleared only after the encrypted message
leaves through the socket; that does not represent confirmation of execution by
the Mac.

## Consequences and verification

We do not extend the authorization's validity to hide disconnections. Renewal
adds two controls to v4 and preserves revocation without a webhook. Publishing
the relay before the clients allows gradual activation and a code rollback
without migration.

`relay/src/worker.integration.test.ts` checks renewal, watchers, expiration and
ticket rejection. `src/team-organizations.test.ts` checks authorship,
continuity and organization switching. `e2e/mobile.spec.ts` exercises the real
shell with an encrypted peer in Chromium and WebKit, widths of 320/390px, the
keyboard and a send failure without losing the draft.
