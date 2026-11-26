---
type: Decision
title: "ADR 0003 — Capability-driven features"
description: "The interface shows features from the capabilities of each agent, not from the name of the provider."
status: stable
---

# ADR 0003 — Capability-driven features

Date: 2026-09-03

## Context

At first, the model identified the provider without an explicit field. Some
parts of the interface compared the name to `codex` to hide the plan mode or
the plugins. `agent` was an open string in TypeScript and in the Rust state.

That design operates with two known CLIs, but it puts knowledge about the
providers in many locations. For a third provider, a person must find each
condition and make each decision about the correct combinations again. Different
versions of the same CLI can also have different capabilities.

## Options considered

1. Continue to add conditions for each provider.
2. Make a different interface for each provider.
3. Find providers in typed descriptors and let the capabilities control the
   interface.

## Decision

Add a closed `ProviderId`, `AgentDescriptor`, `AgentModel` and
`AgentCapabilities`. The catalog explicitly connects a model to a provider. The
interface and the validation use the capabilities of the descriptor. Only the
registry and the runtime select the code path by `ProviderId`.

The contract in force is in [agent-runtime.md](../contracts/agent-runtime.md).

## Implementation

- `ProviderId` is a closed union in TypeScript and an enum in Rust.
- When the application loads the board, it accepts `agent: ""`. The next write
  changes the value to `agent: "claude"`. During the migration, unknown values
  also change to the default.
- `src-tauri/src/agents.rs` makes `AgentDescriptor[]` with the installation
  status and the capabilities. `agent_models` supplies live catalogs
  independently (ADR 0053). `src/agents.ts` is the boundary that the interface
  uses.
- The launcher, the conversation and the status bar use descriptors and
  capabilities. Selection by name stays only in the catalog and in the
  adapters.
- `scripts/check-architecture.mjs` shows a failure if the primary screens make a
  decision again when they compare the provider with `claude` or `codex`.
- Rust tests examine the normalization of the stored values and the
  capabilities of the descriptors.

## Consequences

Positive:

- One structure shows what each agent can do.
- The interface does not know the names of the providers.
- Capabilities can change with the CLI or the model, and no conditions go into
  a release.
- During development, a new provider causes a compilation failure because each
  selection in the code must include all providers.
- The project can make the conformance matrix from the catalog.

Negative:

- The catalog and the stored state must have a migration.
- Some capabilities are not only on or off. They can also have parameters.
- An incorrect descriptor can show a feature that has a failure at runtime.
- When a CLI does not answer, the discovery must have an explicit fallback.

## Acceptance criteria met

- The conditions for the current plan mode, MCP, plugins and attachments moved
  to capabilities.
- The runtime publishes the current capabilities for each provider.
- The `agent: "" | "claude" | "codex"` migration has a serialization test.
- The fitness function stops new decisions in the interface that use the name
  of the provider.
- For each behavior, the matrix points to evidence or to a limit.
