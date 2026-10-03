# ADR 0083 — WSL workspace catalog and isolated execution contexts

Date: 2026-09-30
Status: Accepted

## Context

The native WSL shell could only attach to one conversation and supporting shell.
Integrating workspace navigation requires ownership in the runtime: changing the
visible workspace must not restart a provider, mix transcripts or send input to
another shell. The production board models already separate work stage from
observed agent status.

## Decision

Reuse `prometeu-core::board` project/workspace models in an in-place catalog
service with injected `CatalogStore`, `WorkspaceFolders` and provider selection.
`WorkspaceWorktrees` prepares isolated checkouts; the Unix `GitWorktrees` adapter
receives a bounded command runner and returns repository/checkout/branch metadata.
The Unix runtime supplies private atomic storage and directory validation.
Catalog changes save before replacing the in-memory state; invalid or future
catalogs fail explicitly rather than starting with an empty replacement.

The runtime host retains a context per opened session through `ContextFactory`.
A workspace selection resolves its active tab; application commands introduced by
[ADR 0084](0084-shared-windows-desktop.md) address sessions independently.
Each context owns its existing `Runtime`, `SessionStore` and `TerminalService`.
Selecting a workspace obtains its context before committing the active selection.
Only the selected context delivers legacy preview conversation/terminal frames;
application events retain explicit session identities. Other
contexts continue execution and retain their snapshots with detached terminal
credit semantics. Shutdown visits every context, including those off screen.

The primary context retains the original root, transcript and provider identity.
Additional contexts use UUID child roots. `workspaces.json` is additive and reuses
the board shape; it is not production `board.json` and does not adopt desktop data.
Catalog tab fields represent saved configuration, not live status. Conversation
snapshots and events remain authoritative for observed execution state.

`WorkspaceClient` adds typed bridge operations over the existing development
envelope. The bridge now imports the portable core's catalog type, without Unix
or Tauri dependencies. Frontend `Workspaces` consumes an injected `WorkspacePort`;
`Session` coordinates snapshot replacement and buffered events during selection.
Presentation reuses shared forms, buttons, selectors and section headers. Drafts
remain per-workspace memory in the open window. Neither core dispatches by OS.

## Compatibility and limits

The `workspaces.v1` capability and four request variants are additive. Old roots
become the primary workspace without rewriting their transcript. Old clients
operate on the selected context; older runtime binaries ignore the catalog and
can still open the primary root. Selection is not retried after a lost reply.

Worktree creation requires the additive `worktrees.v1` capability. It creates a
new branch from an already available commit, without changing the original checkout.
The host derives a UUID checkout path under its private root. The portable service
validates catalog admission before invoking Git and persists the prepared result.
Git and file persistence cannot form one atomic transaction: on uncertain outcomes,
retain artifacts and report their path instead of risking user files with forced
rollback. Restarting the runtime checks for a catalog commit before manual folder adoption.
This trades automatic cleanup for recoverability; no mutation is retried.

The diagnostic preview exposes existing-folder and new-branch worktree workspaces
with a selected Codex conversation and supporting terminal, up to 64 workspaces
per runtime. ADR 0084 extends the shared application to multiple conversation tabs
and terminal docks. Preexisting-branch worktrees, setup execution, worktree cleanup,
desktop-root adoption, project discovery and other providers remain unimplemented. Reconnection uses the same bootstrap configuration even when a
different workspace is selected. Missing selected directories fail explicitly at
host startup; they must be restored before reopening that catalog.

See the [workspace contract](../contracts/wsl-workspaces.md) for wire operations,
storage and verification. ADRs 0079–0082 remain in force for execution, transport
and attachment guarantees; this extends the original single-workspace slice.
