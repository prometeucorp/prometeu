---
type: Decision
title: "ADR 0105 — Codex on remote machines"
description: "A remote Codex conversation runs its native engine and TUI on the machine. The desktop reads the RPC of the engine through a relay terminal on the host."
status: stable
---

# ADR 0105 — Codex on remote machines

Date: 2026-11-26

## Context

[ADR 0101](0101-remote-machines.md) moved only Claude to remote machines. The
host relays the native hooks of Claude and mirrors its transcripts, and the
desktop monitors them. Prometeu monitors a local Codex terminal conversation
differently:

- A private `codex app-server` owns the thread.
- The native TUI attaches to it with `--remote`.
- The desktop reads the JSON-RPC of the engine through WebSocket on a private
  Unix socket.

That stream is the only canonical source of the conversation. Prometeu drains
the hooks, and the rollouts are only evidence
([native Codex engine](../contracts/native-codex-engine.md)).

Thus, to run Codex on a machine, the desktop must get the RPC of the engine. It
does not need hooks or a transcript mirror. The stream must continue through the
same events as remote terminals. For example, the laptop sleeps, SSH
disconnects, or the app starts again while the machine continues to operate.

## Options considered

1. Run the engine on the desktop and only the TUI on the machine. The engine
   runs the commands of the model. Thus they run on the incorrect computer.
2. Forward the Unix socket of the engine with `ssh -L`. Each conversation must
   have a second SSH process. The forwarded stream stops when SSH stops. The
   peer and socket checks of the engine cannot operate between machines.
3. Add host methods that tunnel the socket through the attachment. The stream
   continues to stop with each attachment. Thus each sleep causes a full
   reattachment, and the host protocol gets a second streaming mechanism.
4. Run the engine and a small relay as usual host terminals. The relay owns the
   WebSocket connection on the machine. It writes one JSON message on each line
   of a raw PTY. The terminal output of the host has sequence numbers. It moves
   the messages to the desktop and replays the data that a desktop did not get.

## Decision

Use option 4.

- Codex declares `remoteMachines`. A remote Codex conversation runs in the
  terminal mode, as all remote conversations do.
- The desktop prepares the same engine plan as for a local conversation,
  without an account profile, plugins or MCP. The `~/.codex` sign-in and
  configuration of the machine apply. The desktop opens
  `codex app-server --listen unix://<root>/sessions/<tab>/rpc` as the
  terminal `<tab>-codex`, in the slice and limits of the workspace. Then it
  opens a relay terminal `<tab>-rpc-<incarnation>` that runs the
  `prometeu-host codex-relay --socket <path> --pid <engine pid>` of this build.
- The relay does the same checks as the local client:
  - The alias of Codex goes to its endpoint with a SHA-256 name.
  - The folder of the endpoint is private, and the user owns the socket.
  - Its peer must be the process of the engine or a descendant. Thus the Node
    launcher of an npm installation continues to pass the check.

  Then the relay prints a fixed announcement line and relays messages. The
  desktop reads JSON lines only after that announcement.
- The desktop runs the bootstrap without changes through that stream. The
  bootstrap has these steps: initialize, the accurate start or resume of the
  thread, the verification of the settings, and the history in pages. Then the
  desktop opens `codex --remote unix://… resume <thread>` as the TUI terminal
  `<tab>` of the conversation.
- Prometeu does not install hooks for remote Codex. The local terminal path
  also discards Codex hooks. The registry of the host does not show Codex
  conversations to agent clients, because these clients resume only Claude
  ([ADR 0103](0103-agents-on-remote-machines.md)).
- When the SSH connection stops, the relay continues to run. The reattachment
  replays the lines of the relay that the desktop did not get, as for all
  terminal output. Sometimes the host cannot replay the gap (more than its
  buffer of 2 MiB while the desktop was away), or the relay stops. Then
  Prometeu releases the conversation and adopts it again. The processes on the
  machine continue. Prometeu replaces the relay, and the bootstrap reads the
  history from the engine. The native item IDs of the engine keep the journal
  free of duplicates.
- After a new start of the app or a reconnection, the adoption follows the TUI
  and the engine that run. It does not start a second engine for the same
  thread at any time. When two computers follow the same conversation, each
  computer has its own relay
  ([ADR 0104](0104-several-computers-on-one-machine.md)).
- When the TUI exits while this app follows it, Prometeu stops the engine. A
  launch stops an engine before its TUI only when the launch opened that
  engine. In these conditions, the engine continues to run:
  - A resume lost a race on a different computer (`terminal_exists`).
  - An adoption had a failure.

  At the next attachment, Prometeu stops an engine that has no TUI and that is
  older than the bootstrap of a launch. When a Codex TUI exits while no desktop
  is attached, the host forgets it and does not recover it. Its content is in
  the thread of the engine, and the next launch resumes it.
