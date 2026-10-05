# Versioned automations

Status: implemented native desktop executor; platform and provider limits below.
Decision: [ADR 0087](../decisions/0087-versioned-automation-workflows.md).

## Definition and ownership

A workflow is independent of a conversation tab. Its JSON graph is the source of
truth for conversation editing, visualization, MCP, validation, simulation and runtime. Nodes have
stable IDs, positions and a tagged configuration; directed edges select named
output ports. Supported nodes are trigger, query, condition, Jev classifier,
agent, action, human approval and wait. Cycles, missing references, incompatible
operations/ports and unreachable nodes are rejected. The operation registry
identifies implementations by ID and version; it is not executable user code.

Conditions evaluate local typed values. Jev is remote inference through the
existing [evaluation port](context-evaluation.md), using the person's TypeSafe
connection. An omitted answer is an abstention. Classification never grants an
action permission. Agent execution is a separate capability, with a structured
result and explicit tools; a worktree by itself is not a sandbox.

Saving uses optimistic revision comparison. Each run keeps an immutable workflow
snapshot, event, selected outputs and history. Editing the workflow never changes
a run's definition or grants. Deleting a definition retains execution history.
Legacy Actions and `/review` remain readable and usable without data conversion;
the new graph cannot silently inherit their `auto` permission.

The desktop host owns a private, atomically persisted automation store. The core
crate owns validation and transitions without filesystem, provider or network
dependencies. The browser backend is development-only: it persists browser
fixtures, simulates graphs and refuses real provider calls and effects.

Automations opens a library rather than selecting a workflow implicitly. Saved
workflows show their active/paused state and revision; a separate template gallery
explains each starting point and creates a disabled copy. Opening a saved workflow
shows its Steps view. New workflows start in Conversation. The editor's **My
workflows** action returns to the library and refreshes the native snapshot.
The current unsaved draft remains screen-local and is labeled accordingly.
Returning to the library and reopening the current workflow preserves its graph,
conversation, proposal and unsent message. Selecting a different workflow asks
before discarding any of those unsaved inputs. Saved definitions belong to the
current app data root; development and installed apps remain separate.

The Workflow tab displays a read-only Mermaid diagram derived locally from the
structured graph. The default overview omits error/uncertainty routes and nodes
reachable only through those routes, with an explicit count of omitted edges.
The detailed toggle includes every route. Steps remains a readable alternative
with every configured destination, including unhandled error/uncertainty ports.
Neither view changes saved positions, configuration, scope or execution behavior.

Graph logic changes go through the conversation and the existing proposal review.
The per-step change action appends a request to the unsent composer without
calling a model. There are no drag handles, port wiring, node configuration forms
or layout mutations. Scope, permissions, activation, pause, revisions and execution
controls remain direct desktop actions. Restoring a revision still creates a draft.

Mermaid is a lazy local dependency, with strict security and HTML labels disabled.
The app constructs syntax with generated node IDs and entity-encoded labels;
provider replies, persisted IDs and user names never become raw Mermaid source.
The renderer has no callback/link binding or external rendering service. Stale
asynchronous results are discarded after view replacement or a detail toggle.
Rendering failure preserves the workflow and offers retry or the Steps view.

## Events and execution

Polling is deterministic and happens only while the desktop host is running.
Minimum configured cadence is 60 seconds; API latency means it is not a delivery
guarantee. Offline failures preserve the cursor and surface a diagnostic. A
closed app or suspended computer does not poll. There is no hosted executor.

Initial observation establishes a baseline unless inclusion of existing items was
explicitly chosen. Scope binds local project, repository and connected identity;
multiple targets have explicit mappings. Account changes cannot silently adopt a
new identity. Deduplication and resource reservations are persisted with accepted
work. An event key binds its target cursor and a persisted observation occurrence;
the content fingerprint detects changes but is not the event identity. Repeated
observations do not enqueue work, while a return to an earlier state is a new
occurrence. Cursor advancement and admission commit together, so a failed enqueue
cannot consume an occurrence. Older cursors default the occurrence counter to zero
and retain their existing baseline and fingerprints.

Selected branches are processed from the actual graph. Wait and approval
nodes suspend rather than polling a model.

Mutation intent is persisted before dispatch. A crash or ambiguous transport
result can leave an uncertain effect, which requires reconciliation rather than
blind replay. There is no exactly-once guarantee for external effects. Merge
requires a fresh state check independent of graph wiring, matching authorization,
identity and analyzed SHA, required CI/review checks, and a SHA-bound human
approval when configured. The native mutation also uses a head-SHA precondition.

