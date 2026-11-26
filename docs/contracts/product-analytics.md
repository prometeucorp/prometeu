---
type: Contract
title: "Product analytics"
description: "The PostHog event vocabulary, the payload, the identity of the installation and the rules for new events."
status: stable
---

# Product analytics

Prometeu can record two types of product evidence: the activations of
identified controls and the explicit use of features. This function is off
until the person enables it in the startup prompt or in Settings / General /
Privacy. [ADR 0093](../decisions/0093-anonymous-product-analytics.md) records the
decision. Execution diagnostics stay in [local telemetry](telemetry.md), which
does not go out of the machine at any time.

## Tracking plan and ownership

- [`analytics::Event`](../../src-tauri/src/analytics.rs) is the native allowlist.
- [`AnalyticsEvent`](../../src/analytics.ts) and
  [`FeatureEvent`](../../src/analytics-events.ts) give the types of the events
  that the frontend sends.
- The [analytics catalog](product-analytics-catalog.json) is a file that a
  script makes. It lists the control identities that authors write in code and
  their source files. It also lists the composed metadata expressions and the
  closed fields of each feature event. Run
  `node scripts/analytics-catalog.mjs` after you change the vocabulary or the
  control metadata. CI makes sure that the catalog stays current.
- The [coverage map](../quality/product-analytics-coverage.md) connects product
  modules to evidence, boundaries and verification.

The product team owns the question and the interpretation. The owner of the
feature keeps the code that starts the event and its tests correct. Names give facts in the
lowercase `object_verb` form. Variations use closed properties. They do not use
event names that come from customer data. New fields must be enums or booleans.
Control identities are identifier tokens that authors write in code.

Do not send these items at any time:

- User content, filenames, paths, URLs or names.
- Record IDs or search queries.
- Commands, credentials or prompts.
- Output or error messages.

## Named interactions

`ui_clicked {control, target, area}` records the activation of an identified
Prometeu control. `target` and `area` are necessary tokens that are not empty.
A token has 1–80 ASCII letters, digits, dots, underscores or hyphens. Rust
rejects nulls, empty strings, sentences, paths and URLs.

`target` comes only from explicit `data-track`, `data-t-title` or `data-t`
metadata. The last two use the authored key itself. Prometeu does not look for a
key from the text of a translated label. There is no fallback to element IDs,
text on the screen, CSS paths, positions, `unknown` or generic control names.
Data rows that repeat share
one action identity, for example `rail.workspace`. Prometeu does not send their
names and IDs.

`area` comes from explicit `data-track-area` metadata on the control or on a
parent element. Shared menus, dialogs and pickers keep the area of the control
that opened them when Prometeu puts them out of its DOM tree. Screen hosts can
supply a more accurate area. If the action of a control changes with its state, the
control must have the applicable target. Containers use area metadata. They do
not use one click target for all their contents.

One listener monitors these controls:

- Buttons, links, tabs, menu items and options.
- Checkboxes, radios, selectors, fields and disclosures.
- Custom controls with an explicit mark.

A nested control resolves to the nearest control. The authored
`data-track-double-click` and `data-track-context-menu` metadata record those
different actions. A keyboard activation can make a DOM click. Owned callbacks
of the native menu also record their authored action key with
`area: native_menu`. These callbacks do not show the difference between a menu
click and its accelerator.

If the metadata is missing or incorrect, Prometeu does not make a null event or
an invented event. Prometeu does these steps:

1. It adds `data-track-missing` to the element.
2. It sends the local `analytics-unidentified` diagnostic.
3. In development, it shows a warning.

`unidentifiedControls` examines rendered controls before an activation too. It
does not examine a disabled control until the control becomes enabled. It
continues to examine hidden controls that are enabled. Coverage checks must show
and repair these missing items. If Prometeu discards unidentified events, this alone does not prove
complete coverage. These diagnostics do not go out of the machine at any time.
This listener does not monitor content in embedded websites or PDF viewers. It
also does not monitor menu actions that the operating system owns.

