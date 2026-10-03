# Windows desktop with an injected WSL runtime

Date: 2026-09-27
Status: In progress (updated 2026-10-03). The portable core
([ADR 0085](../../decisions/0085-portable-core.md)), the resident WSL runtime
([ADR 0082](../../decisions/0082-resident-wsl-attachments.md)) and the shared
Windows interface ([ADR 0084](../../decisions/0084-shared-windows-desktop.md))
are implemented. A per-user NSIS installer with the embedded runtime passed native
acceptance with authenticated Codex. Full application parity is not claimed.

## Scope

Run Prometeu's existing interface directly on Windows. Projects, worktrees, Git,
agent CLIs, supporting terminals and their credentials stay inside the default
WSL2 distribution. The Windows shell owns windows, menus, clipboard, dialogs,
notifications, external links and embedded webviews. WSLg is not part of this
path.

The first release targets Windows x64 with one WSL2 distribution and one runtime
root per desktop instance. macOS and Linux keep local, in-process execution
([ADR 0050](../../decisions/0050-tested-application-boundaries.md)). Native
Windows agent execution and boards from several distributions are later work.
Nothing installs WSL, replaces a distribution or relocates projects silently.

## Principles

- Composition roots select implementations; features receive only the ports
  they use. No service locator, universal `Platform` object or generic
  `execute(method, JSON)` API reaches application code.
- Application rules and presentation contain no Windows/WSL/macOS dispatch.
  Capabilities describe behavior, not disguised OS identifiers.
- The WSL runtime is the sole writer of its board, transcripts, accounts, tools
  and execution state. The desktop keeps only host settings and its connection.
- A WSL path is never interpreted with Windows rules; translation happens only
  at the Windows boundary.
- Connection loss detaches a client; it does not complete a turn or change a
  workspace stage. A lost reply never resends a prompt, keystroke or worktree
  creation.
- Missing services are explicit errors or capabilities, never fabricated
  successes or a parallel copy of a screen.

## Implemented

- Portable core and Tauri-free adapters shared by desktop and runtime: board,
  conversation, processes, terminals, sessions, launch, accounts, profiles,
  tools, files, Git and OAuth.
- `prometeu-runtime` with a leased private root, per-session contexts, a saved
  catalog, resident attachment, snapshot recovery and safe replacement
  ([protocol](../../contracts/wsl-runtime.md)).
- The existing `index.html`/`src/main.ts` in a native window over an injected
  transport: desk, multiple tabs, launcher (in-place and new-branch worktrees),
  file editor and tree, binary viewers, terminals, Setup/Run, Git review and
  commit, workspace lifecycle and cleanup, project import, Explorer reveal,
  attachments, local MCP/plugin/skill libraries, tool selection and MCP OAuth
  ([coverage](../../contracts/windows-application.md)).
- Default WSL discovery, embedded runtime installation, automatic reconnection
  and a per-user NSIS installer.

## Remaining work

1. **Execution parity.** Managed login profiles and account switching, Claude and
   Antigravity registration, built-in delegation MCP, agent-generated plugins,
   actions, telemetry and usage, Cloud catalog and organization sync,
   collaboration and remote control.
2. **Workspace parity.** Existing-branch worktrees, multi-repository groups,
   plan mode and custom instructions in the launcher, explicit retry of
   interrupted launches, and moving remaining slow effects (initial indexing,
   tree mutations, plugin Git, provider start) off the runtime request loop.
3. **Cross-boundary effects.** Browser preview routing from WSL development
   servers, notifications, power handling, provider login links and capture
   import into the runtime's private attachment storage.
4. **Transport hardening.** Typed error codes instead of development strings,
   operation receipts or status lookup for uncertain mutations, and recovery of
   processes orphaned by an uncatchable host kill.
5. **Distribution.** Code signing, automatic updates, ARM64 or a broader Linux
   runtime baseline, missing-WebView2 and clean-VM installation checks, and a CI
   runner with WSL2 for the native journey.
6. **Existing-root adoption.** Opening a root previously used by the WSLg app
   requires stopping it first, an inventory of every persisted file and
   documented rollback; it is not implemented.

Each step needs its contract update, compatibility tests and native evidence. An
early Windows demo is a milestone, not the release gate.

## Release acceptance

A native Windows installation opens an existing WSL project, preserves its
history and accounts, creates a worktree, runs an agent, answers a request,
interrupts and resumes, uses terminals and Git, imports an attachment, previews
the app and recovers after desktop and runtime interruption. Existing macOS and
Linux behavior passes its regression gates. Browser mocks do not prove WSL
processes or native integration; follow the
[E2E scope policy](../../operations/development.md#e2e-scope).
