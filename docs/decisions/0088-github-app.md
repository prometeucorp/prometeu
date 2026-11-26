---
type: Decision
title: "ADR 0088 — GitHub App for issues, PRs and notifications"
description: "A GitHub App for issues, PRs and notifications through the Cloud."
status: stable
---

# ADR 0088 — GitHub App for issues, PRs and notifications

Date: 2026-10-05

This decision replaced the GitHub inbox that used the local `gh` credential. It
also makes the "no provider credentials" rule of
[ADR 0015](0015-cloud-rails.md) narrower for GitHub webhook metadata.

## Context

The GitHub integration uses the installed `gh` CLI and its credential.
Periodic `gh pr list` scans find the PR state. The app knows only the number,
the title, the draft flag and the state. Prometeu does not tell the person that
a PR got an approval, that a person wrote a comment or that CI had a failure. The
person must find this on GitHub.

GitHub sends events through webhooks. A webhook must have a public HTTPS
endpoint, and the desktop has no such endpoint. The Cloud has a public origin, a
desktop Bearer and the GitHub numeric ID of accounts that signed in with GitHub
(`identities.uid`). It has no inbound webhook and no push channel to the
desktop. The desktop polls the Cloud at this time.

## Options considered

1. Keep `gh`, and poll the GitHub Notifications API from the desktop. This adds
   no new service, and it includes all repositories. But it does not operate
   from events. It uses the GitHub subscription settings of the person. It does
   not report CI reliably. It also must have a classic token, because the
   endpoint rejects GitHub App tokens and fine-grained tokens.
2. A classic OAuth App with a `repo`-scoped token in the Cloud. The Cloud sends
   all GitHub reads and writes for the desktop. The server stores a credential
   with wide access, and the Rails process with three threads becomes an API
   gateway.
3. One GitHub App. The Cloud receives its webhooks and stores only metadata
   that it gets from them. The desktop gets its own user token through the
   device flow of the App, and it calls GitHub directly.

## Decision

Prometeu selected option 3. One GitHub App, which the `prometeucorp`
organization owns, replaces the `gh` dependency in the GitHub adapters of the
app. The App can see only the repositories that have an installation of the App
and that the person has access to. This one rule applies to issues, PR status and
notifications.

### Desktop credential

- The desktop does the GitHub device flow with the public client ID of the App.
  The binary contains no client secret, and the refresh operates without one.
  Tokens from the device flow refresh with only `client_id` and
  `refresh_token`.
- Access tokens stay correct for 8 hours, and refresh tokens for 6 months. Rust
  stores the pair privately at `<root>/github.json`, as it does `linear.json`.
  The pair does not go through IPC. The Cloud does not see it.
- `github_issues.rs` and the PR scan in `github.rs` call the GitHub REST or GraphQL API
  with this token, not with `gh`. Agents can continue to run `gh` in their own
  terminals. These are the tools of the agent, not of the app.
- If the App is not installed on the repository of a workspace, the workspace
  keeps its last known PR. But it receives no new status. The issue lists and
  the settings have a link to the App installation. There is no `gh` fallback.
  Thus there is only one path.
- Only github.com. Prometeu keeps the lists that load correctly in a cache in
  memory, for each login and scope. It does not save issue bodies.

### Issues

Keep one Issues destination with Linear and GitHub tabs. The GitHub tab
operates as the Linear integration does. It has these items:

- a connect button
- the **Mine** tab (open issues that GitHub gives to the person) and the
  **Available** tab (open issues without an assignee)
- a repository filter, where Linear has a team filter
- a claim action that makes the person the assignee of the issue

The PRs of the person and the requested reviews stay. The repository scope
comes from the installations of the App. Thus Prometeu retired the manual
selection in `github-issues.json`.

The navigation to the source and the creation of a workspace stay two
different actions. The current launcher controls the creation. Prometeu
normalizes the GitHub data before IPC. The data uses a namespace in the current
`IssueRef.id`, so the formats of the board and of the shared data do not change. Prometeu
connects local projects to issues through the canonical repository names from
their Git remotes. To prepare a PR, Prometeu fetches the pull ref of the base
repository into a unique review branch. It does not change the branch of the
original clone. The target branch stays the base of the diff. The PR URL stays
the explicit identity for discovery. Synthetic branches operate with forks and
keep local branches safe. But they do not follow subsequent pushes.

