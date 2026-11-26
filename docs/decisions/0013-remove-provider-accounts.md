---
type: Decision
title: "ADR 0013 — Account removal and empty selection"
description: "A person can remove each account, and a provider can have no selected account."
status: stable
---

# ADR 0013 — Account removal and empty selection

Date: 2026-09-05

This decision adds the removal of accounts and an optional selection for each
provider to the local profiles of [ADR 0012](0012-provider-accounts.md).

## Context

The person must be able to remove accounts. This includes the last account and
the account that the application imported from the CLI. Processes keep the
profile that they got at their start, and they can continue to operate after
the removal. If the application deletes the credentials or the directories at
that time, it can stop a turn or break the links to the shared history.

## Options considered

- Do not let a person remove the external profile: the mandatory selection
  stays, but the person cannot make the list empty.
- Delete credentials and profiles: the application must coordinate each process
  and query that operates. It must also know the special storage of each
  provider.
- Remove the registration and let the selection be empty: the selector does
  what the person wants. The login of the terminal does not change, and the
  current processes do not stop.

## Decision

Let a person remove each account from the registry. If the person removes the
active account, the provider has no selection. The application does not select
a different account automatically. A new message must have an explicit selection.
A turn that started before can continue until it is complete.

The application stores the empty registry. It imports terminal profiles only if
there is no registry file. When a person adds a different account, the
official login starts and the application does not tell the person to type a nickname. The
email identifies the account. The application can read previous registries with
`label`, but it does not use or write that field.

## Consequences

The removal does not log out, and it does not revoke credentials. The private
directories, the Keychain and the history stay. The application does not query
removed profiles in the next quota rounds. Responses that come late do not
add the profiles to the registry again. This operation does not collect profiles
that have no registration. That collection must coordinate each reader before
it deletes credentials and links.

## Evidence

The [accounts contract](../contracts/accounts.md) tells about the optional
selection, the removal IPC and the limit of the cleanup. Tests in `accounts.rs` examine a
legacy registry, a full removal and a read of an empty registry.
`e2e/accounts.spec.ts` examines the removal, the next opening, a new login, and
that the conversation and the other provider do not change.
