# ADR 0008 — explicit Git index and per-repository operations

Date: 2026-09-05
Status: Accepted

## Context

Changes gathered the branch's commits and local modifications into a diff from
the merge base. The dirty-file filter did not change the patches' base. That
model served review, but it did not describe the index needed to prepare a
commit. Agent sessions may also keep editing while the person reviews, and a
workspace may contain several independent repositories.

## Options considered

1. Add a stage to the existing diff, keeping patches mixed together.
2. Expose index, worktree and conflicts separately, preserving branch review and
   history as explicit comparisons.
3. Delegate every operation to the agent or to an external Git client.

## Decision

Adopt the second option. The UI stages and commits per repository, and separates
a local commit from a push. The backend resolves paths from the workspace and
owns the Git operations; the presentation does not run shell commands. The new
commands use `workspace_git_*`; the mixed-diff IPC has been removed as described
in [ADR 0043](0043-retire-unused-ipc.md). The current operations are described in
[`../contracts/git.md`](../contracts/git.md).

Branch selection reuses the launcher and the worktree lifecycle. It does not
switch the checkout of a workspace that has running conversations. The colored
initial avatar remains the repository's visual identity.
The launcher can attach a new worktree to an existing local branch or create a
local branch from a remote ref. The branch must be free in Git's worktree list.
This keeps checkout selection explicit and leaves the source clone unchanged.

## Consequences

- Partially staged files appear in both groups, with specific patches. New agent
  edits stay out of the commit.
- Local counters and upstream counters have distinct meanings.
- Git errors are explicit; a failing repository does not look clean.
- The UI keeps drafts and protects actions against old navigation responses.
- The app does not become a complete Git client: it does not add a checkout on
  top of agents, force-push, line staging or rebase completion.
- Discarding unstaged work is offered per file, confirmed, limited to the
  worktree side and refused while an agent runs; staged content and conflicts
  are never discarded.
- There is no change in persistence, agent contracts or the relay.

## Evidence

Real tests in `src-tauri/src/session/git_tests.rs`, UI in `e2e/git.spec.ts` and
IPC parity in `src-tauri/tests/mock.rs`. Visual approval happened in a temporary
prototype; that artifact is not part of the versioned documentation.
