---
type: Decision
title: "ADR 0054 — Opt-in local notifications with independent sound"
description: "Local notifications that the person turns on, with a sound setting that operates independently."
status: stable
---

# ADR 0054 — Opt-in local notifications with independent sound

Date: 2026-09-21

## Context

The person must have explicit control of the events and the presentation of
notifications. But the sounds that stopped the work of the person again and
again must not come back. These sounds were the cause for the removal of audio
signals. The
approved design keeps the visual style and the sound apart. Push notifications
on the phone are for a subsequent version.

## Decision

Give local notification settings. By default, notifications and sound are off.
Use the current execution tracker and its one-second window for the state to
become stable. These items do not change:

- the sequence of accepted input
- the rule that keeps background tasks isolated
- the rule that a view with focus records its items as seen
- the counts of workspaces and comments on the Dock

Only new live requests and applicable completed turns can show a notification.
Comments and a transcript replay do not make a sound or show a notification. A
turn that ends with an error can show a notification without assistant
activity. An interrupted turn or a correct turn without activity does not show
a notification.

Use the macOS UserNotifications API for the permission, the system banners and
the clicks. Its Objective-C bindings add to the current objc2 boundary. Do not
add a plugin for many platforms, because its desktop implementation cannot give
the necessary permission state and click routing. Use a local Tauri window for
the optional notch presentation. Use the standard macOS sounds for the audio,
which operates independently. All the text on the screen uses the current i18n catalogs and shared controls.

Save the settings under a new localStorage key with a version. Keep the
previous sound key, which has no effect. Current installations stay silent
until the person turns on the setting. A migration of the relay, the transcript
or the board is not necessary.

## Trade-offs

The newest notch notice replaces the previous notice. The Dock keeps the
activity that waits. There is no notification queue or new history store.
macOS controls the delivery of system banners. The custom overlay and the
sounds do not obey the Focus filter. It is necessary to examine the native behavior in the bundled app.
The browser tests are not sufficient. Push
infrastructure, audio uploads and settings for many devices stay out of scope.
At this time, the notch style is an island that stays on the screen. See
[ADR 0089](0089-notch-island.md). The
[notification contract](../contracts/notifications.md) gives the full payload
and the compatibility guarantees.

## Evidence

- [Eligibility tests](../../src/alert.test.ts).
- [Compatibility of settings and routing](../../src/notifications.test.ts).
- [Flows for settings and delivery](../../e2e/notifications.spec.ts).
- [Checks of the silent default](../../src/alert.test.ts).
- [Validation of the native payload](../../src-tauri/src/notifications.rs).
