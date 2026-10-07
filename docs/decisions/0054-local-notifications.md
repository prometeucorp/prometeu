# ADR 0054 — Opt-in local notifications with independent sound

Date: 2026-09-21
Status: Accepted

## Context

Notifications need explicit control over events and presentation without
restoring the repeated sound interruptions that led to removing audio alerts.
The approved design separates visual style from sound. Phone push is deferred.

## Decision

Offer local notification preferences with notifications and sound disabled by
default. Reuse the existing execution tracker and its one-second settling
window. Keep accepted-input ordering, background-task isolation, focused
visibility consumption, and Dock workspace/comment counting unchanged.
Only new live requests and eligible completed turns may notify; comments and
transcript replay remain silent. Errors may complete without assistant activity;
interruptions and successful turns without activity do not.

Use the macOS UserNotifications API for permission, system banners and clicks.
Its Objective-C bindings extend the existing objc2 boundary instead of adding a
cross-platform plugin whose desktop implementation cannot provide the required
permission state and click routing. Use a local Tauri window for the optional
notch presentation and fixed macOS sounds for independent audio. All visible
text uses the existing i18n catalogs and shared controls.

Persist preferences under a new versioned localStorage key. Preserve the inert
legacy sound key. Existing installations remain silent until explicit opt-in.
No relay, transcript or board migration is needed.

## Trade-offs

The latest notch notice replaces the previous one; the Dock retains pending
activity. There is no notification queue or new history store. macOS controls
system banner delivery; the custom overlay and sounds bypass Focus filtering.
Native behavior needs bundled-app verification in addition to browser tests.
Push infrastructure, audio uploads and cross-device preferences remain out of
scope. The notch style is now a persistent island; see
[ADR 0089](0089-notch-island.md). The complete payload and compatibility
guarantees are in the [notification contract](../contracts/notifications.md).

## Evidence

- [Eligibility tests](../../src/alert.test.ts).
- [Preference compatibility and routing](../../src/notifications.test.ts).
- [Settings and delivery flows](../../e2e/notifications.spec.ts).
- [Silent default checks](../../src/alert.test.ts).
- [Native payload validation](../../src-tauri/src/notifications.rs).
