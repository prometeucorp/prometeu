---
type: Decision
title: "ADR 0048 — Offer scoped worktree cleanup after archiving"
description: "Scoped worktree cleanup after archiving or finishing."
status: stable
---

# ADR 0048 — Offer scoped worktree cleanup after archiving

Date: 2026-09-17

## Context

Archiving and finishing deliberately preserved the worktree, local branch and
transcript. Cleanup was available only from the archived-workspaces screen, so
recovering disk space depended on remembering a separate later action.

Removing a worktree can also remove uncommitted or unmerged work. The existing
cleanup dialog already classifies that risk, requires explicit force selection
and protects original clones.

## Options considered

1. Delete every worktree automatically after archiving.
2. Add a new confirmation prompt that duplicates cleanup safety rules.
3. Open the existing cleanup dialog scoped to the workspace just archived.

## Decision

By default, after a successful manual archive or finish, open the cleanup dialog
with only that workspace. Keep archive and cleanup as separate decisions. A safe row
starts selected; a blocked row starts unselected and must be selected before the
destructive action becomes available. The dialog states that cleanup removes
the folder and, for a newly created branch, the local branch, while preserving
the archived card and PR metadata. The scoped title explicitly states that the
workspace is already archived, and
the dismissal button says **Keep worktree** instead of **Cancel**. This avoids
presenting the cleanup choice as an archive confirmation (issue #100). Keeping
the worktree leaves **Unarchive** available; the full cleanup screen retains its
ordinary **Cancel** label.

Use the same `cleanup_list` eligibility scan and `cleanup_worktree` command as
the full cleanup screen. Do not offer cleanup for original clones, remote
workspaces or workspaces already cleaned. A cancellation is not persisted;
cleanup remains available from the archived-workspaces screen.

Settings / General also offers explicit opt-in automatic cleanup (issue #260).
The preference is local to the webview and off on new installs and upgrades.
When enabled, inspect only the selected row from the existing scan, then clean
it with `force: true`: opting in is the approval to lose uncommitted or unmerged
work, because Git merge detection misses squash merges and PRs the GitHub
integration cannot read. Force still requires an archived, non-original
checkout and keeps saved existing branches. Desktop archive/finish
commands wait for the foreground archive hook to exit before replying; merely
spawning it would let automatic deletion race resource teardown. Hooks must
not detach teardown work. This preserves existing exit-status policy and adds
no hook timeout. Desktop closes per-workspace launch admission across teardown, waits for
already admitted launches to publish their tabs, and stops agents both before
the hook and before archive completion. Admission reopens after completion,
but an archived workspace must be explicitly restored before launching again.
The same memory-only gate excludes cleanup during a repeated archive hook.
Errors are reported without retrying. Manual cleanup remains unchanged. The setting explains that this
removes the whole worktree, including ignored files, rather than only
`node_modules`. This reuses the cleanup boundary instead of introducing a
second filesystem deletion policy. It trades a repeated confirmation for an
explicit persistent choice that also covers dirty or unmerged work.

## Consequences

People can reclaim disk space at the lifecycle point where it becomes relevant
without weakening the backend guards. Blocked worktrees still permit explicit
force cleanup, but require both selecting the risk-marked row and confirming the
destructive action.

Cleanup deletes branches created for the workspace but preserves branches
already present in the clone or selected as existing at creation. The persisted
`preserve_branches` list records clone paths separately; an older board defaults
to an empty list and keeps its previous cleanup behavior. A preserved branch
does not need to be merged before its clean worktree is removed. Refusing the
offer keeps the diff reachable and does not suppress future cleanup access.

## Evidence

- `src/cleanup.test.ts` covers preference defaults, persistence, automatic
  scoped cleanup, blocked/missing rows and failures without force.

- `e2e/audit-regressions.spec.ts` covers scoped offers from both archive and
  finish and confirms that only the affected workspace appears. It also covers
  the explicit archived state and keeping a worktree for later restoration in
  the English UI.
- `src-tauri/src/session.rs` tests cover archived eligibility, force guards,
  dirty worktrees and commits relative to the base.
- [`../contracts/git.md`](../contracts/git.md) defines cleanup behavior and
  preserved data.
