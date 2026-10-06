# GitHub issues and pull requests

Status: current desktop contract. Decision: [ADR 0088](../decisions/0088-github-app.md).

## Connection

GitHub access goes through the Prometeu GitHub App (`prometeu-app`, owned by
`prometeucorp`), not the `gh` CLI. **Settings › Integrations › GitHub** runs
GitHub's device flow in `src-tauri/src/github_auth.rs`:

1. The backend requests a device code with the App's public client ID and opens
   GitHub's verification page in the system browser.
2. The settings row shows the code the person types on github.com, with actions
   to reopen the page or cancel. A second connection attempt is refused while
   one is waiting.
3. After approval, the backend reads `/user`, stores the credential and emits
   the `github` status event. Cancelling at any point, including during the
   token exchange, stores nothing.

`<root>/github.json` holds the access token, refresh token, expiry and login,
written privately. The token never crosses IPC and the Cloud never receives it.
Access tokens last 8 hours and refresh with only the client ID, a property of
device-flow tokens; refreshes are serialized because refresh tokens are single
use. A rejected refresh deletes the file, so the person connects again.
Disconnecting deletes the file; it does not revoke the GitHub authorization,
which the person manages on github.com. Only github.com is supported.

The token reaches only repositories where the App is installed and the person
has access. The settings row and the empty lists link to the App's installation
page. Agents may still run `gh` in their own terminals; the app does not.

## Discovery and interaction

Issues contains Linear and GitHub provider tabs. GitHub has four scopes, all
open items: **My issues** (assigned to the connected login), **Available**
(unassigned issues in installed repositories), **My PRs** (authored, including
drafts) and **To review** (requested reviews, including applicable team
requests). Opening the provider fetches all four scopes so every tab has its
count. Explicit refresh updates every scope. A list fetched while the person
reconnected as another login is rejected with `err.github.accountChanged`
instead of being shown under the new account.

Available searches `no:assignee` once per installation account and keeps only
repositories listed by the installations, since an installation can select
some repositories. Its rows offer **Claim**, which assigns the issue to the
connected login after checking it is still open, unassigned and not a PR.
GitHub has no conditional assignment and silently ignores people without
triage access, so the result must contain the login; otherwise the claim fails.
A claim clears the list cache and moves the row to My issues.

Both providers use the same compact search field, scope tabs, refresh button
and filter pills from `src/issues-controls.ts`. Rows reuse the Linear layout.
Clicking a row, or Enter on a focused row, opens its github.com URL. **Open
workspace** reuses a live workspace or opens the launcher with title,
description and URL. There is no details panel. Discovery does not post
reviews, merge PRs or start agents.

Without a connection the pane offers to connect. With no installed repository
it offers to install the App. The launcher's issue picker still lists assigned
Linear issues; GitHub items enter through this tab. The sidebar counts assigned
issues from both providers, not PRs.

## Limits and cache

REST search runs with a 30-second deadline per request and at most five pages
of 100 items per query, ordered by update time; the response marks truncation
or incomplete results and the UI shows a partial-list notice. Installations
list up to five pages of 100 repositories each. Responses above 16 MiB are
rejected. A rate-limited response reports that GitHub asked to slow down.

Lists cache in memory for two minutes per login and scope. Connecting,
disconnecting and claiming advance a generation, so a response that started
before cannot repopulate the cache. Scope searches run concurrently, outside the
cache lock. A failed scope keeps the others; a lost connection, or lists from two
logins in one batch, rejects the whole batch. There is no persisted issue-body
cache and no background polling of issues.

The retired `<root>/github-issues.json` (manual repository selection) is no
longer read or written; older files are left in place and ignored.

## Pull request status on workspaces

`src-tauri/src/github.rs` reads PR state through the same credential. For each
clone, the base repository is the `upstream` remote when present, otherwise
`origin`; branches are matched with `head=<origin owner>:<branch>`. A clone scan
lists up to 60 PRs (all states, newest first) and asks per branch when the
clone has more. A workspace whose primary repository came from a GitHub PR URL
reads that PR directly. `state` maps to `OPEN`, `CLOSED` or `MERGED`, and the
lifecycle timestamps feed local telemetry.

Without a connection, or for repositories where the App is not installed,
requests fail and the board keeps its last known PR: empty or failed results
never erase a PR. Opening a PR uses the persisted number or resolves the
worktree branch, then opens `https://github.com/<base>/pull/<number>`.

