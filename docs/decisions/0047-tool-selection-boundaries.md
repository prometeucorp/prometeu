---
type: Decision
title: "ADR 0047 — Bind tool selection to the reviewed declaration and execution provider"
description: "Tool declarations that the person examined, the active provider and native IPC patches."
status: stable
---

# ADR 0047 — Bind tool selection to the reviewed declaration and execution provider

Date: 2026-09-15

This ADR replaces one section of [ADR 0045](0045-layered-tool-selection.md)
(Trust) and one section of [ADR 0046](0046-cli-inherited-mcp-base.md)
(Discovery). The other decisions of these ADRs stay in effect.

## Context

The first implementation removed JSON null values when Tauri parsed the
arguments. Thus, when the person set a value back to the inherited value, the
native state did not change. The implementation also resolved the MCP
inheritance for the provider of the workspace, also when a tab or an action
selected a different provider. Claude discovery used the user configuration
before the local configuration. It also ignored the Claude home folder
in the configuration.

The trust dialog showed one declaration. But at submission it approved the
declaration that was on the disk at that time. Prometeu read the hash only at
submission. Thus the approval can include packages and hooks that the person
did not examine. Also, the prompt did not show for some declarations. A
declaration with only removals or with an empty replacement does not make an
item that waits for approval.

## Decision

- Read the axis patches from the JSON body of the native IPC. If a key is
  missing, the axis does not change. A null value sets the axis back to the inherited value. An object
  replaces the axis. Validate all the global axes in the request before
  Prometeu writes one of them. The saved selection formats do not change.
- Resolve the tools for the active provider of the tab or of the action
  profile, before Prometeu writes the configuration. The read-only commands
  `workspace_tools` and `mcp_inherited` accept an optional `agent` for the tab
  on the screen. Without this argument, they use the default of the workspace. Keep
  the inherited rows in the cache for each workspace and provider.
- Claude discovery uses the same function as the adapter to find the
  configuration file. This function includes `CLAUDE_CONFIG_DIR`. When two
  definitions have the same name, Prometeu uses the local scope first. Then it
  uses the project scope (nearest parent first), and then the user scope. A
  hub definition continues to replace a definition that discovery found.
  This agrees with the
  [Claude CLI scope contract](https://code.claude.com/docs/en/mcp#scope-hierarchy-and-precedence).
- `project_tools_trust` must receive the hash that the dialog showed. The
  backend reads the declaration again and calculates its hash independently.
  Then it compares the two hashes before it records the decision of the
  person. If the hashes are different, or if there is no declaration,
  the command returns `err.tools.changed`. The previous decisions do not
  change, and the person must open the dialog again. The backend does not
  store a hash from the client before it compares it.
- Show the trust prompt when `project_tools.pending` has items. The provenance
  of each item does not change this. The menus of the project and of the
  workspace also show the declaration. Thus the person can open empty
  replacements, rejected decisions and approved decisions. The dialog tells how
  a replacement operates, and how added and removed items operate.
- Tool changes continue to apply only when Prometeu starts or resumes a stopped
  process. Show this rule also when the current process does not do work. The next
  message alone does not load the tools again.

## Alternatives and consequences

Prometeu rejected this alternative: calculate the hash again and do not
compare it with the version on the screen. The backend calculates the hash, but
this does not tell which version the person examined. Prometeu also rejected
an automatic update and approval of a changed declaration. That alternative
gives the new declaration the approval of the previous dialog.

If the provider resolution stays at the workspace level, tabs and action
profiles with different providers do not operate correctly. When Prometeu sends
the active provider, the hub selection stays portable. Also, IDs that only
Claude can use do not go into the Codex configuration.

If a client does not send the new hash argument, the trust command stops with
an error. This is intentional. Prometeu releases the desktop and the browser
mock together. Thus it is not necessary to change the saved trust data or the
board. The discovery arguments are optional and new. The axis payload keeps
its current wire format.

## Verification

- `session.rs::tool_axis_ipc_preserves_absent_null_and_replacement` examines
  native IPC bodies, how Prometeu sets an axis back to the inherited value,
  empty selections and incorrect values.
- `session.rs::tool_resolution_uses_the_tab_provider_and_configured_claude_home`
  operates the two providers and their configuration writers with isolated
  files.
- `session.rs::project_tools_require_approval_and_invalidate_it_when_hash_changes`
  rejects an expired decision to approve or to reject. It does not change the
  saved decisions.
- `mcp.rs::local_mcp_shadows_project_and_user_and_materializes_the_same_definition`
  examines the precedence of duplicate names through the strict configuration
  that Prometeu writes.
- `e2e/tools.spec.ts` examines an empty declaration, expired dialogs, pickers with
  different providers, the inheritance menu and the message that tells when
  the tool changes apply.
