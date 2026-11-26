---
type: Decision
title: "ADR 0101 — Workspaces on remote machines over SSH"
description: "A resident host on the remote machine owns terminal agents. The desktop keeps the board, the conversations and the interface."
status: stable
---

# ADR 0101 — Workspaces on remote machines over SSH

Date: 2026-11-22

## Context

Persons want to run some workspaces on a different computer that they own, for
example a home server or a VPS. That computer has more CPU and memory than a
laptop. Its agents continue to operate while the laptop sleeps or after the person
closes the app. The other parts of the board stay local, and those workspaces must
show in the board adjacent to the local workspaces.

One procedure is to run all of the application on the machine and attach the
interface to it. This procedure divides the system at the top. Each desktop
command must have a second implementation on the machine. Thus the coverage on
the machine is always less than on the desktop. The SSH sessions of Orca, cmux
and Claude Desktop divide the system at a lower level. A small helper with a
version on the remote host owns the processes and keeps their output in a
buffer. The laptop stays the interface.

## Options considered

1. Run all of the application on the machine and attach the interface to it.
2. Mount the files of the machine locally (sshfs) and run only the processes
   remotely.
3. Keep the desktop as the owner of the board, the conversations and the
   interface. Move only the process of the terminal agent, its hooks and its
   native transcript to a resident host.

## Decision

Use option 3.

### The host

`prometeu-host` (`src-tauri/crates/host/`) is one statically linked Linux
executable without Tauri. It uses `prometeu-process` again for PTYs. It runs as
a resident daemon on a private Unix socket in `~/.prometeu-remote`. For debug
builds, the socket is in `~/.prometeu-remote-dev`, because these builds have a
separate board. One host operates for one root. If a second host cannot get the lock
of the root, it exits.

- The desktop runs `attach` through SSH. When the socket does not answer,
  this command starts the host in its own session. Then it relays stdio to the
  socket. When SSH disconnects, only the client detaches. The host keeps all
  terminals.
- A new attachment from the same computer replaces the previous attachment.
  After sleep, the old SSH connection can look alive for many seconds. The
  desktop that the person uses at this time must have priority. A replaced
  desktop does not connect again until the person tells it to. Attachments from
  different computers can operate at the same time
  ([ADR 0104](0104-several-computers-on-one-machine.md)).
- Each terminal keeps the last 2 MiB of its output in chunks with sequence
  numbers. A reattachment replays only the output that the desktop did not
  apply. If the host dropped a part of the gap, the reattachment does a screen
  reset and replays all of the buffer that the host kept.
- The host keeps hooks (512 for each terminal). When a computer attaches that
  terminal again, the host sends the hooks that the computer did not receive. If
  no desktop is attached, a permission request answers with no decision. Thus
  the native dialog of Claude stays in its TUI.
- The host streams transcript folders to the desktop from the offsets that the
  desktop has.

The host uses the `PATH` of the login shell. Thus it finds agent CLIs that the
person installed in the home folder.

### Resource limits

When systemd linger is enabled, each terminal starts in a transient scope in a
slice for each workspace (`prometeu-w<uuid>.slice` in `prometeu.slice`). The
memory and CPU limits of the machine for each workspace apply to that slice.
Thus the limits include the agent and all processes that it starts. Without
linger, the systemd manager of the person stops the scopes at logout. Thus
terminals run without limits, and Settings tells the cause. The installation
tries `loginctl enable-linger`. Many distributions let a person run this
command without root.

### The desktop

- **Machines.** The machines are in `<root>/machines.json`. Each machine has a
  name, an SSH destination and limits. Prometeu sends the destination as one
  argument after `--`. The system `ssh` authenticates with the keys and the
  agent of the person. Thus the app does not store credentials. Connection
  sharing is disabled for these connections. A master connection that sleep
  made stale can stop them.
- **Host fields.** `Workspace.host` and `Project.host` name the machine. Their
  paths are paths on that machine. Remote project IDs are
  `remote:<machine>:<path>`. Thus they are not the same as local folders at any
  time.
