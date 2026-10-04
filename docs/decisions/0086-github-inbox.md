# ADR 0086: GitHub discovery inside Issues

Status: accepted.

## Decision

Keep a single Issues destination with Linear and GitHub tabs. Preserve Linear's
claim flow and compact rows. GitHub adds assigned issues, selected-repository
issues, authored PRs and requested reviews. Source navigation and workspace
creation remain separate actions; the existing launcher owns creation.

Use the installed `gh` credential and bounded CLI subprocesses at the native
GitHub adapter. Limit this inbox to github.com. Store repository preferences
privately per account and cache successful search results only in memory.
Normalize GitHub data before IPC and namespace the existing `IssueRef.id`,
preserving the board and sharing formats.

Match local projects using canonical repository names from their Git remotes.
Prepare PRs by fetching the base repository's pull ref into a unique review
branch, without switching the original clone. Preserve the target branch as the
diff base and the PR URL as explicit identity for discovery and monitoring.

## Rationale and trade-offs

The existing GitHub adapter already depends on `gh`. Reusing it avoids another
token store, OAuth client and service, at the cost of requiring CLI installation
and login. Provider differences remain at the adapter and feature boundaries;
the launcher retains one workspace creation path.

Separate account/scoped caches avoid mixing personal and repository lists.
Opening the integration requests all four scopes so every tab has its count
without being visited. Bounded pagination keeps subprocess work finite and
reports partial results. Failed scopes drop their previous results while
successfully revalidated scopes remain available. Mixed account identities or
authentication failure clear the whole batch, trading offline list continuity
for avoiding stale private content after an account switch outside the app.

Synthetic PR branches support forks and protect local branches from accidental
reset. They require an explicit PR URL rather than branch-name discovery and do
not automatically follow future PR pushes. The person retains control of Git
updates and review publication. Canceled preparation may leave fetched refs.

The desktop and browser mock implement the commands. This decision does not
silently expand the separate Windows/WSL runtime's supported command set.

## Compatibility and verification

No existing persisted field changes. Older binaries ignore `github-issues.json`
and can read the existing issue reference shape. Existing Linear behavior and
agent-provider behavior remain compatible. Contracts, command signatures,
limits and tests are in [GitHub issues](../contracts/github-issues.md).
