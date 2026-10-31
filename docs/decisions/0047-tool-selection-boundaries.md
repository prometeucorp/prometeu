---
type: Decision
title: "ADR 0047 — Bind tool selection to the reviewed declaration and execution provider"
description: "Reviewed tool declarations, effective provider and native IPC patches."
status: stable
---

# ADR 0047 — Bind tool selection to the reviewed declaration and execution provider

Date: 2026-09-15

Partially supersedes [ADR 0045](0045-layered-tool-selection.md) (Trust) and
[ADR 0046](0046-cli-inherited-mcp-base.md) (Discovery). Their other decisions
remain in force.

## Context

The first implementation lost explicit JSON nulls during Tauri argument
parsing, so restoring inheritance did not change native state. It also resolved
MCP inheritance for the workspace provider even when a tab or action selected
another provider. Claude discovery prioritized user configuration over local
configuration and ignored the configured Claude home.

The trust dialog displayed one declaration but approved whichever declaration
was on disk at submission. Reading the hash only at submission could therefore
authorize packages and hooks that the person had not reviewed. The prompt was
also unreachable for declarations containing only removals or an empty
replacement, because neither creates a pending item.

## Decision

- Read axis patches from the native IPC JSON body. An absent key preserves the
  axis; null resets it; an object replaces it. Validate all supplied global axes
  before writing any of them. Persisted selection formats do not change.
- Resolve tools for the effective tab or action-profile provider, before
  materialization. Read-only `workspace_tools` and `mcp_inherited` accept an
  optional `agent` for the displayed tab; absence keeps workspace-default
  behavior. Cache inherited rows by workspace and provider.
- Claude discovery shares the adapter's configuration-file lookup, including
  `CLAUDE_CONFIG_DIR`. Duplicate names use local scope, then project scope
  (nearest ancestor first), then user scope. Hub definitions still shadow
  discovered definitions. This follows the
  [Claude CLI scope contract](https://code.claude.com/docs/en/mcp#scope-hierarchy-and-precedence).
- `project_tools_trust` requires the hash shown by the dialog. The backend
  independently rereads and hashes the declaration, then compares it before
  recording approval or rejection. A mismatch or removed declaration returns
  `err.tools.changed`, leaves previous decisions intact, and requires reopening
  the dialog. No client-supplied hash is stored without comparison.
- Offer the trust prompt based on `project_tools.pending`, independently of
  per-item provenance. Project and workspace menus also expose the declaration
  so empty replacements, rejected decisions, and approved decisions remain
  accessible. The dialog explains replacement semantics as well as additions
  and removals.
- Tool changes still apply only at spawn or resume of a stopped process. Show
  this rule even when the existing process is idle; the next message alone
  does not reload tools.

## Alternatives and consequences

Recomputing the hash without comparing the displayed version was rejected:
backend ownership of the hash does not establish what the person reviewed.
Automatically refreshing and approving a changed declaration was also rejected
because it would give the new declaration the old dialog's consent.

Keeping provider resolution at workspace level would break mixed-provider tabs
and action profiles. Passing the effective provider preserves portable hub
selection while preventing Claude-only IDs from reaching Codex materialization.

The trust command intentionally fails closed for clients that omit the new
hash argument. The desktop and browser mock ship together; no persisted trust
or board migration is required. The discovery arguments are optional additions,
and the axis payload keeps its existing wire shape.

## Verification

- `session.rs::tool_axis_ipc_preserves_absent_null_and_replacement` checks native
  IPC bodies, reset semantics, explicit empty selections and malformed values.
- `session.rs::tool_resolution_uses_the_tab_provider_and_configured_claude_home`
  exercises both providers and materializers with isolated files.
- `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes`
  rejects stale approvals and rejections without changing stored decisions.
- `mcp.rs::local_mcp_shadows_project_and_user_and_materializes_the_same_definition`
  checks duplicate-name precedence through strict configuration generation.
- `e2e/tools.spec.ts` covers an empty declaration, stale dialogs,
  mixed-provider pickers, the inheritance menu and the application-time notice.
