---
type: Contract
title: "Bundled code search"
description: "Automatic agent access to local code search, isolated worktree graphs and cache lifecycle."
status: stable
---

# Bundled code search

Prometeu links a pinned revision of `prometeu-search-core` through the Unix
`prometeu-tools` adapter. The desktop executable and the Linux WSL runtime
provide the same stdio MCP entry point before starting their normal hosts:

```text
<host-executable> --prometeu-search-mcp <absolute-root> <absolute-database>
```

The engine, parsers and SQLite ship with the application. No runtime download,
model, API key or separately installed search executable is required. The
native Windows shell embeds the Linux runtime; it does not link the engine.
See [ADR 0098](../decisions/0098-bundled-worktree-search.md).

The engine revision is pinned in `src-tauri/crates/tools/Cargo.toml` and
`src-tauri/Cargo.lock`. Updating the pin requires verifying the app's supported
Rust toolchain and dependency compatibility.
Development builds optimize the bundled search crate and Tree-sitter at level 2.
The host retains its normal debug profile and these crates retain debug symbols; without
this override, full-worktree parsing and graph resolution distort interactive
development behavior. Release builds keep their existing profile.

## Availability and selection

Every fresh or resumed Claude, Codex and Cursor desktop process receives the
search server configuration. Codex in WSL receives the same configuration.
Antigravity has no supported MCP injection and receives no search server.
The existing opt-in `prometeu` delegation MCP remains separate.

Search is application infrastructure outside the optional hub selection. An
explicitly empty MCP selection removes optional servers and retains search,
including in delegated workers. An absent selection preserves the provider's
native MCP configuration and adds search. Declared selections keep their
existing provider semantics and add search. The app never writes these entries
to project files, the hub or the provider's global configuration.

The host supplies the actual launch directory, including before a new tab is
inserted in the board. A Git worktree or ordinary folder gets one server. A
multi-repository grouping directory gets one server for each direct child with
a `.git` marker. Each server fixes one root; tool arguments cannot select a
different root or database. Multi-repository queries use the server for the
repository concerned rather than merging graphs across repositories.

## Worktree identity and storage

The cache lives at
`<root>/search/v<engine-schema>/<sha256-of-canonical-root>/index.sqlite`.
The hash uses the canonical path's filesystem bytes. Worktree identity is its
physical directory, not the branch name, repository remote, workspace ID or
conversation ID. Tabs and workspaces using the same physical directory share
its graph; separate worktrees have separate graphs. Symlink aliases resolve to
the same identity. The engine also records and validates the root in the
database, refusing a different worktree.

The schema namespace changes with the engine schema. An upgrade builds a new
derived index without rewriting incompatible databases. The cache is separate
from the board and telemetry databases; no persisted application format changes.
Private cache directories and database files use `0700` and `0600` on Unix.

The cache contains source snapshots and stays local. Archiving or removing a
workspace retains its cache for reuse; removing the worktree does not erase
cached source. There is no automatic cache eviction in this version. Stop all
clients before manually deleting a derived cache. A later query rebuilds it.
See [local persistence](persistence.md).

## Freshness and process lifetime

The provider owns the search subprocess through its stdio connection. One
process owns the filesystem watcher for each database; concurrent clients share
that database and retry watcher ownership after its owner exits. Closing stdin
ends the server and its watcher. Existing conversation shutdown and process
group termination also reach these subprocesses.

Watchers update the graph after agent edits, editor edits and Git changes while
a client runs. Every search, exploration and status tool call reconciles before
reading. Initial refresh and periodic recovery check file contents; repeated
queries use metadata to avoid hashing unchanged files. Metadata-preserving
changes can wait for the engine's periodic full-content recovery, currently on
a 60-second interval. Results are snapshots, not a guarantee that source cannot
change again.

No turn-completion hook or separately installed daemon is needed. A workspace
with no active search client has no watcher. Its next client reconciles before
answering. Watcher failure is diagnostic; query reconciliation remains available.
Index and query failures return MCP tool errors rather than silently answering
from an old snapshot or ending the conversation.
Failure to open the database makes the search server unavailable; ordinary agent
file tools remain available.

## Query contract and limits

The engine supplies three bounded tools: `code_search`, `code_explore` and
`code_search_status`. Responses include source locations and clipped snippets;
`limit` and `max_bytes` bound returned context. The engine's MCP instructions
and tool descriptions direct agents to use `code_explore` first for feature
flows and graph context, or `code_search` for precise lexical locations and
direct callers/callees, before broad grep or whole source reads. Queries should
use short identifiers or a few keywords, not full natural-language questions.
Agents should start with default budgets, wait for results and reuse snippets
before reading missing lines or verifying live source for edits.
`code_search_status` is diagnostic, not a required preflight; queries refresh
automatically. This guidance does not insert the graph into every agent turn.
Cursor resolves delayed MCP metadata into the server and tool names, so a graph
query is displayed as `prometeu-search · code_explore` rather than a permanent
`MCP: tool` placeholder.

Search is lexical. Graph relationships are static candidates with uncertainty,
not compiler proof. Supported languages are TypeScript, TSX, JavaScript/JSX,
Rust and Ruby. Ignore rules, unsupported files, symlinks and the engine's size
limits constrain coverage. Ordinary agent file search remains available for
unsupported content, exact-text misses and failures. No measured token savings
are claimed.
Repository instructions and Markdown documentation still require ordinary file
reads. Tool availability does not force the model to use search for every read.

The engine makes no network requests. Returned source may reach the chosen
model provider through the agent, like other file tools. Source remains
untrusted project content. Search uses the local user's permissions and is not
a sandbox or a new remote access interface.

## Interactive Map

