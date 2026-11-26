---
type: Decision
title: "ADR 0028 — Prometeu on the phone as a web app served by the Cloud"
description: "Prometeu Cloud supplies a web app that lets a phone follow shared sessions."
status: stable
---

# ADR 0028 — Prometeu on the phone as a web app served by the Cloud

Date: 2026-09-08

This ADR uses the core from [ADR 0026](0026-portable-collaboration-core.md) and
the companion devices from [ADR 0027](0027-companion-devices.md).

## Context

The collaboration core operates without Tauri, and the Cloud issues companion
tickets. The missing part was the client. That client is a page on the phone
that goes into the room of the organization and follows shared conversations. It
also sends messages to the Mac of the owner and adds comments. The session
continues to operate only on the Mac.

## Options considered

1. **A native app (iOS/Android).** This option has stores, signatures and a
   second codebase for an interface that reads and replies.
2. **A bundle built in Rails.** The Cloud build must then have Node and a
   checkout of Prometeu. The Design System prevents this because the Cloud
   keeps a copy of the assets of the package.
3. **A bundle built in Prometeu and vendored in the Cloud.** This is the same
   pattern as the Design System. `npm run build:mobile` makes `dist-mobile/`.
   `bin/mobile` copies it to `vendor/mobile/assets` with a hash manifest.

## Decision

Use option 3. `src/mobile/` is the composition root of the browser:

- `shell.ts` has pure ports. It has a companion ID that the browser makes one
  time, a `SecurityStore` on `localStorage`, and a `Membership`. The
  membership has the same encryption scope as the desktop
  (`["organization", Cloud origin, organization]`). It also has a `url()`. This
  function requests the ticket at `POST /orgs/:slug/companion-ticket` with CSRF.
  It builds the same room URL as `src-tauri/src/cloud.rs`.
- `main.ts` adds `team-comments` and `team-viewer` to the member. It does not
  add an owner.
- `view.ts` shows the list of shared conversations and the transcript through
  the `timeline.ts` reducer. It has a composer that sends `write` to the owner,
  and it shows comments and the inbox. It uses `markdown.ts`,
  `chat-presentation.ts` and the Design System again.

The Cloud supplies `GET /app` with its own layout for a browser session. It
renders the canonical origin, the user and the memberships on the root element.
It lets the page connect to the relay in `connect-src`. With
`GET /app/manifest`, a person can install the page. After the login, the Cloud
sends the person back to `/app`.

This bundle does not make conversations, does not run Git and does not answer
permission cards. The Mac must do these operations. Remote input goes to the
agent with the initial text when the authenticated sender is a device of the
owner. The messages of other persons get the authorship prefix of the team. See
[ADR 0034](0034-mobile-pairing-continuity.md).

## Consequences

- The same code has two copies: the vendored bundle in the Cloud and the source
  in Prometeu. `bin/mobile --check` shows a difference between them. The version
  comes from `package.json`.
- The private identity stays in `localStorage` as a JWK. This is the same
  scheme as `team-security.json`. If a person removes the stored data of the
  browser, the browser becomes a different device. A `CryptoKey` in IndexedDB
  that nobody can export gives more protection. That is a different step, and
  the code has a note about it.
- The phone shows workspaces through the team audience or through the explicit
  [remote control](0030-remote-control.md) permission of the owner. The Mac must
  be awake, the app must be open, and the Cloud and the relay must be available.
- `src/mobile/*` does not import Tauri, IPC, the desktop shell or `chat.ts`.
  `npm run architecture:check` gives protection to that boundary.
- The move of the code into `packages/team-core` is for a time after this
  change. The bundle imports the files from `src/` directly, and the Cloud
  receives only the artifact.

## Evidence

- `src/mobile/shell.test.ts` has tests for a stable identity and a room URL
  that is the same as the URL of the desktop. It also has tests for a ticket
  with CSRF and the selection of an organization.
- `src/team-member.test.ts` has tests for a composition without Tauri on the
  simulated relay.
- `prometeu-cloud/test/integration/mobile_test.rb` has tests for the mandatory
  login, the embedded memberships, the CSP with the relay and the manifest.
- `prometeu-cloud/test/browser/mobile.spec.js` has tests for the entry through
  the browser, the list of the organization and the companion registration. It
  also makes sure that there are no script errors.
