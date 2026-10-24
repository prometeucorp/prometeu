# Anonymous product analytics

Prometeu sends anonymous feature events to PostHog so we can tell which features
are used. The decision and its trade-offs are in
[ADR 0093](../decisions/0093-anonymous-product-analytics.md). This is separate
from the [local telemetry](telemetry.md), which never leaves the machine.

## Vocabulary

`analytics::Event` in [`src-tauri/src/analytics.rs`](../../src-tauri/src/analytics.rs)
is the source of truth; [`src/analytics.ts`](../../src/analytics.ts) mirrors it.

This document is the tracking plan: every event that exists is in the table.
The trigger defines what an event proves; the measurement section below defines
the product questions it can answer. Coverage and test evidence follow.

| Event | Properties | Emitted when |
| --- | --- | --- |
| `app_opened` | none | The desktop app starts |
| `screen_viewed` | `screen` | The visible screen changes |
| `workspace_created` | `provider`, `is_worktree` | A workspace is created, before agent preparation, including MCP delegation |
| `conversation_created` | `provider` | Another conversation is opened in an existing workspace |
| `message_accepted` | `provider`, `is_action`, `is_delegated` | A `message.send` command succeeds at the common process boundary |
| `turn_finished` | `provider`, `outcome`, `is_action`, `is_delegated`, `has_agent_activity` | Each live canonical main-turn terminal observed after the first accepted input |
| `ui_clicked` | `control`, `target`, `area` | Any click on a button, link, menu item, tab, option, checkbox or select |

`provider` is `claude`, `codex`, `antigravity` or `gemini` (the retired value).
Unknown provider text deserializes as `claude`, so it never reaches the payload.
`screen` is `desk`, `settings`, `issues`, `notifications`, `archived`,
`workspace` or `project`; moving between workspaces is not a screen change.

### Conversation events

The desktop composition in [`chat/host.rs`](../../src-tauri/src/chat/host.rs)
observes successful commands and live canonical output through the existing
capture gate. Analytics has its own in-memory capture state; it does not read,
upload or depend on the local telemetry store or its deletion generation.
`provider` comes from the running process, not a later UI selection.

- `message_accepted` counts one successful command. It covers initial prompts,
  mouse and Enter sends, queued input after resume, actions, MCP delegation and
  remote input accepted by the owner. A failed write emits nothing. Queues can
  combine prompts into one command, so this is not a count of clicks, drafts or
  individual remote submissions. Acceptance does not prove model execution.
- `is_action` means the conversation has a task; `is_delegated` means it belongs
  to a delegation. Both are sampled from the board for the accepted command.
  They describe conversation context, not whether a human or an agent sent the
  latest message. No task, delegation, workspace or conversation ID is sent.
- `turn_finished.outcome` is `completed`, `failed` or `interrupted`, mapped from
  the canonical `turn.completed` outcome. A terminal before the first accepted
  input is ignored. Each later live canonical terminal is recorded, including
  successive terminals without intermediate assistant activity. Providers
  deduplicate their own native identities in their adapters; analytics adds no
  global deduplication heuristic based on input or activity.
- `assistant.started` and `assistant.block` mark `has_agent_activity`. The flag
  reports activity observed since the previous recorded terminal, or since
  accepted-input capture began. A new accepted command does not clear it.
  User echoes, usage events and other frames do not set it. It is an interval
  observation, not exact attribution to a turn or input.
- A local slash command can finish successfully without assistant activity.
  Filtering `has_agent_activity: true` excludes completions with no observed
  activity. Overlapping input/output can supply activity from another turn;
  the flag does not establish answer quality or useful output.
- These are main-turn terminals. Background children may still be running;
  the event does not mean the whole conversation has settled. Process exit,
  kill or crash without a canonical terminal has an unknown outcome and emits
  no synthetic completion. Transcript replay is not counted.

Accepted messages and observed turns are independent counts, not a one-to-one
pair. Do not divide `turn_finished` by `message_accepted` to report a success
rate. There is no operation correlation identifier in this payload.

## Naming

- Event names are `object_verb` in lowercase snake_case, with the verb in the
  past tense from a fixed list: `opened`, `viewed`, `created`, `clicked`,
  `archived`, `deleted`, `accepted`, `finished`. Add a verb to this list before
  using it.
