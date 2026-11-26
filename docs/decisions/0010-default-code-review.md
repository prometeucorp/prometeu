---
type: Decision
title: "ADR 0010 — Code review included in the actions registry"
description: "The actions registry starts with an editable Code review profile and its command."
status: stable
---

# ADR 0010 — Code review included in the actions registry

Date: 2026-09-05

## Context

The registry was empty. Thus, a person had to configure an agent and a command
before the first review. The screen also gave too much space to the manual
installation of an example and to the settings of the PR button.

## Decision

Add the editable **Code review** profile, with `/review`, when the person opens
the registry for the first time after this change. The profile does a review
of the changes. It tells the results and the missing validation, and then it
completes the turn. By default, it does not publish a PR, change files or follow CI.
Other profiles can do those operations.

The added `defaults_initialized` field records the initialization. The registry
stores the default as a usual entry. It is not a hidden layer that comes back
after a person removes it. If the current configuration has the same command or
identity, the application keeps the current configuration. It does not
replace it. The Open PR button keeps the selection of the person.

The backend and the mock use the same source JSON. The decision does not change
transcripts, and it does not change the configuration of tasks that started
before.

## Consequences

An action is available on the first visit. The person can edit or remove the
profile and the command. When the person opens the registry again, the
application does not change these selections back. The screen shows commands
first, puts the agents in groups and closes the section with the settings of the PR button.

This decision adds to [ADR 0009](0009-reusable-actions.md). It changes only the
initial catalog, and it keeps the execution contracts of ADR 0009.

## Evidence

- [Shared default](../../src/action-defaults.json).
- [Initialization and compatibility](../../src-tauri/src/actions.rs).
- [Mock parity](../../src/actions.test.ts).
- [Registration, edits and removal](../../e2e/actions.spec.ts).
