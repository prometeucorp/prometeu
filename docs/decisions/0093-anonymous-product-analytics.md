---
type: Decision
title: "ADR 0093 — Product analytics in PostHog"
description: "Rust sends anonymous feature and click events to PostHog. Only i18n keys and static IDs identify them."
status: stable
---

# ADR 0093 — Product analytics in PostHog

Date: 2026-10-23

## Context

Product decisions must have evidence about the adoption of features and about
use that occurs again and again. Local [execution telemetry](0059-local-telemetry-foundation.md)
answers questions about operation and stays on the machine. A click that Prometeu
finds from translated labels can have a null target or identify the incorrect
feature. Also, a click cannot show that the related operation occurred.

The product must have named interactions and also feature-use events. Prometeu
must not collect customer content. The
[contract](../contracts/product-analytics.md) contains the tracking plan and the
delivery rules. The [coverage map](../quality/product-analytics-coverage.md)
gives a list of the modules.

## Options considered

1. **Upload local telemetry.** It contains execution data and local
   identities for private analysis. An upload makes its privacy boundary larger.
   Also, it does not show directly which UI features persons use.
2. **Send daily aggregate counters to our Worker.** This option gives less
   linkage to each person. But it cannot show recurrence and funnels for each
   installation.
3. **Use a minimal analytics service.** This option decreases the preparation
   work. But it limits the cohort and exploration functions that are
   necessary for feature decisions.
4. **Send a closed vocabulary from Rust to PostHog.** This option gives product
   analysis without one more service to operate. It does not show the webview
   DOM.

## Decision

Use option 4, with two categories of evidence:

- **Named control activations:** one `ui_clicked` vocabulary with the
  necessary `target` and `area` that a person wrote. Use explicit tracking
  metadata or explicit i18n key metadata. Do not find a key from translated
  text. Menus and dialogs send the initial area into their portals. A row that shows again and again
  identifies the action, not the item of the customer. Native-menu
  callbacks that Prometeu owns share the same model.
- **Feature use:** typed events that explicit application operations send.
  The button that starts the operation has no effect on these events.
  - The native use case records the changes that are correct.
  - The presentation records explicit open, select and copy actions where it
    owns those actions.
  - Where applicable, the events cover shared keyboard, paste and drop, remote
    and MCP paths.
  - Configuration and execution stay different facts.
  - No product event records the completion of a model turn. No product event
    uploads local execution telemetry.

Rust validates the event vocabulary and the closed fields before delivery.
TypeScript types and shared test fixtures keep the two sides equal. Control
identifiers have a restrictive token shape. These items are not permitted:
prompts, code, filenames, paths, URLs, user names, search text, commands,
output and error strings. Shared UI components contain metadata. But they do not
import app controllers, analytics or IPC.

A [generated source catalog](../contracts/product-analytics-catalog.json)
records these items:

- Control identities.
- Source locations.
- Composed expressions.
- Feature event schemas.

Runtime checks examine controls also before a click. Controls without an
identity make local diagnostics and no analytics event. Coverage checks must
repair these missing identities. They must not replace them with `unknown`. They must not
tell that the coverage is complete because the null bucket is empty. Disabled
controls are not eligible until a person activates them. The checks continue to
examine hidden controls that a person can use.

Events go out through HTTPS to the US PostHog project. There is no PostHog JS
SDK, no session replay, no heatmap and no DOM upload. A UUID in
`<root>/analytics-id` identifies an installation. Accounts and local telemetry IDs have no
effect on this UUID. The project disables person profiles and geolocation.
The project must discard client IP data. This data is pseudonymous metadata,
not a dataset of identified users.

The tracking sends events and does not wait for a result. A failure of the tracking
cannot cause a failure of a feature. One thread collects batches of five seconds. Prometeu
discards requests that have a failure and batches that wait at shutdown. There is no
retry and no offline queue. Only opted-in builds with a `PROMETEU_POSTHOG_KEY`
that is not empty send events. In this delivery, mobile and the browser mock do
not have a product-event transport.

Version 2 adds these items:

