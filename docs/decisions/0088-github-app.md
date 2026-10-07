# ADR 0088 — GitHub App for issues, PRs and notifications

Date: 2026-10-05
Status: Accepted

This decision replaced the GitHub inbox through the local `gh` credential and
narrows the "no provider credentials" rule of [ADR 0015](0015-cloud-rails.md)
for GitHub webhook metadata.

## Context

The GitHub integration depends on the installed `gh` CLI and its credential.
PR state is discovered by periodic `gh pr list` scans, and the app knows only
number, title, draft flag and state. Nothing tells the person that a PR was
approved, that someone commented or that CI failed; they find out on GitHub.

GitHub delivers events through webhooks, which need a public HTTPS endpoint.
The desktop has none. The Cloud already has a public origin, a desktop Bearer
and the GitHub numeric ID of accounts that signed in with GitHub
(`identities.uid`). It has no inbound webhook and no push channel to the
desktop; the desktop already polls it.

## Options considered

1. Keep `gh` and poll GitHub's Notifications API from the desktop. No new
   service, covers every repository, but is not event-driven, depends on the
   person's GitHub subscription settings, does not report CI reliably and
   requires a classic token (the endpoint rejects GitHub App and fine-grained
   tokens).
2. A classic OAuth App whose `repo`-scoped token lives in the Cloud, which
   proxies all GitHub reads and writes. Broad credential stored on the server,
   and the three-thread Rails process becomes an API gateway.
3. One GitHub App: the Cloud receives its webhooks and stores only derived
   metadata; the desktop obtains its own user token through the App's device
   flow and calls GitHub directly.

## Decision

Adopt option 3. A single GitHub App, owned by the `prometeucorp` organization,
replaces the `gh` dependency in the app's GitHub adapters. Its reach is the
intersection of the repositories where it is installed and the repositories
the person can access. That one rule applies to issues, PR status and
notifications.

### Desktop credential

- The desktop runs GitHub's device flow with the App's public client ID. No
  client secret ships in the binary or is needed for refresh: tokens issued by
  the device flow refresh with `client_id` and `refresh_token` only.
- Access tokens last 8 hours and refresh tokens 6 months. The pair is stored
  privately in Rust at `<root>/github.json`, like `linear.json`, and never
  crosses IPC. The Cloud never sees it.
- `github_issues.rs` and the PR scan in `github.rs` call the REST/GraphQL API
  with this token instead of `gh`. Agents may still run `gh` inside their own
  terminals; that is the agent's tooling, not the app's.
- Workspaces whose repository does not have the App installed keep their last
  known PR but receive no new status; the issue lists and settings link to the
  App installation. There is no `gh` fallback, so there is one path.
- github.com only. Successful lists are cached in memory per login and scope;
  no issue bodies are persisted.

### Issues

Keep one Issues destination with Linear and GitHub tabs. The GitHub tab follows
the Linear integration: connect button, **Mine** (assigned open issues) and
**Available** (unassigned open issues) tabs, a repository filter in place of
Linear's team filter, and a claim action that assigns the issue to the person.
Authored PRs and requested reviews remain. The repository scope comes from the
App's installations, so the manual selection in `github-issues.json` is retired.

Source navigation and workspace creation stay separate actions; the existing
launcher owns creation. GitHub data is normalized before IPC and namespaces the
existing `IssueRef.id`, preserving the board and sharing formats. Local projects
are matched by canonical repository names from their Git remotes. PRs are
prepared by fetching the base repository's pull ref into a unique review branch
without switching the original clone; the target branch stays the diff base and
the PR URL stays the explicit identity for discovery. Synthetic branches support
forks and protect local branches, at the cost of not following later pushes.

### Cloud webhooks

- `POST /webhooks/github` verifies `X-Hub-Signature-256` against the App's
  webhook secret, ignores duplicate `X-GitHub-Delivery` IDs and processes the
  event inside the request, since the Cloud has no job process.
- Recipients are Cloud users whose `identities.uid` appears in the payload as
  the PR or issue author, an assignee, a requested reviewer or an `@login`
  mention. The acting user is excluded. Mentions require the GitHub login,
  stored beside the UID and refreshed when seen.
- Notification kinds: `review_approved`, `changes_requested`,
  `review_requested`, `commented`, `mentioned`, `assigned`, `merged`, `closed`
  and `ci_failed`. CI uses `workflow_run` failures and notifies the triggering
  actor, so no installation token is needed.
- Each row stores only metadata: recipient, kind, `owner/repository`, number,
  issue or PR, actor login, the GitHub ID of the comment or review, installation
  ID, delivery ID and time. Titles and bodies are never stored; comment bodies
  are read in memory only to detect mentions. Content can be edited on GitHub,
  and the desktop fetches it on demand with its own token.
- Rows expire after 30 days: the feed hides older rows and a daily scheduled
  task deletes them, so retention holds even when an installation stops
  receiving deliveries. A scheduler entry is lighter than a job process.
  `installation.deleted` removes that installation's rows. There is no
  installation table.
- A signed-in Cloud user can link GitHub from account settings; today linking
  only happens on GitHub sign-in by matching the verified email.

### Delivery and presentation

- The desktop polls `GET /api/notifications?after=<id>` with its existing
  Bearer every 30 seconds while signed in. Read state is local to each Mac; the
  Cloud is a time-limited feed, not an inbox.
- A permanent **Notifications** destination in the rail replaces the
  conditional Mentions item and its modal sheet. It merges GitHub events and
  relay comment mentions in one chronological list with All, GitHub and
  Mentions filters. Mentions keep their current rule: they leave when the
  thread is resolved.
- Clicking a GitHub notification opens it on GitHub. Row actions open the
  workspace whose PR matches and expand the row to fetch the comment or review
  text. A matching workspace is also marked unread and its PR status refreshed
  immediately.
- Local notifications ([ADR 0054](0054-local-notifications.md)) gain a
  `github` event under the same master switch, which starts disabled, and the
  Dock badge counts unread notifications. The first load after startup never
  notifies.

## Consequences

- The Cloud now handles GitHub payloads from private repositories in memory and
  persists who did what, where and when. It still stores no titles, bodies or
  credentials beyond the existing UID link.
- Installing the App on an organization requires an organization owner.
  Repositories without it produce no issues, PR status or notifications.
- Polling adds up to 30 seconds of latency and one cheap request per Mac.
  Server push (SSE or Action Cable) would hold Puma threads and is deferred.
- People who relied on `gh` must connect once through the device flow; the
  retired `github-issues.json` is ignored.
- Webhook development needs a tunnel to the local Cloud.
- The desktop and the browser mock implement the commands. This decision does
  not expand the separate Windows/WSL runtime's supported command set.

## Evidence

Contracts and tests: [GitHub issues](../contracts/github-issues.md),
[GitHub notifications](../contracts/github-notifications.md),
[local notifications](../contracts/notifications.md) and
[account](../contracts/cloud-account.md), plus the `prometeu-cloud` webhook,
feed and linking tests. GitHub documents the device flow, token refresh
without a client secret for device-flow tokens, and the user-token access
intersection under "Generating a user access token for a GitHub App" and
"Refreshing user access tokens". The real device flow, installation reach and
webhook delivery need a manual check against github.com.
