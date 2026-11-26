---
type: Decision
title: "ADR 0044 — Current documentation with history in Git"
description: "The documentation tells the current decisions, and Git keeps the history."
status: stable
---

# ADR 0044 — Current documentation with history in Git

Date: 2026-09-15

## Context

The accepted ADRs collected instructions that were not current, with
partial notes about the replacement decisions. Thus a reader of one document
can think that some items were in the product, but they were not in that
form at that time. These items were an importer, sound alerts, a manual
review of keys and a mobile implementation. Also, the decision index had
two copies, and ADR 0034 was not in the lifecycle index.

## Options considered

1. Keep all the historical ADRs adjacent to the current decisions.
2. Move the previous documents into an archive in the documentation tree.
3. Keep the current decisions in the tree and use Git for the history.

## Decision

Adopt option 3. Accepted ADRs describe implemented decisions that are in force.
They include the causes, the trade-offs, the compatibility and the limits of the
verification. Update a decision when its implementation changes. Add a new ADR
for a different choice when that makes the responsibility for the decision
clearer.

When you replace or remove a decision, do these steps:

1. Move the guarantees that stay in force into the current ADR or contract.
2. Update the links that go to the previous document.
3. Delete the previous document.

Do not keep two instructions that do not agree, with a note about the
replacement. Do not change ADR numbers, and do not use a retired number again.
Apply the same rule to retired contracts. Write the compatibility that the code
continues to implement in the current contract. Proposals must have a clear
mark and must stay separate from the accepted guidance.

Concepts use the lifecycle field that [ADR 0001](0001-repository-knowledge.md)
specifies:

- `stable` is for accepted guidance.
- `draft` is for proposals.
- `deprecated` is only for previous guidance whose compatibility the product
  continues to support.

Deprecation is not an archive for replaced decisions. When an implementation
plan is complete, record its current guarantees and verified limits in
contracts or ADRs. Then remove the plan from `docs/`. The Git history keeps
the previous plans and audit snapshots.

## Consequences

A reader can use each accepted decision alone. The reader does not put
together a sequence of previous ADRs. A person can get the historical causes and the deleted
files again through `git log --all -- docs/decisions/` and
`git show <commit>:<path>`. The trade-off is that a person who studies past
alternatives must read the history of the repository.

When a person removes a previous document, this does not let the person remove
compatibility code, stored data or security guarantees. The tests and the code
set the current behavior. An audit of this checkout does not prove the state of
a deployment.

## Evidence

- [Lifecycle and current index](README.md).
- [Agent maintenance rules](../../AGENTS.md).
- `npm run docs:check` examines the local links and the documentation index.
- Each ADR in the tree links to or names its implementation and the evidence of
  its verification.
