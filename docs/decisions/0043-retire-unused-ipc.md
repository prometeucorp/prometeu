# ADR 0043 — Retire unused desktop IPC commands

Date: 2026-09-15
Status: Accepted

The Git index and per-repository decisions in
[ADR 0008](0008-explicit-git-index.md) use the current IPC described here.

## Context

The desktop no longer calls `workspace_diff`, `resume_tab` or `catalog_refresh`.
Their registrations, mock handlers and TypeScript declarations keep unused
entry points alive. The old diff also retains a separate implementation and
tests for a presentation that no longer exists.

## Options considered

1. Keep the unused commands indefinitely for hypothetical callers.
2. Remove them from the bundled desktop contract and preserve the live paths.

## Decision

Remove all three commands from Rust, the IPC map and the browser mock.
Git uses `workspace_git_*`; sending a message resumes a stopped tab through
`chat_send` and `revive`; account refresh pulls the catalog through
`cloud_status` with `refresh: true`.

Keep shared helpers used by Git cleanup and the current diff, existing team
configuration loading, and all persistence and relay formats.

## Consequences

The frontend and Rust backend ship in the same desktop bundle, so no supported
caller needs a deprecation shim. An independently modified frontend calling a
retired command receives an unknown-command error. This deliberately narrows
the internal IPC contract without changing visible behavior.

No persisted-format migration test applies: no stored schema changes. Mobile
and relay clients do not call these desktop commands.

## Evidence

- [IPC parity](../../src-tauri/tests/mock.rs) checks agreement between Rust,
  TypeScript and the browser mock.
- [IPC types](../../src/ipc.test.ts) reject retired command names.
- [Git tests](../../src-tauri/crates/git/src/tests.rs) cover the current Git
  operations; [session tests](../../src-tauri/src/session.rs) retain coverage
  of the cleanup commit counter.
- [Cloud flows](../../e2e/cloud.spec.ts) and
  [Git flows](../../e2e/git.spec.ts) exercise the supported UI paths.