- **Projects.** A repository is one project at all locations where it runs.
  Projects record their normalized `origin`. A clone on a machine with the same
  origin as a local project shows as that project. Its workspaces have the mark
  of the machine. The selection of a project is different from the selection of
  its execution location. The launcher lists only registered projects whose
  `host` agrees with the selected machine. Prometeu does not show a local
  repository with an `origin` as an implicit remote clone. The origin continues
  to put sidebar workspaces in a group, and it does not hide remote
  registrations.
- **Projects page.** Settings has a Projects page with a filter for each
  computer. It lists each registered folder on that computer. This includes
  remote clones that have a local equivalent. The person adds current remote
  repositories through a folder dialog. A clone is an explicit action with a URL
  and a destination on the machine. The launcher does not show installation
  shortcuts. The person must prepare a machine there first. Thus the
  installation is different from the creation of a workspace. The selected
  computer controls which projects Prometeu shows. The current backend
  resolution of the origin stays for compatibility with callers from earlier
  versions.
- **Workspace creation.** The creation of a workspace runs Git on the machine
  and puts new worktrees in the host root. The first conversation always uses
  the terminal mode. The branch list for remote projects also comes from the
  machine.
- **Conversations.** A remote conversation uses the same terminal launch as a
  local conversation (`chat::terminal::launch_on`). Its `Execution` opens the
  terminal on the host and writes the hook settings of Claude there. It points
  the observation to a private local mirror of the Claude project folder of the
  machine. Hooks go to the local bridge of the conversation through
  `terminal_agent_hooks::relay`. That relay puts in the socket, the credential
  and the process nonce of the bridge. Thus a process that continues after a new
  start of the app sends reports to the new bridge.
- **Reconnection.** The connection connects again with backoff (1–30 s). It
  attaches all terminals and mirrors again. After each attachment, the desktop
  resumes the conversations whose processes continue to operate on the host. It
  does this without a request from the person. It also adopts the conversations
  that completed while it was away, and it does not start Claude again. Thus
  their status, transcripts and notifications update when the laptop wakes or
  the app starts. After the desktop collects the output, the hooks and the
  transcript of a completed terminal, the host forgets that terminal. Thus the
  host does not keep its buffer.
- **Adoption before the first hook.** Sometimes the desktop adopts a process
  before its first hook comes. For example, Claude waits at its folder-trust
  question while the laptop sleeps. This process does not have a transcript
  binding at that time. Prometeu sees it as a new start. Thus its queued
  `SessionStart` binds it, and Prometeu continues to send its first prompt.
  Before the adoption, the desktop flushes the transcript mirror. Thus Prometeu
  reads the turns that the process wrote while the desktop was away from the
  point where the desktop stopped.
- **Binding before the transcript.** A first hook can also bind a path before
  Claude makes its transcript. After the desktop flushes the mirror, the
  adoption can see that process as one that waits for its first prompt. This
  occurs only when these conditions are true:
  - The bound file is not there.
  - The journal and the canonical log prove that Prometeu captured no
    conversation work.

  The adoption keeps the native identity and the journal. A replacement process
  or captured work continues to need the usual verified history bootstrap. A
  missing file alone does not prove an empty session at any time.
- **Profiles.** The profile of a remote conversation names the machine
  (`remote:<machine>`). It does not agree with the selected account of this
  computer at any time. Thus a change of the local account does not start
  remote conversations again.
- **First output.** A terminal that this desktop opened recently sends its
  first output live, also when the output comes through the replay of the
  attachment. The program can wait for answers to its startup queries at that
  time.
- **Replayed bytes.** Bytes that the host replays after a reattachment do not
  answer terminal queries at any time. The desktop answered those queries when
  the program first sent them, or the program stopped the wait for them.
