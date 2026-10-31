---
type: Quality
title: "Product analytics coverage"
description: "Feature inventory, named interactions, observation boundaries and verification."
status: stable
---

# Product analytics coverage

Implemented vocabulary and source inventory; live delivery requires a
release with the key and explicit consent. Read the [tracking contract](../contracts/product-analytics.md)
for exact meanings and the [generated catalog](../contracts/product-analytics-catalog.json)
for control identities and closed event fields.

The product inventory groups the application into the following areas. Names of
customer-created workspaces, files, actions, models and resources are never
control identities. Repeated items share an authored action token.

| Feature/module | Feature-use evidence and interaction scope |
| --- | --- |
| Navigation/layout | `screen_viewed`; named navigation, back/forward, tabs and panel controls |
| Projects | `project_action_used`: register, create Git project, remove and configure; named project forms |
| Sidebar organization | Project group/order changes, workspace pins, manual read/unread actions; named groups/rows |
| Workspace lifecycle | Existing creation plus `workspace_action_used`: stage, rename, finish, archive, restore, remove, cleanup |
| Desk | `desk_layout_changed`, `conversation_opened`; panel controls and resize completion |
| Conversation tabs | Existing additional-tab creation, explicit tab/desk opening and Continue use; no turn-end event |
| Composer/context | New attachments and structured context; named send/stop/attachment controls; keyboard, paste/drop share attachment paths |
| Output/requests | Copied message/code, accepted question responses and tool/plan decisions from conversation or island; named detail, link and decision controls |
| Models/effort/favorites | Explicit selection/change events; static model picker and option identities |
| Provider accounts | Login start/cancel, selection, removal, quota opening; account data excluded |
| Usage/context tools | Summary/quota views; named compact/report/new-conversation controls; automatic usage updates excluded |
| File tree/Quick Open | Explicit successful file opening; named folder/file rows, palette, contextual actions |
| File viewing/editing | Successful file open/save; explicit find opening; format is a closed category |
| Git changes/history | Diff/comparison entry and completed mutations; named stage, diff, history and conflict controls |
| Review notes | Note persistence and local batch submission; named note/summary/send controls |
| Terminal/Run | New PTY spawn and script start/explicit stop; reattaching an existing terminal is excluded from spawn count |
| Browser preview | Opening, capture and inspected-context attachment; only Prometeu controls, not the inspected page's arbitrary content |
| Dictation | Recognition start and first final insertion per session; no transcription content |
| MCP setup | Explicit resource setup/auth/check/discovery and selection; configuration is not invocation |
| Plugins/skills | Install/save/update/remove/catalog actions and selection; plugin generation means job acceptance |
| Actions | Prompt expansion or agent launch, plus configuration changes; no custom names |
| Integration setup | Explicit Linear/GitHub/Cloud/TypeSafe setup actions; see contract for authentication boundaries |
| Issues/PRs | Claim, source/existing-workspace opening and launcher preparation; creation stays a separate workspace fact |
| Notification inbox | Explicit read/open/navigation actions; arrivals and background fetches excluded |
| Local alerts/island | Preference actions and owned island navigation/answers; delivery itself is not use |
| Sharing/remote input | Explicit desktop-shell sharing, rights and pending-input actions; no participant identifiers |
| Comments/mentions | Accepted comment/reply/resolve actions; named threads/mention options; no text |
| Cloud/organizations/devices | Explicit login/organization/companion settings; no automatic-reconnect count |
| Delegation MCP | Successful authorized tool calls, with internal/external category; these can be automation and retries |
| Preferences/updates/support | Preference changes, explicit update/release-note/feedback actions; recovery controls are named |
| Companion/Windows surfaces | Existing UI reuse; **no product transport** in these shells, so absence is not non-use |
| Resource inheritance/trust | Defaults/global/workspace selection and explicit project trust decisions; no provider invocation inference |
| Machine status/power | Opening machine/quota panels; named keep-awake controls; background sampling excluded |
| Local history | Visible panel exposure, explicit refresh/filter, export and clear; all history content stays local |
| Optional TypeSafe setup | Configuration and synthetic test only; the evaluator has no feature consumer |

