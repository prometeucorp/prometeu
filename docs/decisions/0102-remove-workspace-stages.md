---
type: Decision
title: "Remove workspace stages"
description: "Retire manual workspace stages while preserving sessions and v4 sharing compatibility."
status: stable
---

# ADR 0102: Remove workspace stages

Manual workspace stages are unused. Remove their menus, icons, sorting, archive
labels, translations, analytics action, domain fields, launcher inputs and desktop
and WSL mutation commands. Sidebar workspaces retain board order. Finish keeps
its archive, process shutdown and optional worktree cleanup behavior.

Old boards and launch drafts may contain stage fields; deserialization ignores
them. New saves omit them, preserving all other workspace and transcript data.
Downgrades to versions requiring those fields need a pre-upgrade backup.
This favors removing obsolete domain state over maintaining downgrade-only data.

The v4 relay keeps its legacy `stage` string at the network boundary. New owners
always send an empty string; viewers do not project it into workspace state.
Removing that wire field would break deployed relays without providing user value.
Git staging and observed conversation status remain independent features.

An upgraded Windows shell can remain attached to an old, busy resident. Creation
requires `application.stageless-workspaces.v1`; without it, the bridge explains
that running conversations and terminals must finish before reconnecting to apply
the update. This avoids restoring retired domain data or sending invalid drafts
while keeping existing sessions available.

Core board compatibility tests verify legacy fields disappear on serialization.
Existing lifecycle tests verify archive and finish behavior, and encrypted channel
tests cover the v4 compatibility field, including an actual owner announcement
decrypted and parsed by a viewer.