- Names are static. A variation is a property (`ui_clicked` with a `target`),
  never a new name built from a value.
- Properties are snake_case nouns. Booleans start with `is_` or `has_`.
- A typed event exists only for an outcome a click cannot show: something was
  created, failed or finished, or happened without a click (shortcuts, MCP
  delegation). Interface usage is `ui_clicked`.
- Readable names for specific clicks are PostHog Actions over `ui_clicked`
  filtered by `target`, for example "Settings opened" for `top.settings`. They
  need no code change.

### Clicks

One capture-phase listener in [`src/analytics.ts`](../../src/analytics.ts)
records every click on a control, so new UI is covered without annotation. It
never reads visible text into the event:

- `target` is the control's `data-track` token when it has one, else the i18n
  key of the control's label. The label (`data-t` or
  `data-t-title`, then `aria-label`, `title`, menu label, text) is looked up in
  the active catalog; only an exact match of a parameterless translation
  produces a key. Otherwise `target` is the control's own ID if that ID was in
  `index.html` at startup, else `null`. Workspace names, file names, agent
  output and typed text never match, so they never become a target. Translations
  shared by several keys resolve to the first key in the catalog.
- `data-track` names controls whose label is user data or a parameterized
  translation, which would otherwise send `null`: rail workspaces, groups and
  agents (`rail.*`), file tree rows (`tree.folder`, `tree.file`) and center tabs
  (`tab.conversation`, `tab.file`, `tab.terminal`, `tab.browser`). An element
  with `data-track` counts as a control even when it is not one, so a close
  icon nested in a tab (`tab.close`, `tab.file.close`, …) is told apart from the
  tab itself. The token is a static string we author; never build it from data.
- `area` is `menu` inside a menu, otherwise the nearest ancestor ID present in
  `index.html` at startup (`deskView`, `settingsView`, `tabbar`, `rail` and so
  on), else `null`.
- `control` is one of `button`, `link`, `menuitem`, `tab`, `option`,
  `checkbox` or `input`.

Rust accepts `target` and `area` only as identifier tokens: 1 to 80 ASCII
letters, digits, `.`, `_` or `-`. Sentences, paths and URLs fail to deserialize
even if a caller passes text by mistake.

Generic click capture is unchanged in analytics version 2. `null` targets can
still occur, and translation collisions can assign an ambiguous key. Absence
from a click chart does not prove that a feature is unused: shortcuts, missing
identifiers, delivery loss and unsupported shells also affect coverage.

## Measurement plan

The unit is an observed installation, not an identified person. Compare only
versions and shells that emit the events needed for a question. Conversation
events first exist in `analytics_version: 2`; older installations are not a
valid denominator for those events.

| Question | Evidence | Interpretation limit |
| --- | --- | --- |
| Which installations start the app? | Unique installations with `app_opened` | Starting the process is not all activity; an app can remain open for days |
| Which installations submit conversation input and return? | Unique installations and distinct days/weeks with `message_accepted` | Includes local slash commands and combined queued input; context flags do not identify the sender |
| Which providers report main-turn outcomes? | `turn_finished` by `provider` and `outcome` | Only observed terminals; missing terminals are unknown, and input/output counts cannot form a success ratio |
| Do installations reach a completion after observed assistant activity? | `message_accepted`, then `turn_finished` with `outcome: completed` and `has_agent_activity: true` | A provisional activation proxy; activity belongs to an observation interval, not necessarily the same turn, input or conversation |
| Which screens and controls are visited? | `screen_viewed` and `ui_clicked` | Navigation and intent do not prove completion or feature adoption |

An initial activation funnel can start at the first observed `app_opened`,
then use the accepted-input and assistant-completion proxy above. First observed
opening is not proof of a new installation: existing users become observable
when they install a version with analytics. Funnels link events by installation,
so concurrent conversations can supply different steps. Do not present this
proxy as a correlated end-to-end conversation success rate.

The existing PostHog dashboard retains its original queries while the new
events await release and ingestion. Its titles and descriptions now distinguish
initializations, reopens, additional conversations and captured clicks. A
creation funnel ending in `conversation_created` does not measure first use:
that event excludes the workspace's first conversation, and
`workspace_created` precedes agent preparation.

