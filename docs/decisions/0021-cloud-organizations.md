---
type: Decision
title: "ADR 0021 — Organizations and invitations in the Cloud"
description: "Organizations and invitations in the Cloud."
status: stable
---

# ADR 0021 — Organizations and invitations in the Cloud

Date: 2026-09-07

Complements the optional account in [ADR 0015](0015-cloud-rails.md) and personal
catalog in [ADR 0020](0020-personal-catalog-and-local-items.md).

## Context

Team creation and enrollment existed on the desktop, through a shared secret.
The Cloud had only a static organization preview. The product needs to manage
organizations in the browser, require invitation acceptance by email and share
both live workspaces and tool catalogs.

## Decision

The organization in Rails becomes the authority over identity, memberships,
roles and catalogs. We do not introduce a second team layer. We use existing
Active Record, SQLite constraints, signed tokens and Action Mailer. Catalogs
reuse the revisioned document and the portability validation; sharing between
catalogs is an explicit, independent copy.

Single-use individual tickets authenticate 60-second leases, with a Cloud query
in the handshake and each renewal. Compatible clients renew on the same socket
without repeating snapshots when the roster is unchanged; reconnection handles
connection loss or peers without lease renewal. See
[ADR 0034](0034-mobile-pairing-continuity.md). This avoids custom JWT signing and
a revocation webhook. Traffic stops after the deadline even when the Durable
Object's alarm is late. Content uses the v4 encrypted boundary from
[ADR 0022](0022-end-to-end-encryption.md).

Consent to send a workspace includes the organization and the membership.
Switching context does not send old work to another group. Legacy teams keep
their namespace and credentials during the transition; selecting an organization
keeps a local backup. Anonymous identities are not converted into inferred
emails.

## Consequences

The desktop stays useful without an account. Institutional collaboration
requires the Cloud to be available to renew authorization. Revocation has a
60-second limit. Organizations support the same 64 members as the relay.
Personal catalogs stay synchronized as before. Organization definitions are
available directly in the desktop hubs. Installation creates a local copy
without automatically subscribing to later edits; a personal catalog copy is
optional. See [ADR 0039](0039-organization-catalog-on-desktop.md).
Received code is never activated just by accepting an invitation.

A rollback keeps the expanded schema and the legacy storage. Publication follows
Cloud, relay, desktop; there is no implicit destructive contraction. Contracts,
configuration, limits and evidence are in
[cloud-organizations.md](../contracts/cloud-organizations.md).
