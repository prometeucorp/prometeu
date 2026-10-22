# ADR 0092 — Terminal sessions owned by the person

Date: 2026-10-22
Status: Accepted

## Context

Every shell belonged to something else: a workspace, or since the project-only
view, a registered project. Starting a shell needed that owner's folder, creating
one needed the owner on screen, and archiving or removing the owner killed it.
Running `claude`, `gh` or a server by hand, or just looking around a folder,
therefore meant registering a project or creating a workspace with a worktree,
branch and conversation the person did not want
([#207](https://github.com/prometeucorp/prometeu/issues/207)).

## Options considered

- **Persist the session record in `board.json`.** The record outlives its
  process, as a workspace outlives its agent; after a restart the session is
  listed stopped and opening it starts a new shell in the same folder. Needs a
  new board field and its compatibility.
- **Keep sessions ephemeral, like dock shells.** No persisted format changes, but
  a restart forgets every session and the sidebar needs a separate event or
  command to list them.
- **Turn project-view shells into sessions rooted at the project.** Unifies two
  kinds of shell, but changes behavior the person already relies on.
- **Implement the same entity in the WSL runtime now.** Windows parity, at the
  cost of the runtime's catalog, `file_root` and dock rules in the same change.

## Decision

A terminal session is a local shell owned by the session itself. `Board.terminals`
keeps each record (`id`, `folder`, `title`, `running`) in sidebar order and is
written only when not empty. The id is `terminal-<uuid>`: it holds no `:` and
cannot collide with a workspace UUID or a project path, so dock keys
(`<id>:terminal`) and every `"<owner>:"` prefix rule keep working.

- **Create:** "New terminal" (sidebar, File menu, ⇧⌘T) starts in the home folder;
  "New terminal in a folder…" uses the directory picker. The backend accepts only
  an absolute existing directory. Creating a session registers no project,
  creates no worktree or branch, reserves no port and starts no agent.
- **Run:** its single shell goes through the dock shell path, with Prometeu's
  terminal identity and no workspace variables. Navigation keeps it alive and
  reattaches its retained output, even if its starting folder moved. A restored
  session whose folder is gone shows a translated error instead of starting
  elsewhere.
- **End:** × in the sidebar, Close in its menu, ⌘W on its view, or `exit` in the
  shell ends the session and stops the process (hangup, terminate, kill). Only a
  shell that ends on its own removes the record; closing, quitting the app and a
  replacement take the PTY out first. Starting, ending and closing are
  serialized, so a shell started as the previous one ends always keeps its
  record. Quitting the app keeps the session listed, and revival clears
  `running`.
- **Name:** the person's name wins over the folder's own name; an empty name
  returns to the folder name. Program titles from
  [#202](https://github.com/prometeucorp/prometeu/issues/202) will sit between
  them.
- **Scope:** sessions are local only. They never reach the relay, have no stage
  and no agent status, and stay out of the desk, the notch island and agent
  notifications. Archiving or removing a workspace or project leaves them
  running. Project-view shells are unchanged.
- **Windows:** the WSL runtime does not implement sessions yet. The Windows shell
  hides every entry point, and the runtime rejects the commands as unknown.

## Consequences

- An older version ignores `terminals` and drops it on its next save; workspaces
  and projects are unaffected.
- `session.rs::cwd_of` still resolves only workspaces and projects, so file
  commands never accept a session id; only `open_dock` reads a session's folder.
- The status bar names a session's process after the session.
- No persisted scrollback: a restart lists the session stopped, with no output.

## Evidence

- [Core rules](../../src-tauri/crates/core/src/terminal_sessions.rs) and
  [board compatibility](../../src-tauri/crates/core/src/board.rs) tests.
- [Desktop commands and shell lifecycle](../../src-tauri/src/terminal_sessions.rs)
  tests; `open_dock` routing in [`dock.rs`](../../src-tauri/src/dock.rs).
- [`fixtures/backend-contract.json`](../../fixtures/backend-contract.json) and
  the IPC parity test in [`src-tauri/tests/mock.rs`](../../src-tauri/tests/mock.rs).
- [IPC](../contracts/ipc.md#terminal-sessions),
  [persistence](../contracts/persistence.md#board) and the
  [Windows contract](../contracts/windows-application.md#files-git-and-terminals).