Shared UI components contain authored metadata, but they do not send analytics
events. They do not import the analytics service, IPC or app controllers.
Optional tracking arguments keep current consumers. Feature owners must supply
metadata for the desktop.

## Feature-use vocabulary

Feature events record the agreed action at its shared application path. They do
not use a second handler on its button. When the operation is shared, the event
includes the mouse, the keyboard, paste or drop, and accepted remote or MCP
paths.
These items are not feature use: render, background refresh, polling, token
streams and process completion.

The analytics catalog gives all property types. This table gives the meaning:

| Event(s) | What is recorded |
| --- | --- |
| `app_opened` | The start of the desktop process. It is not daily activity. |
| `analytics_consent_granted` | The person enables sharing from the startup prompt or Settings. If the person declines or disables it, Prometeu sends nothing. |
| `screen_viewed` | A change of the screen that shows. |
| `workspace_created` | A new workspace before its preparation, which includes delegation. `provider`, `is_worktree`. |
| `conversation_created` | A conversation tab after the first conversation of the workspace. `provider`. |
| `project_action_used` | A project that a person adds, a new local Git project, a removal, a change of sequence or group, or a new script configuration. |
| `workspace_action_used` | Rename, pin, finish, archive, return from the archive, remove or cleanup. The UI sends manual read and unread actions. Automatic notification updates are not included. |
| `desk_layout_changed` | A completed change of visibility, sequence or dimension. |
| `conversation_opened` | An explicit activation of a workspace tab, or a desk conversation that the desk starts to show. |
| `conversation_resumed` | Continue sends its command without an error. It is not model execution. |
| `attachment_added`, `context_attached` | New attachment files or structured draft context, which includes selected files. It is not an inspection of free text. |
| `output_copied` | A copy of a message or a code block without an error. |
| `request_answered` | An accepted response to a question from the conversation or the island, locally or through the relay. The answers are not included. |
| `approval_submitted` | An accepted "allow" or "deny" decision for a tool approval or a plan, from the conversation or the island. Prometeu records only the approval boolean. |
| `model_selection_changed`, `effort_changed`, `model_favorite_changed` | An explicit selection in the picker or a change of a favorite. It is not an automatic refresh of the catalog. |
| `usage_summary_viewed`, `quota_viewed`, `machine_panel_opened` | The person opens the panel. Its periodic updates are not included. |
| `file_opened` | The person opens a viewer explicitly after Prometeu reads the file without an error, by closed viewer type. A refresh or a selection of the active file is not included. |
| `file_search_used` | The person opens the search in the file viewer. |
| `file_operation_completed` | A save, a new file, a rename, a move, a move to the trash or a return from the trash, without an error. No path or contents. |
| `diff_viewed`, `git_comparison_opened` | An explicit entry into a diff view or into a view that compares. Status polling is not included. |
| `git_operation_completed` | A stage, unstage, commit, fetch, pull, push, publish, discard or conflict resolution without an error. |
| `review_note_created`, `review_notes_sent` | Prometeu stores a new review note, or records locally that a batch was sent. The send does not prove that the provider received it or that the review is complete. |
| `terminal_opened` | Prometeu spawns a new interactive terminal. A new connection to a current PTY is not included. |
| `project_script_started`, `project_script_stopped` | The start of a new Setup or Run process, or an explicit stop of a process that operates. Setup includes the automatic preparation of the workspace. |
| `preview_opened`, `browser_context_attached`, `preview_capture_created` | The person opens the preview from a different view, attaches context from the inspector or makes a capture. Prometeu sends no URL, HTML or image. |
| `dictation_started`, `dictation_text_inserted` | Recognition starts. Then the first insertion of a complete transcript in each dictation session. Interim text is not counted or sent. |
| `action_used` | A prompt action expands into the draft, or an agent action starts without an error. |
| `settings_section_viewed`, `settings_search_used` | An entry into General, Agents, Resources, Actions, Projects, Work and team or Analytics, or the start of a settings search that is not empty. Each redraw or keystroke is not included. |
| `preference_changed` | An explicit change to one of the permitted preference categories. |
| `resource_action_used` | A resource save, remove, install, update, copy or share, or an explicit operation of authentication, check, discovery or generation. A check can report a probe failure. Generation records that the service accepted the job. |
| `resource_selection_changed` | A change of resource defaults, of the global selection or of a workspace selection. Enabling does not prove that the resource operated. |
| `integration_action_used` | Integration preparation actions. GitHub connect must return a connected status. Cloud connect must have a completed login poll. |
| `provider_account_action_used` | A login start or cancellation, an account selection or a removal. A login start is not a completed authentication. |
| `action_configuration_changed` | A person saves or removes a command, a profile or an override configuration. |
| `issue_action_used` | A person claims an issue, opens its source, opens a current workspace or prepares the creation launcher. The preparation is not a new workspace. |
| `notification_action_used` | Explicit actions in the inbox or the island, read actions and navigation. Notification delivery is not included. |
| `collaboration_action_used` | Explicit actions about organizations, sharing, permissions, pending input and comments through the desktop shell. No participant identifiers. |
| `usage_history_action_used` | The history panel starts to show, an explicit refresh or filter, an export without an error, or a person deletes the local history. Prometeu does not upload history records. |
| `update_action_used`, `release_notes_viewed` | Explicit update actions, or the person opens the release notes. Prometeu records the intent to start again before the relaunch. If the app stops before the send, this event does not go out. |
| `feedback_action_used` | The person opens the feedback panel or the public issue form, or sends a private submission without an error. When GitHub opens, this does not prove that the person sent an issue. |
| `trust_decision_made` | An explicit trust decision about project resources. |
| `delegation_action_used` | Approved MCP operations without an error, with the internal or external client category. Retries count tool use. They do not count unique new agents. |

