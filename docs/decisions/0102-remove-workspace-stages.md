---
type: Decision
title: "Remove workspace stages"
description: "Remove the manual workspace stages. Keep the sessions and the compatibility with v4 sharing."
status: stable
---

# ADR 0102: Remove workspace stages

Persons did not use the manual workspace stages. Remove these items of the
stages:

- The menus, the icons, the sort and the archive labels.
- The translations and the analytics action.
- The domain fields and the inputs of the launcher.
- The desktop commands that change the stage.

The workspaces in the sidebar keep the order of the board. Finish continues to
archive the workspace, stop the processes and, as an option, remove the
worktree.

Boards and launch drafts from earlier versions can contain stage fields. The
deserialization ignores these fields. New saves do not write them, and they
keep all other data of the workspace and of the transcript. A downgrade to a
version that must have these fields is possible only with a copy of the data.
The person must make this copy before the upgrade. This decision removes domain state that is not
necessary. It does not keep data that only a downgrade uses.

The v4 relay keeps its legacy `stage` string at the network boundary. New
owners always send an empty string. Viewers do not put it into the state of the
workspace. If Prometeu removes that network field, deployed relays break, and the
persons get no value from the change. Git staging and the status of the conversation
that Prometeu sees are different features. They do not depend on the stages.

The compatibility tests of the core board make sure that serialization removes
the legacy fields. The current lifecycle tests make sure that archive and
finish operate correctly. The tests of the encrypted channel cover the v4
compatibility field. These tests include an announcement that an owner sent
and that a viewer decrypts and parses.
