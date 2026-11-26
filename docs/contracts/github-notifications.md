---
type: Contract
title: "GitHub notifications"
description: "The Cloud changes GitHub App webhooks into notifications with only metadata, and the desktop shows them in the Notifications destination."
status: stable
---

# GitHub notifications

This is the current contract. `prometeu-cloud` and the desktop obey it.
Decision: [ADR 0088](../decisions/0088-github-app.md).

The same authenticated webhook endpoint also sends data to the optional
[workspace supervision](supervision.md). That consumer uses explicit workspace
bindings and a durable callback delivery. It does not use the recipient
filters, the read state or the polls of this feed for persons. It can report
CI changes that are correct. To use supervision, enable the other GitHub App
subscriptions that its contract gives. This does not change the notification
types below.

## Source

The GitHub App `prometeu-app`, which `prometeucorp` owns, sends webhooks to
`<cloud>/webhooks/github`. It subscribes to `issues`, `issue_comment`,
`pull_request`, `pull_request_review`, `pull_request_review_comment` and
`workflow_run`. GitHub always sends `installation`. Only the repositories that
have the App installed make events.

The Cloud authenticates a delivery only with `X-Hub-Signature-256`. This value
is an HMAC SHA-256 of the raw body with `GITHUB_WEBHOOK_SECRET`. If that
environment variable is not set, the endpoint answers 404. These are the other answers:

- A signature that is not correct gets 401.
- A body of more than 5 MiB gets 413.
- An accepted or ignored delivery gets 204.

## Recipients and kinds

The recipients are the Prometeu accounts with a linked GitHub identity
(`identities.uid`) that is in the delivery. Persons without a linked account do not
make rows. The Cloud does not send a notification to the user who did the
action (`sender`), but it sends CI failures to that user. One delivery makes a
maximum of one row for each person. If a person gets a mention and a different type, the
row uses the different type.

| Kind | Event | Recipients |
| --- | --- | --- |
| `review_approved`, `changes_requested`, `commented` | `pull_request_review.submitted`, from the review state | The PR author and the assignees |
| `commented` | `issue_comment.created` | The issue or PR author and the assignees |
| `review_requested` | `pull_request.review_requested` | The requested user. The Cloud ignores team requests. |
| `assigned` | `issues.assigned`, `pull_request.assigned` | The assignee |
| `merged`, `closed` | `pull_request.closed` | The PR author and the assignees |
| `closed` | `issues.closed` | The issue author and the assignees |
| `ci_failed` | `workflow_run.completed` with `failure` or `timed_out`, attached to a PR | The actor that started the run |
| `mentioned` | `@login` in a review, a comment, or the body of an opened issue or PR | The mentioned person |

Inline review comments make only mentions. GitHub also sends their review,
which is the single notice. The Cloud compares mentions with the stored login
and ignores the difference between capital letters and small letters. The Cloud
saves the login when the person signs in with GitHub or links GitHub. It updates
the login each time that a delivery contains that user.

## Storage

`github_notifications` keeps only metadata:

- The recipient and the type.
- `owner/repository` and the number.
- The subject, `pr` or `issue`.
- The login of the actor.
- The type and the ID of the target.
- The installation ID and the delivery ID.
- The creation time.

The Cloud does not store titles or bodies. It reads comment text
only in memory to find mentions. The desktop gets the content from GitHub when
it is necessary. `(delivery, user)` is unique. Thus a second delivery of the
same webhook is idempotent.

The Cloud keeps the rows for 30 days. The feed does not return older rows. The `github_notifications:prune` task of the Cloud operates each day
and deletes them. Thus the expiry occurs also when there are no new deliveries.
`installation.deleted` deletes the rows of that installation. When a person
deletes the account, the Cloud deletes its rows.

## Desktop API

`GET /api/notifications` uses the current desktop Bearer token. Without `after`,
it returns the newest 100 rows in the sequence of their IDs, from small to large. With `after=<id>`, it
returns a maximum of 100 rows with a larger ID. `more: true` means that a
different request is necessary. The endpoint ignores a malformed cursor. The
read state is on each Mac. The Cloud does not have an endpoint to read or
dismiss notifications. A Bearer token that is not correct gets 401.

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

`github` is `null` when the account does not have a linked GitHub identity.
`login` can be `null` for links that the Cloud made before it stored logins.
`target` is `issue_comment`, `review`, `review_comment`, `workflow_run` or
`null`. The IDs are decimal strings. `url` points to the comment, review, PR,
issue or workflow run on github.com.

## Desktop

`src/notification-center.ts` polls `github_feed`:

- Each 30 seconds.
- When the window gets the focus.
- When the destination opens.

