---
type: Decision
title: "ADR 0021 — Organizations and invitations in the Cloud"
description: "Prometeu Cloud owns organizations, memberships and invitations."
status: stable
---

# ADR 0021 — Organizations and invitations in the Cloud

Date: 2026-09-07

This ADR adds to the optional account in [ADR 0015](0015-cloud-rails.md) and the
personal catalog in [ADR 0020](0020-personal-catalog-and-local-items.md).

## Context

The desktop made teams and added members through a shared secret. The Cloud had
only a static preview of an organization. The product must manage organizations
in the browser. A person must accept an invitation by email. Organizations must
share live workspaces and tool catalogs.

## Decision

The organization in Rails becomes the authority for identity, memberships, roles
and catalogs. We do not add a second team layer. We use the current Active
Record, SQLite constraints, signed tokens and Action Mailer. Catalogs use the
document with revisions and the portability validation again. To share between
catalogs is an explicit copy that does not change with its source.

Single-use individual tickets authenticate leases of 60 seconds. The relay sends
a query to the Cloud in the handshake and each time that a client extends the
lease. Compatible clients extend the lease on the same socket. They do not send
snapshots again when the roster did not change. A new connection is necessary
when the connection stops or when a peer cannot extend a lease. See [ADR
0034](0034-mobile-pairing-continuity.md). With this method, custom JWT
signatures and a webhook for revocation are not necessary. Traffic stops after
the deadline also when the alarm of the Durable Object is late. Content uses the
v4 encrypted boundary from [ADR 0022](0022-end-to-end-encryption.md).

The consent to send a workspace includes the organization and the membership.
When a person changes the context, the desktop does not send previous work to a
different group. Legacy teams keep their namespace and credentials during the
change. When a person selects an organization, the desktop keeps a local copy of
the previous state. The Cloud does not change anonymous identities into email
addresses that it guesses.

## Consequences

The desktop stays useful without an account. For collaboration in an
organization, the Cloud must be available to extend the authorization.
Revocation occurs in a maximum of 60 seconds. An organization can have the same
64 members as the relay. Personal catalogs continue to go to all desktops as
before. Organization definitions are available directly in the hubs of the
desktop. An installation makes a local copy. Changes that the organization makes
after that do not go to the copy. A copy in the personal catalog is optional.
See [ADR 0039](0039-organization-catalog-on-desktop.md). Received code does not
become active only because a person accepts an invitation.

A rollback keeps the expanded schema and the legacy storage. Publication occurs
in this sequence: Cloud, relay, desktop. There is no implicit contraction that
deletes data. The contracts, configuration, limits and evidence are in
[cloud-organizations.md](../contracts/cloud-organizations.md).
