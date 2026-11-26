---
type: Decision
title: "ADR 0043 — Retire unused desktop IPC commands"
description: "Remove the desktop IPC commands that the application does not call."
status: stable
---

# ADR 0043 — Retire unused desktop IPC commands

Date: 2026-09-15

The decisions about the Git index and about each repository in
[ADR 0008](0008-explicit-git-index.md) use the current IPC that this ADR
specifies.

## Context

The desktop does not call `workspace_diff`, `resume_tab` or `catalog_refresh`
at this time. Their registrations, mock handlers and TypeScript declarations
keep entry points that nothing uses. The previous diff also keeps a separate
implementation and tests for a presentation that is not in the product now.

## Options considered

1. Keep the unused commands for an unlimited time for possible callers.
2. Remove them from the desktop contract in the bundle, and keep the paths that
   the application uses.

## Decision

Remove the three commands from Rust, from the IPC map and from the browser mock.
The current paths are these:

- Git uses `workspace_git_*`.
- When a person sends a message, the application resumes a stopped tab through
  `chat_send` and `revive`.
- When the application refreshes the account, `cloud_status` with
  `refresh: true` loads the catalog.

Keep these items:

- The shared helpers that the Git cleanup and the current diff use.
- The code that loads the current team configuration.
- All the formats of the stored data and of the relay.

## Consequences

The frontend and the Rust backend are in the same desktop bundle. Thus no
caller that the product supports must have a deprecation shim. If a frontend
that a person changed calls a retired command, it gets an unknown-command
error. This decision makes the internal IPC contract smaller on purpose. The
visible behavior does not change.

A migration test for a stored format does not apply, because no stored schema
changes. The mobile clients and the relay clients do not call these desktop
commands.

## Evidence

- [IPC parity](../../src-tauri/tests/mock.rs) makes sure that Rust, TypeScript
  and the browser mock agree.
- [IPC types](../../src/ipc.test.ts) refuse the names of retired commands.
- [Git tests](../../src-tauri/crates/git/src/tests.rs) examine the current Git
  operations. [Session tests](../../src-tauri/src/session.rs) continue to
  examine the commit counter of the cleanup.
- [Cloud flows](../../e2e/cloud.spec.ts) and
  [Git flows](../../e2e/git.spec.ts) operate the interface paths that the
  product supports.
