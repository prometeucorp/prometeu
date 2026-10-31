---
type: Decision
title: "ADR 0012 — Local accounts and global selection per provider"
description: "Local accounts and global selection per provider."
status: stable
---

# ADR 0012 — Local accounts and global selection per provider

Date: 2026-09-05

Account removal and an empty selection follow
[ADR 0013](0013-remove-provider-accounts.md). The plugin layer uses this account
boundary as described in [ADR 0005](0005-portable-plugin-marketplace.md).

The external Antigravity account extends this decision through
[ADR 0052](0052-antigravity-runtime.md); global selection remains unchanged.

## Context

A person may have several Claude and Codex subscriptions. The footer already
shows global quotas, but the app inherited a single login from each CLI.
Swapping global credential files would affect external terminals and running
processes.

## Options considered

- Swap the CLI's global credential: fewer directories, but it changes processes
  outside the app and allows a turn to change identity mid-execution.
- Associate accounts with workspaces: it adds configuration to the board and
  does not serve the requested global choice.
- Local profiles per account, with authentication delegated to the CLI and
  shared history: it requires adapting homes, quotas and resume, but keeps the
  decision in the footer and the credential captured per process.

## Decision

Adopt local profiles and an optional selection persisted per provider. The login
starts in Prometeu and uses the CLI's official flow in the browser. The app does not
implement its own OAuth server for the agents' subscriptions. Accounts that
already exist in the terminal are imported as external profiles only when the
registry file is absent. Removing them is permanent in the registry; restarting
does not recreate them or silently choose another account.

A turn ends with the account that started it. The next message may restart the
process with the new selection, resuming the same transcript. A message received
during the transition stays in the queue. The choice is not sent to the relay.

Credentials of different accounts stay separate. History, plugins and skills
stay available through explicit links and derived configuration. Codex uses
private file storage for accounts created by the app, allowing the derived
plugin layer to point to the correct credential. The external profile's storage
choice is not changed.

## Consequences

The quota cache now distinguishes accounts and the catalog follows the
selection. Codex plugin homes must distinguish workspace and account so that the
old turn and the new process can coexist. Alternative authentication
configurations cannot silently replace a managed subscription.

The profiles share local history: they are not a confidentiality barrier between
the same person's accounts. Switching accounts also chooses which
provider/account receives the continuation of that history. Credentials never
enter the shared conversation. The board's format and the relay's protocol do
not change.

## Evidence

The [accounts contract](../contracts/accounts.md) records IPC, persistence,
per-CLI adaptation, tests and the limit of validating without two real logins.
