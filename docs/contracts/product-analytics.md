---
type: Contract
title: "Product analytics"
description: "PostHog event vocabulary, payload, installation identity and rules for new events."
status: stable
---

# Product analytics

Prometeu optionally records two kinds of product evidence: identified control
activations and explicit feature use. Tracking is off until the person enables
it in the startup prompt or Settings / General / Privacy. The decision is in
[ADR 0093](../decisions/0093-anonymous-product-analytics.md). Execution diagnostics
remain in [local telemetry](telemetry.md), which never leaves the machine.

## Tracking plan and ownership

- [`analytics::Event`](../../src-tauri/src/analytics.rs) is the native allowlist.
- [`AnalyticsEvent`](../../src/analytics.ts) and
  [`FeatureEvent`](../../src/analytics-events.ts) type frontend emissions.
- The [generated catalog](product-analytics-catalog.json) lists authored control
  identities, their source files, composed metadata expressions and every
  feature event's closed fields. Run `node scripts/analytics-catalog.mjs` after
  changing the vocabulary or control metadata; CI checks that it stays current.
- The [coverage map](../quality/product-analytics-coverage.md) connects product
  modules to evidence, boundaries and verification.

Product owns the question and interpretation; the feature owner maintains the
trigger and its tests. Names describe facts in lowercase `object_verb` form.
Variations use closed properties, not customer-derived event names. New fields
must be enums or booleans; control identities are identifier tokens authored in
code. Never send user content, filenames, paths, URLs, names, record IDs, search
queries, commands, credentials, prompts, output or error messages.

## Named interactions

`ui_clicked {control, target, area}` records activation of an identified
Prometeu control. `target` and `area` are required nonempty tokens. The token
shape is 1–80 ASCII letters, digits, dots, underscores or hyphens. Rust rejects
nulls, empty strings, sentences, paths and URLs.

`target` comes only from explicit `data-track`, `data-t-title` or `data-t`
metadata. The latter two use the authored key itself; there is no reverse lookup
of translated labels and no fallback to element IDs, visible text, CSS paths,
positions, `unknown` or generic control names. Repeated data rows share an action
identity such as `rail.workspace`; their names and IDs are not sent.

`area` comes from explicit `data-track-area` metadata on the control or an
ancestor. Shared menus, dialogs and pickers preserve the opener's area when
mounting outside its DOM tree. Screen hosts can supply a more specific area.
Changing a control's action with its state requires the corresponding target.
Containers use area metadata, not a click target for their entire contents.

The single listener observes buttons, links, tabs, menu items, options,
checkboxes, radios, selectors, fields, disclosures and explicitly marked custom
controls. Nested controls resolve to the closest control. Authored
`data-track-double-click` and `data-track-context-menu` record those distinct
actions. Keyboard activation can generate a DOM click. Owned native-menu
callbacks also record their authored action key with `area: native_menu`;
these callbacks do not distinguish a menu click from its accelerator.

Missing or malformed metadata does not produce a null or fabricated event.
It marks the element with `data-track-missing`, dispatches the local
`analytics-unidentified` diagnostic and warns in development.
`unidentifiedControls` inspects rendered controls even before activation.
Disabled controls are ineligible until enabled; hidden enabled controls remain
inspectable. Coverage checks must expose and fix these gaps; dropping unidentified events
alone is not evidence of complete coverage. These diagnostics never leave the
machine. Arbitrary content inside embedded websites/PDF viewers and OS-owned
menu actions are outside this listener's boundary.

Shared UI components carry authored metadata without emitting analytics events. They do not
import the analytics service, IPC or app controllers. Optional tracking arguments
preserve existing consumers; feature owners must supply metadata for the desktop.

## Feature-use vocabulary

Feature events record the agreed action at its shared application path, not a
second handler on its button. Mouse, keyboard, paste/drop, and accepted remote
or MCP paths are covered where the operation is shared. Render, background refresh,
polling, token streams and process completion do not count as feature use.

The generated catalog defines all property types. This table defines meaning:

