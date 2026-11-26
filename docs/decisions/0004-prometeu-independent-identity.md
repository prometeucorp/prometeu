---
type: Decision
title: "ADR 0004 — Prometeu's independent identity"
description: "Prometeu is an independent application with its own repository, bundle, state and configuration."
status: stable
---

# ADR 0004 — Prometeu's independent identity

Date: 2026-09-04

## Context

The public name of the product became Prometeu, and its domain is
`prometeu.co`. If the project changes the identity of the previous application
in the same installation, that change mixes these items until it is complete:

- The bundle and the updater.
- The local state and the worktrees.
- The contracts of the scripts.

That change also stops a person from installing the two applications together.
Thus, the person cannot examine the new application before the migration of
the data of the person.

The project copied the code and the Git history into a new repository. The
Git history continues to show the previous identity. It also stays in
compatibility tokens that the current transcripts must have.

## Options considered

1. Rename the current application and move its data during an update.
2. Share the same state roots between the two names.
3. Make Prometeu a different application and import the data after that.

## Decision

Prometeu has its own repository, bundle, executable, updater, local namespace,
project configuration and environment variables:

- The bundle id is `co.prometeu.desktop`.
- The state is in `~/.prometeu` and `~/.prometeu-dev`.
- The worktrees are in `~/prometeu/worktrees[-dev]`.
- The configuration is in `.prometeu/settings.toml`.
- Public variables have the `PROMETEU_` prefix.
- The application makes branches with the `prometeu/` prefix.
- Releases are in `prometeucorp/prometeu`, together with the source, as
  [ADR 0040](0040-open-source.md) specifies.

The application loads project settings only from `.prometeu/settings.toml`.
Workspace scripts get `PROMETEU_` variables and the usual `PORT` variable. The
application does not use other configuration files or other variable prefixes
as a fallback.

The application does not read or change Prometheus data automatically. The
project removed the temporary importer. The application can read the boards and
transcripts that a person imported before. The cleanup accepts an inherited
worktree with many repositories only at the path that
`paths::prometheus_multi_dir` calculates. The cleanup does not move folders,
and it does not find previous boards. See [persistence](../contracts/persistence.md).

Prometeu writes only V1 events. For the current transcripts, the reader can
read the historical `prometheusV1Mirror` and `type: "prometheus"` tokens. It
does not write a mirror for rollback. See
[ADR 0002](0002-canonical-conversation-protocol.md).

## Consequences

Positive:

- A person can install and operate the two products together.
- Work on Prometeu does not put the current state at risk.
- The application can read imported data without access to the previous state root.
- New names do not accidentally move public contracts.

Negative:

- Data from an installation without a migration must have a previous importer or
  a manual migration.
- A person must make the `.prometheus` configurations of other repositories
  again, or import them carefully.
- External integrations, signing and the release infrastructure must have new
  credentials.
- The legacy reader continues to contain two identifiers with the previous name.

Linear uses the OAuth registration of Prometeu, with Authorization Code, PKCE
and the `read,write` scopes. The write grant lets a person explicitly become the assignee of an issue that
has no assignee. The issue must be in a team that has the person as a member.
Read-only credentials from before continue to operate for discovery. For an
assignment, the person must authorize again. The Linear `write` scope gives
more than issue assignment. Prometeu uses it only for an explicit local claim
action, and it does not get admin access. See the
[Linear contract](../contracts/linear-issues.md).

The claim action stays available, but two clients can make an assignment for
the same issue at the same time. The Linear `issueUpdate` cannot make the assignment conditional
on an issue with no assignee. If the Available query has a failure, the
interface shows the failure and continues to show the result of a correct Mine
query. When there is a last available list, the interface continues to show it.

## Evidence

- `paths.rs` tests examine the new roots.
- `scripts.rs` tests examine the new file and the new variables.
- `branch.ts` tests examine the new prefix.
- Conversation tests keep fixtures of the historical tokens.
- The Tauri configuration specifies a different product, binary and bundle
  id.
- `session.rs::check_allows_only_clean_workspaces_that_were_entered` examines
  the cleanup of inherited worktrees with many repositories.
