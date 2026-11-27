---
type: Decision
title: "ADR 0108 — A standalone to-do list on one selected computer"
description: "Tasks shows one standalone to-do list. The list is on this computer or on one remote machine that the person selects. The two MCP servers use the same list."
status: stable
---

# ADR 0108 — A standalone to-do list on one selected computer

Date: 2026-11-27

## Context

Agents in Prometeu must add standalone tasks to a list that the person sees in
Tasks. In the future, this list can replace the kanban of OpenBot. A to-do has
no project, workspace, issue or provider.

Agents operate in two configurations:

- On the desktop, through the embedded MCP.
- On a remote machine, through the MCP of `prometeu-host`. These agents operate
  also while the laptop sleeps ([ADR 0103](0103-agents-on-remote-machines.md)).

The two MCP servers and the tab must use the same list. Two lists that only
have the same tool names do not satisfy this requirement.

## Options considered

1. The desktop keeps the list. The host sends the calls of its agents to an
   attached desktop.
2. Each host and the desktop keep a list. The tab shows a combination of the
   lists.
3. One computer keeps the list. The person selects this computer or one
   machine. The other parts send their calls to that computer.

Option 1 needs requests from the host to the desktop, and that direction does
not exist in protocol 1. Agents on the machine also lose the list while the
laptop sleeps. Option 2 makes many lists. An agent cannot know which list the
person reads, and status changes have no single owner.

## Decision

Use option 3. The rules are in the [contract](../contracts/todos.md).

- `prometeu-core::todos` owns the model, the validation and the store with a
  file port. The desktop and the host give the file.
- The desktop stores the location. The tab always shows it in a selector.
- A host serves the list to its agents only while a desktop selected that
  machine. Thus agents on a machine that does not keep the list get
  `todos_not_here`. They do not write to a list that nobody sees.
- When the selected machine is offline, the desktop shows the error. It does
  not use the local list as a replacement.
- Internal conversation credentials can use the list, because the person
  selected the built-in MCP for that conversation. External clients of the
  desktop need the explicit `--todos` grant. The host token gives access,
  because it already gives all host operations.
- The store limits the list to 1,000 to-dos and to 512 KiB of serialized
  items. Thus each MCP can return the full list in one response, and the tools
  do not need pages.

## Consequences

- The list of a machine stays available to the agents of the machine without a
  desktop.
- The desktop can use the list of a machine only while it is connected.
- A change of location does not copy to-dos. Each list stays in its file.
- Two desktops that select different locations for the same machine can
  disable the list for each other. The contract records this limit.
- The host protocol and the agent protocol add methods and one event. They do
  not change their version.
