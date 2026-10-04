# Versioned automations

Status: implemented native desktop executor; platform and provider limits below.
Decision: [ADR 0087](../decisions/0087-versioned-automation-workflows.md).

## Definition and ownership

A workflow is independent of a conversation tab. Its JSON graph is the source of
truth for the visual editor, MCP, validation, simulation and runtime. Nodes have
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

## Events and execution

Polling is deterministic and happens only while the desktop host is running.
Minimum configured cadence is 60 seconds; API latency means it is not a delivery
guarantee. Offline failures preserve the cursor and surface a diagnostic. A
closed app or suspended computer does not poll. There is no hosted executor.

Initial observation establishes a baseline unless inclusion of existing items was
explicitly chosen. Scope binds local project, repository and connected identity;
multiple targets have explicit mappings. Account changes cannot silently adopt a
new identity. Deduplication and resource reservations are persisted with accepted
work. Selected branches are processed from the actual graph. Wait and approval
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

The editor reuses the shared design system. Adding, moving, connecting, editing
or deleting a node edits the same draft. Navigation preserves its unsaved state.
The assistant produces a proposed graph; the person reviews structural changes,
including policy and scope, before applying it. Applying to the editor is separate
from saving/activating the persisted workflow. CLI authentication failures stay
visible rather than generating scripted responses.

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
- `automations_propose({prompt, projectId?, workflow?})`: real tool-free proposal.
- `automations_run({id, event?})`: explicit execution.
- `automations_approve({runId, nodeId, headSha?})`: human approval.
- `automations_resume({runId})` and `automations_cancel({runId})`: explicit recovery;
  unresolved effects cannot be blindly retried.

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
an automation worker. Proposal generation currently uses tool-free Claude.

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
edit, inspect failures and retry within the check budget. The backend separately
runs `workspace.validate`; a model's success claim cannot authorize a commit.
Its command list must also be literal saved configuration: event and node-output
references cannot choose the independent checks.
Validation evidence binds the exact source fingerprint and local Git HEAD.
`workspace.commit` and `github.publish` require independent explicit grants.
Publication approval binds that validated local commit, separate from the
observed remote PR SHA. Git operations are deterministic host capabilities,
not shell tools exposed to a model. Failed intent persistence prevents dispatch;
uncertain effects remain blocked rather than being retried automatically.

Validation executes only inside an OS sandbox: Linux requires usable bubblewrap
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
identity, a clean validated local commit descending from the observed head, and
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
