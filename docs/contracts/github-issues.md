---
type: Contract
title: "GitHub issues and pull requests"
description: "The GitHub App device flow, assigned and available issues with claim, authored PRs, requested reviews, the PR status and isolated workspaces."
status: stable
---

# GitHub issues and pull requests

This is the current desktop contract. Decision:
[ADR 0088](../decisions/0088-github-app.md).

A provider that is not connected shows a compact connection card. The scope
tabs, the filters and the column headings stay hidden until there is a
connection. Column headings show only when there are tasks for them. The
keyboard navigation does not go to hidden controls.

## Connection

GitHub access goes through the Prometeu GitHub App (`prometeu-app`, which
`prometeucorp` owns). It does not use the `gh` CLI.
**Settings › Integrations › GitHub** starts the GitHub device flow in
`src-tauri/src/github_auth.rs`:

1. The backend requests a device code with the public client ID of the App.
   It opens the GitHub verification page in the system browser.
2. The settings row shows the code that the person types on github.com. The row
   has actions to open the page again or to cancel. While one connection waits,
   the backend refuses a second connection.
3. After the approval, the backend reads `/user` and stores the credential. Then
   it sends the `github` status event. A cancel at a point in the flow does not
   store data. This includes the token request.

`<root>/github.json` contains the access token, the refresh token, the expiry
and the login. The backend writes this file privately. The token does not go
through IPC, and the Cloud does not receive it. An access token expires after 8
hours. The backend refreshes them with only the client ID, because device-flow
tokens let it do this. The backend does refreshes one at a time, because a
person can use a refresh token one time only. If GitHub rejects a refresh, the
backend deletes the file, and the person must connect again.

A disconnection deletes the file. It does not revoke the GitHub authorization.
The person controls that authorization on github.com. The application operates
only with github.com.

The token can access a repository only if the App is installed there and the
person has access. The settings row and the empty lists have a link to the
installation page of the App. Agents can continue to run `gh` in their own
terminals. The app does not run it.

## Discovery and interaction

Tasks contains the Linear and GitHub provider tabs. GitHub has four scopes, and
all of them show only open items:

- **My issues**: issues that are assigned to the connected login.
- **Available**: issues without an assignee in installed repositories.
- **My PRs**: PRs that the login wrote, which include drafts.
- **To review**: requested reviews, which include applicable team requests.

When the provider opens, the app gets all four scopes. Thus each tab has its
count. An explicit refresh updates all scopes. A person can connect again with a
different login while the app gets a list. If this occurs, the app rejects the
list with `err.github.accountChanged`. It does not show the list under the new
account.

Available searches `no:assignee` one time for each installation account. It
keeps only the repositories that the installations list, because an
installation can select only some repositories. Each row has **Claim**. Claim
first makes sure that the issue is open, has no assignee and is not a PR. Then
it assigns the issue to the connected login.

GitHub does not have a conditional assignment. It also ignores persons without
triage access and does not show an error. Thus the result must contain the
login. If it does not, the claim has a failure. A claim deletes the list cache
and moves the row to My issues.

The two providers use the shared search, scope tabs, refresh buttons and
dropdown filters from `src/issues-controls.ts`. The Desktop toolbar contains
these controls. GitHub uses the repository and label filters together with the
text search. The text search includes the author and the labels. The filters
apply to the loaded scope. A person can clear all filters with one action.

The rows have the same labeled columns as Linear. Below the title, they show
the repository, the author and the labels. Narrow layouts keep the title and
the workspace action. A click on a row, or Enter on a row with the focus, opens
its github.com URL. **Open workspace** uses a live workspace again. **Start**
opens the launcher with the title, the description and the URL. There is no
panel with more data. The discovery does not post reviews, merge PRs or start agents.

If there is no connection, the pane shows the connect action. If there is no
installed repository, it shows the action to install the App. The issue picker
of the launcher continues to list assigned Linear issues. GitHub items come in
through this tab. The sidebar counts the assigned issues from the two
providers. It does not count PRs.