Retries and agent work are bounded by workflow policy. Provider cost may be
unknown; an unknown cost is not zero or a guaranteed monetary budget. External
comments and model output cannot change scope or grant tools.

## Editor, proposal and simulation

The editor reuses the shared design system. Conversation uses the existing chat
composer, user content, Markdown blocks, thinking indicator, error/notice cards
and feed styles. The composer stays mounted during replies and view changes,
preserving input focus, selection and the next draft. The controller explicitly
hides inactive panels, including after shared chat CSS reloads in development;
conversation and graph never share the graph's grid row. Graph entry preserves
readable zoom; fitting the whole graph is an explicit action. Save failures keep
the selected view and offer a collapsed explanation, a correction request to the
assistant, and account binding where needed. Correction preserves unsent text.
Adding, moving, connecting, editing
or deleting a node edits the same draft. Navigation preserves its unsaved state.
The assistant produces a proposed graph; the person reviews structural changes,
including policy and scope, before applying it. Applying to the editor is separate
from saving/activating the persisted workflow. CLI authentication failures stay
visible rather than generating scripted responses.
The conversation view is the default and can start a blank draft from a request.
Sending appends the user's message immediately and shows a pending reply. Enter
sends; Shift+Enter inserts a newline. The person can type the next request while
waiting; a failure preserves that text and keeps the failed request in the thread.
An explicit retry resends that failed request and its conversation context, without
duplicating the user turn or replacing a different next message being typed. A
rejected graph includes validation feedback on retry. No retry starts automatically.
After 30 seconds the pending reply explains that the app is still waiting.
Follow-up requests include user/assistant history and the latest pending proposal
as context. The assistant can answer or ask a clarification question without
producing a graph. Review still compares a proposed graph with the current editor
draft. Switching to the graph preserves the typed request. Messages are local to
the open screen and are not workflow revisions or a persisted provider session.

Both providers receive a strict output schema for the full workflow envelope,
node variants, positions, edges, scope and policy. Only arbitrary JSON leaves
use provider-only `inputsJson`, `contextJson` and `valueJson`
strings. The adapter decodes these before native graph validation; persisted
workflows and IPC graphs keep their existing shape. Claude uses `--json-schema`
and reads `structured_output`, with at most two turns for its structured reply.
Codex uses the same schema through `--output-schema`. Legacy Claude text envelopes
and Codex whole-graph strings remain readable. Invalid graphs cannot be applied;
validation diagnostics remain visible and enter the next request's history.
Before returning a graph to the editor, the host applies the same native checks
used for saving, including restricted worker schemas, tools and fixed commands.
The adapter supplies the worker's supported `summary`/`outcome` output contract
for new agent nodes. It is not a model-authored field or a JSON string enum in the
provider schema: quoted JSON literals are incompatible with strict output formats.
Legacy explicit `outputSchema` and `outputSchemaJson` values remain readable and
undergo native validation without being overwritten by the default.
Provider schema rejection returns `automation_proposal_format`, distinct from
invalid generated graphs and account failures. The UI preserves the draft and
reports an app integration error. The adapter extracts nested provider diagnostics
before bounding them to 2048 characters; these errors do not enter graph-repair
history or trigger automatic retries.
Proposal responses have a five-minute process deadline, separate from Codex's
short configuration preflight. Timeout, output overflow, missing CLI and transport
failures return distinct `automation_proposal_timeout`,
`automation_proposal_output_limit`, `automation_proposal_cli_missing` and
`automation_proposal_transport` diagnostics. The adapter does not recast these as
account configuration failures. A transport failure discards partial model output
and leaves the current draft/proposal intact. Dropping the bounded query owns
child cleanup. These paths are tested through injected process failures without
a model call; they do not establish live provider availability.
Existing draft validation findings enter the next request as context; diagnostics
include the affected node so the UI can open its editor directly.

