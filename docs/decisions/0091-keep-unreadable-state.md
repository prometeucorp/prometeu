---
type: Decision
title: "ADR 0091 — Keep unreadable state before replacing it"
description: "Prometeu copies unreadable board and team files to a recovery folder before it replaces them. Saves stop while Prometeu cannot make the copy."
status: stable
---

# ADR 0091 — Keep unreadable state before replacing it

Date: 2026-10-14

## Context

When `board.json` does not deserialize, Prometeu reads `board.json.bak`. If
that file also does not deserialize, Prometeu starts with an empty board. This
occurs, for example, after a new version of Prometeu writes a format that an
earlier version cannot read.

Prometeu updated `board.json.bak` only from a board that it can parse. But when
Prometeu did not parse the two files, two saves wrote the empty board on the
two files. The person saw an empty sidebar and got no information about the
data that was not there.

`team.json` had the same problem. Prometeu read a file that it did not parse
as "no team". When the person connected to a team again, Prometeu replaced the
file.

## Options considered

- **Move unreadable files to a different name at load.** This operation does
  not read the bytes. But it moves the initial files also when the app stops
  without a save. Thus the new version that wrote the files cannot find them
  again.
- **Copy at the first save.** The initial files stay in their location. But
  the notice at startup cannot show copies that Prometeu did not make at this
  time.
- **Copy at load and stop saves while Prometeu cannot make the copy.** The
  initial files stay in their location until a save replaces them. The notice
  can show copies that are on the disk.

## Decision

Prometeu does not replace bytes that it cannot read.

- At load, Prometeu copies an unreadable `board.json` byte for byte into the
  private `<root>/recovery/` folder.
- While Prometeu cannot make that copy, or when Prometeu cannot read the file,
  the board store returns an error for each save. Thus the data on the disk
  does not change.
- When the board uses an unreadable `board.json.bak`, Prometeu copies that file
  at load. If not, Prometeu copies it before the first save of the session
  updates it. Without a copy, saves do not update `board.json.bak`.
- Prometeu writes the same copy one time only. A new copy does not use the name
  of a current copy. Prometeu keeps the five most new different copies of each
  file.
- At startup, the interface tells the person that the board came from
  `board.json.bak` or started empty. It shows the recovery folder and a copy
  error, if there is one. It can open the folder.
- Prometeu copies `team.json` in the same procedure before a person connects to
  a team or goes out of a team. These operations replace or remove the file.

## Consequences

- A downgrade or a corrupted file does not break workspaces and tabs. The
  person puts them back manually from the copies.
- If Prometeu cannot write to the recovery folder, the app continues to
  operate, but it does not save the board. This condition continues until the
  person repairs the cause. The dialog tells the person about this condition.
- The formats of the stored data do not change.

## Evidence

- [Board store recovery](../../src-tauri/src/board_store.rs).
- [Copies, deduplication and the limit on copies](../../src-tauri/src/recovery.rs).
- [`team.json` preservation](../../src-tauri/src/team.rs).
- [Persistence contract](../contracts/persistence.md#unreadable-state) and
  [IPC](../contracts/ipc.md#board-recovery).
