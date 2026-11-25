---
type: Decision
title: "ADR 0104 — Several computers follow the same remote machine"
description: "The host serves one attachment per computer at once; hooks, permission requests, terminal size and exited terminals are shared between them."
status: stable
---

# ADR 0104 — Several computers follow the same remote machine

Date: 2026-11-25

## Context

[ADR 0101](0101-remote-machines.md) let a new attachment replace the previous
one, so a desktop's stale SSH connection after sleep could not keep the
machine. It also meant that a person with two Macs could not follow the same
machine from both: connecting the second one marked the first **In use by another
computer**, and that one stopped reconnecting. Reconnecting it took the machine
back from the other.

The host was written for one client: terminal output, hooks, registry events and
permission requests went to a single attachment. The hook queue emptied when that
attachment collected it, the desktop that collected an exited terminal removed it,
and every desktop's window size was applied to the PTY.

## Options considered

1. **Several attachments to the host over SSH**, one per computer.
2. **One controlling attachment with read-only observers**, and an explicit
   takeover.
3. **Prometeu Cloud as it is**: a second Mac follows the first through the relay as
   a companion with remote control ([ADR 0030](0030-remote-control.md),
   [ADR 0036](0036-second-mac-as-companion.md)).
4. **The host joins the relay as a device**, with end-to-end encryption on the
   machine.
5. **The second computer uses the agent API** ([ADR 0103](0103-agents-on-remote-machines.md)).

## Decision

Option 1.

Option 2 needs the same fan-out and per-computer bookkeeping, and adds a
takeover step to send a message. Option 3 does not remove the exclusivity: only
the first Mac stays attached to the machine, and the companion sees nothing while
that Mac sleeps, which is what remote machines exist to avoid. Option 4 would
put keys on a server and change the shared-session invariant of
[ADR 0022](0022-end-to-end-encryption.md) for people who already reach the
machine over SSH. It remains the path for computers without SSH access. Option 5
is a second interface with no terminal or permission requests.

### Identity, authorization and revocation

`attach` passes the desktop's device ID and computer name
(`device.json`, shared with [ADR 0036](0036-second-mac-as-companion.md)).
The host serves at most eight attachments, one per device. A new
attachment from the same device replaces the earlier one, as before; one from
another device sits beside it. An attachment without a device, from an older
desktop, replaces other attachments without one.

The device is a label, not a credential. Every attachment arrives through the
person's own SSH login to the machine's user account, and that login can already
run anything as that user. SSH stays the identity, the authorization and the
encrypted transport; no relay or third party sees the machine's traffic.
Settings lists the connected computers and disconnects one, which receives the
existing `replaced` event and stops reconnecting until its person asks.
Removing the computer's key from `authorized_keys` revokes its access. An
allowlist of devices on the host would add no protection against someone who
already has a shell there, so it was not added.

### Shared state

- Terminal output and exit go to every attachment that follows the terminal.
  Each one replays its own gap from its last sequence.
- Registry events go to every attachment.
- Non-permission hooks are kept per terminal (the last 512) with the last one
  each device confirmed through the `hook_reply` desktops already send for every
  hook. A device receives the hooks it did not confirm, whether or not another
  device received them meanwhile. A hook queued to an attachment that was
  already dead, such as a Mac asleep behind a half-open connection, is not lost:
  it is sent again when that Mac comes back. The cost is that a hook handled just
  before a connection dropped, with its confirmation lost, can arrive twice. A
  device that terminal never served receives only hooks no device confirmed, as
  the single-desktop queue did.
- A permission request goes to every attachment that follows the conversation.
  The first decision wins, and the others receive `hook_resolved` and withdraw
  their card. An attachment that answers with nothing or goes away abstains.
  When all abstain, Claude's own dialog answers, as when no desktop is attached.
- The desktop that last typed into a terminal sets its size. Other computers'
  sizes are applied when they type, so two windows never fight over the PTY. A
  computer that is not setting the size sees the output laid out for the other
  one's size.
- An exited terminal stays until every attachment that follows it collected it,
  or left.

### Commands and conflicts

Input is serialized by the host, so messages never interleave, and nothing is
retried, as in ADR 0101. A message sent from one computer reaches the others as
input typed in the TUI. A conversation started on the machine between one
computer's terminal list and its open fails with `err.remote.startedElsewhere`;
trying again adopts the running process instead of risking the launch's input.
Stop, closing a tab and cleaning a workspace still end the remote process for
every computer, which then see it as ended, as they would after an agent ended
it.

### Disconnection

A computer that stops reading has its outgoing frames dropped after 16 MiB, and
the host closes its attachment. It replays what it missed when it comes back.
Transcript streaming waits for the writer instead of reaching that limit.

### Mixed versions

The host's `ready` frame reports `clients` and the app `release` that started
it. A desktop replaces an idle host from another build only when that host was
not started by a newer app release. Without this, two computers on different
releases would retire each other's host each time it went idle. An older host
still serves one attachment at a time until it is idle and replaced. Connected
computers are listed only for hosts that report `clients`.

## Consequences

- Two or more computers follow the same remote workspaces at once, and each keeps
  following while the others sleep.
- A computer that types in a conversation resizes it for the others.
- Revoking a computer is an SSH key change on the machine; the app only
  disconnects it.
- Closing a tab or stopping a conversation on one computer ends it for all of
  them. Asking first when another computer follows the conversation remains
  open.

## Evidence

- [Remote host protocol](../contracts/remote-host.md).
- `src-tauri/crates/host/tests/host.rs`:
  `desktops_on_different_devices_stay_attached_and_see_the_same_output`,
  `every_device_receives_each_hook_once_including_those_it_missed`,
  `the_first_decision_wins_and_the_other_desktops_withdraw_theirs`,
  `an_exited_terminal_stays_until_every_follower_collected_it`,
  `the_desktop_that_types_drives_the_terminal_size`,
  `a_desktop_that_stops_reading_is_dropped_instead_of_buffered`,
  `the_host_reports_the_release_of_the_app_that_started_it` and the earlier
  `a_new_attachment_replaces_the_previous_one` for desktops without a device.
- `src-tauri/src/remote/connection.rs`: `the_attach_command_names_this_computer_quoted`,
  `only_a_newer_host_is_kept_over_this_apps_build`.
- `src/machine-clients.test.ts`: ordering and confirmation of disconnecting.
