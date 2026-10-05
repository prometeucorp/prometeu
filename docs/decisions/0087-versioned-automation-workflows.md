# ADR 0087 — Versioned automation workflows

Date: 2026-10-04
Status: Accepted

## Context

The existing action monitor is bound to a task tab and copies a prompt/profile.
Persistent routines need explicit decisions, inspectable execution state and one
editable definition shared by conversation and visualization. Polling a model
would spend work when nothing changed and would obscure authorization.

## Options considered

- Extend profile prompts with additional implicit orchestration.
- Adopt an unrestricted script language or a separate workflow-editor framework.
- Use a small typed DAG, operation registry and durable host around existing ports.

## Decision

Use a versioned DAG owned by the portable Rust core. Keep provider, GitHub,
Linear, filesystem and process adaptations in the desktop host. Keep the editor
on the current TypeScript DOM/design-system stack. Chat proposes the same JSON
that the runtime executes; proposals do not directly activate routines.
The assistant selects a Claude or Codex model for each proposal. Both providers
run without tools; Codex also keeps its restricted sandbox and configuration preflight.
The conversation carries bounded user/assistant history and permits replies with
no graph, so clarification does not create a speculative workflow. Conversation
messages remain in the mounted screen; only explicitly saved workflows enter
the durable automation store. This reuses the bounded proposal adapter while
keeping provider sessions and execution authority outside the editor.

Use a read-only Mermaid projection and a complete textual Steps view instead of
maintaining a custom canvas editor. Graph logic is edited through conversation
and reviewed proposals. This trades direct offline graph editing for a simpler
interaction and automatic diagram layout; model availability and usage limits
therefore also constrain graph editing. Keep policy, scope, activation and run
controls independent of the model. Persisted positions remain compatible but do
not control Mermaid layout. Do not persist Mermaid or let models author it.
Entity-encode graph labels and use generated IDs with strict rendering and no
HTML labels, links or callbacks. The overview explicitly counts omitted exception
edges; the complete diagram and Steps preserve access to every branch.

Persist revision snapshots, event deduplication, resource reservations and effect
intent separately from the board's legacy Actions. No implicit migration grants
permissions. Pure conditions, remote Jev inference and tool-using agents remain
different node types. Restrict automated worker capabilities in the adapter;
never interpret a prompt as a security boundary. Every merge passes the native
policy gate even if the graph contains a direct edge to that action.

Separate observation occurrences from content fingerprints: scope and a durable
cursor counter identify an admitted event, so returning to an earlier state does
not suppress new work. Derive pending approval in the run read model from graph
readiness rather than duplicating execution state in history or persisted fields.

Use a restricted file/check broker for automated code work. Fixed validation
commands run in an OS sandbox because repository scripts remain untrusted code.
Default to independent local validation before commit/publication. Let the person
explicitly choose GitHub CI after push with `requireLocalChecks=false` and remove
per-publication approval independently. This permits publishing code before its
CI outcome is known; it never implies CI passed. Bind any local validation and
configured human approval to the source fingerprint and local HEAD. Recheck the
remote SHA before publication, regardless of validation mode.
Separate file-write, commit and push grants. Keep provider-specific constraints
explicit: an unavailable hard cost cap or configuration boundary refuses work
before a paid call rather than silently weakening policy.

Reuse the chat presentation components while keeping the draft controller and
restricted proposal transport separate. Constrain provider replies with a full
workflow output schema. Encode only arbitrary JSON leaves at the provider edge;
this avoids an unconstrained whole-graph string while retaining operation-specific
inputs and the existing persisted graph. Native validation remains authoritative.
Forward graph validation feedback on explicit retries without automatic model calls.
Before showing a proposal, run the native save validator as well as graph validation.
Supply the fixed worker result contract in the adapter instead of asking the model
to echo a quoted JSON enum that strict output formats reject. Validate explicit
legacy contracts without replacing them. The host binds
missing GitHub identities using the connected account and preserves already reviewed
bindings for unchanged project/repository pairs. The model cannot discover or assign
account authority; changed scope resets mutation grants, required local checks
and approvals, and remains reviewable. Repair proposals can be designed with
conservative draft permissions; the person grants execution authority once in
the editor, without making the model an authority source.

## Consequences

The graph is inspectable and simulation can prove routing without effects. A
finite DAG deliberately excludes user-defined loops; cadence, retries and waits
belong to the engine. The initial host remains local: closing it pauses polling.
External effects may require reconciliation after a crash. Unknown usage is not
reported as a zero cost. Unsupported provider capabilities fail explicitly.

## Evidence

The [automation contract](../contracts/automations.md) defines the persisted and
IPC boundaries, compatibility, support differences and test locations.
