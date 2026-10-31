---
type: Decision
title: "ADR 0051 — Git projects in portable catalogs"
description: "Git projects in portable catalogs."
status: stable
---

# ADR 0051 — Git projects in portable catalogs

Date: 2026-09-19

## Context

People repeat Git clones and project registration on each Mac. The account and
organization catalogs already distribute portable definitions with revisions.

## Decision

Add `projects` to the existing catalog document and browser tabs. Each definition
contains a stable name, remote Git source and optional description. Reuse catalog
authorization, revision checks, local cache and board project registration.

The desktop offers multiple selection and one destination directory. It clones
each selected project into its named subdirectory and registers successful clones.
It can instead link an existing repository after checking its root and origin.
Repositories already registered with the same origin do not appear as available;
a stale installation action reuses their path. Retries preserve completed
projects; conflicting folders remain untouched.

Cloning uses Git installed on the Mac and local authentication. HTTPS and SSH are
the only accepted transports; credentials, local paths and executable transport
helpers cannot enter the document. Git hooks are disabled during clone. Prometeu
does not execute setup, install dependencies or select tools during registration.

## Consequences

Cloud stores definitions, never source files or Git credentials. Local project
paths live in board state and catalog installation links. Losing a definition,
membership or account connection never removes registered projects or files.

Older documents decode with an empty projects collection. Older desktop PUTs
that omit projects preserve stored definitions; an explicit empty collection
removes them. Publish Cloud support before the new desktop. Rolling back the
desktop preserves definitions; rolling back Cloud code requires a version that
still accepts stored projects. No schema migration is needed.

Existing catalog synchronization serializes installation operations and local
project additions/removals, so registration cannot change between the origin
check and the catalog installation. Project commands wait on worker threads,
without holding the board lock. Large clones can delay catalog refresh and local
project edits, as plugin installations already delay catalog refresh. Board reads
and unrelated workspace edits stay available. No background job system or
automatic setup is introduced.

The [catalog contract](../contracts/cloud-catalog.md) specifies wire behavior.
[Rust tests](../../src-tauri/src/catalog/projects.rs) use a local Git upload-pack
fixture for cloning, retry, existing-folder registration and file preservation.
[Browser tests](../../e2e/projects.spec.ts) cover selection, partial failure and
retry in Chromium, plus a representative layout and focus check in WebKit.

## Local creation

The same Projects dialog also creates a local project without a Cloud account.
It accepts a name and parent directory, initializes a new empty Git repository
through the shared native Git adapter, and reuses board registration. Desktop
and WSL expose the additive `create_project` command described in the
[IPC contract](../contracts/ipc.md). Existing registration and catalog formats
are unchanged.

Creation refuses existing paths instead of initializing or overwriting them.
Git templates and inherited repository-location overrides cannot populate or
redirect the new repository. Failed initialization registers nothing; cleanup
only removes an empty directory, preserving partial state or concurrent writes.
An initialized directory remains on disk if subsequent registration fails.

No initial commit is invented: that would require an identity or write authored
history on the person's behalf. The launcher opens a repository without branch
references directly in its project folder until the first commit, preserving
the saved worktree preference for subsequent launches. No setup, remote,
dependencies or provider process starts during project creation.

Tests in `crates/git/src/tests.rs` cover empty initialization, invalid names,
existing paths and symlinks. The resident runtime test covers registration,
rejection without a board change and restart persistence; the bridge test checks
Windows parent-path conversion. `e2e/projects.spec.ts` adds the project-to-first-
conversation journey, including focus and retention of typed input after a
creation failure. Existing clone scenarios do not exercise this nested dialog or
its launcher integration; native tests cannot prove browser focus behavior.