| Event(s) | What is recorded |
| --- | --- |
| `app_opened` | Desktop process start; not daily activity |
| `analytics_consent_granted` | Sharing turns on from the startup prompt or Settings; declining or disabling sends nothing |
| `screen_viewed` | A change of visible screen |
| `workspace_created` | Workspace creation before preparation, including delegation; `provider`, `is_worktree` |
| `conversation_created` | An additional conversation tab, excluding the workspace's first conversation; `provider` |
| `project_action_used` | New registration/addition, creation of a local Git project, removal, changed order/group or creation of script configuration |
| `workspace_action_used` | Changed stage, rename, pin, finish, archive, restore, remove or cleanup; manual read/unread actions are emitted by the UI, not automatic notification updates |
| `desk_layout_changed` | A completed visibility, order or size change |
| `conversation_opened` | Explicit workspace-tab activation or revealing a desk conversation |
| `conversation_resumed` | Continue sends its command successfully; not model execution |
| `attachment_added`, `context_attached` | Adding new attachment files or structured draft context, including selected file references; not free-text inspection |
| `output_copied` | Successful copying of a message or code block |
| `request_answered` | An accepted question response from the conversation or island, locally or through the relay; answers are excluded |
| `approval_submitted` | An accepted allow/deny decision for a tool approval or plan from the conversation or island; only the approval boolean is recorded |
| `model_selection_changed`, `effort_changed`, `model_favorite_changed` | Explicit picker selection or favorite change; not automatic catalog refresh |
| `usage_summary_viewed`, `quota_viewed`, `machine_panel_opened` | Opening the respective panel, not its periodic updates |
| `file_opened` | Explicit viewer opening after successful reading, by closed viewer kind; refreshing or selecting the already active file does not count |
| `file_search_used` | Opening search in the file viewer |
| `file_operation_completed` | A successful save, create, rename, move, trash or restore operation; no path or contents |
| `diff_viewed`, `git_comparison_opened` | Explicit entry into a diff or comparison view, not status polling |
| `git_operation_completed` | A successful stage, unstage, commit, fetch, pull, push, publish, discard or conflict resolution |
| `review_note_created`, `review_notes_sent` | Persisting a new review note or marking a sent batch submitted locally; sending does not prove provider receipt or completed review |
| `terminal_opened` | Spawning a new interactive terminal; reattaching an existing PTY is excluded |
| `project_script_started`, `project_script_stopped` | Starting a new Setup/Run process or explicitly stopping a live one; setup includes automatic workspace preparation |
| `preview_opened`, `browser_context_attached`, `preview_capture_created` | Opening preview from another view, attaching inspected context or creating a capture; no URL, HTML or image is sent |
| `dictation_started`, `dictation_text_inserted` | Recognition starts, then the first final transcript insertion per dictation session; interim text is not counted or transmitted |
| `action_used` | Expanding a prompt action into the draft or successfully starting an agent action |
| `settings_section_viewed`, `settings_search_used` | Entering a settings page or beginning a nonempty settings search; not every redraw or keystroke |
| `preference_changed` | An explicit change to one of the allowed preference categories |
| `resource_action_used` | Resource save/remove/install/update/copy/share or an explicit authentication/check/discovery/generation operation; a check may report a failed probe, and generation records job acceptance |
| `resource_selection_changed` | Changing resource defaults, global selection or a workspace selection; enabling does not prove resource execution |
| `integration_action_used` | Integration setup actions; GitHub connect requires returned connected status, and Cloud connect requires a completed login poll |
| `provider_account_action_used` | Login start/cancellation, account selection or removal; login start is not successful authentication |
| `action_configuration_changed` | Saving/removing a command, profile or override configuration |
| `issue_action_used` | Claiming an issue, opening its source, opening an existing workspace or preparing the creation launcher; preparation is not workspace creation |
| `notification_action_used` | Explicit inbox/island actions, reading and navigation; notification delivery is excluded |
| `collaboration_action_used` | Explicit organization, sharing, permissions, pending-input and comment actions through the desktop shell; no participant identifiers |
| `usage_history_action_used` | The history panel becoming visible, explicit refresh/filter, successful export or clearing local history; no history records are uploaded |
| `update_action_used`, `release_notes_viewed` | Explicit update actions or opening release notes; restart is an intent recorded before relaunch and may be lost at shutdown |
| `feedback_action_used` | Opening the feedback panel/public issue form or successful private submission; opening GitHub does not prove an issue was sent |
| `trust_decision_made` | An explicit project-resource trust decision |
| `delegation_action_used` | Successful authorized MCP operations, with internal/external client category; retries count tool use, not unique new agents |