Feature-removal decisions need verified instrumentation, an eligible active
denominator, recurrence over a sufficient observation period and inspection of
concentrated use. These events alone do not establish that a feature is unused.

## Payload

Every event is posted in a batch to `https://us.i.posthog.com/batch/`:

```json
{
  "api_key": "<PROMETEU_POSTHOG_KEY>",
  "batch": [
    {
      "event": "message_accepted",
      "distinct_id": "<random installation UUID>",
      "offset": 5125,
      "properties": {
        "provider": "codex",
        "is_action": false,
        "is_delegated": false,
        "$process_person_profile": false,
        "$geoip_disable": true,
        "$lib": "prometeu",
        "analytics_version": 2,
        "app_version": "0.33.0",
        "os": "macos",
        "arch": "aarch64"
      }
    }
  ]
}
```

No other properties are sent. `distinct_id` is a UUID v4 created at first use
and stored privately in `<root>/analytics-id`; an unreadable file is replaced
with a new one.

`offset` is the elapsed milliseconds from capture to batch construction,
measured with a monotonic clock. PostHog subtracts this value from its server
time ([timestamp rules](https://posthog.com/docs/data/timestamps)). This keeps
events in a batch separated by their observed timing without sending a client
wall-clock timestamp. Network and request-processing delay still affect the
absolute time; this is not a latency measurement.

### Compatibility

Version 2 adds the two conversation events, `analytics_version: 2` on every
event and the top-level `offset`. Existing event names and their properties,
including click capture, retain their meanings. Older history has no
`analytics_version` and remains readable; it is not backfilled or reclassified.
No persisted application format or conversation protocol changes. Payload and
capture tests protect the additive contract and legacy event acceptance.

## Rules for new events

- Clicks need no new event; follow the naming rules above before adding one.
- An i18n key or `data-track` token used as a click `target` is part of this contract. Renaming it
  starts a new series in PostHog; update the Actions that filter on it.
- Add the variant to the Rust enum and the TypeScript union, add a row to the
  table above, and emit it with a single `track` call at the use case. Document
  its product question, exact trigger, coverage, introduction version and tests.
- Fields must be closed enums, booleans or identifier tokens we author. Never
  add user content, record IDs, names, paths, titles, URLs, branch names, model
  output or error messages.
- Prefer emitting from the Rust use case every caller goes through.
- `track` is fire-and-forget. Do not await it or branch on its result.

## Delivery and configuration

- One background thread collects events for 5 seconds after the first one, then
  posts the batch with a 10-second timeout. Failures, and a batch still waiting
  when the app quits, are dropped; there is no offline queue.
- Only builds compiled with a non-empty `PROMETEU_POSTHOG_KEY` send events. Release CI reads
  it from the `PROMETEU_POSTHOG_KEY` repository variable; other builds send
  nothing.
- The PostHog project is in the US region; a key from another region is
  rejected silently. **Discard client IP data** must be enabled, because US
  projects record the request IP by default and the client cannot remove it.
- There is no opt-in or opt-out control in this version.

## Coverage

| Shell | Sends events |
| --- | --- |
| macOS and Linux desktop | Yes, release builds with the key |
| Native Windows (WSL runtime) | No; `analytics_track` is unsupported and ignored |
| Browser mock | No; the handler ignores events |
| Mobile shell | No |

## Verification

The tests in [`analytics.rs`](../../src-tauri/src/analytics.rs) check that the
payload contains only the event's fields plus the fixed metadata, that unknown
events are rejected, that extra or unknown values are dropped, and that click
tokens reject sentences, paths and URLs. They also check independent message
and turn counts, successive streaming terminals without intermediate activity, closed
outcomes/context, content exclusion and capture offsets. The existing
[`session/output.rs`](../../src-tauri/crates/core/src/session/output.rs) and
[`pump tests`](../../src-tauri/crates/core/src/session/pump/tests.rs) verify
accepted versus failed command capture and ordering under the shared gate.
These tests do not certify delivery to PostHog or every installed CLI version.
[`src/analytics.test.ts`](../../src/analytics.test.ts)
checks that only catalog keys become click targets, that captured clicks carry
the right control, target and region, that `data-track` tokens name dynamic
controls and their nested parts, and that repeated screens are not counted. The IPC parity
test in [`src-tauri/tests/mock.rs`](../../src-tauri/tests/mock.rs) covers command
registration.
