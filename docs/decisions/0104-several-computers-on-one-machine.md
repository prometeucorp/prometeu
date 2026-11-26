---
type: Decision
title: "ADR 0104 — Several computers follow the same remote machine"
description: "The host serves one attachment for each computer at the same time. The computers share hooks, permission requests, the terminal dimensions and the terminals that stopped."
status: stable
---

# ADR 0104 — Several computers follow the same remote machine

Date: 2026-11-25

## Context

In [ADR 0101](0101-remote-machines.md), a new attachment replaced the previous
attachment. Thus a stale SSH connection of a desktop after sleep did not keep
the machine. But a person with two Macs did not have a procedure to follow the
same machine from the two Macs. When the second Mac connected, the first Mac showed **In use by another
computer**, and it stopped the reconnection. When the person connected the
first Mac again, it took the machine back from the second Mac.

The host had a design for one client. Terminal output, hooks, registry events
and permission requests went to a single attachment. The hook queue became
empty when that attachment collected it. The desktop that collected a terminal
that stopped removed that terminal. The host applied the window dimensions of
each desktop to the PTY.

## Options considered

1. **Several attachments to the host through SSH**, one for each computer.
2. **One attachment that controls, with read-only observers**, and an explicit
   step to take the control.
3. **Prometeu Cloud as it is**: a second Mac follows the first Mac as a
   companion with remote control ([ADR 0030](0030-remote-control.md),
   [ADR 0036](0036-second-mac-as-companion.md)). It uses the relay.
4. **The host connects to the relay as a device**, with end-to-end encryption on
   the machine.
5. **The second computer uses the agent API** ([ADR 0103](0103-agents-on-remote-machines.md)).

## Decision

Use option 1.

- Option 2 needs the same fan-out and the same records for each computer. It
  also adds a step to take the control before the person can send a message.
- Option 3 does not remove the exclusive access. Only the first Mac stays
  attached to the machine. The companion sees nothing while that Mac sleeps.
  Remote machines are there to prevent this problem.
- Option 4 puts keys on a server. It changes the shared-session invariant of
  [ADR 0022](0022-end-to-end-encryption.md) for persons who get access to the
  machine through SSH at this time. It stays the path for computers without
  SSH access.
- Option 5 is a second interface without a terminal or permission requests.

### Identity, authorization and revocation

`attach` sends the device ID and the computer name of the desktop
(`device.json`, shared with [ADR 0036](0036-second-mac-as-companion.md)).
The host serves a maximum of eight attachments, one for each device. A new
attachment from the same device replaces the earlier attachment, as before. An
attachment from a different device operates adjacent to it. An attachment
without a device, from a desktop of an earlier version, replaces the other
attachments without a device.

The device is a label, not a credential. Each attachment comes through the SSH
login of the person to the user account of the machine. That login can run all
commands as that user at this time. SSH stays the identity, the authorization
and the encrypted transport. No relay or third party sees the traffic of the
machine. Settings lists the connected computers and can disconnect one. That
computer receives the current `replaced` event, and it does not connect again
until its person tells it to. When the person removes the key of the computer
from `authorized_keys`, the computer has no access. A list of permitted devices
on the host gives no protection against a person who has a shell there. Thus
the team did not add it.

### Shared state

- Terminal output and exit go to each attachment that follows the terminal.
  Each attachment replays its own gap from its last sequence.
- Registry events go to each attachment.
- The host keeps the hooks that are not permission hooks for each terminal (the
  last 512). It also keeps the hooks that each device confirmed through the
  `hook_reply` that desktops send for each hook.
  - Desktops process hooks on separate threads. Thus each reply confirms only
    its own hook.
  - A device receives the hooks that it did not confirm, also when a different
    device received them. This includes the first hook after the device started
    to follow a conversation with an empty log.
  - The host does not lose a hook that it sent to an attachment that had
    stopped. An example is a Mac that sleeps behind a half-open connection. The
    host sends the hook again when that Mac comes back.
  - As a result, a hook can come two times. This occurs when a desktop
    processed the hook immediately before a connection stopped, and the
    confirmation was lost.
  - A device that the terminal did not serve receives only the hooks that no
    device confirmed, as the queue for a single desktop did.
