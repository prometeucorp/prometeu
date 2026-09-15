# Agent runtime contract

Status: contract in force; identity/capabilities implemented by ADR 0003 and
canonical events implemented by ADR 0002.

This contract defines the boundary between Prometeu and an agent CLI. It is not
an API for language models: it describes local processes that have their own
catalog, session, protocol and capabilities.

## Identity

Provider and model are distinct concepts. A model belongs to a provider; the
model name must not be used to rediscover its provider after the catalog has
been loaded.

```ts
type ProviderId = "claude" | "codex";

type AgentModel = {
  id: string;
  label: string;
  efforts: string[];
};

type AgentDescriptor = {
  id: ProviderId;
  label: string;
  installed: boolean;
  models: AgentModel[];
  capabilities: AgentCapabilities;
};
```

When a provider is added, `ProviderId` grows explicitly. An old or unknown value
coming from disk falls back to the default provider only during persistence
migration; new code uses exhaustive matching. The JSON field is still called
`agent` for compatibility, but its normalized value is `"claude" | "codex"`.

## Capabilities

```ts
type AgentCapabilities = {
  initialPlanMode: boolean;
  workspaceMcpSelection: boolean;
  workspacePluginSelection: boolean;
  resume: boolean;
  compact: boolean;
  contextReport: boolean;
  approvals: boolean;
  userQuestions: boolean;
  attachments: boolean;
};
```

This is the minimum set observed by the current interface. A capability only
enters here when it changes behavior offered by the application. Protocol
details, such as the name of a JSON-RPC method, are not capabilities.

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

Ordinary tab creation and resume both use `Workspace::launch_with`. Tool
selection is resolved by the core: `session.rs` composes the global layer from
the board, the project layer from the primary repository's
`.prometeu/settings.toml` and the workspace layer into the three `SessionLaunch`
lists, leaving out project-declared items whose hash is not approved yet
([ADR 0043](../decisions/0043-layered-tool-selection.md)). A tab's
provider/model/effort override changes its choice while preserving that resolved
set. Task tabs continue to use their frozen resolved profile. The regressions in
`session.rs` cover these selections for Claude and Codex.

The resolved set is captured at spawn, following the execution-account rule
above: a change at any layer applies at the next spawn or resume and never
restarts a running session.

`claude.rs::launch_args` owns Claude flags and MCP/plugin/skill materialization;
`session.rs` resolves application choices and passes `Launch` to the adapter.
The relocated argument tests preserve existing flags, resume behavior, and
configuration handling.

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
