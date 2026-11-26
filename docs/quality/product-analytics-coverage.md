---
type: Quality
title: "Product analytics coverage"
description: "The list of features, the named interactions, the limits of observation and the checks."
status: stable
---

# Product analytics coverage

This page gives the vocabulary in the code and the list of sources. Live
delivery occurs only in a release with the key and with the explicit consent of
the person. For the accurate meanings, read the
[tracking contract](../contracts/product-analytics.md). For the control
identities and the closed event fields, read the
[generated catalog](../contracts/product-analytics-catalog.json).

The product list divides the application into the areas below. The names of
workspaces, files, actions, models and resources that customers make are not
control identities at any time. Items that occur many times use the same action
token, which the code author writes.

| Feature/module | Feature-use evidence and interaction scope |
| --- | --- |
| Navigation/layout | `screen_viewed`. Named navigation, back and forward, tab and panel controls. |
| Projects | `project_action_used`: register, make a Git project, remove and configure. Named project forms. |
| Sidebar organization | Changes to project groups and order, workspace pins, manual read and unread actions. Named groups and rows. |
| Workspace lifecycle | The current creation event and `workspace_action_used`: rename, finish, archive, restore, remove, cleanup. |
| Desk | `desk_layout_changed`, `conversation_opened`. Panel controls and the end of a resize. |
| Conversation tabs | The current events for an added tab, an explicit open of a tab or the desk, and Continue. No event at the end of a turn. |
| Composer/context | New attachments and structured context. Named send, stop and attachment controls. Keyboard, paste and drop use the same attachment paths. |
| Output/requests | Copied messages and code, accepted answers to questions and tool or plan decisions from the conversation or the island. Named detail, link and decision controls. |
| Models/effort/favorites | Explicit selection and change events. Static identities for the model picker and its options. |
| Provider accounts | Login start and cancel, selection, removal, quota open. No account data. |
| Usage/context tools | Summary and quota views. Named compact, report and new-conversation controls. No automatic usage updates. |
| File tree/Quick Open | An explicit file open that is satisfactory. Named folder and file rows, palette, contextual actions. |
| File viewing/editing | A file open or save that is satisfactory. An explicit open of the find bar. The format is a closed category. |
| Git changes/history | The start of a diff or comparison and completed changes. Named stage, diff, history and conflict controls. |
| Review notes | The storage of notes and the local batch submission. Named note, summary and send controls. |
| Terminal/Run | A new PTY and the start or explicit stop of a script. A connection to a current terminal is not in the count of new PTYs. |
| Browser preview | Open, capture and the attachment of inspected context. Only Prometeu controls, not the content of the inspected page. |
| Dictation | The start of recognition and the first final insertion for each session. No transcription content. |
| MCP setup | Explicit resource setup, authentication, check, discovery and selection. Configuration is not a call. |
| Plugins/skills | Install, save, update, remove, catalog actions and selection. Plugin generation means that the job is accepted. |
| Actions | Prompt expansion or agent launch, and configuration changes. No custom names. |
| Integration setup | Explicit setup actions for Linear, GitHub, Cloud and TypeSafe. Refer to the contract for the authentication limits. |
| Issues/PRs | Claim, open from the source or from a current workspace, and launcher preparation. The creation of a workspace stays a different event. |
| Notification inbox | Explicit read, open and navigation actions. No arrivals and no background fetches. |
| Local alerts/island | Preference actions and the navigation and answers in the island. A delivery alone is not use. |
| Sharing/remote input | Explicit sharing actions in the desktop shell, rights and actions on pending input. No participant identifiers. |
| Comments/mentions | Accepted comment, reply and resolve actions. Named threads and mention options. No text. |
| Cloud/organizations/devices | Explicit login, organization and companion settings. No count of automatic new connections. |
| Delegation MCP | Satisfactory authorized tool calls, with an internal or external category. These calls can be automation or retries. |
| Preferences/updates/support | Preference changes, explicit update, release-note and feedback actions. The recovery controls have names. |
| Companion surfaces | They use the current interface again. These shells have **no product transport**. Thus, no events does not mean no use. |
| Resource inheritance/trust | Default, global and workspace selection, and explicit project trust decisions. No conclusion about provider calls. |
| Machine status/power | Open of the machine and quota panels. Named keep-awake controls. No background samples. |
| Local history | The panel comes into view, explicit refresh, filter, export and clear. All history content stays local. |
| Optional TypeSafe setup | Configuration and the synthetic test only. The evaluator has no feature consumer. |