Finishing a workspace also archives it; those are distinct facts, so do not sum
them as independent workspaces. A control activation and its feature event are
also separate evidence about the same interaction. Avoid summing event counts
across a workflow to estimate people or unique operations. History `view` is
visible-panel exposure, once per settings entry/page transition; refresh, filter,
export and clear distinguish explicit interaction.

The unshipped conversation-runtime experiment (`message_accepted`,
`turn_finished`, activity flags) is not part of this vocabulary. Existing local
execution telemetry remains unchanged.

## Measurement and interpretation

The unit is an observed installation, not an identified person. For feature
adoption, count installations using the feature among eligible active
opted-in installations on versions and platforms that emit the necessary facts. Follow
recurrence across days/weeks and concentration of activity. An app can stay open
for days; process starts are not an activity denominator.

Availability, discovery, configuration and use are different. Installing or
enabling a plugin does not prove a provider invoked it. A rarely used feature
may be important to a small group. Absence from a chart can reflect missing
instrumentation, missing consent, unsupported shells or delivery loss. Opt-in
introduces selection bias. Removing features needs
verified coverage, sufficient observation and qualitative context.

The PostHog project separates installation/activity/retention from feature
adoption. Both dashboards require `analytics_version = 2`; this excludes older
events from analysis without deleting them. First observed installations are
not new installations: an existing installation can update or opt in later.
There is no download, installation-completed, accepted-message or completed-turn
event in this contract.

The shared PostHog activity action includes the feature vocabulary except
`delegation_action_used` and `project_script_started`, plus `ui_clicked` and
`conversation_created`. It excludes process starts, screen navigation and
workspace creation, which can also come from delegation. Daily, weekly and
monthly activity and retention use this same observed-interaction definition,
not process reopens or a claim of completed work.

The adoption inventory includes all 50 feature events and the two creation
events, including entries with no observations. Its fixed trailing 28-day
window counts feature use among active installations, repeats across at least
two days or weeks, and the busiest installation's share of occurrences.
Zero denominators produce null percentages. The separate variant table includes
observed operation/resource combinations, including automation-only activity;
it is not the adoption denominator. SQL tables explicitly retain their fixed
window when the dashboard date selector changes.

On 2026-10-25, query validation found no ingested v2 events. The inventory
therefore reports awaiting ingestion, not unused features. Release and consent
must precede live delivery verification. A requested historical deletion through
the MCP persons endpoint found no profiles and queued no event deletion; all
observed identities were propertyless. The analytical reset is not a storage
reset, and old clicks are never manufactured into new feature facts.

## Payload, privacy and delivery

Events are sent in HTTPS batches to `https://us.i.posthog.com/batch/`. Every
payload contains the event's allowed fields and fixed metadata:
`analytics_version: 2`, app version, OS, architecture, `$lib: prometeu`,
`$process_person_profile: false`, and `$geoip_disable: true`. No PostHog JS SDK,
session replay, DOM upload or local telemetry synchronization is introduced.

Identity is a random UUID stored in `<root>/analytics-id`, created only when
consent and the build key allow capture. It is pseudonymous:
events from an installation can be linked, but no account or workspace identity
is sent. The PostHog US project must discard client IP data.

Tracking defaults to off for new and existing installations. An existing
`analytics-id` is not consent. The preference is a JSON boolean in the private,
atomically written `<root>/analytics-enabled.json`; missing, unreadable or
malformed values mean off and need a choice. On startup, the desktop asks to
share or decline when there is no valid saved boolean, including upgrades. Both
answers persist through the same command; existing `true` and `false` values
suppress the prompt without a migration or a separate asked marker. Escape
leaves the preference unanswered and asks again at the next startup. Save errors
stay in the dialog for retry. Release notes wait until the privacy dialog closes.
Explicitly saving a choice replaces the value, and Settings can change it later.
Turning sharing on emits `analytics_consent_granted` as the first event under
the new consent; saving `true` again or saving `false` emits nothing.

`analytics_needs_consent` takes no arguments and returns whether no valid saved
choice exists. Unsupported backends skip the prompt. `analytics_enabled` takes
no arguments and returns a boolean.
`analytics_set_enabled` takes `{ enabled: boolean }`, persists it and returns
the saved boolean. A write failure rejects with `err.analytics.storage` and
leaves the running preference unchanged. These preference commands are additive;
the version-2 tracking payload follows the required metadata rules above.

