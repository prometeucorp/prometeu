---
type: Decision
title: "ADR 0099 — One level of sub-workspaces"
description: "Put delegated workspaces in a group below their coordinator, and keep one conversation for each delegation."
status: stable
---

# ADR 0099 — One level of sub-workspaces

Date: 2026-11-08

## Context

Delegated worktrees show as workspaces without a relation. Thus it is
not easy to follow work that occurs at the same time. The current delegation
records identify the accurate owner conversation and the worker workspace. A
second stored parent field adds a second relation that is not the same. It
also makes a migration necessary.

## Decision

Show the local, active delegation records on one navigation level. Follow the
owner conversation to its workspace. If that workspace is also a delegated
workspace, show all of the chain as siblings below the main workspace. Do not
change the MCP ownership. The group does not give transitive authority. The
name of the group is **sub-workspaces**. The name of its root is the **main
workspace**.

Each delegated workspace keeps its initial conversation as its only
conversation:

- The native desktop, the portable workspace service and the browser mock
  reject more conversations. This rule includes agent Actions.
- Prompt Actions continue to use the current conversation.
- When an agent Action makes a new tab, the PR button uses its current
  path for the current conversation.
- The composer, the shortcut and the tab controls obey the same rule.

Main workspaces keep their usual tabs.

The sidebar shows one row with the status, the provider mark and the title. The
branch and the model stay in the tooltip. The selected sub-workspace has a link
back to the main workspace, and it shows the name of its coordinator
conversation. Hide the tab strip of its conversation when it is not necessary.
But show the tab strip when the person must go to files, Changes, preview or
terminals. This rule is about conversations, not about supporting tools.

Current delegated workspaces with more than one conversation keep all of their
transcripts and their tab selectors. They cannot add a new conversation.

- External clients, and coordinators that are not there or in the archive,
  do not have a main workspace that the person can see. Thus the person can open their
  delegated workspaces independently.
- Archived children stay in the archive.
- When the person puts a coordinator back from the archive, Prometeu shows
  its group again.
- A cycle does not stop the navigation. Prometeu shows the workspaces
  independently and does not hide them.

A family follows the project of its main workspace. This rule includes workers
in a different project. If the person pins one member, all of the family goes
into Pinned. Prometeu keeps the pin flag of each member. When the person types in
the project search, the results include the titles of sub-workspaces. The person controls archives, worktrees
and processes independently. The group does not add cascade operations.

## Compatibility and consequences

The stored fields, the IPC fields and the relay fields do not change. Boards
from earlier versions without delegations keep the current presentation. A
downgrade shows the flat list again and lets the person add more tabs. The
history stays. The current ownership records continue to give authority only
for the conversation that the person delegated first. Remote shares do not
inherit this local hierarchy.

The flat projection prevents deep trees and a second store for relations. But
when a person deletes the coordinator conversation, the link that the person
sees goes away. The workspaces and their transcripts that stay are available. If the
links must stay after a delete in the future, a different contract for stored
data is necessary. Prometeu must not find the links from branch names or from the
order of the tabs.

## Verification

`src/workspace-hierarchy.test.ts` includes these items:

- Legacy boards and explicit ownership.
- The flat projection and the tabs that Prometeu keeps.
- Cycles, external owners, the archive, the return from the archive, and
  remote exclusions.

The core board tests and workspace tests make sure that Prometeu accepts a
conversation before it changes the state. They also make sure that Prometeu
keeps legacy conversations through serialization and a new start of the app. See
[embedded MCP](../contracts/embedded-mcp.md) and the
[provider matrix](../quality/provider-matrix.md).
