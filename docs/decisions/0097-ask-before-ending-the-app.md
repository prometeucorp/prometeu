---
type: Decision
title: "ADR 0097 — Ask before ending the app while local work runs"
description: "If local work will stop, the app shows a question before a Quit, a close of the main window or an update."
status: stable
---

# ADR 0097 — Ask before ending the app while local work runs

Date: 2026-10-27

This ADR adds to the interrupted turns of
[the conversation flow](../architecture/conversation-flow.md). It also adds to
the window policy of [ADR 0089](0089-notch-island.md).

## Context

On macOS and Linux, the app owns each agent, shell and script that it starts.
Three actions stop all of them without a message:

- Quit.
- A close of the main window. Per ADR 0089, this action stops the app.
- A new start of the app for an update.

A conversation comes back with the interrupted status and with **Continue**.
But the work that operated at that time is not there. This work includes the
reply that was not complete and the open requests. It also includes the
background tasks and the work that the agent must do again. A command that
operates in a terminal, for example a development server in Run, also stops
without a record. A workspace that Prometeu prepares at that time also stops.
Before this decision, the only warning was in the tooltip of the button that
starts the app again for the update.

Three facts about the platforms have an effect on the options:

- The Quit item of the system sends `terminate:`. The tao version that
  Prometeu uses has only `applicationWillTerminate`. Thus the app receives
  `RunEvent::Exit` without an `ExitRequested` that the app can stop. The Quit
  item of the Dock, ⌘Tab and then Q, AppleScript, logout and shutdown use the
  same path.
- When a window closes, it sends `CloseRequested`. Tauri holds this event while
  the webview listens for it.
- Prometeu installs the update on the disk before **Restart to update** shows.
  The update starts to operate at the next start in all conditions.

## Options considered

1. **Keep the tooltip.** The behavior does not change. All exits continue to
   stop work.
2. **Hold `terminate:` natively.** Add `applicationShouldTerminate:` to the
   application delegate of tao at runtime. This option also includes the Dock
   and AppleScript. But it changes a class that the windowing library owns,
   from a location that is not in the crate. Also, logout and shutdown cannot
   wait for an answer from the webview.
3. **Ask from the interface.** The Quit item of the Mac becomes an item of the
   app menu. A close listener holds the main window. The update button shows a
   question before it starts the app again.
4. **Restart when idle.** The update waits until the conversations stop, and
   then it starts the app again. This option must have a wait phase and a
   procedure to cancel it. It must also have a rule for requests that wait for
   the person. But the installed version operates at the next start in all
   conditions.

## Decision

Use option 3. Option 4 can be a subsequent step for the same question.

- **What counts.** Only work on this computer:
  - Tabs with the status `rodando` or `querendo` in local workspaces. After a
    new start, `Board::revive` sets these tabs to interrupted. Background tasks
    keep a tab in `rodando` ([ADR 0056](0056-background-tasks-hold-completion.md)).
    The shares of other persons operate on the computers of their owners.
  - Workspaces with a preparation that continues to operate. That preparation
    does not complete if the app stops.
  - Terminals that `running_terminals` counts:
    - Setup and Run while their script operates.
    - A shell or a terminal session while it runs a command.

    A command runs while the foreground process group of the PTY is not the
    group of the shell. A command also runs while the process of the shell is
    not a shell at that time, because `exec` keeps the process and its group. A
    process is a shell when /etc/shells lists its name or when the terminal
    started it. When the app stops, a shell that waits at its prompt has a
    decrease of its scrollback only.
- **When.** The app shows the question in these conditions:
  - On macOS, for ⌘Q and for the Quit item of the menu.
  - On macOS and Linux, when the person closes the main window with its button
    or with ⇧⌘W.
  - For **Restart to update**, on all platforms where the updater operates.

  When no work will stop, the app stops immediately, as before.
- **How.** One confirmation shows the counts. It stops on **Cancel**. Its
  primary action stops the app, or starts it again, immediately. For an update,
  the question also tells that the installed version operates at the next
  start. Prometeu first shows the window and puts the focus on it. Thus a
  window at its minimum dimension cannot hide the question. Prometeu shows only one
  question at a time. It rejects a request that comes during the question, and
  the answer to the open question applies. If Prometeu cannot show the
  question, the app stops, as before.

## Consequences

- On macOS, some actions continue to stop the app without a question. These
  actions are the Quit item of the Dock, ⌘Tab and then Q, AppleScript `quit`,
  logout and shutdown.
- Linux has no native Quit item to replace. The app shows the question when the
  person closes the window or starts the AppImage again for an update.
  Prometeu does not add Ctrl+Q.
- ⌘Q and the close of the main window operate only when the webview
  answers. If Prometeu cannot show the question, the app stops. If the webview
  does not answer, the person can use the Quit item of the Dock and Force Quit.
- A job that `&` sends to the background does not count. A full-screen
  program, for example an editor, `ssh` or an agent CLI, counts. A command that
  starts with `exec` also counts. After `exec` into a different shell, for
  example `exec zsh`, the shell continues to wait at a prompt. An interactive
  shell that starts as a job counts as a command.
- The capability of the main window gets these permissions:
  - `core:window:allow-destroy`. The close listener uses it after the person
    accepts the close.
  - `allow-unminimize`.
  - `allow-set-focus`.
  - `process:allow-exit`.

## Evidence

- `src/quit.test.ts`: what counts, the counts together, the answer that
  applies, one question at a time, and the app stops when the question has a
  failure.
- `src/update.test.ts` and `src/update-init.test.ts`: the new start waits for
  the answer, and a refusal keeps the update prepared.
- `src/appmenu.test.ts`: the Mac stops the app through the item of the app.
- `crates/process/src/terminal/tests.rs`: the tests find the difference
  between two conditions. In the first, a command operates as a job or as a
  replacement of the shell. In the second, a shell waits for input. The tests
  operate on macOS and on Linux, where `/bin/sh` is dash.
- `src-tauri/src/dock.rs`: the terminals that stop when the app stops.
- A unit test cannot make native menu events, `CloseRequested` and the new
  start of the app. Only a run of the macOS app can examine them.
