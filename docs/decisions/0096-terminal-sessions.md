---
type: Decision
title: "ADR 0096 — Terminal sessions owned by the app"
description: "The app owns terminal sessions and keeps them as records on the board."
status: stable
---

# ADR 0096 — Terminal sessions owned by the app

Date: 2026-10-27

This ADR adds to the dock shells of workspaces and projects in the
[IPC contract](../contracts/ipc.md). It also adds to the portable board of
[ADR 0085](0085-portable-core.md).

## Context

Each terminal was part of a workspace or, after #26, of a project on the
board. A
shell had to have one of them as its owner:

- `open_dock` found the folder of the shell through the owner, and it rejected
  all other shells.
- The only entry points were in the tab strip of a workspace or a project.

A person had to add a project to the board or make a workspace (worktree,
branch, conversation) to run `gh`, `brew`, a server or an agent CLI. The
person did not have a use for these items. Prometeu did not keep information
about a shell. Thus the shell was not there after the app started again.

## Options considered

1. **Ephemeral sessions.** Shells without an owner that Prometeu keeps only in
   memory, as dock shells. The stored format does not change. But after the
   app starts again, the list that the person put in order and named is not
   there.
2. **Stored session records.** The board has a record for each session (id,
   folder, optional name, order). The record stays after the process stops, as
   a workspace stays after its agent stops. The scrollback stays in memory.
3. **Project-view shells as sessions.** Change the `<project>:terminal-N`
   shells into sessions with their root at the project folder. There is one
   concept less. But it changes a flow that operates correctly, and it connects
   the project view to the new list.

## Decision

Use option 2. Keep the project-view shells as they are.

- **Record.** `Board.terminals` holds `{ id, folder, name? }` in the order of
  the sidebar. The id is `terminal-<uuid>`. It does not contain `:`. Thus
  `<id>:terminal` is a usual dock key, and its prefix is not the same as a
  workspace UUID or a project path. Prometeu writes the field only when there
  is a session. Thus boards from earlier versions load and save without a
  change.
- **Shell.** A session runs one dock shell with the `<id>:terminal` key through
  `open_dock`. Thus the session uses the same functions as dock shells:
  - The start of the process and the cleanup of the launcher environment.
  - The output that Prometeu keeps and the reattachment.
  - The status bar.

  The shell does not get the workspace environment values (`PROMETEU_*`, `CONDUCTOR_*`,
  `PORT`).
- **Folder.** When the person does not select a folder, the session uses the
  home folder. A folder that the person selects must be on the disk, and
  Prometeu does not add it to the board as a project. For all owners, a shell starts
  only in a folder that is on the disk. If not, the native PTY uses the home
  folder. Sessions are not file roots. Only shells find their folder. Thus the
  file commands that use `cwd_of` cannot get access to the home folder.
- **Lifetime.** × and `exit` stop the session. The shell stops and Prometeu
  removes the record. The frontend stops the session when `pty-closed` comes for
  its key. The desktop does not send this event for a shell that it stopped.
  When the person stops the app, Prometeu stops only the shell. Thus the list
  shows the session again as stopped. When the person opens it, a new shell
  starts in the same folder. Changes to workspaces and projects do not change
  sessions.
- **Names.** Prometeu shows a name that the person sets, not the name of the
  folder. When the person removes it, the session uses the name of the folder
  again. When the program titles from #202 are available, Prometeu will use them
  after the name from the person and before the name of the folder.
- **Local only.** Sessions do not go to the relay, the delegation MCP, the
  telemetry, the desk, the notch island or agent notifications. These
  components read only workspaces and their tabs.

## Consequences

- A project or a workspace is not necessary for a shell. The list stays after
  the app starts again, and Prometeu does not keep processes in operation.
- An earlier version of the app ignores `Board.terminals` and removes it at its
  next save. Thus a downgrade removes the list, but no other data.
- The scrollback does not stay after the person stops the app. A session that
  Prometeu shows again starts a new shell.

## Evidence

- `crates/core/src/terminal_sessions/tests.rs`: ids, names, removal and a board
  from v0.29.0 that loads and saves without a change.
- `src-tauri/src/terminal_sessions.rs`: shells start only in a folder that is
  on the disk. When a session closes, Prometeu stops only its shell.
- `src-tauri/src/machine.rs`: resource rows show the name of the session, not
  its id.
- `src/terminals.test.ts`: names, the end of a session when its shell stops,
  creation and live state.
- `fixtures/backend-contract.json`: the shape of `terminals` on the board
  against the TypeScript contract.
