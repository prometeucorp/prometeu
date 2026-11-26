---
type: Quality
title: "Provider matrix"
description: "The features that each agent can use, and the tests that show them."
status: stable
---

# Provider matrix

This page shows the current behavior of the code. "To confirm" means that the
feature can be available, but the tests do not show sufficient conformance at
this time. Thus it is not a capability in the contract.

Browser tests obey the [E2E scope policy](../operations/development.md#e2e-scope).
Chromium runs the scenarios that the project keeps. Only the scenarios with the
explicit `@webkit` tag run again in WebKit. When a row names the two engines, it
means representative coverage. It does not mean all combinations of providers,
states, languages and screen dimensions.

## Remote machines

The flow that connects a remote MCP client can prepare the delivery of events.
In the same flow, it can do a test of the path from the host to the receiver.
This function operates independently of the receiver format (OpenBot or
Prometeu). It does not add providers that can run on the machine. When the flow
adds events, the current MCP tokens do not change. `src/machine-supervision.test.ts`,
the native `remote/supervision.rs`, the host supervision tests and the Cloud
test `supervision_connections_test.rb` examine consent, credentials and delivery
receipts. These tests do not examine a live deployment or the MCP installation on
the receiver.

The optional [workspace supervision](../contracts/supervision.md) publishes
metadata from the remote Claude host to the Cloud and to general receivers of
signed webhooks. Codex, Cursor and Antigravity do not publish supervision events.
The selection of a local supervisor and the MCP transport in the opposite
direction are future work. The core supervision tests show completion that
includes child work, the removal of duplicates and reconciliation after the
process starts again. `crates/host/src/supervision.rs` and `crates/host/tests/agents.rs`
examine persistence, publication and the MCP path without calls to a live
provider. The Cloud tests examine ingestion for each account, GitHub routing and
callback delivery.

Settings shows a list of the processes that the host owns, and it can stop them.
These processes include idle conversations and Claude sign-in terminals. This
function adds a guarded stop to the terminal protocol of the host. It does not
change which providers run on a remote machine. `src/machine-processes.test.ts`
examines liveness, labels for each machine, confirmation races, earlier host
versions and the refresh lifetime.
`stopping_a_listed_process_never_kills_a_replacement_under_the_same_id` makes
sure that an expired confirmation cannot stop a resumed conversation.
`terminals_survive_detachment_and_replay_the_gap` in the host tests examines the
list before and after the host stops one specified process.

More than one computer can follow the same machine at the same time
([ADR 0104](../decisions/0104-several-computers-on-one-machine.md)). This
function changes how computers attach to the host. It does not change the
providers that the host can operate. The host shares hooks and permission
requests when Claude sends them. Each computer that follows a Codex conversation
reads the engine through a relay terminal of its own
([ADR 0105](../decisions/0105-codex-on-remote-machines.md)). The host
integration tests in `src-tauri/crates/host/tests/host.rs` and
`src/machine-clients.test.ts` examine this function.

Claude and Codex declare `remoteMachines`. The remote host sends the native
hooks of Claude to the desktop. It also makes a mirror copy of the project
transcripts of Claude for the observer
([ADR 0101](../decisions/0101-remote-machines.md)). Codex runs its native engine
and TUI on the machine. The desktop reads the RPC of the engine through a relay
terminal on the host ([ADR 0105](../decisions/0105-codex-on-remote-machines.md)).
Prometeu rejects Cursor and Antigravity for remote workspaces until their hooks
and stores can run on the host.

The two remote commands are the same as the local terminal commands, but they
have no arguments for the account profile, MCP or plugins. The login and the
configuration of the machine apply. Settings signs Claude in on the machine with
`claude auth login --claudeai`, the browser of the person and a code that the
person pastes. The opt-in `vps_claude_login` test examines the link with Claude
Code 2.1.330. The person signs Codex in on the machine
(`codex login --device-auth` through SSH). Settings and the launcher show if
Codex is installed, if its version is 0.169.0 or newer and if it is signed in.
If one of these conditions is not true, the launch stops and tells the necessary
step.

| Remote behavior | Claude | Codex | Evidence |
| --- | --- | --- | --- |
| observation | hooks that the host sends, and a mirror copy of the transcript | the engine RPC through the `codex-relay` terminal of the host | `host.rs` relay tests, `wire.rs` tests |
| sign-in | Settings runs `claude auth login` on the machine | the person runs `codex login --device-auth` on the machine. Prometeu shows only the status | `remote/codex.rs` tests, `src/machines.test.ts` |
| SSH loss | the host replays the terminal and the hooks | the host replays the relay lines. After a gap of more than 2 MiB or a relay that stops, the desktop takes control of the conversation again from the engine history | `remote/codex.rs` tests, `host.rs` detachment test |
| app restart | adopts the TUI that runs and the transcript binding | adopts the TUI and the engine that run, with a new relay | no test on a physical machine |
| exit while away | adopts the stopped terminal to collect the output and the transcript | forgets the stopped TUI. Stops the orphan engine at the next attachment | `remote/codex.rs` tests |
| agent clients (ADR 0103) | in the list, and can resume | not in the registry of the machine | `remote/registry.rs` tests |

Settings > Projects manages the project registration and filters it by computer.
The launcher shows only the registrations on the selected machine. Local origin
aliases do not hide remote clones. They also do not cause an installation that
the person did not request. `src/machines.test.ts` examines this selection rule.
`src/settings-navigation.test.ts` examines the navigation compatibility.
`e2e/projects.spec.ts` examines the browser journey through the machine selection
and the nested registration, and the workspace prompt stays. Remote execution is
available for Claude and Codex.

Agent clients on the machine
([ADR 0103](../decisions/0103-agents-on-remote-machines.md)) start only Claude,
without permission prompts. `src-tauri/crates/host/tests/agents.rs` examines
them. The desktop adopts their workspaces as Claude terminal tabs that run in
that mode. `src-tauri/src/remote/registry.rs` examines the adoption. Settings
enables the agent clients on a connected machine. It shows a general local stdio
MCP configuration that all clients can use.

The integrator prepares the container and the network.
`the_generated_local_config_lists_tools_and_relays_calls` in the host
integration tests runs the generated configuration. The host unit tests examine
that the home permissions do not change, and the cleanup of earlier files. This
function changes the client integration. It does not change the providers:
execution continues to start only Claude.

Tests: the host integration tests and the opt-in `vps_journey` examine
transport, reattachment and limits. This page does not record a live Claude
response, completion or reattached status on a remote machine at this time. No
test ran remote Codex on a physical machine with Codex 0.169.0 or newer,
installed and signed in. Thus this page has no record of these items:

- A new response.
- An interrupt.
- A request answer.
- A reconnection.
- A new start of the app.

`remote_reattachment_before_first_prompt_preserves_identity_and_requires_empty_history`
in `src-tauri/src/chat/terminal/ingestion/tests.rs` examines this condition: the
desktop starts again after Claude binds its native identity, but before its
first transcript is available. Adoption keeps that identity and the status
metadata. Local resumes, current transcripts, captured work and prepared input
continue to use the strict check of the history.

## Native conversation mode

Claude, Codex, Antigravity and Cursor use the same optional format for the
conversation mode. The format keeps a global preference for new conversations.
It also keeps the fixed mode of each tab independently, and action tabs are
included. On macOS, **Settings → General → New conversations** selects Chat or
Terminal for new conversations. A change to this setting does not change current
tabs and does not start their processes again. Linux does not show the control
and rejects terminal mode explicitly.

The local Chat view and the local Terminal view are exclusive. Terminal tabs do
not have a transcript screen or a surface switcher. Canonical persistence,
status, notifications and remote transcript sharing operate independently of that local
presentation. The native terminal fills the conversation area. It has no
permanent footer and no floating controls.

These menus open the shared controls when the person selects them:

- The workspace menu at the right of the central tab bar.
- The context menus of the tabs.
- The menus in the desk header.

The shared controls are actions, MCP, plugins, skills, context, dictation and
remote access. Quotation and manual resume are actions in the context menus. A
collaboration notice stays on the screen while the owner must answer it.

Changes to the model and the effort use the native CLI. The launch defaults stay
available. When a person selects, drops or pastes files, Prometeu puts their
paths directly into the native draft. It does not send Enter. Browser context,
review notes and text previews stay staged in the tools dialog until the person
puts them in the draft. The CLI has the only local message input that a person can edit.

If an insertion is not successful, the draft of the initial tab stays. Prometeu does not
resume a process and does not move the insertion to a different tab. The default
Open PR action also writes its request into the native composer as a draft, and
the person submits it. The action does not put the request in the queue as app
input. If the agent stopped, Prometeu resumes it with the request in the queue
(`e2e/composer.spec.ts`).

A conditional indicator shows queued local prompts and staged context. The
indicator is in the action group of the tab bar or in the desk header. It is
different from the agent status. The tools dialog shows their preview for all
providers.

On macOS, native typography uses 14 px SF Mono with monospace fallbacks, 500/700
weights and a line height of 1. The neutral palette uses `#282c34` and white
text. The shared defaults of the helper keep OSC reports and replay aligned.
Native color changes from before stay after restoration. Helpers that run keep
their current colors until the usual replacement.

The versions of the renderer and the helper stay pinned to xterm 6.0.0. The
desktop view draws with the WebGL addon. If WebGL is not available, it uses DOM
rendering. No automated test examines GPU rendering. `e2e/composer.spec.ts`,
`src/term.test.ts` and the replay and query tests of terminal-state examine
geometry and input. Browser tests are different from conformance tests of the
native macOS app.

For all terminal providers and for the terminals that can show them, OSC 8
HTTP(S) hyperlinks open in the system browser. Prometeu does not find plain text
URLs that have no OSC 8 markup. `src/term.test.ts` examines how external IPC goes to the system,
rejected schemes, hover text for the destination and errors when a link opens.
It also examines the UI callbacks of the two terminals, with mocked xterm and
IPC.

The native conversation scenario in `e2e/composer.spec.ts` examines the mouse
propagation of the xterm library in Chromium and WebKit. A primary click on a link
opens the link one time and does not go to the PTY. Clicks that are not on links
continue to go to the PTY. This shared behavior prevents a second opening by
CLIs that use the mouse, for example Codex. A check in the macOS app is necessary
for the native browser launch.

The four local terminal providers use the same automatic resume of the saved
session when a person opens a stopped tab. `src/terminal-conversation.test.ts`
examines concurrent surfaces, new snapshots, no input submission, a retry with a limit
after a failure, and expired navigation and disposal.
`src/conversation-view.test.ts` makes sure that the manual retry is not shown
while the tab attaches or runs. These lifecycle tests use mocked IPC. The
compatibility tests are in `crates/core/src/board.rs`,
`crates/core/src/session/launch/tests.rs`,
`src-tauri/src/session.rs`, and `src/backend-contract.test.ts`.

The macOS application composes these parts: native command preparation in
`src-tauri/src/terminal_agent_launch.rs`, the PTY host, screen restoration and
canonical ingestion. A manual check of the macOS Tauri app used isolated
scratch projects and authenticated accounts from before. For the four providers,
it showed correct native responses, canonical user and assistant content, and
`turn.completed`. Cold resume kept each bound native identity and recovered the
history without duplicate prompts. Cursor also gave a correct live follow-up
after the repair of the physical workspace path.

After a normal app exit, none of the nine child processes of the app
continued to operate. These processes are the four provider CLIs, the Codex
engine and the four terminal-state helpers. The programmatic follow-up of
Antigravity got to the native TUI. After the repair for hidden system messages,
canonical recovery kept accurately two user messages and two answers. Recovery did
not change the earlier completion count, and no test examined a new live
completion after that repair.

The native CLI smoke tests and the automated tests below keep their smaller
scope. These checks do not show all combinations of tools, decisions and
sharing. The subsequent provider tables describe the current Chat adapters,
unless they name native terminal behavior.

Before a process write, native queued input has its own durable dispatch token
and an intent for prepared input. The composite tests of the core and the journal
examine these items:

- Crash recovery.
- The same receipt again, without one more send.
- Subsequent pending text that stays.
- No retry when Prometeu is not sure of the acceptance.

Causal source replay uses native Cursor and Codex identities, Claude row UUIDs and
Antigravity step indices to make an accepted intent agree. If Prometeu cannot
read the source content, it does not mark coverage. For AGY 1.5.10 terminal
content, Prometeu reads the scoped initial SQLite trajectory and the full
output files. A native smoke test with an Ask-approved `printf` and an offline
projection showed one correct identity for the call and the result.

Synthetic tests examine WAL immutability, output changes, paths, Fresh/Resume and
durable acknowledgment. Auxiliary Any metadata can go with full native output.
Unknown result forms continue to be coverage failures. This does not show parity
for all tools. Refer to the [AGY store boundary](../contracts/native-antigravity-store.md).

Observer and journal fixtures examine replay, rewrites and the order of metadata
receipts. These tests do not show full parity of native control. The fixture for
the boundary between the board and TypeScript includes the additive dispatch
state. Its text stays on the screen until Prometeu durably accepts it. The usual Chat
queue behavior does not change.

The ingestion composite test also makes sure that a sealed source row keeps the
current tool effects. A result after completion, after an interruption or after
a new committed app input updates the history without a notice. Causal-user
bindings and trajectory bindings stay after the app starts again. Journal schema migrations
keep the public bytes and the native receipts.

A new start with an empty session has a smaller limit. A tab without a native
session identity cannot resume. Prometeu continues to stop an unresolved
prepared input when Prometeu is not sure of its acceptance. In the recorded empty AGY and
Cursor conditions, Prometeu does not automatically make a conversation again or
send the input again. The journal regression test for a cancellation that is
known as not written examines a safe retry after a live reject. It does not
let Prometeu try an uncertain intent again after a crash.

| Provider | Native preparation and behavior | Evidence and specific remaining checks |
| --- | --- | --- |
| Claude | The accurate native session id, the captured account, model and effort, an explicit permission mode, instructions and the selected MCP and packages | Correct results: Tauri response, history, completion, cold resume, native hook question response, native dialog race and explicit Bash approval. The ExitPlanMode request opens correctly. Its response delivery and the app decision controls have no test at this time |
| Codex | An app-server that Prometeu owns, a remote TUI, the accurate resume id, a derived home for the account and the tools, native config and the selected hook hashes | Correct results: Tauri response, history, completion, cold resume and engine cleanup at app exit. Also correct: native Ask/Plan bootstrap, RPC recovery, question and answer, and explicit command approval. File-change approval, quota and shared control have no conformance test at this time |
| Antigravity | Workspace, the accurate resume id, model and effort, permission and plan flags, and private instructions that always apply. Decisions stay in the native TUI. This is a product decision | Correct results: Tauri initial response, history, completion, cold resume, and programmatic follow-up with silent canonical recovery. A new live completion after the repair for hidden system messages has no test. Native instructions and an Ask-approved printf tool projection are correct. Hub selections that are not empty are not available |
| Cursor | Workspace, the accurate resume id, model, native permission and plan flags, private instructions through the native prompt hook and the activation of the selected stdio MCP. Decisions stay in the native TUI. This is a product decision | Correct results: Tauri response, history, completion, cold resume, live follow-up, the native fence for a correct stop and one Shell call with recovery. MCP tool calls and remote transports have no test. A different effort control and hub packages are not available |

The default test run ignores the opt-in native smoke test in
`terminal_agent_hooks/smoke.rs`. This test examined a new response and an
accurate resume for Claude 2.1.326, Codex 0.169.0, Antigravity 1.5.9 and Cursor
2026.11.13. In this harness, Claude did not operate without the query replies of
the headless emulator. Codex and Antigravity also accepted a bracketed-paste
follow-up after an explicit native idle or done report. The test found no OSC
readiness signal from Cursor.

These short prompts use no tools. They use isolated scratch workspaces and
external credentials from before. The earlier Codex smoke test without the engine used
`on-request` with `read-only`. For Ask and Plan, the owned engine proof below
replaces that preparation path. Antigravity also obeyed private instructions that
always apply, on a new start and on resume.

In this build, Cursor did not load rules from private plugins or from added
roots. As an alternative, its native prompt-hook context was correct on a new
start and on resume, with a changed instruction and the same user prompt. The
examined carrier has a limit of 10,000 UTF-16 units. These smoke results are
different from end-to-end conformance of Prometeu.

The ignored test `terminal_agent_launch::mcp_smoke::native_cursor_loads_selected_stdio_mcp_without_inference`
also gave a correct result on Cursor `2026.11.13-eacade1`. It uses the production preparation
and MCP materializer, a private plugin, `CURSOR_CONFIG_DIR` and the native
authentication from before. The selected stdio server received `initialize`,
`notifications/initialized` and `tools/list`. Its configured environment
replaced an inherited value. The test did not send a prompt or `tools/call`.

The test makes sure of the scratch path before native trust. It examines the
modes of private artifacts. It reaps the PTY, the fixture server and the screen
helper. It does not examine remote MCP transports or native tool invocation. The
test does not use `agent mcp list-tools` as proof, because that subcommand does
not load invocation plugins.

The [Claude decision bridge](../contracts/native-claude-decisions.md) also gave
correct results in two short tests with synthetic questions on 2.1.326:

- The native tool returned an explicit hook answer.
- A local selection of Blue stayed Blue after the hook delivered a late answer of
  Green.

The canonical card closed as `resolved`. This status records the delivery. It
does not tell that the hook won. Timeout, disconnection and retirement stay
neutral. A different native Bash test examined these items:

- The literal printf command, without changes.
- The permission dialog before the explicit hook permission.
- Native tool completion.
- The accurate result in the scratch file.
- Prometeu rejects a duplicate response.

The Ask+Plan startup keeps manual permissions and does not enable bypass.
ExitPlanMode got to its native empty-plan dialog and sent the plan request. No
test examined the delivery of a live response. The empty-plan selector has an
offline regression test. No test examined more combinations of native rules or
shared remote control.

The separate [Codex engine proof](../contracts/native-codex-engine.md) uses the
native TUI with `untrusted`, `dangerFullAccess`, Plan and low effort for each
thread. It sends no model request. A different short smoke test with one turn
sent live RPC into the runtime consumer. The cold `thread/turns/list` replay did
not change the canonical bytes, with one user message and one completion. Tests
with a fake process and with the consumer examine pagination with a limit and
decisions that are only explicit. They also examine cleanup, source receipts and
crash tails with an active tool.

A live Plan question also went correctly through the native TUI and the attached
answer adapter. The model returned Blue, the native card resolved, and Prometeu
rejected answers that came again. An explicit one-time command approval was also
correct. The native resolution, the identity of the completed tool, the contents
of the scratch file were correct, and Prometeu rejected answers that came again.
File-change approvals, quota and shared remote control have no native
conformance test at this time. The previous on-request stop smoke test does not
examine those behaviors.

In the first version, Cursor and Antigravity terminal decisions use the native
TUI. This is a product decision. Application buttons and shared buttons for
approvals and questions are not in that accepted scope. Chat keeps its current
capabilities. These tests examine ownership for each conversation, encrypted
announcements, responses from earlier clients and native input that stays:

- `conversation-capabilities.test.ts`, `team-viewer.test.ts`,
  `team-owner.test.ts`, `team-channel.test.ts` and `island.test.ts`.
- The relay compatibility tests.
- The native input rejection test.

For the four native providers, slash commands and permission-mode changes stay
in the TUI. Shared transcript controls do not show provider command suggestions,
compact and context actions, or approval actions that change to bypass. User
Actions continue to expand first. Usual prompts keep paths, for example
`/Users/file`. Claude and Codex keep individual responses for approvals and
questions.

When Prometeu rejects a manual slash command, the composer of the viewer that
sent it stays. Shared mode metadata stays encrypted. The viewer and the owner
stop commands that are not available. Pure grammar tests, relay compatibility
tests, encrypted round-trip tests and peer-input tests examine this boundary.

Native input admission in `native_ui_readiness.rs` uses viewport captures from the installed CLIs
of Claude, Cursor, Codex and Antigravity. The Codex test for a new start, cold
resume, literal text, Home and clear uses no inference. The dim placeholder
shows the difference between empty input and the same draft at the same
physical cursor. A separate native idle signal is also necessary.

For Antigravity 1.5.10, admission uses the captured single-row composer, the
physical cursor at Home, full-width borders and the accurate idle footer. A
separate native idle signal is also necessary. A read-only audit of its pinned
submit handler shows that it discards plain spaces when there are no
pasted-content placeholders. Regression tests reject literal mode-hint text at
Home. The native empty hint must have its captured palette-8 foreground and the
accurate native truncation.

An explicit `NO_COLOR` hint stays ambiguous and does not release programmatic
input. Tests also reject literal drafts, chips, menus, paint that is not complete, changed
geometry and unknown versions. Application payloads do not change. Refer to the
[fixture records](../../src-tauri/src/native_ui_readiness/fixtures/README.md).

The tests in `crates/protocols/src/cursor_store/tests.rs` also examine these
items in the pinned native-store decoder of Cursor:

- Tool arguments and results.
- MCP JSON values.
- Read-content blobs.
- Compacted tool references.

Call and result revisions keep the native call ID, and Prometeu commits them
independently. They do not add approval cards or a turn completion that the source
does not have. Unknown tool fields, turns with only shell commands and synthetic
users continue to prevent full source coverage.

A controlled native Shell smoke test also gave correct results for the prompt
paint, one explicit native approval and execution. The recovery from the copied
store was also correct. The native call IDs and result IDs agreed. Other tool variants
have only fixture coverage. These checks do not show full conformance of app
approvals or programmatic approvals. Cursor approvals stay native. This is a
product decision.

A native stop profile in `terminal_agent_controls.rs` operates only when it sees
an active turn and a current composer hint with a version. Codex 0.169.0 and
Antigravity 1.5.9/1.5.10 also gave correct results in the opt-in interruption
smoke test without tools. One Escape caused an explicit idle or done report and
a native interrupted message. The same process continued to operate. In this
check, Codex uses `on-request`/`read-only`, and Antigravity uses explicit Ask
settings.

Startup MCP cancellation is a negative Codex fixture. It is not proof of a model
turn. Claude 2.1.326 also gave a correct result for one Escape after its prompt
hook and native Working, after the startup-hook phase. Then it showed explicit
Idle, and the process continued to operate. The compact composer of Claude can
be above the bottom of the viewport. While Claude works, it can hide the physical
cursor. The current cursor position, the borders and the footer identify the
control.

Cursor 2026.11.13-eacade1 also gave a correct result for an interruption without
tools. Its native prompt, the model output and the Working footer came before
only one Ctrl+C. An authenticated root Stop reported `aborted`, and the
process continued to operate. After the proof, its waiter showed that it reaped
the process. Cursor puts the interrupted prompt back as a draft, and the harness
keeps it. The harness records subsequent Stop causes independently and does not add an
OSC idle or done signal.

The control accepts a startup banner that scrolled off the screen only when the
current composer, footer and cursor are aligned. The fixture tests reject
conflicting bindings, paints that are not complete, transcript mentions and versions below
the minimum. Layouts that the profile does not know stay unavailable. Cursor
does not show its native hint when its composer has a draft.

For Cursor, the pinned source also accepts a local Escape with an empty prompt
when no overlay, history or queue action uses it. The application Stop profile
continues to send its advertised Ctrl+C. Before Prometeu commits the canonical
`interrupted`, it connects the authenticated root aborted hook to the current
durable request. Observer regression tests reject earlier requests and races with
a new prompt. They keep the cancellation when a subsequent completed hook for the same
generation comes.

These tests also make sure that the pending prompt-generation barrier becomes empty
after an accurate durable capture. Then a subsequent native request can start without
one more prompt hook. Stop candidates with a limit keep the proof of each request when
earlier hooks come late.
`journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet` examines
the production portable pump, duplicate receipts, a restored native draft and late
source or tool activity without a success notification. These tests examine the
native interruption proof above, and they do not include a new Tauri run with Esc.

The Cursor spinner also starts before the native store is current. This occurs
when a newer busy composer that Prometeu examined comes after a local Enter
boundary. The host test
`cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`
examines initial work, that a stable screen is rejected and a new native
submission.
`cursor_working_observation_requires_its_current_complete_busy_composer` and
`observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once`
examine the screen profile and the shared execution cycle. Enter alone and paste
are negative test conditions.
`cursor_restored_prompt_retry_settles_without_repeating_before_submit_hook`
examines the independent barrier for source identity and the order of late Stop
hooks. None of these synthetic tests includes a new native model run.

The recorded AGY 1.5.10 Escape sequence without tools is `Idle` → `Working` →
`Idle`. `terminal_execution` and the native host regression tests examine these
items:

- The silent settlement of that root activity.
- Late source updates.
- The next input.
- Children, requests and applicable Done tickets that stay.

`src/timeline.test.ts` and `src/alert.test.ts` examine how the busy presentation
stops without a success notification. These regression tests use the native
sequence that the recording shows.

A different native run on 1.5.10 that Prometeu owned had a file read and Escape.
It captured `Idle` → `Working` → `Done` → `Idle`. The run had an explored file and
the native interruption notice, and the process continued to operate. The
projector and the host replay that captured sequence with late source content for
thinking and tools. The tab stays ready and keeps the content. Prometeu does not
show success from Done alone.

Regression tests for completion tickets and late input keep the independent
source barrier. This proof comes from the native CLI and from host regression
tests. It is not a new Tauri cancellation run. The same recorded interrupted
viewport also shows input readiness. The native interruption banner above the
empty composer does not stop queued input. The host regression test keeps the
local-input fence until a newer examined paint and a native idle signal. Then it
admits the next message and does not send it again.

Claude 2.1.332 sends `Error` for turn failures, except failures related to
account access. `ExecutionProjector` sends `idle` for the `root` record and keeps
the state of active child tasks. Transcript data that comes after the error does not start
the work indication again. The agent can send `Working` in the same cycle
after a usage limit.

These tests examine `ExecutionProjector` and `SessionPump` with reports that
the tests make:

- `terminal_execution_error_stops_root_activity_without_completing_or_blocking_retry`
- `claude_native_error_stops_spinner_keeps_late_content_quiet_and_allows_retry`

The tests examine these conditions:

- A child task stops with an error.
- Input from the transcript comes after the error.
- The agent tries again without a new prompt.

Source inspection and these tests show the behavior. They do not include a run
of Claude that reaches the usage limit. `ExecutionProjector` uses this rule for
all providers in `ConversationMode::Terminal` that send `Error`. The rule does
not add information from the Cursor screen. The Codex engine continues to
control when its turn completes.

The OSC parser and reducer tests are the same for all providers. They examine
chunked status records, retention with a limit and explicit completion for each
record. Native PTY tests examine input, resize, shared process control and the
reap of processes. The two groups do not show permission responses, notification
eligibility, content sharing or usage capture.

All providers use the same native renderer. `src/term.test.ts`
examines the filter by generation, accurate screen replay, pending UTF-8, query
suppression, ordered text and binary input, and cleanup.
`src/conversation-view.test.ts` examines exclusive local views, how remote
transcripts go to the correct view and native quotations. `src/native-conversation-draft.test.ts`
examines concurrent context retention and accurate review acknowledgment by tab
and envelope. It also examines snapshots that come again, the cleanup after a tab
closes, the format of individual image paths and document mentions.

The image format obeys the source normalizer of Codex 0.169.0. Each paste has one
literal path, with quotes when necessary and no `@` prefix. This inspection of
the source does not show native Tauri image-chip rendering or image recognition
by other providers. `src/term.test.ts` and
`src/terminal-conversation.test.ts` examine insertion for one generation,
bracketed paste readiness and rejected controls. They also make sure that there
is no automatic Enter or resume. The native-screen scenario in
`e2e/composer.spec.ts` examines the xterm geometry in the browser and keyboard focus. These
browser fixtures do not examine native provider behavior.

Claude image reads have native-ingestion regression tests in
`terminal_transcript::tests::terminal_transcript_claude_tool_images_keep_results_and_validate_the_envelope`
and `terminal_observation_claude_read_image_survives_ingestion_and_cold_resume`.
They examine these items:

- Base64 images that are too large, nested in tool results.
- Small images that stay the same byte for byte, for journals that earlier
  parsers saved.
- Text and tool identity that stay.
- Envelopes that are not correct.
- Cold replay of large and small images without duplicate canonical output.

This proof comes from synthetic parser and journal tests. It is not a native
Tauri run. Codex keeps its different inline-image shapes. The behavior of other
providers does not change.

The private [terminal state helper](../contracts/terminal-state.md) has shared
tests for headless replay and stdio in `packages/terminal-state/test/`. The macOS
desktop host and the renderer use its screen contract, its single query
authority and its generation order. A local check on macOS arm64 without
installed Node showed that the Node SEA/Tauri bundle operates. The standard
macOS app and release build include it. CI examines the behavior of the helper
and the host, and the Linux release does not use the helper. These shared
transport tests do not show more provider capabilities, and they do not replace
the check of the Tauri app above.

## Automatic worktree cleanup

The local opt-in preference applies in the same way to Claude, Codex,
Antigravity and Cursor. It applies when a person archives or finishes a workspace
manually from the desktop UI. It uses the current cleanup IPC and sends no
provider payloads. An explicit selection is necessary for worktrees that are
dirty, not merged or that Prometeu cannot examine. The preference does not
include the initial clones and remote workspaces. `src/cleanup.test.ts`
examines defaults, persistence, safe scoped deletion and errors.
`e2e/audit-regressions.spec.ts` examines the current connection of archive and
finish.

## Sidebar resizing

A person can change the width of the desktop sidebar with the pointer or with
the Left and Right keys. Home or a double-click sets the default width again. The
app keeps the width locally. The behavior is the same for all providers: Claude,
Codex, Antigravity and Cursor. The browser check in
`e2e/sidebar-resize.spec.ts` examines these items in Chromium and WebKit:

- Pointer capture and the flex width in the browser.
- Input on the separator, which can get the focus.
- Collapse, open again and reload.
- A change of the window dimensions.
- The two panels at large saved widths, and a drag of each edge.
- Panel visibility, and a smaller window that keeps the space of the
  conversation.

This browser test prevents errors in geometry and pointer capture in the
two engines. Unit tests cannot show hit testing or flex layout. No provider
protocol changes.

## GitHub issues and pull requests

The discovery of GitHub issues and PRs uses the same desktop path for Claude,
Codex and Antigravity. The three agents receive the title, the description and
the URL of the origin through the current launcher. The preparation of the PR
worktree operates independently of the agent.
Tests: `src-tauri/src/github_auth.rs`, `src-tauri/src/github_issues.rs`,
`src-tauri/src/github.rs`, `src/github-issues-model.test.ts` and the workspace
journey in `e2e/issues.spec.ts`. The browser scenario examines the focus on the
provider tab, nested row actions, a claim from Available and the prepared PR
source that stays.

Refer to the [contract](../contracts/github-issues.md). GitHub notifications also
operate independently of the agent. The tests are in `src-tauri/src/github_notifications.rs` and
`src/notification-feed.test.ts` ([contract](../contracts/github-notifications.md)).

## Local telemetry foundation

Settings / Analytics uses the reviewed Usage screen.
`src/usage-history.test.ts`, `src/telemetry-settings.test.ts`,
`src/telemetry.test.ts` and the native telemetry tests examine inclusive totals,
session coverage, unknown values, query cohorts, reads of expired data and
deletion. The added Settings browser scenario in `e2e/usage-insights.spec.ts`
examines live detail updates, accessible-name updates, keyboard focus and
horizontal scroll in Chromium and WebKit.

A browser test is necessary for the DOM replacement and the overflow focus of
WebKit. The pure mapping and controller tests cannot give this proof. Provider
measurement capabilities do not change. The
[improvement proposal](telemetry-improvements.md) gives the work that is not
complete.

[ADR 0059](../decisions/0059-local-telemetry-foundation.md) adds the same local
path for capture, store and settings for all providers. The primary completion
is different from the conversation settlement rule in ADR 0056.

| Measurement | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Primary execution and accepted message turns | Captured | Captured | Captured |
| Independent child execution and explicit human waits | When canonical signals show them | When canonical signals show them | Unavailable |
| Input and output of the primary agent | Turn result. Examined `modelUsage` deltas can also include the full tree | Delta of examined thread totals. Without a baseline, resume or reset stays not complete or unknown | Current steps without duplicates. Examined cumulative deltas complete the turn |
| Cache and reasoning | Cache read and cache write are subsets of the input. Reasoning is unknown | Cache is a subset of the input. Reasoning is a subset of the output | Reported cache and thinking subsets. If there is no report, unknown |
| Models and calls that Prometeu sees | Primary assistant IDs and examined cumulative model rows. Each message call one time | Unknown without examined proof of the model and the call | Different step IDs that Prometeu sees. The model breakdown is unknown |
| Cost for each turn | Cumulative delta in the same session. Spend from restored, reset or overlapping children is not included | Unknown | Unknown |
| Context meter and actions | Input of the primary call and the reported model window. Explicit compact, report and new conversation | Last input and model window. Explicit compact, report and new conversation | Not shown without a reported window |
| Cache rebuild signal | Calls without duplicates, with a known previous context and proof of a cache write | Unknown without reliable proof of different calls | Only if there is proof of a cache write in different steps. If not, unknown |
| Reply footers and workspace summaries | Canonical usage. A local hashed lookup restores the native transcript footer | Canonical usage. Prometeu does not add a price | Canonical usage that Prometeu sees. Prometeu does not add a price |
| Usage screen and session details | Known tokens, cache, conditional cost and model rows, and coverage | Known tokens, cache and reasoning, and coverage. Cost and models stay unknown | Known step and turn usage, and coverage. Missing fields stay unknown |
| Source of app calls | Name generation and plugin maker, each measured independently | Name generation, measured independently | No model call that the app starts |

`telemetry/tests.rs`, the `telemetry_*` tests of the adapters and
`src/telemetry.test.ts` examine these items:

- Normalization and coverage that is not complete.
- Local query and erasure behavior.
- Late terminals from before an erasure.
- Concurrent WAL readers, streamed export and symlinks that are rejected.

`src/telemetry-settings.test.ts` examines the refresh of the workspace option
while a filter stays selected. The current command and publication tests examine
accepted sends and sends with a failure, and the order of responses. If streamed input has no
native terminal that Prometeu can relate to it, the turn attribution stays
unknown. Capture health shows that limit. These guarantees come from fixtures,
and they do not examine all installed provider versions live. The
[contract](../contracts/telemetry.md) gives the full semantics.

`src/usage-presentation.test.ts`, `src/workspace-usage.test.ts`,
`src/timeline.test.ts`, `src/conversation.test.ts`, `src/telemetry.test.ts` and
`e2e/usage-insights.spec.ts` examine the presentation of issue 131. The browser
scenario examines keyboard and Escape focus. It also makes sure that the typed
draft stays when a context action uses the shared popover. This includes a turn
that starts and stops while the panel stays open.

The transition to a disabled action must keep Tab and Escape focus in Chromium
and WebKit. Pure tests cannot show browser focus after a DOM replacement. The
pure presentation tests examine how model labels and provider labels stay clear.
The browser scenario does not add variants for each provider or locale. It does
not use a native CLI.

Adapter fixture tests examine consecutive counters, restored, reset and error
results, and duplicate observations. Synthetic Claude fixtures declare their
provenance explicitly. Telemetry tests examine reply privacy and lookup, groups,
sources and PR proof. Measurements that a provider does not give stay unknown.
Prometeu does not show a billing estimate for them.

The native telemetry producer test makes sure that the store shows the commit
and releases the lock before `telemetry-changed`. The workspace consumer test
examines a refresh without a board change, the merge of refreshes, navigation
and the invalidation after a history deletion.

These added checks apply to the current features:

- `boundary_contract.rs`, `scripts/check-contracts.mjs` and
  `src/backend-contract.test.ts` compare the serialization of the backend board
  and model, and synthetic Claude and Codex output, with frontend types and
  reducers. Antigravity keeps its recorded canonical fixture in
  `src/conversation.test.ts`.
- `codex/process_tests.rs` uses a controlled subprocess to examine
  initialization, split UTF-8, interruption, output that is not complete at process exit
  and resume. It does not show live CLI compatibility. Claude and Antigravity do
  not have the same subprocess coverage. Refer to the [runtime contract](../contracts/agent-runtime.md#process-boundary-verification).
- The tests in `crates/core/src/workspace_lifecycle.rs` examine how finish,
  archive and restore keep the state for all providers through shared board
  rules. They do not start a provider.

## Product analytics

[ADR 0093](../decisions/0093-anonymous-product-analytics.md) records the
activations of named controls and the explicit feature events as different
events. The [contract](../contracts/product-analytics.md) defines the triggers.
The [coverage map](product-analytics-coverage.md) gives the list of product
modules.

When there is no correct saved selection, the app asks the same consent question
at startup for Claude, Codex and Antigravity. The app keeps the two answers. A
person can change the selection in Settings / General / Privacy. Analytics stay
disabled until the person accepts. The setting controls native and frontend
events, and it does not change agent execution or local history.

The Rust tests for consent, persistence and batches are in
`src-tauri/src/analytics.rs`. The tests for accept, decline and defer at startup,
and the Settings tests, are in `src/analytics-settings.test.ts`.

| Measurement | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Named controls and navigation | Shared desktop UI | Shared desktop UI | Shared desktop UI |
| Files, Git, review, terminal and preview | Shared feature paths | Shared feature paths | Shared feature paths |
| Model and effort selection | Only available selections | Only available selections | Only available selections |
| Requests and approvals | Only the capabilities that the adapter shows | Only the capabilities that the adapter shows | Interactive approval and plan controls are not available |
| Resource setup and actions | Configuration events and usage events are different | Configuration events and usage events are different | Only the resources and actions that are available |

Product events do not record canonical turn completion or assistant output at this time.
The capability matrix continues to control which features are available. If a
control is not available, the absence of its events does not show that a person
did not use it. The app sends no provider payload and no content.

Native analytics tests and shared fixtures validate the closed event schema.
TypeScript tests examine parity, identity capture and feature triggers. The
analytics check with a browser DOM examines metadata on shared controls and
portals. These checks do not examine live PostHog delivery or all CLI versions.

Only macOS and Linux desktop builds that have the compiled key and the opt-in of
the person send events. Mobile does not send events. The browser mock keeps the
selection but ignores events. Missing events from these shells, or from
installations without consent, do not show missing usage.

## Legend

- **Native:** the CLI sends the form that Prometeu uses at this time.
- **Adapted:** Prometeu changes the format or does the feature itself.
- **Unavailable:** the UI does not show it, because the provider cannot do the
  flow.
- **To confirm:** a fixture or a dedicated test is missing.

| Capability | Claude | Codex | Antigravity | Main evidence |
| --- | --- | --- | --- | --- |
| terminal sessions without a workspace or project, with a name, closed with × or `exit` and shown again after the app starts again | operates independently of the CLI. No agent runs in them | operates independently of the CLI | shared application behavior | `crates/core/src/terminal_sessions/tests.rs`, `src-tauri/src/terminal_sessions.rs`, `src/terminals.test.ts`, `fixtures/backend-contract.json`. A person examined the sidebar group, view and shortcut manually in the browser mock. They have no E2E scenario |
| task inbox with search, filters for team and status or repository and label, and workspace actions | shared UI that operates independently of the CLI | same application flow | same application flow | `src/issues-filter.test.ts`. `e2e/issues.spec.ts` for menu focus, narrow layout and workspace launch. Cursor uses the same UI |
| claim an unassigned Linear issue from a joined team | operates independently of the CLI. An assigned issue can start a workspace | same application flow | same application flow | `linear.rs` eligibility, scope and cache tests. `e2e/issues.spec.ts` for keyboard, pending-action and read-only navigation paths through the mock. A manual check is necessary for live Linear consent and mutation |
| grouped settings and resource library with search | shared UI. Current account and tool capabilities | same UI and current capabilities | same navigation. The external account notice and the tool execution that is not available do not change | `src/settings-navigation.test.ts`, `e2e/settings.spec.ts`, the current accounts, Cloud, MCP and notifications scenarios |
| make, rename, trash and restore from the side file tree | operates independently of the CLI | operates independently of the CLI | shared application behavior | `src/tree-menu.test.ts` for the menus. `src/tree-moves.test.ts` for the tab strip, drafts and expanded folders that follow a moved entry. A person examined their connection in `workspace.ts` and `viewer.ts` manually in the browser mock, and E2E does not include it. `crates/files/src/entries.rs` tests for names, Git metadata, conflicts, case-only renames on a case-sensitive disk and symlinks. `restore_deleted` tests in `crates/git/src/tests.rs` for staged content and deletions. Linux tests do not examine case-insensitive disks |
| Git marks in the side file tree | operates independently of the CLI | operates independently of the CLI | shared application behavior | `src/tree-git.test.ts`, `src/git-refresh.test.ts`, `tree_marks` and single-flight tests in `crates/git/src/tests.rs`. `e2e/audit-regressions.spec.ts` makes sure that file rows stay stable after a save. E2E does not examine mark colors |
| copy a full Markdown code block, with diff fences | shared presentation | shared presentation | shared presentation | `src/markdown.test.ts`, `e2e/markdown.spec.ts`. Clipboard success and failure through the browser mock |
| public bug reports and private feedback with an account, image and capture | operates independently of the CLI | operates independently of the CLI | shared application behavior | `e2e/feedback.spec.ts`, the `FeedbackTest` tests of the Cloud. A manual smoke test is necessary for the system browser, native capture and the GitHub service |
| organizations, invitations and institutional sharing | the same relay V4. Local execution | the same relay V4. Local execution | shared application behavior | `team-organizations.test.ts`, `worker.integration.test.ts`, `e2e/organizations.spec.ts`, Rails integration and browser tests |
| automatic E2EE with TOFU in collaboration | the same HPKE Auth channel. No forward secrecy | the same HPKE Auth channel. No forward secrecy | shared application behavior | `team-crypto.test.ts`, `team-security.test.ts`, `team-channel.test.ts`, `worker.integration.test.ts`, E2E comments in Chromium and WebKit |
| leave a share, and admit again a person who left | the same relay V4. Local execution | the same relay V4. Local execution | shared application behavior | `relay/src/logic.test.ts`, `relay/src/protocol.test.ts`, `team-channel.test.ts`, `team-owner.test.ts`. E2E does not examine the sidebar menu and the confirmation |
| rights for each person to send messages and to control, only from approved devices, with a notice on the Mac of the owner | Send messages is necessary for prompts and interrupt requests. Control is necessary for request answers. Viewers keep content and comments | same rule before the Codex adapter | same rule before the Antigravity adapter | `team-rights.test.ts`, `team-security.test.ts`, `team-owner.test.ts`, `team-channel.test.ts`, `team-member.test.ts`, `team-organizations.test.ts`, `share_tests` in the core crate. A person examined the notices, the share menu and the hidden controls manually in the browser mock. They have no E2E scenario |
| owner confirmation of messages from teammates | the prompt of a teammate with the right waits above the composer of the owner with Send, Edit and Discard, only in memory. Interrupt requests, answers and the devices of the owner run without a wait | same rule before the Codex adapter | same rule before the Antigravity adapter | `team-owner.test.ts`, `team-organizations.test.ts`, `share_tests` in the core crate, `fixtures/backend-contract.json`, the `chat-pending` gallery story. A person examined the card, the share-menu option and the desk tiles manually in the browser mock (`mock.message()`). They have no E2E scenario |
| optional Prometeu account in the sidebar | operates independently of the CLI | operates independently of the CLI | shared application behavior | `cloud.rs`, `e2e/cloud.spec.ts`. The app sends no transcript |
| Git project catalog, batch clone and local registration | operates independently of the CLI. No setup | operates independently of the CLI. No setup | shared application behavior | `catalog/projects.rs`, `e2e/projects.spec.ts`, Cloud `projects_catalog_test.rb` |
| make an empty local Git project and start its first workspace | operates independently of the CLI. No initial commit or setup | same flow | shared application behavior | `crates/git/src/tests.rs`. `e2e/projects.spec.ts` examines that the nested form input and the focus stay. Then it starts a conversation in the project folder without a worktree |
| catalog of plugins, MCP and Actions in the account | operates independently of the CLI | operates independently of the CLI | shared application behavior | `catalog.rs`, `catalog_test.rb`. Secrets and installation stay on each Mac |
| the plugins, MCPs and skills of the organization | explicit local installation in the hub | the same UI and installation. Current adapters | Unavailable | `catalog.rs`, `organizations_test.rb`. The optional installation form has no dedicated E2E. A personal copy is not necessary, and the installation does not activate automatically |
| Actions form with shared components | the same UI | the same UI. The options come from the catalog | shared application behavior | `e2e/actions.spec.ts` in Chromium. Shared form focus and validation in `e2e/design-system.spec.ts` on the two engines |
| isolated Desktop resource view and reusable components in Resources/Actions | same view and catalog rules | same view and catalog rules | same view. The current tool capabilities do not change | `src/resources/adapters.test.ts`, `e2e/settings.spec.ts`, import-boundary fixtures. The gallery runs without provider initialization |
| Desktop component catalog, chat and Git extraction | same canonical chat blocks and current capabilities | same canonical chat blocks and current capabilities | same components. Requests that are not available stay not available | `src/components/catalog.test.ts`, the current Git, critical-flow and Markdown tests. Isolated chat and Git stories have no dedicated E2E scenario and use no providers |
| the executable DS components of the company | operates independently of the provider | operates independently of the provider | shared application behavior | `e2e/design-system.spec.ts`, menu, submenu, password, focus, validation and error on the two engines |
| Code review included, and a person can edit it | the initial profile. A person can change the model and the provider | a person can select it in the profile | Adapted (see the verification boundary) | `actions.rs`, `actions.test.ts` for defaults and removal. `e2e/actions.spec.ts` for profile edits |
| prompt commands and tasks | the app adapts them | the app adapts them | Adapted (see the verification boundary) | `actions.test.ts`, `actions.rs`. `e2e/actions.spec.ts` continues to examine prompt edits |
| profile for each task | instructions and permissions through flags | instructions and permissions through JSON-RPC | Adapted (see the verification boundary) | `session.rs` and `codex.rs` tests |
| find the installation | adapted | adapted | version examined (minimum 1.2.7) | `agents.rs` |
| accounts and global selection in the footer | adapted through `CLAUDE_CONFIG_DIR` | adapted through `CODEX_HOME` | one external agy account, explicit attachment | `accounts.rs`, `e2e/accounts.spec.ts` |
| grouped controls to make a workspace, and account controls that are easy to read | shared fields. Plan mode is available | same layout with model search and native effort | same layout. Plan, effort and tool controls that are not available are not shown. An attached agy profile shows the attachment status | `e2e/launcher-layout.spec.ts`, `e2e/accounts.spec.ts`, `e2e/model-picker.spec.ts`. English, narrow windows, Chromium and WebKit |
| remove all accounts, and an empty selection | available. Keeps the login of the CLI | available. Keeps the login of the CLI | available. Keeps the external login | `accounts.rs`, `e2e/accounts.spec.ts`. Directories and credentials stay local |
| login through the app | injected native `auth login` and browser | injected native `account/login/start` and browser | attachment only. Official interactive agy login | `crates/core/src/accounts/login/tests.rs`: lifecycle order, cancellation, and guards for revision, error and busy state. Identity fixtures in `claude.rs` and `codex/account.rs`. A manual check of OAuth with two provider accounts is necessary |
| change accounts between turns | resume the shared transcript | resume the shared rollout and index | Unavailable. The identity has an external management | `crates/core/src/session/tests.rs`, profile tests. The tests do not show an authenticated continuation between two provider accounts at this time |
| turn that stops when the app quits or updates | shown as interrupted after the app starts again, with Continue | same | same | `board.rs::tabs_working_when_the_app_ended_are_marked_interrupted`, `session/tests.rs::an_accepted_message_clears_the_interrupted_marker`, `stream/tests.rs::a_snapshot_names_the_process_presence_last` and `a_recorded_cut_stays_in_replay_after_continuing_and_reloading`, `src/timeline.test.ts` (a conversation without its process). A person examined the notice and Continue in the browser mock (`mock:interrupted`). They have no E2E scenario |
| a question before the app quits, closes the primary window or starts again to update, when local work can stop | the count includes conversations that work and that wait. Operates independently of the CLI | same | same | `src/quit.test.ts`, `src/update.test.ts`, `src/appmenu.test.ts`, `crates/process/src/terminal/tests.rs` (a command that runs as a job or through `exec`) and `dock.rs` (`cuts_off`). A run of the macOS app is necessary for the native menu item, the window close and the relaunch |
| notch island: follow, approve, answer and open again | approvals with Edit diffs, plans and questions through the shared request cards | approvals for commands and file changes without a diff. Questions. No plans | shared rows and quota. Requests follow the canonical events of the adapter | `src/island.test.ts`. A run of the macOS app is necessary for native geometry, hover and answers. When the island is on, a close of the primary window quits the app. A macOS dev build examined this, because a unit test cannot make Tauri run events |
| quotas for each account | live stream and internal endpoint. The selected account has a shorter native poll budget | live app-server event, an app-server query with a limit and an internal fallback | native `/usage` child. Used and remaining amounts by model group. Completion uses an exit notification, not a 20 ms loop | `usage.rs`, `usage_scheduler.rs` tests for the controlled clock, generations and resume after a cancelled login, the child-limit test in `antigravity.rs`, `statusbar.test.ts`, `e2e/accounts.spec.ts`. A controlled native run is necessary for provider launch counts |
| live model catalog | native `list_models` control request | native paginated app-server `model/list`, with more models | native `agy models` | `agents/catalog.rs`, `agents.test.ts`. Errors for each agent and expired state |
| start a session | stream-json adapter with shared workers | JSON-RPC adapter with shared workers | NDJSON adapter with shared workers. A text smoke test with the installed CLI gave a correct result | `crates/core/src/session/workers/tests.rs`: initialization, order of output and exit, filters and scheduling failure. Native process tests and current provider fixtures |
| provider preparation and input | stream-json. Read-only native history. Input can come during a turn | JSON-RPC. Canonical app log. A closed writer releases the pipe, and the reader state stays | NDJSON. Canonical app log. Messages only when idle, and process interruption | `agent_launch.rs` preparation, resume and store tests without Claude or Codex binaries. The `canonical_input_*` tests of the three providers. Current native argument fixtures |
| resume a session | the transcript if it is there. If not, a new session with the same ID | the saved app-server thread. If not, a new session | the saved native conversation ID. If not, a new session | `crates/core/src/session/launch/tests.rs`: preparation and spawn failure, queue retention, publication and readiness, and permission overrides. The current `session.rs` and provider fixtures keep the launch compatibility |
| model search and favorites | same picker, native labels | same picker. More models only when the person selects them | same picker. A different provider identity, also for IDs that overlap | `e2e/model-picker.spec.ts`, `e2e/search-picker.spec.ts`. Chromium and WebKit |
| workspace tools with tab model and effort overrides | same selections for new and resumed tabs | same selections for new and resumed tabs | shared application behavior | `session.rs::new_and_resumed_tabs_preserve_resolved_tools_with_model_overrides` |
| tools in tabs and actions with mixed providers | the CLI base follows the effective Claude provider and the configured home | only the hub items, also in a Claude workspace | Adapted (see the verification boundary) | `session.rs::tool_resolution_uses_the_tab_provider_and_configured_claude_home`, `e2e/tools.spec.ts` |
| tool reset and project trust | null restores inheritance. Decisions apply to the shown hash, and empty declarations also | same contract | Adapted (see the verification boundary) | `session.rs::tool_axis_ipc_preserves_absent_null_and_replacement`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes`, `e2e/tools.spec.ts` |
| ordered prompt, transcript and live delivery | shared portable pump and conversation mutex | same pump with JSON-RPC translation at the edge | same pump with NDJSON translation at the edge | `crates/core/src/session/pump/tests.rs`: capture order, deferred reactions, scheduling, private usage, retirement and writes with a failure. Regression tests for concurrent delivery and fast responses in `chat.rs` |
| model selection | native through a flag | adapted in `thread/start`/`thread/resume` | native --model slug | `session.rs`, `codex.rs` |
| effort levels | only the advertised native levels and the provider default | only the advertised native levels. The earlier ultracode level maps to ultra | no effort control without advertised levels | `model-choice.test.ts`, `e2e/model-picker.spec.ts` |
| conversation footer that agrees with the width | model and activity apart from the tools. Remote control has a highlight when it is active | the same UI | Adapted (see the verification boundary) | `e2e/composer.spec.ts`, Chromium and WebKit, English and narrow frames |
| dictation in the message box | operates independently of the CLI. WebKit recognition | operates independently of the CLI. WebKit recognition | shared application behavior | `voice.test.ts`. A manual smoke test with a physical microphone is necessary in the bundled app |
| initial plan mode | native through permission mode | unavailable | Unavailable | `session.rs`, `launcher.ts` |
| streamed text | adapted to V1 | adapted to V1 | Adapted (see the verification boundary) | `conversation.test.ts`, `timeline.test.ts`, `codex.rs` tests |
| thinking | adapted to V1 | adapted to V1 | Adapted (see the verification boundary) | `timeline.test.ts`, `codex.rs` tests |
| tool call and result | adapted to V1 | adapted to V1 | Adapted (see the verification boundary) | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| questions to the user | native | adapted from a JSON-RPC request | Unavailable | `chat.ts`, `codex.rs` tests |
| approval requests | native | adapted | Unavailable | `chat.rs`, `codex.rs` |
| interruption | control request | `turn/interrupt` | Adapted (see the verification boundary) | `codex.rs`, Rust tests |
| compaction | the command of the CLI | `thread/compact/start` | Unavailable | `codex.rs` tests |
| context report | stream and transcript | made from token usage | Unavailable | `context.test.ts`, `codex.rs` tests |
| MCP selection for each workspace | the strict config of the CLI | table and environment that the app assembles | Unavailable | `mcp.rs`, `codex.rs`. A preparation error prevents the spawn |
| MCP connection checks and local OAuth, with Cloud definitions | shared hub actions. The token comes at process launch | same hub actions and local token | MCP execution unavailable | `e2e/mcp.spec.ts` simulates checks, login retry, logout and independence from Cloud revisions. A manual check is necessary for consent in a browser |
| plugin selection for each workspace | session flags | marketplace and config isolated for each workspace | Unavailable | `plugins.rs`, the CLI smoke test, `codex.rs`, `launcher.ts`, E2E |
| install a skill-bundle or bare `skills/*/SKILL.md` Git repository | a synthesized portable package through `--plugin-dir` | same package through the derived marketplace | Unavailable | `crates/tools/src/plugins.rs` skill-bundle and bare-skill tests, desktop `plugins.rs` discovery, [ADR 0098](../decisions/0098-skill-bundle-install.md). Installation does not activate automatically |
| local and account skills | a package with SKILL.md through the plugin selection | the same package with a native manifest | Unavailable | `crates/tools/src/skills.rs`, `catalog.rs`, `e2e/cloud.spec.ts`. Installation does not activate automatically |
| start a conversation from a skill with the declared artifact path | the app adds the package to the resolved `--plugin-dir` set of the first conversation. The opening line is in the first message | the app adds the package to the derived marketplace of that conversation. Same opening line | Unavailable. No hub tool selection | `kickoff.rs` tests, `session.rs::first_message_opens_with_the_kickoff_line`, `session.rs::artifact_path_follows_the_primary_repository`, `scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented`, `src/kickoff.test.ts`. Skills from plugins must come from a local folder. Subsequent tabs do not get the method again. If the skill of a resumed conversation is not there, it continues without the skill and shows a warning |
| layered selection (global, project, workspace) for each axis | resolved at spawn before the adapter | resolved at spawn before the adapter | Adapted (see the verification boundary) | `selection.rs` resolve tests, `session.rs::provenance_classifies_each_hub_item`, `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes` and `session.rs::tool_axis_payload_is_validated_before_persistence`. The project `[tools]` layer operates only after a person trusts its hash (trust on first use). An undecided item stays `pending`, and a rejected item stays `rejected`. Prometeu resolves the two, but it does not inject them. The setters reject a payload that is not correct or an id on an incorrect axis |
| CLI-inherited MCP base (ADR 0046 and ADR 0063) | found in `~/.claude.json`, in `.mcp.json` of the working directory and its ancestors (nearest first), and in the claude.ai connectors of the active account. The picker shows it with the `cli` provenance, and a person can remove it as a workspace delta. A declared axis materializes the full effective set through the strict config, with the connectors as `claudeai-proxy` entries | no discovered base. The CLI continues to load its own configuration, which the picker does not show | Unavailable | `selection.rs::cli_base_participates_in_the_chain`, `mcp.rs` inherited and universe tests, `mcp.rs::account_connectors_join_the_inherited_base` and `mcp.rs::missing_selected_servers_prevent_materialization`, `session.rs::provenance_classifies_inherited_cli_configuration`, `src/mcp.test.ts`, `src/tool-picker.test.ts`, the picker with mixed providers, the scroll of a long base and the inheritance reset in `e2e/tools.spec.ts` |
| hooks of a selected plugin | active from `SessionStart` | `enabled = true` and trust only for the `pluginId` and the hash before the thread | Unavailable | `codex.rs` tests. A failure prevents the thread |
| attachments in a message, capture thumbnails and paste | adapted through a local path. On macOS, the system materializes the promise and the pasteboard. On Linux, GTK materializes the pasted image | adapted through a local path. On macOS, the system materializes the promise and the pasteboard. On Linux, GTK materializes the pasted image | Adapted (see the verification boundary) | `file_drop.rs`, `chat.ts`, `paste.ts`, `tree-menu.ts`, `changes-menu.ts` and the shared `file-menu.ts`. `e2e/file-drop.spec.ts`, the dropped-file scenarios and the file tree menu in `e2e/critical-flows.spec.ts` examine the UI through the mock. `paste.test.ts` examines the paste detour, with the empty WebKitGTK clipboard. `paths.test.ts` examines how sent attachments come back as numbered image tags, the same for the two agents. `tree-menu.test.ts` examines the menu that an agent without attachments gets. `changes-menu.test.ts` examines the same groups on a changed file |
| the visual context of the browser | a tag in the draft and the history. Full HTML, CSS, URL and PNG mention on send | the same interface and text contract | Adapted (see the verification boundary) | `browser-context.test.ts`, `e2e/browser-inspector.spec.ts`, `e2e/browser.spec.ts`, `browser.rs` tests. A native check is necessary for WKWebView capture and the AppKit gesture |
| unknown external event | the adapter ignores it | the adapter ignores it | Adapted (see the verification boundary) | `conversation.test.ts`, `claude.rs`/`codex.rs` tests |
| the subagents of the CLI | a sidechain that is not on the screen. Tasks in `background.changed` | `collabAgentToolCall.agentsStates`, `subAgentActivity` and known child events update the tasks. The content of the children stays isolated | no child session signal. The turn ends on its terminal | `claude.rs`. Isolation, spawn, activity and partial-state tests in `codex.rs` |
| a turn that stops while its subagents run | the tab, the Stop button, queued input, delegation execution and the notice all wait until the tasks drain ([ADR 0056](../decisions/0056-background-tasks-hold-completion.md)) | the same rule through the same normalized signal | ends on the terminal, because the provider does not report a task at any time | `crates/core/src/conversation/work.rs::work_tests`. The recorded output of each adapter operates the same rule in `claude.rs` and `codex.rs`. Held-completion and interruption tests in `delegation.rs`. `src/timeline.test.ts`, `src/alert.test.ts`. Stop control in `e2e/composer.spec.ts` |
| local notifications and Dock attention indicators | completion, error and request events. Silent until the person opts in | same rule through canonical live events | completion and error through canonical events. Interactive requests unavailable | `src/alert.test.ts`, `src/notifications.test.ts`, `e2e/notifications.spec.ts`. Native check limits in [notifications](../contracts/notifications.md) |
| live sharing | V1 after normalization | V1 after normalization | shared application behavior | `team*.test.ts`, E2E through the mock |
| remote control from the devices of the owner | the same relay v4. Execution stays local | the same relay v4. Execution stays local | shared application behavior | `team-channel.test.ts`, `team-organizations.test.ts`, `e2e/organizations.spec.ts` |
| comments in a shared session | adapted after V1 | adapted after V1 | shared application behavior | `notes.test.ts`, `team.test.ts`, `relay/src/logic.test.ts`, E2E through the mock |
| desk with more than one conversation at the same time | adapted (the same conversation screen) | adapted (the same conversation screen) | shared application behavior | `desk.test.ts`, E2E through the mock |
| streamed conversation paint, and catch-up of a hidden window | shared V1 presentation. The subagent work state continues to update the composer | same V1 presentation and work-state rule | same presentation. No native child-task signal | `src/timeline.test.ts`, `e2e/critical-flows.spec.ts` in Chromium and WebKit for streamed text and hidden catch-up, `e2e/markdown.spec.ts` for the final Markdown and copy. WebKit timing limits in [energy profile](energy-profile.md) |
| sleep preference during local agent work | `agent` prevents display sleep until native child tasks end. On macOS, `agent-system` prevents only system sleep | same rule through the normalized child signals of Codex | same selections, but no native child-task signal to extend an ended turn | `src/statusbar.test.ts`, `awake.rs` command and transition tests, [ADR 0061](../decisions/0061-background-energy-policy.md). Only an AC `pmset` check, battery not available |
| Git: unified and side-by-side review, stage, discard, commit, remotes, branches, conflicts and the changed-file menu | the app adapts it. Operates independently of the CLI | the app adapts it. Operates independently of the CLI | shared application behavior | `crates/git/src/tests.rs`, `diff.test.ts`, `changes-menu.test.ts`, `e2e/git.spec.ts` in Chromium and WebKit, and a large review in `e2e/critical-flows.spec.ts`. The `git.md` contract |
| Worktree from a current branch | the app makes the worktree before it starts the CLI | same behavior | shared application behavior | `session.rs::new_branches_start_from_the_selected_base` examines local and remote refs. `e2e/launcher-layout.spec.ts` examines the selector. [Git contract](../contracts/git.md) |
| Markdown file reader, source editor, and preview of PDF, CSV and images | shared file viewer. Operates independently of the CLI | shared file viewer. Operates independently of the CLI | shared application behavior | `e2e/critical-flows.spec.ts`. SVG keeps source edits and gets a preview from the draft. Raster preview uses the current `read_bytes` IPC and the image decoder of the browser |
| find in the open file, and quick open by name | shared file viewer and `find_paths`. Operates independently of the CLI | shared file viewer and `find_paths`. Operates independently of the CLI | shared application behavior | `find.test.ts` for matches, smart case, wraparound and marker markup. `crates/files/src/search.rs` for path rank and host cache isolation. The find bar of the viewer and the Command-P palette have no dedicated E2E gate |
| scoped worktree cleanup after archive or finish | operates independently of the CLI | operates independently of the CLI | shared application behavior | `e2e/audit-regressions.spec.ts`, `session.rs` cleanup tests. An explicit force selection is necessary for blocked worktrees |
| agents for each workspace in the sidebar | the brand and the status of each tab | the brand and the status of each tab | Adapted (see the verification boundary) | sidebar flows in `e2e/critical-flows.spec.ts`. A remote agent uses the avatar of the owner, and the app does not guess the provider |

## Rule for a new feature

Before you enable a feature for a provider, do these steps:

1. Declare its shared semantics in the contract.
2. Add or change the capability in the descriptor.
3. Capture a fixture from the installed CLI without secrets or personal data.
4. Show that the translation into shared events is correct.
5. Examine the shared semantics in the reducer tests.
6. Add browser coverage only for a core UI journey or for a specified condition
   under the E2E scope policy.
7. Update this matrix with the path to the proof.

If a test is missing, the capability must not become `true` only because two
providers are similar.

## Known limitations

Desktop distribution operates independently of the provider. The macOS Apple
Silicon build and the Linux x86_64 AppImage share one release. The macOS app and the AppImage have signed
in-app updates for Claude, Codex and Antigravity. Other Linux installation types
keep external updates.
Tests: `scripts/test_release.py`, `src/update-init.test.ts` and
`src/update.test.ts`. Native installation and replacement stay manual checks
under the [release contract](../contracts/releases.md).

- A plugin source in a local `.zip` or a URL operates in Claude. Codex rejects
  it and shows an error. A local folder is the portable format.
- Skills, commands, MCP and hooks have a portable adaptation. `agents/*.md`
  stays only for Claude, because it is not part of the current Codex manifest.
- The global registry of each CLI continues to control the plugins that a person
  enabled outside Prometeu. These plugins are not part of the workspace
  selection.
- The CLI-inherited MCP base (ADR 0046 and ADR 0063) is only for Claude.
  Prometeu does not find servers configured for Codex in `~/.codex/config.toml`,
  and the picker does not show them. This is recorded future work.
- Prometeu reads the account connectors from an endpoint that it does not own.
  It keeps them in a cache for each login for five minutes. Thus a change on
  claude.ai can take five minutes to get to the picker. If the account is not
  available, the picker keeps the last known list of that login.
- If Prometeu did not read a connector list at any time, the picker shows the
  file base. In that condition, a spawn with a declared MCP selection that
  inherits the base stops, and Prometeu keeps the connectors. A `base: "none"`
  replacement can start independently. This is not true for an explicitly added
  account connector without its configuration.
- Attachments have UI tests through the mock and native validation of the saved
  destination. A person examined the thumbnail gesture in the app in Prometeu Dev on
  2026-09-06. A manual check is necessary to show that the CLI reads the
  attachments. If a promise has a failure or continues for more than 30 seconds, the app shows
  an error.

## Desired conformance suite

| Scenario | External fixture | Adapter | Reducer | E2E |
| --- | --- | --- | --- | --- |
| simple message | per provider | necessary | necessary | smoke |
| streaming + final message | per provider | necessary | necessary | critical |
| successful tool | per provider | necessary | necessary | critical |
| tool with an error | per provider | necessary | necessary | critical |
| question and answer | per provider | necessary | necessary | critical |
| interruption | per provider | necessary | necessary | smoke |
| resume | per provider | necessary | replay | critical |
| compaction | per capable provider | necessary | necessary | smoke |
| background task | per capable provider | necessary | necessary | smoke |
| unknown event | synthetic | necessary | necessary | not necessary |

E2E does not replace the contract. It examines some expensive paths. Fixtures
and reducers give a fast diagnosis for all combinations.

## Sub-workspace navigation

Claude, Codex and Antigravity workers use the same groups with one level, a
provider mark and one conversation for each delegated workspace. Cursor can
coordinate these workers, but Cursor cannot be a worker at this time. External
MCP clients have no local parent workspace. Remote shares keep flat navigation.

Tests: `src/workspace-hierarchy.test.ts` and the delegated-conversation tests in
`crates/core/src/board.rs`. Earlier workspaces with more than one tab keep their
tabs. [ADR 0099](../decisions/0099-sub-workspaces.md) describes how the
hierarchy becomes flat without a change to provider or MCP ownership.

## Built-in delegation MCP

Claude, Codex and Cursor can opt in to the bundled `prometeu` MCP. Then they can
make a Claude, Codex or Antigravity worker in an isolated workspace. Cursor is
not a worker provider. Each coordinator must name the `provider` of the worker. A
coordinator does not give its own provider to the worker.

`list_agents` reports the live catalog of each worker provider. Before Prometeu
makes the workspace, it compares the `provider`/`model`/`effort` selections with
this catalog. Thus it rejects a model from a different provider with a hint, and
the model does not go to an incorrect CLI. Antigravity workers have no approval or
question prompts. They run with automatic execution, unless the copied action
profile is Ask. Ownership, execution IDs, the reject rule for sends while busy, file
access and canonical history use the same backend implementation.

Claude receives a private strict MCP file. Codex receives the stdio command and
the private environment wrapper. Cursor receives the same stdio command and
environment through ACP `mcpServers`. Workers start with empty MCP, plugin and
skill selections. Background reports come only from the current
`background.changed` signals of each adapter. An unknown set is different from
an empty set that Prometeu sees.

An execution stays `running`, with its result recorded, until those tasks drain.
Thus it does not report completion at a time when Prometeu continues to reject
sends as busy.

The worker providers also use the same configured setup and Run controls and
dock logs with a limit. They also use the same process and exit status, and an
explicit opening of the desktop preview. These tools accept only an owned `agent_id`. There is no
execution path for one provider only, and there is no argument for a free shell
command or URL. Setup continues to run when Prometeu makes the workspace.
Runtime status is temporary and does not show HTTP readiness.

Tests: the `delegation.rs` and `embedded_mcp.rs` unit tests, the built-in
materialization tests in `mcp.rs`, and the opt-in picker test in
`e2e/tools.spec.ts`. `delegation.rs`/`embedded_mcp.rs` examine workspace
controls and log limits. `crates/process/src/terminal/tests.rs` examines
PTY exit and output retention. `e2e/browser.spec.ts` examines preview navigation
on Chromium, and selected browser interactions run again on WebKit.

Browser tests use the catalog mock and do not run models. For live model
delegation, installed and authenticated CLIs are necessary. Automated tests do
not use model credits. Refer to the [contract](../contracts/embedded-mcp.md).

External local MCP hosts use the same stdio protocol and tools with all worker
providers. They do not have a coordinator conversation in Prometeu. Registered
clients have an explicit project scope and credentials that a person can revoke.
These credentials stay after the desktop starts again. The clients also name
the `provider` (Claude, Codex or Antigravity). The provider starts with its CLI
defaults, unless the client gives `model`/`effort`.

Tests: the authorization and compatibility tests in `mcp_access.rs` and
`delegation.rs`, and `tests/mcp_client.rs` for stdio subprocess
registration and socket rediscovery with a fixture. That subprocess test does
not use a live provider or a native webview. Refer to
[registration](../contracts/embedded-mcp.md#external-client-registration).

## Antigravity verification boundary

The integration is for the installed agy 1.2.7. Its native headless protocol is
different from Gemini CLI ACP. A live prompt with a cached account returned
`AGY_OK`. `agy -p /usage` returned the remaining quotas by model group. A TSV
fixture and parser regression tests record this result. `agy models` returned
14 slugs that the account can see.

Live tool execution, a new start and resume with a remembered marker, and SIGINT
gave correct results. Adapter fixtures and tests examine text deltas, tools,
completion, interruption, explicit native resume and the `--add-dir` worktree
argument. They examine the argument when a session starts and when it resumes.
A future check with the installed runtime must examine the working directory of
the tool, not only the tool success. The browser mock uses only the generated V1
fixture.

Prometeu cannot give interactive approval responses for agy, management of more
than one Google account or a direct browser-login API that a test examined. Initial plan
mode and hub selections are unavailable. This replacement does not complete the
full account and approval scope of the initial issue. The agy CLI continues to
control external login and account changes, and Prometeu gives no guarantee of an
isolated profile. Prometeu keeps earlier Gemini accounts and transcripts, and
they cannot start a different runtime accidentally. Refer to
[ADR 0052](../decisions/0052-antigravity-runtime.md).

## Local diff review notes

| Behavior | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Notes on lines, ranges and files, local persistence, draft batch and portable history | Shared application behavior | Shared application behavior | Shared application behavior |

The [review contract](../contracts/diff-review.md) uses usual text. It adds no
new provider capability, IPC or V1 event. `review-comments.test.ts`,
`review-context.test.ts` and `review-store.test.ts` examine rules and
compatibility. `e2e/git.spec.ts` and `e2e/mobile.spec.ts` examine representative
keyboard, reload and portable history behavior. These tests do not show that a
provider obeys each note or that the model received a queued batch.

## Cursor verification boundary

Cursor is one more desktop provider. The comparison of Claude, Codex and
Antigravity above keeps its current scope. After V1 normalization, Cursor uses
the same paths for Git, worktrees, transcripts, sharing and presentation.

| Capability | Cursor | Evidence |
| --- | --- | --- |
| Installed CLI and model discovery | ACP capability probe. The accurate ACP model IDs that the account can see | `cursor.rs::installed`, `protocols::cursor::query_catalog`, ignored live smoke test |
| Account | One external login that the person attached explicitly. An isolated selection. A person can remove it | core account round-trip test. Shared account UI |
| Streamed text, thinking, tools and errors | Native ACP translated to V1 | recorded `cursor/fixtures/tool.ndjson`, protocol tests, `src/conversation.test.ts` |
| Resume and interruption | Native load with replay suppression. ACP cancellation. After a handshake failure, Prometeu disconnects, so the next input starts the process again | protocol tests, with the pipe release after a handshake failure. The ignored desktop `live_cursor_round_trip` |
| Permissions, questions, plan approvals | Current request cards. Option IDs operate independently of the click order, and plan decisions are explicit | protocol tests include a reversed multi-select and free text. A live smoke test examines a permission round trip and the initial plan mode |
| Attachments | Current local path mentions. No binary upload | shared presentation contract. No isolated test of Cursor image understanding |
| MCP from Prometeu | Hub selection encoded as ACP `mcpServers` on new and resumed sessions. The configuration of Cursor continues to apply | `tools` ACP encoding tests, `session::tests::tool_resolution_uses_the_tab_provider_and_configured_claude_home`, `agent_launch` Cursor tool injection. Native Cursor configuration continues to apply |
| Plugins and skills from Prometeu | Unavailable. If the workspace has no axis, Prometeu stops inherited packages. Explicit selections that are not empty fail | the same session test examines how Prometeu stops and rejects plugins and skills |
| Account quotas | Dashboard API with the CLI login. Unified weekly limits or explicit earlier individual monthly limits | `cursor/quota/tests.rs`: map, failure and compatibility tests. The ignored `live_cursor_quota` examines the signed-in account |
| Token and cost meters for each turn, context and compaction controls | Unavailable | descriptor capabilities. Prometeu does not add readings |
| Subagent lifecycle | No background signal with an isolated check | the native prompt terminal ends the turn |
| Native terminal recovery (macOS) | The host restores missing native tails before it seeds the history. It commits live batches before publication. Native IDs stay after derived JSONL rewrites. A correct completion must have the accurate request, a private pre-ACK witness, a post-hooks marker and an empty composer. Content that the projection does not include prevents coverage | `protocols::cursor_store` synthetic protobuf and projection tests, `terminal_agent_cursor_store` copied DB/WAL tests, `terminal_agent_observation/cursor/tests.rs`, pre-ACK hook tests and `chat/terminal/ingestion/tests.rs`. Examined on CLI `2026.11.13-eacade1` and accepted on newer builds. Smoke tests of the scratch store and the request identity, with hook permission |
| Native terminal interruption (macOS) | An authenticated root aborted hook that agrees with the durable source request commits `interrupted`. A restored draft does not keep the spinner on. Subsequent source or tool activity, or a completed hook for that request, cannot open it again or complete it successfully. New prompts cancel earlier proof | `cursor_interruption_*` observer regression tests and `journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet`. The pinned native Ctrl+C smoke test and the local Escape source inspection above |
| Native terminal running feedback (macOS) | A newer examined busy composer after a native submission boundary starts the Working state before source persistence. Source capture claims the same cycle, and earlier paints cannot open an ended cycle again | `cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`, `cursor_working_observation_requires_its_current_complete_busy_composer`, `observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once` |

Native-terminal preparation resolves the workspace one time. It gives the same
physical cwd to the PTY, the Cursor workspace argument, the transcript slug and
the store hash. `preparation_captures_profiles_and_resume_state_without_starting_agents`
examines symlink traversal on new and resumed launches. This repairs a
Tauri scratch condition: `/tmp` and `/private/tmp` selected different stores,
although the native response came. The path regression test does not include
one more model run or UI run.

Native check uses Cursor CLI `2026.10.22-27dcf5f`. A live prompt in a
scratch workspace ran `pwd`, wrote `CURSOR_OK` and remembered a marker after the
process started again. ACP cancellation returned `cancelled`. Fixtures examine
question and plan decisions. This page does not record a manual native-webview
interaction. The current browser journeys for accounts and the launcher examine
the shared controls without an E2E matrix for each provider. Refer to
[ADR 0095](../decisions/0095-cursor-acp.md).
