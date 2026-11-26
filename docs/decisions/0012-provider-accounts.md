---
type: Decision
title: "ADR 0012 — Local accounts and global selection per provider"
description: "Local account profiles for each provider, with one global selection that the footer shows."
status: stable
---

# ADR 0012 — Local accounts and global selection per provider

Date: 2026-09-05

[ADR 0013](0013-remove-provider-accounts.md) gives the rules for the removal of
an account and for an empty selection. The plugin layer uses this account
boundary, as [ADR 0005](0005-portable-plugin-marketplace.md) tells.

[ADR 0052](0052-antigravity-runtime.md) adds the external Antigravity account to
this decision. The global selection does not change.

## Context

A person can have many Claude and Codex subscriptions. The footer shows global
quotas, but the application used the single login of each CLI. If the
application replaces the global credential files, this change has an effect on
external terminals and on processes that operate.

## Options considered

- Replace the global credential of the CLI: there are fewer directories. But
  it changes processes out of the application, and a turn can change its
  identity during its execution.
- Connect accounts to workspaces: it adds configuration to the board, and it
  does not give the global selection that the person wants.
- Local profiles for each account, with the CLI that does the authentication,
  and a shared history: the homes, the quotas and the resume must change. But
  the decision stays in the footer, and each process keeps the credential that
  it got at its start.

## Decision

Use local profiles and an optional selection that the application stores for
each provider. The login starts in Prometeu and uses the official flow of the
CLI in the browser. The application does not have its own OAuth server for the
subscriptions of the agents. The application imports the accounts of the
terminal as external profiles only when there is no registry file. When a
person removes an account, the removal is permanent in the registry. When the application
starts again, it does not make the account again, and it does not silently select a different
account.

A turn completes with the account that started it. The next message can start the
process again with the new selection, and it resumes the same transcript. If a
message comes during the change, it stays in the queue. The application does
not send the selection to the relay.

The credentials of different accounts stay isolated. The history, the plugins
and the skills stay available through explicit links and a derived
configuration. For the accounts that the application makes, Codex uses private
file storage. Thus, the derived plugin layer can point to the correct
credential. The application does not change the storage selection of the
external profile.

## Consequences

The quota cache keeps a different entry for each account, and the catalog
follows the selection. A Codex plugin home must be different for each workspace
and each account. Thus, the previous turn and the new process can operate at the same
time. Other authentication configurations cannot silently replace a managed
subscription.

The profiles share the local history. They do not keep the data of one account
secret from the other accounts of the same person. When a person changes the
account, this change also selects the provider and the account that continue
that history. Credentials do not go into the shared conversation at any time.
The format of the board and the protocol of the relay do not change.

## Evidence

The [accounts contract](../contracts/accounts.md) records the IPC, the storage,
the adaptation for each CLI and the tests. It also records a limit: the project
did not do the validation with two different accounts that are logged in.
