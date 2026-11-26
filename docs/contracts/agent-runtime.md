---
type: Contract
title: "Agent runtime contract"
description: "Agent discovery, capabilities and the port that starts and controls agents."
status: stable
---

# Agent runtime contract

This contract is in force. ADR 0003 implemented the identity and the
capabilities. ADR 0002 implemented the canonical events.

## Process-boundary verification

`codex/process_tests.rs` runs the actual `codex::Link` and `chat::output_lines`
against a controlled Node subprocess through stdin, stdout and stderr. It
examines these conditions:

- Queued input during the initialization.
- UTF-8 characters that the pipe writes divide.
- Incorrect and unknown lines.
- An interruption.
- Output that is not completed at an exit code that is not 0.
- A new process that resumes with the provider thread identity that the adapter
  saw.

A deadline kills a peer that does not answer, and the test reaps each child.

The peer is synthetic. It is explicitly not a record of a CLI session. It uses no
provider account, network or model. The tests prove the message frames and the adapter
behavior. They do not prove these items:

- The Tauri publication.
- The setup of the account and the configuration.
- The stop of actual provider child processes by the operating system.
- The compatibility with an installed provider version.

Claude and Antigravity keep their current adapter and fixture tests. This subprocess
scenario does not prove those transports.

This contract specifies the boundary between Prometeu and an agent CLI. It is
not an API for language models. It tells about local processes that have their
own catalog, session, protocol and capabilities.

## Identity

The provider and the model are different concepts. A model is part of a
provider. After the app loads the catalog, it must not use the model name to
find the provider again.

```ts
type ProviderId = "claude" | "codex" | "antigravity" | "cursor" | "gemini";

type AgentModel = {
  id: string;
  label: string;
  efforts: string[];
  additional?: boolean; // Absent means visible in the standard picker.
};

type AuthMethod = { id: string; kind: "browser" | "external"; label: string };

type AgentDescriptor = {
  authMethods: AuthMethod[];
  unavailableReason?: string | null;
  accountNotice?: string | null;
  id: ProviderId;
  label: string;
  installed: boolean;
  models: AgentModel[];
  capabilities: AgentCapabilities;
};
```

When the app adds a provider, `ProviderId` gets an explicit new value. If a previous
or unknown value comes from the disk, the app uses the default provider. It
does this only during the storage migration. New code compares all values explicitly.
The app keeps the known retired value `gemini` without a fallback, and that
value cannot operate. The JSON field continues to have the name `agent` for
compatibility, but its normalized value is
`"claude" | "codex" | "antigravity" | "cursor" | "gemini"`.

## Native preparation

