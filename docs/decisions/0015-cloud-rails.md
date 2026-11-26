---
type: Decision
title: "ADR 0015 — Optional account in a separate Rails service"
description: "The optional Prometeu account is in a different Rails service."
status: stable
---

# ADR 0015 — Optional account in a separate Rails service

Date: 2026-09-06

## Context

The maintainer selected Rails. The maintainer knows this stack and uses it in
other projects. The team prepared a Node/Better Auth prototype on the VPS, but
did not publish it. The prototype had no users and no production database. The
desktop client has a small connection contract that operates through the
browser.

## Decision

The `prometeu-cloud` project is a different repository. It uses Rails 8.1, Ruby
4.0, SQLite and ERB. Authentication starts from the native generator of Rails:
`has_secure_password`, stored sessions, signed cookies, CSRF protection and
Action Mailer. The site does not use an SPA, Node on the server or Redis.
Optional workspace supervision adds a stored SQLite delivery queue and one
worker in the same process. [ADR 0106](0106-workspace-supervision.md) specifies
them. The account and catalog operations do not use that worker.

The Prometeu account is optional for local work. It is different from the
subscriptions of the providers. A person makes the account, changes the profile
and deletes the account in the browser. The desktop stores the credential
privately in Rust, not in the webview. The Cloud keeps the accounts,
organizations and portable catalogs. It does not operate agents, and it does not
store transcripts or provider credentials. Collaboration uses the relay, which
is a different service, and its E2EE contract. Authenticated feedback is the
only content upload that is explicitly permitted, in [ADR
0035](0035-feedback-requires-account.md).

Keep the four endpoints that the desktop uses. These endpoints issue the code,
give a token for the code, read the session and log out. `DeviceGrant` stores
only the hash of the private code. The code expires after 10 minutes, and the
authenticated person must accept it. The service issues the Bearer and uses the
code in the same transaction. The database stores only the hash of the Bearer.
Sessions expire after a constant period of 30 days. The service does not keep
the renewal on activity of the previous prototype.

Registration, changes and deletion use usual Rails controllers and forms. The
service does not keep the internal endpoints of the Better Auth site, because no
published consumers use them. The IPC, `cloud.json`, the relay, the board and
the transcript formats do not change. A person must connect again after a test
token from the prototype.

## Consequences and rollback

The maintainer can continue to work on the SaaS with the usual tools. The small
device flow becomes project code, and contract tests and concurrency tests cover
it. Encryption and passwords use the primitives of Rails, Ruby and bcrypt.

SQLite and rate limits in memory operate correctly only with one Puma process.
Horizontal scale makes a shared database and shared limits necessary. SMTP
delivery is synchronous and has a timeout. The supervision delivery queue does
not change the mail delivery. Production must have SMTP and an HTTPS origin.

The team stopped work on the Node prototype and its rollback image. Before a
Rails migration, make a complete and correct copy of the data. A rollback must
change the image, the database and `SECRET_KEY_BASE` together. When the Cloud is
not available, local work and the identity in the cache stay available. An
account does not give automatic consent to share.

## Evidence

See the [account contract](../contracts/cloud-account.md), `cloud.rs`,
`e2e/cloud.spec.ts`, and the integration, concurrency and browser suites of the
`prometeu-cloud` project.
