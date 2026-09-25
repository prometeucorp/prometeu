# TypeScript ↔ Rust IPC contract

Status: current contract; TypeScript commands and browser handlers share typed
arguments and results. Rust bindings remain manually synchronized.

## Sources and guarantees

The preview, element inspection and captures use the
[browser contract](browser.md). Those commands are additive and do not open
general IPC to the inspected page.

- `src/ipc.ts` owns `Commands`, with one argument/result pair for each command.
  `invoke` infers the result from the command and checks its arguments. Callers
  cannot supply an arbitrary result generic. `IpcArgs`, `IpcArguments`,
  `IpcResult`, `IpcCall`, and `IpcHandlers` support typed consumers and wrappers.
  Command unions must travel with their corresponding arguments as an `IpcCall`
  tuple; widening the command generic cannot bypass required arguments.
- `src/mock.ts` implements `IpcHandlers`. TypeScript checks every command's
  arguments and result. The browser's dynamic Tauri bridge performs one dispatch
  cast after checking that the command belongs to the handler map; plugin
  commands stay outside the application contract.
- `src-tauri/src/main.rs` registers Rust handlers in `generate_handler!`.
  `src-tauri/tests/mock.rs` checks exact name parity across this registration,
  the TypeScript map, and browser handlers.
- `src/ipc.test.ts` checks invocation forwarding and compile-time rejection of
  missing/invalid arguments, unknown commands, arbitrary result types, and
  incorrect mock results. `npm run typecheck` checks the entire frontend.

These checks do not generate Rust DTOs or validate runtime payloads. Rust
argument names, serde behavior, and serialized results must still match the
map and the existing boundary tests. Untrusted relay/control input remains
subject to backend validation. Errors keep their existing rejection format.
See [ADR 0024](../decisions/0024-typed-ipc.md).

Local notification commands and the `notification-open` event are described in
the [notification contract](notifications.md).

The `typesafe_status`, `typesafe_save_key`, `typesafe_remove_key`,
`typesafe_set_enabled` and `context_evaluate` commands are additive and
described in the [context evaluation contract](context-evaluation.md). No
command returns the API key; failures use `err.evaluation.*` codes and the
browser mock answers without a key or network.

## Command rules

- names use `snake_case` and must be unique;
- TypeScript arguments use the keys expected by Tauri's deserialization;
- a Rust return value must implement `Serialize`;
- a Rust argument must implement `Deserialize`;
- a fallible operation returns `Result<Response, Error>`;
- a domain error uses a stable code translated by the frontend;
- an external or I/O detail may accompany the code without becoming fixed UI
  text;
- a heavy operation does not block the main thread.

The commands for status, diffs, branches, commits and conflict resolution are in
[`git.md`](git.md). The retired `workspace_diff`, `resume_tab` and
`catalog_refresh` commands are no longer registered; see
[ADR 0043](../decisions/0043-retire-unused-ipc.md). The desktop frontend and
backend ship together. Persisted state and relay clients are unaffected.

The `actions_save`, `action_start` and `action_pause` commands are described in
the [actions contract](actions.md). They use the existing `board` event.

`set_workspace_mcp`, `set_workspace_plugins` and `set_workspace_skills` return
empty on success and validate the supplied selection before changing state.
Saving during a turn preserves every tab and existing process. The selection
applies at the next spawn or resume of a stopped process; sending another
message to an idle process alone does not reload tools. The browser mock
preserves the same validation and state-update behavior.

## Model catalogs

`agents` returns installation/capability descriptors with empty `models` lists.
`agent_models` accepts `{ agent: ProviderId }` and returns
`{ models: AgentModel[], fetchedAt: number }`, independently for each provider.
`AgentModel.additional` defaults to false when absent. Failures reject with an
object `{ code: string }` in the `err.modelsCatalog.*` namespace, never raw CLI
output. The catalog UI translates these errors, distinguishes an empty success
and retains the last known list only within the current account generation.

