---
type: Decision
title: "ADR 0053 — Live model catalogs and explicit selection identity"
description: "Live model catalogs and explicit selection identity."
status: stable
---

# ADR 0053 — Live model catalogs and explicit selection identity

Date: 2026-09-19

## Context

The picker combined a hand-maintained Claude fallback with Codex's local cache.
Failures looked like valid catalogs, model IDs implicitly selected a provider,
and a cyclic effort control invented levels when discovery supplied none.
Antigravity increases list size and can advertise names also offered by another
agent. These assumptions no longer produce reliable choices.

## Decision

Discover installations independently from asynchronous account-scoped catalogs.
Use each CLI's native discovery, including paginated Codex app-server model/list.
Bound discovery subprocesses and expose empty, failed, loading and stale states.
Refresh after account changes and on picker opening once five minutes old, with
an explicit refresh action; no background timer or persistent catalog cache.

Use explicit agent/model identity in every selector and persisted preference.
Reuse a searchable design-system primitive; compose model groups, local
favorites and additional-model filtering in the application. Favorites do not
establish availability. Existing conversations remain within their own provider.

Effort is selected directly from native catalog values or the provider default.
Remove synthetic Claude Ultracode and generic effort fallbacks. Preserve legacy
labels and saved choices; interpret Codex's old spelling at the boundary.

## Alternatives and consequences

Keeping cache files avoids a subprocess but cannot establish freshness or account
availability. Always querying on open adds latency; a five-minute in-memory cache
with visible failures balances responsiveness and accuracy. Discovery still
reflects the CLI's advertised list, not a promise that every inference succeeds.

A unified model-and-effort form adds confirmation to a frequent interaction.
Separate compact controls retain direct selection; changing a model explicitly
resets only an incompatible effort and indicates the adjustment.

No board, transcript or relay migration is needed. Local preference migration
must avoid guessing providers from duplicate IDs. Account changes invalidate
in-flight work; catalog updates never restart a conversation or overwrite choices.

## Contracts and evidence

- [Runtime discovery](../contracts/agent-runtime.md#model-discovery)
- [IPC](../contracts/ipc.md#model-catalogs)
- [Local preferences](../contracts/persistence.md#model-selection-preferences)
- Rust `agents::catalog::tests`: protocol parsing, pagination, errors and cleanup.
- `src/agents.test.ts`, `src/model-choice.test.ts`: races, freshness, identity and migration.
- `e2e/model-picker.spec.ts`, `e2e/search-picker.spec.ts`, `e2e/actions.spec.ts`:
  large catalogs, keyboard, favorites, dialogs and historical choices.