- Explicit feature events.
- The constant value `analytics_version: 2`.
- A monotonic top-level `offset`. With it, PostHog keeps the time between
  events in a batch, and it does not use the client clock.

The time that the network uses continues to change the absolute time. The current lifecycle and
navigation meanings stay. Click series from earlier versions with null or inferred identities are
historical data. Prometeu does not add them to the new catalog.

**Consent.** The tracking operates only after an explicit opt-in.

- At startup, the desktop shows a question if there is no correct saved selection.
  This rule includes upgrades.
- The two answers, share and decline, keep the current boolean preference.
- Prometeu obeys the current selections.
- Escape moves the question to the next startup.
- When a save has a failure, the dialog stays open. Then the person can try
  again.
- After this, the person can change the selection in Settings / General /
  Privacy.

The question shows the selection to the person, and Prometeu does not enable
external tracking without a message. New and current installations start with
the tracking off. An installation ID from an earlier time is not consent.

Rust keeps the selection. It obeys the selection before it puts an event in the queue
and before delivery. When the person disables the tracking, Prometeu also
discards the events that wait, for all subsequent opt-ins. Prometeu cannot get
back HTTP requests that started before. A delivery gate puts the events that Prometeu sends
and the consent changes in sequence until the HTTP request completes. Thus a save can
wait until the request timeout on a background thread. That gate has no
effect on the event capture. With this decision, the save does not complete
immediately. As a result, a batch that is not current cannot start after the
person disables the tracking. This decision has no effect on the local telemetry
store and on the rules for its deletion.

## Consequences

The team can see the difference between a person who finds or activates a
control and a person who uses its feature. When actions or surfaces change,
stable identities must have explicit maintenance. The catalog checks become a
part of the contribution validation. The source catalog does not show that the
tests examined each dynamic state. Runtime tests and feature tests stay
necessary.

A high granularity has a cost. A new event must have these items:

- A product question and an accurate cause.
- The permitted properties and the eligible audience.
- Coverage and verification.

Use properties for the variants of one operation. Do not record each poll, each
keystroke, each pixel of a resize, or streams. Do not add clicks and feature
events together as unique operations. Do not count app starts as daily
activity.

A low use that PostHog records is not sufficient to remove a feature. Examine
availability, discovery, recurrence, use by a small group, shells that Prometeu
does not support, and delivery loss. Then collect qualitative context. The
PostHog dashboards show the installation, activity and retention that they
record in a different area from feature adoption. They filter for version 2. Activity uses
explicit interaction, not process starts.

When the team prepares and validates these queries, this does not show live
ingestion. Before the team finds a result about adoption, the events
must be in a release, and the team must make sure that the delivery operates. When the queries do not include the vocabulary from
earlier versions, PostHog does not
delete historical events. See the tracking contract for the dashboard
definitions and the verification boundary.

The opt-in decreases the coverage and adds a selection bias. But it gives
persons control of external product tracking. The preference is in the backend.
Thus startup and native use cases obey the same selection before the webview
loads. If Prometeu cannot read the preference, Prometeu disables the tracking.
When a save has a failure, Prometeu keeps the previous selection in operation and shows an error.
It does not tell that the save was correct.

## Evidence

- [Typed native vocabulary and payload tests](../../src-tauri/src/analytics.rs),
  [feature schema parity](../../src/analytics-events.test.ts), and
  [shared fixtures](../../src-tauri/tests/fixtures/product-analytics.json).
- [`analytics.rs`](../../src-tauri/src/analytics.rs) tests the persistence of
  consent and the invalidation of batches that wait.
  [`analytics-settings.test.ts`](../../src/analytics-settings.test.ts) tests
  the Settings saves and the recovery from a failure.
- [Click capture and diagnostics](../../src/analytics.ts),
  [capture tests](../../src/analytics.test.ts), and
  [checks on the DOM and portals of the app](../../scripts/check-analytics.mjs).
- [Catalog generator](../../scripts/analytics-catalog.mjs) and its
  [test for extraction and stale data](../../scripts/analytics-catalog.test.mjs).
- The coverage map lists the feature tests and the observation boundaries.
