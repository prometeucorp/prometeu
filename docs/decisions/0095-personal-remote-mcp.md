# ADR 0095 — Personal remote MCP through Prometeu Cloud

Status: accepted.

## Context

Cloud-hosted personal agents cannot reach local stdio. They need a public,
authenticated MCP endpoint and completion/input notifications while execution,
workspaces and provider credentials remain on the Mac.

## Decision

Add an opt-in Rust WSS connector, personal-device Durable Objects in the existing
Worker, and OAuth/device/grant management in Rails. Reuse the local MCP dispatcher,
project authorization, delegation ownership, request keys and desktop publication.
No organization is required. The Mac opens the connection using one-use tickets;
the Worker never receives its desktop account Bearer.

Calls return delegation identities quickly. Reject offline work; uncertain calls
keep their original request keys. Metadata events use a durable local outbox,
gateway cursor feed/SSE, and optional signed webhooks with durable Rails receipts.

## Trade-offs

- The gateway sees tool traffic. Generic OAuth MCP clients cannot transparently
  participate in collaboration E2EE. Consent discloses this separate trust boundary;
  Rails still receives no transcripts or provider credentials.
- A native WebSocket dependency keeps connectivity independent of the frontend.
  Long calls and SSE do not occupy Rails's three Puma threads.
- MCP `2025-06-18` is the compatibility baseline. Experimental Tasks and vendor wake
  APIs are not required. Vendor applications still need independent certification.
- Revocation applies to new requests, with bounded connection leases. Already
  accepted work remains visible and can be interrupted in the app.
- No daemon or wake-on-LAN promise. Mac suspension prevents local execution.
- Notifications are at-least-once and bounded. Gaps require reconciliation; receipt
  does not imply a third-party agent resumed. Webhook recovery requires a schedule.

See the [contract](../contracts/remote-mcp.md) for limits, rollout, rollback and tests.
