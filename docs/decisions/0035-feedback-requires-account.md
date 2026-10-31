---
type: Decision
title: "ADR 0035 — Authenticated private feedback through the Cloud"
description: "Authenticated private feedback through the Cloud."
status: stable
---

# ADR 0035 — Authenticated private feedback through the Cloud

Date: 2026-09-10

## Context

Creating an issue on GitHub requires a token, and there is no anonymous
creation. Distributing that token in the client would hand it to anyone who
opened the binary, with write and read access to the private repository. The
intermediary is mandatory.

Feedback is a Prometeu product channel, not a public form. Whoever submits must
be identifiable so the team can answer and so the channel does not become a spam
target.

## Decision

`POST /api/feedback` requires a session: the desktop sends the connected
account's Bearer and a Cloud page sends the signed session cookie. Without a
session, 401. The limit becomes per account, five reports per hour, replacing
the per-IP limit and the global daily quota.

The desktop delivers through the Rust backend, in the `feedback_send` command,
because the account's token has lived outside the webview since the optional
account ADR. The webview assembles the report and never sees the credential.

Without a connected account, the panel replaces the form with a notice and the
button that starts the same device authorization as the sidebar. The feedback
button stays visible: hiding the channel also hides the path to using it.

The endpoint stops answering CORS and preflight. The desktop arrives without
`Origin` and the Cloud page is same-origin; requiring `application/json` without
enabling CORS prevents a third-party page from posting with the cookie of
whoever is logged in.

Before forwarding any content, the Cloud checks through GitHub that the
configured destination is the expected private repository, defaulting to
`prometeucorp/prometeu-cloud`. Failure prevents delivery. Chosen images are sent
through the native `uploads.github.com/user-attachments/assets` endpoint using
the repository's numeric ID and `Net::HTTP`. When an image is supplied, issue
creation requires a valid attachment URL to embed in its body. GitHub controls access to
both text and images; there is no public attachment route or separate reviewer
list in the Cloud.

The client receives only `{ id }`, without an internal URL. SQLite stores a
receipt with the ID, content hash, GitHub URLs and attempt timestamps, without
text or images. Repeating the same ID and content reuses delivery; different
content with the same ID is rejected. An uncertain issue creation requires
reconciliation through the `Feedback: <id>` marker, never an automatic retry.
An attachment URL already recorded can be reused after a failed issue creation.

Only typed text and an explicitly chosen or captured image are sent. Captures
require review before submission; there is no automatic telemetry or transcript
upload. Feedback is separate from E2EE collaboration: the Cloud processes the
request and GitHub stores its content.

## Consequences

Whoever has no account cannot submit feedback. The site's anonymous widget loses
the channel: the bundle still exists for authenticated Cloud pages, and the
landing page needs another route if it wants to hear from visitors.

The team now knows which account each report came from, through the request's
session. That does not enter the table: the receipt still has no content, no
image and no identification of a person. The association exists only during the
request.

Resending depends on the form preserved in the client, as before. The per-account
limit uses the in-memory cache of one Puma process; multiple replicas require a
shared cache, like the Cloud's other limits.

Reviewers need access to the private GitHub repository. Content removal happens
there; Cloud backups contain only receipts. An interruption before an upload URL
is saved can leave an orphan attachment. A rollback must preserve authenticated
submission and must not restore public attachments or content storage in the
Cloud. Native capture and real GitHub delivery require a separate smoke test.

## Evidence

`FeedbackTest` in the Cloud covers 401 without a session, the desktop's Bearer,
the browser's cookie, the per-account limit, the private repository check, the
upload, the issue with the marker, idempotency, conflict, uncertain delivery,
the absence of content in the table and the refusal of a foreign origin.
[`e2e/feedback.spec.ts`](../../e2e/feedback.spec.ts) covers the draft preserved
on error and resending the same report. The panel's account-connection shortcut
has no dedicated browser scenario; authentication is enforced by the Cloud
tests above.

References: [contract](../contracts/feedback.md),
[optional account](../contracts/cloud-account.md).
