---
type: Decision
title: "ADR 0044 — Current documentation with history in Git"
description: "Current documentation with history in Git."
status: stable
---

# ADR 0044 — Current documentation with history in Git

Date: 2026-09-15

## Context

Accepted ADRs accumulated obsolete instructions followed by partial supersession
notes. Reading one document could suggest an importer, sound alerts, manual key
review or a mobile implementation that no longer exists in that form. Repeating
the decision index also left ADR 0034 absent from the lifecycle index.

## Options considered

1. Keep every historical ADR beside the current decisions.
2. Move obsolete documents into an archive in the documentation tree.
3. Keep current decisions in the tree and use Git for history.

## Decision

Adopt option 3. Accepted ADRs describe implemented decisions still in force,
including their rationale, trade-offs, compatibility and verification limits.
Update a decision when its implementation changes. Add a new ADR for a distinct
choice when that makes ownership clearer.

When a decision is replaced or removed, consolidate any guarantees still in
force into the current ADR or contract, update incoming links and remove the
obsolete document. Do not retain competing instructions with a supersession
note. Keep numbers stable and never reuse retired numbers. Apply the same rule
to retired contracts; document compatibility still implemented in the current
contract. Proposals must be explicitly marked and kept separate from accepted
guidance.

Concepts use the lifecycle field defined by [ADR 0001](0001-repository-knowledge.md):
`stable` for accepted guidance, `draft` for proposals and `deprecated` only for
legacy guidance whose compatibility remains supported. Deprecation is not an
archive for replaced decisions. Completed implementation plans leave `docs/`
after their current guarantees and verified limits are recorded in contracts
or ADRs; previous plans and audit snapshots remain in Git history.

## Consequences

Readers can use each accepted decision without reconstructing a chain of older
ADRs. Historical rationale and deleted files remain recoverable through
`git log --all -- docs/decisions/` and `git show <commit>:<path>`. The trade-off
is that studying past alternatives requires repository history.

Removing an obsolete document does not authorize removing compatibility code,
stored data or security guarantees. Tests and code determine the current
behavior; an audit of this checkout does not prove deployment state.

## Evidence

- [Lifecycle and current index](README.md).
- [Agent maintenance rules](../../AGENTS.md).
- `npm run docs:check` checks local links and the documentation index.
- Each retained ADR links or names its implementation and verification evidence.
