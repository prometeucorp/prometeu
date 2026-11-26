---
type: Decision
title: "ADR 0008 — explicit Git index and per-repository operations"
description: "The interface shows the Git index, the worktree and the conflicts, and it does the Git operations for each repository."
status: stable
---

# ADR 0008 — explicit Git index and per-repository operations

Date: 2026-09-05

## Context

The Changes view put the commits of the branch and the local changes into one
diff from the merge base. The filter for changed files did not change the base
of the patches. That model was satisfactory for a review. But it did not show
the index, which is necessary to prepare a commit. Agent sessions can
also continue to change files while the person does the review. A workspace can
also contain many independent repositories.

## Options considered

1. Add a stage to the current diff, and keep the patches mixed.
2. Show the index, the worktree and the conflicts independently. Keep the branch
   review and the history as explicit comparisons.
3. Give each operation to the agent or to an external Git client.

## Decision

Use the second option. The interface stages and commits for each repository.
It keeps a local commit and a push as two different operations. The backend
finds the paths from the workspace, and it owns the Git operations. The
presentation does not run shell commands. The new commands use
`workspace_git_*`. The project removed the IPC of the mixed diff, as
[ADR 0043](0043-retire-unused-ipc.md) tells. The current operations are in
[`../contracts/git.md`](../contracts/git.md).

The branch selection uses the launcher and the worktree lifecycle again. It does
not change the checkout of a workspace that has conversations that operate. The
colored avatar with the initial letter stays the visual identity of the
repository. The launcher can attach a new worktree to a current local branch.
It can also make a local branch from a remote ref. The branch must not be in
the worktree list of Git. Thus, the checkout selection stays explicit, and the
source clone does not change.

## Consequences

- Partially staged files show in the two groups, each with its own patch. New
  changes from the agent stay out of the commit.
- Local counters and upstream counters have different meanings.
- Git errors are explicit. A repository with a failure does not look clean.
- The interface keeps drafts. It ignores the
  responses of a previous navigation.
- The application does not become a full Git client. It does not add these
  operations:
  - A checkout under a running agent.
  - A force-push.
  - The staging of single lines.
  - The completion of a rebase.
- A person can discard unstaged work for one file at a time, after a
  confirmation. The discard applies only to the worktree side. The application
  refuses the discard while an agent runs. The application does not discard
  staged content or conflicts at any time.
- The decision does not change the storage, the agent contracts or the relay.

## Evidence

Tests with Git repositories are in `src-tauri/crates/git/src/tests.rs`. The
interface tests are in `e2e/git.spec.ts`. The IPC parity test is in
`src-tauri/tests/mock.rs`. The visual approval used a temporary prototype. That
prototype is not part of the versioned documentation.
