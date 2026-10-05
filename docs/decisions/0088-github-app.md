# ADR 0088 — GitHub App for issues, PRs and notifications

Date: 2026-10-05
Status: Proposed

When accepted, this decision replaces [ADR 0086](0086-github-inbox.md) and
narrows the "no provider credentials" rule of [ADR 0015](0015-cloud-rails.md)
for GitHub webhook metadata. Until each phase ships, ADR 0086 and the current
contracts remain the implemented behavior.

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
- Workspaces whose repository does not have the App installed lose PR status
  and show how to install it. There is no `gh` fallback, so there is one path.

### Issues

The GitHub tab follows the current Linear integration: connect button, **Mine**
(assigned open issues) and **Available** (unassigned open issues) tabs, a
repository filter in place of Linear's team filter, and a claim action that
assigns the issue to the person. Authored PRs and requested reviews remain.
The repository scope comes from the App's installations, so the manual
selection in `github-issues.json` is retired.

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
- Rows expire after 30 days, pruned on insert. `installation.deleted` removes
  that installation's rows. There is no installation table.
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
- Clicking a GitHub notification opens the workspace whose PR matches, or
  expands the row and fetches the comment or review text, with an action to
  open it on GitHub. A matching workspace is also marked unread and its PR
  status refreshed immediately.
- Local notifications ([ADR 0054](0054-local-notifications.md)) gain a
  `github` event, off by default like the others, and the Dock badge counts
  unread notifications.

## Consequences

- The Cloud now handles GitHub payloads from private repositories in memory and
  persists who did what, where and when. It still stores no titles, bodies or
  credentials beyond the existing UID link.
- Installing the App on an organization requires an organization owner.
  Repositories without it produce no issues, PR status or notifications.
- Polling adds up to 30 seconds of latency and one cheap request per Mac.
  Server push (SSE or Action Cable) would hold Puma threads and is deferred.
- People who rely on `gh` must connect once through the device flow.
- Webhook development needs a tunnel to the local Cloud.

## Phases

1. This ADR.
2. Cloud: webhook endpoint, notification table, read API, GitHub linking.
3. Desktop: device-flow credential replacing `gh` in both adapters.
4. Desktop: Linear-style GitHub issues.
5. Desktop: Notifications destination, polling, local notification and badge.

Each phase updates its contracts — [GitHub issues](../contracts/github-issues.md),
[local notifications](../contracts/notifications.md),
[account](../contracts/cloud-account.md) and a new GitHub notifications
contract — in the same change.

## Evidence

Phase 2 is implemented in `prometeu-cloud`; see the
[GitHub notifications contract](../contracts/github-notifications.md) and its
tests. The desktop phases are pending. GitHub documents the device flow, token refresh
without a client secret for device-flow tokens, and the user-token access
intersection under "Generating a user access token for a GitHub App" and
"Refreshing user access tokens".
