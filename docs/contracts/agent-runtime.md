# Agent runtime contract

Status: contract in force; identity/capabilities implemented by ADR 0003 and
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
type ProviderId = "claude" | "codex" | "antigravity" | "gemini";

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
`agent` for compatibility, but its normalized value is `"claude" | "codex" | "antigravity" | "gemini"`.

## Native preparation

Provider preparation is injected separately from conversation spawning. Native
adapters return configuration, transcript ownership and a connection factory;
the launcher supplies process input/control and receives canonical input/output
interfaces. Provider interruption and turn-wait policy remain in those adapters.
This preserves the CLI and V1 protocols; it introduces no bridge payload. See
the [application contract](application-core.md#provider-preparation-and-input).

Codex translation is shared through `prometeu-protocols::codex`; desktop launch
composition remains in `src/codex.rs`. The host supplies language selection and
client version, preserving existing desktop behavior. The experimental
[headless executable](headless-runtime.md) composes the same adapter for one
conversation using native authentication/configuration, with Ask approvals.

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
The injected private query port enforces that deadline across nonblocking
writes and every response page, with 1 MiB total stdout. Protocols stay in the
provider adapter; see [bounded queries](application-core.md#bounded-commands-and-queries).

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
pipeline. MCP encoding uses injected catalog/token and private-file ports in
`prometeu-tools`; package preparation and Codex cache installation also live in
that native crate through injected catalog/files/installer ports. Desktop
`tool_materialization.rs` and WSL `runtime::tools` compose the shared `NativeTools`
implementation with their package backend and MCP adapters. The WSL provider reads
the current catalog through injected `ToolSelection` at spawn, preserving the same
trust and selection rules without desktop dependencies. Existing argument fixtures and injected startup tests
preserve flags, resume behavior, scope, environment and failure ordering. See
[ADR 0077](../decisions/0077-injected-startup-tools.md).

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

## Implemented conversation-stream boundary

The shared conversation stream is in `prometeu-core::conversation`. Its
`ConversationInput` port receives canonical commands and the replay buffer;
the existing `Chat` adapter implements it using the provider links. Injected
`TranscriptStore`, `ConversationEvents` and `Clock` ports cover recording,
publication and timestamps. This is an implemented ordering boundary, while
native preparation and feature effects remain in adapters. Start/resume now
uses the injected `LaunchService`/`ConversationLauncher` boundary; desktop
provider registration lives in `session/launch.rs`. See
[launch coordination](application-core.md#session-launch-and-resume). The core
now also owns process-supervision ports and shutdown policy;
`prometeu-process` implements Unix spawning, group control and reaping without
Tauri. Shared `ConversationWorkers` now initialize the adapter and consume
translated output through an injected `TaskExecutor`, with lifecycle effects
supplied by the host. See the [worker contract](application-core.md#conversation-workers). Claude and Codex keep protocol interruption; Antigravity uses the
injected group interrupt. See the
[process contract](application-core.md#process-supervision). See the [core contract](application-core.md#conversation-stream).

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

## Session coordination boundary

The portable `SessionService` handles queued input, account handoffs and process
recovery through injected effects. `SessionReactions` consumes canonical events
for tab state and background settlement; provider usage translation remains in
the desktop adapter. `SessionOutput` orders execution observation and private
capture around transcript delivery. See the
[core contract](application-core.md#session-coordination) and
[ADR 0066](../decisions/0066-injected-session-coordination.md). No provider
capability or V1 shape changes; launch resolution remains in the native host.

## Shared native discovery

Provider descriptors and model result types now live in `prometeu-core::agents`.
The desktop and WSL runtime share `prometeu-protocols::catalog` for bounded private
queries, pagination and response validation. Hosts supply the query launcher,
prepared command/profile and client version; raw provider replies remain at the
edge. `ProviderDiscovery` is the runtime registration port. The original desktop
catalog tests retain coverage of profile isolation, deadlines, malformed replies,
limits and cleanup. Windows integration coverage and account limits are in the
[Windows application contract](windows-application.md).
