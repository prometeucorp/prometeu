---
type: Decision
title: "ADR 0015 — Optional account in a separate Rails service"
description: "Optional account in a separate Rails service."
status: stable
---

# ADR 0015 — Optional account in a separate Rails service

Date: 2026-09-06

## Context

The maintainer explicitly asked for Rails, a stack they know and already use in
other projects. The Node/Better Auth prototype was prepared on the VPS but not
published, with no users and no production database. The desktop client already
has a small browser-based connection contract.

## Decision

The separate `prometeu-cloud` project uses Rails 8.1, Ruby 4.0, SQLite and ERB.
Authentication starts from Rails' native generator: `has_secure_password`,
persisted sessions, signed cookies, CSRF protection and Action Mailer. The site
needs no SPA, no Node on the server, no Redis and no job process.

The Prometeu account is optional for local work and is separate from provider
subscriptions. Registration, profile changes and account deletion happen in the
browser. The desktop stores the credential privately in Rust, outside the
webview. The Cloud owns accounts, organizations and portable catalogs; it does
not execute agents or store transcripts or provider credentials. Collaboration
uses the separate relay and its E2EE contract. Authenticated feedback is the
explicit content-upload exception in [ADR 0035](0035-feedback-requires-account.md).

Preserve the four routes used by the desktop: issuing and exchanging the code,
querying the session and logging out. `DeviceGrant` stores only the hash of the
private code, expires in 10 minutes and requires confirmation by the
authenticated person. Issuing the Bearer and consuming the code happen in the
same transaction. The database stores only the Bearer's hash. Sessions expire
after a fixed 30 days; the previous prototype's renewal on activity is not kept.

Registration, editing and deletion use conventional Rails controllers and forms.
The Better Auth site's internal endpoints are not kept, since there are no
published consumers. The IPC, `cloud.json`, relay, board and transcript formats
do not change. Previous test tokens require reconnecting.

## Consequences and rollback

The maintainer can evolve the SaaS with their usual tools. The small device flow
becomes project code, covered by contract and concurrency tests. Encryption and
passwords use Rails/Ruby/bcrypt primitives.

SQLite and in-memory rate limiting assume a single Puma process. Horizontal
scale requires a shared database and shared limits. SMTP delivery is synchronous
with a timeout; a durable queue will be needed only when volume/retries justify
it. SMTP and an HTTPS origin are production requirements.

The Node prototype and its rollback image are no longer maintained. Rails
migrations require a consistent backup and a coordinated rollback of the image,
database and `SECRET_KEY_BASE`. Local work and cached identity remain available
during Cloud outages; an account does not grant automatic sharing consent.

## Evidence

See the [account contract](../contracts/cloud-account.md), `cloud.rs`,
`e2e/cloud.spec.ts` and the `prometeu-cloud` project's Rails integration,
concurrency and browser suites.