### Cloud webhooks

- `POST /webhooks/github` examines `X-Hub-Signature-256` against the webhook
  secret of the App. It ignores duplicate `X-GitHub-Delivery` IDs. It processes
  the notification metadata during the request. Optional supervisor callbacks
  use the different saved delivery queue of
  [ADR 0106](0106-workspace-supervision.md).
- The recipients are Cloud users with an `identities.uid` in the payload. The
  payload can have that UID for these persons:
  - the PR or issue author
  - an assignee
  - a requested reviewer
  - an `@login` mention

  The user who did the action is not a recipient. For mentions, the GitHub login
  is necessary. The Cloud stores it adjacent to the UID and updates it when it
  sees it.
- Notification types: `review_approved`, `changes_requested`,
  `review_requested`, `commented`, `mentioned`, `assigned`, `merged`, `closed`
  and `ci_failed`. CI uses `workflow_run` failures and tells the actor that
  started the workflow. Thus an installation token is not necessary.
- Each row stores only metadata:
  - the recipient and the type
  - `owner/repository`, the number, and issue or PR
  - the login of the actor
  - the GitHub ID of the comment or review
  - the installation ID, the delivery ID and the time The Cloud does
  not store titles or bodies. It reads comment bodies only in memory to find
  mentions. A person can edit the content on GitHub. The desktop gets it when
  necessary, with its own token.
- Rows expire after 30 days. The feed does not show older rows, and a daily
  scheduled task deletes them. Thus the retention limit applies also when an
  installation stops to receive deliveries. A scheduler entry uses fewer
  resources than a job process. `installation.deleted` removes the rows of that
  installation. There is no installation table.
- A Cloud user who signed in can link GitHub from the account settings. At this
  time, the link occurs only when a person signs in with GitHub and the email that
  GitHub examined agrees.

### Delivery and presentation

- While the person is signed in, the desktop polls
  `GET /api/notifications?after=<id>` with its current Bearer each 30 seconds.
  Each Mac keeps its own read state. The Cloud is a feed with a time limit, not
  an inbox.
- A permanent **Notifications** destination in the rail replaces the
  conditional Mentions item and its modal sheet. It shows GitHub events and
  relay comment mentions together in one chronological list. It has All, GitHub
  and Mentions filters. Mentions keep their current rule: they go away when the
  person resolves the thread.
- When the person clicks a GitHub notification, it opens on GitHub. Row actions
  open the workspace with the same PR. They also expand the row to get the text
  of the comment or review. Prometeu also shows the related workspace as unread
  and refreshes its PR status immediately.
- Local notifications ([ADR 0054](0054-local-notifications.md)) get a `github`
  event under the same master switch. This switch is off at the start. The Dock
  badge counts unread notifications. The first load after the app starts does
  not show a notification.

## Consequences

- With this decision, the Cloud processes GitHub payloads from private repositories in memory.
  It saves who did what, where and when. It continues to store no titles,
  bodies or credentials, other than the current UID link.
- Only an organization owner can install the App on an organization.
  Repositories without the App give no issues, PR status or notifications.
- Polling adds a wait of a maximum of 30 seconds, and one small request for
  each Mac. A server push (SSE or Action Cable) holds Puma threads. Prometeu
  moved it to a subsequent version.
- Persons who used `gh` must connect one time through the device flow.
  Prometeu ignores the retired `github-issues.json`.
- For webhook development, a tunnel to the local Cloud is necessary.
- The desktop and the browser mock contain the commands.

## Evidence

Contracts and tests: [GitHub issues](../contracts/github-issues.md),
[GitHub notifications](../contracts/github-notifications.md),
[local notifications](../contracts/notifications.md) and
[account](../contracts/cloud-account.md), and the `prometeu-cloud` tests for
the webhook, the feed and the link. GitHub documents the device flow, the token
refresh without a client secret for device-flow tokens, and the access rule for
user tokens. The pages are "Generating a user access token for a GitHub App"
and "Refreshing user access tokens". The device flow on github.com, the installation
access and the webhook delivery must have a manual check against github.com.