- Each attachment and each launch run `codex --version` and
  `codex login status` on the machine. Prometeu reads only the exit codes and
  the version line. In these conditions, a new launch stops with a message that
  tells the person what to do:
  - The CLI is not there.
  - The release is earlier than 0.169.0.
  - The sign-in is not there.

  Settings and the launcher show the same state. The app does not copy the
  Codex credentials of this computer at any time.

## Sign-in on the machine

Codex must sign in on the machine, with the Unix user of the SSH destination.
The team considered these options:

1. Copy the Codex credential of this computer to the machine. All persons who
   use the same Unix user can read it. [ADR 0101](0101-remote-machines.md)
   rejects this procedure for Claude for the same cause.
2. Run the browser login of Codex on the machine. Its callback goes to
   `localhost` on the machine. Thus the desktop must forward a port through SSH
   for each sign-in.
3. Run the device sign-in of Codex in a host terminal. The desktop reads only
   the link and the one-time code that the CLI prints. The person enters the
   one-time code in a browser on any computer. Codex on the machine gets the
   credential and keeps it there.

Use option 3.

- Settings shows "Sign in to Codex" when Codex is installed, supported and
  not signed in on a connected machine. Before a start, the desktop reads
  `codex login status` again. Codex removes the stored credential when a login
  starts. Thus the desktop does not start a sign-in after a different computer
  completed one.
- The desktop opens the host terminal `codex-login-<id>`, which runs
  `codex login --device-auth` in the home folder. It reads the output for a
  maximum of 30 s. It accepts only a link to
  `https://auth.openai.com/codex/device` and a one-time code on a line after
  that link. It accepts a line only when the line is complete. If it does not
  find the two values, it stops the terminal and shows
  `err.remote.codexLoginStart`.
- The desktop sends only the link and the one-time code to the interface. It
  does not send other output of the CLI to the interface, to a log or to an error
  message.
- Settings shows the one-time code, the link, and controls to open and copy
  them. Then it waits for the exit of the terminal. After the exit, the desktop reads
  `codex login status` again and sends the result as `codex` in `machines`. The
  account state decides the result, not the exit code.
- Codex stops its token requests 15 minutes after it gets the one-time code.
  The desktop waits a maximum of 16 minutes. If the terminal continues to run after that time, the
  desktop stops it and shows `err.remote.codexLoginExpired`.
- The desktop records each attempt before it sends `terminal_open`. The
  record keeps the connection and the SSH destination of the start. The
  desktop sends a status of the attempt to `machines` only while the machine
  has the same connection and destination.
- Cancel, a new sign-in on the same machine, the removal of the machine and a
  change of its SSH destination stop the terminal. These events also stop an
  attempt before it shows its challenge. The desktop sends `kill` to the
  process group and waits a maximum of 5 s for the exit. Then it forgets the
  terminal on the host and releases its subscription.
- Sometimes the desktop cannot confirm the end of a login terminal. For
  example, SSH stops before the reply to `terminal_open`, or the person
  cancels while the machine is offline. Then the desktop keeps the terminal
  name and the destination in memory. After the next attachment to the same
  destination, it stops and forgets that terminal.
- This cleanup can fail. For example, the list of terminals does not arrive,
  or the terminal does not stop in 5 s. Then the desktop keeps the record. It
  tries again each 30 s while the connection continues, a maximum of 10 times.
  Each later attachment starts the cleanup again.
- After each attachment, the desktop also stops each running login terminal
  that is older than 16 minutes. Each computer stops its own sign-in at that
  age. Thus this rule does not stop a sign-in in progress. The rule also
  operates after a restart of the app, when the app loses the records in memory.
- The host writes the start time of a terminal with the clock of the machine.
  Thus the desktop reads the time of the machine with `date +%s` and
  calculates the age with that clock. The clock of the desktop can be
  different, and that difference must not stop a new sign-in of a different
  computer. If the desktop cannot read the clock of the machine, it does not
  stop a terminal because of its age.
- When SSH disconnects, the login terminal continues on the host, as all host
  terminals do. The subscription attaches again with the connection, and the
  host replays the output and the exit. When the app quits, or the machine
  stays offline, the CLI on the machine stops after its own limit of 15
  minutes. It does not write a credential after that limit. The next
  attachment of any computer forgets that stopped terminal.
- Two computers can each start a sign-in on the same machine. The first sign-in
  that completes writes the credential. The other one continues until the
  person cancels it or its one-time code expires.

