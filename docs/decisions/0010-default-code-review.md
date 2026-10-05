# ADR 0010 — Code review included in the actions registry

Date: 2026-09-05
Status: Accepted

## Context

The empty registry required configuring an agent and a command before trying a
review. The screen also gave excessive prominence to manually installing an
example and to the PR button's settings.

## Decision

Include the editable **Code review** profile, associated with `/review`, on the
first opening after this change. The profile reviews changes, reports findings
and validation gaps and ends the turn. It does not publish a PR, modify files or
follow CI by default. Those responsibilities stay configurable in other
profiles.

The additive `defaults_initialized` field records the initialization. The
default is materialized in the registry, instead of being an implicit layer that
reappears after removal. An existing configuration with the same command or
identity takes precedence; it is not replaced. The Open PR button keeps the
person's choice.

The source JSON is shared by the backend and the mock. There is no transcript
migration and no change in the configuration of tasks already started.

## Consequences

There is a usable action on the first visit. The person can edit or remove the
profile and the command without the app undoing their choice on reopening. The
screen prioritizes commands, groups the agents and collapses the PR button's
settings.

This decision complements [ADR 0009](0009-reusable-actions.md), changing only
the initial catalog; it preserves its execution contracts.

## Evidence

- [Shared default](../../src/action-defaults.json).
- [Initialization and compatibility](../../src-tauri/src/actions.rs).
- [Mock parity](../../src/actions.test.ts).
- [Registration, editing and removal](../../e2e/actions.spec.ts).
