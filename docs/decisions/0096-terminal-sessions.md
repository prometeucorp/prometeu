---
type: Decision
title: "ADR 0096 — Terminal sessions owned by the app"
description: "Terminal sessions owned by the app, persisted as records on the board."
status: stable
---

# ADR 0096 — Terminal sessions owned by the app

Date: 2026-10-27

Builds on the dock shells of workspaces and projects described in the
[IPC contract](../contracts/ipc.md) and on the portable board of
[ADR 0085](0085-portable-core.md).

## Context

Every terminal belonged to a workspace or, since #26, a registered project. A
shell needed one of them as its owner: `open_dock` resolved its folder through
the owner and refused anything else, and the only entry points lived in a
workspace's or project's tab strip. Running `gh`, `brew`, a server or an agent
CLI by hand meant registering a project or creating a workspace (worktree,
branch, conversation) that the person did not need. Nothing about a shell
persisted, so a restart forgot it.

## Options considered

1. **Ephemeral sessions.** Shells without an owner, kept only in memory like dock
   shells. No persisted format changes, but a restart loses the list the person
   arranged and named.
2. **Persisted session records.** A record per session (id, folder, optional
   name, order) on the board. The record outlives the process, the same way a
   workspace outlives its agent; scrollback stays in memory.
3. **Project-view shells as sessions.** Turn `<project>:terminal-N` shells into
   sessions rooted at the project folder. One concept fewer, but it changes a
   flow that works and couples the project view to the new list.

## Decision

Option 2, keeping project-view shells as they are.

- **Record.** `Board.terminals` holds `{ id, folder, name? }` in sidebar order. The
  id is `terminal-<uuid>`: it has no `:`, so `<id>:terminal` is an ordinary dock
  key, and its prefix never collides with a workspace UUID or a project path.
  The field is written only when a session exists, so older boards load and save
  unchanged.
- **Shell.** A session runs one dock shell under `<id>:terminal` through
  `open_dock`, so spawning, launcher-environment cleanup, retained output,
  reattachment and the status bar are the ones dock shells already use. It gets
  no workspace variables (`PROMETEU_*`, `CONDUCTOR_*`, `PORT`).
- **Folder.** Creation without a folder uses the home folder; a chosen folder
  must exist and is never registered as a project. A shell starts only in a
  folder that exists, for every owner: the native PTY would otherwise fall back
  to the home folder. Sessions are not file roots; only shells resolve them, so
  the file commands behind `cwd_of` cannot reach the home folder.
- **Lifetime.** × and `exit` end the session: its shell stops and the record
  goes. The frontend ends it on `pty-closed` for its key, which the desktop does
  not emit for a shell it killed. Quitting the app only stops the shell, so the
  session is listed again, stopped, and opening it starts a new shell in the
  same folder. Workspace and project changes never touch sessions.
- **Names.** A name the person sets wins over the folder's name; clearing it
  falls back. Program titles from #202, once they land, sit between the two.
- **Local only.** Sessions never reach the relay, the delegation MCP, telemetry,
  the desk, the notch island or agent notifications; those read workspaces and
  their tabs only.
- **Windows.** The WSL runtime has no sessions yet. The Windows entry point hides
  every entry; the runtime rejects the commands and carries `Board.terminals`
  through its catalog unchanged.

## Consequences

- A shell no longer needs a project or workspace, and the list survives restarts
  without keeping processes alive.
- An older app version ignores `Board.terminals` and drops it on its next save,
  so a downgrade loses the list but nothing else.
- Scrollback does not survive quitting; a restored session starts a new shell.
- Windows needs its own implementation in the runtime before the entries can
  show there.

## Evidence

- `crates/core/src/terminal_sessions/tests.rs`: ids, names, removal and a board
  written by v0.29.0 that loads and saves back unchanged.
- `src-tauri/src/terminal_sessions.rs`: shells start only in an existing folder,
  and closing a session stops only its shell.
- `src-tauri/src/machine.rs`: resource rows name the session, not its id.
- `src/terminals.test.ts`: names, the end of a session whose shell exits, creation
  and live state.
- `fixtures/backend-contract.json`: the board's `terminals` shape against the
  TypeScript contract.
