---
type: Decision
title: "ADR 0097 — Ask before ending the app while local work runs"
description: "Quitting, closing the main window or restarting for an update asks first while local work would be cut off."
status: stable
---

# ADR 0097 — Ask before ending the app while local work runs

Date: 2026-10-27

Builds on the interrupted turns of
[the conversation flow](../architecture/conversation-flow.md), the window policy
of [ADR 0089](0089-notch-island.md) and the resident Windows runtime of
[ADR 0082](0082-resident-wsl-attachments.md).

## Context

On macOS and Linux the app owns every agent, shell and script it starts, so
quitting, closing the main window (which quits, per ADR 0089) or restarting for
an update ends all of them without a word. A conversation comes back marked as
interrupted, with **Continue**, but what was in flight is gone: the partial
reply, open requests, background tasks and the work the agent must redo. A
command running in a terminal, such as a development server in Run, and a
workspace still being prepared are lost with no trace. The update offered the
only warning, in the tooltip of its restart button.

Three platform facts shape the options:

- The system's Quit item sends `terminate:`. The tao version in use implements
  only `applicationWillTerminate`, so the app receives `RunEvent::Exit` without
  an `ExitRequested` it could prevent. The Dock's Quit, ⌘Tab followed by Q,
  AppleScript, logout and shutdown take the same path.
- Closing a window raises `CloseRequested`, which Tauri holds while the webview
  listens for it.
- The update is installed on disk before **Restart to update** appears, and it
  takes effect at the next start anyway.

## Options considered

1. **Keep the tooltip.** Nothing changes; every way out still cuts work off.
2. **Hold `terminate:` natively.** Add `applicationShouldTerminate:` to tao's
   application delegate at runtime. It would also cover the Dock and
   AppleScript, but it patches a class the windowing library owns, from outside
   any crate boundary, and logout or shutdown still could not wait for an
   answer from the webview.
3. **Ask from the interface.** The Mac's Quit becomes an app menu item, a close
   listener holds the main window, and the restart button asks before
   relaunching.
4. **Restart when idle.** Let the update wait until conversations settle and
   restart by itself. It needs a waiting phase, a way to cancel it and a rule
   for requests that wait on the person, while the installed version already
   applies at the next start.

## Decision

Option 3. Option 4 stays open as a later addition to the same question.

- **What counts.** Only work on this computer:
  - tabs `rodando` or `querendo` in local workspaces, the set `Board::revive`
    marks interrupted after a restart; background tasks keep a tab `rodando`
    ([ADR 0056](0056-background-tasks-hold-completion.md)), and other people's
    shares run on their owners' computers;
  - workspaces whose preparation is still running, which would fail;
  - terminals counted by `running_terminals`: Setup and Run while their script
    lives, and a shell or terminal session while it runs a command. A command
    runs while the PTY's foreground process group is not the shell's own, or
    while the shell's own process stopped being a shell: `exec` keeps the
    process and its group. A process counts as a shell when /etc/shells lists
    its name or the terminal started it. A shell waiting at its prompt loses
    only its scrollback.
- **When.** ⌘Q and the menu's Quit on macOS; closing the main window, with its
  button or ⇧⌘W, on macOS and Linux; **Restart to update** wherever the updater
  runs. With nothing to cut off, the app ends at once, as before.
- **How.** One confirmation names the counts and stops on **Cancel**; its
  primary action quits or restarts now. The restart question adds that the
  installed version also applies at the next start. The window is restored and
  focused first, so a minimized window cannot hide the question. One question
  is open at a time: a request made meanwhile is refused and the open question
  decides. If asking fails, the app ends, as it did before.
- **Windows.** The resident runtime keeps every agent and terminal after the
  window closes. `src/windows/main.ts` switches the question off, and the
  runtime does not implement `running_terminals`.

## Consequences

- The Dock's Quit, ⌘Tab followed by Q, AppleScript `quit`, logout and shutdown
  still end the app without asking on macOS.
- Linux has no native Quit item to replace; closing the window and the AppImage
  restart ask, and no Ctrl+Q is added.
- ⌘Q and closing the main window now depend on the webview answering. A
  question that fails lets the app end; a webview that hangs leaves the Dock's
  Quit and Force Quit.
- A job sent to the background with `&` does not count; a full-screen program
  such as an editor, `ssh` or an agent CLI does, and so does a command started
  with `exec`. `exec` into another shell, such as `exec zsh`, still waits at a
  prompt; an interactive shell started as a job counts as a command.
- The main window's capability gains `core:window:allow-destroy`, which the
  close listener uses once the close is allowed, `allow-unminimize`,
  `allow-set-focus` and `process:allow-exit`.

## Evidence

- `src/quit.test.ts`: what counts, the joined counts, the answer deciding, one
  question at a time, failing open and Windows never asking.
- `src/update.test.ts` and `src/update-init.test.ts`: restarting waits for the
  answer and a refusal keeps the update ready.
- `src/appmenu.test.ts`: the Mac quits through the app's own item.
- `crates/process/src/terminal/tests.rs`: a command running as a job or in
  place of the shell is told apart from a shell waiting for input, on macOS and
  on Linux, where `/bin/sh` is dash.
- `src-tauri/src/dock.rs`: which terminals quitting cuts off.
- Native menu events, `CloseRequested` and relaunch cannot be built in a unit
  test; they need a macOS app run.