When a person uses Finish on a workspace, Prometeu also archives it. These are two
different facts. Do not add them together as two workspaces. A control
activation and its feature event are also two different items of evidence about
the same interaction. Do not add event counts across a workflow to estimate
persons or unique operations. The history `view` means that the panel
shows. Prometeu records it one time for each entry into Settings or change of
page. Refresh, filter, export and clear show explicit interaction.

The conversation-runtime experiment that was not released (`message_accepted`,
`turn_finished`, activity flags) is not part of this vocabulary. The current
local execution telemetry does not change.

## Measurement and interpretation

The unit is an installation that PostHog sees, not an identified person. To measure the
adoption of a feature, count the installations that use it. Count them in the
eligible active installations that opted in, on versions and platforms that
send the necessary facts. Monitor the recurrence across days or weeks and the
concentration of activity. An app can stay open for days. Thus process starts
are not a denominator for activity.

Availability, discovery, configuration and use are different. If a person
installs or enables a plugin, this does not prove that a provider used it. A
feature that persons use rarely can be important to a small group. If a feature
is not on a chart, the cause can be one of these:

- Missing instrumentation.
- Missing consent.
- Shells that do not send events.
- Lost deliveries.

The opt-in causes a selection bias. Before you remove a feature, you must have
coverage that a person examined, sufficient observation and qualitative context.

The PostHog project has one dashboard for installation, activity and retention.
It has a different dashboard for feature adoption. The two dashboards use only events with
`analytics_version = 2`. Thus the analysis does not include older events, but
PostHog does not delete them. The first installations that PostHog sees are not new
installations. A current installation can update or opt in after some time. This
contract has no download, installation-completed, accepted-message or
completed-turn event.

The shared PostHog activity action includes the feature vocabulary, plus
`ui_clicked` and `conversation_created`. It does not include
`delegation_action_used` and `project_script_started`. It also does not include
process starts, screen navigation and workspace creation, because delegation can
also make a workspace. Daily, weekly and monthly activity and retention use this
same definition of seen interaction. New starts of the process and claims of
completed work are not part of it.

