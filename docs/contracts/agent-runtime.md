---
type: Contract
title: "Agent runtime contract"
description: "Discovery, capabilities and the agent execution port."
status: stable
---

# Agent runtime contract

Contract in force; identity/capabilities implemented by ADR 0003 and
canonical events implemented by ADR 0002.

## Process-boundary verification

`codex/process_tests.rs` runs the real `codex::Link` and `chat::output_lines`
against a controlled Node subprocess over stdin/stdout/stderr. It checks queued
input through initialization, UTF-8 split across pipe writes, malformed and
unknown lines, interruption, incomplete output at nonzero exit, and a new
process resuming with the observed provider thread identity. A deadline kills
a stalled peer and every child is reaped.

The peer is synthetic, explicitly not a CLI recording. It uses no provider
account, network or model. The tests prove framing and adapter behavior, not
Tauri publication, account/config materialization, OS shutdown of real provider
descendants, or compatibility with an installed provider version. Claude and
Antigravity retain their existing adapter/fixture coverage; this subprocess
scenario does not certify those transports.

This contract defines the boundary between Prometeu and an agent CLI. It is not
an API for language models: it describes local processes that have their own
catalog, session, protocol and capabilities.

## Identity

Provider and model are distinct concepts. A model belongs to a provider; the
model name must not be used to rediscover its provider after the catalog has
been loaded.

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

When a provider is added, `ProviderId` grows explicitly. An old or unknown value
coming from disk falls back to the default provider only during persistence
migration; new code uses exhaustive matching. The recognized retired `gemini`
value is retained without fallback and cannot execute. The JSON field is still called
`agent` for compatibility, but its normalized value is `"claude" | "codex" | "antigravity" | "cursor" | "gemini"`.

## Native preparation