The local Files/Changes panel also offers Map, independently of the conversation
provider. It is available for usable local workspaces and directly opened
projects, including when no agent is running. Remote, preparing, failed and
cleaned workspaces cannot request local graph data.

`workspace_code_map` accepts `{ id, root?, query?, path?, line?, force? }`.
The host resolves `id` from its board; `root` selects an index ID returned by
that workspace's previous response. Callers cannot supply a database or absolute
filesystem root. Optional `path` and one-based `line` select indexed source for
exploration. The shared adapter validates selectors and query limits before
indexing. Desktop runs blocking work outside the UI thread; Windows uses the
runtime's deferred application-operation transport.

The response contains `roots`, `root_id`, `status`, `overview` and nullable
`result`. Each root includes an ID, display label, canonical path and `relative`
path within the workspace/project directory. Source navigation uses `relative`
to open the correct file in grouped repositories, including symlink aliases.
Status reports indexed file/symbol/reference counts, generation and
`last_indexed` in Unix seconds, or null before reconciliation. This timestamp
means the last successful index reconciliation, not the last source edit.

The backend overview groups symbols by file. Its edges aggregate resolved
cross-file static relationships with a count;
`uncertain` marks low/unknown-confidence or ambiguous evidence. All relationships
remain static candidates, including solid edges. Unresolved destinations never
become invented file connections. Overview limits are 1,000 files, 4,000 edges
and 512 KiB; truncation is visible and queries can surface files beyond that
overview. Search uses the same `Engine::explore` as agent MCP, with at most
50 returned symbols and 64 KiB, retaining source ranges, snippets, uncertainty
and truncation. Initial matches and expanded neighbors share that symbol limit.
For lexical exploration, at most half the symbol budget, rounded up, starts as
ranked seeds; relationships receive priority over the remaining matches.
Unused context slots return further ranked matches. Exact symbol, file and line
selection keep their existing semantics. Agent MCP uses the same allocation
with its requested budgets. Zero returned cross-file edges in a truncated
response does not establish that the code has no connections.
These transport limits are not rendering budgets. The initial view groups files
by directory and aggregates their actual cross-directory connections. Opening a
directory reveals its files; searching shows only the returned file context.
The graph displays at most 80 nodes and 160 connections, and the inspector pages
through at most 40 rows at once. Omitted connections are indicated explicitly.
This keeps a repository with thousands of relationships usable without drawing
the entire graph in the webview. Cytoscape renders labeled cards and connections
on Canvas, using built-in grid or breadthfirst layouts sized to the container.
Initial layouts keep labels readable; Fit shows the full bounded graph, and
selecting an inspector result reveals its file when it is outside the viewport.
The DOM inspector provides keyboard-accessible file and symbol selection.
Selecting a symbol previews its indexed snippet;
Open source reads the live file and selects its line without replacing a draft.

Map shares the agent cache identity and database. Each host retains one map
engine, replacing it when the selected root changes. It starts no additional
watcher. Within each graph generation, the engine reuses import target resolution
and the aggregated overview; file changes invalidate both with the graph.
The visible foreground panel reconciles every 15 seconds and after
editor saves; requests coalesce while indexing. Hiding the panel stops its
polling, and obsolete workspace/root/query responses cannot replace current
results. Typing retains the last completed graph while the query is pending;
unchanged generations reuse the existing overview and query presentation.
The renderer handles panning and zooming without rebuilding the graph or its
inspector. Agent watchers continue independently. Sync now performs a full content
scan, preserving the query and last successful snapshot if it fails. Overview,
exploration and status carry their own generations because a concurrent watcher
may commit between reads. No graph is persisted in frontend storage or sent to
the relay or analytics.

## Verification

`src-tauri/crates/tools/src/search.rs` tests canonical identities, separate
worktrees and multi-repository server roots. Provider preparation tests cover
automatic injection and preservation of optional MCP selection semantics.
Real stdio subprocess tests exercise both host entry points, queries and process
shutdown without starting a model: `src-tauri/tests/search_mcp.rs` and
`src-tauri/crates/runtime/tests/search_mcp.rs` share assertions in
`src-tauri/tests/support/search_mcp.rs`. They cover real divergent Git worktrees,
updates and deletions, concurrent clients, fixed-root validation and
shutdown. Upstream engine tests cover incremental
updates, deletions, ignore rules, concurrent clients, watcher handoff and byte
budgets. Native packaging and live model behavior remain separate checks.

Map service tests in `crates/tools/src/search/map.rs` prove shared MCP databases,
root and query validation, grouped roots and symlink aliases, query snippets,
updates and forced reconciliation. Core admission tests reject unavailable
workspace roots; runtime lifecycle tests exercise the actual deferred transport.
The Rust-produced `codeMap` boundary fixture is checked against the TypeScript
IPC type. `src/components/code-map-model.test.ts` covers graph projection,
uncertainty, aggregation and rendering bounds;
`src/workspace-code-map.test.ts` covers request coalescing, stale responses,
retained graph references, hidden/background polling and recovery. The catalog
includes the production view in populated, search, loading, empty, error,
bounded and large states, including 1,000 files and 4,000 connections.
`e2e/code-map.spec.ts` guards Canvas painting, panning, retained canvases during
pending input and keyboard selection in Chromium and WebKit, following the
previous WebKit stall. A separate source-opening journey preserves an unsaved
editor draft through synchronization and Map expansion. These browser checks
cover renderer scheduling, focus and live viewer interaction beyond the pure
projection and controller tests. Compact-panel checks reproduce the height left
by an open Setup dock and verify that scrolling exposes results and controls
without overlap. They do not replace native indexing tests;
performance investigations must measure real repository indexing, graph loading
and webview interaction separately rather than relying on a small mock.
