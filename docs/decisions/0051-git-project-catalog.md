---
type: Decision
title: "ADR 0051 — Git projects in portable catalogs"
description: "Git projects in portable catalogs."
status: stable
---

# ADR 0051 — Git projects in portable catalogs

Date: 2026-09-19

## Context

Persons clone the same repositories and add the same projects again on each Mac.
The catalogs of accounts and organizations send portable definitions with
revisions at this time.

## Decision

Add `projects` to the current catalog document and to the browser tabs. Each
definition contains a stable name, a remote Git source and an optional
description. Use the current catalog authorization, revision checks, local
cache and project registration on the board.

The desktop lets the person select many projects and one destination directory.
It clones each selected project into a subdirectory with the project name. It
adds each clone that completes correctly to the board. As an alternative, it
can link a repository that is on the disk. Before it does this, it examines the
root and the origin of that repository. A repository that is on the board with
the same origin does not show as available. An expired installation action uses the
path of that repository. When the person tries again, the completed projects
stay. Prometeu does not change a folder that has different contents.

The clone operation uses the Git installation on the Mac and the local
authentication. HTTPS and SSH are the only transports that Prometeu accepts.
The document cannot contain credentials, local paths or executable transport
helpers. Prometeu disables Git hooks during a clone. During registration,
Prometeu does not run setup, install dependencies or select tools.

## Consequences

The Cloud stores definitions. It does not store source files or Git
credentials. The local project paths are in the board state and in the catalog
installation links. If a definition, a membership or an account connection
stops, Prometeu does not remove the projects on the board or their files.

Older documents decode with an empty collection of projects. Older desktop PUT
requests that do not contain projects keep the stored definitions. An
explicit empty collection removes them. Publish the Cloud changes before the
new desktop. If Prometeu installs an older desktop version again, the
definitions stay. If Prometeu installs older Cloud code again, that version must
accept stored projects. A schema migration is not necessary.

The current catalog synchronization puts installation operations and local
project changes in sequence. Thus the registration cannot change between the
origin check and the catalog installation. Project commands wait on worker
threads and do not hold the board lock. Large clones can make the catalog
refresh and local project changes wait. Plugin installations make the catalog
refresh wait in the same procedure. Board reads and changes to other workspaces stay
available. This decision does not add a background job system or an automatic
setup.

The [catalog contract](../contracts/cloud-catalog.md) specifies the wire
behavior. The [Rust tests](../../src-tauri/src/catalog/projects.rs) use a local
Git upload-pack fixture. They examine the clone, a second try, the registration
of a current folder and that the files stay. The
[browser tests](../../e2e/projects.spec.ts) examine selection, a failure of some projects
and a second try in Chromium. They also do a representative check of layout
and focus in WebKit.

## Local creation

The same Projects dialog also makes a local project without a Cloud account. It
accepts a name and a parent directory. It makes a new empty Git repository
through the shared native Git adapter. Then it uses the current board
registration. The desktop has the new `create_project` command that the
[IPC contract](../contracts/ipc.md) specifies. The current registration and
catalog formats do not change.

Prometeu rejects a path that is on the disk. It does not initialize that path or
replace its contents. Git templates and inherited overrides of the repository location
cannot add files to the new repository or move it. If a failure occurs during the
initialization, Prometeu adds no project to the board. The cleanup removes only
an empty directory. Thus it keeps a state that is not complete, or files that a different
process writes at the same time. If a failure occurs during the registration
after the initialization, the initialized directory stays on the disk.

Prometeu does not make an initial commit. For a commit, an identity is necessary,
or Prometeu must write history for the person. Until the first commit, the
launcher opens a repository that has no branches directly in its project
folder. The saved worktree preference stays for subsequent launches. Project
creation does not start setup, a remote, dependencies or a provider process.

The tests in `crates/git/src/tests.rs` examine empty initialization, incorrect
names, paths that are on the disk and symlinks. `e2e/projects.spec.ts` adds the
full path from a new project to its first conversation. This test also
examines the focus, and it makes sure that typed input stays after a creation
failure. The current clone scenarios do not operate this nested dialog or its
launcher integration. Native tests cannot prove the focus behavior of the
browser.
