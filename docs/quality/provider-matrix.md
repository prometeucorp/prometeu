---
type: Quality
title: "Provider matrix"
description: "Support per agent and expected evidence."
status: stable
---

# Provider matrix

Current behavior observed in the code. "To confirm" means the feature
may exist, but does not yet have enough conformance evidence to become a
contractual capability.

Browser evidence follows the [E2E scope policy](../operations/development.md#e2e-scope).
Chromium runs the retained scenarios; only cases explicitly tagged `@webkit`
repeat in WebKit. References to both engines mean representative coverage, not
every combination of providers, states, languages and screen sizes.

## Remote machines

Optional [workspace supervision](../contracts/supervision.md) publishes metadata
from the remote Claude host to the Cloud and generic signed webhook receivers.
Codex, Cursor and Antigravity do not publish supervision events; local supervisor
selection and reverse MCP transport are deferred. Core supervision tests prove
child-aware completion, duplicate suppression and restart reconciliation;
`crates/host/src/supervision.rs` and `crates/host/tests/agents.rs` exercise
persistence, publication and the MCP path without live provider calls. Cloud
tests cover account-scoped ingestion, GitHub routing and callback delivery.

Settings lists and ends host-owned processes, including idle conversations and
Claude sign-in terminals. This adds a guarded stop to the host terminal protocol and
does not change which providers run remotely. `src/machine-processes.test.ts`
checks liveness, machine-scoped labels, confirmation races, legacy hosts and refresh lifetime;
`stopping_a_listed_process_never_kills_a_replacement_under_the_same_id` verifies
that stale confirmations cannot kill a resumed conversation;
`terminals_survive_detachment_and_replay_the_gap` in the host tests checks
listing before and after a targeted kill.

Several computers can follow the same machine at once
([ADR 0104](../decisions/0104-several-computers-on-one-machine.md)). This
changes the host attachment, not provider support: hooks and permission requests
are shared as Claude emits them; each computer following a Codex conversation
reads the engine through its own relay ([ADR 0105](../decisions/0105-codex-on-remote-machines.md)). The host
integration tests in `src-tauri/crates/host/tests/host.rs` and
`src/machine-clients.test.ts` cover it.

Claude and Codex declare `remoteMachines`. Claude's native hooks are relayed by
the remote host and its project transcripts are mirrored for the observer
([ADR 0101](../decisions/0101-remote-machines.md)). Codex runs its native engine
and TUI on the machine, and the desktop reads the engine's RPC through a relay
terminal on the host ([ADR 0105](../decisions/0105-codex-on-remote-machines.md)).
Cursor and Antigravity are rejected for remote workspaces until their hooks and
stores run on the host. Both remote invocations are the local terminal ones
without account profile, MCP or plugin arguments: the machine's own login and
configuration apply. Settings signs Claude in on the machine with
`claude auth login --claudeai`, the person's browser and a pasted code; the
opt-in `vps_claude_login` test covers the link on Claude Code 2.1.330. Codex is
signed in by the person on the machine (`codex login --device-auth` over SSH);
Settings and the launcher show whether it is installed, at least 0.169.0 and
signed in, and a launch without them fails with the step to take.

| Remote behavior | Claude | Codex | Evidence |
| --- | --- | --- | --- |
| observation | relayed hooks and mirrored transcript | engine RPC through the host's `codex-relay` terminal | `host.rs` relay tests, `wire.rs` tests |
| sign-in | Settings runs `claude auth login` on the machine | `codex login --device-auth` run by the person on the machine; status only | `remote/codex.rs` tests, `src/machines.test.ts` |
| SSH loss | terminal and hooks replayed | relay lines replayed; a gap over 2 MiB or a lost relay re-adopts from engine history | `remote/codex.rs` tests, `host.rs` detachment test |
| app restart | adopts the running TUI and transcript binding | adopts the running TUI and engine with a new relay | not exercised against a real machine |
| exit while away | finished terminal adopted to collect output and transcript | finished TUI forgotten; orphan engine ended at the next attachment | `remote/codex.rs` tests |
| agent clients (ADR 0103) | listed and resumable | not listed in the machine's registry | `remote/registry.rs` tests |

Project registration is managed in Settings > Projects, filtered by computer.
The launcher lists only registrations on the selected machine; local origin
aliases do not hide remote clones or cause implicit installation. This selection
rule is covered by `src/machines.test.ts`; navigation compatibility by
`src/settings-navigation.test.ts`. `e2e/projects.spec.ts` covers the browser
journey through machine selection and nested registration without losing the
workspace prompt. Remote execution covers Claude and Codex.

Agent clients on the machine
([ADR 0103](../decisions/0103-agents-on-remote-machines.md)) launch Claude only,
without permission prompts; `src-tauri/crates/host/tests/agents.rs` covers them.
The desktop adopts their workspaces as Claude terminal tabs that run in that
mode, and `src-tauri/src/remote/registry.rs` covers the adoption.
Settings enables them on a connected machine and shows a generic local stdio MCP
configuration for any client. Container and network setup belong to the
integrator. `the_generated_local_config_lists_tools_and_relays_calls` in the host
integration tests executes the generated configuration; host unit tests cover
unchanged home permissions and legacy cleanup. This changes client integration,
not provider support: execution still launches Claude only.

Evidence: the host integration tests and the opt-in `vps_journey` cover
transport, reattachment and limits. A live Claude response, completion and
reattached status on a remote machine remain to be recorded here. Remote Codex
has not yet run against a real machine with Codex 0.169.0 or newer installed and
signed in: a fresh response, an interrupt, a request answer, a reconnection and
an app restart remain to be recorded.

`remote_reattachment_before_first_prompt_preserves_identity_and_requires_empty_history`
in `src-tauri/src/chat/terminal/ingestion/tests.rs` covers restarting the desktop
after Claude bound its native identity but before its first transcript exists.
Adoption preserves that identity and status metadata; local resumes, existing
transcripts, captured work and prepared input retain strict history verification.

## Native conversation mode

The optional conversation mode format is shared by Claude, Codex, Antigravity
and Cursor. It persists a global new-conversation preference independently
from each tab's fixed mode, including action tabs. On macOS, **Settings → General
→ New conversations** selects Chat or Terminal for newly created conversations.
Changing it does not convert existing tabs or restart their processes. Linux
keeps the control hidden and explicitly rejects terminal mode.
Local Chat and Terminal views are exclusive. Terminal tabs have no transcript
screen or surface switcher; canonical persistence, status, notifications and
remote transcript sharing remain independent of that local presentation.
The native terminal fills the conversation area without a permanent footer or
floating controls. The workspace menu at the right of the central tab bar, tab
context menus and desk-header menus open the shared actions, MCP, plugins,
skills, context, dictation and remote access controls on
demand; quotation and manual resume are contextual menu actions. Actionable collaboration notices
remain visible when the owner must respond. Model and effort
changes use the native CLI; launch defaults remain available. Selecting,
dropping or pasting files inserts paths directly into the native draft without
Enter. Browser context, review notes and text previews remain staged in the
tools dialog for explicit insertion; the CLI owns the only editable local
message input. Failed insertion retains the original tab's draft without
resuming a process or retargeting another tab. The default Open PR action also
drafts its request into the native composer for the person to submit, instead of
queueing it as app input; an exited agent is resumed with the request queued
(`e2e/composer.spec.ts`).
Queued local prompts and staged context draw attention through a conditional
indicator in the tab-bar action group or desk header, separate from agent status. Their preview is
available through the tools dialog, shared across providers.
Native typography uses 14 px SF Mono on macOS with monospace fallbacks,
500/700 weights and line height 1. The neutral palette uses `#282c34` and white
text; shared helper defaults keep OSC reports and replay aligned. Existing
native color changes survive restoration, and running helpers keep their current
colors until normal replacement. The renderer/helper versions remain pinned to
xterm 6.0.0; the desktop view draws with the WebGL addon and falls back to DOM
rendering, without automated GPU rendering conformance. Geometry and input coverage remain in
`e2e/composer.spec.ts`, `src/term.test.ts` and the terminal-state replay/query tests;
browser evidence remains distinct from native macOS app conformance.
OSC 8 HTTP(S) hyperlinks use the system browser for all terminal providers and
supporting terminals. Plain text URLs without OSC 8 markup are not detected.
`src/term.test.ts` checks external IPC routing, rejected schemes, destination
hover text and opening errors, including both supporting terminals' UI callbacks,
with mocked xterm/IPC; native browser launch still
requires a macOS app check.
All four local terminal providers share automatic saved-session resume when a
stopped tab is opened. `src/terminal-conversation.test.ts` covers concurrent
surfaces, fresh snapshots, no input submission, bounded failure retry and stale
navigation/disposal; `src/conversation-view.test.ts` keeps the manual retry
hidden while attaching or running. These lifecycle tests use mocked IPC.
Compatibility tests live in `crates/core/src/board.rs`,
`crates/core/src/session/launch/tests.rs`,
`src-tauri/src/session.rs`, and `src/backend-contract.test.ts`.

Native command preparation in `src-tauri/src/terminal_agent_launch.rs`, the PTY
host, screen restoration and canonical ingestion are composed in the macOS
application. Actual macOS Tauri verification with isolated scratch projects and
existing authenticated accounts passed native responses, canonical user and
assistant content, and `turn.completed` for all four providers. Cold resume
retained each bound native identity and recovered history without duplicate
prompts. Cursor also passed a live follow-up after the physical-workspace path
fix. Graceful app exit left none of the nine owned direct children alive: four
provider CLIs, the Codex engine and four terminal-state helpers. Antigravity's
programmatic follow-up reached the native TUI, and canonical recovery after the
hidden-system-message fix retained exactly two users and two answers. Recovery
kept the earlier completion count unchanged; a new live completion after that
fix was not separately exercised. The native CLI smokes and automated tests
below retain their narrower scope; these checks do not establish
every tool, decision or sharing combination. Later provider tables describe the
existing Chat adapters unless they explicitly name native terminal behavior.

Native queued input has a separate durable dispatch token and a prepared-input
intent before process writes. Core and journal composite tests cover crash
recovery, exact receipt reuse without another send, preservation of later
pending text and refusal to retry uncertain acceptance. Causal source replay
uses native Cursor/Codex identities, Claude row UUIDs and Antigravity step indices
to reconcile an accepted intent; unsupported source content withholds coverage.
AGY 1.5.10 terminal content now reads its scoped original SQLite trajectory and
complete output files. A native Ask-approved `printf` smoke and offline projection
proved one exact call/result identity; synthetic tests cover WAL immutability,
output changes, paths, Fresh/Resume and durable acknowledgment. Auxiliary Any
metadata can accompany complete native output; unknown result forms remain
coverage failures. This does not establish all-tool parity. See
the [AGY store boundary](../contracts/native-antigravity-store.md).
Observer and journal fixtures cover replay, rewrites and metadata receipt ordering;
those tests do not establish complete native control parity. The board/TypeScript
boundary fixture includes the additive dispatch state, whose text remains
visible until durable acceptance. Ordinary Chat queue behavior is unchanged.
The ingestion composite also checks that a sealed source row keeps current tool
effects, while a result after completion, interruption or a new committed app
input updates history quietly. Causal-user and trajectory bindings survive
restart; journal schema migrations preserve public bytes and native receipts.

Empty-session restart has a narrower limitation: a tab without a native session
identity cannot resume, and an unresolved prepared input with uncertain
acceptance remains explicitly blocked. The observed empty AGY and Cursor cases
do not automatically recreate a conversation or resend input. The journal
regression for proven-unwritten cancellation covers safe retry after a live
rejection; it does not authorize retry of an uncertain intent after a crash.

| Provider | Native preparation and behavior | Evidence and specific remaining checks |
| --- | --- | --- |
| Claude | Exact native session id, captured account, model/effort, explicit permission mode, instructions and selected MCP/packages | Tauri response/history/completion and cold resume passed, as did native hook question response, native-dialog race and explicit Bash approval; ExitPlanMode request opening passed, while its response delivery and app decision controls remain unverified |
| Codex | Owned app-server and remote TUI, exact resume id, derived account/tools home, native config and selected hook hashes | Tauri response/history/completion, cold resume and engine cleanup on app exit passed; native Ask/Plan bootstrap, RPC recovery, question/answer and explicit command approval also passed. File-change approval, quota and shared-control conformance remain |
| Antigravity | Workspace, exact resume id, model/effort, permission/plan flags and private always-on instructions; decisions stay in the native TUI by product choice | Tauri initial response/history/completion, cold resume and programmatic follow-up with quiet canonical recovery passed; a new live completion after the hidden-system-message fix remains unverified. Native instructions and an Ask-approved printf/tool projection passed. Nonempty hub selections remain unsupported |
| Cursor | Workspace, exact resume id, model, native permission/plan flags, private native prompt-hook instructions and selected stdio MCP activation; decisions stay in the native TUI by product choice | Tauri response/history/completion, cold resume and live follow-up passed, as did native successful-stop fence and one Shell call/recovery; MCP tool calls/remote transports remain unverified. Separate effort and hub packages remain unsupported |

The opt-in ignored native smoke in `terminal_agent_hooks/smoke.rs` has verified
fresh response and exact resume for Claude 2.1.326, Codex 0.169.0,
Antigravity 1.5.9 and Cursor 2026.11.13. Claude required the real headless
emulator's query replies in this harness. Codex and Antigravity also accepted a bracketed-paste follow-up after
an explicit native idle/done report. Cursor emitted no verified OSC readiness
signal. These short no-tool prompts use isolated scratch workspaces and existing
external credentials. The earlier direct Codex smoke used `on-request` with `read-only`; the owned
engine proof below supersedes that preparation path for Ask/Plan. Antigravity also obeyed private always-on instructions on fresh start
and resume. Cursor rules from private plugins/additional roots did not load in
this build; its native prompt-hook context instead passed fresh/resume with a
changed instruction and unchanged user prompt. The verified carrier is limited
to 10,000 UTF-16 units. These
smoke results are separate from end-to-end Prometeu conformance.

The ignored `terminal_agent_launch::mcp_smoke::native_cursor_loads_selected_stdio_mcp_without_inference`
also passed on Cursor `2026.11.13-eacade1`. It uses the real preparation and
MCP materializer, a private plugin and `CURSOR_CONFIG_DIR`, and the existing
native authentication. The selected stdio server received `initialize`,
`notifications/initialized` and `tools/list`, with its configured environment
overriding an inherited value. No prompt or `tools/call` was sent. The test
confirms the scratch path before native trust, checks private artifact modes
and reaps the PTY, fixture server and screen helper. It does not certify remote
MCP transports or native tool invocation; `agent mcp list-tools` is not used
as evidence because that subcommand does not load invocation plugins.

The [Claude decision bridge](../contracts/native-claude-decisions.md) also passed
two bounded synthetic-question cases on 2.1.326: an explicit hook answer was
returned by the native tool; a local Blue choice remained Blue after a late
hook answer of Green was delivered. The canonical card closed as `resolved`,
which records delivery rather than claiming that the hook won. Timeout,
disconnection and retirement remain neutral. A separate native Bash case verified
the exact literal printf command, permission dialog before the explicit hook
allow, native tool completion, the exact scratch-file result and duplicate
response rejection. Ask+Plan startup now preserves manual permissions without
enabling bypass. ExitPlanMode reached its native empty-plan dialog and emitted
the plan request; real response delivery remains unverified. The empty-plan
selector has an offline regression. Broader native rule combinations and shared
remote control remain unverified.

The separate [Codex engine proof](../contracts/native-codex-engine.md) uses the
actual native TUI with per-thread `untrusted`, `dangerFullAccess`, Plan and low
effort without a model request. A separate bounded one-turn smoke fed actual
RPC into the runtime consumer and cold `thread/turns/list` replay left canonical
bytes unchanged, with one user and one completion. Fake-process and consumer
tests cover bounded pagination, explicit-only decisions, cleanup, source receipts
and active-tool crash tails. A real Plan question also passed through the native
TUI and attached answer adapter, with Blue returned by the model, native card
resolution and rejection of repeated answers. An explicit one-shot command
approval also passed, with native resolution, matching completed tool identity,
exact scratch-file contents and repeated-answer rejection. File-change approvals,
quota and shared remote control still require native conformance.
The previous on-request stop smoke does not certify those behaviors.

Cursor and Antigravity terminal decisions intentionally use the native TUI in the first version;
application and shared approval/question buttons are outside that accepted scope.
Chat keeps its existing capabilities. `conversation-capabilities.test.ts`, `team-viewer.test.ts`,
`team-owner.test.ts`, `team-channel.test.ts`, `island.test.ts`, relay compatibility
tests and the native input rejection test cover per-conversation ownership,
encrypted announcements, old-client responses and preserved native input.

Across all four native providers, slash commands and permission-mode changes
stay in the TUI. Shared transcript controls omit provider command suggestions,
compact/context actions and approval actions that switch to bypass. User Actions
still expand first, ordinary prompts retain paths such as `/Users/file`, and
Claude/Codex keep individual approval/question responses. Manual slash rejection
preserves the sending viewer's composer; shared mode metadata remains encrypted and both
viewer and owner guard unsupported commands. Pure grammar, relay compatibility,
encrypted round-trip and peer-input tests cover this boundary.

Native input admission in `native_ui_readiness.rs` uses real Claude, Cursor,
Codex and Antigravity viewport captures. Codex's fresh/cold-resume and literal/Home/clear test
uses no inference: the dim placeholder distinguishes empty input from an
identical draft at the same physical cursor. It still requires separate native
idle. Antigravity 1.5.10 uses the captured single-row composer, physical cursor
at Home, full-width borders and exact idle footer, also with separate native
idle. A read-only audit of its pinned submit handler proves plain spaces are
discarded when no pasted-content placeholders exist. Regression tests reject
literal mode-hint text at Home; the native empty hint instead requires its
captured palette-8 foreground and exact native truncation. An explicit
`NO_COLOR` hint remains ambiguous and does not release programmatic input.
Tests also reject
literal drafts, chips, menus, incomplete paint, changed geometry and unknown
versions; application payloads remain unchanged. See the
[fixture evidence](../../src-tauri/src/native_ui_readiness/fixtures/README.md).

Cursor's pinned native-store decoder additionally covers tool arguments/results,
MCP JSON values, read-content blobs and compacted tool references in
`crates/protocols/src/cursor_store/tests.rs`. Call/result revisions retain the
native call ID and are committed separately; they do not invent approval cards
or turn completion. Unknown tool fields, shell-only turns and synthetic users
still withhold full source coverage. A controlled native Shell smoke additionally
passed prompt painting, one explicit native approval, execution and copied-store
recovery with matching native call/result IDs. Other tool variants retain fixture
coverage only; these checks do not establish full app or programmatic approval
conformance. Cursor approvals intentionally remain native.

Native stop profiles in `terminal_agent_controls.rs` require an observed active
turn and a versioned, current composer hint. Codex 0.169.0 and Antigravity 1.5.9/1.5.10
have additionally passed the opt-in no-tool interruption smoke: one Escape
produced an explicit idle/done report and a native interrupted message while
the same process remained alive. Codex uses `on-request`/`read-only` in this
check; Antigravity uses explicit Ask settings. Startup MCP cancellation is a
negative Codex fixture, not model-turn evidence. Claude 2.1.326 also passed one
Escape after its prompt hook and native Working, outside the startup-hook phase,
followed by explicit Idle with the process alive. Its compact composer can sit
above the viewport bottom and hide the physical cursor while working; the
current cursor position, borders and footer identify the control. Cursor
2026.11.13-eacade1 also passed a no-tool interruption: its native prompt, model
output and Working/footer preceded exactly one Ctrl+C; an authenticated root
Stop reported `aborted`, the process remained alive, and its waiter confirmed
reaping after the proof. Cursor restores the interrupted prompt as a draft; the
harness preserves it. Any later Stop reasons are recorded separately, and no
OSC idle/done is invented. The control accepts a scrolled-off startup banner
only with the current aligned composer/footer/cursor. The fixture tests reject conflicting bindings,
partial paints, transcript mentions and versions below the floor; unsupported layouts
remain unavailable. Cursor's native hint is hidden when its composer has a draft.

For Cursor, the pinned source also accepts local Escape with an empty prompt
when no overlay/history/queue action consumes it; the application Stop profile
still sends its advertised Ctrl+C. The authenticated root aborted hook is bound
to the current durable request before committing canonical `interrupted`.
Observer regressions reject old requests and new-prompt races and retain
cancellation across a later completed hook for the same generation. They also
clear the pending prompt-generation barrier after exact durable capture and
allow a later native request without a repeated prompt hook; bounded Stop
candidates preserve each request's evidence despite delayed older hooks.
`journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet` covers
the real portable pump, duplicate receipts, a restored native draft and late
source/tool activity without a success notification. These tests cover the
native interruption evidence above; they do not claim a new Tauri Esc run.

Cursor's spinner also starts before the native store catches up when a local
Enter boundary is followed by a newer, verified busy composer. The host test
`cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`
checks initial work, settled-screen rejection and a new native submission.
`cursor_working_observation_requires_its_current_complete_busy_composer` and
`observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once` cover
the screen profile and shared execution cycle. Enter alone and paste are
negative cases. `cursor_restored_prompt_retry_settles_without_repeating_before_submit_hook`
covers the independent source identity barrier and delayed Stop ordering; none
of these synthetic tests claims a new native model run.

The recorded no-tool AGY 1.5.10 Escape sequence is `Idle` → `Working` → `Idle`.
`terminal_execution` and native host regressions cover quiet settlement of that
root activity, late source updates, the next input and preservation of children,
requests and valid Done tickets. `src/timeline.test.ts` and `src/alert.test.ts`
cover clearing the busy presentation without a success notification. These
regressions exercise the observed native sequence.
A separate owned native file-read/Escape run on 1.5.10 captured `Idle` → `Working`
→ `Done` → `Idle`, with an explored file, the native interruption notice and the
process still alive. The projector and host replay that captured sequence with
late source thinking/tool content: the tab stays ready, content is retained, and
no success is inferred from Done alone. Completion-ticket and delayed-input
regressions preserve the independent source barrier. This is native CLI evidence
plus host regression coverage, not a new Tauri cancellation run.
The same recorded interrupted viewport also covers input readiness: the native
interruption banner above the empty composer does not block queued input. The
host regression retains the local-input fence until a newer verified paint and
native idle, then admits the next message without retrying its submission.

The provider-neutral OSC parser/reducer tests cover chunked status records,
bounded retention and explicit per-record completion. Native PTY tests cover
input, resize, shared process control and reaping. Neither group proves
permission response, notification eligibility, content sharing or usage capture.

The native renderer is shared across providers. `src/term.test.ts`
covers generation filtering, faithful-screen replay, pending UTF-8, query suppression,
ordered text/binary input and cleanup;
`src/conversation-view.test.ts` covers exclusive local views, remote transcript
routing and native quotations. `src/native-conversation-draft.test.ts` covers
concurrent context retention and exact review acknowledgment by tab and envelope,
including repeated snapshots and closed-tab cleanup. It also checks individual
image-path formatting and document mentions. The image format follows the
Codex 0.169.0 source normalizer: one literal path per paste, with quoting when
needed and no `@` prefix. This source inspection does not establish native Tauri
image-chip rendering or another provider's image recognition. `src/term.test.ts` and
`src/terminal-conversation.test.ts` cover generation-bound insertion, bracketed
paste readiness, rejected controls and no automatic Enter or resume. The
native-screen case in `e2e/composer.spec.ts` checks actual xterm geometry and
keyboard focus. Those
browser fixtures do not certify native provider behavior.

Claude image reads have native-ingestion regressions in
`terminal_transcript::tests::terminal_transcript_claude_tool_images_keep_results_and_validate_the_envelope`
and `terminal_observation_claude_read_image_survives_ingestion_and_cold_resume`.
They cover oversized base64 images nested in tool results, small ones kept
byte-identical for journals saved by earlier parsers, preserved text and tool
identity, invalid envelopes, and cold replay of both without duplicate canonical
output. This is synthetic parser and journal evidence, not a native Tauri run.
Codex retains its separate inline-image shapes; no other provider behavior changes.

The private [terminal state helper](../contracts/terminal-state.md) has shared
headless replay and stdio tests in `packages/terminal-state/test/`. The macOS
desktop host and renderer use its screen contract, single query authority and
generation ordering. The Node SEA/Tauri bundle has been verified locally
on macOS arm64 without installed Node and is included by the standard macOS
app/release build. CI verifies helper and host behavior; the Linux release does
not consume the helper. These shared transport tests
establish no additional provider capability and do not replace the Tauri app
verification above.

## Automatic worktree cleanup

The local opt-in preference applies equally to Claude, Codex, Antigravity and
Cursor when manually archiving or finishing from the desktop UI. It uses the existing cleanup IPC with no provider
payloads. Dirty, unmerged or unverifiable worktrees retain explicit selection;
original clones and remote workspaces are excluded. `src/cleanup.test.ts`
covers defaults, persistence, safe scoped deletion and error handling; existing
archive/finish wiring is covered by `e2e/audit-regressions.spec.ts`.

## Sidebar resizing

The desktop sidebar supports pointer dragging, Left/Right keyboard resizing and
Home/double-click reset, with a local saved width. Behavior is identical for all
providers, including Claude, Codex, Antigravity and Cursor. The browser check in
`e2e/sidebar-resize.spec.ts` covers pointer capture, actual flex width, focusable
separator input, collapse/reopen, reload and window resizing in Chromium/WebKit.
It also covers both panels at large saved widths, dragging either edge, panel
visibility and shrinking the window while preserving conversation space.
This browser exception protects geometry and pointer capture across engines;
unit tests cannot prove hit testing or flex layout. No provider protocol changes.

## GitHub issues and pull requests

GitHub issue and PR discovery uses the same desktop path for Claude, Codex and
Antigravity. All three receive the originating title, description and URL through
the existing launcher; PR worktree preparation is independent of the agent.
Tests: `src-tauri/src/github_auth.rs`, `src-tauri/src/github_issues.rs`,
`src-tauri/src/github.rs`, `src/github-issues-model.test.ts` and the workspace
journey in `e2e/issues.spec.ts`. The browser scenario checks provider-tab focus,
nested row actions, claiming from Available and prepared PR source preservation.
See the [contract](../contracts/github-issues.md). GitHub notifications are also
agent-independent; tests are in `src-tauri/src/github_notifications.rs` and
`src/notification-feed.test.ts` ([contract](../contracts/github-notifications.md)).

## Local telemetry foundation

Settings / Analytics now uses the reviewed Usage screen.
`src/usage-history.test.ts`, `src/telemetry-settings.test.ts`,
`src/telemetry.test.ts` and native telemetry tests cover inclusive totals,
session coverage, unknown values, query cohorts, stale reads and deletion.
The additional Settings browser scenario in `e2e/usage-insights.spec.ts` protects
live detail and accessible-name updates, keyboard focus and horizontal scrolling in Chromium/WebKit;
DOM replacement and WebKit's overflow focus require browser evidence, which the
pure mapping/controller tests cannot provide. Provider measurement capabilities
remain unchanged. Remaining work is in the
[improvement proposal](telemetry-improvements.md).

[ADR 0059](../decisions/0059-local-telemetry-foundation.md) adds the same local
capture/store/settings path for all providers. Main completion remains separate
from the conversation settlement rule in ADR 0056.

| Measurement | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Main execution and accepted message turns | Captured | Captured | Captured |
| Independent child execution and explicit human waits | When canonical signals expose them | When canonical signals expose them | Unavailable |
| Main-agent input/output | Turn result; verified `modelUsage` deltas can additionally cover the whole tree | Delta of verified thread totals; resume/reset without a baseline stays partial/unknown | Deduplicated current steps; verified cumulative deltas complete the turn |
| Cache/reasoning | Cache read/write are input subsets; reasoning unknown | Cache is an input subset; reasoning is an output subset | Reported cache/thinking subsets, otherwise unknown |
| Observed models/calls | Main assistant IDs and verified cumulative model rows; distinct message calls | Unknown without verified actual-model/call evidence | Distinct observed step IDs; model breakdown unknown |
| Per-turn cost | Same-session cumulative delta; restored/reset/overlapping-child spend excluded | Unknown | Unknown |
| Context meter/actions | Observed main-call input and reported model window; explicit compact/report/new conversation | Last input and model window; explicit compact/report/new conversation | Hidden without a reported window |
| Cache rebuild signal | Deduplicated calls with known preceding context and cache-write evidence | Unknown without reliable distinct-call evidence | Only if distinct-step cache-write evidence exists; otherwise unknown |
| Reply footers/workspace summaries | Canonical usage; local hashed lookup restores native transcript footer | Canonical usage; no invented price | Canonical observed usage; no invented price |
| Usage screen and session details | Known tokens, cache, conditional cost/model rows and coverage | Known tokens, cache/reasoning and coverage; cost/models remain unknown | Known step/turn usage and coverage; missing fields stay unknown |
| App-call source | Naming and plugin maker, separately measured | Naming, separately measured | No app-initiated model call |

`telemetry/tests.rs`, the adapters' `telemetry_*` tests and
`src/telemetry.test.ts` cover normalization, incomplete coverage, local query and
erasure behavior, including late pre-erasure terminals, concurrent WAL readers,
streaming export and symlink rejection. `src/telemetry-settings.test.ts` covers
workspace option refresh while a filter stays selected. Existing command/publication tests cover accepted/failed sends
and response ordering. Streaming input without a correlatable native terminal
retains unknown turn attribution; capture health exposes that limit. These are
fixture-based guarantees, not live certification of every installed provider
version. Full semantics are in the [contract](../contracts/telemetry.md).

Issue 131's presentation is covered by `src/usage-presentation.test.ts`,
`src/workspace-usage.test.ts`, `src/timeline.test.ts`, `src/conversation.test.ts`, `src/telemetry.test.ts` and
`e2e/usage-insights.spec.ts`. The browser scenario protects keyboard/Escape focus
and preservation of the typed draft when a context action uses the shared
popover, including a turn starting and ending while the panel stays open. The
disabled-action transition must preserve Tab/Escape focus in Chromium and WebKit;
pure tests cannot establish browser focus after DOM replacement. Model/provider
label disambiguation stays in the pure presentation tests. The browser case does
not multiply provider or locale variants, and it does not exercise a native CLI.
Adapter fixture tests cover consecutive counters, restored/reset/error results
and duplicate observations; synthetic Claude fixtures explicitly declare their
provenance. Telemetry tests cover reply privacy/lookup, grouping, sources and PR
evidence. Unsupported measurements remain unknown, never a billing estimate.
The native telemetry producer test verifies commit visibility and lock release
before `telemetry-changed`; the workspace consumer test covers a refresh without
a board change, coalescing, navigation and history-deletion invalidation.

Verification additions shared by the existing features:

- `boundary_contract.rs`, `scripts/check-contracts.mjs` and
  `src/backend-contract.test.ts` check backend board/model serialization and
  synthetic Claude/Codex output against frontend types and reducers. Antigravity
  retains its recorded canonical fixture in `src/conversation.test.ts`.
- `codex/process_tests.rs` checks initialization, split UTF-8, interruption,
  partial output at process exit and resume through a controlled subprocess.
  It does not establish live CLI compatibility or equivalent subprocess coverage
  for Claude/Antigravity; see the [runtime contract](../contracts/agent-runtime.md#process-boundary-verification).
- `crates/core/src/workspace_lifecycle.rs` tests finish/archive/restore state preservation for
  every provider through shared board rules, without invoking any provider.

## Product analytics

[ADR 0093](../decisions/0093-anonymous-product-analytics.md) separates named
control activations from explicit feature use. The
[contract](../contracts/product-analytics.md) defines triggers and the
[coverage map](product-analytics-coverage.md) inventories product modules.

Claude, Codex and Antigravity share the same startup consent question when no
valid saved choice exists. Both answers persist; Settings / General / Privacy
can change the choice. Tracking stays off until acceptance. The setting gates native and frontend events
without changing agent execution or local history. Rust consent/persistence/batch
tests live in `src-tauri/src/analytics.rs`; startup accept/decline/defer and
Settings tests live in `src/analytics-settings.test.ts`.

| Measurement | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Named controls and navigation | Shared desktop UI | Shared desktop UI | Shared desktop UI |
| Files, Git, review, terminal and preview | Shared feature paths | Shared feature paths | Shared feature paths |
| Model and effort selection | Available choices only | Available choices only | Available choices only |
| Requests and approvals | Only capabilities exposed by the adapter | Only capabilities exposed by the adapter | Interactive approval and plan controls unavailable |
| Resource setup and actions | Configuration/use separated | Configuration/use separated | Only supported resources/actions |

Product events no longer observe canonical turn completion or assistant output.
Feature availability remains governed by the capability matrix; absence of an
unsupported control is not evidence of non-use. No provider payload or content
is sent.

Native analytics tests and shared fixtures validate the closed event schema;
TypeScript tests check parity, identity capture and feature triggers. The
real-DOM analytics check verifies metadata on shared controls and portals.
These checks do not certify live PostHog delivery or every CLI version.

Only opted-in macOS/Linux desktop builds compiled with the key send events.
Mobile does not; the browser mock persists the choice but ignores
events. Missing events from those shells or installations without consent must
not be counted as missing usage.

## Legend

- **Native:** the CLI already speaks the form consumed today.
- **Adapted:** Prometeu converts or implements the feature.
- **Unavailable:** the UI does not offer it because the provider does not
  support the flow.
- **To confirm:** a fixture or dedicated test is missing.

| Capability | Claude | Codex | Antigravity | Main evidence |
| --- | --- | --- | --- | --- |
| terminal sessions without a workspace or project, named, closed by × or `exit` and listed again after a restart | independent of the CLI; no agent runs in them | independent of the CLI | shared application behavior | `crates/core/src/terminal_sessions/tests.rs`, `src-tauri/src/terminal_sessions.rs`, `src/terminals.test.ts`, `fixtures/backend-contract.json`; the sidebar group, view and shortcut were checked by hand in the browser mock and have no E2E scenario |
| task inbox with search, team/status or repository/label filters and workspace actions | shared UI, independent of the CLI | same application flow | same application flow | `src/issues-filter.test.ts`; `e2e/issues.spec.ts` for menu focus, narrow layout and workspace launch; Cursor uses the same UI |
| claim an unassigned Linear issue from a joined team | independent of the CLI; assigned issue can start a workspace | same application flow | same application flow | `linear.rs` eligibility, scope and cache tests; `e2e/issues.spec.ts` keyboard, pending-action and read-only navigation paths through the mock; live Linear consent and mutation still need manual verification |
| grouped settings and searchable resource library | shared UI; existing account and tool capabilities | same UI and existing capabilities | same navigation; external account notice and unsupported tool execution remain unchanged | `src/settings-navigation.test.ts`, `e2e/settings.spec.ts`, existing accounts, Cloud, MCP and notifications scenarios |
| create, rename, trash and restore from the side file tree | independent of the CLI | independent of the CLI | shared application behavior | `src/tree-menu.test.ts` for the menus; `src/tree-moves.test.ts` for the tab strip, drafts and expanded folders following a moved entry, while their wiring in `workspace.ts` and `viewer.ts` was checked by hand in the browser mock and is not covered in E2E; `crates/files/src/entries.rs` tests for names, Git metadata, conflicts, case-only renames on a case-sensitive disk and symlinks; `restore_deleted` tests in `crates/git/src/tests.rs` for staged content and deletions; case-insensitive disks are not exercised in Linux tests |
| Git marks in the side file tree | independent of the CLI | independent of the CLI | shared application behavior | `src/tree-git.test.ts`, `src/git-refresh.test.ts`, `tree_marks` and single-flight tests in `crates/git/src/tests.rs`; `e2e/audit-regressions.spec.ts` verifies file-row stability after saving, while mark colors are not covered in E2E |
| copy a complete Markdown code block, including diff fences | shared presentation | shared presentation | shared presentation | `src/markdown.test.ts`, `e2e/markdown.spec.ts`; clipboard success and failure over the browser mock |
| public bug reporting and private feedback with an account, image and capture | independent of the CLI | independent of the CLI | shared application behavior | `e2e/feedback.spec.ts`, the Cloud's `FeedbackTest` tests; opening the system browser, native capture and real GitHub require a manual smoke test |
| organizations, invitations and institutional sharing | the same relay V4; local execution | the same relay V4; local execution | shared application behavior | `team-organizations.test.ts`, `worker.integration.test.ts`, `e2e/organizations.spec.ts`, Rails integration/browser |
| automatic E2EE with TOFU in collaboration | the same HPKE Auth channel; no forward secrecy | the same HPKE Auth channel; no forward secrecy | shared application behavior | `team-crypto.test.ts`, `team-security.test.ts`, `team-channel.test.ts`, `worker.integration.test.ts`, E2E comments in Chromium/WebKit |
| leave a share and readmit a person who left | the same relay V4; local execution | the same relay V4; local execution | shared application behavior | `relay/src/logic.test.ts`, `relay/src/protocol.test.ts`, `team-channel.test.ts`, `team-owner.test.ts`; the sidebar menu and confirmation are not covered in E2E |
| per-person rights to send messages and control, from approved devices only, with a notice on the owner's Mac | prompts and interrupt need Send messages, request answers need Control; viewers keep content and comments | same rule before the Codex adapter | same rule before the Antigravity adapter | `team-rights.test.ts`, `team-security.test.ts`, `team-owner.test.ts`, `team-channel.test.ts`, `team-member.test.ts`, `team-organizations.test.ts`, `share_tests` in the core crate; the notices, share menu and hidden controls were checked by hand in the browser mock and have no E2E scenario |
| owner confirmation of teammates' messages | a granted teammate's prompt waits above the owner's composer with Send, Edit and Discard, only in memory; interrupts, answers and the owner's devices run at once | same rule before the Codex adapter | same rule before the Antigravity adapter | `team-owner.test.ts`, `team-organizations.test.ts`, `share_tests` in the core crate, `fixtures/backend-contract.json`, the `chat-pending` gallery story; the card, the share-menu option and desk tiles were checked by hand in the browser mock (`mock.message()`) and have no E2E scenario |
| optional Prometeu account in the sidebar | independent of the CLI | independent of the CLI | shared application behavior | `cloud.rs`, `e2e/cloud.spec.ts`; no transcript is sent |
| Git project catalog, batch clone and local registration | independent of the CLI; no setup | independent of the CLI; no setup | shared application behavior | `catalog/projects.rs`, `e2e/projects.spec.ts`, Cloud `projects_catalog_test.rb` |
| create an empty local Git project and start its first workspace | independent of the CLI; no initial commit or setup | same flow | shared application behavior | `crates/git/src/tests.rs`; `e2e/projects.spec.ts` checks nested form input retention and focus, then starts a conversation in the project folder without a worktree |
| catalog of plugins, MCP and Actions in the account | independent of the CLI | independent of the CLI | shared application behavior | `catalog.rs`, `catalog_test.rb`; secrets and installation stay per Mac |
| the organization's plugins, MCPs and skills | explicit local installation in the hub | the same UI and installation; existing adapters | Unavailable | `catalog.rs`, `organizations_test.rb`; optional installation form has no dedicated E2E; it requires no personal copy and does not activate automatically |
| Actions form with shared components | the same UI | the same UI; options come from the catalog | shared application behavior | `e2e/actions.spec.ts` in Chromium; shared form focus/validation in `e2e/design-system.spec.ts` on both engines |
| isolated Desktop resource view and reusable components in Resources/Actions | same view and catalog rules | same view and catalog rules | same view; existing tool capabilities unchanged | `src/resources/adapters.test.ts`, `e2e/settings.spec.ts`, import-boundary fixtures; gallery runs without provider initialization |
| Desktop component catalog, chat and Git extraction | same canonical chat blocks and existing capabilities | same canonical chat blocks and existing capabilities | same components; unsupported requests remain unsupported | `src/components/catalog.test.ts`, existing Git, critical-flow and Markdown tests; isolated chat/Git stories have no dedicated E2E scenario and use no providers |
| the company's executable DS components | independent of the provider | independent of the provider | shared application behavior | `e2e/design-system.spec.ts`, menu, submenu, password, focus, validation and error on both engines |
| Code review included and editable | the initial profile; the model/provider can be changed | can be chosen in the profile | Adapted; see verification boundary | `actions.rs`, `actions.test.ts` for defaults/removal; `e2e/actions.spec.ts` for profile editing |
| prompt commands and tasks | adapted by the app | adapted by the app | Adapted; see verification boundary | `actions.test.ts`, `actions.rs`; `e2e/actions.spec.ts` retains prompt editing |
| per-task profile | instructions and permissions through flags | instructions and permissions through JSON-RPC | Adapted; see verification boundary | `session.rs` and `codex.rs` tests |
| detecting the installation | adapted | adapted | version checked (minimum 1.2.7) | `agents.rs` |
| accounts and global selection in the footer | adapted through `CLAUDE_CONFIG_DIR` | adapted through `CODEX_HOME` | single external agy account, explicit attachment | `accounts.rs`, `e2e/accounts.spec.ts` |
| grouped workspace creation and readable account controls | shared fields; plan mode available | same layout with model search and native effort | same layout; unsupported plan, effort and tool controls stay absent; attached agy profile shows attachment status | `e2e/launcher-layout.spec.ts`, `e2e/accounts.spec.ts`, `e2e/model-picker.spec.ts`; English, narrow windows, Chromium/WebKit |
| removing every account and an empty selection | supported; preserves the CLI's login | supported; preserves the CLI's login | supported; preserves external login | `accounts.rs`, `e2e/accounts.spec.ts`; directories and credentials stay local |
| login through the app | injected native `auth login` and browser | injected native `account/login/start` and browser | attachment only; official interactive agy login | `crates/core/src/accounts/login/tests.rs`: lifecycle ordering, cancellation, revision/error/busy guards; identity fixtures in `claude.rs` and `codex/account.rs`; OAuth with two real accounts still requires manual validation |
| switching accounts between turns | resuming the shared transcript | resuming the shared rollout/index | Unavailable; externally managed identity | `crates/core/src/session/tests.rs`, profile tests; an authenticated continuation between two real accounts is not yet proven by the suite |
| turn cut off by quitting or updating | shown as interrupted after restart, with Continue | same | same | `board.rs::tabs_working_when_the_app_ended_are_marked_interrupted`, `session/tests.rs::an_accepted_message_clears_the_interrupted_marker`, `stream/tests.rs::a_snapshot_names_the_process_presence_last` and `a_recorded_cut_stays_in_replay_after_continuing_and_reloading`, `src/timeline.test.ts` (a conversation whose process is gone); the notice and Continue were checked in the browser mock (`mock:interrupted`) and have no E2E scenario |
| asking before quitting, closing the main window or restarting to update while local work would be cut off | working and waiting conversations count, independent of the CLI | same | same | `src/quit.test.ts`, `src/update.test.ts`, `src/appmenu.test.ts`, `crates/process/src/terminal/tests.rs` (a command running as a job or through `exec`) and `dock.rs` (`cuts_off`); the native menu item, window close and relaunch need a macOS app run |
| notch island: follow, approve, answer and reopen | approvals with Edit diffs, plans and questions through the shared request cards | approvals for commands and file changes without a diff; questions; no plans | shared rows and quota; requests follow the adapter's canonical events | `src/island.test.ts`; native geometry, hover and answering need a macOS app run; closing the main window with the island on quits the app, checked with a macOS dev build because Tauri run events cannot be built in a unit test |
| quotas per account | live stream and internal endpoint; selected account has a shorter native polling budget | live app-server event, bounded separate app-server probe and internal fallback | native `/usage` child; used and remaining by model group; completion uses an exit notification rather than a 20 ms loop | `usage.rs`, `usage_scheduler.rs` controlled-clock/generation and cancelled-login resumption tests, `antigravity.rs` bounded-child test, `statusbar.test.ts`, `e2e/accounts.spec.ts`; real provider launch counts still need a controlled native run |
| live model catalog | native `list_models` control request | native paginated app-server `model/list`, including additional models | native `agy models` | `agents/catalog.rs`, `agents.test.ts`; per-agent errors and stale state |
| starting a session | stream-json adapter with shared workers | JSON-RPC adapter with shared workers | NDJSON adapter with shared workers; real text smoke passed | `crates/core/src/session/workers/tests.rs`: initialization, output/exit ordering, filtering and scheduling failure; native process tests and existing provider fixtures |
| provider preparation and input | stream-json; read-only native history; input may arrive during a turn | JSON-RPC; canonical app log; closed writer releases the pipe while reader state survives | NDJSON; canonical app log; idle-only messages and process interruption | `agent_launch.rs` preparation/resume/store tests without Claude/Codex binaries; all three provider `canonical_input_*` tests; existing native argument fixtures |
| resuming a session | transcript existence, otherwise fresh with the same ID | saved app-server thread, otherwise fresh | saved native conversation ID, otherwise fresh | `crates/core/src/session/launch/tests.rs`: preparation/spawn failure, queue retention, publication/readiness and permission overrides; existing `session.rs` and provider fixtures retain launch compatibility |
| model search and favorites | same picker, native labels | same picker; additional models opt-in | same picker, separate provider identity even for overlapping IDs | `e2e/model-picker.spec.ts`, `e2e/search-picker.spec.ts`; Chromium/WebKit |
| workspace tools with tab model/effort overrides | same selections for new and resumed tabs | same selections for new and resumed tabs | shared application behavior | `session.rs::new_and_resumed_tabs_preserve_workspace_tools_with_model_overrides` |
| tools in mixed-provider tabs and actions | CLI base follows the effective Claude provider and configured home | hub-only universe even in a Claude workspace | Adapted; see verification boundary | `session.rs::tool_resolution_uses_the_tab_provider_and_configured_claude_home`, `e2e/tools.spec.ts` |
| tool reset and project trust | null restores inheritance; decisions bind to the displayed hash, including empty declarations | same contract | Adapted; see verification boundary | `session.rs::tool_axis_ipc_preserves_absent_null_and_replacement`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes`, `e2e/tools.spec.ts` |
| ordered prompt, transcript, and live delivery | shared portable pump and conversation mutex | same pump with JSON-RPC translation at the edge | same pump with NDJSON translation at the edge | `crates/core/src/session/pump/tests.rs`: capture order, deferred reactions, scheduling, private usage, retirement and failed writes; concurrent delivery and fast-response regressions in `chat.rs` |
| choosing a model | native through a flag | adapted in `thread/start`/`thread/resume` | native --model slug | `session.rs`, `codex.rs` |
| effort levels | only advertised native levels plus provider default | only advertised native levels; legacy ultracode maps to ultra | no effort control without advertised levels | `model-choice.test.ts`, `e2e/model-picker.spec.ts` |
| conversation footer adapted to the width | model and activity separated from the tools; remote control highlighted when active | the same UI | Adapted; see verification boundary | `e2e/composer.spec.ts`, Chromium/WebKit, English and narrow frames |
| dictation in the message box | independent of the CLI; WebKit recognition | independent of the CLI; WebKit recognition | shared application behavior | `voice.test.ts`; a real microphone needs a manual smoke test in the bundled app |
| initial plan mode | native through permission mode | unavailable | Unavailable | `session.rs`, `launcher.ts` |
| streaming text | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `conversation.test.ts`, `timeline.test.ts`, `codex.rs` tests |
| thinking | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `timeline.test.ts`, `codex.rs` tests |
| tool call and result | adapted to V1 | adapted to V1 | Adapted; see verification boundary | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| questions to the user | native | adapted from a JSON-RPC request | Unavailable | `chat.ts`, `codex.rs` tests |
| approval requests | native | adapted | Unavailable | `chat.rs`, `codex.rs` |
| interruption | control request | `turn/interrupt` | Adapted; see verification boundary | `codex.rs`, Rust tests |
| compaction | the CLI's command | `thread/compact/start` | Unavailable | `codex.rs` tests |
| context report | stream/transcript | synthesized from token usage | Unavailable | `context.test.ts`, `codex.rs` tests |
| MCP selection per workspace | the CLI's strict config | table and environment assembled by the app | Unavailable | `mcp.rs`, `codex.rs`; a preparation error prevents the spawn |
| MCP connection checks and local OAuth, including Cloud definitions | shared hub actions; token supplied at process launch | same hub actions and local token | MCP execution unavailable | `e2e/mcp.spec.ts` simulates checks, login retry, logout and independence from Cloud revisions; real browser consent requires manual validation |
| plugin selection per workspace | session flags | marketplace + config isolated per workspace | Unavailable | `plugins.rs`, the CLI smoke test, `codex.rs`, `launcher.ts`, E2E |
| install a skill-bundle or bare `skills/*/SKILL.md` Git repository | synthesized portable package through `--plugin-dir` | same package through the derived marketplace | Unavailable | `crates/tools/src/plugins.rs` skill-bundle and bare-skill tests, desktop `plugins.rs` discovery; [ADR 0098](../decisions/0098-skill-bundle-install.md); installation does not activate automatically |
| local and account skills | a package with SKILL.md through the plugin selection | the same package with a native manifest | Unavailable | `crates/tools/src/skills.rs`, `catalog.rs`, `e2e/cloud.spec.ts`; installation does not activate automatically |
| start a conversation from a skill with the declared artifact path | package added to the first conversation's resolved `--plugin-dir` set; opening line in the first message | package added to the derived marketplace of that conversation; same opening line | Unavailable; no hub tool selection | `kickoff.rs` tests, `session.rs::first_message_opens_with_the_kickoff_line`, `session.rs::artifact_path_follows_the_primary_repository`, `scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented`, `src/kickoff.test.ts`; plugin-shipped skills must come from a local folder; the method is not re-announced to later tabs; a resume whose skill was removed continues without it and warns |
| layered selection (global, project, workspace) per axis | resolved at spawn before the adapter | resolved at spawn before the adapter | Adapted; see verification boundary | `selection.rs` resolve tests, `session.rs::provenance_classifies_each_hub_item`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes` and `session.rs::tool_axis_payload_is_validated_before_persistence`; the project `[tools]` layer is gated on trust-on-first-use of its hash, an undecided item stays `pending` and a rejected one stays `rejected` (both resolved yet not injected), and the setters refuse a malformed payload or an id on the wrong axis |
| CLI-inherited MCP base (ADR 0046 and ADR 0063) | discovered from `~/.claude.json` and the working directory's `.mcp.json` plus its ancestors, nearest first, plus the active account's claude.ai connectors; visible in the picker with the `cli` provenance and removable as a workspace delta; a declared axis materializes the whole effective set through the strict config, connectors included as `claudeai-proxy` entries | no discovered base; the CLI keeps loading its own configuration outside the picker | Unavailable | `selection.rs::cli_base_participates_in_the_chain`, `mcp.rs` inherited/universe tests, `mcp.rs::account_connectors_join_the_inherited_base` and `mcp.rs::missing_selected_servers_prevent_materialization`, `session.rs::provenance_classifies_inherited_cli_configuration`, `src/mcp.test.ts`, `src/tool-picker.test.ts`, mixed-provider picker, long-base scrolling and inheritance reset in `e2e/tools.spec.ts` |
| hooks of a chosen plugin | active from `SessionStart` | `enabled = true` + trust limited to the `pluginId` and hash before the thread | Unavailable | `codex.rs` tests; a failure prevents the thread |
| attachments in a message, capture thumbnails and pasting | adapted through a local path; promise and pasteboard materialized by macOS, pasted image by GTK on Linux | adapted through a local path; promise and pasteboard materialized by macOS, pasted image by GTK on Linux | Adapted; see verification boundary | `file_drop.rs`, `chat.ts`, `paste.ts`, `tree-menu.ts`, `changes-menu.ts` and the shared `file-menu.ts`; `e2e/file-drop.spec.ts`, dropped-file scenarios and the file tree menu in `e2e/critical-flows.spec.ts` cover the UI over the mock; `paste.test.ts` covers the paste detour, including the empty WebKitGTK clipboard; `paths.test.ts` covers reading sent attachments back as numbered image tags, shared by both agents; `tree-menu.test.ts` covers the menu an agent without attachments receives, `changes-menu.test.ts` the same groups on a changed file |
| the browser's visual context | a tag in the draft and the history; complete HTML, CSS, URL and PNG mention on send | the same interface and textual contract | Adapted; see verification boundary | `browser-context.test.ts`, `e2e/browser-inspector.spec.ts`, `e2e/browser.spec.ts`, `browser.rs` tests; WKWebView capture and the AppKit gesture still require native verification |
| unknown external event | ignored by the adapter | ignored by the adapter | Adapted; see verification boundary | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| the CLI's subagents | a sidechain off-screen; tasks in `background.changed` | `collabAgentToolCall.agentsStates`, `subAgentActivity` and known child events update the tasks; the children's content stays isolated | no child session signal; the turn settles on its terminal | `claude.rs`; isolation, spawn, activity and partial-state tests in `codex.rs` |
| a turn that ends while its subagents run | tab, Stop button, queued input, delegation execution and notice all wait for the drain ([ADR 0056](../decisions/0056-background-tasks-hold-completion.md)) | the same rule over the same normalized signal | settles on the terminal, since no task is ever reported | `crates/core/src/conversation/work.rs::work_tests`; each adapter's real output drives the same rule in `claude.rs` and `codex.rs`; held-completion and interruption tests in `delegation.rs`; `src/timeline.test.ts`, `src/alert.test.ts`; Stop control in `e2e/composer.spec.ts` |
| local notifications and Dock attention indicators | completion, error and request events; silent until opt-in | same rule over canonical live events | completion/error over canonical events; interactive requests unavailable | `src/alert.test.ts`, `src/notifications.test.ts`, `e2e/notifications.spec.ts`; native verification limits in [notifications](../contracts/notifications.md) |
| live sharing | V1 after normalization | V1 after normalization | shared application behavior | `team*.test.ts`, E2E over the mock |
| remote control from the owner's devices | the same relay v4; execution stays local | the same relay v4; execution stays local | shared application behavior | `team-channel.test.ts`, `team-organizations.test.ts`, `e2e/organizations.spec.ts` |
| comments in a shared session | adapted after V1 | adapted after V1 | shared application behavior | `notes.test.ts`, `team.test.ts`, `relay/src/logic.test.ts`, E2E over the mock |
| desk with several conversations at once | adapted (the same conversation screen) | adapted (the same conversation screen) | shared application behavior | `desk.test.ts`, E2E over the mock |
| streamed conversation painting and hidden-window catch-up | shared V1 presentation; subagent work state still updates the composer | same V1 presentation and work-state rule | same presentation; no native child-task signal | `src/timeline.test.ts`, `e2e/critical-flows.spec.ts` in Chromium/WebKit for streaming and hidden catch-up, `e2e/markdown.spec.ts` for final Markdown and copy; WebKit timing limits in [energy profile](energy-profile.md) |
| sleep preference during local agent work | `agent` keeps display awake through native child-task settlement; macOS `agent-system` keeps only the system awake | same rule over Codex's normalized child signals | same choices, but no native child-task signal to extend a settled turn | `src/statusbar.test.ts`, `awake.rs` command/transition tests, [ADR 0061](../decisions/0061-background-energy-policy.md); AC `pmset` check only, battery unavailable |
| Git: unified/side-by-side review, stage, discard, commit, remotes, branches, conflicts and the changed-file menu | adapted by the app; independent of the CLI | adapted by the app; independent of the CLI | shared application behavior | `crates/git/src/tests.rs`, `diff.test.ts`, `changes-menu.test.ts`, `e2e/git.spec.ts` in Chromium/WebKit and a large review in `e2e/critical-flows.spec.ts`; the `git.md` contract |
| Worktree from an existing branch | app creates the worktree before starting the CLI | same behavior | shared application behavior | `session.rs::new_branches_start_from_the_selected_base` verifies local and remote refs; `e2e/launcher-layout.spec.ts` exercises the selector; [Git contract](../contracts/git.md) |
| Markdown file reader, source editor and PDF, CSV, image preview | shared file viewer; independent of the CLI | shared file viewer; independent of the CLI | shared application behavior | `e2e/critical-flows.spec.ts`; SVG retains source editing and gains preview from the draft; raster preview uses existing `read_bytes` IPC and browser image decoder |
| find in the open file and quick open by name | shared file viewer and `find_paths`; independent of the CLI | shared file viewer and `find_paths`; independent of the CLI | shared application behavior | `find.test.ts` for matching, smart case, wraparound and marker markup; `crates/files/src/search.rs` for path ranking and host cache isolation; the viewer find bar and Command-P palette have no dedicated E2E gate |
| scoped worktree cleanup after archiving or finishing | independent of the CLI | independent of the CLI | shared application behavior | `e2e/audit-regressions.spec.ts`, `session.rs` cleanup tests; blocked worktrees require explicit force selection |
| agents per workspace in the sidebar | each tab's brand and status | each tab's brand and status | Adapted; see verification boundary | sidebar flows in `e2e/critical-flows.spec.ts`; a remote one uses the owner's avatar, without inferring the provider |
## Rule for a new feature

Before enabling a feature for a provider:

1. declare its common semantics in the contract;
2. add or adjust the capability in the descriptor;
3. capture a real CLI fixture without secrets or personal data;
4. prove the translation into common events;
5. cover common semantics in the reducer; add browser coverage only for a core
   UI journey or a concrete exception under the E2E scope policy;
6. update this matrix with the path to the evidence.

A missing test must not become `true` by similarity between providers.

## Known limitations

Desktop distribution is independent of the provider. macOS Apple Silicon and
Linux x86_64 AppImage share one release. macOS and AppImage have signed in-app
updates for Claude, Codex and Antigravity. Other Linux installation types keep
external updates.
Evidence: `scripts/test_release.py`, `src/update-init.test.ts` and
`src/update.test.ts`; native installation and replacement remain manual checks
under the [release contract](../contracts/releases.md).

- a plugin source in a local `.zip` or a URL works in Claude and is refused with
  a visible error in Codex; a local folder is the portable format;
- skills, commands, MCP and hooks have a portable adaptation; `agents/*.md`
  stays exclusive to Claude because it is not part of the current Codex
  manifest;
- plugins enabled outside Prometeu stay subject to each CLI's global registry
  and are not part of the workspace's selection;
- the CLI-inherited MCP base (ADR 0046 and ADR 0063) covers Claude only;
  servers configured for Codex in `~/.codex/config.toml` are not discovered and
  stay invisible to the picker, a recorded follow-up;
- the account connectors are read from an endpoint Prometeu does not own and
  are cached per login for five minutes, so an edit made on claude.ai can take
  that long to reach the picker, and an unreachable account keeps that login's
  last known list; with no list ever read, the picker shows the file base and a
  spawn with a declared MCP selection that inherits the base fails instead of
  dropping connectors; a `base: "none"` replacement can start independently
  unless an explicitly added account connector lacks its configuration;
- attachments have UI tests over the mock and native validation of the saved
  destination; the real thumbnail gesture was confirmed in Prometeu Dev on
  2026-09-06. Actual reading by the CLI still requires manual verification.
  Promises that fail or exceed 30 seconds produce a visible error.

## Desired conformance suite

| Scenario | External fixture | Adapter | Adapted; see verification boundary | E2E |
| --- | --- | --- | --- | --- |
| simple message | per provider | required | Adapted; see verification boundary | smoke |
| streaming + final message | per provider | required | Adapted; see verification boundary | critical |
| successful tool | per provider | required | Adapted; see verification boundary | critical |
| tool with an error | per provider | required | Adapted; see verification boundary | critical |
| question and answer | per provider | required | Unavailable | critical |
| interruption | per provider | required | Adapted; see verification boundary | smoke |
| resume | per provider | required | Adapted; see verification boundary | critical |
| compaction | per capable provider | required | Unavailable | smoke |
| background task | per capable provider | required | Adapted; see verification boundary | smoke |
| unknown event | synthetic | required | Adapted; see verification boundary | not needed |

E2E does not replace the contract: it covers a few expensive paths. Fixtures and
reducers give fast diagnosis for every combination.

## Sub-workspace navigation

Claude, Codex and Antigravity workers share one-level grouping, a provider mark
and one conversation per delegated workspace. Cursor can coordinate these
workers; Cursor worker creation remains unsupported. External MCP clients have
no local parent workspace. Remote shares keep flat navigation.

Evidence: `src/workspace-hierarchy.test.ts`, delegated-conversation tests in
`crates/core/src/board.rs`. Legacy multi-tab
workspaces retain their tabs. [ADR 0099](../decisions/0099-sub-workspaces.md)
describes flattening without changing provider or MCP ownership.

## Built-in delegation MCP

Claude, Codex and Cursor can opt into the bundled `prometeu` MCP and create a
Claude, Codex or Antigravity worker in an isolated workspace; Cursor is not a
worker provider. Every coordinator must name the worker's `provider`; none
inherits its own. `list_agents` reports each worker provider's live catalog, and
`provider`/`model`/`effort` choices are checked against it before the
workspace exists, so a model from another provider is rejected with a hint
instead of reaching the wrong CLI. Antigravity workers have no approval or
question prompts and run with automatic execution unless the copied action
profile is Ask. Ownership, execution IDs, busy-send
rejection, file access and canonical history share the backend implementation.
Claude receives a private strict MCP file; Codex receives the stdio command
and private environment wrapper; Cursor receives the same stdio command and
environment through ACP `mcpServers`. Workers start with empty MCP/plugin/skill
selections. Background reporting is limited to each adapter’s existing
`background.changed` signals; unknown is distinct from an observed empty set. An
execution holds `running` with its outcome recorded until those tasks drain, so
it never reports completion while sends are still rejected as busy.

Both providers also use the same configured setup/Run controls, bounded dock
logs, process/exit status and explicit desktop preview opening. These tools
accept only an owned `agent_id`; there is no provider-specific execution path
or arbitrary shell command/URL argument. Setup continues to run on creation.
Runtime status is ephemeral and does not imply HTTP readiness.

Evidence: `delegation.rs` and `embedded_mcp.rs` unit tests, built-in
materialization tests in `mcp.rs`, and the opt-in picker test in
`e2e/tools.spec.ts`. Workspace controls and log limits are tested in
`delegation.rs`/`embedded_mcp.rs`, real PTY exit/output retention in `crates/process/src/terminal/tests.rs`, and
preview navigation in `e2e/browser.spec.ts` on Chromium, with selected browser
interactions repeated on WebKit.
Browser tests use the catalog mock and do not run models.
Live model delegation requires installed/authenticated CLIs; automated tests
do not spend model credits. See the [contract](../contracts/embedded-mcp.md).

External local MCP hosts use the same stdio protocol and tools with any worker
provider, without a coordinator conversation in Prometeu. Registered clients have
explicit project scope and revocable credentials that survive desktop restart.
They also name the `provider` (Claude, Codex or Antigravity), which starts from
its CLI defaults unless `model`/`effort` are passed.
Evidence: `mcp_access.rs` and `delegation.rs` authorization/compatibility tests,
and `tests/mcp_client.rs` for real stdio subprocess registration and socket
rediscovery against a fixture. No live provider or native webview is exercised
by that subprocess test. See [registration](../contracts/embedded-mcp.md#external-client-registration).


## Antigravity verification boundary

The integration targets installed agy 1.2.7, whose native headless protocol is
separate from Gemini CLI ACP. A real cached-account prompt returned `AGY_OK`;
`agy -p /usage` returned real remaining quotas by model group, with a TSV fixture
and parser regressions. `agy models` returned 14 account-visible slugs. Real tool execution, restart/resume
with a remembered marker, and SIGINT passed. Adapter fixtures and tests cover
text deltas, tools, completion, interruption, explicit native resume and the
`--add-dir` worktree argument on both creation and resume. Future real-runtime
verification must assert the tool's working directory, not only tool success.
The browser mock consumes only the generated V1 fixture.

Prometeu cannot offer agy interactive approval responses, managed multiple
Google accounts or a verified direct browser-login API. Initial plan
mode and hub selections are unavailable. The original issue's complete
account/approval scope is not concluded by this replacement. External login and
account switching remain owned by agy; no isolated-profile guarantee is made.
Former Gemini accounts and transcripts are preserved and cannot launch another
runtime accidentally. See [ADR 0052](../decisions/0052-antigravity-runtime.md).

## Local diff review notes

| Behavior | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Line/range/file notes, local persistence, draft batch and portable history | Shared application behavior | Shared application behavior | Shared application behavior |

The [review contract](../contracts/diff-review.md) uses ordinary text with no new
provider capability, IPC or V1 event. `review-comments.test.ts`,
`review-context.test.ts` and `review-store.test.ts` cover rules and compatibility;
`e2e/git.spec.ts` and `e2e/mobile.spec.ts` cover representative keyboard/reload
and portable history behavior. These tests do not assert that a provider follows
every note or that a queued batch has been received by the model.

## Cursor verification boundary

Cursor is an additional desktop provider; the preceding Claude/Codex/Antigravity
comparison retains its existing scope. Cursor uses the same Git, worktree,
transcript, sharing and presentation paths after V1 normalization.

| Capability | Cursor | Evidence |
| --- | --- | --- |
| Installed CLI and model discovery | ACP capability probe; exact account-visible ACP model IDs | `cursor.rs::installed`, `protocols::cursor::query_catalog`, ignored live smoke |
| Account | One explicitly attached external login; separate selection; removable | core account round-trip test; shared account UI |
| Streaming, thinking, tools and errors | Native ACP translated to V1 | recorded `cursor/fixtures/tool.ndjson`, protocol tests, `src/conversation.test.ts` |
| Resume and interruption | Native load with replay suppression; ACP cancellation; failed handshake disconnects so next input restarts | protocol tests, including failed-handshake pipe release; ignored desktop `live_cursor_round_trip` |
| Permissions, questions, plan approvals | Existing request cards; option IDs independent of click order and explicit plan decisions | protocol tests include reversed multi-select and free text; live smoke covers permission round trip and initial plan mode |
| Attachments | Existing local path mentions; no binary upload | shared presentation contract; Cursor image understanding not separately verified |
| MCP from Prometeu | Hub selection encoded as ACP `mcpServers` on new and resumed sessions; Cursor's own configuration still applies | `tools` ACP encoding tests, `session::tests::tool_resolution_uses_the_tab_provider_and_configured_claude_home`, `agent_launch` Cursor tool injection; native Cursor configuration still applies |
| Plugins and skills from Prometeu | Unavailable; absent workspace axes suppress inherited packages; explicit nonempty selections fail | same session test covers plugin/skill suppression and rejection |
| Account quotas | Dashboard API using the CLI login; unified weekly or explicit legacy individual monthly limits | `cursor/quota/tests.rs`: mapping, failure and compatibility tests; ignored `live_cursor_quota` verifies the signed-in account |
| Per-turn token/cost meters, context and compaction controls | Unavailable | descriptor capabilities; no invented readings |
| Subagent lifecycle | No separately verified background signal | native prompt terminal settles turn |
| Native terminal recovery (macOS) | The host restores missing native tails before seeding history and commits live batches before publication; native IDs survive derived JSONL rewrites; successful completion requires the exact request, a private pre-ACK witness, post-hooks marker and empty composer; unprojected content prevents coverage | `protocols::cursor_store` synthetic protobuf/projection tests, `terminal_agent_cursor_store` copied DB/WAL tests, `terminal_agent_observation/cursor/tests.rs`, pre-ACK hook tests and `chat/terminal/ingestion/tests.rs`, verified on CLI `2026.11.13-eacade1` and accepted on newer builds; hook-authorized scratch store and request identity smokes |
| Native terminal interruption (macOS) | An authenticated root aborted hook matching the durable source request commits `interrupted`; a restored draft does not keep the spinner running; later source/tool activity or a completed hook for that request cannot reopen or successfully complete it; new prompts invalidate older proof | `cursor_interruption_*` observer regressions and `journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet`; pinned native Ctrl+C smoke and local Escape source inspection above |
| Native terminal running feedback (macOS) | A newer verified busy composer after a native submission boundary starts observed Working before source persistence; source capture claims the same cycle, and old paints cannot reopen a settled cycle | `cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`, `cursor_working_observation_requires_its_current_complete_busy_composer`, `observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once` |

Native-terminal preparation resolves the workspace once and shares the physical
cwd between the PTY, Cursor workspace argument, transcript slug and store hash.
`preparation_captures_profiles_and_resume_state_without_starting_agents` covers
symlink traversal on fresh/resumed launches. This fixes the actual Tauri scratch
case where `/tmp` and `/private/tmp` selected different stores despite a native
response; the path regression itself does not claim another model or UI run.

Native verification targets Cursor CLI `2026.10.22-27dcf5f`. A real scratch
workspace prompt ran `pwd`, wrote `CURSOR_OK`, and remembered a marker across
process restart. ACP cancellation returned `cancelled`. Fixtures test question
and plan decisions; no manual native-webview interaction is claimed. Existing
browser account/launcher journeys exercise the shared controls without adding
a provider-by-provider E2E matrix. See [ADR 0095](../decisions/0095-cursor-acp.md).
