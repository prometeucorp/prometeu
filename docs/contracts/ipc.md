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
[`git.md`](git.md). They are additive: `workspace_diff` keeps the previous
contract and does not start meaning stage.

The `actions_save`, `action_start` and `action_pause` commands are described in
the [actions contract](actions.md). They use the existing `board` event.

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
[account contract](cloud-account.md); `catalog_state` and `catalog_refresh` are
in the [catalog contract](cloud-catalog.md). They do not return a Bearer or a
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
| `file-drag` | `file_drop.rs`, main webview | `{ type, paths, position?, id?, error? }` |

`paste_files` completes that path for the clipboard: with no arguments, it reads
the macOS general pasteboard and returns paths. Files copied in Finder keep the
original path; an image is written as PNG in
`<root>/attachments/<uuid>/pasted.png`, converting TIFF when that is the only
available representation. A clipboard with neither a file nor an image returns
an empty list, and the command is synchronous because reading the pasteboard
requires AppKit's main thread. Outside macOS, it returns an empty list.

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

`list_dir`, `read_file`, `read_bytes`, `write_file`, `find_paths` and `reveal`
receive in `id` the workspace **or** the project. A workspace resolves in the
worktree; a project resolves in the registered clone's folder, which is what
supports reading and editing a repository with no workspace on it at all. The
two id spaces do not collide, and `session.rs::cwd_of` is the only function that
performs that resolution — `dock.rs` imports it instead of repeating the rule. A
path outside the root is still refused.

`open_dock` accepts a project id only for a terminal: the shell only needs the
folder, and without a workspace there is no script variable to pass. Setup and
Run still require a workspace and answer `err.session.noWorkspace`.
`workspace_scripts` and `dock_state` already tolerated an id without a workspace
— they return an empty catalog and no port.

## Tool selection

The global, project and workspace layers of MCP, plugin and skill selection
([ADR 0043](../decisions/0043-layered-tool-selection.md)) travel as one shape:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

- `set_tools_global`: receives `{ mcp?, plugins?, skills? }`, each an optional
  `Selection`. An absent axis is not changed; `null` returns it to inherit. The
  global layer is a board field, so the result reaches the frontend through the
  existing `board` event and the command returns nothing on success.
- `set_workspace_mcp`, `set_workspace_plugins` and `set_workspace_skills`:
  receive `{ id }` plus the axis value as a `Selection`, replacing the previous
  `string[] | null`. Absent keeps the current value; `null` inherits.
- All four setters validate the payload before writing and answer an i18n-coded
  error instead of storing garbage: a malformed `Selection` fails with
  `err.tools.badPayload`, and an id on the wrong axis — a `skill-<id>` package
  on `plugins`, or a plain plugin on `skills` — fails with `err.tools.badAxis`.
- `workspace_tools`: receives `{ id }` and returns the effective set per axis,
  each item with its provenance — inherited, added, removed, or `cli` for the
  MCP servers the person's Claude configuration loads (ADR 0044) — and the
  project-declared items whose trust is pending or was rejected. An unknown
  workspace answers `err.session.noWorkspace`. It exists so the picker shows the
  result without reading the three layers.
- `mcp_inherited`: receives `{ id }` of a workspace and returns the MCP servers
  discovered from the CLI configuration for its working directory that the hub
  lacks, empty for other providers. It is the visible inherited base of the
  workspace picker (ADR 0044).
- `project_tools`: receives the `{ id }` of a project or a workspace and returns
  the `[tools]` declared by the primary repository, the settings file that
  declared it, the SHA-256 of that section and the stored decision, if any.
  `pending` is true only while no decision — approval or rejection — exists for
  the current hash, so an explicit rejection quiets the prompt until the
  declaration changes.
- `project_tools_trust`: receives `{ id, approved }` and records the decision on
  the board. The backend derives both the repository identity and the current
  declaration hash from `id`, so a recorded decision always binds to the
  declaration the command just read; a repository without a declaration is a
  no-op. One decision per repository: a new verdict replaces the previous one.

There is no new event: the global layer and the trust decisions are board
fields, and the workspace layer already was. `Selection` is a typed shape in
`src/types.ts`, and the mock implements every command above. ADR 0043 introduces
them in its interface phase; the parity test in `src-tauri/tests/mock.rs` is what
makes each one real.

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
