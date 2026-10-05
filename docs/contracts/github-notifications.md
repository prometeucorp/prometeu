# GitHub notifications

Status: the Cloud side is implemented in `prometeu-cloud`; the desktop consumer
is pending. Decision: [ADR 0088](../decisions/0088-github-app.md) (proposed).

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
idempotent. Each delivery deletes rows older than 30 days, and
`installation.deleted` deletes that installation's rows. Deleting the account
deletes its rows.

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

## Verification

`fixtures/cloud-api.json` (`github_notifications`) is the shared wire fixture;
`prometeu-cloud` imports it and checks the response in
`test/integration/desktop_contract_test.rb`. Webhook signature, recipients,
idempotency, retention and the cursor are covered by
`test/integration/github_webhooks_test.rb` and account linking by
`test/integration/github_login_test.rb` in `prometeu-cloud`.
