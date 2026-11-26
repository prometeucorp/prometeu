---
type: Decision
title: "ADR 0105 — Codex on remote machines"
description: "A remote Codex conversation runs its native engine and TUI on the machine; the desktop reads the engine's RPC through a relay terminal on the host."
status: stable
---

# ADR 0105 — Codex on remote machines

Date: 2026-11-26

## Context

[ADR 0101](0101-remote-machines.md) moved only Claude to remote machines: its
native hooks are relayed by the host and its transcripts are mirrored, and the
desktop observes those. A local Codex terminal conversation is observed
differently. A private `codex app-server` owns the thread, the native TUI
attaches to it with `--remote`, and the desktop reads the engine's JSON-RPC over
WebSocket on a private Unix socket. That stream is the conversation's only
canonical source: hooks are drained and rollouts are evidence only
([native Codex engine](../contracts/native-codex-engine.md)).

Running Codex on a machine therefore needs the engine's RPC on the desktop, not
hooks or a transcript mirror. The stream must survive what remote terminals
survive: the laptop sleeping, SSH dropping and the app restarting while the
machine keeps working.

## Options considered

1. Run the engine on the desktop and only the TUI on the machine. The engine
   executes the model's commands, so they would run on the wrong computer.
2. Forward the engine's Unix socket with `ssh -L`. Each conversation needs a
   second SSH process, the forwarded stream dies with SSH, and the engine's
   peer and socket checks cannot run across machines.
3. Add host methods that tunnel the socket over the attachment. The stream would
   still end with each attachment, so every sleep would cost a full
   reattachment, and the host protocol would grow a second streaming mechanism.
4. Run the engine and a small relay as ordinary host terminals. The relay owns
   the WebSocket connection on the machine and writes one JSON message per line
   on a raw PTY; the host's sequence-numbered terminal output carries it to the
   desktop and replays what a desktop missed.

## Decision

Option 4.

- Codex declares `remoteMachines`. A remote Codex conversation runs in terminal
  mode, as every remote conversation does.
- The desktop prepares the same engine plan as locally, without account profile,
  plugins or MCP: the machine's own `~/.codex` sign-in and configuration apply.
  It opens `codex app-server --listen unix://<root>/sessions/<tab>/rpc` as the
  terminal `<tab>-codex`, in the workspace's slice and limits, and then a relay
  terminal `<tab>-rpc-<incarnation>` running this build's
  `prometeu-host codex-relay --socket <path> --pid <engine pid>`.
- The relay applies the checks the local client applies: Codex's alias maps to
  its SHA-256 named endpoint, the endpoint's directory is private and the socket
  belongs to the user. Its peer must be the engine's process or a descendant, so
  an npm installation's Node launcher still verifies. It then prints a fixed
  announcement line and relays messages. The desktop reads JSON lines only after
  that announcement.
- The desktop runs the unchanged bootstrap through that stream (initialize,
  exact thread start or resume, settings verification, paginated history), then
  opens `codex --remote unix://… resume <thread>` as the conversation's TUI
  terminal `<tab>`.
- Hooks are not installed for remote Codex. The local terminal path already
  discards Codex hooks, and the host's registry does not offer Codex
  conversations to agent clients, which resume Claude only
  ([ADR 0103](0103-agents-on-remote-machines.md)).
- A dropped SSH connection leaves the relay running. Reattachment replays the
  relay's missed lines like any terminal output. When the host can no longer
  replay the gap (more than its 2 MiB buffer while away) or the relay ends, the
  conversation is released without ending the machine's processes and adopted
  again: the relay is replaced and the bootstrap reads history from the engine,
  whose native item IDs keep the journal free of duplicates.
- Adoption after an app restart or a reconnection follows the running TUI and
  engine; it never starts a second engine for the same thread. Two computers
  following the same conversation each have their own relay
  ([ADR 0104](0104-several-computers-on-one-machine.md)).
- When the TUI exits while this app follows it, the engine is terminated. A
  launch ends an engine before its TUI only when it opened that engine itself:
  a resume that lost a race on another computer (`terminal_exists`) or a failed
  adoption leaves it running. An
  engine whose TUI is gone and that is older than a launch's bootstrap is
  terminated at the next attachment. A Codex TUI that exited while no desktop
  was attached is forgotten instead of recovered: its content is in the engine's
  thread, and the next launch resumes it.
- Each attachment and each launch run `codex --version` and
  `codex login status` on the machine; only the exit codes and version line are
  read. A missing CLI, a release before 0.169.0 or a missing sign-in stops a new
  launch with an actionable message. Settings and the launcher show the same
  state. Codex signs in on the machine itself (`codex login --device-auth` over
  SSH); the app never copies this computer's Codex credentials.

## Consequences

- Remote Codex keeps the local engine's guarantees: one ordered RPC source, exact
  thread identity, verified settings and native item receipts.
- A long disconnection during a busy turn costs a relay replacement and a history
  read instead of a seamless replay. Deltas produced while away are not shown;
  final items are.
- Requests the desktop sends to the engine (interrupts, request answers, quota
  reads) are relay writes the host confirms. A write that is not confirmed fails
  without ending the link: a request answer stays open and can be sent again once
  the machine is reachable. A confirmation lost after the host wrote it still has
  an unknown outcome; the engine then rejects the repeated answer.
- Codex plugins, MCP selection, hooks and account selection stay unavailable in
  remote workspaces, as for Claude. Signing Codex in from Settings is not
  offered; the person runs the native device login on the machine.

## Evidence

- `src-tauri/crates/host/tests/host.rs`: the relay carries JSON lines both ways
  and drops partial input, refuses an engine served by an unrelated process, and
  as a host terminal is transparent (no echo, no CRLF) and replays the lines a
  detached desktop missed.
- `src-tauri/crates/host/src/codex.rs` unit tests: terminal naming, socket
  location and process ancestry.
- `src-tauri/src/terminal_agent_codex/wire.rs` tests: the desktop reads only after
  the announcement, keeps notifications ordered around replies and ends the
  stream at a broken line.
- `src-tauri/src/remote/codex.rs` tests: relay sequence handling, unrecoverable
  gaps, orphan engines, actionable status problems, unconfirmed relay writes and
  `a_resume_that_lost_the_race_never_ends_the_winners_engine`.
- `src-tauri/src/terminal_agent_codex/tests.rs`
  `an_answer_the_machine_did_not_receive_stays_pending_and_can_be_sent_again`.
- `src-tauri/src/terminal_agent_launch.rs`
  `remote_codex_uses_the_machines_own_account_and_configuration` and
  `src-tauri/src/remote/registry.rs`
  `agent_clients_never_see_conversations_they_cannot_resume`.
- `src/machines.test.ts`: the machine row's Codex state.
- Not yet verified against a real machine with Codex installed and signed in;
  see the [provider matrix](../quality/provider-matrix.md).
