---
type: Decision
title: "ADR 0028 — Prometeu on the phone as a web app served by the Cloud"
description: "Prometeu on the phone as a web app served by the Cloud."
status: stable
---

# ADR 0028 — Prometeu on the phone as a web app served by the Cloud

Date: 2026-09-08

Uses the core from [ADR 0026](0026-portable-collaboration-core.md) and companion
devices from [ADR 0027](0027-companion-devices.md).

## Context

The collaboration core already composes without Tauri and the Cloud already
issues companion tickets. The client was missing: a page on the phone that
enters the organization's room, follows shared conversations, talks to the
owner's Mac and comments. The session still runs only on the Mac.

## Options considered

1. **A native app (iOS/Android).** Stores, signing and a second codebase for an
   interface that reads and replies.
2. **A bundle built inside Rails.** It would require Node and the Prometeu
   checkout in the Cloud's build; the Design System already avoids that by
   vendoring assets.
3. **A bundle built in Prometeu and vendored in the Cloud.** The same pattern as
   the Design System: `npm run build:mobile` generates `dist-mobile/`,
   `bin/mobile` copies it to `vendor/mobile/assets` with a hash manifest.

## Decision

Option 3. `src/mobile/` is the browser's composition root:

- `shell.ts`: pure ports. A companion ID generated once per browser, a
  `SecurityStore` over `localStorage`, a `Membership` with the same encryption
  scope as the desktop (`["organization", Cloud origin, organization]`) and a
  `url()` that requests the ticket at `POST /orgs/:slug/companion-ticket` with
  CSRF and builds the same room URL as `src-tauri/src/cloud.rs`.
- `main.ts`: registers `team-comments` and `team-viewer` on the member; no
  owner.
- `view.ts`: the list of shared conversations, the transcript through the
  `timeline.ts` reducer, a composer that sends `write` to the owner, comments
  and inbox. It reuses `markdown.ts`, `chat-presentation.ts` and the Design
  System.

The Cloud serves `GET /app` with its own layout for a browser session, renders
the canonical origin, the user and the memberships on the root element, and
allows the relay in `connect-src`. `GET /app/manifest` makes the page
installable. Login redirects back to `/app`.

This bundle does not create conversations, does not run Git and does not answer
permission cards; the Mac is still required for that. Remote input reaches the
agent with the original text when the authenticated sender belongs to the
owner's person. Other people's messages receive the team's authorship prefix.
See [ADR 0034](0034-mobile-pairing-continuity.md).

## Consequences

- Two copies of the same code in the Cloud (the vendored bundle) and in Prometeu
  (the source). `bin/mobile --check` flags a divergence; the version comes from
  `package.json`.
- The private identity stays in `localStorage` as a JWK, the same scheme as
  `team-security.json`. Clearing the browser's storage creates another device.
  Hardening it with a non-extractable `CryptoKey` in IndexedDB is a separate
  step, noted in the code.
- The phone sees workspaces through the team audience or the owner's explicit
  [remote control](0030-remote-control.md) permission, with the Mac awake, the
  app open and the Cloud and relay available.
- `src/mobile/*` does not import Tauri, IPC, the desktop shell or `chat.ts`;
  `npm run architecture:check` protects that boundary.
- Extraction into `packages/team-core` stays deferred: the bundle imports the
  files from `src/` directly and the Cloud only receives the artifact.

## Evidence

- `src/mobile/shell.test.ts`: stable identity, a room URL equal to the
  desktop's, a ticket with CSRF, organization choice.
- `src/team-member.test.ts`: composition without Tauri over the simulated relay.
- `prometeu-cloud/test/integration/mobile_test.rb`: mandatory login, embedded
  memberships, CSP with the relay, manifest.
- `prometeu-cloud/test/browser/mobile.spec.js`: entry through the browser, the
  organization's list, companion registration, no script errors.
