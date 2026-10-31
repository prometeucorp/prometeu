---
type: Decision
title: "ADR 0091 — Keep unreadable state before replacing it"
description: "Unreadable board and team files are copied to a recovery folder before anything replaces them, and saves wait while copying fails."
status: stable
---

# ADR 0091 — Keep unreadable state before replacing it

Date: 2026-10-14

## Context

`board.json` falls back to `board.json.bak` and then to an empty board when it
does not deserialize, for example after a newer version wrote a format this one
cannot read. The backup was only refreshed from a board that still parsed, but
when both files failed, two saves were enough to write the empty board over
both, and the person saw an empty sidebar with no hint of what had been lost.
`team.json` had the same shape: a file that did not parse read as "no team" and
joining again replaced it.

## Options considered

- **Rename unreadable files aside at load.** Works without reading the bytes,
  but moves the originals even when the app quits without saving, so the newer
  version that wrote them would no longer find them.
- **Copy at the first save.** Leaves the originals in place, but the notice at
  startup could not point at copies that do not exist yet.
- **Copy at load and block saves while copying fails.** Leaves the originals in
  place until a save replaces them, and the notice can point at real copies.

## Decision

Prometeu never replaces bytes it could not read. An unreadable `board.json` is
copied byte for byte into the private `<root>/recovery/` folder at load. While
that copy fails, or when the file cannot be read at all, the board store returns
an error for every save, so nothing on disk changes. An unreadable backup is
copied at load when the board falls back to it, and otherwise before the first
save of the session would refresh it; without a copy, saves skip the refresh and
leave it untouched. Identical copies are written once, a new copy never takes an
existing name, and the newest five distinct copies of each file are kept. The
interface tells the person at startup that the board came from the backup or
started empty, shows the recovery folder and any copy error, and can open the
folder. `team.json` is copied the same way before joining or leaving replaces or
removes it.

## Consequences

- A downgrade or a corrupted file no longer destroys workspaces, stages and tabs;
  restoring them is manual, from the copies.
- A recovery folder that cannot be written leaves the app running with an
  unsaved board until the cause is fixed. The dialog says so.
- The WSL runtime keeps its own rule: an invalid catalog is an error and is
  never replaced by an empty one.
- No persisted format changes.

## Evidence

- [Board store recovery](../../src-tauri/src/board_store.rs).
- [Copies, deduplication and pruning](../../src-tauri/src/recovery.rs).
- [`team.json` preservation](../../src-tauri/src/team.rs).
- [Persistence contract](../contracts/persistence.md#unreadable-state) and
  [IPC](../contracts/ipc.md#board-recovery).