The desktop injects the provider preparation independently of the conversation
spawn. Native adapters return the configuration, the transcript ownership and a
connection factory. The launcher supplies the process input and control. The
adapters keep the interruption policy and the turn-wait policy. This adds no
IPC payload. The Codex translation is in `prometeu-protocols::codex`. The
desktop supplies the language and the client version. See the
[application-core contract](application-core.md#sessions-launch-and-workers).

## Native terminal conversations

The conversation mode does not change with the provider and model identity
([ADR 0100](../decisions/0100-conversation-transport-identity.md)). When the
value is missing, the tab keeps the current chat transport. The app resolves
the board default when it makes a tab, and it saves the value on the tab. A
resume uses that saved value. macOS shows **Settings → General → New
conversations**, with Chat and Terminal options for new conversations. This
includes action tabs. A change of the default does not change current tabs and
does not start their processes again. Terminal mode uses the installed native
CLI for Claude, Codex, Antigravity and Cursor. Linux rejects terminal mode with
`err.terminalAgent.unsupported`, and its control is hidden.

The version gates are minimum values
([ADR 0100](../decisions/0100-conversation-transport-identity.md)). The launch
must have Claude 2.1.326, Codex 0.169.0, Antigravity 1.5.9 or Cursor 2026.11.13.
The native stores, the composer readiness, the interruption controls and the
completion fence accept the build that we examined and all newer builds. The versions
below tell where we examined each behavior. They are not a maximum.

Each local tab renders only its saved mode: the native TUI or the structured
Chat interface. Terminal tabs have no second transcript screen and no product
composer that a person can edit. The native terminal fills its area, and no
controls float above the output. These menus show the shared actions, MCP,
plugins, skills, context, dictation and remote access in a dialog:

- The workspace menu at the right of the central tab bar.
- The context menus of the tabs.
- The menus in the desk header.

Quotation and manual resume are available through the same menus.
A person can see the collaboration notices that need an action without that
dialog.
Model and effort changes stay in the CLI. The native font and color defaults
follow the
[terminal state contract](terminal-state.md#terminal-query-ownership). Native
color changes stay after a replay.

The canonical history continues to supply the storage, the status, the
notifications and the remote share. Remote viewers continue to use the
transcript, because the PTY is local. When a person opens a stopped local
terminal tab, the app automatically resumes the saved native session and
shows its screen again. This also occurs after the app starts again. The app sends
no artificial message. Manual resume is available only when the connection
does not make a running process. The checks of the identity and of submissions
that are not resolved continue to apply.

The Claude transcript ingestion removes validated base64 image bytes in message
content. The ingestion removes images in `tool_result.content` (also images that
`Read` returns) only when the record cannot fit the 1 MiB JSONL record budget
with them. Smaller records stay the same byte for byte, because saved journals
find records by a digest of that value. The omission keeps the tool identity,
the adjacent text and the image metadata. Incorrect removed images and ordinary
text that is too large continue to cause a verification failure. Live ingestion and
cold resume use the same filter. Thus, an image read does not stop a resume of
the native history.

Queued product messages show an indicator for context that waits. This
indicator is in the action group of the central tab bar or in the desk header.
The agent status does not change it. A person can open the full text from
the tools dialog. Input that waits does not use space for a transcript, a
footer or a large readiness notice.

The [provider matrix](../quality/provider-matrix.md) records the macOS
Tauri responses, the canonical history, the cold resume and the stop. It keeps
them different from native CLI smoke tests, automated tests and the remaining
checks for each provider.

`prometeu-protocols::terminal_status` reads OSC 7501 and does not change the
rendered bytes. It accepts BEL and ST at the end of a sequence, across chunks of
all dimensions. It limits each sequence to 4096 bytes, validates encoded text on
one line and keeps explicit root and child records. Only a change from working
or blocked to done or error can report the completion of that record. Idle,
clear, eviction and no output cannot. A child completion does not tell that the
root is completed. These observations contain no channel for a permission
response and no conversation text.

Only the live native reader that has the advertised states sends the query
response. A snapshot replay does not answer queries. In each live output batch,
the PSP response comes before the emulator replies. Claude uses its DA response
as the barrier for capability discovery. The native reader updates the screen
independently of the capture, the input and the tab publication. A different
reply queue with a limit keeps this sequence. Thus, a long input cannot stop
the output. If the queue is full, the app closes the connection. Thus, no reply
goes missing. The implementation follows the
[Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status).

The preparation in `terminal_agent_launch.rs` builds interactive CLI commands
with the selected account and the native tool materializers. Native `--help`
queries and synthetic tests are different from actual model runs. The native
transport operates together with canonical content, durable queues, telemetry,
sharing, screen restoration and notifications. Correct preparation tests or
parser tests alone do not prove these product flows.

Claude Ask selects the explicit native manual policy. When Ask operates together
with Plan, the preparation gives `--permission-mode plan` without the flag that
makes the bypass available. That flag opens one more bypass consent screen. New
start tests and accurate resume tests examine that combination.

The interactive MCP selection of Cursor uses a private native plugin that
contains `.cursor-plugin/plugin.json` and `mcp.json`. The current MCP
materializer resolves the selected servers. The terminal adapter changes the
ACP environment and header pairs into the native objects of Cursor. The values
stay in files with mode `0600`. They are not native command arguments. The native CLI continues to control
the parts of its configuration that are not in this injected selection. The optional `terminal_agent_launch::mcp_smoke` test
examines the selected stdio initialization, the tool discovery and the delivery
of the environment on `2026.11.13-eacade1`. It does no inference and no tool
execution. HTTP and SSE activation and actual MCP tool calls must have a different
native test.

The [private native engine](native-codex-engine.md) of Codex prepares the accurate
thread through app-server RPC. It connects the official TUI through a private
Unix socket. The isolated test without inference examines Ask and Plan on a new
start and on a cold resume. Live RPC tests and recovery tests examine the
canonical content and the explicit responses. A full Tauri application
verification is a different level of evidence.

`chat/terminal.rs` owns the PTY as a `Chat` transport. A different emulator
reader lets an input transaction wait for the painted draft while the output
continues. Live UI events and the canonical observation worker wait for the tab
publication before they report the identity or the exit. One waiter owns the process and reaps the process before the closure.

The waiter operates on its own task (`TerminalExit` in
`crates/core/src/terminal.rs`). Thus, the read stops at EOF or at `EXIT_DRAIN`
after the process exit, at the first of the two. A child process that the agent
started can keep the PTY open. Without this rule, the tab can stay in its
running state. When this app kills a conversation (`chat::kill`), the
app first removes its registry entry. Thus, its tab gets the stopped
state at that point, and its subsequent exit publishes no data.

An explicit native root `idle` report can release the startup input. A spawn,
the first output, a hook with the name `SessionStart` and no output do not tell
that the input is prepared. Programmatic input uses bracketed paste and rejects
terminal control characters. Raw local keyboard input stays native.

Local native insertion is a draft operation. It is different from the
submission of queued messages. File selection, drop and clipboard files put
paths directly into the terminal. **Insert into terminal** puts staged context
from the tools dialog into the terminal. The two paths obey these rules:

- They record the initial tab, the terminal connection and the process
  generation.
- The terminal must be prepared for input and have bracketed paste enabled.
- They write only the paste envelope, without Enter, an automatic resume or a
  change of target.

Each image uses its own bracketed paste that contains a literal path, between quotation
marks when necessary, without an `@` prefix. Other files keep document mentions. The
app puts the files in sequence before other staged context. Each correct
write removes only that file from the draft. Thus, after a subsequent failure,
the remaining files and context stay available, and the correct writes do not
occur again.

Browser context, review notes and dictated text can have an insertion failure.
In that condition, these items stay staged in the tools dialog of the initial
tab. A correct paste keeps
all context that a person added while the write waited. A review submission
waits for canonical user content that agrees with it. The app does not guess
the submission from a correct PTY write or from the terminal paint. See the
[conversation flow](../architecture/conversation-flow.md).

Native slash commands, model changes, effort changes and permission-mode
changes use the controls of the CLI. The app keeps the model and effort
defaults for the launch. But it does not show those selectors again in the
local native conversation. The shared transcript view does not show these
items:

- The slash suggestions of the provider.
- The compact and context actions.
- Allow always.
- The plan action that enables the bypass.

User Actions continue to expand before the command validation. Usual prompts
and the answers to each Claude or Codex request keep their current paths. The
app rejects manual slash input before the composer discards its draft or
attachments. Expired callbacks validate the conversation mode and the
connection again. The encrypted shared mode applies the same boundary to
viewers. The owner examines the text again before the app adds a prefix to
remote text.

The Cursor TUI that we examined must have a send in two phases:

1. The host reserves native input and writes bracketed paste without Enter.
2. The host waits until a newer emulator snapshot shows the painted
   acknowledgment of that accurate draft.

The host writes Enter one time, only after the acknowledgment. A deadline or a
retirement does not authorize Enter or a new try. A draft that the
host did not send stays on the screen. The reader and the query replies continue to
operate while the capture and the input stay reserved. The packaged-helper
transport test examines the paint barrier, one Enter and the retirement between
the paste and the submission.

The window-focus report of Cursor controls its painted caret independently of
the input availability. The empty placeholder that we examined stays dim without the
focus, also its arrow. A literal draft has a usual arrow. Thus, a paste that
the host owns exclusively can acknowledge its accurate content without an inverse
caret. Disabled ordinary text and modal footers continue to reject the
admission. The native wrap sets the height of the paste. A workspace path that
wraps in the footer does not.

Native idle and an available composer are different gates. After local edits,
an older empty screen cannot authorize queued input. A newer snapshot must
prove that the input is empty, with its actual cell attributes. If not, the native
CLI must accept that draft. Literal text that agrees with a placeholder
stays user input.

Codex 0.169.0 must have its dim native placeholder, the focused cursor and the
current footer, and also native idle. A literal draft that is the same at Home and a
draft that contains only space characters cannot release the queue. The pinned captures
(new start, resume, literal, Home and clear) examine this difference without
model inference. Paint alone does not tell that Codex has activity or a
completion.

Antigravity 1.5.10 accepts its focused composer on one line when native idle,
full-width borders and the accurate idle footer are there. A new process can
accept its first input before a PSP report in one condition. The app examined
that same empty composer and saw no input or activity. The app cancels this one-use
startup gate before a composer write, also local keyboard input. Native
activity also cancels it. A resume and all subsequent input must have the current
native idle authority. An empty new paint cannot complete a turn or give back the
startup gate.

ASCII space characters at Home are equivalent to empty, because the pinned native submit
handler removes them. When there are no paste placeholders, the handler
discards an empty result without a command. The native mode hint is empty only
with its accurate pinned text and truncation and the palette-8 foreground. An
actual literal capture at Home uses the default foreground. The app obeys an
explicit `NO_COLOR`. A hint without color cannot release programmatic input,
because the app cannot tell it from a draft. Chips, drafts on more than one
line, menus, missing metadata and paints that are not completed stop the
admission. This proof releases the local-edit fence. It does not change
the text that the app sent and does not guess how a hook normalizes its user-message alias.
The binary hash, the symbol addresses and the captured geometry are adjacent to
the
[readiness fixtures](../../src-tauri/src/native_ui_readiness/fixtures/README.md).

The captured native interruption banner immediately above the top composer
border of Antigravity is compatible with empty input. It is not a draft and
does not stop the admission. A newer empty paint that the app examined releases the local
Escape input fence. Separate native idle evidence lets queued product
messages continue. The banner alone does not tell that the agent is idle or
that a completion was correct.

Startup trust keys before a person can see a composer do not reserve a draft that is
not there. After the app sees root PSP reports, late hook activity cannot replace
that state. Hooks and terminal bytes come through separate queues. The
PSP-enabled title of Claude can have no idle prefix. Thus, the authoritative
status replaces only the title test. The composer, authentication and modal
checks stay.

Antigravity instructions that the preparation keeps become native rules in the
private added root of the process. Cursor uses its native `beforeSubmitPrompt`
`additional_context` carrier. Thus, updated instructions also apply after a
resume and to input that a person types directly in the TUI. The private hook
sends JSON data through `/bin/cat`. Instruction contents are not shell source
and not a permission override. The carrier limit of the examined Cursor version
is 10,000 UTF-16 units. Larger instructions cause an explicit failure before the launch.
These paths do not change the prompt that the app sends.

Claude and Codex can send each application approval or question response
through their native decision channels. By product decision, Cursor and
Antigravity decisions stay in their own TUIs. Shared viewers see that the owner
must answer. Responses that a provider cannot accept cause a failure and do not close a
canonical request.

Native interruption uses `terminal_agent_controls::NativeControls`. This value
is connected to the examined CLI version and the screen of the current
generation. A default key alone is not sufficient. The active composer or footer
must show that binding, and the host must have seen an active turn. The Stop
control of the application uses Escape for Claude, Codex and Antigravity, and
Ctrl+C for Cursor. Local keyboard input keeps the bindings of each CLI. The
pinned Cursor also accepts Escape with an empty prompt when no other native
action uses it.

Unknown versions, paints that are not completed, hints that do not agree and
unknown layouts return no control. The host uses the interruption one time for
each active cycle, because a second Ctrl+C can stop Cursor. The MCP startup
cancel hint of Codex is intentionally different from a model-turn interruption.
The [provider matrix](../quality/provider-matrix.md) records the current
fixture and native smoke tests.

The versioned busy composer of Cursor also supplies a Working observation
quickly after a native input boundary. The host writes an Enter while the agent is idle.
Then a newer, full paint must show the busy footer and the caret that
agrees. The key alone or a screen from the previous turn is
not sufficient. This observation starts the spinner without a new user message.
The subsequent durable source capture claims the same execution cycle. A
closed cycle cannot open again from a previous busy paint.

The authenticated root `stop` / `aborted` hook of Cursor sets the canonical
outcome as `interrupted`. This occurs when its generation agrees with the
current durable source request and the ingestion is up to date. New prompt
boundaries cancel older evidence. Their barrier for the generation that waits
opens only after the durable source capture that agrees. Subsequent requests
that the source approved do not need that hook again. An older late hook
cannot replace the bounded Stop evidence of a request. Subsequent
`completed` hooks for the cancelled request cannot replace it.

The host records that outcome in the journal before the publication. An empty
composer is not necessary, because Cursor can put back the interrupted prompt as
a draft. The app does not send the draft. Raw Escape does not change the status. Late source content or tool hooks from the cancelled request do not
start the spinner again. This path makes no notification of a correct reply.
The [Cursor contract](cursor.md#native-interruption-evidence) gives its source
sequence and its regression tests.

An explicit root `Working`/`Blocked` → `Idle` report closes the observed
activity. It does not make a `turn.completed` event. AGY 1.5.10 uses this
sequence after Escape. The key alone does not change the status. Late source
content from the closed activity continues to update the history, and the work
does not open again. A new accepted input starts the live execution again.
Active child tasks and actual requests without an answer keep their own state.
When the app removes or loses a record, the state stays unknown. This end of
the activity
makes no notification of a correct reply. The current Done completion path does
not change.

A root idle after `Done` or `Error` also makes late source content quiet. The
completion ticket that waits does not change this. AGY 1.5.10 can report
`Working` → `Done` → `Idle` when Escape stops a turn after file
exploration. Its native `Done` label does not prove a correct reply. The host
keeps the source barrier for the completion. Late reasoning and tool content
cannot open an idle tab again. New accepted input removes the quiet-content
boundary. A late capture of the same native input keeps it.

`terminal_agent_hooks.rs` supplies the internal `--terminal-agent-hook` helper.
The native conversation transport configures one bridge for each PTY
incarnation. The helper sends a bounded native hook payload through a private
authenticated Unix socket. The session, the process nonce and the native
identity connect an observation to one launch. The content that is
necessary to make the conversation again stays in this private channel. The
app does not send credentials to the observer and does not write them to a log.

When the bridge closes, it waits for its listener and closes the sender. The app can
read observations that are in the queue before the publication of the process
exit. Usual observation hooks send no permission decision. Claude
PermissionRequest also lets the app send an explicit response for one
invocation, and it keeps its native dialog. A timeout and a retirement make no
decision. See [native Claude decisions](native-claude-decisions.md).

The worker records correct programmatic submissions under the input lock.
Then it reads the hooks and the bound transcript before it releases that lock.
Its callbacks stage observations. The publication and the telemetry operate
after the release, so that the current capture lock sequence stays. Prompts
that the native terminal accepts directly use the usual canonical input and
capture path. The app does not write them to the PTY again.

A resumed transcript must complete its silent baseline before the app accepts
programmatic input. Native input readiness does not change this. At the
exit, the worker closes the hook sender and reads the last observations. Then it
publishes the change of the session lifecycle. The readiness and decision rules
of each provider stay at the native edge. The restoration uses the shared
[terminal state contract](terminal-state.md).

After the loss of the process, a terminal tab without a native session identity
cannot resume. It has an explicit failure and does not silently make a new
conversation. A prepared input that is not resolved also stays blocked until
native source evidence shows if the agent accepted it. An empty transcript on the screen
does not prove that the app did not try a write. Only the live owner
of a submission that had a failure before a write can discard that intent. These
limits do not stop the resume of a current, bound native history.

`prometeu-protocols::terminal_transcript` keeps the replay different from live
observations. It reads JSONL records that are not completed and tells when a full snapshot
replaces the content after a rewrite. The binding must have an explicit native
session and transcript path. Its synthetic fixtures use the provider formats.
They are not recordings of user conversations. The initial replay, a truncation
and missing records do not tell that a turn is completed. The desktop must keep
the events that the app owns as different items when it replaces a native projection.
It must not send the replacement replay through the live pump.

AGY 1.5.10 terminal content uses its
[initial native store](native-antigravity-store.md), not the truncated JSONL
summary. The Cursor reader also uses its copy of the DB and WAL. Tool results
keep the explicit native call identity and the approved full output
files. The current NDJSON Chat adapter does not change.

For the Claude and Antigravity versions that we examined, the host uses three
items together:

- The native successful-stop fence.
- The accurate root PSP Done report.
- Its current execution ticket.

The fence must have these items:
- A new raw user boundary.
- The stored last assistant source that agrees.
- A full poll that started after the Stop and the Done.

A private report number correlates the ticket. It does not make the submission
counter of the observer equal to the execution cycle. Reports that come during
an app input reservation wait until the app records the submission that it waits for.
The app commits canonical content before it publishes the related execution
reports.

The completion callback validates the ticket again under the usual capture and
input lock sequence. It commits a durable source receipt before it closes the
execution or publishes the completion. A replay or a duplicate receipt cannot
cause the results of the completion again. This proves only the success path
that the app can operate. Native errors, approvals and provider versions that
we did not examine must have their own authoritative evidence. The pure fence, the
observer boundary tests and `chat/terminal/ingestion/tests.rs` examine these
conditions.

The success path of the pinned Cursor version uses a different method. It
records a private transcript stamp before it acknowledges its authenticated
Stop hook. The observer must have these items:

- An accurate native request identity.
- Complete committed source coverage.
- A changed stable JSONL revision that ends with the post-hooks success marker
  of the provider. A previous marker is not sufficient.

The host then examines the current empty composer and the guard for input that
waits, under the canonical capture and input sequence. Only the durable source
receipt lets the completion results occur. The optional run records of Cursor stay
optional. The host examines them when they are there. See the
[Cursor contract](cursor.md) for the writer sequence of each version, the trust
boundary and the remaining limits of the tests. The interrupted outcome uses
the authenticated aborted hook and the durable request identity. Its native
marker can come before the hook, and it does not use this success fence.

## Model discovery

`agents` finds installations and capabilities, and returns empty model lists.
`agent_models({ agent })` independently queries the selected account. It returns
`{ models: AgentModel[], fetchedAt: number }`, with a Unix time in
milliseconds. Failures reject with `{ code }` from
`err.modelsCatalog.noAccount`, `unavailable`, `timeout`, `invalid` or `failed`.
A correct empty list is different from an error. No raw vendor response and no
credentials go through this boundary.

Claude uses `list_models`. It does not include disabled entries and the default
entry without a name. Codex uses the app-server `initialize`, `initialized` and
all `model/list` pages with `includeHidden: true`. `model` is the launch
identifier, `hidden` becomes `additional`, and the native reasoning levels stay.
The CLI cache is not a source for the picker. Antigravity uses `agy models`. Its
labels do not tell that the model has effort levels. The discovery sends no
inference prompt. Each query has a limit of twenty seconds. The discovery reaps
its subprocess group after a correct result, a failure or a timeout. The
injected query port applies that deadline to all writes and all pages, with a
limit of 1 MiB for all of stdout.

The frontend refreshes the catalog at the start and after account changes. It
also refreshes it when a person opens the picker and five minutes went by after
the last refresh. A person can also refresh it manually. The frontend merges
concurrent requests for the same provider. Account changes clear the catalog
generations and discard late responses. After a refresh failure, only the last
correct catalog of the current generation stays in memory, with a mark that
tells that it is expired. No catalog stays after the app starts again. A
correct refresh replaces the list, also when it removes items or is empty.

Model selections always contain `{ agent, model }`. The app does not use names
to find the provider identity again. The only exception is the explicit
migration of legacy preferences. Historical Claude aliases give labels to previous selections,
but they cannot tell that a model is available. Unknown saved selections stay
readable, but they do not become new models that a person can select. For new
workspaces, a person must explicitly correct saved models that are not
available or efforts that the model cannot use. A refresh alone does not change
a workspace, a tab or an action profile.

Effort menus contain the provider default (an empty string in stored
selections) and only the levels that the native CLI advertises. Unknown level
names stay usable. When a person changes the model, the app keeps an effort
that the new model can use. If not, the app sets the effort to the native
default and shows an indication. When there are no levels, the control is
hidden. The exception is a historical value that is not empty and must have a
correction. Claude does not get a synthetic `ultracode` option. The app reads a
historical Codex `ultracode` as native `ultra`. Incorrect historical selections
stay on the screen until a person explicitly edits them.

See [ADR 0053](../decisions/0053-live-model-selection.md) and the
[preference format](persistence.md#model-selection-preferences).

## Capabilities

```ts
type AgentCapabilities = {
  initialPlanMode: boolean;
  workspaceMcpSelection: boolean;
  workspacePluginSelection: boolean;
  resume: boolean;
  compact: boolean;
  contextReport: boolean;
  usageTokens: boolean;
  usageCost: boolean;
  contextWindow: boolean;
  approvals: boolean;
  userQuestions: boolean;
  attachments: boolean;
};
```

This is the minimum set that the current interface uses. A capability goes into
this set only when it changes a behavior that the application gives. Protocol
data, for example the name of a JSON-RPC method, are not capabilities.

`usageTokens` enables the measured consumption of a turn. `usageCost` enables
only a normalized estimate that the CLI reports. Prometeu does not calculate
prices. `contextWindow` enables the occupancy meter only when the app also saw a
usable current window. Claude and Codex show the meter. Antigravity supplies
usage without a window or a cost. When a capability is available, an unknown
measurement does not become 0. The
[provider matrix](../quality/provider-matrix.md) records the normalization and
the regression evidence.

The skills selection uses the plugin pipeline. Thus, the current
`workspacePluginSelection` capability also controls it. The core resolves the
global and project layers, and this adds no capability.

Some capabilities can change with the CLI version or the model. In that
condition, the descriptor that the runtime returns is the source of truth. The
frontend does not keep a second table. Before the discovery, or after an IPC failure,
the frontend bootstrap shows only Claude as installed and shows no optional
capability.

## Execution account

The account selection is global for each provider and is not part of
`SessionLaunch`. The adapter records the selected profile at the spawn. The
process keeps its ID and revision until the turn is completed. When necessary, the next
message resumes the same transcript with the current selection. The login, the
profile and the credentials stay at the edge of the provider. The core receives
only the normalized identity and state. When a person removes the active
account, the provider has no selection. A turn that started before can
complete. New sends return `err.account.noActive` until the next selection. See
[`accounts.md`](accounts.md).

## Session configuration

```ts
type SessionLaunch = {
  provider: ProviderId;
  model: string | null;
  effort: string | null;
  initialPlanMode: boolean;
  mcp: string[] | null;
  plugins: string[] | null;
  skills: string[] | null;
  cwd: string;
  resume: string | null;
};
```

The optional values have these meanings:

- `null` in the model or the effort lets the provider select its default.
- `null` in MCP, plugins or skills tells that Prometeu sets no selection and
  keeps the configuration of the CLI.
- An empty list tells that Prometeu adds no item from its hub. A global
  registry that the CLI loads itself stays under the control of the CLI.
- The three lists come resolved. The core composes the global, project and
  workspace layers. An adapter does not resolve layers and does not read the
  board.
- `resume` is an identity that the provider accepts and that Prometeu does not
  read. Prometeu can select
  it, as for Claude, or the provider can return it, as for Codex.

The core validates `SessionLaunch` against the capabilities before it starts the
adapter. The adapter must not silently correct an incorrect combination. If the
adapter cannot write a selected MCP, plugin or skill configuration, the launch
stops with an error before the spawn. If the adapter cannot enable a declared
hook, the launch stops with an error before the thread opens. A selected MCP ID that is not in the registry is
an example. The two adapters return `err.mcp.missing` and do not silently
discard it. A start without the requested behavior is not a correct fallback.
The full plugin contract is in
[`plugin-marketplace.md`](plugin-marketplace.md).

## Workspace launch resolution

The usual tab creation and the resume use `session::workspace_launch_with`. The
tool selection uses the shared `core::tool_resolution`. The desktop
`session.rs` composes these layers into the three `SessionLaunch` lists:

- The global layer from the board.
- The project layer from `.prometeu/settings.toml` of the primary repository.
- The workspace layer.

It does not include the items that the project declares if their hash is not
approved
([ADR 0045](../decisions/0045-layered-tool-selection.md)). The resolution uses
the provider that the tab uses, not the workspace default. This includes the
override of a new tab or the provider of an action profile. A model or effort
override in that provider keeps the resolved set. Task tabs continue to use
their frozen resolved profile. The regressions in `session.rs` examine these
selections for Claude and Codex.

The app records the resolved set at the spawn, as the execution-account rule
above tells. A change in a layer applies at the next spawn or resume of a
stopped process. It does not start a running session again. An idle
process also keeps its recorded set. A new message alone does not load the
tools again.

Provider adapters own the CLI flags. They use the tool artifacts through the
injected `prometeu-tools::StartupTools`. `session.rs` resolves the application
selections and gives `Launch` to the adapter. Plugins and standalone skills
share the package pipeline. The desktop `tool_materialization.rs` composes
`NativeTools`. The current argument fixtures and the injected startup tests
keep the flags, the resume behavior, the scope, the environment and the failure
sequence. See [tools and packages](application-core.md#tools-and-packages).

The Tauri commands that change the MCP servers, plugins and skills of a
workspace delegate to
[`workspace_tools.rs`](../../src-tauri/crates/core/src/workspace_tools.rs).
This application boundary validates the selection of each axis and updates the
explicit board state. It does not use processes and does not change other
tabs. The commands keep the native argument processing, the error translation
and the board publication. The tests operate without a Tauri application. They
examine incorrect payloads, selection changes and the tabs that stay. The
extraction keeps the layered IPC and the stored formats. See
[ADR 0050](../decisions/0050-tested-application-boundaries.md).

## Skill kickoff

The *Start with* option of the launcher (`Draft.kickoff`, `<package>/<skill>`,
empty for none) is a temporary item added to the layered resolution above
([ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md)). After the
app resolves the three lists for the first conversation, the core adds the
package of the skill to the axis of its ID: `skill-<id>` to `skills`, and a
plugin to `plugins`. An inherited `null` becomes a list with one item. No
global, project or workspace layer changes. Thus, subsequent tabs keep the usual
resolution.

`Tab.kickoff` stores the selection. A resume of that tab adds the package again
after it validates the skill against the installed catalog, as the creation
does. If the package does not contain the skill, or the package is not in the
hub, the resume continues. The package is not added, and the conversation gets
a `system.notice` with `level: "warning"` and `code: "kickoff.missing"`. The
backend renders its `detail` in the display language. Adapters receive usual
lists and write them as before.

The first message starts with one line that the app writes. This line gives these
items:

- The name of the skill.
- The name of its plugin, when it has one.
- The `[method] artifacts` path of the repository, relative to the working
  folder.

Then come the attachments and the prompt of the person. The backend renders the
line in the display language. The line does not give a path that the repository
did not declare. The option must have `workspacePluginSelection`. The core rejects
it for other providers with `err.kickoff.unsupported`. A skill that is not in
the installed catalog causes `err.kickoff.missing` before the app publishes
a card.

## Conceptual port

The design can use a trait, an enum dispatcher or grouped methods. The meaning
is more important than the syntax:

```text
AgentCatalog
  discover() -> AgentDescriptor[]

AgentRuntime
  start(SessionLaunch) -> SessionHandle
  send(SessionHandle, ConversationCommand)
  stop(SessionHandle)
  output(SessionHandle, RawProviderEvent) -> ConversationEvent[]
```

The adapter has these responsibilities:

- Start the executable and configure its environment.
- Change `SessionLaunch` into the arguments or requests of the provider.
- Write MCP, plugins and skills in the form that the provider uses, and do not
  show that form to the domain.
- Correlate the requests and responses of the external protocol.
- Change external output into `ConversationEventV1`.
- Change `ConversationCommandV1` into external input.
- Show failures with a stable code and diagnostic data.
- Stop the process and its child processes as the policy of the app tells.

These responsibilities are not part of the adapter:

- The selection of what the UI shows.
- The storage of the board state.
- The control of the team audience.
- The rendering of tools or Markdown.
- The global resume policy and the message-queue policy.

## Task profiles

[Task](actions.md) launches add stored instructions and a permission policy
(`ask` or `auto`). The session stores the resolved configuration. Claude gets
the added instructions through `--append-system-prompt`. Codex gets
`developerInstructions` when it starts or resumes the thread. Approval with
`ask` uses the usual mode of Claude and `approvalPolicy: untrusted` in Codex.
Previous launches keep the current bypass. The Codex configuration of a task is
isolated for each session. Thus, different profiles do not replace the files in the
same home folder.

## Compatibility

- A change only in the external protocol must change one adapter and its
  fixtures.
- A change in shared behavior changes this contract and the conformance suite.
- A new capability starts as `false` in the current providers until there is
  evidence and a test.
- A provider that is not available does not stop the catalog of the other
  providers from loading.
- A discovery failure must not show a capability that is not there. The
  fallback must be explicit and a person must be able to see it.

## Minimum conformance

When the capability is there, each provider must show these items:

1. A new session starts.
2. A resume does not duplicate messages.
3. A message gets a last answer.
4. A stream comes before the authoritative event.
5. A tool call gets its result.
6. A question or an approval gets a correlated answer.
7. An interruption stops the turn.
8. A compaction and a context report occur.
9. The process and its child processes stop.
10. An unknown external event causes no failure.

The current matrix of this evidence is in
[`../quality/provider-matrix.md`](../quality/provider-matrix.md).

## Antigravity CLI

`antigravity.rs` changes the native NDJSON of `agy` 1.2.7 and newer versions into V1. The
start uses `--input-format stream-json --output-format stream-json`. Each prompt
is an `event: user` with text content. `init` records the native identity,
`step_update` sends text and tool events, and `result` completes the turn. A
last response must not duplicate streamed deltas. The usage and time
counters from agy are cumulative. Thus, the adapter measures the local time
of the turn and does not show totals for each turn. Follow-up messages that wait
stay in the board until the completion. The adapter does not move a private
queue forward before the core records the completion of the previous turn.

`Tab.agent_session` stores `conversation_id`. `--conversation` resumes the
native context. The app replays its own V1 log, not raw agy messages. SIGINT
stops the process group. After the loss of the process, the next prompt
can start it again and resume. `agy models` supplies slugs and labels. The app
does not make synthetic effort options.

Headless control messages and approvals are not available. Usual conversations
use automatic execution (`--dangerously-skip-permissions`), the same as the
current default for Claude conversations. Explicit Ask task profiles keep the
agy policy. It is possible that agy denies commands that must have approval, without
an error. The tool state `ERROR` is the last state. A result that contains
`denied_actions` is an error, also if the native status is `SUCCESS`. The app
shows the translated permission message and does not show raw errors. Auto task
profiles also use automatic execution. Initial plan mode, hub tool selection,
compaction, context reports and structured questions are not available. Hub
selections that are not empty cause a failure at the adapter boundary. Local attachment
paths stay text.

Authentication methods stay descriptor data. Claude and Codex advertise
browser. Antigravity advertises `external`, with an `accountNotice` that tells
the limits. The previous Gemini identity is a retired storage marker. Each execution
refuses with `err.provider.retired`. See
[ADR 0052](../decisions/0052-antigravity-runtime.md).

## Shared native discovery

The provider descriptors and the model result types are in
`prometeu-core::agents`. The desktop uses `prometeu-protocols::catalog` for
bounded queries, pages and response validation. It supplies the query launcher,
the prepared command and the client version.

## Cursor

The app finds the additive `cursor` provider through `cursor-agent acp --help`.
Its descriptor shows these capabilities: external account connection, initial
plan mode, resume, approvals, questions, path mentions and hub MCP. The model
discovery uses the accurate ACP model IDs from a native session without a prompt,
with the current query deadline and output limit. Usage and context controls
for each turn, and plugins and skills that Prometeu controls, are not
available. Account quotas use a different dashboard query and the current usage
scheduler. See the [Cursor contract](cursor.md).