- A permission request goes to each attachment that follows the conversation.
  The first decision applies. The other attachments receive `hook_resolved`
  and remove their card. An attachment that answers with nothing or that goes
  away does not vote. When no attachment votes, the dialog of Claude answers, as
  when no desktop is attached.
- The desktop that typed last into a terminal sets its dimensions. The host
  applies the dimensions of other computers when they type. Thus two windows do
  not change the PTY at the same time. A computer that does not set the
  dimensions sees the output in the layout for the dimensions of the other
  computer.
- A terminal that stopped stays until each attachment that follows it collects
  it or goes away.

### Commands and conflicts

The host puts input in sequence. Thus messages do not mix, and the host does
not try again, as in ADR 0101. A message from one computer goes to the other
computers as input typed in the TUI. Sometimes a conversation starts on the
machine after a computer gets its terminal list and before it opens the
conversation. Then the open fails with `err.remote.startedElsewhere`. When the
person tries again, the computer adopts the process that runs. Thus it does
not risk the input of the launch. The host reserves a terminal ID before it
starts its process. Thus, when two computers open the same conversation at the
same time, only one Claude starts. The other computer gets that error.
Stop, the close of a tab and the cleanup of a workspace continue to stop the
remote process for all computers. Then all computers see it as stopped, as
when an agent stops it.

### Disconnection

When a computer stops the read of its frames, the host drops its outgoing
frames after 16 MiB and closes its attachment. The computer replays the data
that it did not get when it comes back. The transcript streaming waits for the
writer, and it does not get to that limit.

### Mixed versions

The `ready` frame of the host reports `clients` and the app `release` that
started it. A desktop replaces an idle host from a different build only when a
newer app release did not start that host. Without this rule, two computers on
different releases replace the host of the other computer each time the host
becomes idle. A host from an earlier version continues to serve only one
attachment at a time until it is idle and the desktop replaces it. Settings
lists the connected computers only for hosts that report `clients`.

## Consequences

- Two or more computers follow the same remote workspaces at the same time.
  Each computer continues to follow them while the other computers sleep.
- A computer that types in a conversation changes its dimensions for the other
  computers.
- To revoke the access of a computer, change the SSH keys on the machine. The
  app only disconnects the computer.
- When the person closes a tab or stops a conversation on one computer, it
  stops for all computers. A question before this action, when a different
  computer follows the conversation, is not decided at this time.

## Evidence

- [Remote host protocol](../contracts/remote-host.md).
- `src-tauri/crates/host/tests/host.rs`:
  `desktops_on_different_devices_stay_attached_and_see_the_same_output`,
  `every_device_receives_each_hook_once_including_those_it_missed`,
  `the_first_decision_wins_and_the_other_desktops_withdraw_theirs`,
  `an_exited_terminal_stays_until_every_follower_collected_it`,
  `the_desktop_that_types_drives_the_terminal_size`,
  `a_desktop_that_stops_reading_is_dropped_instead_of_buffered`,
  `the_host_reports_the_release_of_the_app_that_started_it`,
  `a_device_attached_before_any_hook_still_receives_the_first_one_it_missed`,
  `a_later_confirmation_does_not_count_for_an_earlier_hook`,
  `opening_the_same_conversation_from_two_desktops_starts_one_process`, and the
  earlier `a_new_attachment_replaces_the_previous_one` for desktops without a
  device.
- `src-tauri/src/remote/connection.rs`: `the_attach_command_names_this_computer_quoted`,
  `only_a_newer_host_is_kept_over_this_apps_build`.
- `src/machine-clients.test.ts`: the order and the confirmation of a
  disconnection.
