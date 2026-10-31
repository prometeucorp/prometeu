---
type: Contract
title: "Public issues and private feedback"
description: "Public GitHub issues or account-backed private reports with text and images."
status: stable
---

# Public issues and private feedback

Implemented on the desktop, in the Cloud and in the web bundle. Enabling
it in production depends on configuring the GitHub credential in the Cloud.
Decision: [ADR 0035](../decisions/0035-feedback-requires-account.md).

The widget first offers a link to create a public issue in
`prometeucorp/prometeu`. The fixed link opens GitHub without copying the private
form's description or image and remains available without a Prometeu account.
The person reviews and publishes the issue through GitHub.

The private form offers Problem, Idea and Other, a description, an optional
image selected or dropped onto the panel, and a capture started by the person.
The form states that the report will be handled privately by the Prometeu team
and asks for a review of sensitive data before sending. No transcript,
workspace path, email, credential or navigation URL is collected automatically.
Captures may contain such data: the person reviews the thumbnail and can remove
or replace the image.

## HTTP

`POST /api/feedback` in the Cloud requires a connected account and accepts JSON:

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

The session comes from the account's Bearer on the desktop or from the signed
cookie on a Cloud page. Without a session the response is 401 and nothing is
sent to GitHub.

`id` is a UUID v4. `kind` accepts `problem`, `idea`, `other`; `source` accepts
`desktop`, `site`, `cloud`. The required description allows up to 4,000
characters; the optional version up to 40. The optional image accepts PNG, JPEG
or WebP up to 5 MiB, with strict base64 and a matching signature. The complete
body has a 7 MiB limit, applied by Puma before the Rails parser. Smaller desktop
endpoint limits remain in their controllers. The Cloud's production smoke test
checks that a request containing a 5 MiB image reaches authentication and that
Puma rejects an oversized request.

There is no CORS and no preflight. An origin other than the public one receives
403, and requiring `application/json` without enabling CORS prevents a
third-party page from posting with the cookie of whoever is logged in. The
desktop delivers through the Rust backend, without `Origin`, and the account
credential never enters the webview.

Success: `201 { id }`. The private issue's URL stays on the server. The client
shows a confirmation without a link to the internal repository.
Errors: 401 without a session; 403 for a foreign origin; 400/415 for the format;
413 for size; 422 for invalid content; 409 for reuse of an ID with different
content or for uncertain delivery; 429 for the rate limit; 503 for an
unavailable service/credential. No error clears the form. A new attempt with the
same content uses the same ID.

The Cloud forwards the text and image to GitHub, without persisting that
content. The `feedbacks` table stores only the ID, the content's SHA-256, the
GitHub URLs of the attachment and the issue, the timestamp of the issue-creation
attempt and timestamps. A completed ID returns the same receipt without creating
another issue. Different content with the same ID receives 409, including when
only the image changes.

The receipt is refreshed and claimed under a short database lock before the
issue-creation POST. Concurrent retries either see the completed receipt or
receive `uncertain`; only the claimant sends the issue. GitHub calls run outside
the database transaction. `test/models/feedback_concurrency_test.rb` in the Cloud
checks both single delivery and the transaction boundary with a stub transport.