## Consequences

- Remote Codex keeps the guarantees of the local engine: one RPC source in
  sequence, the accurate thread identity, verified settings and native item
  receipts.
- A long disconnection during a busy turn causes a relay replacement and a read
  of the history. The replay does not continue without a break. Prometeu does
  not show the deltas that the engine made while the desktop was away. It shows
  the last items.
- The desktop sends requests to the engine (interrupts, request answers, quota
  reads). These requests are relay writes that the host confirms. When the host
  does not confirm a write, the write fails, but the link continues. A request
  answer stays open, and the desktop can send it again when the machine is
  available. If the confirmation is lost after the host wrote it, the result is
  unknown. Then the engine rejects the repeated answer.
- Codex plugins, the MCP selection, hooks and the account selection are not
  available in remote workspaces, as for Claude. Settings signs Codex in on the
  machine with the device sign-in of Codex. The person can also run
  `codex login --device-auth` through SSH.
- The sign-in depends on the text that the CLI prints. If a Codex release
  changes the link or the position of the one-time code, Settings shows
  `err.remote.codexLoginStart`. Then the person can use SSH.
- If the Codex configuration requires a configuration server, the CLI signs in
  to that server with a different message. Settings does not recognize that
  message.

## Evidence

- `src-tauri/crates/host/tests/host.rs`: the relay moves JSON lines in the two
  directions and drops partial input. It rejects an engine that a different
  process serves. As a host terminal, it is transparent (no echo, no CRLF), and
  it replays the lines that a detached desktop did not get.
- `src-tauri/crates/host/src/codex.rs` unit tests: terminal names, socket
  location and process ancestry.
- `src-tauri/src/terminal_agent_codex/wire.rs` tests: the desktop reads only
  after the announcement. It keeps notifications in sequence around replies,
  and it stops the stream at a broken line.
- `src-tauri/src/remote/codex.rs` tests: relay sequences, gaps that Prometeu
  cannot recover, orphan engines, status problems with an action, relay writes
  without a confirmation and
  `a_resume_that_lost_the_race_never_ends_the_winners_engine`.
- `src-tauri/src/terminal_agent_codex/tests.rs`
  `an_answer_the_machine_did_not_receive_stays_pending_and_can_be_sent_again`.
- `src-tauri/src/terminal_agent_launch.rs`
  `remote_codex_uses_the_machines_own_account_and_configuration` and
  `src-tauri/src/remote/registry.rs`
  `agent_clients_never_see_conversations_they_cannot_resume`.
- `src/machines.test.ts`: the Codex state of the machine row, and the sign-in
  in Settings (the challenge, the completion, the expiry, the cancel and a
  replaced attempt).
- `src-tauri/src/remote/codex_login.rs` tests: the link and the one-time code
  from the output of Codex 0.172.0, cut output, other hosts and paths, and the
  result from the account state. Lifecycle tests with a fake connection:
  - `an_open_whose_reply_was_lost_is_swept_at_the_next_attachment`
  - `a_cancel_while_the_terminal_opens_stops_it_and_hides_the_challenge`
  - `a_challenge_wait_cut_by_ssh_keeps_the_terminal_for_the_sweep`
  - `a_login_cancelled_while_ssh_is_down_is_stopped_after_the_reconnection`
  - `a_cleanup_that_fails_after_the_reconnection_stays_pending_until_a_retry_ends_it`
  - `the_age_of_a_login_uses_the_clock_of_the_machine`
  - `an_attempt_belongs_only_to_its_connection_and_destination`
  - `the_sweep_stops_orphans_and_old_logins_but_not_logins_in_progress`
  - `an_orphan_on_an_earlier_destination_is_not_stopped_on_the_new_one`
- `src/machine-processes.test.ts`: the process list names the login terminal.
- Observed CLI behavior, 2026-11-26. The team read
  `codex-rs/login/src/device_code_auth.rs` and `codex-rs/cli/src/login.rs` at the tag `rust-v0.172.0` (commit
  `3d86e43b`). The team also operated the published `@openai/codex@0.172.0`
  Linux build in a PTY with an empty `CODEX_HOME` and a local stub issuer
  (`--experimental_issuer`). The CLI printed the prompt that the fixture
  records. Then it sent token requests at the interval of the stub. Only
  the issuer host of the link was different. The test did not send a request
  to OpenAI and did not use a real credential. `codex login status` without a credential printed
  "Not logged in" and exited with code 1.
- The team did not verify this decision on a real machine with Codex installed
  and signed in at this time. The sign-in from Settings did not operate with a
  real OpenAI account. See the
  [provider matrix](../quality/provider-matrix.md).
