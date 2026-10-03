# ADR 0077 — Startup tools behind injected native ports

Date: 2026-09-28
Status: Accepted

## Context

Provider preparation still called desktop MCP and plugin modules directly.
Configuration encoding mixed native secrets, catalog discovery and persistence,
preventing a headless host from reusing it independently of Tauri.

## Decision

Create the Tauri-free Unix `prometeu-tools` crate. `StartupTools` supplies typed
Claude configuration/plugin arguments, Codex plugin artifacts and Codex MCP
configuration. `main.rs` injects its desktop implementation into
`NativeProviders`; provider adapters consume artifacts and retain their own
command/protocol assembly. The existing package-selection rule combines plugins
and standalone skills before calling the port. Native paths and captured profiles
remain execution-side values, outside the portable core and future bridge payloads.

MCP encoding moves into the crate. `McpSources` supplies effective catalogs,
session-owned built-ins and refreshed OAuth tokens; `McpFiles` supplies private
atomic writes and returns already encoded errors. The desktop implements both
ports, retaining discovery, embedded MCP ownership, OAuth and the existing file
writer. The core receives no vendor configuration or platform branches.

The startup interface has separate Codex plugin and MCP operations to preserve
ordering: profile preparation, plugin installation, profile environment, then
MCP. Claude materializes MCP and plugin arguments before profile preparation.
Failures stop subsequent preparation; no conversation process starts here.

## Compatibility and limits

No persisted format, IPC, selection semantics or CLI flags change. Absent
selections preserve defaults; explicit empty MCP selections still override them.
Missing selected MCP servers fail startup. Deleted plugin entries still follow
the existing skip behavior. Claude retains unknown server fields and uses a
strict private file. Codex remote secrets use child environment variables;
stdio secrets use private environment files and the existing Unix shell wrapper.
The private writer retains 0700 directories and 0600 files.

Codex retains account/workspace configuration isolation, canonical plugin IDs,
hook IDs, cache installation and trust behavior. Antigravity keeps rejecting
unsupported tool selections without invoking materialization. Preparation adds
no multi-file transaction or rollback; a later failure may leave earlier artifacts.

[ADR 0078](0078-injected-native-packages.md) extracts startup package preparation,
file catalog reads and Codex cache installation behind native ports.
[ADR 0084](0084-shared-windows-desktop.md) composes the shared `NativeTools` in WSL
with local catalog mutation and core selection/trust rules. Plugin repository
installation, OAuth and built-in lifecycle still require host integration.
ADR 0050's desktop in-process deployment remains in force.

## Evidence

- [Native ports and artifacts](../../src-tauri/crates/tools/src/lib.rs).
- [MCP encoding](../../src-tauri/crates/tools/src/mcp.rs) and
  [standalone compatibility tests](../../src-tauri/crates/tools/src/tests.rs).
- [Desktop startup implementation](../../src-tauri/src/tool_materialization.rs).
- [Injected preparation tests](../../src-tauri/src/agent_launch.rs) and existing
  [MCP](../../src-tauri/src/mcp.rs), [plugin](../../src-tauri/src/plugins.rs) and
  [provider](../../src-tauri/src/claude.rs) fixtures.
- [Application contract](../contracts/application-core.md#native-startup-tools).
