# ADR 0078 — Native package preparation with injected installation

Date: 2026-09-28
Status: Accepted

## Context

Startup consumed injected tool artifacts, but plugin/skill flags, derived Codex
homes, manifests and installed-cache coordination still lived in desktop
`plugins.rs`. A headless execution host could not reuse those implementations.

## Decision

Move native package preparation into `prometeu-tools::packages`.
`PackageBackend` exposes Claude flags and Codex preparation; `NativePackages`
receives explicit workspace-home and user-home roots, marketplace namespace,
`PackageCatalog`, `PackageFiles`, `PackageInstaller` and a shared preparation gate.
`NativeTools` receives this backend at desktop and WSL composition (ADR 0084).

`FilePackageCatalog` implements the existing private catalog read behavior from
an explicit path. `PackageFiles` supplies private directory creation and atomic
writes, returning raw causes so each operation retains its existing error code.
Native reads, hashing, copying, links and derived configuration belong to the
execution adapter. `CodexInstaller` implements the cache interface with the
existing CLI commands, taking an explicit executable and marketplace namespace.
Installer errors are already application encoded and propagate unchanged.

Hosts must share the same gate among instances accessing a shared installed
cache. Desktop composition and compatibility facades use one gate, preserving
the previous serialization of preparation across workspaces and accounts.
Build-mode namespace selection remains at desktop composition; native preparation
adds no platform dispatch or vendor configuration to the portable core.

## Compatibility and limits

Persisted plugin fields, absent-field defaults, invalid/missing catalog behavior,
selection filtering and CLI flags are unchanged. Absent selections skip catalog
and materialization effects; explicit empty Codex selections still prepare a
derived home. Standalone skills keep the same package pipeline. Deleted plugin
IDs are skipped and duplicate Codex IDs are deduplicated as before.

Workspace/account paths, snapshot hashes and revision 2 cache versions remain
stable. Hook detection, manifest overlay, unknown fields, retained hook trust,
credential links and global configuration preservation retain their existing
behavior. Installation failure still attempts removal of the failed canonical
ID and propagates the original failure. Cleanup remains best effort; preparation
has no cross-file transaction or rollback. The native CLI adapter retains its
existing blocking calls without adding deadlines or output bounds.

The startup backend captures roots at composition and reads the current catalog
on each explicit preparation. Desktop cleanup/catalog facades discover roots at
call time, retaining their existing behavior. Root changes require recomposition
of startup dependencies.

ADR 0084 now shares local catalog mutation and repository clone/import/update
through `PluginLibrary`. Agent-generated plugin creation and Cloud publication
remain desktop use cases. MCP discovery, OAuth and built-in lifecycle
also remain to be extracted. No headless executable, Windows shell or WSL bridge
is introduced; ADR 0050's deployment decision remains in force.

## Evidence

- [Package ports and native implementation](../../src-tauri/crates/tools/src/packages.rs).
- [Moved and injected compatibility tests](../../src-tauri/crates/tools/src/packages/tests.rs).
- [Native installer and hermetic CLI fixture](../../src-tauri/crates/tools/src/package_installer.rs).
- [Desktop composition and private-file fixture](../../src-tauri/src/plugins.rs).
- [Startup composition](../../src-tauri/src/tool_materialization.rs).
- [Application contract](../contracts/application-core.md#native-package-preparation).