Provider preparation is injected separately from conversation spawning. Native
adapters return configuration, transcript ownership and a connection factory;
the launcher supplies process input and control. Interruption and turn-wait
policy stay in the adapters, and no bridge payload is introduced. Codex
translation is shared through `prometeu-protocols::codex` by the desktop and the
[WSL runtime](wsl-runtime.md); the host supplies language and client version. See
the [application-core contract](application-core.md#sessions-launch-and-workers).

## Native terminal conversations

Conversation mode is independent of provider/model identity
([ADR 0100](../decisions/0100-conversation-transport-identity.md)). Missing values
retain the existing chat transport. The board default is resolved on creation
and saved on the tab; resume uses that saved value. macOS exposes **Settings →
General → New conversations**, with Chat and Terminal choices for new
conversations, including action tabs. Changing the default does not convert
existing tabs or restart their processes. Terminal uses the installed native
CLI for Claude, Codex, Antigravity and Cursor. Linux and WSL reject terminal
mode with `err.terminalAgent.unsupported`; their control is hidden.

Version gates are minimums ([ADR 0100](../decisions/0100-conversation-transport-identity.md)):
launch requires Claude 2.1.326, Codex 0.169.0, Antigravity 1.5.9 or Cursor
2026.11.13. Native stores, composer readiness, interruption controls and the
completion fence accept their verified build and every newer one. Versions
cited below record where each behavior was verified, not an upper bound.

Each local tab renders only its saved mode: the native TUI or the structured
Chat interface. Terminal tabs have no second transcript screen or editable
product composer. The native terminal fills its area without floating controls
over the output. The workspace menu at the right of the central tab bar, tab
context menus and desk-header menus expose shared actions, MCP, plugins, skills,
context, dictation and remote access in a dialog;
quotation and manual resume are available through the same surrounding menus.
Actionable collaboration notices remain visible without opening it. Model and
effort changes remain in the CLI. Native typography and palette defaults follow
the [terminal state contract](terminal-state.md#terminal-query-ownership), with
native color changes preserved through replay.
Canonical history
still feeds persistence, status, notifications and remote sharing; remote viewers
continue to use the transcript because the PTY is local.
Opening a stopped local terminal tab automatically resumes the saved native
session and restores its screen, including after app restart. It sends no
artificial message. Manual resume remains available only after attachment fails
to produce a running process; identity and unresolved-submission checks still apply.
Queued product messages draw attention through a pending-context indicator in
the central tab-bar action group or desk header, independently of agent status, with their
full text available on demand from the tools dialog. Pending input does not
reserve space for a transcript, footer or prominent readiness notice.

The [provider matrix](../quality/provider-matrix.md) records verified macOS Tauri
responses, canonical history, cold resume and shutdown separately from native
CLI smokes, automated tests and the remaining provider-specific checks.

`prometeu-protocols::terminal_status` reads OSC 7501 without changing rendered
bytes. It accepts BEL and ST termination across arbitrary chunks, bounds each
sequence to 4096 bytes, validates encoded single-line text and retains explicit
root/child records. Only a working/blocked to done/error transition can report
completion for that record; idle, clear, eviction and output silence cannot.
Child completion never implies root completion. These observations contain no
permission response channel or conversation text.

The query response is sent only by the live native reader implementing the
advertised states. Snapshot replay never answers queries. Within each live
output batch, the PSP response precedes emulator replies: Claude uses its DA
response as the capability-discovery barrier. The native reader updates the
screen independently of capture, input and tab publication. A separate bounded
reply queue retains this ordering without letting a long input block output
draining; queue overflow invalidates the connection instead of losing a reply.
The implementation follows the
[Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status).

Preparation in `terminal_agent_launch.rs` builds interactive CLI commands
using the selected account and native tool materializers. Native `--help` probes
and synthetic tests are distinct from actual model runs. The native transport
is composed with canonical content, durable queueing, telemetry, sharing,
screen restoration and notifications. Passing preparation or parser tests
alone does not certify these product flows.

Claude Ask selects the explicit native manual policy. When combined with Plan,
preparation passes `--permission-mode plan` without the bypass-availability
flag, which would otherwise open an additional bypass consent screen. Fresh
and exact-resume tests cover that combination.

Cursor's interactive MCP selection uses a private native plugin containing
`.cursor-plugin/plugin.json` and `mcp.json`. The existing MCP materializer
resolves selected servers; the terminal adapter converts ACP environment/header
pairs to Cursor's native objects. Values remain in files with mode `0600`, never
native command arguments. The native CLI still controls its own configuration
outside this injected selection. The opt-in `terminal_agent_launch::mcp_smoke`
test verifies selected stdio initialization, tool discovery and environment
delivery on `2026.11.13-eacade1`, without inference or tool execution. HTTP/SSE
activation and actual MCP tool calls require separate native conformance.

Codex's [private native engine](native-codex-engine.md) prepares the exact thread
through app-server RPC and attaches the official TUI over a private Unix socket.
The isolated no-inference test verifies Ask and Plan on fresh and cold resume;
live RPC and recovery tests separately cover canonical content and explicit
responses. Full Tauri application verification is a separate level of evidence.

`chat/terminal.rs` owns the PTY as a `Chat` transport. An independent emulator
drain lets an input transaction await the painted draft while output continues.
Live UI events and the canonical observation worker wait for
tab publication before reporting identity or exit, and one owning waiter reaps
the process before closure. An explicit native root `idle` report can release
startup input; spawn, first output, a hook named `SessionStart` and silence do
not establish input readiness. Programmatic input uses bracketed paste and
rejects terminal control characters; raw local keyboard input remains native.

Local native insertion is a draft operation, separate from submitting queued
messages. File selection, drop and clipboard files insert paths directly;
**Insert into terminal** handles staged context from the tools dialog. Both
capture the original tab, terminal attachment and process generation,
require an input-ready terminal with bracketed paste enabled, and write only
the paste envelope, without Enter, automatic resume or retargeting. Each image
uses its own bracketed paste containing a literal path, quoted when necessary,
without an `@` prefix; other files retain document mentions. Files are inserted
sequentially before other staged context. Each successful write removes only
that file from the draft, so a later failure leaves the remaining files and
context available without repeating earlier successful writes. Browser context,
review notes and dictated text stay staged in the original tab's tools dialog
if their insertion fails. Successful paste preserves any context added while
the write was pending. Review submission waits for matching
canonical user content; it is not inferred from a successful PTY write or from
terminal paint. See the
[conversation flow](../architecture/conversation-flow.md).

Native slash commands, model, effort and permission-mode changes use the CLI's
own controls. The app retains model and effort defaults for launch but does not
repeat those selectors in the local native conversation.
The shared transcript view omits provider slash suggestions, compact/context actions,
Allow always and the plan action that enables bypass. User Actions still expand
before command validation; ordinary prompts and individual Claude/Codex request
answers keep their existing paths. Manual slash input is rejected before the
composer discards its draft or attachments. Stale callbacks revalidate the
conversation mode and attachment. The encrypted shared mode applies the same
boundary to viewers, and the owner checks again before prefixing remote text.

The verified Cursor TUI requires a two-phase send: the host reserves native
input, writes bracketed paste without Enter, then waits for that exact draft's
painted acknowledgment in a newer emulator snapshot. Enter is written once only
after acknowledgment. A deadline or retirement never authorizes Enter or a
retry; an unsubmitted draft remains visible. The reader and query replies keep
running while capture/input remain reserved. The packaged-helper transport test
checks the paint barrier, one Enter and retirement between paste and submission.
Cursor's window-focus report controls its painted caret separately from input
availability. The verified empty placeholder remains dim, including its arrow,
without focus; a literal draft has a normal arrow. An exclusively owned paste
can therefore acknowledge its exact content without an inverse caret, while
disabled ordinary text and modal footers still reject admission. Native wrapping
determines the paste height; a wrapped workspace path in the footer does not.
Native idle and an available composer are separate gates. After local editing,
an older empty screen cannot authorize queued input; a newer snapshot must prove
the input empty using its actual cell attributes, or native acceptance must
settle that draft. Literal text matching a placeholder remains user input.
Codex 0.169.0 requires its dim native placeholder, focused cursor and current
footer as well as native idle. A literal identical draft at Home and a draft
containing only spaces cannot release the queue. The pinned fresh/resume and
literal/Home/clear captures exercise this distinction without model inference;
painting alone never establishes Codex activity or completion.
Antigravity 1.5.10 admits its focused single-line composer with native idle,
full-width borders and the exact idle footer. A fresh process may admit its
first input before any PSP report when that same empty composer is verified and
no input or activity has been observed. This one-use startup gate is revoked
before any composer write, including local keyboard input, and by native
activity. Resume and all subsequent input require the existing native idle
authority; an empty repaint cannot complete a turn or restore the startup gate.
Plain ASCII spaces at Home are
equivalent to empty because the pinned native submit handler trims them and
discards an empty result without a command when no paste placeholders exist.
The native mode hint is also empty only with its exact pinned text/truncation
and palette-8 foreground. A real literal-at-Home capture uses default
foreground instead. Explicit `NO_COLOR` remains respected; an uncolored hint
cannot release programmatic input because it is indistinguishable from a draft.
Chips, multiline drafts, menus, missing metadata and incomplete paints fail
admission. This proof releases the local-edit fence without rewriting submitted
text or assuming how a hook normalizes its user-message alias. The binary hash,
symbol addresses and captured geometry are recorded beside the
[readiness fixtures](../../src-tauri/src/native_ui_readiness/fixtures/README.md).
The captured native interruption banner immediately above Antigravity's top
composer border is compatible with empty input. It does not act as a draft or
modal veto. A newer verified empty paint releases the local Escape input fence,
and independent native idle evidence allows queued product messages to proceed.
The banner itself never establishes idle or successful completion.
Startup trust keys before any composer do not reserve a nonexistent draft.
Once root PSP reports are observed, delayed hook activity cannot overwrite that
state: hooks and terminal bytes arrive through independent queues. Claude's
PSP-enabled title may omit its idle prefix, so authoritative status replaces only
the title test, preserving composer, authentication and modal checks.
Antigravity instructions retained by preparation become native rules in the
process's private added root. Cursor uses its native `beforeSubmitPrompt`
`additional_context` carrier, so updated instructions also apply after resume
and to input typed directly in the TUI. The private hook emits JSON data through
`/bin/cat`; instruction contents are never shell source or a permission override.
Cursor's verified carrier limit is 10,000 UTF-16 units; larger instructions fail
explicitly before launch. These paths do not alter the submitted prompt.
Claude and Codex support individual application approval/question responses
through their native decision channels. Cursor and Antigravity decisions stay
in their own TUIs by product choice; shared viewers see that the owner must
answer. Unsupported responses fail without closing a canonical request.

Native interruption uses `terminal_agent_controls::NativeControls`, bound to
the verified CLI version and the current generation's screen. A default key
alone is insufficient: the active composer/footer must advertise that binding,
and the host must have an observed active turn. The application Stop control
uses Escape for Claude, Codex and Antigravity, and Ctrl+C for Cursor. Local
keyboard input retains each CLI's own bindings; the pinned Cursor also accepts
Escape with an empty prompt when no other native action consumes it. Unknown
versions, incomplete paints, conflicting
hints and unsupported layouts return no control. The host consumes interruption
once per active cycle; a repeated Ctrl+C can exit Cursor. Codex's MCP startup
cancel hint is deliberately separate from a model-turn interruption. Current
fixture and native-smoke coverage is recorded in the
[provider matrix](../quality/provider-matrix.md).

Cursor's versioned busy composer also supplies early observed Working after a
native input boundary: Enter written while idle must be followed by a newer,
complete paint with its busy footer and matching caret. Neither the key nor a
screen from the previous turn is sufficient. This observation starts the
spinner without inventing a user message; later durable source capture claims
the same execution cycle. A settled cycle cannot reopen from an old busy paint.

Cursor's authenticated root `stop` / `aborted` hook settles the canonical outcome
as `interrupted` when its generation matches the current durable source request
and ingestion is caught up. New prompt boundaries invalidate older evidence;
their pending-generation barrier clears only after matching durable source
capture. Later source-authorized requests do not depend on repeating that hook,
and bounded request-specific Stop evidence cannot be overwritten by an older
request's delayed hook. Later `completed` hooks for the cancelled request cannot
override it. The host
journals that outcome before publishing, without requiring an empty composer:
Cursor may restore the interrupted prompt as a draft. The draft is not sent,
raw Escape never changes status, and late source content or tool hooks from the
cancelled request do not restart the spinner. This path creates no successful
reply notification. Its source ordering and regression coverage are in the
[Cursor contract](cursor.md#native-interruption-evidence).

An explicit root `Working`/`Blocked` → `Idle` report settles observed activity
without manufacturing `turn.completed`. AGY 1.5.10 uses this sequence after
Escape. The key itself does not change status. Late source content from the
settled activity still updates history without reopening work; a new accepted
input restores live execution. Active children
and real unanswered requests retain their own state. Clearing or losing a
record remains uncertain. This activity settlement creates no successful reply
notification and leaves the existing Done completion path unchanged.

Root idle after `Done` or `Error` also makes late source content quiet, independently
of the pending completion ticket. AGY 1.5.10 can report `Working` → `Done` → `Idle`
when Escape interrupts a turn after file exploration. Its native `Done` label is
not proof of a successful reply. The host retains the source barrier for completion
while keeping late thinking and tool content from reopening an idle tab. New
accepted input clears the quiet-content boundary; a delayed capture of the same
native input preserves it.

`terminal_agent_hooks.rs` supplies the internal `--terminal-agent-hook` helper.
The native conversation transport configures one bridge per PTY incarnation. The helper sends a
bounded native hook payload through a private authenticated Unix socket;
session, process nonce and native identity bind an observation to one launch.
Content needed for conversation reconstruction stays in this private channel;
credentials are not forwarded to the observer or logged. Closing the bridge
joins its listener and closes the sender; already queued observations may be
drained before process-exit publication. Ordinary observation hooks emit no
permission decision. Claude PermissionRequest additionally supports an explicit,
invocation-scoped response while preserving its native dialog; timeout and
retirement remain neutral. See [native Claude decisions](native-claude-decisions.md).

The worker registers successful programmatic submissions under the input lock,
then observes hooks and the bound transcript before releasing that lock. Its
callbacks stage observations; publication and telemetry run after release to
preserve the existing capture lock order. Prompts accepted directly in the
native terminal use the ordinary canonical input/capture path without writing
to the PTY again. A resumed transcript must finish its silent baseline before
programmatic input is admitted, independently of native input readiness. On
exit the worker closes the hook sender and drains final observations before
publishing the session lifecycle transition. Provider-specific readiness and
decision rules remain at the native edge; restoration uses the shared
[terminal state contract](terminal-state.md).

After process loss, a terminal tab that has not obtained a native session
identity cannot resume: it fails explicitly instead of silently creating a new
conversation. An unresolved prepared input also remains blocked until native
source evidence reconciles its acceptance; an empty visible transcript does
not prove that no write was attempted. Only the live owner of a submission
proven to have failed before any write may discard that intent. These limits
do not prevent resuming an existing, bound native history.

`prometeu-protocols::terminal_transcript` separates replay from live observations,
handles partial JSONL records and signals full snapshot replacement on rewrites.
Binding requires an explicit native session and transcript path. Its synthetic
fixtures are based on provider formats, not recordings of user conversations.
Initial replay, truncation and missing records never imply turn completion.
The desktop must preserve app-owned events separately when replacing a native
projection; it must not feed replacement replay through the live pump.

AGY 1.5.10 terminal content uses its [original native store](native-antigravity-store.md)
rather than the truncated JSONL summary. Its DB/WAL copy is shared with the Cursor
reader; tool results retain explicit native call identity and authorized complete
output files. The existing NDJSON Chat adapter is unchanged.

For verified Claude and Antigravity versions, the host combines the native
successful-stop fence with the exact root PSP Done report and its current
execution ticket. The fence requires a new raw user boundary, the matching
persisted final assistant source, and a caught-up poll begun after both Stop
and Done. A private report ordinal correlates the ticket without equating the
observer's submission counter with the execution cycle. Reports received during
an app input reservation wait until its expected submission is registered.
Canonical content is committed before the corresponding execution reports are
published. The completion callback revalidates the ticket under the ordinary
capture/input lock order and commits a durable source receipt before closing
execution or publishing completion. Neither replay nor duplicate receipts can
repeat the completion's effects. This proves only the supported success path;
native errors, approvals and unverified provider versions require their own
authoritative evidence. The pure fence, observer boundary tests and
`chat/terminal/ingestion/tests.rs` cover these conditions.

The pinned Cursor success path instead captures a private transcript stamp
before acknowledging its authenticated Stop hook. The observer requires an
exact native request identity, complete committed source coverage and a changed
stable JSONL revision ending in the provider's post-hooks success marker; an old
marker cannot qualify. The host then verifies the current empty composer and
pending-input guard under the canonical capture/input ordering. Only the durable
source receipt permits completion effects. Cursor's optional run records remain
optional and are checked when present. See the [Cursor contract](cursor.md)
for the versioned writer ordering, trust boundary and remaining coverage limits.
The separate interrupted outcome relies on the authenticated aborted hook and
durable request identity; its native marker may be written before the hook and
does not use this success fence.

## Model discovery

`agents` discovers installations and capabilities, returning empty model lists.
`agent_models({ agent })` independently queries the selected account and returns
`{ models: AgentModel[], fetchedAt: number }`, with a Unix millisecond timestamp.
Failures reject with `{ code }` from `err.modelsCatalog.noAccount`, `unavailable`,
`timeout`, `invalid`, or `failed`; successful empty lists are distinct from errors.
No raw vendor response or credentials cross this boundary.

Claude uses `list_models`, excluding disabled entries and the unnamed default.
Codex uses app-server `initialize`, `initialized`, and every `model/list` page
with `includeHidden: true`; `model` is the launch identifier, `hidden` maps to
`additional`, and native reasoning levels are preserved. The CLI cache is not a
picker source. Antigravity uses `agy models`; its labels do not imply effort
support. Discovery sends no inference prompt, is bounded to twenty seconds per
query and reaps its subprocess group on success, failure or timeout.
The injected query port applies that deadline across writes and every page,
with 1 MiB total stdout.

Frontend refresh runs at startup/account changes, and on picker opening after
five minutes, with a manual refresh override. Concurrent requests per provider
are coalesced. Account changes clear catalog generations and discard late
responses. Failed refreshes retain only the current generation's last successful
catalog in memory, visibly marked stale; no catalog survives an app restart.
Successful refreshes replace the list, including removals and empty results.

Model choices always carry `{ agent, model }`; names are never used to recover
provider identity except for explicit legacy preference migration. Historical
Claude aliases label old choices but cannot establish availability. Unknown saved
choices remain readable, without becoming new selectable models. New workspaces
require explicit correction of unavailable saved models or unsupported efforts.
Refresh alone
never mutates a workspace, tab or action profile.

Effort menus contain the provider default (empty string in persisted choices)
and exactly the native advertised levels. Unknown level names remain usable.
Switching models retains a supported effort or resets it to the native default
with a visible indication. Empty levels hide the control unless a historical
nonempty value needs correction. Claude no longer receives a synthesized
`ultracode` option; historical Codex `ultracode` is interpreted as native `ultra`.
Invalid historical choices remain visible until explicitly edited.

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

This is the minimum set observed by the current interface. A capability only
enters here when it changes behavior offered by the application. Protocol
details, such as the name of a JSON-RPC method, are not capabilities.

`usageTokens` enables measured turn consumption; `usageCost` enables only a
normalized CLI-reported estimate, never pricing inferred by Prometeu.
`contextWindow` enables the occupancy meter only when a usable current window
has also been observed. Claude and Codex expose the meter; Antigravity supplies
usage without a window or cost. Capability availability does not turn an
unknown measurement into zero. The [provider matrix](../quality/provider-matrix.md)
records normalization and regression evidence.

Skills selection reuses the plugin pipeline, so the existing
`workspacePluginSelection` also gates it. Resolving the global and project
layers is a core concern and adds no capability.

Some capabilities may vary by CLI version or model. In that case, the descriptor
returned at runtime is the source of truth; the frontend does not keep a
parallel table. Before discovery, or if IPC fails, the frontend bootstrap keeps
only Claude installed and announces no optional capability.

## Execution account

The account choice is global per provider and stays outside `SessionLaunch`. The
adapter captures the selected profile at spawn; the process keeps its ID and
revision until the turn ends. The next message resumes the same transcript with
the current selection when needed. Login, profile and credentials belong to the
provider's edge; the core receives only normalized identity and state. Removing
the active account leaves the provider without a selection. The turn already
started may finish; new sends return `err.account.noActive` until the next
choice. See [`accounts.md`](accounts.md).

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

Semantics of the optional values:

- `null` in model or effort lets the provider choose its default;
- `null` in MCP/plugins/skills means imposing no selection and preserving the
  CLI's configuration;
- an empty list means injecting no item from Prometeu's hub; a global registry
  that the CLI itself loads stays under its control;
- the three lists arrive already resolved: composing the global, project and
  workspace layers is a core concern, and an adapter never resolves layers or
  reads the board;
- `resume` is an opaque identity accepted by the provider. It may have been
  chosen by Prometeu, as in Claude, or returned by the provider, as in Codex.

The core validates `SessionLaunch` against the capabilities before starting the
adapter. The adapter must not silently fix an invalid combination. A chosen MCP,
plugin or skill configuration that cannot be materialized fails before the
spawn; a declared hook that cannot be activated fails before the thread is
opened. A chosen MCP id the registry no longer has is such a case: both adapters
fail with `err.mcp.missing` instead of quietly dropping it. Starting without the
requested behavior is not a valid fallback. The
detailed plugin contract is in
[`plugin-marketplace.md`](plugin-marketplace.md).

## Workspace launch resolution

Ordinary tab creation and resume both use `session::workspace_launch_with`. Tool
selection uses shared `core::tool_resolution`: desktop `session.rs` composes the global layer from
the board, the project layer from the primary repository's
`.prometeu/settings.toml` and the workspace layer into the three `SessionLaunch`
lists, leaving out project-declared items whose hash is not approved yet
([ADR 0045](../decisions/0045-layered-tool-selection.md)). Resolution uses the
effective tab provider, including a new tab's override or
an action profile's provider, rather than the workspace default. A model/effort
override within that provider preserves the resolved set. Task tabs continue
to use their frozen resolved profile. The regressions in
`session.rs` cover these selections for Claude and Codex.

The resolved set is captured at spawn, following the execution-account rule
above: a change at any layer applies at the next spawn or resume of a stopped
process and never restarts a running session. An idle process also keeps its
captured set; sending another message alone does not reload tools.

Provider adapters own CLI flags and consume tool artifacts through injected
`prometeu-tools::StartupTools`. `session.rs` resolves application choices and
passes `Launch` to the adapter; plugins and standalone skills share the package
pipeline. Desktop `tool_materialization.rs` and WSL `runtime::tools` compose the
same `NativeTools`; the WSL provider reads current choices through
`ToolSelection` at spawn. Existing argument fixtures and injected startup tests
preserve flags, resume behavior, scope, environment and failure ordering. See
[tools and packages](application-core.md#tools-and-packages).

The Tauri commands for changing workspace MCP/plugins/skills delegate to
[`workspace_tools.rs`](../../src-tauri/crates/core/src/workspace_tools.rs). This application
boundary validates axis selections and updates explicit board state without
accessing processes or changing sibling tabs. The commands retain native
argument handling, error translation and board publication. Tests run without
a Tauri application and verify invalid payloads, selection changes and tab
preservation. The extraction preserves the layered IPC and persisted formats;
see [ADR 0050](../decisions/0050-tested-application-boundaries.md).

## Skill kickoff

The launcher's *Start with* choice (`Draft.kickoff`, `<package>/<skill>`, empty
for none) is an ephemeral addition over the layered resolution above
([ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md)). After the
three lists are resolved for the first conversation, the core adds the skill's
package to the axis its ID belongs to — `skill-<id>` to `skills`, a plugin to
`plugins` — turning an inheriting `null` into a one-item list. No global,
project or workspace layer changes, so later tabs keep the ordinary resolution.
`Tab.kickoff` stores the choice and a resume of that tab adds the package again
after validating the skill against the installed catalog, like creation does.
A skill its package no longer ships, or a package no longer in the hub, never
blocks the resume: the package is not added and the conversation receives a
`system.notice` with `level: "warning"` and `code: "kickoff.missing"`, whose
`detail` the backend renders in the display language. Adapters receive
ordinary lists and materialize them as today.

The first message opens with one app-written line naming the skill, its
plugin when it has one, and the repository's `[method] artifacts` path relative
to the working directory, followed by attachments and the person's prompt. The
line is rendered in the display language by the backend. An undeclared path is
never named. The choice requires `workspacePluginSelection`; the core rejects it
for other providers with `err.kickoff.unsupported`, and a skill missing from the
installed catalog fails with `err.kickoff.missing` before a card is published.

## Conceptual port

The design can be implemented with a trait, enum dispatch or grouped functions.
The semantics matter more than the syntactic form:

```text
AgentCatalog
  discover() -> AgentDescriptor[]

AgentRuntime
  start(SessionLaunch) -> SessionHandle
  send(SessionHandle, ConversationCommand)
  stop(SessionHandle)
  output(SessionHandle, RawProviderEvent) -> ConversationEvent[]
```

Adapter responsibilities:

- start the executable and configure its environment;
- convert `SessionLaunch` into the provider's arguments or requests;
- materialize MCP, plugins and skills in the form the provider requires without
  exposing that form to the domain;
- correlate the external protocol's own requests and responses;
- turn external output into `ConversationEventV1`;
- turn `ConversationCommandV1` into external input;
- expose failures with a stable code and diagnostic detail;
- terminate the process and its descendants according to the app's policy.

Responsibilities that stay outside the adapter:

- choosing what the UI shows;
- persisting board state;
- enforcing the team audience;
- rendering tools or markdown;
- deciding global resume and message-queue policies.

## Task profiles

[Task](actions.md) launches add persisted instructions and a permission policy
(`ask` or `auto`). The session stores the resolved configuration. Claude receives
additional instructions through `--append-system-prompt`; Codex receives
`developerInstructions` when starting or resuming the thread. Approval under
`ask` uses Claude's normal mode and `approvalPolicy: untrusted` in Codex.
Previous launches preserve the existing bypass. Codex configuration derived from
a task is isolated per session, preventing distinct profiles from overwriting
the same home.

## Compatibility

- A change only in the external protocol must change one adapter and its
  fixtures.
- A change in common behavior changes this contract and the conformance suite.
- A new capability starts as `false` in the existing providers until there is
  evidence and a test.
- An unavailable provider does not prevent the others' catalog from loading.
- A discovery failure must not invent support; the fallback must be explicit and
  observable.

## Minimum conformance

Each provider must demonstrate, when the capability exists:

1. starting a new session;
2. resuming without duplicating messages;
3. a message and a final answer;
4. streaming followed by the authoritative event;
5. a tool call and its result;
6. a question or approval and a correlated answer;
7. interruption;
8. compaction and context report;
9. termination of the process and its descendants;
10. tolerance of an unknown external event.

The living matrix of this evidence is in
[`../quality/provider-matrix.md`](../quality/provider-matrix.md).

## Antigravity CLI

`antigravity.rs` adapts `agy` 1.2.7+ native NDJSON to V1. Startup uses
`--input-format stream-json --output-format stream-json`; each prompt is an
`event: user` with text content. `init` records native identity, `step_update`
emits text/tool events, and `result` completes the turn. A final response must
not duplicate streamed deltas. Usage/duration counters from agy are cumulative;
the adapter measures local turn duration rather than claiming per-turn totals.
Pending follow-ups stay in the board until completion; the adapter never advances
a private queue before the core records the previous turn completion.

`Tab.agent_session` stores `conversation_id`; `--conversation` resumes native
context. The app replays its own V1 log, never raw agy messages. SIGINT interrupts
the process group; the next prompt can restart and resume after process loss.
`agy models` supplies slugs and labels; effort choices are not synthesized.

Headless control messages and approvals are unsupported. Ordinary conversations
use automatic execution (`--dangerously-skip-permissions`), matching the existing
Claude conversation default. Explicit Ask task profiles preserve agy policy;
commands requiring approval may be soft-denied. Tool state `ERROR` is terminal.
A result containing `denied_actions` is an error even if the native status is
`SUCCESS`; the translated permission message is shown without leaking raw errors. Auto task profiles also use automatic execution. Initial plan mode, hub tool selection,
compaction, context reports and structured questions are unavailable. Nonempty
hub selections fail at the adapter boundary. Local attachment paths remain text.

Authentication methods remain descriptor data: Claude/Codex advertise browser;
Antigravity advertises `external`, with `accountNotice` explaining limits. The
old Gemini identity is a retired persistence marker; any execution refuses with
`err.provider.retired`. See [ADR 0052](../decisions/0052-antigravity-runtime.md).

## Shared native discovery

Provider descriptors and model result types live in `prometeu-core::agents`.
The desktop and WSL runtime share `prometeu-protocols::catalog` for bounded
queries, pagination and response validation; hosts supply the query launcher,
prepared command and client version. `ProviderDiscovery` registers providers in
the runtime. Windows coverage is in the
[Windows application contract](windows-application.md).

## Cursor

The additive `cursor` provider is discovered through `cursor-agent acp --help`.
Its descriptor exposes external account attachment, initial plan mode, resume,
approvals, questions, path mentions and hub MCP. Model discovery uses exact ACP
model IDs from an unprompted native session, with the existing query deadline and
output limit. Per-turn usage/context controls and Prometeu-managed plugins and
skills remain unavailable. Account quotas use a separate dashboard probe and the
existing usage scheduler. See the [Cursor contract](cursor.md).
