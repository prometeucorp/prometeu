# GitHub notifications

Status: current contract, implemented in `prometeu-cloud` and the desktop.
Decision: [ADR 0088](../decisions/0088-github-app.md).

## Source

The GitHub App `prometeu-app`, owned by `prometeucorp`, sends webhooks to
`<cloud>/webhooks/github`. It subscribes to `issues`, `issue_comment`,
`pull_request`, `pull_request_review`, `pull_request_review_comment` and
`workflow_run`; GitHub always delivers `installation`. Only repositories where
the App is installed produce events.

The Cloud authenticates a delivery solely by `X-Hub-Signature-256`, an HMAC
SHA-256 of the raw body with `GITHUB_WEBHOOK_SECRET`. Without that variable the
route answers 404. Invalid signatures answer 401, bodies above 5 MiB answer 413,
and accepted or ignored deliveries answer 204.

## Recipients and kinds

Recipients are Prometeu accounts whose linked GitHub identity
(`identities.uid`) appears in the delivery. People without a linked account
produce no rows. The acting user (`sender`) is excluded, except for CI failures.
One delivery yields at most one row per person; a direct kind wins over a
mention.

| Kind | Event | Recipients |
| --- | --- | --- |
| `review_approved`, `changes_requested`, `commented` | `pull_request_review.submitted` by review state | PR author and assignees |
| `commented` | `issue_comment.created` | Issue or PR author and assignees |
| `review_requested` | `pull_request.review_requested` | Requested user; team requests are ignored |
| `assigned` | `issues.assigned`, `pull_request.assigned` | Assignee |
| `merged`, `closed` | `pull_request.closed` | PR author and assignees |
| `closed` | `issues.closed` | Issue author and assignees |
| `ci_failed` | `workflow_run.completed` with `failure` or `timed_out`, attached to a PR | Triggering actor |
| `mentioned` | `@login` in a review, comment, or the body of an opened issue or PR | Mentioned person |

Inline review comments only produce mentions: GitHub also delivers their
review, which is the single notice. Mentions match the stored login without
case sensitivity. The login is saved on GitHub sign-in or linking and refreshed
whenever a delivery carries that user.

## Storage

`github_notifications` keeps metadata only: recipient, kind, `owner/repository`,
number, subject `pr` or `issue`, actor login, target type and ID, installation
ID, delivery ID and creation time. Titles and bodies are never stored; comment
text is read in memory only to find mentions, and the desktop fetches content
from GitHub when needed. `(delivery, user)` is unique, so redelivery is
idempotent. Retention is 30 days: the feed never returns older rows, and the
Cloud's `github_notifications:prune` task, scheduled daily, deletes them, so
expiry does not depend on new deliveries. `installation.deleted` deletes that
installation's rows. Deleting the account deletes its rows.

## Desktop API

`GET /api/notifications` with the existing desktop Bearer. Without `after`, it
returns the newest 100 rows in ascending ID order. With `after=<id>`, it returns
up to 100 rows with a greater ID; `more: true` means another request is needed.
A malformed cursor is ignored. Read state lives on each Mac; the Cloud has no
read or dismiss route. An invalid Bearer answers 401.

```json
{
  "github": { "login": "fixture-contributor" },
  "notifications": [
    {
      "id": "7001",
      "kind": "review_approved",
      "repository": "example/app",
      "number": 12,
      "subject": "pr",
      "actor": "fixture-reviewer",
      "target": "review",
      "target_id": "9001",
      "url": "https://github.com/example/app/pull/12#pullrequestreview-9001",
      "created_at": "2026-10-05T12:00:00Z"
    }
  ],
  "more": false
}
```

`github` is `null` when the account has no linked GitHub identity, and `login`
may be `null` for links created before logins were stored. `target` is one of
`issue_comment`, `review`, `review_comment`, `workflow_run` or `null`; IDs are
decimal strings. `url` points at the comment, review, PR, issue or workflow run
on github.com.

## Desktop

`src/notification-center.ts` polls `github_feed` every 30 seconds, on window
focus and when the destination opens, following `more` for up to ten pages.
`src-tauri/src/github_notifications.rs` calls the Cloud with the stored Bearer
(the webview never sees it) and returns `null` without an account or with a
rejected session. Rows outside the contract (unknown kind, subject or target,
malformed repository or ID, non-github.com URL) are dropped.

The rail's permanent **Notifications** item replaces the former conditional
Mentions item and its modal sheet. It shows unread GitHub notifications plus
pending relay mentions, and the Dock badge adds the same count to unread
workspaces. The destination merges both sources by time, grouped by day, with
All, GitHub and Mentions filters; mentions keep their relay rule and leave when
the thread is resolved.

`github_feed` adds `account` (`<origin>#<user id>`) naming the Prometeu account
whose credential made the request. When it changes between polls, the desktop
drops the rows, cursor and read marks of the previous account, discards a page
fetched with the old cursor, and treats the new account's first response as
history.

Read state stays on each Mac, per account, in localStorage
`prometeu:github-notifications:read:<account>`: `{ "before": "<id>", "ids": ["<id>"] }`. **Mark all as read** moves `before` to the
newest id; opening a row adds its id (at most 500 are kept). A damaged record
resets read marks only. Up to 300 rows stay in memory; after a restart the first
request without a cursor returns the newest 100.

Titles come from `github_subjects`, one GraphQL request for up to 50 issues or
PRs with variables, through the GitHub App credential; inaccessible items are
omitted and rows fall back to `owner/repo#number`. Titles and text are cached
in memory for one Prometeu account and GitHub login; a switch clears them and
discards requests that finish afterwards, so access is checked again. Opening a row marks it read
and opens the workspace whose GitHub item or PR matches (the clone's GitHub
repository comes from `github_projects`). Otherwise the row expands and
`github_detail` fetches the current comment or review text, or the workflow run
name. Every row can open its `url` on GitHub.

The first feed response after startup or signing in is history. Later arrivals
trigger the `github` local notification when enabled, for at most the three
newest per poll ([local notifications](notifications.md)), mark
a matching workspace unread and, for PRs, refresh its PR state through
`pr_open`.

| Command | Input | Result |
| --- | --- | --- |
| `github_feed` | `{ after: string \| null }` | The feed above plus `account`, or `null` without an account |
| `github_subjects` | `{ keys: { repository, number }[] }` | `{ repository, number, title, state }[]` |
| `github_detail` | `{ repository, number, target, id }` | Current text, empty when absent |

## Verification

`fixtures/cloud-api.json` (`github_notifications`) is the shared wire fixture;
`prometeu-cloud` imports it and checks the response in
`test/integration/desktop_contract_test.rb`. Webhook signature, recipients,
idempotency, retention and the cursor are covered by
`test/integration/github_webhooks_test.rb` and account linking by
`test/integration/github_login_test.rb` in `prometeu-cloud`. On the desktop,
`github_notifications.rs` decodes the shared fixture through the production
type and covers row validation and the batched title query;
`src/notification-feed.test.ts` covers ordering, read state and workspace
matching. The Notifications destination has no browser scenario: its rules are
covered by those unit tests, and E2E scope is reserved for the core journey.