The adoption inventory includes all 50 feature events and the two creation
events. It includes entries with no observations. Its window is always the
last 28 days. In this window, it counts these values:

- The feature use in active installations.
- The repetitions across a minimum of two days or weeks.
- The part of the occurrences that comes from the installation with the most
  activity.

If a denominator is zero, the percentage is null. A different table shows the
variants. It includes the seen combinations of operation and resource, and
activity from automation only. It is not the adoption denominator. SQL tables
keep their window explicitly when the date selector of the dashboard
changes.

On 2026-10-25, a query validation found no v2 events in PostHog. Thus the
inventory shows that it waits for ingestion. It does not show features that persons do not use.
The release and the consent must occur before a verification of live delivery.
A request for a historical deletion through the MCP persons endpoint found no
profiles. It did not put an event deletion in the queue. All the seen
identities had no properties. The reset of the analysis is not a reset of the
storage. Prometeu does not change clicks from before into new feature facts at any time.

## Payload, privacy and delivery

Prometeu sends events in HTTPS batches to `https://us.i.posthog.com/batch/`.
Each payload contains the permitted fields of the event and metadata that does
not change:
`analytics_version: 2`, the app version, the OS, the architecture,
`$lib: prometeu`, `$process_person_profile: false` and `$geoip_disable: true`.
Prometeu does not add the PostHog JS SDK, a session replay, a DOM upload or a
synchronization of local telemetry.

The identity is a random UUID in `<root>/analytics-id`. Prometeu makes it only
when the consent and the build key let Prometeu capture events. The identity is
pseudonymous. PostHog can connect the events from one installation, but Prometeu
sends no account identity or workspace identity. The PostHog US project must
discard the IP data of the client.

For new and current installations, the default is off. A current `analytics-id`
is not consent. The preference is a JSON boolean in
`<root>/analytics-enabled.json`. This file is private, and Prometeu writes it
atomically. If the value is missing or incorrect, or if Prometeu cannot read it, the
preference is off and the person must make a selection.

At startup, if there is no correct saved boolean, the desktop shows a prompt. The
person can share or decline. This includes upgrades. The two answers go through the same
command. A current `true` or `false` value stops the prompt without a migration
or a marker for the question. Escape keeps the preference without an answer, and
the desktop shows the prompt again at the next startup. Save errors stay in the dialog, and the
person can try again. The release notes wait until the privacy dialog closes.
An explicit save replaces the value. The person can change it after that in
Settings.

When the person enables sharing, Prometeu sends `analytics_consent_granted` as
the first event with the new consent. If the person saves `true` again or saves
`false`, Prometeu sends nothing.

The preference commands are these:

- `analytics_needs_consent` has no arguments. It returns a boolean that tells if
  there is no correct saved selection. Backends that do not have this function skip the
  prompt.
- `analytics_enabled` has no arguments and returns a boolean.
- `analytics_set_enabled` receives `{ enabled: boolean }`, stores it and returns
  the saved boolean. If the write has a failure, the command rejects with
  `err.analytics.storage`. The preference in memory does not change.

These preference commands add to the IPC and do not change other commands. The version-2 tracking payload obeys
the metadata rules above.

The backend examines the consent before it starts delivery or puts an event in
the queue. This includes the app startup and the events that native operations
send. Changes in Settings have an effect without a restart. After the batch
window, the delivery examines the consent again. It discards pending events from
before a disable, also if tracking is enabled again after that. Prometeu does
not send again the events that it discarded while the function was off.

Consent changes and the send of batches share a delivery gate. Prometeu validates a
prepared batch again in that gate. The gate stays closed until the HTTP request
is complete. A save waits on a background thread for a request that did not
complete. The 10-second HTTP timeout limits this wait. The capture does not
lock the delivery gate, and the UI continues to operate. After a disable is
complete, no batch from before can start. A disable does not erase the events that
PostHog received or the local installation ID. This preference does not change
local telemetry or its capture.

