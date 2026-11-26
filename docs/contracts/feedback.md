---
type: Contract
title: "Public issues and private feedback"
description: "Public GitHub issues, or private reports with text and images that a Prometeu Cloud account sends."
status: stable
---

# Public issues and private feedback

The implementation is complete on the desktop, in the Cloud and in the web
bundle. To enable it in production, configure the GitHub credential in the
Cloud. Decision: [ADR 0035](../decisions/0035-feedback-requires-account.md).

The widget first shows a link to make a public issue in
`prometeucorp/prometeu`. This link does not change, and it opens GitHub. It does not copy the
description or the image of the private form. The link stays available without
a Prometeu account. The person examines the issue and publishes it on GitHub.

The private form has these items:

- The Problem, Idea and Other options.
- A description.
- An optional image. The person selects the image or puts it on the panel with
  a drag.
- A capture that the person starts.

The form tells the person that the Prometeu team will process the report
privately. It also tells the person to examine the report for sensitive data
before the person sends it. The form does not collect a transcript, a workspace
path, an email, a credential or a navigation URL automatically. A capture can
contain such data. The person examines the thumbnail and can remove or replace
the image.

## HTTP

`POST /api/feedback` in the Cloud accepts JSON. It must have a connected account:

```json
{
  "id": "2c65b70e-80a8-408d-8831-18346584fee4",
  "kind": "problem",
  "source": "desktop",
  "version": "0.6.1",
  "description": "The window does not open.",
  "image": { "type": "image/png", "data": "base64" }
}
```

On the desktop, the session comes from the Bearer token of the account. On a
Cloud page, it comes from the signed cookie. If there is no session, the
response is 401 and the Cloud does not send data to GitHub.

`id` is a UUID v4. `kind` accepts `problem`, `idea` and `other`. `source`
accepts `desktop`, `site` and `cloud`. These limits apply:

- The description is mandatory and has a maximum of 4,000 characters.
- The version is optional and has a maximum of 40 characters.
- The image is optional. It can be PNG, JPEG or WebP, with a maximum of 5 MiB.
  It must have strict base64 and a signature that agrees with its type.
- The full body has a limit of 7 MiB. Puma applies this limit before the Rails
  parser.

The controllers of the desktop endpoints keep their smaller limits. The
production smoke test of the Cloud makes sure of two items. A request with an
image of 5 MiB gets to the authentication. Puma rejects a request that is too
large.

There is no CORS and no preflight. An origin that is not the public origin
receives 403. The endpoint accepts only `application/json` and does not enable
CORS. Thus a third-party page cannot send a request with the cookie of the
person that is logged in at that time. The desktop sends the request through the Rust
backend, without `Origin`. The account credential does not go into the webview


The response for a correct request is `201 { id }`. The URL of the private
issue stays on the server. The client shows a confirmation without a link to
the internal repository. These are the errors:

| Status | Cause |
| --- | --- |
| 401 | There is no session. |
| 403 | The origin is not the public origin. |
| 400, 415 | The format is not correct. |
| 413 | The request is too large. |
| 422 | The content is not correct. |
| 409 | The ID has different content, or the delivery is not sure. |
| 429 | The rate limit. |
| 503 | The service or the credential is not available. |

An error does not clear the form. If the person tries again with the same
content, the client uses the same ID.

The Cloud sends the text and the image to GitHub. It does not keep that
content. The `feedbacks` table stores only these items:

- The ID.
- The SHA-256 of the content.
- The GitHub URLs of the attachment and of the issue.
- The time when the Cloud tried to make the issue, and the other timestamps.

A completed ID returns the same receipt and does not make a second issue. If
the content with the same ID is different, the response is 409. This also
applies when only the image is different.

Before the POST that makes the issue, the Cloud reads the receipt again and
claims it under a short database lock. A retry that operates at the same time
sees the completed receipt or receives `uncertain`. Only the claimant sends the
issue. The GitHub calls occur after the database transaction, not in it. In the Cloud,
`test/models/feedback_concurrency_test.rb` examines the single delivery and the
transaction boundary with a stub transport.

Before the Cloud sends content, `GET /repos/prometeucorp/prometeu-cloud` must
show `private: true` and the correct name. If the query has a failure or the
repository is public, the response is 503. The upload of the image uses the
numeric ID from that response in
`POST https://uploads.github.com/user-attachments/assets?name=...&content_type=...&repository_id=...`.
The request has these items:

- The binary bytes.
- `Content-Type: application/octet-stream`.
- The Bearer token of the server.

