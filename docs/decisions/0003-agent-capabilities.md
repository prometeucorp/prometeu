---
type: Decision
title: "ADR 0003 — Capability-driven features"
description: "Capability-driven features."
status: stable
---

# ADR 0003 — Capability-driven features

Date: 2026-09-03

## Context

Originally, the model implicitly identified the provider and parts of the UI
compared the name `codex` to hide plan mode or plugins. `agent` was an open
string in TypeScript and in the Rust state.

That design works with two known CLIs, but it spreads vendor knowledge around. A
third provider would require finding every conditional and deciding again which
combinations are valid. Different versions of the same CLI may also offer
different capabilities.

## Options considered

1. Keep adding per-provider conditionals.
2. Create a different UI interface for each provider.
3. Discover providers in typed descriptors and drive the UI by capabilities.

## Decision

Introduce a closed `ProviderId`, `AgentDescriptor`, `AgentModel` and
`AgentCapabilities`. The catalog explicitly associates a model with a provider.
The UI and validation use the descriptor's capabilities; only the
registry/runtime dispatches by `ProviderId`.

The implemented contract is in [agent-runtime.md](../contracts/agent-runtime.md).

## Implementation

- `ProviderId` is a closed union in TypeScript and an enum in Rust.
- Board loading accepts `agent: ""` and normalizes the next write to
  `agent: "claude"`; unknown values also fall back to the default during the
  migration.
- `src-tauri/src/agents.rs` produces `AgentDescriptor[]` with installation status and
  capabilities; `agent_models` supplies live catalogs separately (ADR 0053); `src/agents.ts` is the boundary consumed by the UI.
- Launcher, conversation and status bar use descriptors/capabilities. Nominal
  dispatch stays only in the catalog and in the adapters.
- `scripts/check-architecture.mjs` fails if the main screens start deciding by
  direct comparison with `claude` or `codex` again.
- Rust tests cover persisted normalization and the descriptors' capabilities.

## Consequences

Positive:

- support becomes visible in a single structure;
- the UI stops knowing provider names;
- capabilities can vary with the CLI/model without releasing conditionals;
- new providers fail through non-exhaustive matching during development;
- the conformance matrix can be derived from the catalog.

Negative:

- the catalog and the persisted state need a migration;
- some capabilities are not purely boolean and may require parameters;
- an incorrect descriptor may offer a feature that fails at runtime;
- discovery needs an explicit fallback when a CLI does not answer.

## Acceptance criteria met

- the current plan mode, MCP, plugin and attachment conditionals were migrated;
- the current capabilities are published per provider at runtime;
- the `agent: "" | "claude" | "codex"` migration has a serialization test;
- the fitness function prevents new UI decisions based on the provider's name;
- the matrix points to evidence or a limitation for each behavior.
