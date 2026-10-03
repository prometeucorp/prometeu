# ADR 0004 — Prometeu's independent identity

Date: 2026-09-04
Status: Accepted

## Context

The product's public name became Prometeu and its domain is `prometeu.co`.
Changing the previous application's identity in place would mix bundle, updater,
local state, worktrees and script contracts during the transition. It would also
prevent installing both applications side by side to check the new line before
migrating real data.

The code and the Git history were copied into a new repository. The previous
identity remains visible in Git history and in compatibility tokens that
existing transcripts still require.

## Options considered

1. Rename the existing application and migrate its data during an update.
2. Share the same state roots between the two names.
3. Create Prometeu as an independent application and import data later.

## Decision

Prometeu has its own repository, bundle, executable, updater, local namespace,
project configuration and environment variables:

- bundle id `co.prometeu.desktop`;
- state in `~/.prometeu` and `~/.prometeu-dev`;
- worktrees in `~/prometeu/worktrees[-dev]`;
- configuration in `.prometeu/settings.toml`;
- public variables with the `PROMETEU_` prefix;
- branches created with the `prometeu/` prefix;
- releases published in `prometeucorp/prometeu`, alongside the source, as defined
  in [ADR 0040](0040-open-source.md).

The application neither reads nor modifies Prometheus data automatically. The
temporary importer has been removed. Already-imported boards and transcripts
remain readable. Cleanup accepts an inherited multi-repository worktree only
at the path computed by `paths::prometheus_multi_dir`; it does not move folders
or discover old boards. See [persistence](../contracts/persistence.md).

Prometeu writes only V1 events. The reader supports the historical
`prometheusV1Mirror` and `type: "prometheus"` tokens for existing transcripts,
without emitting a rollback mirror. See
[ADR 0002](0002-canonical-conversation-protocol.md).

## Consequences

Positive:

- both products can be installed and run side by side;
- developing Prometeu does not risk the existing state;
- already-imported data remains readable without accessing the old state root;
- new names do not accidentally carry public contracts.

Negative:

- data from an installation that was never migrated requires an older importer
  or a manual migration;
- `.prometheus` configurations of other repositories must be recreated or
  imported consciously;
- external integrations, signing and release infrastructure need new
  credentials;
- the legacy reader still contains two identifiers with the previous name.

Linear uses Prometeu's own OAuth registration, with Authorization Code, PKCE
and `read,write` scopes. The write grant allows explicit assignment of an
unowned issue from a joined team. Existing read-only credentials remain usable
for discovery and require reauthorization for assignment. Linear's `write`
scope is broader than issue assignment; Prometeu uses it only for an explicit
local claim action and does not ask for admin access. See the
[Linear contract](../contracts/linear-issues.md).

## Evidence

- `paths.rs` tests cover the new roots;
- `scripts.rs` tests cover the new file and variables;
- `branch.ts` tests cover the new prefix;
- conversation tests keep fixtures of the historical tokens;
- the Tauri configuration defines an independent product, binary and bundle id.
- `session.rs::check_allows_only_clean_workspaces_that_were_entered` covers cleanup
  of inherited multi-repository worktrees.