[GitHub CLI 2.99.0](https://github.com/cli/cli/blob/v2.99.0/internal/attachments/client.go)
uses the same endpoint. The response must contain a
`https://github.com/user-attachments/assets/<uuid>` URL. The Cloud puts this URL
in the issue body as a Markdown image. A person can see the text and the image
on GitHub with the permissions of the private repository. The Cloud does not
have an attachment endpoint or a list of reviewers. The server does not use the
`gh` executable.

`FEEDBACK_GITHUB_TOKEN` stays only in the Cloud. It has Issues: write access
only for the destination repository. The default repository is
`prometeucorp/prometeu-cloud`. `FEEDBACK_GITHUB_REPO` sets a different
repository. The owner of the PAT must have write access to the repository to
attach images. The client does not receive that credential at any time. If
there is no token, the endpoint answers 503 and does not send data.

The response returns only a receipt, without text, image or internal URL. The
limit is 5 submissions each hour for each account, and refused submissions count.
The counter is correct only for one Puma process. More than one replica must
have a shared cache. The table does not record who sent the report. The Cloud
knows the account only during the request.

An upload failure prevents the creation of the issue. When the person tries
again, the form stays the same. The Cloud uses the attachment URL again if it
recorded it before. An interruption before the Cloud receives or saves the URL
can keep an attachment on GitHub without an issue. Then a new upload is
necessary.

It is possible that GitHub accepted a creation POST that had an interruption.
In this condition, the Cloud keeps the receipt and answers
`409 { error: "uncertain", id }`. It does not send that POST again
automatically. To find the result, the operator does these steps:

1. Look for `Feedback: <id>` on GitHub.
2. If the issue is there, write its URL in `issue_url` of the receipt.
3. If the issue is not there, clear `attempted_at` to let the person send the
   form again.

The Cloud does not keep a copy of the content to process it again.

To remove reports and images, use GitHub. The backup copies of the Cloud contain only
receipts, without text or images. Access control keeps the content private, not
E2EE. The Cloud processes the request and GitHub stores the content.

## Capture and interface

`feedback_capture` has no arguments. It is an additive IPC command. It returns a
base64 PNG, or `null` when the person cancels. On the Mac,
`screencapture -i -W` lets the person select the window. The file stays in a
private temporary folder, and the command deletes it at the end. The failure
messages use i18n. The mock returns a fictional image. It does not capture the
computer.

Tauri gets the files that a person drags on the desktop before HTML receives
them. When a person drags files to the open form, the additive `feedback_image` IPC command
receives the first path. The command reads a maximum of 5 MiB plus one byte. It
examines the PNG, JPEG or WebP signature, and returns the file name, the media
type and the base64 bytes. The other targets for a drag keep their current behavior.

While an image loads, the submission is disabled and the form reports
`aria-busy`. A newer attachment selection replaces the load that is not
complete. The form ignores the results and errors of older loads. These actions
also cancel the loads that are not complete:

- The person removes the image.
- The person starts a capture.
- The person closes the panel.
- The application removes the widget.

The last accepted image and the draft stay after a load failure. When the load
is complete, the person must do a new explicit action to send. The form does not
put the send in a queue automatically.

In the browser, `getDisplayMedia` lets the person select a surface when it is
available. The media tracks stop after the capture, also after an error. Browsers
without that API keep the upload. The widget is hidden during the capture. It
comes back with a thumbnail and does not send automatically.

If there is no connected account, the panel shows a notice and a button. The
button starts the same device authorization as the sidebar. The panel examines
the state each time it opens. After the person connects, the panel shows the
form when it opens again.

The portable composition in `packages/design-system/src/feedback.ts` receives
these items:

- The texts.
- The optional URL for public issues.
- The callbacks, which include `blocked` for that notice.

The client in `src/feedback-client.ts` owns the identity for a new send and the
transport. In the browser, the transport is `fetch` with a same-origin cookie.
On the desktop, it is `feedback_send`.

The manual popover uses the top layer. In a modal, it moves to that modal and
stays interactive. Escape closes the widget first. On the desktop, the
"Feedback" button is on the right side of the footer of the sidebar. It is
before an update action on the next row. The keyboard focus follows the same
sequence. The panel opens above that footer only after the click. The native
Run preview uses its full area and is hidden while the panel is open.

`npm run build:feedback` makes `dist-feedback/feedback.js` and `feedback.css`.
The bundle is for authenticated Cloud pages, which import the two files from
`app/assets/feedback/`. On an anonymous page, the submission receives 401. The
consumers keep the files that the build makes in version control. They supply
the files from their own server.
The tokens of the bundle stay in the widget. Thus the tokens of the page do not
change.

## Compatibility and evidence

The change adds one SQLite migration. The desktop state, the transcripts, the
account and the relay do not change. The team did not publish the previous
feedback prototype. Before the publication, the team changed its migration to
make only receipts, without content columns. The application does not migrate
or delete local prototype databases automatically. The tests use a new
database that they discard.

The `POST /api/feedback` body and the `{ id }` receipt stay the same. The only
change is that a session is necessary, and the answer without a session is 401.
`feedback_send` is an additive IPC command and does not change the current
commands. Claude and Codex use the same interface. Their CLIs do not have a
function in the delivery.

- [`e2e/feedback.spec.ts`](../../e2e/feedback.spec.ts) uses the mock. It
  examines an error with a retry, the upload, the thumbnail, a simulated
  capture, a modal and a narrow viewport. Native loads that the test holds
  examine the blocked submission, results and errors in an incorrect sequence,
  the replacement and the closed panel. The other replacement and cancel
  controls use the same cancel path. They do not have their own browser
  scenarios.
- The `FeedbackTest` tests in the Cloud examine 401 without a session, Bearer
  and cookie, and the limit for each account. They examine an origin that is not
  public, the format, the signature, the creation and the idempotency. They also examine an
  ambiguous failure, the privacy of the repository, the native upload, the retry
  and that SQLite has no content. An in-memory transport replaces GitHub.
- Native capture, macOS permissions and the delivery to the GitHub service
  must have a manual smoke test. The tests do not publish issues and do not capture
  personal data.
