---
type: Decision
title: "ADR 0101 — Workspaces on remote machines over SSH"
description: "A resident host on the remote machine owns terminal agents; the desktop keeps the board, conversations and interface."
status: stable
---

# ADR 0101 — Workspaces on remote machines over SSH

Date: 2026-11-22

## Context

People want some workspaces to run on another computer they own, such as a home
server or a VPS: more CPU and memory than a laptop, and agents that keep working
while the laptop sleeps or the app is closed. The rest of the board stays local,
and those workspaces should appear in it next to local ones.

The WSL runtime ([ADR 0084](0084-shared-windows-desktop.md)) already runs a
Unix execution host behind a pipe, but it cuts at the top: every desktop command
needs a second implementation inside the runtime, so coverage always lags the
desktop. Orca, cmux and Claude Desktop's SSH sessions all cut lower: a small,
versioned helper on the remote host owns processes and buffers their output,
and the laptop stays the interface.

## Options considered

1. Run the whole application on the machine, as the WSL runtime does, and attach
   the interface to it.
2. Mount the machine's files locally (sshfs) and run only processes remotely.
3. Keep the desktop as the owner of the board, conversations and interface, and
   move only the terminal agent's process, its hooks and its native transcript
   to a resident host on the machine.

## Decision

Option 3.

### The host

`prometeu-host` (`src-tauri/crates/host/`) is one statically linked Linux
executable without Tauri. It reuses `prometeu-process` for PTYs and runs as a
resident daemon on a private Unix socket under `~/.prometeu-remote` (or
`~/.prometeu-remote-dev` for debug builds, matching their separate boards). One
host serves one root; a second one exits when it cannot take the root's lock.

- `attach` is what the desktop runs over SSH. It starts the host in its own
  session when the socket does not answer, then relays stdio to the socket. Losing
  SSH only detaches the client: the host keeps every terminal.
- A new attachment replaces the previous one. After sleep the old SSH connection
  can look alive for many seconds, and the desktop the person is using now must
  win. A replaced desktop stops reconnecting until the person asks it to.
- Each terminal keeps its last 2 MiB of output in sequence-numbered chunks.
  Reattachment replays exactly what the desktop has not applied yet, or the whole
  retained buffer after a screen reset when part of the gap was dropped.
- Hooks queue on the host while no desktop is attached (512 per terminal) and are
  delivered when the desktop reattaches that terminal. A permission request with
  no desktop attached answers with no decision, so Claude's native dialog stays
  in its TUI.
- Transcript directories are streamed to the desktop from the offsets it already
  has.

The host adopts the login shell's `PATH`, so agent CLIs installed under the
person's home are found.

### Resource limits

When systemd linger is enabled, each terminal starts in a transient scope inside
a slice per workspace (`prometeu-w<uuid>.slice` under `prometeu.slice`). The
machine's per-workspace memory and CPU limits apply to that slice, so they cover
the agent and everything it starts. Without linger, the person's systemd manager
would stop the scopes at logout, so terminals run without limits and Settings says
why. Installation tries `loginctl enable-linger`, which many distributions allow
without root.

### The desktop

- Machines live in `<root>/machines.json`: a name, an SSH destination passed as
  one argument after `--`, and limits. The system `ssh` authenticates with the
  person's keys and agent, so the app stores no credentials. Connection sharing
  is disabled for these connections, because a master connection left stale by
  sleep could hang them.
- `Workspace.host` and `Project.host` name the machine. Their paths are paths on
  that machine. Remote project IDs are `remote:<machine>:<path>`, so they never
  collide with local folders.
- A repository is one project wherever it runs. Projects record their normalized
  `origin`, and a machine's clone with the same origin as a local project appears
  as that project, with its workspaces marked by the machine. Choosing a local
  project for a machine uses its clone there, or clones its `origin` under
  `~/code` while the workspace prepares. Matching by origin rather than by name
  keeps two different repositories with the same folder name apart.
- Repositories that exist only on the machine are added from the launcher by
  browsing its folders and ticking them, several at once, or by cloning a URL
  into the folder being browsed.
- Workspace creation runs Git on the machine and puts new worktrees under the
  host root. The first conversation always uses terminal mode. Branch listing for
  remote projects also runs on the machine.
- A remote conversation goes through the same terminal launch as a local one
  (`chat::terminal::launch_on`), with an `Execution` that opens the terminal on the
  host, writes Claude's hook settings there, and points observation at a private
  local mirror of the machine's Claude project directory. Hooks reach the
  conversation's local bridge through `terminal_agent_hooks::relay`. That relay
  substitutes the bridge's own socket, credential and process nonce, so a process
  that outlives an app restart still reports to the new bridge.
- The connection reconnects with backoff (1–30 s) and reattaches every terminal
  and mirror. After each attachment the desktop resumes, without being asked,
  the conversations whose processes the host still runs. Their status,
  transcripts and notifications therefore update when the laptop wakes or the app
  starts.
- Bytes replayed after reattachment never answer terminal queries. Those queries
  were answered when first produced, or the program stopped waiting for them.
- Claude signs in on the machine, as in Orca. Settings runs `claude auth login
  --claudeai` in one of the host's terminals, opens the authorization link in
  this computer's browser, and writes the code Claude shows back to that
  terminal. The credential is created and kept on the machine; this computer's
  Claude account never leaves it. Sending the local credential (Claude Desktop's
  approach) was rejected, because anyone using the same Unix account on the
  machine could read it. Each attachment refreshes `claude auth status`, so the
  machine row shows whether its workspaces can start.
- Quitting the app does not warn about remote work, and dropping a remote waiter
  leaves its process running. Closing a tab, Stop and removing or cleaning a
  workspace still end the remote process.

### Scope

Only Claude declares the `remoteMachines` capability. Its hooks and transcript
layout are the ones relayed and mirrored. The host ships for Linux x86_64 only.
Remote workspaces show conversations only: the file tree, Git panel, diff
review, Setup/Run, docks, MCP and plugin selection, kickoff skills, delegation
and multi-repository workspaces stay local until each gets a host
implementation. Cleanup removes the remote worktree and branch through Git on the
machine.

## Consequences

- Execution survives the desktop's sleep, network changes and restarts. It does
  not survive a reboot of the machine or the host's death: the transcript then
  resumes in a new process, as for a local terminal agent after an app restart.
- The interface needs no second implementation. Missing remote features are
  hidden instead of half-working, and each can move to the host later without
  changing the board format.
- No input is retried after a lost connection. A keystroke sent while the SSH
  process was already dead has an unknown outcome, like every other lost reply.
- The desktop must be running for remote work to start. An external client on the
  machine (for example a bot asking for workspaces) needs a separate decision.

## Evidence

- [Remote host protocol](../contracts/remote-host.md).
- `src-tauri/crates/host/tests/host.rs`: detachment and gap replay, queued hooks
  and decisions, transcript streaming, attachment replacement.
- `src-tauri/src/remote/terminal.rs`, `connection.rs`, `agent.rs` and
  `workspace.rs` unit tests: sequence ordering, lost terminals, mirror paths and
  appends, transcript path rewriting, project identity.
- `remote::journey::vps_journey` (opt-in, against a real machine): install,
  terminal output across a dropped SSH connection with no gap or duplicate, and
  the workspace slice's applied `memory.max` and `cpu.max`.
- `remote_hosts_round_trip_and_local_entries_omit_them` in
  `crates/core/src/board.rs`: boards without remote entries save unchanged.
