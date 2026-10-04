# ADR 0087 — Versioned automation workflows

Date: 2026-10-04
Status: Accepted

## Context

The existing action monitor is bound to a task tab and copies a prompt/profile.
Persistent routines need explicit decisions, inspectable execution state and one
editable definition shared by conversation and a visual editor. Polling a model
would spend work when nothing changed and would obscure authorization.

## Options considered

- Extend profile prompts with additional implicit orchestration.
- Adopt an unrestricted script language or a separate workflow-editor framework.
- Use a small typed DAG, operation registry and durable host around existing ports.

## Decision

Use a versioned DAG owned by the portable Rust core. Keep provider, GitHub,
Linear, filesystem and process adaptations in the desktop host. Keep the editor
on the current TypeScript DOM/design-system stack. Chat proposes the same JSON
that the editor edits; proposals do not directly activate routines.

Persist revision snapshots, event deduplication, resource reservations and effect
intent separately from the board's legacy Actions. No implicit migration grants
permissions. Pure conditions, remote Jev inference and tool-using agents remain
different node types. Restrict automated worker capabilities in the adapter;
never interpret a prompt as a security boundary. Every merge passes the native
policy gate even if the graph contains a direct edge to that action.

Use a restricted file/check broker for automated code work. Fixed validation
commands run in an OS sandbox because repository scripts remain untrusted code.
Bind independent validation, commit and human publication approval to the same
source fingerprint and local HEAD; recheck the remote SHA before publication.
Separate file-write, commit and push grants. Keep provider-specific constraints
explicit: an unavailable hard cost cap or configuration boundary refuses work
before a paid call rather than silently weakening policy.

## Consequences

The graph is inspectable and simulation can prove routing without effects. A
finite DAG deliberately excludes user-defined loops; cadence, retries and waits
belong to the engine. The initial host remains local: closing it pauses polling.
External effects may require reconciliation after a crash. Unknown usage is not
reported as a zero cost. Unsupported provider capabilities fail explicitly.

## Evidence

The [automation contract](../contracts/automations.md) defines the persisted and
IPC boundaries, compatibility, support differences and test locations.
