# ADR 0084 — Shared desktop interface over an injected WSL runtime

Date: 2026-09-30
Status: Accepted

## Context

Windows users need Prometeu without WSLg, while projects, Git, agent CLIs,
shells and credentials stay inside WSL. The portable core
([ADR 0085](0085-portable-core.md)) lets execution run outside Tauri, but the
production desktop links Unix adapters and assumes local paths. A first
iteration built separate Windows screens over a narrow conversation protocol; it
validated the transport and attachment survival, then duplicated the product
interface and postponed real integration.

## Options considered

- Run the Linux desktop through WSLg.
- Grow the separate Windows screens until they match the desktop.
- Load the existing desktop interface in a native Windows window and inject a
  transport that executes the existing commands in a WSL runtime.

## Decision

Use the third option. The separate preview screens are retired.

### Runtime in WSL

`prometeu-runtime` is a Unix executable without Tauri. It composes the same
core services and native adapters as the desktop (process, profiles, tools,
files, Git, protocols) with its own storage, event delivery and task execution.
Codex translation is shared through `prometeu-protocols`; the host supplies the
language callback and client version. The runtime owns an exclusively leased
private root and never adopts a desktop root or `board.json`. It stays resident
across client loss under [ADR 0082](0082-resident-wsl-attachments.md).

The runtime keeps its workspace catalog in `workspaces.json`, reusing the board
models and the shared workspace, lifecycle and tool rules. Catalog changes save
before replacing memory; malformed or future catalogs fail explicitly instead of
starting empty. Saved tab fields are configuration; conversation snapshots and
events are the observed status. Each conversation session has its own context
(runtime, transcript store and terminals) opened through `ContextFactory`, so
selecting or addressing one never restarts or redirects another. Git and catalog
writes cannot be one transaction: on an uncertain outcome the runtime keeps the
checkout and reports its path instead of forcing rollback.

### Composition and transport

The Windows build uses the existing `index.html` and `src/main.ts`; a Vite entry
transformation selects the Windows bootstrap. The bootstrap injects an
`IpcTransport` that sends the existing typed commands to the runtime through
`application.v1`, plus the application menu. The Tauri transport remains the
default for the desktop and the browser mock. Screens contain no OS checks.
Commands address sessions and workspaces by stable ID; the runtime never
switches a shared active workspace before executing. Unsupported commands reject;
missing services are never answered with success placeholders.

`prometeu-bridge` holds the typed client and the injected WSL process, path,
environment and installer ports. The Windows shell links no Unix execution crate
(`prometeu-process`, `-profiles`, `-tools` or `-runtime`); `npm run
architecture:check` guards the crate graph. Supporting terminals use their own bounded credit flow
and never travel as conversation messages.

### Host effects

Window-local effects stay native and are injected at the Windows boundary:

- `ApplicationPaths` translates folder, picker, drop and clipboard selections.
  Only the connected distribution's UNC paths are accepted; drive paths go
  through that distribution's bounded `wslpath`. Canonical validation stays in
  WSL. Linux names that Windows would reinterpret are rejected.
- `FileManager` reveals WSL-validated locations in Explorer through the Shell
  APIs, never through a command shell or a file's default application.
- Attachments keep the existing `@path` contract. Clipboard images are stored in
  the Windows app-local cache and referenced through the WSL mount, avoiding an
  upload protocol at the cost of requiring that mount to stay reachable.
- MCP OAuth consent opens the Windows browser and owns the loopback listener;
  the verifier and tokens stay in WSL. This avoids relying on WSL loopback
  forwarding.
- Connection recovery and terminal snapshots are injected ports with inert
  desktop defaults. Only attachment and read-only restoration retry.

### Long-running effects

Git, checkout preparation, discovery, Setup and MCP checks can exceed the
transport deadline. The runtime runs them as bounded, memory-only deferred jobs
negotiated through `application.operations.v1`, `application.initialization.v1`
and the MCP operations; the bridge polls and preserves the original command
result. Only the owner loop commits catalog changes. Older residents keep
synchronous commands; an uncertain start or poll never falls back or replays.
The cost is polling and loss of unfinished jobs on runtime death. Destructive
lifecycle operations and shutdown refuse while effects run.

### Startup and distribution

The app discovers the default WSL distribution, embeds a matching x86_64 Linux
runtime and installs it content-addressed under a private user directory after
checksum and loader checks. There is no setup screen or runtime download. The
persisted root identity, read through `RuntimeRoots`, wins over WebView storage.
Packaging is Tauri's per-user NSIS installer with the product identifier
`co.prometeu.desktop` and the root package version. Updates use the next
installer; signing and automatic updates are separate work. The installer never
touches WSL state or projects.

## Consequences

The Windows app reuses product screens instead of duplicating them, but
opening the shared shell is not feature parity: each command needs a runtime
implementation, and gaps stay explicit. The desktop, browser mock, runtime and
native Windows journeys are separate validation boundaries. Only Codex with its
external CLI account is registered in WSL; managed login, other providers, Cloud
synchronization and built-in delegation remain to be integrated. The production
desktop stays in-process under [ADR 0050](0050-tested-application-boundaries.md).

## Evidence

- [Windows application contract](../contracts/windows-application.md): command
  coverage, envelope, deadlines, paths and verification.
- [WSL runtime protocol](../contracts/wsl-runtime.md): framing, capabilities,
  storage and compatibility.
- `src-tauri/crates/runtime/tests/`, `src-tauri/crates/bridge/src/` tests and
  the opt-in native journey `scripts/test-windows-application.mjs`.