## IPC

All commands are registered in `src/ipc.ts`, the desktop handler and the browser
mock. These adapters do not add implementations to the separate WSL runtime.

| Command | Input | Result |
| --- | --- | --- |
| `github_status` | None | `{ connected, login, busy, code, url, install_url }` |
| `github_connect` | None | Runs the device flow; resolves with the final status |
| `github_disconnect` | None | Cancels a waiting flow, or forgets the credential |
| `github_issues` | `{ scope }` with `mine`, `available`, `authored` or `reviews`, and `force` | `{ login, repositories, items, fetched_at, truncated }` |
| `github_claim` | `{ url }` of an issue | The assigned item |
| `github_issue_open` | `{ url }` | Opens a validated github.com issue or PR URL |
| `github_projects` | None | `{ project, repository }[]` from registered local Git remotes |
| `github_prepare` | `{ project, url }` | `{ base, branch, source }` for an isolated PR workspace |

The `github` event carries the same status as `github_status`. `repositories`
lists the installed repositories. An item contains `id`, `identifier`, `title`,
`url`, nullable `description`, `repository`, `number`, `kind` (`issue` or `pr`),
`author`, `draft`, `updated_at` and label names. Upstream REST objects stop at
the adapter. IDs use `github:owner/repository/issues/number` or
`github:owner/repository/pull/number`. The existing four-field `IssueRef`
persists that identity, title and link; board, shared snapshot and launcher
request formats remain compatible. Old Linear IDs retain their meaning.

External URL opening accepts only canonical HTTPS github.com issue/PR URLs,
without credentials, custom ports, query strings or fragments. Repository and
item-number validation applies again in the backend.

## Local projects and PR branches

The UI matches canonical GitHub names against registered projects' remotes,
including `upstream` and SSH/HTTPS URLs. If several clones match, the person
chooses one. If none match, the person can add an already-cloned local directory;
the adapter verifies its remote before proceeding. Cloning and SSH aliases are
not inferred. Workspace creation from an issue uses the selected local project
and the normal launcher branch rules.

PR preparation validates the registered project and matching remote, checks
through the API that the PR remains open, and fetches `refs/pull/N/head` from
its base repository with the clone's Git credentials. This supports fork PRs
even when their head branch is named `main`. A fresh `github-pr-N-<suffix>`
branch is seeded through a private remote-tracking ref; the target branch
remains the separate diff base. Fetch never checks out, resets or deletes the
person's existing clone branches. Cancellation can leave fetched objects and
the preparation ref, but creates no workspace or agent.

The launcher locks the prepared project's repository, source and worktree
selection to keep the reviewed PR and the checked-out code aligned. The usual
workspace preparation checks still run. Opening a live linked workspace reuses
it. PR discovery and external opening retain the originating PR URL
instead of guessing from the synthetic local branch. The fetched review branch
does not automatically track subsequent pushes; the person controls subsequent
Git work. The PR action prompts the agent to update the linked URL and verify
its actual head repository and branch before pushing, rather than publish the
isolated local branch as another PR. Opening a workspace never publishes a
GitHub review.

## Evidence

Rust tests in `github_auth.rs` cover credential refresh and login validation;
`github_issues.rs` covers search qualifiers, repository/URL validation, claim
preconditions, the per-login cache and late writes after an account change, old
`IssueRef` compatibility and fork PR preparation against an upstream remote;
`github.rs` covers the REST PR mapping, scan gating and PR preservation;
`session.rs` covers worktree preparation. `src/github-issues-model.test.ts`
covers source preservation, safe issue branch names, workspace reuse without
identity collisions, eager loading of all scopes, partial failures and retry,
and batches spanning a lost connection or a different login.

`e2e/issues.spec.ts` extends the workspace creation journey: provider keyboard
navigation, source-link versus nested workspace-button event routing, claiming
from Available, and PR source preservation through the launcher. These are
DOM/focus integration risks that pure query and mapping tests cannot establish.
The browser mock proves UI behavior, not real GitHub authorization, the device
flow or native Git networking; those need a manual check against github.com.

The feature is independent of Claude, Codex and Antigravity; see the
[provider matrix](../quality/provider-matrix.md). Windows/WSL command support is
listed separately in the [Windows contract](windows-application.md).
