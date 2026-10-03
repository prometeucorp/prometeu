# WSL workspaces

Status: implemented under [ADR 0083](../decisions/0083-wsl-workspace-catalog.md).
This extends the [native WSL preview](wsl-preview.md) with saved workspaces in
existing Linux directories or newly created Git worktrees. The diagnostic preview exposes the selected Codex conversation and its supporting
terminal. The [shared Windows application](windows-application.md) additionally
addresses multiple conversation tabs and independent terminal docks. Opening a
worktree on a preexisting branch is not exposed yet.

## User flow and ownership

Connect with the same distribution, root, initial project, runtime and provider
paths as before. The initial project becomes the primary workspace. Add workspace
asks for a name and an existing absolute Linux directory, validated and
canonicalized by the runtime. It never copies files or creates a Git branch.
Multiple workspaces can use the same directory with separate conversation state;
their file changes share that directory.

Selecting a workspace replaces conversation and terminal snapshots. Its process
can already be running; Start / resume remains disabled until that process is
stopped. Other workspaces continue running, including turns awaiting approval.
Return to their workspace to see and answer their request. Stop affects only the
selected conversation; Close terminal affects its shell; Shut down runtime ends
all contexts. Disconnect and window closure preserve all resident contexts.

Work stage uses the existing board stage values and never stops or changes the
agent's observed status. The catalog is saved in WSL. Selection survives native
window and runtime restart. Transcripts/provider identities survive runtime
restart, but processes and terminals do not. Rerun Start / resume after that
restart. Unsent drafts survive selection in the same open window only.

## Git worktree creation

Create worktree asks for a name, an existing Git working directory, a new branch
and a starting reference (HEAD by default). The repository field starts with the
selected workspace's source repository. The runtime resolves the repository root
and the reference to a commit before creating a checkout under
`<private-root>/checkouts/<workspace-uuid>`. References must exist locally; no
fetch, commit, push, setup script or original-checkout switch is performed.
Uncommitted source changes are preserved and are not copied to the new checkout.
New branch names cannot be options or checkout-history expansions. Git refuses
existing branches and destinations; there is no force mode or automatic adoption.

The workspace records the canonical source project, checkout path, branch and
comparison base in the existing board fields. Creation leaves the current
selection and running conversations unchanged. Select the new workspace and use
Start / resume or Open terminal to execute in the new checkout.

`WorkspaceWorktrees` is an injected preparation port. The portable catalog validates
admission before preparation and registers the result through the same persistence
path as existing-folder workspaces. `GitWorktrees` uses the injected bounded
`CommandRunner<Command>`, argument arrays and closed input, with a twenty-second
total command deadline and 256 KiB per captured stream. Git remains on the Unix
side; Windows and portable rules never invoke Git or choose an OS implementation.