The top-level `offset` is the monotonic time in milliseconds from the capture to
the construction of the batch. PostHog subtracts it from the time when the
server receives the batch. This keeps the intervals in a batch, and PostHog does
not trust the client clock. The time on the network continues to change the
absolute time. This value is not a measurement of latency.

One thread collects events for five seconds after the first event. It uses a
request timeout of ten seconds. Prometeu discards deliveries with a failure and
the batches that wait at exit. There is no offline queue and no retry. Tracking
does not stop a feature or cause a failure of it at any time. Only builds that opted
in and that have a `PROMETEU_POSTHOG_KEY` that is not empty send events.

| Shell | Coverage |
| --- | --- |
| macOS and Linux desktop | Explicit consent and the ingestion key. |
| Browser mock | No upload. The preference stays in the mock localStorage. |
| Mobile companion | No transport for product events in this delivery. |

## Compatibility and verification

Version 2 keeps the meanings of the four current lifecycle and navigation
events. It adds feature events. It also makes explicit click metadata that is
not null necessary. The version-1 clicks from before stay in PostHog. These
include nulls and unclear targets that came from translations. Prometeu does
not write them again or fill them in. They are not directly equivalent to new
control series. Prometeu releases the native contracts and the frontend contracts
together. No stored board, transcript or relay format changes.

[Native tests](../../src-tauri/src/analytics.rs) reject null click identities,
events that are not in the allowlist and incorrect closed values. They also make sure
that payloads do not include content and that time offsets stay correct.
Shared [fixtures](../../src-tauri/tests/fixtures/product-analytics.json)
and [TypeScript parity tests](../../src/analytics-events.test.ts) use the
same event schemas. [Click tests](../../src/analytics.test.ts) examine explicit
metadata and diagnostics. [Browser DOM checks](../../scripts/check-analytics.mjs)
examine fields, selections, menus, dialogs, pickers and feedback. This includes
controls that no test clicked. The [shared E2E fixture](../../e2e/fixtures.ts)
rejects missing metadata on controls that current browser scenarios activate.
Feature tests examine actions with and without errors, the storage of reviews,
dictation and configuration flows. See the coverage map for the scope.

Native consent tests examine these conditions:

- Upgrades with the default off.
- Preferences that are incorrect or that Prometeu cannot read.
- Persistence across restarts.
- Write failures.
- The rejection of pending events across cycles of disable and enable.
- Consent grants that Prometeu reports only when sharing becomes enabled.

Dispatch tests examine a disable after the preparation of a batch. They also
examine the serialization with a send in progress and a capture that does not wait
for the network.

[`src/analytics-settings.test.ts`](../../src/analytics-settings.test.ts) examines
these items:

- The save and close sequence at the first start.
- The load and the change of the consent.
- Pending saves.
- The recovery from errors.
- Backends that do not have this function.

The first-run scenario in
[`e2e/audit-regressions.spec.ts`](../../e2e/audit-regressions.spec.ts) does these
steps:

1. It pushes Escape.
2. It clicks the two consent buttons in the browser.
3. It holds saves to examine disabled controls and blocked dismissal.
4. It tries again the saves that had a failure.
5. It examines the persistence across reloads.

This scenario examines the consent boundary through native dialog cancellation
and form submission in Chromium and WebKit. A call to mocked callbacks cannot
prove their connections. A different Settings scenario examines the focus after
saves with and without errors, in Chromium and WebKit. This includes a move to a
different control during a save. This scenario must have a replacement of the browser DOM
and native focus behavior on disabled inputs. The mocked unit tests of settings
cannot prove this behavior. The IPC parity test in
[`src-tauri/tests/mock.rs`](../../src-tauri/tests/mock.rs) examines the
registration of commands.

During development, run `npm run analytics:check` and the nearest feature tests.
For changes to more than one module, run `npm run check`. These tests make
sure that local contracts and the capture behavior are correct. They do not make
sure that the delivery to a live PostHog project is correct.