It follows `more` for a maximum of ten pages.
`src-tauri/src/github_notifications.rs` calls the Cloud with the stored Bearer
token. The webview does not see the token. The command returns `null` if there
is no account or if the Cloud rejected the session. The command discards rows
that do not agree with the contract. These rows have an unknown type, subject or
target, a malformed repository or ID, or a URL that is not on github.com.

The permanent **Notifications** item of the rail replaces the previous
conditional Mentions item and its modal sheet. It shows GitHub issues and PRs
with unread notifications, and the relay mentions that are not resolved. The
Dock badge adds the same count to the unread workspaces. The destination merges
the two sources by time and puts them in groups by day. It has the All, GitHub
and Mentions filters. Mentions keep their relay rule, and they go away when the
thread is resolved.

The destination shows one row for each issue or PR, under the day of its newest
event. If there are unread events, the row shows the most urgent unread event.
This is the sequence of urgency: `ci_failed`, `changes_requested`,
`review_requested`, `mentioned`, `review_approved`, `merged`, `closed`,
`assigned`, `commented`. When all events are read, the row shows the newest
event. The row also shows the count of the other events. The row is unread
while one of its events is unread.

`github_feed` adds `account` (`<origin>#<user id>`). This value identifies the
Prometeu account that gave the credential for the request. If it changes between two
polls, the desktop does these actions:

- It discards the rows, the cursor and the read marks of the previous account.
- It discards a page that it got with the previous cursor.
- It uses the first response of the new account as history.

The read state stays on each Mac, for each account, in the localStorage key
`prometeu:github-notifications:read:<account>`: `{ "before": "<id>", "ids": ["<id>"] }`.
**Mark all as read** moves `before` to the newest id. When a person opens a row,
the desktop adds its id. The desktop keeps a maximum of 500 ids. If the record is damaged,
the desktop sets only the read marks to empty. A maximum of 300 rows stay in
memory. After the app starts again, the first request without a cursor returns the newest 100 rows.

The titles come from `github_subjects`. This command sends one GraphQL request
with GraphQL variables for a maximum of 50 issues or PRs. It uses the credential of the
GitHub App. The response does not include items that the App cannot access.
For these items, the rows show `owner/repo#number`. The desktop keeps the titles
and the text in a memory cache for one Prometeu account and one GitHub login.
When the account or the login changes, the desktop deletes the cache. It also
discards requests that end after the change. Thus the desktop examines the
access again.

When a person opens a row, the desktop marks all events of its issue or PR as
read. Then it opens the issue or PR on GitHub. The row has these actions:

- Open the workspace with the GitHub item or PR of the row. This action
  shows only when such a workspace is there. `github_projects` gives the GitHub
  repository of the clone.
- Expand the events of the row, with the newest first. This action shows when
  the thread has more than one event or has event text. `github_detail` then
  gets the current text of the comment or review, or the name of the workflow
  run.

Comment text and review text render as Markdown. They also render the HTML
subset that GitHub comments use: headings, `details`, links, lists and tables.
The desktop discards other tags, attributes and comments. Links keep only
http(s). Images show their alt text because the CSP blocks remote images. In a
thread with more than one event, each expanded event also opens its own `url`.

The first feed response after the start or after a sign in is history. When
new items come after that, the desktop does these actions:

- It sends the `github` local notification, if it is enabled. It sends one
  notification for each issue or PR, and a maximum of three for each poll. These
  are the three that changed most recently
  ([local notifications](notifications.md)).
- It marks a workspace that agrees with the item as unread.
- For PRs, it refreshes the PR state of the workspace through `pr_open`.

| Command | Input | Result |
| --- | --- | --- |
| `github_feed` | `{ after: string \| null }` | The feed above with `account`, or `null` if there is no account |
| `github_subjects` | `{ keys: { repository, number }[] }` | `{ repository, number, title, state }[]` |
| `github_detail` | `{ repository, number, target, id }` | The current text, or empty if there is no text |

## Verification

`fixtures/cloud-api.json` (`github_notifications`) is the shared fixture for the
network format.
`prometeu-cloud` imports it and examines the response in
`test/integration/desktop_contract_test.rb`. In `prometeu-cloud`,
`test/integration/github_webhooks_test.rb` examines the webhook signature, the
recipients, the idempotency, the expiry of rows and the cursor.
`test/integration/github_login_test.rb` examines the account link.

On the desktop, `github_notifications.rs` decodes the shared fixture through
the production type. It also examines the row validation and the title query for
a batch. `src/notification-feed.test.ts` examines the sequence, the read state,
the groups, the threads that expand and the workspace that agrees with a row. The Notifications
destination does not have a browser scenario. Those unit tests examine its rules,
and the E2E scope is only for the core journey.
