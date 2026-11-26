---
type: Decision
title: "ADR 0035 — Authenticated private feedback through the Cloud"
description: "The Cloud receives feedback only from an authenticated account and sends it to a private GitHub repository."
status: stable
---

# ADR 0035 — Authenticated private feedback through the Cloud

Date: 2026-09-10

## Context

To make an issue on GitHub, a token is necessary. GitHub does not let an
anonymous client make an issue. If the client contained that token, each person
who opens the binary gets it. The token gives write and read access to the
private repository. Thus, a server between the client and GitHub is mandatory.

Feedback is a product channel of Prometeu. It is not a public form. The team
must know who sends feedback. Then the team can answer, and the channel does not
become a target for spam.

## Decision

`POST /api/feedback` accepts a request only with a session. The desktop sends
the Bearer of the connected account. A Cloud page sends the signed session
cookie. Without a session, the response is 401. The limit is for each account:
five reports in each hour. This limit replaces the limit for each IP address and
the global quota for each day.

The desktop sends the report through the Rust backend, in the `feedback_send`
command. The cause is that the token of the account is not in the webview since
the ADR for the optional account. The webview assembles the report. The webview
does not get the credential at any time.

If no account is connected, the panel shows a notice, not the form. The
panel also shows the button that starts the same device authorization as the
sidebar. The feedback button stays on the screen. If the application hides the
channel, it also hides the path to the channel.

The endpoint does not answer CORS and preflight requests. The desktop sends
requests without `Origin`, and the Cloud page has the same origin. The endpoint
accepts only `application/json` and does not enable CORS. Thus, a page from a
third party cannot post with the cookie of the person who is logged in.

Before the Cloud sends the content, it examines the configured destination through
GitHub. The destination must be the correct private repository. The default
destination is `prometeucorp/prometeu-cloud`. If this check finds a different destination,
the Cloud does not send the content. The Cloud sends the selected images through the
native `uploads.github.com/user-attachments/assets` endpoint. It uses the numeric
ID of the repository and `Net::HTTP`. When the report has an image, the Cloud
makes the issue only with a correct attachment URL in the body of the issue.
GitHub controls the access to the text and to the images. The Cloud has no
public attachment path and no different list of reviewers.

The client gets only `{ id }`, without an internal URL. SQLite stores a receipt
with the ID, the content hash, the GitHub URLs and the time of each try. The
receipt has no text and no images:

- If a client sends the same ID and content again, the Cloud uses the first
  delivery.
- If a client sends different content with the same ID, the Cloud does not accept it.
- If the result of an issue creation is not known, the operator finds the
  issue on GitHub through the `Feedback: <id>` marker. Then the operator
  corrects the receipt.
  The Cloud does not try again automatically at any time.
- After a failure of an issue creation, the Cloud can use again an attachment
  URL that it recorded before.

The Cloud sends only the typed text and an image that the person selected or
captured. The person must review a capture before the person sends it. The
application does no automatic telemetry and does not upload the transcript.
Feedback is apart from the E2EE collaboration. The Cloud processes the
request, and GitHub stores its content.

## Consequences

A person without an account cannot send feedback. The anonymous widget of the
site cannot use the channel. The bundle continues to be available for authenticated
Cloud pages. The landing page must have a different path if it must get
messages from visitors.

After this change, the team knows the account of each report, through the session of the
request. This data does not go into the table. The receipt continues to have no
content, no image and no identification of a person. The application knows the
relation only during the request.

As before, the person can send a report again because the client keeps the
form. The limit for each account uses the in-memory cache of one Puma process.
If the Cloud has more than one replica, a shared cache is necessary, as for the
other limits of the Cloud.

Reviewers must have access to the private GitHub repository. The team removes
content there. The backup copies of the Cloud data contain only receipts. If an interruption occurs
before the Cloud saves an upload URL, an orphan attachment can stay on GitHub. A
rollback must keep the authenticated submission. It must not put back public
attachments or content storage in the Cloud. A smoke test that is apart from these tests is necessary for
native capture and for delivery to the GitHub service.

## Evidence

`FeedbackTest` in the Cloud includes these items:

- 401 without a session.
- The Bearer of the desktop and the cookie of the browser.
- The limit for each account.
- The check of the private repository.
- The upload and the issue with the marker.
- Idempotency, conflict and uncertain delivery.
- No content in the table.
- The Cloud does not accept an unknown origin.

[`e2e/feedback.spec.ts`](../../e2e/feedback.spec.ts) includes the draft that stays
after an error, and a second submission of the same report. The shortcut in the
panel that connects an account has no browser scenario. The Cloud tests above
make sure that authentication is mandatory.

References: [contract](../contracts/feedback.md),
[optional account](../contracts/cloud-account.md).
