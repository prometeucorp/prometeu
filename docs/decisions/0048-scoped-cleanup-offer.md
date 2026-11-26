---
type: Decision
title: "ADR 0048 — Offer scoped worktree cleanup after archiving"
description: "Prometeu offers worktree cleanup for one workspace after the archive or finish operation."
status: stable
---

# ADR 0048 — Offer scoped worktree cleanup after archiving

Date: 2026-09-17

## Context

The archive and finish operations kept the worktree, the local branch and the
transcript. This was intentional. The person was able to start cleanup only
from the screen of archived workspaces. Thus, to get disk space back, the
person had to remember a different action at a subsequent time.

When Prometeu removes a worktree, it can also remove work that the person did
not commit or merge. The current cleanup dialog puts this risk into classes.
For a risk, the person must select force. The dialog also keeps original
clones safe.

## Options considered

1. Delete all worktrees automatically after the archive operation.
2. Add a new confirmation prompt that copies the safety rules of cleanup.
3. Open the current cleanup dialog only for the workspace that the person
   archived.

## Decision

By default, after a manual archive or finish that completes correctly, open the
cleanup dialog with only that workspace. Keep archive and cleanup as two
different decisions:

- The dialog selects a safe row at the start.
- A blocked row is not selected at the start. The person must select it before
  the destructive action becomes available.
- The dialog tells that cleanup removes the folder. For a branch that Prometeu
  made for the workspace, cleanup also removes the local branch. The archived
  card and the PR metadata stay.
- The title of the scoped dialog tells that Prometeu archived the workspace
  before. The button that closes the dialog shows **Keep worktree**, not
  **Cancel**. Thus the person does not think that the cleanup selection is a
  confirmation of the archive operation (issue #100).
- If the person keeps the worktree, **Unarchive** stays available. The full
  cleanup screen keeps its usual **Cancel** label.

Use the same `cleanup_list` scan and `cleanup_worktree` command as the full
cleanup screen. The scan tells which worktrees cleanup can remove. Do not offer
cleanup for original clones, remote workspaces or workspaces that cleanup
removed before. Prometeu does not save a cancellation. Cleanup stays available
from the screen of archived workspaces.

Settings / General also has an optional automatic cleanup (issue #260). The
person must select it. This setting is local to the webview. It is off after a
new installation and after an upgrade. When the setting is on, Prometeu does
these steps:

- It examines only the selected row from the current scan.
- It removes the worktree with `force: true`. When the person selects the
  setting, the person accepts the loss of work that is not committed or not
  merged. This is necessary because the merge detection of Git does not find
  squash merges. It also does not find PRs that the GitHub integration cannot
  read.
- Force continues to apply only to an archived checkout that is not an
  original clone. It keeps the saved branches that the person selected as
  current branches.

The archive and finish commands of the desktop wait until the foreground
archive hook stops. Then they send their response. If the commands only start
the hook, the automatic deletion can occur at the same time as the cleanup of
the resources. Hooks must not move this cleanup work to a background process.
This keeps the current rules for the exit status, and it does not add a hook
timeout.

The desktop prevents new launches in the workspace while it does this cleanup
of resources. It waits until the launches that it accepted before publish their
tabs. It stops the agents before the hook and again before the archive
operation completes. After the archive operation, the desktop accepts launches
again. But the person must unarchive an archived workspace before an agent can
start in it again. The same gate, which is only in memory, prevents cleanup
while an archive hook operates again.

Prometeu shows errors and does not try again. Manual cleanup does not change.
The setting tells that it removes the full worktree, with the ignored files.
It does not remove only `node_modules`. This decision uses the current cleanup
boundary. It does not add a second policy for deletions in the file system.
The person makes one saved selection, and does not accept each deletion again. This
selection also applies to work that is not committed or not merged.

## Consequences

The person can get disk space back at the step of the lifecycle where this is
important. The protections of the backend stay as strong as before. The person can
continue to use force cleanup on a blocked worktree. But the person must select
the row with the risk mark and then click the destructive action.

Cleanup deletes the branches that Prometeu made for the workspace. It keeps the
branches that were in the clone before and the branches that the person
selected as current branches when Prometeu made the workspace. The saved
`preserve_branches` list records the clone paths independently. An older board
has an empty list by default and keeps its previous cleanup behavior. It is not
necessary to merge a branch that cleanup keeps before Prometeu removes its clean
worktree. If the person does not accept the offer, the diff stays available,
and the person can start cleanup at a subsequent time.

## Evidence

- `src/cleanup.test.ts` examines the defaults of the setting, how Prometeu saves
  it, the automatic scoped cleanup, blocked or missing rows, and errors without
  force.

- `e2e/audit-regressions.spec.ts` examines scoped offers after archive and
  after finish. It makes sure that only the related workspace shows. It also
  examines the archived state in the dialog title, in the English UI. It also
  examines a kept worktree that the person unarchives at a subsequent time.
- The tests in `src-tauri/src/session.rs` examine which archived worktrees
  cleanup can remove, the force protections, worktrees with changes and commits
  that are not in the base.
- [`../contracts/git.md`](../contracts/git.md) specifies how cleanup operates
  and which data it keeps.
