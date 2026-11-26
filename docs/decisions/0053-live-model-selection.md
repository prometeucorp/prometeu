---
type: Decision
title: "ADR 0053 — Live model catalogs and explicit selection identity"
description: "Live model catalogs and an explicit identity for each selection."
status: stable
---

# ADR 0053 — Live model catalogs and explicit selection identity

Date: 2026-09-19

## Context

The picker used a Claude fallback list that persons edited manually, together
with the local cache of Codex. These problems occurred:

- A failure looked like a correct catalog.
- A model ID selected a provider implicitly.
- A cyclic effort control showed effort values that discovery did not supply.

Antigravity makes the list longer. It can also show model names that a
different agent shows. With these assumptions, the selections that the
picker showed were not reliable.

## Decision

Find the installations independently from the catalogs of each account. These
catalogs load asynchronously. Use the native discovery of each CLI. For Codex,
this includes the app-server model/list with pages. Put a time limit on the
discovery subprocesses. Show these states: empty, failure, load and expired.
Refresh the catalog after an account change. Also refresh it when the picker
opens and the catalog is older than five minutes. Give an explicit refresh
action. Do not use a background timer or a catalog cache on the disk.

Use an explicit agent and model identity in each selector and in each saved
preference. Use the searchable primitive of the design system again. The
application adds model groups, local favorites and the filter for more
models. A favorite does not prove that a model is available. Current
conversations stay with their own provider.

The person selects the effort directly from the native catalog values or the
provider default. Remove the synthetic Claude Ultracode and the general effort
fallbacks. Keep the labels and the saved selections from before this change.
Translate the previous spelling of Codex at the boundary.

## Alternatives and consequences

Cache files do not start a subprocess. But they cannot show if the data is new
or if the account has access. A query each time the picker opens makes the
picker slow. A cache in memory for five minutes, with failures that the person
can see, gives a good balance between speed and precision. Discovery continues to
show the list that the CLI gives. It does not promise that each model request
will complete correctly.

One form for the model and the effort adds a confirmation to a frequent
operation. Two small different controls let the person select each value directly. When the
person changes the model, Prometeu changes the effort only if the new model
cannot use it. The interface then shows this change.

A migration of the board, the transcript or the relay is not necessary. The
migration of the local preferences must not guess a provider from a duplicate
ID. An account change cancels the work that continues at that time. A catalog update
does not start a conversation again and does not replace the selections of the
person.

## Contracts and evidence

- [Runtime discovery](../contracts/agent-runtime.md#model-discovery)
- [IPC](../contracts/ipc.md#model-catalogs)
- [Local preferences](../contracts/persistence.md#model-selection-preferences)
- Rust `agents::catalog::tests`: protocol parsing, pages, errors and cleanup.
- `src/agents.test.ts`, `src/model-choice.test.ts`: race conditions, how new the
  data is, identity and migration.
- `e2e/model-picker.spec.ts`, `e2e/search-picker.spec.ts`, `e2e/actions.spec.ts`:
  large catalogs, keyboard, favorites, dialogs and saved selections from before.
