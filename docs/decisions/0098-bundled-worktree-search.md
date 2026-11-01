---
type: Decision
title: "ADR 0098 — Bundled search with one graph per worktree"
description: "Link the local search engine into execution hosts and add worktree-scoped MCP access to supported agents."
status: stable
---

# ADR 0098 — Bundled search with one graph per worktree

Date: 2026-10-31

## Context

Agents repeatedly read source to locate symbols and relationships. The separate
`prometeu-search-core` project already provides bounded lexical and graph
queries, persistent SQLite indexes, filesystem watching and stdio MCP. Prometeu
must supply this capability automatically while keeping isolated worktrees and
existing provider configuration independent.

## Options considered

1. Install a global daemon and multiplex all workspaces through it. This adds
   service installation, routing and authorization despite an existing stdio
   lifecycle and watcher ownership protocol.
2. Package a separate search executable. This preserves process independence
   but adds binary distribution, signing and WSL installer paths.
3. Link the engine into each Unix execution host and expose an early stdio MCP
   mode. Provider subprocesses reuse the engine's cache and watcher protocol.

## Decision

Use option 3. Pin the engine revision in the application. The shared native
tools adapter owns search configuration; engine
and provider details do not enter the portable application core or presentation.
Both the desktop and WSL runtime expose the same search mode.

Each canonical worktree directory owns a database under a schema-versioned app
cache. Multiple tabs share that database. Multi-repository workspaces receive
one server per worktree, preserving the same isolation boundary.

Every supported provider process receives search automatically, including
delegated workers. Search is built-in infrastructure outside optional MCP hub
selection. This qualifies the selection guarantees in ADRs
[0045](0045-layered-tool-selection.md) and
[0046](0046-cli-inherited-mcp-base.md): an empty optional selection retains
search, and inheritance retains native servers while adding search. The
delegation MCP remains opt-in.

Reuse engine watchers and reconciliation before queries. A turn hook would miss
external edits and Git operations. A global daemon would keep indexing inactive
workspaces unnecessarily. No client means no watcher; a new client refreshes
before answering. Keep derived caches after workspace archive or removal.

The interactive Map panel reuses the same index through a workspace-scoped
application command. Hosts retain one map engine and perform blocking requests
off their request/UI loops; the visible foreground panel reconciles periodically
and can force a content scan. This avoids adding a second daemon or persistent
watcher solely for the view. Agent watchers remain independent.

Retain a file-level backend overview, but present directory groups first and
file context on demand. Thousands of connections made a whole-worktree SVG
unusable even after limiting the source response: a real 586-file, 4,000-edge
snapshot created over 12,000 DOM elements. Aggregate actual resolved static
relationships, expose weaker evidence and limit the visible graph to 80 nodes
and 160 edges, with at most 40 inspector rows per page. Search retains the
engine's symbol-level context, snippets and uncertainty, showing the associated
files without the unrelated whole-worktree graph. Preserve completed results
while input is pending and reuse unchanged graph generations; avoid rebuilding
the presentation per keystroke or polling response.
Lexical exploration reserves part of the existing symbol budget for connected
context and fills unused slots with further matches. Otherwise broad matches
can consume the entire budget before any relationship endpoint is returned.
Exact selectors and wire formats remain unchanged.

Use Cytoscape for Canvas rendering, layout, hit testing, node dragging and
pan/zoom. The bounded custom SVG renderer improved responsiveness but still
produced tiny labels and excess empty space. Built-in grid and breadthfirst
layouts account for labeled cards and the current container without a continuous
force simulation. Keep the same projection limits: switching renderers does not
justify drawing the entire repository. The paged DOM inspector remains the
keyboard and assistive-technology path to every returned file and symbol.
Cytoscape adds one dependency with bundled TypeScript declarations and no
runtime dependencies; owning a second renderer or separate layout engine would
retain more application code. The component destroys its graph instance when
unmounted.

The app's registered-root admission and returned relative root paths preserve
workspace isolation and correct source navigation for grouped repositories.

## Consequences

- Supported agents get bounded source context without manual setup. Actual
  token savings require comparative agent evaluation.
- The host binary contains the engine and parsers, increasing its size. Engine
  dependencies must remain compatible with the app's Rust and SQLite versions.
- Antigravity remains unsupported because its adapter has no MCP injection.
- Caches retain source snapshots until explicitly removed with clients stopped.
  Schema changes use fresh namespaces rather than mutating incompatible caches.
- Search failures surface as tool failures. Ordinary file tools remain
  available, and the conversation does not depend on successful indexing.

## Evidence

The [code-search contract](../contracts/code-search.md) records provider
coverage, storage, lifecycle and verification. Shared adapter tests verify
worktree isolation and provider preparation; stdio subprocess tests exercise
the linked engine without spending model credits.
