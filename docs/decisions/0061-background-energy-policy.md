---
type: Decision
title: "ADR 0061 — Separate display sleep from agent work"
description: "An explicit macOS option that keeps only the system on while an agent operates."
status: stable
---

# ADR 0061 — Separate display sleep from agent work

Date: 2026-09-25

## Context

The initial keep-awake switch started `caffeinate -d -i -s -w` in two
conditions: when the switch was always on, or when a local agent tab operated.
The display assertion can keep the screen on during a long background turn.
Some persons want remote work to continue while the display goes to sleep. But
the saved `agent` selection always promised that the display stays on. If
Prometeu changes its meaning, the behavior of current installations changes
without a warning. A turn stays active while native background children operate,
as [ADR 0056](0056-background-tasks-hold-completion.md) specifies.

## Decision

Keep the saved `off`, `on` and `agent` values and their display behavior. Add
an explicit macOS value, `agent-system`. This value keeps an idle-system
assertion only while a local tab is `rodando`. It uses
`caffeinate -i -s -w <app pid>` without `-d`. The current modes use the display
assertion.

The command receives an explicit `off`, `system` or `display` mode. It changes
its child process only when the requested mode changes. A usual exit stops the
child. If the app stops unexpectedly, `-w` stops the child. A change of the
power source does not change the policy that the person selected.

Linux keeps the previous options and its `systemd-inhibit` behavior. This
continues until Prometeu can make sure that the system and the display
inhibition operate differently on the desktops that it supports. On Linux, the
frontend does not show `agent-system`. The desktop compositor can also apply
the current inhibition differently.

## Consequences

The setting stays local to the desktop webview. Prometeu does not send it to
other devices. `agent-system` does not change sleep that the user starts, the closed lid or
other power policies. It only prevents system sleep when there is no input, while the app and the
agent are active. The tab state on the screen, with its background children,
stays the source of the agent activity. The sleep setting in the UI does not
change provider processes or shared sessions.

Tests: `src/statusbar.test.ts` examines how each saved mode changes into a command mode.
`src-tauri/src/awake.rs` examines the two macOS command lines, the replacement,
a second request with the same mode and the release. [The energy
profile](../quality/energy-profile.md) records the native `pmset` confirmation
on AC power and on battery.
