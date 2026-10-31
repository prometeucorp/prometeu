---
type: Decision
title: "ADR 0013 — Account removal and empty selection"
description: "Account removal and empty selection."
status: stable
---

# ADR 0013 — Account removal and empty selection

Date: 2026-09-05

Complements the local profiles of [ADR 0012](0012-provider-accounts.md) with
removal and an optional selection per provider.

## Context

The person needs to remove accounts, including the last account and the one
inherited from the CLI. Processes capture their profiles and may keep running
after the removal. Deleting credentials or the directories at that moment could
interrupt a turn or affect the links to the shared history.

## Options considered

- Prevent removing the external profile: it preserves the mandatory selection,
  but does not allow clearing the list as requested.
- Delete credentials and profiles: it requires coordinating every running
  process and query, plus each provider's specific storage.
- Remove the registration and allow an empty selection: it serves the selector
  without changing the terminal's login or interrupting existing processes.

## Decision

Allow removing any account from the registry. Removing the active account leaves
the provider without a selection; it does not choose another account
automatically. New messages require an explicit choice, while the turn already
started may finish.

The empty registry is persisted. Terminal profiles are imported only if the
registry file does not exist yet. Adding another account starts the official
login without asking for a nickname; the email identifies the account. Old
registries with `label` stay readable, but that field is no longer used or
written.

## Consequences

Removal does not log out and does not revoke credentials. Private directories,
Keychain and history are preserved. Removed profiles stop being queried in the
next quota rounds, and late responses do not recreate them in the registry.
There is no collection of orphaned profiles in this operation; that collection
will require coordinating every reader before deleting credentials and links.

## Evidence

The [accounts contract](../contracts/accounts.md) describes the optional
selection, the removal IPC and the cleanup limit. `accounts.rs` tests a legacy
registry, complete removal and an empty re-read. `e2e/accounts.spec.ts` checks
removal, reopening, a new login and the preservation of the conversation and of
the other provider.
