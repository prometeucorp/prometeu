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

  Settings and the launcher show the same state. Codex signs in on the machine
  (`codex login --device-auth` through SSH). The app does not copy the Codex
  credentials of this computer at any time.

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
  available in remote workspaces, as for Claude. Settings does not let the
  person sign Codex in. The person runs the native device login on the machine.

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
- `src/machines.test.ts`: the Codex state of the machine row.
- The team did not verify this decision on a real machine with Codex installed
  and signed in at this time. See the
  [provider matrix](../quality/provider-matrix.md).