## Interaction inventory

The Privacy switch has a target and an area that the code author writes, as
the other controls have. Consent is off by default. The native delivery limit
makes sure that the application obeys it. The application does not upload the preference value as a property
of a feature event.

The generated catalog contains explicit static targets with their sources. It
also has a different list of composed metadata expressions, for example the
`.save` and `.cancel` suffixes of a dialog. Thus, the catalog does not claim
that a source scan operated each control state. Shared controls accept optional
metadata. Desktop consumers write the target and the area. If you change an
action that changes with the state, you must also change its target.

The listener includes the controls that a person can operate. Disabled controls
cannot operate. The listener ignores them until they are enabled. Hidden
controls that are enabled stay available for inspection.

These items are out of the capture limit:

- Content selections.
- Websites in the preview.
- The internal parts of embedded PDF files.
- Menu actions that the operating system owns.

The native menus that Prometeu owns have instrumentation. Their callback cannot
tell the difference between a keyboard shortcut and the mouse.

## Verification evidence

`npm run analytics:check` makes sure that the source catalog is current. It uses
Chromium on the shared components of the product to validate these items:

- Named fields.
- Choices.
- Nested menus.
- Dialogs.
- Pickers.
- Feedback.

It also examines the portal context, metadata that is not correct, controls that
nobody clicked and controls that become disabled and enabled again. The test
does not send data to PostHog.

During the implementation, an audit with the browser mock examined 24 rendered
states. These states included the Desk, the workspace and all Settings pages. They
also included the model, effort and tool menus, the actions and the launcher
pickers. Also, they included the resource menus and dialogs and the empty Desk. All the controls that a person can operate
in those snapshots had a target and an area. This is evidence for those states
only. It does not claim full coverage of physical devices or of
third-party content.

These permanent tests apply:

- [`src/analytics.test.ts`](../../src/analytics.test.ts): explicit metadata,
  resolution of nested controls, identities that are missing or not correct,
  screen changes and the shared classification of question and approval
  results.
- [`src/notification-window.test.ts`](../../src/notification-window.test.ts):
  island feature counts after accepted answers. It does not include failures and
  requests without an answer, and keeps the content of requests private.
- [`src/analytics-events.test.ts`](../../src/analytics-events.test.ts) and
  [`analytics.rs`](../../src-tauri/src/analytics.rs): the frontend and native
  vocabulary, closed fields, content exclusion, identities that are not null,
  batch offsets and consent.
- [`src/analytics-settings.test.ts`](../../src/analytics-settings.test.ts) and
  [`e2e/audit-regressions.spec.ts`](../../e2e/audit-regressions.spec.ts): consent
  saves, errors, backends without support, control metadata and focus
  restoration. The browser scenario for the first run also examines the accept
  and decline buttons and Escape. It also examines the protection against a
  close during a pending save, retries after a save failure and reloads.
- [`src/actions.test.ts`](../../src/actions.test.ts),
  [`src/workspace-review.test.ts`](../../src/workspace-review.test.ts),
  [`src/voice.test.ts`](../../src/voice.test.ts): the acceptance of an action,
  the storage of a review and the final insertion of dictation.
- [`src/typesafe-settings.test.ts`](../../src/typesafe-settings.test.ts),
  [`src/telemetry-settings.test.ts`](../../src/telemetry-settings.test.ts),
  [`src/update.test.ts`](../../src/update.test.ts),
  [`src/team-organizations.test.ts`](../../src/team-organizations.test.ts):
  configuration and shell actions that are satisfactory, that have a failure or
  that the person cancels.
- The current browser scenarios use [`e2e/fixtures.ts`](../../e2e/fixtures.ts).
  A scenario has a failure if a person operates an app control that has no
  analytics metadata. This makes those journeys stronger without new interactions. The
  native service tests continue to examine files, Git, the workspace lifecycle
  and the process behavior.

The full repository check includes these items: documentation, architecture,
builds, IPC contracts, web tests, browser scenarios, native tests, format and
lint. Satisfactory tests do not show that PostHog gets the events from a release.
After a release, examine the payloads, the contexts and the shells with support
in PostHog. Do this before you decide about adoption or no use.