Before sending any content, `GET /repos/prometeucorp/prometeu-cloud` must
confirm `private: true` and the expected name. A query failure or a public
repository returns 503. The image uses the numeric ID from that response in
`POST https://uploads.github.com/user-attachments/assets?name=...&content_type=...&repository_id=...`:
binary bytes, `Content-Type: application/octet-stream`, the server's Bearer.
That is the endpoint used by
[GitHub CLI 2.99.0](https://github.com/cli/cli/blob/v2.99.0/internal/attachments/client.go).
The response must contain a `https://github.com/user-attachments/assets/<uuid>`
URL. It goes into the issue body as a Markdown image; text and image are seen on
GitHub, with the private repository's permissions. There is no attachment route
and no reviewer list in the Cloud. There is no dependency on the `gh` executable
on the server.

`FEEDBACK_GITHUB_TOKEN` stays exclusively in the Cloud, with Issues: write
limited to the destination repository, `prometeucorp/prometeu-cloud` by default
and `FEEDBACK_GITHUB_REPO` when it is another one. The PAT's owner needs write
access to the repository to attach images. The client never receives that
credential. Without the token the endpoint answers 503 without sending anything.
The response returns only a receipt, without text, image or internal URL.
Limit: 5 submissions per hour per account, counting refused attempts. The
counter assumes a single Puma process; multiple replicas require a shared cache.
The table does not record who sent it: the account exists only during the
request.

An upload failure prevents creating the issue. A new attempt preserves the form
and reuses an already-recorded attachment URL, if there is one. An interruption
before receiving or saving the URL may leave an attachment without an issue on
GitHub and require a new upload. An interrupted creation POST may have been
accepted: the Cloud preserves the receipt and answers
`409 { error: "uncertain", id }`, without repeating that POST automatically. The
operator searches for `Feedback: <id>` on GitHub and reconciles `issue_url` in
the receipt. Only after confirming that the issue is absent may they clear
`attempted_at` and allow the form to be resent. There is no copy of the content
in the Cloud for reprocessing.

Removing reports and images happens on GitHub. Cloud backups contain only
receipts, without text or images. The content is private through access control,
not E2EE; the Cloud processes the request and GitHub stores the content.

## Capture and interface

`feedback_capture`, with no arguments, is an additive IPC command: it returns a
base64 PNG or `null` on cancellation. On the Mac, `screencapture -i -W` allows
selecting the window; the file stays in a private temporary directory and is
removed when it finishes. Failures use i18n. The mock returns a fictional image;
it does not capture the computer.

Tauri intercepts desktop file drops before HTML receives them. A drop over the
open form routes its first path through the additive `feedback_image` IPC
command. The command reads at most 5 MiB plus one byte, verifies the PNG, JPEG or
WebP signature and returns the file name, media type and base64 bytes. Other
drop targets keep their existing behavior.

While an image is loading, submission is disabled and the form reports
`aria-busy`. A newer attachment selection replaces the pending load; results
and errors from older loads are ignored. Removing the image, starting a capture,
closing the panel or destroying the widget also invalidates pending loads.
The last accepted image and the draft survive a loading failure. Sending requires
another explicit action after loading finishes; it is never queued automatically.

In the browser, `getDisplayMedia` offers surface selection when available. The
tracks are stopped after the capture, including on error. Browsers without that
API keep the upload. The widget is hidden during the capture, comes back with a
thumbnail and never sends automatically.

Without a connected account, the panel replaces the form with a notice and the
button that starts the same device authorization as the sidebar. The state is
checked on every opening: after connecting, reopening shows the form.

The portable composition in `packages/design-system/src/feedback.ts` receives
texts, the optional public issue URL and callbacks, including `blocked` for that
notice. The client in `src/feedback-client.ts` owns the resend identity and the
transport: `fetch` in the browser, with a same-origin cookie, and
`feedback_send` on the desktop.
The manual popover uses the top layer; inside a modal, it moves to that modal to
stay interactive. Escape closes the widget first. On the desktop, the "Feedback"
button sits on the right of the sidebar footer; the panel opens above that
footer only after the click. The native Run preview uses its whole area and is
hidden while the panel is open.

`npm run build:feedback` produces `dist-feedback/feedback.js` and
`feedback.css`. The bundle serves authenticated Cloud pages, which imports both
from `app/assets/feedback/`; on an anonymous page the submission receives 401.
Consumers version the generated files and serve them locally. The bundle's
tokens stay in the widget, preserving the page's tokens.

## Compatibility and evidence

An additive SQLite migration; desktop state, transcripts, account and relay do
not change. The previous feedback prototype was never published. Its migration
was adjusted before publication to create only receipts, without content
columns. Local prototype databases are not migrated or deleted automatically;
tests use a new, disposable database. The `POST /api/feedback` body and the
`{ id }` receipt stay the same; what changed is the session requirement,
answered with 401. `feedback_send` is an additive IPC command and does not
change existing commands. Claude and Codex use the same interface; their CLIs
do not take part in the delivery.

- [`e2e/feedback.spec.ts`](../../e2e/feedback.spec.ts): a recoverable error, retry, upload,
  thumbnail, simulated capture, modal and narrow viewport over the mock;
  deferred native loads cover submission blocking, out-of-order results/errors,
  replacement and closing the panel. Other replacement/cancellation controls
  share the same invalidation path and have no separate browser scenarios.
- The Cloud's `FeedbackTest` tests: 401 without a session, Bearer and cookie,
  per-account limit, foreign origin, format, signature, creation, idempotency,
  ambiguous failure, repository privacy, native upload, retry and the absence of
  content in SQLite; GitHub replaced by an in-memory transport.
- Native capture, macOS permissions and real GitHub delivery require a manual
  smoke test. The tests do not publish issues and do not capture personal data.
