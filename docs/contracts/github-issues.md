# GitHub issues and pull requests

Status: current desktop contract. Decision: [ADR 0086](../decisions/0086-github-inbox.md).

## Discovery and interaction

Issues contains Linear and GitHub provider tabs. GitHub has four scopes:
assigned open issues, open issues in selected repositories, authored open PRs
(including drafts), and open PRs requesting the authenticated user's review.
Opening the GitHub provider fetches all four scopes, so their counts are available
without visiting each tab. Switching scopes reads the loaded lists; reopening
the integration revalidates them through the backend cache, and explicit refresh
updates every scope. Personal scopes span all accessible repositories; the
repository selection only controls the repository scope. Review requests use GitHub's
`review-requested:<login>` search qualifier, including applicable team requests.

Both providers use the same compact search field, scope tabs, refresh button
and filter pills from `src/issues-controls.ts`. Switching provider tabs retains
focus on the selected tab instead of focusing search. Rows use the existing
compact Linear layout. Clicking a row, or pressing Enter
while the row has focus, opens its canonical source URL in the system browser.
The separate Open workspace button reuses a live workspace or opens the existing
launcher with title, description and URL. There is no details panel. Discovery
does not assign issues, post reviews, merge PRs or start agents. Linear's existing
claim action remains separate from workspace creation.

The launcher’s issue picker still lists assigned Linear issues. GitHub items
enter through the GitHub tab. The sidebar counts known assigned issues from both
providers; PRs and repository suggestions are not included. GitHub counts appear
after that scope has been loaded.

## Authentication, storage and limits

`src-tauri/src/github_issues.rs` calls the installed GitHub CLI with its existing
github.com credential. Setup is `gh auth login --hostname github.com`; Prometeu
does not read, copy or persist the token. There is no additional OAuth app or
Cloud service. GitHub Enterprise hosts are not supported by this inbox.
The identity is checked through `gh api user` before each list/cache access,
after all four scope requests settle, and before repository selection writes.
Saving a selection also checks the login that opened
the dialog; an external account switch cannot overwrite another account’s settings.
Changing accounts clears the backend list cache.
Starting a refresh clears displayed lists until their identity is revalidated.
Each batch replaces retained results: successful scopes remain available, while
failed scopes clear their previous results and offer retry. Results with different
account identities, a final identity that differs from any successful scope,
or a failed authentication check are rejected together, even when another
account's searches fail without returning its identity,
preventing lists from two accounts from being combined. Linear failure behavior
remains unchanged.

The REST search query is passed as one argument, never through a shell. Each
subprocess has a 30-second deadline, an 8 MiB stdout limit and a 1 MiB stderr
limit. Each scope fetches up to five pages of 100 items, ordered by update time;
the response marks truncation or incomplete search results. The UI shows a
partial-list notice. Scope searches run concurrently, with network work outside
the cache lock; only settings writes serialize. Successful lists cache in memory
for two minutes per account, scope and repository selection. Late responses from
a previous account cannot populate the current account's cache.
There is no background network polling or persisted issue-body cache.

`<root>/github-issues.json` contains only
`{ "accounts": { "login": ["owner/repository"] } }`. Reads retain errors rather
than replacing a damaged file. Writes use the private atomic file adapter.
At most 20 repositories are accepted, normalized to lowercase and deduplicated.
The selection dialog offers repositories found in registered projects and lets
the person enter other `owner/repository` names. Personal lists are independent
of that selection. Existing installations need no migration; older binaries
ignore this additional file.

## IPC

All commands are registered in `src/ipc.ts`, the desktop handler and the browser
mock. These adapters do not add implementations to the separate WSL runtime.

| Command | Input | Result |
| --- | --- | --- |
| `github_identity` | None | Current authenticated login, checked again after the scope batch |
| `github_issues` | `{ scope, force }` | `{ login, repositories, items, fetched_at, truncated }` |
| `github_repositories` | `{ selected: string[], login }` | Normalized saved repository names |
| `github_issue_open` | `{ url }` | Opens a validated github.com issue or PR URL |
| `github_projects` | None | `{ project, repository }[]` from registered local Git remotes |
| `github_prepare` | `{ project, url }` | `{ base, branch, source }` for an isolated PR workspace |

An item contains `id`, `identifier`, `title`, `url`, nullable `description`,
`repository`, `number`, `kind` (`issue` or `pr`), `author`, `draft`, `updated_at`
and label names. Upstream REST objects stop at the adapter. IDs use
`github:owner/repository/issues/number` or `github:owner/repository/pull/number`.
The existing four-field `IssueRef` persists that identity, title and link; board,
shared snapshot and launcher request formats remain compatible. No provider
field or board migration is required. Old Linear IDs retain their meaning.

External URL opening accepts only canonical HTTPS github.com issue/PR URLs,
without credentials, custom ports, query strings or fragments. Repository and
item-number validation applies again in the backend. Selection is not an
authorization grant: GitHub still authorizes every query through the CLI account.

## Local projects and PR branches

The UI matches canonical GitHub names against registered projects' remotes,
including `upstream` and SSH/HTTPS URLs. If several clones match, the person
chooses one. If none match, the person can add an already-cloned local directory;
the adapter verifies its remote before proceeding. Cloning and SSH aliases are
not inferred. Workspace creation from an issue uses the selected local project
and the normal launcher branch rules.

PR preparation validates the registered project and matching remote, checks
that the PR remains open, and fetches `refs/pull/N/head` from its base repository.
This supports fork PRs even when their head branch is named `main`. A fresh
`github-pr-N-<suffix>` branch is seeded through a private remote-tracking ref;
the target branch remains the separate diff base. Fetch never checks out,
resets or deletes the person's existing clone branches. Cancellation can leave
fetched objects and the preparation ref, but creates no workspace or agent.

The launcher locks the prepared project's repository, source and worktree
selection to keep the reviewed PR and the checked-out code aligned. The usual
workspace preparation checks still run. Opening a live linked workspace reuses
it. PR discovery, external opening and monitoring retain the originating PR URL
instead of guessing from the synthetic local branch. The fetched review branch
does not automatically track subsequent pushes; the person controls subsequent
Git work. The PR action prompts the agent to update the linked URL and verify
its actual head repository and branch before pushing, rather than publish the
isolated local branch as another PR. Opening a workspace never publishes a
GitHub review.

## Evidence

Rust tests in `github_issues.rs` cover search qualifiers, repository/URL validation,
pagination, concurrent searches and late cache writes across account changes,
old `IssueRef` compatibility and fork PR preparation against an
upstream remote. `github.rs` covers explicit PR identity; `session.rs` covers
worktree preparation. `src/github-issues-model.test.ts` covers source preservation,
safe issue branch names, workspace reuse without identity collisions, eager
loading of all scopes, partial failures/retry and account switches during a batch,
including a new account whose searches all fail and a failed final identity check.

`e2e/issues.spec.ts` extends the workspace creation journey: provider keyboard
navigation, source-link versus nested workspace-button event routing, native
dialog focus after asynchronous refresh, and PR source preservation through the
launcher. These are DOM/focus integration risks that pure query/mapping tests
cannot establish. Existing Linear scenarios retain assignment/permission coverage.
The browser mock proves UI behavior, not real GitHub authentication or native
Git networking. GitHub REST search response fields were also checked against a
public repository using the installed CLI.

The feature is independent of Claude, Codex and Antigravity; see the
[provider matrix](../quality/provider-matrix.md). Windows/WSL command support is
listed separately in the [Windows contract](windows-application.md).