## Limits and cache

REST search has a limit of 30 seconds for each request. Each query gets a
maximum of five pages of 100 items, in the sequence of the update time. The
response tells if the list was cut or if the results are not complete. Then the
UI shows a notice that the list is not complete. The installations list a
maximum of five pages of 100 repositories each. The backend rejects responses
of more than 16 MiB. A rate-limited response reports that GitHub told the app
to send fewer requests.

The lists stay in a memory cache for two minutes for each login and scope.
Connect, disconnect and claim increase a generation number. Thus a response
that started before cannot fill the cache again. The scope searches operate at
the same time, and they do not hold the cache lock. If one scope has a failure, the other
scopes stay. If the connection stops, or if one batch has lists from two
logins, the backend rejects the full batch. There is no stored cache of issue
bodies and no background polls of issues.

The backend does not read or write the retired `<root>/github-issues.json`
(manual repository selection). It does not delete older files, and it ignores
them.

## Pull request status on workspaces

`src-tauri/src/github.rs` reads the PR state through the same credential. For
each clone, the base repository is the `upstream` remote if it is there.
If not, the base repository is `origin`. The backend finds the branches with
`head=<origin owner>:<branch>`. A clone scan lists a maximum of 60 PRs, in all
states, with the newest first. If the clone has more, the scan sends a request for
each branch. If the primary repository of a workspace came from a GitHub PR
URL, the workspace reads that PR directly. `state` changes to `OPEN`, `CLOSED` or `MERGED`. The
lifecycle timestamps go to the local telemetry.

For open PRs, the same refresh reads the CI rollup of the head commit. It uses
one GraphQL `nodes(ids:)` request for each 100 PRs (`statusCheckRollup.state`:
`SUCCESS`, `FAILURE`, `ERROR`, `PENDING` or `EXPECTED`). The board keeps the
result in the optional `checks` field of the PR. The field is not there in these
conditions:

- The commit has no checks.
- The read has a failure.
- The App does not have the read permissions for Checks and Commit statuses.
- The PR is closed.

The value is not live. It refreshes:

- During the general scan.
- When the workspace opens.
- After an agent turn, a maximum of one time in 20 seconds.
- When the PR menu opens.

The PR button shows the most urgent state in the color of its icon: a failure,
checks that run, or a success. The menu shows the state for each repository.

If there is no connection, or if the App is not installed for a repository,
the requests have a failure. Then the board keeps its last known PR. An empty
result or a failure does not erase a PR. To open a PR, the app uses
the stored number, or it finds the PR from the worktree branch. Then it opens
`https://github.com/<base>/pull/<number>`.

## IPC

`src/ipc.ts`, the desktop handler and the browser mock contain all commands.

| Command | Input | Result |
| --- | --- | --- |
| `github_status` | None | `{ connected, login, busy, code, url, install_url }` |
| `github_connect` | None | Starts the device flow. It resolves with the last status. |
| `github_disconnect` | None | Cancels a flow that waits, or deletes the credential. |
| `github_issues` | `{ scope }` with `mine`, `available`, `authored` or `reviews`, and `force` | `{ login, repositories, items, fetched_at, truncated }` |
| `github_claim` | `{ url }` of an issue | The assigned item |
| `github_issue_open` | `{ url }` | Opens a validated github.com issue URL or PR URL. |
| `github_projects` | None | `{ project, repository }[]` from the Git remotes of the local projects |
| `github_prepare` | `{ project, url }` | `{ base, branch, source }` for an isolated PR workspace |
| `github_repositories` | None | Up to 100 `owner/name` values from `/user/repos`, with the most recent push first |

The `github` event sends the same status as `github_status`. `repositories`
lists the installed repositories. An item contains these fields: `id`,
`identifier`, `title`, `url`, the nullable `description`, `repository`,
`number`, `kind` (`issue` or `pr`), `author`, `draft`, `updated_at` and the
label names. The upstream REST objects stay in the adapter.