Git and catalog writes are separate commits. On a Git failure or timeout, the
error reports the attempted checkout path and diagnostic; no forced rollback or
retry occurs. Inspect any remaining checkout/branch before another attempt. On a
catalog save failure, preserve the successful checkout and branch and report its
path. Shut down the runtime and reconnect first: an atomic catalog rename may have succeeded before its
sync failed. If no workspace was saved, Add workspace can register that existing
folder (as an in-place workspace). A host crash can likewise leave an unregistered
checkout under `checkouts/`; this release does not automatically adopt or delete
it. Work stage, shutdown and disconnect never remove a checkout or branch.
The diagnostic preview has no cleanup control. The shared Windows app now uses
the existing archive/finish and separately confirmed cleanup flow; see
[workspace lifecycle](windows-application.md#workspace-lifecycle).

## Ports and operations

`prometeu-core::workspaces::Workspaces` receives `CatalogStore`, `WorkspaceFolders`
and a provider ID from composition. Worktree creation additionally receives
`WorkspaceWorktrees`. It validates names, IDs, stages and a maximum
of 64 workspaces and commits only after persistence succeeds. The host's
`ContextFactory` opens native context effects. Each context uses the existing
injected runtime/provider/process/storage/terminal dependencies.

The v1 [headless envelope](headless-runtime.md) advertises `workspaces.v1` and adds:

| Method | Arguments | Result |
| --- | --- | --- |
| `workspace_worktree` | `request: { title, path, branch, base }` | updated catalog; selection unchanged |
| `workspace_list` | none | catalog |
| `workspace_create` | `title`, `path` | updated catalog; selection unchanged |
| `workspace_select` | `id` | updated catalog after context preparation |
| `workspace_stage` | `id`, `stage` | updated catalog; execution unchanged |

The worktree operation requires the additive `worktrees.v1` capability. The bridge
returns `workspace_worktree_unsupported` before sending to an older host. The
isolated typed IPC adds `wsl_workspace_worktree` with the same request object.

Catalog is `{ v: 1, active: string, board: Board }`, using portable board models.
Tab fields in this persisted catalog are configuration, not live observations.
Use the conversation snapshot and its generation/readiness for live controls.
Unknown workspace IDs cannot select filesystem paths. Stored IDs are `primary`
or canonical UUID strings; the host derives child paths from them.

`WorkspaceClient` hides envelopes from Tauri. The isolated registry adds
`wsl_workspace_list`, `wsl_workspace_create`, `wsl_workspace_select` and
`wsl_workspace_stage`, with matching `src/wsl/ipc.ts`, `TauriSession` and
`MockSession` implementations. Production desktop IPC remains unchanged.
Domain errors use `workspace_*` codes translated at presentation; native I/O
errors retain the experimental transport's diagnostic strings.

`Session.select` disables sending and buffers events until the selected snapshot
arrives. Old generations are ignored, terminal renderer/input state is detached,
and discovery attaches the selected shell. No mutation is retried after a failure.
If the outcome is unknown, reconnect to reload the persisted selection.

## Storage and compatibility

The original root still owns `runtime.lock`, `runtime.json` and `transcript.jsonl`.
The host holds its lease before opening `workspaces.json`. The diagnostic launcher injects `PrimaryCatalog`; a missing catalog
creates a primary entry without moving/replacing history; malformed, unsupported
or oversized catalogs fail without overwriting saved data. The file has a maximum
of 8 MiB, mode 0600, and is synced and atomically replaced under the private root.
The shared application injects `EmptyCatalog` through the same `CatalogSeed` port.
An empty catalog has `active: ""` and no projects or workspaces; an unselected
application catalog may retain an empty active ID after workspace creation. Any
nonempty active ID must identify a stored workspace. Existing v1 catalogs load
unchanged, including their primary workspace; old readers requiring a primary entry
reject new empty catalogs without overwriting them.
Additional workspace roots live in `workspaces/<uuid>/`. The original session
keeps its workspace root; additional conversation sessions use `tabs/<session-uuid>/`
under that root, with the same isolated Store metadata, transcript and lease rules.
Closed tabs retain their directories but are removed from the catalog. Catalog
loading validates unique session IDs, supported choices and active-tab membership. They are opened lazily. Unreachable selected
directories fail explicitly on restart; no automatic relocation is attempted.

The old `start`, `command`, `snapshot`, `stop` and terminal operations now apply to
the selected context. Existing single-workspace clients remain valid. An older
runtime can reopen the primary root because its v1 metadata is unchanged; it
does not know about the additive catalog or child contexts. Desktop board adoption
and conversion are still forbidden. Native bootstrap configuration matching and
single attached-client limits remain unchanged.

## Verification

- `core/src/workspaces.rs`: independent session IDs, project reuse, stage/status
  separation, reload, validation and failed-save preservation through injected ports.
- `runtime/src/worktrees.rs`: real repositories with dirty source files, selected
  commit, nested and Unicode/quoted paths, invalid refs and existing-branch refusal.
- `runtime/tests/resident.rs`: two real provider processes and PTYs, switching
  during a turn, cross-workspace terminal-ID rejection, detach/reconnect, shutdown
  of all contexts, private catalog storage and catalog/history restoration after
  host restart, plus upgrade of an existing v1 root without rewriting its
  transcript or losing its provider identity.
- `src/wsl/workspaces.test.ts`: per-workspace drafts/history, late generation
  events, snapshot races and selection failure without retries.
- `e2e/wsl-preview.spec.ts`: creation form and keyboard selection preserve the
  rendered draft/history and restore focus. These DOM/focus risks are not proven
  by the controller tests and extend the existing preview journey.
- `scripts/test-wsl-native.mjs`: actual WebView2/native IPC/WSL creation and
  switching, a background turn completing while the native window is closed,
  restored selection and both existing provider/shell processes after reopening.
  Creates its second workspace through the native worktree form and verifies the
  original branch/dirty file and isolated committed file on disk. Uses a synthetic
  provider, not authenticated model calls.

Run instructions and remaining release limits live in the
[preview contract](wsl-preview.md#native-windows-acceptance).

Validation on 2026-09-30 passed 579 web unit tests, 620 Rust tests, 132 Chromium
scenarios and 48 WebKit scenarios, plus workspace Clippy, portable Windows target
checks, the Windows executable build and native acceptance. The initial
`npm run check` stopped before WebKit could launch because cached browser library
links referenced a removed temporary directory. After repairing the private
dependency cache, all WebKit scenarios ran successfully; Clippy ran separately.
Native acceptance used the synthetic provider described above and does not prove
authenticated provider behavior or complete desktop feature parity.

The worktree extension passed the full `npm run check` on 2026-09-30: 580 web
unit tests, 623 Rust tests, all 180 browser scenarios and workspace Clippy. The
Windows executable build and updated native acceptance also passed, including
Git checkout creation and preservation of the dirty source directory. Browser
checks used the private dependency cache described above. The new bridge
compatibility test refuses worktree creation on older hosts before sending a
mutation; an injected interrupted command verifies preservation of files after
an uncertain Git outcome.