The former `claude_models` command is replaced; frontend, backend and mock ship
together, so no cross-version IPC support is required. The persisted board and
relay are unchanged. Native discovery, timeouts and freshness are specified in
[the runtime contract](agent-runtime.md#model-discovery).

## Private E2EE state

- `team_security`: no arguments, returns the security envelope or `null` only
  when the file does not exist. An invalid read returns an error.
- `team_security_set`: receives `{ state }`, validates the version and the 8 MiB
  limit and atomically writes the private file; returns empty or an error. An
  existing unreadable file is not overwritten.

The commands exist in Rust, in the typed registry and in the mock. The webview
needs the private identity for WebCrypto; it does not cross the WebSocket. The
scopes' internal schema belongs to `team-security.ts`; the payloads stay
`unknown` in the IPC map until that runtime validation; see
[persistence](persistence.md).

## Events currently emitted

The `cloud_status`, `cloud_login_start`, `cloud_login_poll`,
`cloud_login_cancel` and `cloud_logout` commands are in the
[account contract](cloud-account.md); `catalog_state` is in the
[catalog contract](cloud-catalog.md). Catalog refresh runs through
`cloud_status` with `refresh: true`. They do not return a Bearer or a
password to the webview. `cloud_organizations`, `cloud_relay_ticket` and the
`remoteControl` and `team` arguments of `set_shared` are in the
[organizations contract](cloud-organizations.md); only the short ticket crosses
IPC to authenticate the WebSocket.

| Event | Emitter | Emitted payload |
| --- | --- | --- |
| `board` | `state.rs` | the complete `Board` |
| `chat` | `chat.rs` | `[session, conversationEventV1JsonLine, seq]` |
| `chat-closed` | `chat.rs` | the session id |
| `pty` | `pty.rs` | `[session, bytes, seq]` |
| `pty-closed` | `pty.rs` | `[session, exitCode]` |
| `usage` | `usage.rs` | usage snapshot per local account ID |
| `accounts` | `accounts.rs` | registry, per-provider selection and pending login |
| `account-error` | `chat.rs` | a translatable error from a switch while sending the pending message |
| `machine` | `machine.rs` | machine state |
| `linear` | `linear.rs` | `LinearStatus` |
| `plugin-make` | `plugins.rs` | `[run, step]` |
| `plugin-made` | `plugins.rs` | `[run, error]` |
| `browser:url` | `browser.rs` | `[workspace, url]` |
| `workspace-preview` | `delegation.rs`, main webview only | `{ workspace_id, conversation_id }`, authorized local MCP navigation request |
| `file-drag` | `file_drop.rs`, main webview | `{ type, paths, position?, id?, error? }` |

`paste_files` completes that path for the clipboard: with no arguments, it reads
the system clipboard and returns paths. On macOS it reads the general
pasteboard: files copied in Finder keep the original path; an image is written
as PNG in `<root>/attachments/<uuid>/pasted.png`, converting TIFF when that is
the only available representation. On Linux it reads the GTK clipboard and
writes an image, converted to PNG, to the same path; copied files are not read.
A clipboard with neither a file nor an image returns an empty list. AppKit
reads synchronously on the main thread. GTK requests the image asynchronously
on the main thread; a worker converts and writes it. Linux rejects images above
64 MiB of pixel data or PNG output and fails after 5 seconds if the clipboard
owner does not respond. Elsewhere, it returns an empty list.

`src/paste.ts` calls it when the paste event carries files, and also when the
event carries no type at all: WebKitGTK hides a pasted image from the page that
way. A paste with text types keeps the browser's own paste.

`file-drag` adapts the native drag without changing Tauri's internal events. The
registration uses `on_webview_event`, filtering the `main` webview: with the
`unstable` feature, the runtime creates even the main webview as a child of the
window and does not deliver its drag to the `WindowEvent` listeners.
`enter`, `over`, `leave` and `drop` represent the gesture; `paths` is always a
list. `position` contains `{ x, y }` in the runtime's coordinates: on
macOS/wry 0.55 they are logical window points, without division by DPR.

For a macOS promise, `pending` replaces `drop`, with a unique `id` and the final
position. The frontend captures the target draft at that instant. The draft
counts pending receipts and blocks sending in all of its presentations;
completion or an error releases sending when the count reaches zero. `received`
completes the same `id` with the materialized local paths and an optional
`error`; it may bring valid files even when another one fails. Unknown or
duplicate receipts are ignored. The native wait has a 30-second limit, without
blocking the UI. `src/mock.ts` simulates the same phases.

The contract is additive, internal to the app/frontend bundle. There is no
change to `chat_send`, to V1 or to the relay: the agent still receives path
mentions. See [ADR 0018](../decisions/0018-native-file-promises.md).

The `accounts`, `account_select`, `account_remove`, `account_login` and
`account_login_cancel` commands are defined in [`accounts.md`](accounts.md).

The snapshot of the `usage` event and command is a map per local account ID. The
old `claude` and `codex` keys represent the original CLIs' accounts; additional
accounts use UUIDs, without changing the format of the values. Each entry has
`{ windows, at }`; each window has `{ kind, pct, resets, scope?, label? }`.
`scope` identifies independent quotas so that sparse updates for one model do
not erase the others, and `label` is optional external text for display.
Consumers must accept both fields being absent for compatibility with the
previous cache.

Tauri events are dynamic; the generic passed to `listen<T>` does not validate
the Rust payload at build time. A new event needs a test of the emitter and of
the consumer.

`chat_snapshot.text` may mix V1 and legacy lines after an import. `Timeline`
validates V1 and sends the rest to the legacy reader; historical
`prometheusV1Mirror` projections are ignored by the current reader. Prometeu
does not produce those projections in new logs.

## Root of the file commands

`list_dir`, `tree_git_status`, `tree_restore`, `read_file`, `read_bytes`, `write_file`,
`create_path`, `rename_path`, `trash_path`, `find_paths` and `reveal_path` receive in `id` the workspace **or** the project. A workspace resolves in its
working directory: a dedicated worktree, the clone itself, or the common parent
of multiple worktrees. A project resolves in the registered clone's folder, which is what
supports reading and editing a repository with no workspace on it at all. The
two id spaces do not collide, and `session.rs::cwd_of` is the only function that
performs that resolution — `dock.rs` imports it instead of repeating the rule. A
path outside the root is still refused.

`find_paths` returns at most 40 ranked entries, files and directories alike.
The optional `files: true` argument drops directories before candidate trimming
and that row limit, so Command-P quick open never loses a matching file to
better-ranked directories. Omitting it keeps the composer's `@` completion
behaviour; an older frontend that never sends it is unaffected, and
`session/find.rs` tests that directories cannot crowd out files.

`reveal_path` requires `rel`: an empty string opens the root, while a nonempty
path shows one entry of the tree or, from the Changes panel, a changed file,
whose repository-relative path the frontend prefixes with the repository's
folder under the workspace root. Keeping `rel` required preserves one IPC shape
for both uses. Finder selects a file with `-R`; systems served by `xdg-open` have no
selection flag, so a file there opens the folder holding it rather than the file,
which would launch another application over it. Resolution canonicalizes the
root, so a symlinked root opens its target and a missing root fails before the
file manager starts.

### File tree actions

The side tree works like a file manager through three commands, all relative to
that root:

| Command | Arguments | Effect |
| --- | --- | --- |
| `create_path` | `rel`, `dir` | creates an empty file or a folder; never replaces an existing entry |
| `rename_path` | `from`, `to` | renames or moves an entry; refuses an existing target and moving a folder into itself |
| `trash_path` | `rel` | moves the entry to the system trash instead of deleting it |

The last component of a path must be one plain name: empty, `.`, `..`, `/`,
`\` and NUL answer `err.files.name`, and an existing target answers
`err.files.exists` with `name`. Git metadata is off limits at any depth: a
`.git` component anywhere in the path (in any case), or a parent that resolves
into one through a symlink, answers `err.files.name` for the new entry, both
ends of a rename and the trashed entry. Only the parent folder is resolved
through symlinks and must stay inside the root; the entry itself is not
followed, so renaming or trashing a symlink acts on the link.

A case change of the same name in the same folder is the only rename allowed
onto a name that answers as existing. On a case-insensitive disk, the macOS and
Windows default, the new spelling finds the source itself; the rename is
allowed when the folder lists no entry spelled exactly that way, so a second,
distinct entry (two symlinks to one file included) is still refused. Such a
rename goes through a random temporary name in the same folder, since some file
systems ignore a rename that changes only case, and is undone if the second
step fails. Case-only renames run one at a time, so two concurrent detours can
never meet on the same temporary name and replace a file. The same test covers both kinds of disk: on Linux CI
(case-sensitive) it checks the listing check and the refusal of a second
entry, and on macOS CI (case-insensitive) it checks the case-only rename in
both directions. Trash is used so
untracked work, which Git cannot bring back, is still recoverable; a failure
answers `err.files.trash` with the system's `cause`. `trash_path` is async
because the platform trash can be slow, notably through Finder on macOS.

The UI moves the open file tabs and unsaved drafts of a renamed entry and drops
those of a trashed one, in the workspace or project where the action started.
The commands are new and additive: frontend and backend ship together, no
persisted field changes, and relay clients never see them. `src/mock.ts` edits
its sample tree with the same refusals for names and conflicts.

The former `reveal` command is retired under the bundled IPC policy in
[ADR 0043](../decisions/0043-retire-unused-ipc.md). Frontend and backend ship
together, so no cross-version IPC shim applies; persisted state and relay
clients are unchanged.

`open_dock` accepts a project id only for a terminal: the shell only needs the
folder, and without a workspace there is no script variable to pass. Setup and
Run still require a workspace and answer `err.session.noWorkspace`.
`workspace_scripts` and `dock_state` already tolerated an id without a workspace
— they return an empty catalog and no port.

## Tool selection

The global, project and workspace layers of MCP, plugin and skill selection
([ADR 0045](../decisions/0045-layered-tool-selection.md)) travel as one shape:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

- `set_tools_global`: receives `{ mcp?, plugins?, skills? }`, each an optional
  `Selection`. An absent axis is not changed; `null` returns it to inherit. The
  global layer is a board field, so the result reaches the frontend through the
  existing `board` event and the command returns nothing on success.
- `set_workspace_mcp`, `set_workspace_plugins` and `set_workspace_skills`:
  receive `{ id }` plus the axis value as a `Selection`, replacing the previous
  `string[] | null`. Absent keeps the current value; `null` inherits. Native
  handlers inspect the JSON request body so Tauri cannot collapse null into an
  absent argument; the wire shape is unchanged (ADR 0047).
- All four setters validate the payload before writing and answer an i18n-coded
  error instead of storing garbage: a malformed `Selection` fails with
  `err.tools.badPayload`, and an id on the wrong axis — a `skill-<id>` package
  on `plugins`, or a plain plugin on `skills` — fails with `err.tools.badAxis`.
- `workspace_tools`: receives `{ id, agent? }` and returns the effective set per
  axis,
  each item with its provenance — inherited, added, removed, or `cli` for the
  MCP servers the person's Claude configuration loads (ADR 0046) — and the
  project-declared items whose trust is pending or was rejected. An unknown
  workspace answers `err.session.noWorkspace`. It exists so the picker shows the
  result without reading the three layers. `agent` selects the displayed tab's
  provider; omission uses the workspace provider, preserving existing callers.
- `mcp_inherited`: receives `{ id, agent? }` of a workspace and returns the MCP
  servers
  discovered from the CLI configuration for its working directory that the hub
  lacks, empty for other providers. The optional provider has the same default
  as `workspace_tools`; the frontend cache includes both workspace and provider.
  It is the visible inherited base of the workspace picker (ADR 0046).
- `project_tools`: receives the `{ id }` of a project or a workspace and returns
  the `[tools]` declared by the primary repository, the settings file that
  declared it, the SHA-256 of that section and the stored decision, if any.
  `pending` is true only while no decision — approval or rejection — exists for
  the current hash, so an explicit rejection quiets the prompt until the
  declaration changes.
- `project_tools_trust`: receives `{ id, hash, approved }`, where `hash` is the
  version displayed by the dialog. The backend derives repository identity and
  current hash from `id`, compares the displayed hash, and records a decision
  only on a match. A changed or removed declaration returns `err.tools.changed`
  without writing. Missing hash arguments fail closed. The dialog must reopen
  before deciding on a new version. One decision per repository: a matching new
  verdict replaces the previous one. See [ADR 0047](../decisions/0047-tool-selection-boundaries.md).

The trust entry uses `project_tools.pending`, not the presence of pending items.
Project and workspace menus also open declarations independently of the hub,
including declarations containing only removals or an empty replacement.

There is no new event: the global layer and the trust decisions are board
fields, and the workspace layer already was. `Selection` is a typed shape in
`src/types.ts`, and the mock implements every command above. ADR 0045 introduces
them in its interface phase; the parity test in `src-tauri/tests/mock.rs` is what
makes each one real.

## Starting from a skill

- `plugin_skills`: no arguments; returns `{ plugin, name, description }[]` for
  the skills shipped by installed local-folder plugins, excluding standalone
  skill packages. See the [plugin hub](plugin-marketplace.md#skills-inside-packages).
- `create_workspace`: `draft.kickoff` is an optional `<package>/<skill>` string;
  absent or empty starts without a skill, so older callers and the delegation
  payload keep working. An unknown skill rejects with `err.kickoff.missing` and
  a provider without `workspacePluginSelection` with `err.kickoff.unsupported`,
  both before the card is published.

The browser mock mirrors the catalog from its fixture plugins and the same
validation. Semantics are in the
[runtime contract](agent-runtime.md#skill-kickoff) and
[ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md).

## Legacy import

`legacy_import_plan` does not change state. It returns the source, one of the
situations `ready | missing | imported | targetNotEmpty | invalid`, the preview
counts and, when applicable, the error, timestamp and backup path.

`legacy_import_run` does not receive paths from the presentation: source and
destination are resolved by the backend. It repeats every validation, refuses an
open Prometheus and an occupied destination, runs the import and returns the
same DTO in the `imported` state. The board change is still published through
the `board` event.

## Change checklist

When creating or changing a command:

1. change the Rust function and its error type;
2. register the handler in `main.rs`;
3. update the command name, argument shape, and result in `src/ipc.ts`;
4. implement it or consciously refuse it in `src/mock.ts`;
5. update every TypeScript consumer;
6. add a test of the argument and return shape;
7. document compatibility when persisted state is involved.

When creating or changing an event:

1. define the payload and its ownership;
2. test serialization in the emitter;
3. validate the payload in the consumer when it comes from an untrusted
   boundary;
4. make sure listeners are torn down along with the screen's lifecycle;
5. update the table above.

## Direction of evolution

The implemented map requires no new dependency:

```ts
type Commands = {
  load_board: { args: undefined; result: Board };
  chat_send: { args: { session: string; text: string }; result: void };
};
```

A future step may generate bindings from Rust DTOs. The tool must be chosen in
an ADR after a small proof covering:

- enums with `serde(rename_all)`;
- `Option` and default fields;
- serialized errors;
- events, in addition to commands;
- integration with Tauri 2 and the Rust version used in the project.

Generating types for whole internal structs is not the goal. Only boundary DTOs
should appear in the binding.

## Feedback

`feedback_capture` is additive, takes no arguments, and returns a base64 PNG or
`null`. See [capture and limits](feedback.md). The mock returns a fictional
image.

`feedback_send` receives `{ report }` and returns no value. The backend delivers
it to the Cloud with the account credential, which never enters the webview, and
returns an error with an i18n code: `feedback.needAccount` without an account or
on 401, `feedback.rateLimit` at the limit, `feedback.uncertain` with `{id}` on
uncertain delivery and `feedback.sendError` for the rest. The mock records the
report and nothing leaves the machine.

## Built-in MCP catalog entry

`mcp_hub`, `mcp_save` and `mcp_remove` return the virtual `prometeu` entry with
`config: {type: "stdio", builtin: true}` in addition to mutable registry items.
The built-in cannot be overwritten or removed (`err.mcp.builtin`). It contains
no executable path or credential. `load_board` includes additive
`delegations`, defaulting to an empty list in Rust. There are no new IPC
commands; the browser mock mirrors the catalog and selection behavior, and
does not execute MCP agents. See [embedded MCP](embedded-mcp.md).

`workspace-preview` is additive and local-only. The emitter checks delegation
ownership and workspace availability. The application-lifetime listener reloads
the local board and rechecks the exact workspace/conversation before opening
the existing preview. The browser mock can emit the same event for E2E tests.
It contains no URL or arbitrary action and does not cross the relay.

## Local telemetry

`telemetry_summary`, `telemetry_events`, `telemetry_export` and `telemetry_clear`
are local-only queries/export/erasure commands. Filters, cursor, coverage and
return values are typed in `src/telemetry.ts`; Rust handlers and the browser mock
share command names. No arbitrary SQL or native provider objects cross this
boundary. The export destination comes from the native save dialog and is a
`.jsonl` file. Erasure always clears the entire telemetry dataset. See the
[telemetry contract](telemetry.md) for query cohorts and privacy guarantees.