- **Claude login.** Claude signs in on the machine, as in Orca. Settings runs `claude auth login
  --claudeai` in one of the terminals of the host. It opens
  the authorization link in the browser of this computer. Then it writes the
  code that Claude shows back to that terminal. Claude makes the credential and
  keeps it on the machine. The Claude account of this computer does not go out
  of it at any time. Remote work does not need Claude installed or signed in on
  this computer. The team rejected the procedure of Claude Desktop, which sends
  the local credential. The cause is that all persons who use the same Unix
  account on the machine can read that credential. Each attachment updates
  `claude auth status`. Thus the machine row shows if its workspaces can start.
- **Quit.** When the person quits the app, the app does not show a warning
  about remote work. When Prometeu drops a remote waiter, its process continues
  to run. When the person closes a tab, clicks Stop, or removes or cleans a
  workspace, Prometeu continues to stop the remote process.
- **Running terminals.** Settings lists the terminals that run on the host.
  This list includes idle agents and terminals that no desktop tab shows. The
  person can stop each terminal explicitly after a confirmation. The
  confirmation agrees with the unique launch instance of the terminal. The host
  validates that instance and signals the same process handle. Thus an earlier
  confirmation cannot stop a resumed conversation. The new `terminal_stop`
  method fails closed on hosts from earlier versions. These hosts ignore an
  optional check on `terminal_signal` without a message. The person can
  continue to examine hosts from earlier versions. But the stop action in the
  list is disabled until the host updates. The list shows Prometeu terminals,
  not all processes of the operating system.

### Scope

Claude and Codex declare the `remoteMachines` capability. Prometeu relays and
mirrors the hooks and the transcript layout of Claude. Codex runs its native
engine on the machine, and Prometeu reads it through a relay terminal
([ADR 0105](0105-codex-on-remote-machines.md)). The host is available only for
Linux x86_64.

Remote workspaces show only conversations. These features stay local until
each one has an implementation on the host:

- The file tree, the Git panel and the diff review.
- Setup and Run, and the docks.
- The MCP and plugin selection, and the kickoff skills.
- Delegation and workspaces with more than one repository.

The cleanup removes the remote worktree and branch through Git on the machine.

## Consequences

- The execution continues through the sleep of the desktop, network changes and
  new starts of the app. It does not continue after a reboot of the machine or
  after the host stops. Then the transcript resumes in a new process, as for a
  local terminal agent after a new start of the app.
- The interface does not need a second implementation. Prometeu hides the
  remote features that are not available. It does not show features that
  do not operate fully. Each feature can move to the host later without a
  change to the board format.
- Prometeu does not send input again after the connection stops. A keystroke
  that Prometeu sent after the SSH process stopped has an unknown result, as
  all other lost replies.
- Remote work that started from the desktop needs the desktop in operation.
  Agent clients on the machine start work without the desktop, through the
  host ([ADR 0103](0103-agents-on-remote-machines.md)).

## Evidence

- [Remote host protocol](../contracts/remote-host.md).
- `src-tauri/crates/host/tests/host.rs`: detachment and gap replay, queued hooks
  and decisions, transcript streaming, attachment replacement.
  [ADR 0104](0104-several-computers-on-one-machine.md) covers more than one
  computer.
- `src-tauri/src/remote/terminal.rs`, `connection.rs`, `agent.rs` and
  `workspace.rs` unit tests: the sequence order, lost terminals, mirror paths
  and appends, the change of transcript paths, and the project identity.
- `remote::journey::vps_journey` (opt-in, against a real machine):
  - The installation.
  - The terminal output through an SSH connection that stops, without a gap
    or a duplicate.
  - The `memory.max` and `cpu.max` that apply to the workspace slice.
- `remote_hosts_round_trip_and_local_entries_omit_them` in
  `crates/core/src/board.rs`: boards without remote entries save without a
  change.
- A manual run against a real machine (2026-11-23):
  1. A workspace from a local project on the machine received its prompt and
     started a task of 90 seconds.
  2. The team stopped the desktop process with `SIGSTOP` and stopped its SSH
     connection.
  3. The task completed on the machine.
  4. After `SIGCONT`, the desktop connected again. In 8 seconds, the
     conversation showed the tool results, the last reply and the completed
     turn.
