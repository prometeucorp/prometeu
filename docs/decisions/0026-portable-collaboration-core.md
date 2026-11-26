---
type: Decision
title: "ADR 0026 — portable collaboration core and phone access through the relay"
description: "A portable collaboration core lets a phone get access to a session through the relay."
status: stable
---

# ADR 0026 — portable collaboration core and phone access through the relay

Date: 2026-09-08

The desktop and the [mobile web app](0028-mobile-web-app.md) use this core.
Companion identities use the added v4 field from
[ADR 0027](0027-companion-devices.md).
This ADR adds to [ADR 0021](0021-cloud-organizations.md) and
[ADR 0022](0022-end-to-end-encryption.md). It does not change
[relay v4](../contracts/relay-v4.md).

## Context

The product must let a person follow a conversation from the phone and send
small prompts to it. The session continues to operate only on the Mac of the
owner. The relay sends ciphertext to each approved member, and the encryption
uses WebCrypto. In theory, a browser on the phone is only one more relay member.

But `src/team.ts` had three different roles in one file:

- The configuration of the desktop: team.json, Cloud organizations and Tauri
  IPC.
- The owner role: it announces workspaces, streams the conversation and operates
  remote input in the local agent.
- The member role: identity, online status, the watch function, comments and
  the inbox.

A mobile client on that file starts with conditions for each platform in all
the steps of the flow.

The initial identity model also had a limit. The relay stored one key for each
membership, and the owner did not encrypt boxes for itself. A phone with the
same membership replaced the key of the Mac and did not receive content.

## Options considered

1. Screen share from the Mac (VNC/Tailscale). This option has no code and no
   product.
2. The Mac supplies the desktop interface through Tailscale, with IPC through
   WebSocket. This option gives a complete interface. But it must have a private
   network and a Mac that is not asleep. It has no stored comments and no peers.
3. A PWA from `prometeu-cloud` connects to the relay as a usual member. It uses
   a collaboration core that does not use Tauri.

## Decision

Use option 3. The desktop shell and the browser shell use the same portable
core.

**Split by role and by feature, not by platform.** `src/team-member.ts` owns the
connection that connects again and the identity handshake. It also owns the
encrypted queue and the member directory. Each capability is a module. The
module adds hooks to the member. `src/team-ports.ts` specifies the hooks:
`connecting`, `outgoing`, `frame`, `binary`, `closed` and `reset`. The modules
are these:

- `team-owner.ts` has the announcements, snapshots, live stream and remote
  input. Only the machine that operates agents installs it.
- `team-viewer.ts` has the remote shares, the attach operation, the mirror and
  `write`.
- `team-comments.ts` has the threads, the mentions and the inbox.

The member calls the hooks in the sequence of registration. The composition root
selects the features and that sequence. A new feature is a new file and one line
in each root that uses it. The member ignores a frame that no feature processes.
The `outgoing` gate lets the owner stop content during an audience
change. It is not necessary for the member to know about the audience.

**Ports instead of conditionals.** The shell gives the member a `Membership` and
a `SecurityStore`. The membership has scopes, `shareScope`, `legacy` and a
`url()` function that gets a ticket or a credential. The owner receives an
`OwnerHost` with the local actions.

`src/team.ts` becomes the shell of the desktop. It reads team.json and the
organizations, and it puts the Tauri ports in the core. It adds the three
features to the member and shows the facade that the interface uses. The facade
also translates between board IDs and relay IDs. No `team-*.ts` imports
`@tauri-apps`, `./ipc`, `./mock` or `./team`. If that changes,
`npm run architecture:check` shows an error.

**Source shared directly.** The core stays in `src/team-*.ts`, and the two
shells build in this repository. The Cloud keeps a copy of the mobile bundle.
Thus there is no `packages/team-core` package and no isolated build. The
`SecurityStore` of the browser uses a JWK in `localStorage`. [ADR
0028](0028-mobile-web-app.md) gives this method and its limits.

## Consequences

- The behavior of the desktop does not change. The current tests operate through
  the facade without changes and show no errors. There are two intentional
  differences:
  - If the app cannot load `team-security.json`, it does not try again. Only the
    Cloud path tried again before.
  - When a person disconnects intentionally, the app removes the announcement
    state, as it does when the network stops.
- The phone shows workspaces through the team audience or through explicit
  personal remote control. The Mac of the owner must be awake, the app must be
  open, and the Cloud and the relay must be available. The phone does not make
  conversations or run Git.
- Each shell that adds `team-comments.ts` gets comments and the inbox
  without more work.
- The sequence of registration is a contract. When the owner comes before the
  comments, `share` comes before `notes` after the welcome.

## Evidence

- `src/team-member.test.ts` has tests for a composition without Tauri. It has
  a member, a viewer and comments on the simulated relay. It follows a share and
  answers a mention.
- `src/team.test.ts` and `src/team-organizations.test.ts` show that the facade
  keeps the behavior of the desktop.
- `scripts/check-architecture.mjs` is the fitness function for the core
  boundary.
