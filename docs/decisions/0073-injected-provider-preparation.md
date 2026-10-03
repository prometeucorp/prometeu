# ADR 0073 — Injected provider preparation and canonical input

Date: 2026-09-28
Status: Accepted

## Context

Portable launch and worker services still depended on provider functions that
accepted a desktop handle and launched chat directly. Shared chat also selected
input behavior through a vendor enum, coupling process composition to protocols.

## Decision

Introduce `ProviderPreparation<Prepared>` in the core and inject its native
implementation at composition. `agent_launch.rs` owns a provider registration
table. Adapters prepare native commands, captured account profiles, transcript
stores, error mapping and one-shot protocol factories without a desktop handle.
Preparation may materialize files or use existing bounded account/version probes;
conversation spawning remains the launcher's responsibility.

After `ProcessLauncher` starts the command, its input and control connect the
factory. `AgentProtocol` supplies translation and `AgentInput`, which extends
canonical conversation input with closure and turn-wait policy. Shared chat
uses these interfaces rather than a vendor enum. Antigravity's interruption
uses injected process control; Claude and Codex retain native control messages.

The prepared native type stays at the execution edge. It is not a portable
command representation or serialized bridge contract. This keeps OS commands,
account materialization and protocol construction out of the core while allowing
a future host to reuse the interfaces. A generic prepared type avoids imposing
native configuration on other execution implementations.

## Compatibility and limits

No IPC, persisted field, V1 frame or provider capability changes. Resume and
transcript ownership remain provider-specific: Claude checks native history;
Codex/Antigravity use saved native identities and app-managed display logs.
Fresh starts never infer resume from an existing file. Explicit environment
values survive inherited Claude-variable cleanup. Input closure releases the
pipe even while shared reader state remains alive; Codex retains its sink behavior
for late protocol activity. Preparation errors retain existing restart ordering.

Account registry rules now use [injected storage](0074-injected-account-registry.md);
native profiles, login and tool modules still live in the desktop crate. This is not yet a
standalone provider crate or headless runtime. Feature composition, terminal
ownership, activation after host installation and the Windows/WSL transport
remain pending. ADR 0050's deployment decision remains in force.

## Evidence

- [Core ports](../../src-tauri/crates/core/src/session/provider.rs).
- [Native preparation and isolated compatibility tests](../../src-tauri/src/agent_launch.rs).
- Canonical input fixtures in [Claude](../../src-tauri/src/claude.rs),
  [Codex](../../src-tauri/src/codex.rs) and [Antigravity](../../src-tauri/src/antigravity.rs).
- [Application contract](../contracts/application-core.md#provider-preparation-and-input).