The IDs use `github:owner/repository/issues/number` or
`github:owner/repository/pull/number`. The current `IssueRef` with four fields
keeps that identity, the title and the link. The request formats of the board,
the shared snapshot and the launcher stay compatible. Linear IDs from previous versions keep their
meaning.

The app opens an external URL only if it is a canonical HTTPS github.com issue
URL or PR URL. The URL must not have credentials, custom ports, query strings
or fragments. The backend validates the repository and the item number again.

## Local projects and PR branches

**Projects › Import from GitHub** shows `github_repositories` as suggestions
for the source. The list contains only the repositories where the App is
installed and the person has access. The person can type a different source.
If GitHub is not connected, the dialog shows no suggestions and continues to
operate. The clone uses `clone_project` and the Git credentials of the
computer. See the [IPC contract](ipc.md).

The UI compares canonical GitHub names with the remotes of the projects on
the board. This includes `upstream` and SSH or HTTPS URLs. If more than one clone
agrees, the person selects one. If no clone agrees, the person can add a local
folder that has a clone of the repository. The adapter examines its remote
before it continues. The app does not guess clones or SSH aliases. A workspace
that the app makes from an issue uses the selected local project and the usual
branch rules of the launcher.

The PR preparation does these steps:

1. It validates the project on the board and the remote that agrees with it.
2. It makes sure through the API that the PR is open.
3. It fetches `refs/pull/N/head` from the base repository with the Git
   credentials of the clone.

Thus the preparation operates with PRs from forks, also when their head branch
has the name `main`. The preparation makes a new `github-pr-N-<suffix>` branch
from a private remote-tracking ref. The target branch stays a different diff
base. The fetch does not change or delete the current clone branches of the
person. A cancel can keep fetched objects and the preparation ref. It does not make a workspace or an agent.

The launcher locks the repository, the source and the worktree selection of the
prepared project. Thus the reviewed PR and the checked-out code stay aligned.
The usual workspace preparation checks continue to run. When a person opens a
live linked workspace, the app uses it again. The PR discovery and the external
link keep the URL of the initial PR. They do not guess the PR from the
synthetic local branch.

The fetched review branch does not follow subsequent pushes automatically. The
person controls the subsequent Git work. The PR action tells the agent to
update the linked URL. It also tells the agent to make sure of the head
repository and branch that the PR uses before a push. The agent must not publish the isolated
local branch as a different PR. When a person opens a workspace, the app does
not publish a GitHub review.

## Evidence

The Rust tests examine these areas:

- `github_auth.rs`: the credential refresh and the login validation.
- `github_issues.rs`: the search qualifiers, the validation of repositories and
  URLs, the conditions for a claim and the cache for each login. It also tests
  late writes after an account change, the compatibility of previous `IssueRef`
  values and the preparation of fork PRs against an upstream remote.
- `github.rs`: the REST PR map, the CI rollup map, the scan limits and the
  preservation of PRs.
- `session.rs`: the worktree preparation.

`src/github-issues-model.test.ts` examines these areas:

- The preservation of the source.
- Safe branch names for issues.
- Workspaces that the app uses again without identity collisions.
- The load of all scopes at the start.
- Failures of some scopes and the retry.
- Batches that include a stopped connection or a different login.

`e2e/issues.spec.ts` adds to the journey that makes a workspace. It examines the
keyboard navigation of the provider and the event routing between the source
link and the nested workspace button. It also examines the claim from Available
and the preservation of the PR source through the launcher. The same journey
examines the focus after a menu closes and the actions that show in the narrow
layout. `src/issues-filter.test.ts` examines the search and the filters together.

These are DOM and focus integration risks. Pure query and map tests cannot
find them. The browser mock proves the UI behavior. It does not prove the
GitHub authorization on the service, the device flow or the native Git network operations.
These items must have a manual check against github.com.

The feature does not change between Claude, Codex and Antigravity. See the
[provider matrix](../quality/provider-matrix.md).