## Interaction inventory

The Privacy toggle has an authored target and area like other controls. Consent
defaults to off and is enforced by the native delivery boundary; the preference
value is not uploaded as a feature-event property.

The generated catalog contains explicit static targets with their sources, and
separately lists composed metadata expressions such as a dialog's `.save` and
`.cancel` suffixes. This distinction avoids claiming that a source scan has
executed every control state. Shared controls accept optional metadata; desktop
consumers author the target and area. Changing a state-dependent action must
change its target accordingly.

The listener covers actionable controls. Disabled controls cannot activate and
are ignored until enabled. Hidden but enabled controls remain inspectable.
Content selections, arbitrary websites inside the preview, embedded PDF internals
and OS-owned menu actions are outside this capture boundary. Owned native menus
are instrumented; their callback cannot distinguish accelerator from mouse.

## Verification evidence

`npm run analytics:check` checks source-catalog freshness and uses Chromium over
real shared components to verify named fields, choices, nested menus, dialogs,
pickers and feedback. It exercises portal context, invalid metadata, unclicked
controls, and disabling/re-enabling controls. The test sends no PostHog traffic.

A browser-mock audit during implementation exercised 24 rendered states covering
Desk, workspace, all Settings pages, model/effort/tool menus, actions, launcher
pickers, resource menus/dialogs and the empty Desk. All actionable controls in
those snapshots had target and area. This is evidence for those states, not a
claim of exhaustive real-device or third-party content coverage.

Relevant permanent tests:

- [`src/analytics.test.ts`](../../src/analytics.test.ts): explicit metadata,
  nested control resolution, missing/invalid identities, screen changes and
  shared question/approval outcome classification.
- [`src/notification-window.test.ts`](../../src/notification-window.test.ts):
  island feature counts after accepted responses, failure/non-response exclusion
  and request-content privacy.
- [`src/analytics-events.test.ts`](../../src/analytics-events.test.ts) and
  [`analytics.rs`](../../src-tauri/src/analytics.rs): frontend/native vocabulary,
  closed fields, content exclusion, non-null identities, batch offsets and consent.
- [`src/analytics-settings.test.ts`](../../src/analytics-settings.test.ts) and
  [`e2e/audit-regressions.spec.ts`](../../e2e/audit-regressions.spec.ts): consent
  saves, errors, unsupported backends, control metadata and focus restoration;
  the first-run browser scenario also exercises real accept/decline buttons,
  Escape, pending-save dismissal guards, failed-save retries and reloads.
- [`src/actions.test.ts`](../../src/actions.test.ts),
  [`src/workspace-review.test.ts`](../../src/workspace-review.test.ts),
  [`src/voice.test.ts`](../../src/voice.test.ts): actual action acceptance,
  review persistence and final dictation insertion.
- [`src/typesafe-settings.test.ts`](../../src/typesafe-settings.test.ts),
  [`src/telemetry-settings.test.ts`](../../src/telemetry-settings.test.ts),
  [`src/update.test.ts`](../../src/update.test.ts),
  [`src/team-organizations.test.ts`](../../src/team-organizations.test.ts):
  successful/failed/cancelled configuration and shell actions.
- Existing browser scenarios use [`e2e/fixtures.ts`](../../e2e/fixtures.ts) to
  fail on missing analytics metadata when an app control is activated. This
  strengthens those journeys without adding new interactions. Native service
  tests continue to protect files, Git, workspace lifecycle and process behavior.

The full repository check covers docs, architecture, builds, IPC contracts,
web tests, browser scenarios, native tests, formatting and lint. Test success
does not establish release ingestion. Verify payloads, contexts and supported
shells in PostHog after release before interpreting adoption or non-use.
