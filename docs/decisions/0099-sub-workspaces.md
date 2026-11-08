---
type: Decision
title: "ADR 0099 — One level of sub-workspaces"
description: "Group delegated workspaces under their coordinator and reserve one conversation per delegation."
status: stable
---

# ADR 0099 — One level of sub-workspaces

Date: 2026-11-08

## Context

Delegated worktrees appear as unrelated workspaces, making parallel work hard
to follow. The existing delegation records already identify the exact owner
conversation and worker workspace. A second persisted parent field would
introduce a competing relationship and require migration.

## Decision

Project local, active delegation records into one navigation level. Follow the
owner conversation to its workspace; if that workspace is itself delegated,
show the entire chain as siblings under the main workspace. Keep MCP ownership
unchanged: grouping grants no transitive authority. Call the group
**sub-workspaces** and its root the **main workspace**.

Each delegated workspace reserves its original conversation. The native desktop,
portable workspace service and browser mock reject additional conversations,
including agent Actions. Prompt Actions still use the current conversation;
the PR button uses its existing current-conversation path when an agent Action
would create another tab. The composer, shortcut and tab controls follow the
same restriction. Main workspaces retain their normal tabs.

The sidebar shows one row with status, provider mark and title; branch and model
remain in the tooltip. The selected sub-workspace links back to the main
workspace and names its coordinating conversation. Hide its conversation tab
strip when it is redundant, but show it when files, Changes, preview or terminals
need navigation. This restriction concerns conversations, not supporting tools.

Existing delegated workspaces with several conversations retain every transcript
and their tab selectors. They cannot add another conversation. External clients
and missing or archived coordinators have no visible main workspace, so their
delegated workspaces remain independently accessible. Archived children stay in
the archive. Restoring a coordinator restores its grouping. Cycles fail open for
navigation: show the workspaces independently rather than hiding them.

A family follows its main workspace's project, including cross-project workers.
Pinning any member puts the whole family in Pinned; member pin flags are retained.
Project search includes sub-workspace titles. Stages, archives, worktrees and
processes remain independently controlled; grouping adds no cascading operations.

## Compatibility and consequences

No persisted, IPC or relay fields change. Old boards without delegations retain
the existing presentation. A downgrade restores the flat list and permits more
tabs without losing history. Existing ownership records continue to authorize
only the originally delegated conversation. Remote shares do not inherit this
local-only hierarchy.

The flattened projection avoids deep trees and a second relationship store.
Its trade-off is that deleting the coordinator conversation loses the visible
link; surviving workspaces and their transcripts stay accessible. A future
requirement to retain links after deletion would need a separate persisted
contract, not inference from branch names or tab order.

## Verification

`src/workspace-hierarchy.test.ts` covers legacy boards, explicit ownership,
flattening, multi-tab preservation, cycles, external owners, archive/restore and
remote exclusions. Core board and workspace tests cover admission before
mutation and preservation of legacy conversations across serialization/restart.
See [embedded MCP](../contracts/embedded-mcp.md) and the
[provider matrix](../quality/provider-matrix.md).