GitHub identity binding belongs to the host, not to model output. For an authored-PR
proposal, unchanged project/repository pairs retain their reviewed identities,
including moves between single and multiple target representations. Missing bindings
use one read of the connected GitHub identity; account failures return
`automation_proposal_account` without applying the proposal. Model-supplied
identities cannot replace those bindings. Scope or identity changes remove write,
commit and push grants and restore approvals and required local checks. The proposal remains disabled and its
scope changes remain part of the existing review before apply/save.
No automatic paid repair call occurs. The proposal agent cannot inspect live
repositories or CI jobs; its instructions distinguish runtime GitHub inspection
from fixed local checks and require a clarification when capabilities are missing.
Disabled draft permissions do not prevent designing repair and publication steps.
The assistant explains the one-time editor grants instead of downgrading the
requested workflow to observation. The editor exposes an automatic repair preset
that grants writes, commits and push, disables per-publication approval and local
checks, and retains merge approval. Existing explicit approval/check nodes keep
their meaning; the assistant revises graph wiring on request. Save and activation
remain explicit, and changing a saved scope resets these permissions.

Simulation traverses the actual graph with an explicit input event and supplied
node outputs. It records chosen branches and missing evidence. It never invokes
an API, model, write, timer or approval. Planned effects and approvals in the
result are not execution evidence.

## IPC and MCP

Typed desktop commands:

- `automations_snapshot`: workflows, revisions, runs, registry, templates and diagnostics.
- `automations_save({workflow, expectedRevision})`: validated revision save.
- `automations_validate({workflow})`: structured validation issues.
- `automations_simulate({workflow, fixture})`: effect-free path and evidence.
- `automations_pause({id, paused})` and `automations_delete({id})`.
- `automations_propose({prompt, history?, projectId?, workflow?, provider?, model?})`: tool-free conversation response `{summary, workflow}`. `summary` is the assistant's reply; `workflow` is a validated proposal or explicit `null` for questions and explanations. History contains `{role: "user" | "assistant" | "validation", text}` entries, at most 80 entries and 64 KiB of text; omitted history is empty. Graph validation feedback is included with the distinct `validation` role so an explicit retry can correct the previous rejection. Provider errors and UI notices are excluded. Provider is Claude or Codex; model is an optional provider-native ID. Omitted provider preserves Claude compatibility. Empty model uses the provider default. Existing requests and graph replies retain their shape; native and frontend must both support nullable replies. Adapter tests cover legacy graph replies and replies without a graph.
- `automations_run({id, event?})`: explicit execution.
- `automations_approve({runId, nodeId, headSha?})`: human approval.
- `automations_resume({runId})` and `automations_cancel({runId})`: explicit recovery;
  unresolved effects cannot be blindly retried.

Run read models include optional `pendingApprovalNodeId`, computed by the native
executor from the frozen graph and completed ports while awaiting an unapproved
approval node. The editor uses this selection rather than history entries, which
may have no node ID or describe earlier approvals. The field is not persisted:
existing runs acquire it when read without migrating their state. Approval still
revalidates the current ready node, event and SHA in the native command.

The authenticated [embedded MCP](embedded-mcp.md) exposes catalog, scoped list/get,
validate, simulate, draft save, pause and deletion. It does not expose activation,
human approval or permission grants. A saved MCP draft must be disabled and
within the client's authorized projects; updates authorize both the previous and
proposed scopes. MCP scope is application authorization, not a system sandbox.

## Evidence and support

Pure graph/state tests live in
[`automation.rs`](../../src-tauri/crates/core/src/automation.rs); native adapters
and persistence tests in [`automations`](../../src-tauri/src/automations/).
Frontend helpers and browser interaction tests cover graph edits and actual
pointer/focus behavior. Browser fixtures cannot certify CLI execution or GitHub
mutation. Live paid calls and remote effects require explicit external validation.
The native desktop composes the executor; Windows/WSL does not yet implement the
new commands. See the [provider matrix](../quality/provider-matrix.md).

## Restricted worker boundary and current limits

Claude and Codex workers use private scratch directories and an explicit MCP
broker for the reserved workspace. Claude disables native tools, hooks, plugins
and user instructions. Normal account preparation is reused; bare mode is not
used because it bypasses subscription authentication. Codex disables shell and
optional capabilities, uses a read-only sandbox with approvals denied, and
ignores user configuration. Its account-aware configuration preflight rejects
unexpected system, project or managed layers before starting a model. Unsupported
CLI flags or configuration fail closed. The preflight is not an atomic snapshot
against administrator changes between inspection and launch. Antigravity is not
an automation worker. Proposals use the selected Claude or Codex model without tools. Codex proposals reuse the restricted sandbox and configuration preflight.

