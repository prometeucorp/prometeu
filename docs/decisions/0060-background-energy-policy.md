# ADR 0060 — Separate display sleep from agent work

Date: 2026-09-25
Status: Accepted

## Context

The original keep-awake switch started `caffeinate -d -i -s -w` whenever it
was always on or any local agent tab was working. The display assertion can
keep the screen lit for a long background turn. Some people need remote work
to continue while allowing their display to sleep, but the saved `agent`
choice has always promised that the display stays awake. Changing its meaning
would surprise existing installations. A turn remains active through native
background children under [ADR 0056](0056-background-tasks-hold-completion.md).

## Decision

Keep the saved `off`, `on` and `agent` values and their display behavior. Add
an explicit macOS `agent-system` value. It holds an idle-system assertion only
while a local tab is `rodando`, using `caffeinate -i -s -w <app pid>` without
`-d`. The existing modes use the display assertion. The command receives an
explicit `off`, `system` or `display` mode and changes its child only when the
requested mode changes. Normal exit stops the child; `-w` bounds unexpected
exit. Power-source changes never alter the person's selected policy.

Linux retains the prior options and its `systemd-inhibit` behavior until a
reliable distinction between system and display inhibition is verified across
supported desktops. The frontend does not offer `agent-system` there. Its
desktop compositor may still interpret the existing inhibition differently.

## Consequences

The preference remains local to the desktop webview and is not synchronized.
`agent-system` makes no claim about user-initiated sleep, lid closure or power
policy overrides; it only prevents idle system sleep while the app and agent
are active. The visible tab state, including background children, remains the
source of agent activity. Provider processes and sharing are unaffected by
the UI sleep selection.

Tests: `src/statusbar.test.ts` covers all saved-mode mappings;
`src-tauri/src/awake.rs` checks both macOS command lines, replacement,
idempotence and release. Native `pmset` confirmation on AC and battery is
tracked in [the energy profile](../quality/energy-profile.md).