The backend checks consent before initializing delivery or queuing any event,
including app startup and events emitted by native use cases. Settings changes
take effect without restarting. Delivery rechecks consent after the batch
window, dropping pending events from before a disable even if tracking has
since been enabled again. Events dropped while off are never replayed.
Consent changes and dispatch share a delivery gate: a prepared batch is
revalidated under that gate, which remains held through HTTP completion.
Saving waits for an in-flight request (bounded by the 10-second HTTP timeout)
on a background thread; capture does not acquire the delivery gate and the UI
stays responsive. No stale batch can start after disabling succeeds.
Disabling does not erase events already received by PostHog or the local
installation ID. This preference does not change local telemetry or its capture.

The top-level `offset` is monotonic elapsed milliseconds from capture to batch
construction. PostHog subtracts it from server receipt time, preserving spacing
within a batch without trusting the client clock. Network delay still affects
absolute time; it is not a latency measurement.

One thread batches for five seconds after the first event and uses a ten-second
request timeout. Failed deliveries and batches waiting at exit are dropped.
There is no offline queue or retry. Tracking never blocks or fails a feature.
Only opted-in builds compiled with a nonempty `PROMETEU_POSTHOG_KEY` send events.

| Shell | Coverage |
| --- | --- |
| macOS and Linux desktop | Explicit consent and the ingestion key |
| Native Windows / WSL | Analytics commands unsupported; Settings disables the control |
| Browser mock | No upload; preference persists in mock localStorage |
| Mobile companion | No product-event transport in this delivery |

## Compatibility and verification

Version 2 preserves the four existing lifecycle/navigation event meanings,
adds feature events, and requires explicit non-null click metadata. Historic
version-1 clicks, including nulls and ambiguous translation-derived targets,
remain in PostHog; they are not rewritten, backfilled or directly equivalent to
new control series. Native and frontend contracts ship together; no persisted
board, transcript or relay format changes.

[Native tests](../../src-tauri/src/analytics.rs) reject null click identities,
unregistered events and invalid closed values, exclude content, and preserve
timing offsets. Shared [fixtures](../../src-tauri/tests/fixtures/product-analytics.json)
and [TypeScript parity tests](../../src/analytics-events.test.ts) exercise the
same event schemas. [Click tests](../../src/analytics.test.ts) cover explicit
metadata and diagnostics. [Real-DOM checks](../../scripts/check-analytics.mjs)
exercise fields, choices, menus, dialogs, pickers and feedback, including controls
that were never clicked. The [shared E2E fixture](../../e2e/fixtures.ts) rejects
missing metadata on controls activated by existing browser scenarios.
Feature tests cover successful/failed actions, review
persistence, dictation and configuration flows. See the coverage map for scope.

Native consent tests cover default-off upgrades, invalid/unreadable preferences,
persistence across restarts, failed writes, pending event rejection across
disable/enable cycles and consent grants reported only when sharing turns on. Dispatch tests cover disabling after batch preparation,
serialization with an in-flight send and capture without waiting for the network.
[`src/analytics-settings.test.ts`](../../src/analytics-settings.test.ts) covers
first-run save/close orchestration, loading and changing consent, pending saves,
error recovery and unsupported backends. The first-run scenario in
[`e2e/audit-regressions.spec.ts`](../../e2e/audit-regressions.spec.ts) presses Escape,
clicks both real consent buttons, holds saves to check disabled controls and blocked
dismissal, and retries failed saves before checking persistence across reloads.
This protects the consent boundary through native dialog cancellation and form
submission in Chromium and WebKit; invoking mocked callbacks cannot prove their
wiring. The separate Settings scenario covers focus
after successful and failed saves in Chromium and WebKit, including moving to
another control while saving. This needs real DOM replacement and native
disabled-input focus behavior, which the mocked settings unit tests cannot prove.
The IPC parity test in [`src-tauri/tests/mock.rs`](../../src-tauri/tests/mock.rs)
covers command registration.

Run `npm run analytics:check` and the closest feature tests during development;
run `npm run check` for changes spanning modules. These tests verify local
contracts and capture behavior, not successful delivery to a live PostHog project.