The broker exposes `list_files`, `read_file`, explicitly authorized `write_file`,
and `run_checks` only when fixed commands are configured in the node. It rejects
traversal, symlinks and Git metadata paths. Writes require the last observed
content hash and check filesystem/Git metadata for intervention. Content digests
for Git control files ensure same-size HEAD or ref changes
remain detectable when filesystem timestamps coincide. Control-file reads are
bounded to 64 MiB per file. These checks are
not an atomic compare-and-swap against every external process. Detected changes,
conversation activity, cancellation or revoked grants stop further broker work
and require reconciliation. File operations are bounded to 256 KiB UTF-8,
20,000 entries and 100 calls per worker. The deadline is five minutes. Claude has
a CLI turn limit; Codex has no equivalent native model-turn limit, so its policy
bounds broker calls instead. Those are different measures, not guaranteed parity.

`run_checks` accepts no model-authored commands. It executes the node's frozen
executable/argument lists and returns real bounded stdout/stderr. The model can
edit, inspect failures and retry within the check budget. An explicit
`workspace.validate` node runs independent backend checks; the saved policy
requires their successful evidence before commit/publication by default. A
model's success claim is never validation evidence. The command list must also be
literal saved configuration: event and node-output references cannot choose the
independent checks.
Validation evidence binds the exact source fingerprint and local Git HEAD.
`workspace.commit` and `github.publish` require independent explicit grants.
Publication approval, when enabled, binds the exact local commit, separate from the
observed remote PR SHA. Git operations are deterministic host capabilities,
not shell tools exposed to a model. Failed intent persistence prevents dispatch;
uncertain effects remain blocked rather than being retried automatically.

`policy.requireLocalChecks` defaults to `true`, including legacy workflow and run
snapshots. Setting it to `false` explicitly allows commit and publication without
local commands or successful local validation evidence. This supports GitHub CI
only: a new commit is published before its CI result exists, and subsequent PR
polling (or an explicit wait/query in the graph) observes its checks. This mode
does not attest CI success or relax merge checks. `requirePublishApproval=false`
independently removes the per-publication human gate; explicit approval nodes
still pause. Checks that actually ran retain their source/HEAD binding across a
commit; a commit never manufactures a successful validation record. MCP and
provider-generated drafts cannot disable either requirement. Selected editor
policy is restored only for an unchanged scope. Older applications ignore the
new flag and continue requiring local checks; no persisted schema migration is
needed for this additive field.

Local validation executes only inside an OS sandbox: Linux requires usable bubblewrap
with an isolated network namespace, filesystem mounts and a syscall filter;
macOS requires sandbox-exec. Missing isolation fails closed. Git metadata,
credentials and secret paths are masked. Fixed programs can run repository code,
so the sandbox boundary is required even though commands are configured by a
person. Dependency installation does not run automatically. Compatible existing
Node dependencies may be exposed read-only from the mapped source checkout;
external package stores or unavailable toolchains can require human preparation.
Linux permits isolated IPv4/IPv6 loopback test servers but denies host Unix
sockets and egress. macOS validation denies network access, including local test
servers. Platform-dependent checks must respect these limits.

GitHub publication currently requires a same-repository PR, the saved connected
identity, a clean local commit descending from the observed head, and
an unchanged remote branch. A conditional SHA lease closes the remote update
race; ancestry is checked independently and divergent overwrites are refused.
Fork publication is rejected. Hooks, filters, inherited credential overrides and
unexpected repository configuration cannot silently broaden execution. Remote
publication does not imply the new CI run has passed; subsequent polling observes
its checks and feedback as a new event.

The worker retains finite nonnegative cost estimates and bounded token usage
from the provider's outer response envelope, never from model-authored output.
Claude receives the remaining optional USD budget. Missing usage, timeouts or
incomplete calls remain unknown and block further paid work under a configured
cap. Codex and Jev lack the required hard USD cap and refuse budget-constrained
execution before a paid call. Estimates are not billing reconciliation.

The shared defaults in
[`automation-catalog.json`](../../src/automation-catalog.json) start disabled,
without file-write, commit or push grants. The PR follow-up template diagnoses
feedback; the Node.js repair template includes fixed checks, independent
validation, commit, human approval and guarded publication. Its `npm test`
command must be reviewed for the selected project. The risk template requires
complete diff evidence before Jev, then a local condition, human approval and the
independent merge gate. The Linear template maps the project and reserves a
visible workspace before investigation. Legacy tab actions remain unchanged.
