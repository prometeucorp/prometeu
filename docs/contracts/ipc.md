---
type: Contract
title: "TypeScript ↔ Rust IPC contract"
description: "Typed desktop commands, events, browser mocks and the compatibility of serialization."
status: stable
---

# TypeScript ↔ Rust IPC contract

This is the current contract. TypeScript commands and browser handlers share
typed arguments and results. A person must keep the Rust bindings aligned
manually.

## Sources and guarantees

The preview, the element inspection and the captures use the
[browser contract](browser.md). Those commands are additive. They do not give
general IPC access to the inspected page.

- `src/ipc.ts` owns `Commands`, with one pair of argument and result for each
  command. `invoke` gets the result type from the command and examines its
  arguments. Callers cannot supply a result generic of their choice. `IpcArgs`,
  `IpcArguments`, `IpcResult`, `IpcCall`, and `IpcHandlers` are for typed
  consumers and wrappers. A command union must go with its arguments as an
  `IpcCall` tuple. A wider command generic cannot bypass mandatory arguments.
- `src/mock.ts` implements `IpcHandlers`. TypeScript examines the arguments and
  the result of each command. The dynamic Tauri bridge of the browser does one
  dispatch cast after it makes sure that the command is in the handler map.
  Plugin commands are not part of the application contract.
- `src-tauri/src/main.rs` registers the Rust handlers in `generate_handler!`.
  `src-tauri/tests/mock.rs` makes sure that this registration, the TypeScript
  map and the browser handlers have the same names.
- `src/ipc.test.ts` examines the forwarding of calls. It also makes sure that
  compilation rejects missing or incorrect arguments, unknown commands, result
  types that the caller selects, and incorrect mock results. `npm run typecheck`
  examines the full frontend.

These checks do not make Rust DTOs or validate payloads at runtime. The Rust
argument names, the serde behavior and the serialized results must continue to
agree with the map and with the current boundary tests. The backend continues
to validate relay and control input that is not trusted. Errors keep their
current rejection format. See [ADR 0024](../decisions/0024-typed-ipc.md).

## Conversation mode

`set_conversation_mode({ mode: "chat" | "terminal" })` saves the default for new
conversations and publishes the current `board` event. It returns no value. It
does not change saved tabs or start processes again. Unknown mode values cause a
deserialization failure. The browser mock keeps the same default and applies it
to new tabs, workspace conversations and actions.

The macOS desktop accepts the two modes and shows the preference in Settings /
General. Linux accepts `chat`. It rejects `terminal` with
`err.terminalAgent.unsupported` before it saves or prepares a process. On
Linux, the preference control stays hidden. The desktop workspace creation
resolves and validates the mode before it publishes its preparation card or
changes the file system. The [board contract](persistence.md#board) gives the
storage defaults. The producer and consumer examples include a chat mode that is not
written and an explicit terminal mode.

The same `board`/`load_board` payload can include
`Tab.native_dispatch: { id: string, text: string }` for one native input that
waits for durable acceptance. Older tabs do not have it. The interface shows its
text before `pending_prompt`. This also applies after an interrupted delivery.
The ID is not a control on the screen or a new IPC argument, and the interface does not
parse it. The desktop queue owns the retries and the comparison of receipts.
The typed Board and the browser mock keep this additive state. See
[pending session input](persistence.md#pending-session-input).

The macOS native surface uses these commands:

- `agent_terminal_snapshot({ session })` returns
  `{ data: number[], seq: number, running: boolean, generation?: string, screen?: Snapshot, queryReplies?: "host" }`.
- `agent_terminal_resume({ session })` resumes without an artificial prompt.

When a person opens a local terminal tab and the snapshot shows a stopped
process, the tab calls resume automatically. Then it gets the new snapshot. The
app combines view requests that occur at the same time. The command is idempotent
for a connected process under the session input gate. A failure gives a manual
retry. It does not start an automatic retry loop. Tests in
`src/terminal-conversation.test.ts` examine the resume, views at the same time and
results that come after navigation or after the view is removed.

A native process that the app keeps has a generation, also after it exits. If
there is no process, there is no generation. Agent `pty` events add that
generation as a fourth item of the tuple. `pty-closed` adds it as a third item.
Native `pty_write` and `pty_resize` must have the optional `generation`
argument with the same value. The current dock terminal tuples and callers do not have
it. The frontend rejects late output and exit events from a replaced process.
The backend rejects input and resize requests for a previous generation.

`pty_write` accepts exactly one of `data: string` and `bytes: number[]`. Binary
input keeps the bytes. It does not encode the xterm `onBinary` string as UTF-8.
The app sends native input one write at a time for each attachment. It rejects
writes in the queue for a generation that is not current. Resize and input
handlers do not operate on the GUI thread.

`agent-terminal-started` sends `[session, generation]` after the app publishes
the process and the tab. This includes account changes that keep the tab ready.
The view uses this event only to request the current snapshot. The view does
not use it as a proof that the process of the event is current. An attachment
that is not complete combines these events. It releases the subscription when the
view is removed.

`screen` follows the [terminal state contract](terminal-state.md). Its `seq` is
a byte offset. The outer `seq` stays the sequence of the related PTY chunk.
When `screen` is there, `data` is empty and `queryReplies` is `host`. The
renderer sets again the dimensions, the buffers and the modes. It fills the
pending UTF-8 and filters live chunks by the outer sequence. It stops automatic
replies, but it keeps keyboard, IME, paste, focus and binary input.

If the helper has a failure, the renderer does not restore the screen. It does
not show a raw tail as a full screen. Tests in `src/term.test.ts` examine these
boundaries, overlap, generations, the removal of the view and the sequence of
resize events. The browser mock keeps different native generations and does not
call a provider CLI.

## Archive launch admission

While the archive or finish hook runs, the desktop does not accept new-tab
launches or resumed process launches. It returns `err.session.archiving`
through the current string-error format. After the archive, launches return
`err.session.archived` until the workspace is restored. A manual cleanup at the
same time uses the same gate. The success payloads and the stored workspace
shapes do not change. Tests in `src-tauri/src/session.rs` examine the admission
during the shutdown. They also examine the publication of launches that are not
complete before the archive collects its last stop targets.

## Executable serialization examples

`src-tauri/src/boundary_contract.rs` serializes the production board, snapshot, model
and capability types into `fixtures/backend-contract.json`. It also serializes the
output of synthetic Claude and Codex adapter scenarios. Its usual Rust test
must find that the fixture is fully equal. `npm run contracts:update` makes
the fixture again explicitly. Examine the diff before you accept a contract
change.

`npm run test:contracts` compares those JSON literals with the TypeScript
result types of the commands and with the canonical event types. Then it sends
the events through the production validator and the timeline. The check keeps
literal enum values and nullability, and it accepts more backend fields.
`npm run check` runs the producer check and the consumer check. A run of only
the frontend does not prove that the fixture agrees with Rust.

The coverage includes these items:

- Workspaces with tabs and empty workspaces.
- All tab statuses and provider IDs.
- Inherited and replaced tool selections.
- Nullable fields and a kickoff that is not written.
- Model fields in camelCase.
- Typical conversation events.

Empty action and delegation collections do not prove their nested contracts.
These examples are not a full schema generation, a live certification of
providers or a call through a Tauri webview that operates. Command arguments and
results that the examples do not show must continue to have their current
tests.

Local Rust workspaces do not have `remote`. The TypeScript presentation field
is optional, or null for browser fixtures. The collaboration supplies it only
for remote workspaces. The change adds no backend field or stored format.

`create_project` receives `{ directory: string, name: string }` and returns the
current `Project` shape. The parent must be an absolute folder that is there.
The desktop expands `~/`. The trimmed name must be one folder component. These
names are rejected:

- Empty names, `.` and `..`.
- `.git`, with capital letters or small letters.
- Names with slashes, backslashes or control characters.

The native Git adapter makes the parent path canonical and makes the child
folder exclusively. It rejects current files, folders and symlinks and does not
change them. It runs `git init --template=` without the environment overrides
for the repository location. Then it registers the result and publishes
`board`. It makes no commit, remote, workspace or project files. A Git identity
or a Cloud account is not necessary.

If Git has a failure, the registration does not occur. The cleanup removes only
an empty child folder. It keeps Git state that is not complete, or files that a
different process wrote at the same time, for inspection. If the registration
has a failure after the initialization, the initialized folder stays available
for **Add local folder**. The current `add_project` behavior and the stored
formats do not change. The native tests are in `crates/git/src/tests.rs`.

`reorder_projects` receives `{ ids: string[] }`, returns empty and changes the
sequence of the board `projects` to agree with the list. Projects that are not
in `ids` keep their relative sequence after the listed projects. The command
ignores unknown IDs. Thus a project that a person adds during a drag is not
lost. The change goes to the frontend through the `board` event. The browser
mock applies the same rule. Test:
`project_order_follows_ids_and_keeps_unlisted_projects_last` in
`src-tauri/src/session.rs`.

`group_projects` receives `{ ids: string[], group: string | null }`, returns
empty and sets `group` on the listed projects. The command trims the name. An
empty or null name removes the projects from their folder. This command moves a
project, renames a folder (all its members get the new name) and removes a
folder. Thus there is no separate folder entity. The command ignores unknown
IDs. The change goes to the frontend through the `board` event. The browser mock
applies the same rule. Test:
`project_folders_persist_trimmed_and_clear_on_empty_names` in
`src-tauri/crates/core/src/projects.rs`.

The [notification contract](notifications.md) gives the local notification
commands and the `notification-open` event.

The `typesafe_status`, `typesafe_save_key`, `typesafe_remove_key`,
`typesafe_set_enabled` and `context_evaluate` commands are additive. The
[context evaluation contract](context-evaluation.md) gives them. No command
returns the API key. Failures use `err.evaluation.*` codes. The browser mock
answers without a key or a network.

## Command rules

- Names use `snake_case` and must be unique.
- TypeScript arguments use the keys that the Tauri deserialization expects.
- A Rust return value must implement `Serialize`.
- A Rust argument must implement `Deserialize`.
- An operation that can fail returns `Result<Response, Error>`.
- A domain error uses a stable code that the frontend translates.
- An external or I/O detail can go with the code. It does not become permanent UI
  text.
- A slow operation does not block the main thread.

[`git.md`](git.md) gives the commands for the status, the diffs, the branches,
the commits and the conflict resolution. The retired `workspace_diff`,
`resume_tab` and `catalog_refresh` commands are not registered. See
[ADR 0043](../decisions/0043-retire-unused-ipc.md). The desktop frontend and
backend ship together. This does not change the stored state or the relay
clients.

The [actions contract](actions.md) gives the `actions_save` and `action_start`
commands. They use the current `board` event.

`set_workspace_mcp`, `set_workspace_plugins` and `set_workspace_skills` return
empty on success. They validate the supplied selection before they change the
state. A save during a turn keeps all tabs and the current processes. The
selection applies at the next start or resume of a stopped process. A new
message to an idle process alone does not load the tools again. The browser
mock keeps the same validation and state update behavior.

## Model catalogs

`agents` returns the installation and capability descriptors with empty
`models` lists. `agent_models` accepts `{ agent: ProviderId }` and returns
`{ models: AgentModel[], fetchedAt: number }`, independently for each provider.
`AgentModel.additional` is false when it is missing. Failures reject with an
object `{ code: string }` in the `err.modelsCatalog.*` namespace. They do not
return raw CLI output. The catalog UI translates these errors and shows an empty
success differently. It keeps the last known list only in the current account
generation.

This command replaces the previous `claude_models` command. The frontend, the
backend and the mock ship together. Thus IPC between different versions is not
necessary. The stored board and the relay do not change.
[The runtime contract](agent-runtime.md#model-discovery) gives the native
discovery, the timeouts and the age of the data.

## Private E2EE state

- `team_security`: no arguments. It returns the security envelope, or `null`
  only when the file is not there. A read that is not correct returns an error.
- `team_security_set`: receives `{ state }`. It validates the version and the
  limit of 8 MiB and writes the private file atomically. It returns empty or an
  error. It does not write over a current file that it cannot read.

The commands are in Rust, in the typed registry and in the mock. The webview
must have the private identity for WebCrypto. The identity does not go through
the WebSocket. `team-security.ts` owns the internal schema of the scopes. The
payloads stay `unknown` in the IPC map until that runtime validation. See
[persistence](persistence.md).

## Events currently emitted

The [account contract](cloud-account.md) gives the `cloud_status`,
`cloud_login_start`, `cloud_login_poll`, `cloud_login_cancel` and
`cloud_logout` commands. The [catalog contract](cloud-catalog.md) gives
`catalog_state`. The catalog refresh operates through `cloud_status` with
`refresh: true`. These commands do not return a Bearer token or a password to
the webview. The [organizations contract](cloud-organizations.md) gives
`cloud_organizations`, `cloud_relay_ticket`, and the `remoteControl`, `team`
and `rights` arguments of `set_shared` and `set_confirm_messages`. Only the
short ticket goes through IPC to authenticate the WebSocket.

| Event | Emitter | Emitted payload |
| --- | --- | --- |
| `board` | `state.rs` | the full `Board` |
| `chat` | `chat.rs` | `[session, conversationEventV1JsonLine, seq]` |
| `chat-closed` | `chat.rs` | the session id |
| `pty` | `pty.rs` | `[session, bytes, seq]` |
| `pty-closed` | `pty.rs` | `[session, exitCode]` |
| `usage` | `usage.rs` | the usage snapshot for each local account ID |
| `telemetry-changed` | `telemetry.rs` | `null`. The local capture of a completed turn, an app call or the PR history is complete. |
| `accounts` | `accounts.rs` | the registry, the selection for each provider and the pending login |
| `account-error` | `chat.rs` | a translatable error from an account change during the send of the pending message |
| `machine` | `machine.rs` | the machine state |
| `linear` | `linear.rs` | `LinearStatus` |
| `plugin-make` | `plugins.rs` | `[run, step]` |
| `plugin-made` | `plugins.rs` | `[run, error]` |
| `browser:url` | `browser.rs` | `[workspace, url]` |
| `workspace-preview` | `delegation.rs`, only the main webview | `{ workspace_id, conversation_id }`, an authorized local MCP navigation request |
| `file-drag` | `file_drop.rs`, the main webview | `{ type, paths, position?, id?, error? }` |

`linear_issues({ force })` returns the assigned and the available issues with a
fetch time. It also returns an optional `available_error` when the discovery
has a failure after Mine is correct. `linear_claim({ id })` returns the issue
that it assigned after Linear confirms the change. `linear_status` and `linear`
include `can_assign`. This value identifies credentials that have the `write`
scope. See [Linear issues](linear-issues.md).

`github_status`, `github_connect`, `github_disconnect`, `github_issues`,
`github_claim`, `github_issue_open`, `github_projects` and `github_prepare` give
these functions:

- The GitHub App connection.
- The normalized discovery and the claim.
- The validated navigation to the source.
- The preparation of PR worktrees.

The `github` event sends the connection status. See
[GitHub issues](github-issues.md). `github_feed`, `github_subjects` and
`github_detail` read the Cloud notification feed. They get titles and text when
it is necessary. See [GitHub notifications](github-notifications.md).

`paste_files` completes that path for the clipboard. It has no arguments. It
reads the system clipboard and returns paths:

- On macOS, it reads the general pasteboard. Files that a person copied in
  Finder keep the initial path. The command writes an image as PNG in
  `<root>/attachments/<uuid>/pasted.png`. It changes TIFF to PNG when TIFF is
  the only available format.
- On Linux, it reads the GTK clipboard. It changes an image to PNG and writes it
  to the same path. It does not read copied files.
- On other systems, it returns an empty list.

A clipboard without a file or an image returns an empty list. AppKit reads
synchronously on the main thread. GTK requests the image asynchronously on the
main thread, and a worker changes it and writes it. Linux rejects images with
more than 64 MiB of pixel data or PNG output. If the clipboard owner does not
answer in 5 seconds, the command has a failure.

`src/paste.ts` calls the command when the paste event has files. It also calls
it when the event has no type, because WebKitGTK hides a pasted image from the
page in this way. A paste with text types keeps the paste of the browser.

`file-drag` changes the native drag into an app event. It does not change the
internal events of Tauri. The registration for usual files uses
`on_webview_event` and filters the `main` webview. With the `unstable` feature,
the runtime makes the main webview as a child of the window. Then the
`WindowEvent` listeners do not receive its drag. `enter`, `over`, `leave` and
`drop` show the gesture. `paths` is always a list. `position` contains
`{ x, y }` in the coordinates of the runtime. On macOS/wry 0.55, these are
logical window points, without division by DPR.

For a macOS file promise, the reception starts synchronously in the
`performDragOperation:` callback of the main native view. This occurs before
AppKit ends the drop operation. The Tauri event in the queue is too late to
request the reception. A processed promise bypasses the usual drop callback of
Wry. Thus it does not make a duplicate `drop` event.

`pending` replaces `drop`, with a unique `id` and the last position. The
frontend captures the target draft at that time. The draft counts the pending
receipts and blocks the send in all of its presentations. When the count is 0
after a completion or an error, the send is available again. `received`
completes the same `id` with the local paths of the files and an optional
`error`. It can bring correct files also when a different file has a failure. The
frontend ignores unknown or duplicate receipts. The native wait has a limit of
30 seconds, and it does not block the UI. `src/mock.ts` simulates the same
phases.

The contract is additive and internal to the app and the frontend bundle. It
does not change `chat_send`, V1 or the relay. The agent continues to receive
path mentions. See [ADR 0018](../decisions/0018-native-file-promises.md).

[`accounts.md`](accounts.md) gives the `accounts`, `account_select`,
`account_remove`, `account_login` and `account_login_cancel` commands.

The snapshot of the `usage` event and command is a map with one entry for each
local account ID. The previous `claude` and `codex` keys are the accounts of the
initial CLIs. Other accounts use UUIDs, and the format of the values does not
change. Each entry has `{ windows, at }`. Each window has
`{ kind, pct, resets, scope?, label? }`. `scope` identifies independent quotas.
Thus an update for only one model does not erase the others. `label` is
optional external text for the display. Consumers must accept that the two
fields are not there, for compatibility with the previous cache.

`usage_refresh({ provider? })` requests one immediate attempt, with a limit, for
selected profiles whose data is not current. It does not return quota data or credentials.
The native scheduler publishes the current `usage` and `accounts` events when
the values change. The browser mock implements the command as an empty
operation.

Tauri events are dynamic. The generic in `listen<T>` does not validate the Rust
payload during the build. A new event must have a test of the emitter and a
test of the consumer.

`background_context` returns `{ visible, focused, power, revision }`. `power` is
`ac`, `battery` or `unknown`. The `background-context` event has the same shape.
The app sends it only when the native state that it sees changes. The frontend
subscribes before it requests the snapshot. It ignores an older revision if the
event comes first. `visible` is false for a main window that is hidden or
minimized. `focused` is true only when that native window shows on the screen
and has the focus.

macOS reads the IOKit snapshot of the power sources. Linux reads
`/sys/class/power_supply`. It ignores the batteries of peripheral devices
(`scope` `Device`). It thinks that a Mains, USB or wireless supply that is online
is AC. If the app cannot read the supply, or the read has a failure, the value is
`unknown`. Then optional work uses the battery budget.

Window events refresh the context quickly. A fallback reads the visibility each
five seconds on the event-loop thread. A different fallback reads the power each
60 seconds. Each writer changes only the fields that it read. The app wakes the
native samplers directly on each change. Thus their return to the foreground
occurs also without the webview. The UI uses this context only to select when it refreshes.
Agent processes, transcript capture, terminals and sharing continue
independently. If the native context does not start, the Git presentation uses
the visibility and focus of the document as a fallback. The transcript paint
stays enabled.

`set_awake` accepts `{ mode: "off" | "system" | "display" }`:

- `display` starts the macOS `caffeinate -d -i -s -w <app pid>` assertion that
  the current selections use.
- `system` starts `-i -s -w` without `-d`.
- `off` stops the two modes.

The app also stops the child process on a usual exit. On an exit that is not
expected, `-w` connects the child process to the PID of the app. On Linux, the UI
offers only `off` and `display`. Its current `systemd-inhibit` implementation
depends on the desktop environment. The command is idempotent for a mode that
operates at that time. It replaces an assertion when the mode changes. The
desktop IPC ships with its frontend at the same time. Thus the app does not
accept an older `{ on: boolean }` caller.

`set_resource_detail({ open: boolean })` tells if the native process panel is
open. It wakes the resource sampler to calculate its next time again. The
status bar sends it when the panel opens and closes. The current `machine`
command and event keep `{ rss, cpu, procs, terms, ports }`. Each process
continues to have a numeric `hist` array. The command returns the cached process
snapshot with the current terminal and port counts.

A dock PTY can start or stop, and a workspace port can change. Then the app
sends that snapshot if its counts or ports change. It does not
take a `ps` sample. This also applies while the native window does not have the
focus. The compact sample in the foreground occurs each 15 seconds on AC, or
each 30 seconds on battery or unknown power. An open panel uses 3 or 5 seconds.
A native window that is hidden, minimized or without the focus starts no `ps`
sample. It takes one sample when it comes back.

The CPU value uses the measured time between samples. If the interval is more
than two times the usual interval, each sparkline history starts again. This occurs,
for example, with a hidden window or a Mac in sleep. Compact samples keep their
history. Thus the panel opens with the recent compact points before the
detailed points. The frontend updates the changed resource text and process
fields. It does not replace other status controls.

`chat_snapshot.text` can mix V1 and legacy lines after an import. `Timeline`
validates V1 and sends the other lines to the legacy reader. The current reader
ignores historical `prometheusV1Mirror` projections. Prometeu does not write
those projections in new logs. The portable conversation stream builds the
snapshots. Its
[compatibility tests](../../src-tauri/crates/core/src/conversation/stream/tests.rs)
examine mixed history, sequence resets and the snapshot shape. A retired terminal
stops late output and close events before a replacement uses its key.

## Root of the file commands

`list_dir`, `tree_git_status`, `tree_restore`, `file_base`, `read_file`,
`read_bytes`, `write_file`, `create_path`, `rename_path`, `trash_path`,
`find_paths` and `reveal_path` receive the workspace **or** the project in `id`.
A workspace resolves to its working folder. This is a dedicated worktree, the
clone, or the common parent of more than one worktree. A project resolves to the
folder of the registered clone. Thus a person can read and edit a repository
that has no workspace. The two id spaces do not collide.
`session.rs::cwd_in`, behind `cwd_of`, is the only function that does that
resolution. `dock.rs` imports it and does not repeat the rule. The commands
continue to refuse a path that is not in the root.

`find_paths` returns a maximum of 40 ranked entries, files and folders. The
optional `files: true` argument removes folders before the candidate limit and
that row limit. Thus Command-P quick open does not lose a file that agrees with the
query to folders with a better rank. If the argument is not there, the `@` completion
of the composer keeps its behavior. An older frontend that does not send it
continues to operate. `crates/files/src/search.rs` examines that folders cannot
push out files.

In the view of a local project, Command-F (Control-F on other systems) opens
the same search. It sends the ID of the selected project. Thus all results come
from the registered clone of that project. A project on a remote machine has no
project view. Its heading in the sidebar opens the workspace launcher. Thus
Command-F does not search the files of a remote project. Prometeu does not make
a workspace, a checkout or a file for the search. The command and its arguments
did not change.

`read_bytes` returns an `ArrayBuffer` with the full regular file, with a maximum
of 100 MiB. It does not return a buffer that is not complete.

`reveal_path` must have `rel`. An empty string opens the root. A path that is not
empty shows one entry of the tree. From the Changes panel, it shows a changed
file. The frontend adds the repository folder under the workspace root before
the path relative to the repository. Because `rel` is mandatory, the two uses
have one IPC shape.

Finder selects a file with `-R`. Systems that use `xdg-open` do not have a
selection flag. Thus, on these systems, the command opens the folder that holds
the file. It does not open the file, because that opens a different
application. The resolution makes the root canonical. Thus a root that is a
symlink opens its target, and a missing root has a failure before the file manager
starts.

### File tree actions

The side tree operates like a file manager through three commands. All paths
are relative to that root:

| Command | Arguments | Effect |
| --- | --- | --- |
| `create_path` | `rel`, `dir` | makes an empty file or a folder. It does not replace a current entry. |
| `rename_path` | `from`, `to` | renames or moves an entry. It refuses a target that is there, and it refuses to move a folder into itself. |
| `trash_path` | `rel` | moves the entry to the system trash. It does not delete the entry. |

The last component of a path must be one plain name. Empty, `.`, `..`, `/`, `\`
and NUL answer `err.files.name`. A target that is there answers
`err.files.exists` with `name`. Git metadata is not permitted at all depths. A
`.git` component at a location in the path (with capital or small letters), or
a parent that resolves into one through a symlink, answers `err.files.name`.
This applies to the new entry, to the two ends of a rename and to the entry for
the trash. Only the parent folder resolves through symlinks, and it must stay
in the root. The app does not follow the entry itself. Thus a rename or a move
to the trash of a symlink changes the link.

A rename to a name that answers as present is permitted only in one condition.
The change is only between capital and small letters of the same name in the
same folder. On a disk that
ignores capital and small letters, which is the macOS default, the new spelling
finds the source itself. The rename is permitted when the folder lists no entry
with the same spelling. Thus the app continues to refuse a second, different
entry. This includes two symlinks to one file.

Such a rename goes through a random temporary name in the same folder. Some
file systems ignore a rename that changes only capital and small letters. If
the second step has a failure, the app cancels the first step. These renames operate
one at a time. Thus two renames at the same time cannot use the same temporary
name and replace a file.

The same test examines the two types of disk. On Linux CI, which makes a
difference between capital and small letters, it examines the list check and
the refusal of a second entry. On macOS CI, which does not make a difference,
it examines the rename in the two directions. The app uses the trash because
Git cannot put back untracked work. Thus that work stays recoverable. A failure
answers `err.files.trash` with the `cause` of the system. `trash_path` is async
because the system trash can be slow, for example through Finder on macOS.

The UI moves the open file tabs and the unsaved drafts of a renamed entry. It
discards those of an entry in the trash. It does this in the workspace or
project where the action started. The commands are new and additive. The
frontend and the backend ship together, no stored field changes, and relay
clients do not see the commands. `src/mock.ts` edits its sample tree with the
same refusals for names and conflicts.

The previous `reveal` command is retired under the bundled IPC policy in
[ADR 0043](../decisions/0043-retire-unused-ipc.md). The frontend and the backend
ship together. Thus no IPC shim between versions applies. The stored state and
the relay clients do not change.

`open_dock` accepts a project id only for a terminal. The shell must have only the
folder, and without a workspace there is no script variable to send. Setup and
Run must continue to have a workspace and answer `err.session.noWorkspace`.
`workspace_scripts` and `dock_state` accepted an id without a workspace before
this change. They return an empty catalog and no port.

`remove_project` stops the shells of the project. This is the same as when an
archive, a cleanup or a removal of a workspace stops its docks. Thus when a
person adds the same folder again, it starts with no shells. `open_dock`
resolves the folder of a shell. It adds the new shell under the PTY lock that
the removal uses to stop docks. Thus a shell that starts during the removal also
stops, and a shell that a person requests after the removal finds no folder.

Dock keys are `<owner>:<kind>`. A project owner is a folder path that can
contain `:`. `prometeu_core::terminal::dock_kind` gives an owner only the keys
whose remaining kind has no `:`. A shell that exits while its owner is not on
the screen can show in `dock_state` with `alive: false`, but it gets no tab.
Setup and Run keep their output after they exit. Tests:
`dock_keys_belong_only_to_their_exact_owner` in
`crates/core/src/terminal/tests.rs`,
`removing_a_project_stops_its_shells_and_keeps_other_docks` and
`removing_a_project_while_its_shell_starts_stops_that_shell` in
`src-tauri/src/session.rs`, and `src/dockbar.test.ts`.

## Terminal sessions

Terminal sessions are local shells that the app owns
([ADR 0096](../decisions/0096-terminal-sessions.md)). The board lists them in
`Board.terminals`, and the `board` event sends each change.

| Command | Arguments | Effect |
| --- | --- | --- |
| `terminal_create` | `folder`: an absolute path or `null` | adds a session in that folder, or in the home folder for `null`, and returns it. It does not start a shell. |
| `terminal_rename` | `id`, `name`: `null` or blank clears it | sets the name that shows in place of the name of the folder |
| `terminal_close` | `id` | stops the shell of the session and removes the session. An id that is not a session changes nothing. |

`terminal_create` answers `err.dock.noFolder` with `path` for all values that
are not an absolute folder that is there. It does not register a project. The
shell is a usual dock. `open_dock` with the session id and `kind: "terminal"`
starts it in the folder of the session, or attaches again to the shell that
operates. `pty_*`, `close_dock` and `dock_state` use its `<id>:terminal` key. A
session is not a file root. The file commands continue to accept only
workspaces and projects. For all owners, `open_dock` answers
`err.dock.noFolder` when the folder of a shell is not there. It does not let the
native PTY start in the home folder.

The frontend stops a session when `pty-closed` comes for its key. Thus `exit`
closes it as × does. The desktop does not send `pty-closed` for a shell that it
stopped, and a quit sends no event. Thus sessions stay after a quit. The browser
mock keeps the same rules in memory. Tests: `src-tauri/src/terminal_sessions.rs`,
`src/terminals.test.ts` and `fixtures/backend-contract.json`.

## Remote machines

`remote.rs` keeps the remote machines
([ADR 0101](../decisions/0101-remote-machines.md)). The `machines` event sends
the full list after each connection change.

| Command | Arguments | Effect |
| --- | --- | --- |
| `machines` | none | `[{ id, name, ssh, limits, phase, detail, host, update_pending, claude?, agents? }]`. `agents` is `{ enabled, socket, serving, error }`, and the app reads it after each attachment. `phase` is `offline`, `connecting`, `connected`, `not_installed`, `replaced` or `failed`. `host` is the ready frame of the host. |
| `machine_save` | `machine: { id?, name, ssh, limits }` | adds or edits a machine and connects again. It rejects an `ssh` that is not one argument (`err.remote.destination`) and limits that do not agree with the systemd syntax (`err.remote.limits`). |
| `machine_remove` | `id` | deletes the machine and its projects. It refuses with `err.remote.inUse` while a workspace operates there. |
| `machine_install` | `id` | installs or replaces the host build. Then it waits a maximum of 20 s for the connection. |
| `machine_connect` | `id` | tries the connection again immediately |
| `machine_stats` | `id` | the `stats` of the host for the workspaces of that machine |
| `machine_claude_status` | `id` | `{ installed, loggedIn, email, method }` from `claude auth status` on the machine. The app also refreshes it after each attachment and sends it as `claude` in `machines`. |
| `machine_claude_login` | `id` | runs `claude auth login --claudeai` in a host terminal. It returns the authorization link (`claude.ai`/`claude.com`, `/oauth/authorize`) when the terminal shows it fully, in a maximum of 30 s. |
| `machine_claude_code` | `id`, `code` | writes the code to that login, waits a maximum of 90 s for it to exit and returns the new status. It returns `err.remote.loginExpired` if there is no login that is not complete. |
| `machine_claude_login_cancel` | `id` | stops the login that is not complete |
| `machine_agents_enable` | `id`, `rotate` | runs `agents setup --json` (with `--rotate` on request) on the machine. It waits a maximum of 8 s for the socket to answer or fail. Then it returns `{ config, preview, moved_from }`. `config` is a local stdio MCP configuration with the token, for a copy. `preview` hides the token for the screen. Settings does not store these values. The setup does not change the permissions of the home folder or configure the deployment infrastructure (ADR 0103). |
| `machine_agents_disable` | `id` | runs `agents disable` on the machine. The host closes the agent socket. |
| `remote_add_project` | `machine`, `path` (absolute), `url?` | if `url` is given, clones it into `path`. It registers the root of the repository as a project with `host` and returns it. |
| `remote_list_dir` | `machine`, `path?` | `{ path, home, entries: [{ name, git }] }`: the visible subfolders of the folder (the home folder without `path`). `git` is set on repositories. The launcher uses it to show the folders, and it registers the selected repositories one by one. |

`create_workspace` with `draft.host` makes the workspace on that machine and
sets the terminal mode. Its `draft.project` can be a remote project ID or a
local project ID. For a local project, the workspace uses the clone on the
machine with the same origin. If there is no clone, the preparation first clones
the `origin` of the local project into `~/code/<folder>`. Without an origin,
the result is `err.remote.noOrigin`. If that folder contains a different item,
the result is `err.remote.cloneTaken`.

`list_branches` accepts an optional `host`. It runs Git on the machine for a
remote project ID, or for the clone of a local project on the machine. The app
rejects agents without the `remoteMachines` capability, more repositories
and delegated creation. Remote workspaces do not have file, Git, diff, dock or
script commands at this time. The interface hides those panels. The browser mock
implements each command in memory.

## Ending the app

`running_terminals` has no arguments. It returns the number of terminals that a
quit or a restart stops:

- Setup and Run while their script operates.
- A shell or terminal session while it runs a command, as a foreground job or
  in place of the shell through `exec`.

A shell that waits at its prompt does not count. The interface adds the
conversations of the board that work or wait, and the workspaces in
preparation. Then it asks the person
([ADR 0097](../decisions/0097-ask-before-ending-the-app.md)). The browser mock
counts live Setup and Run docks. Tests: `src-tauri/src/dock.rs`,
`crates/process/src/terminal/tests.rs` and `src/quit.test.ts`.

## Tool selection

The global, project and workspace layers of the MCP, plugin and skill selection
([ADR 0045](../decisions/0045-layered-tool-selection.md)) use one shape:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

- `set_tools_global`: receives `{ mcp?, plugins?, skills? }`. Each is an
  optional `Selection`. An axis that is not there does not change. `null` sets it
  to inherit again. The global layer is a board field. Thus the result goes to
  the frontend through the current `board` event, and the command returns
  no value on success.
- `set_workspace_mcp`, `set_workspace_plugins` and `set_workspace_skills`:
  receive `{ id }` and the axis value as a `Selection`. This replaces the
  previous `string[] | null`. A value that is not there keeps the current value.
  `null` inherits. The native handlers examine the JSON request body, thus Tauri
  cannot change null into an argument that is not there. The wire shape does not
  change (ADR 0047).
- The four setters validate the payload before they write. They answer an error
  with an i18n code and do not store incorrect data. A malformed `Selection`
  causes `err.tools.badPayload`. An id on the incorrect axis causes
  `err.tools.badAxis`. Examples are a `skill-<id>` package on `plugins`, or a
  plain plugin on `skills`.
- `workspace_tools`: receives `{ id, agent? }` and returns the effective set for
  each axis. Each item has its source: inherited, added, removed, or `cli`. The last
  value is for the MCP servers that the Claude configuration of the person loads
  (ADR 0046). The result also has the items that the project declares whose
  trust is pending or rejected. An unknown workspace answers
  `err.session.noWorkspace`. The command lets the picker show the result
  without a read of the three layers. `agent` selects the provider of the tab on
  the screen. If it is not there, the command uses the provider of the
  workspace. Thus the current callers do not change.
- `mcp_inherited`: receives the `{ id, agent? }` of a workspace. It returns the
  MCP servers that the CLI configuration for its working folder gives and that
  the hub does not have. For other providers, it returns empty. The optional
  provider has the same default as `workspace_tools`. The frontend cache
  includes the workspace and the provider. The workspace picker shows this inherited base (ADR 0046).
- `project_tools`: receives the `{ id }` of a project or a workspace. It returns
  the `[tools]` that the primary repository declares and the settings file that
  declares it. It also returns the SHA-256 of that section and the stored
  decision, if there is one. `pending` is true only while there is no decision (approval or rejection)
  for the current hash. Thus an explicit rejection stops the prompt until the
  declaration changes.
- `project_tools_trust`: receives `{ id, hash, approved }`. `hash` is the
  version that the dialog shows. The backend gets the repository identity and
  the current hash from `id`. It compares the hash from the dialog and records a
  decision only if the two agree. A changed or removed declaration returns
  `err.tools.changed` and does not write. If the hash argument is missing, the
  command has a failure and does not write. The dialog must open again before a decision
  on a new version. There is one decision for each repository. A new decision
  with an agreeing hash replaces the previous one. See
  [ADR 0047](../decisions/0047-tool-selection-boundaries.md).

The trust entry uses `project_tools.pending`. It does not use the pending items. Project and workspace menus also open declarations independently
of the hub. This includes declarations with only removals or an empty
replacement.

There is no new event. The global layer and the trust decisions are board
fields, and the workspace layer was a board field before. `Selection` is a typed
shape in `src/types.ts`, and the mock implements each command above. ADR 0045
adds them in its interface phase. The parity test in `src-tauri/tests/mock.rs`
makes sure that each one is there.

## Workspace branch selection

`list_branches` returns `local` with its current `all`, `default` and `git`
fields. Older clients can ignore `local`. `create_workspace` accepts the optional
`draft.newBranch` and `draft.source`. If `newBranch` is not there, the branch
creation behavior stays the same for older callers. With a worktree and
`newBranch: false`, `source` must identify a current local branch or remote ref.
The backend refuses missing refs before it publishes a workspace card.

If the reference discovery finds a Git repository with no branches, the
launcher starts in the project folder with `worktree: false` and
`newBranch: false`. The branch, worktree and other repository controls stay
disabled while that project is selected. Then the person can select a project with
branches in the same launcher. The launcher then sets again the worktree and
new branch selections from before the limit. This includes selections that the
person edited during that launch.

Explicit worktree or branch selections, or the addition of a repository while
the references load, replace the saved selections. Thus a late response cannot
disable worktrees that are necessary for the selected repositories. Prometeu does not make
an initial commit or change the saved worktree preference. When the person opens
the launcher again after the first commit, the launcher also sets those
options again.

## Starting from a skill

- `plugin_skills`: no arguments. It returns `{ plugin, name, description }[]`
  for the skills that installed local-folder plugins contain. It does not
  include standalone skill packages. See the
  [plugin hub](plugin-marketplace.md#skills-inside-packages).
- `create_workspace`: `draft.kickoff` is an optional `<package>/<skill>` string.
  If it is not there or is empty, the workspace starts without a skill. Thus
  older callers and the delegation payload continue to operate. An unknown skill
  rejects with `err.kickoff.missing`. A provider without
  `workspacePluginSelection` rejects with `err.kickoff.unsupported`. The two
  rejections occur before the app publishes the card.

The browser mock makes the same catalog from its fixture plugins and uses the
same validation. The
[runtime contract](agent-runtime.md#skill-kickoff) and
[ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md) give the
meaning.

## Legacy import

`legacy_import_plan` does not change the state. It returns these items:

- The source.
- One of the situations `ready | missing | imported | targetNotEmpty | invalid`.
- The preview counts.
- When applicable, the error, the timestamp and the backup path.

`legacy_import_run` does not receive paths from the presentation. The backend
resolves the source and the destination. It does all validations again. It
refuses an open Prometheus and a destination that is not empty. It runs the
import and returns the same DTO in the `imported` state. The app continues to
publish the board change through the `board` event.

## Change checklist

To make or change a command, do these steps:

1. Change the Rust function and its error type.
2. Register the handler in `main.rs`.
3. Update the command name, the argument shape and the result in `src/ipc.ts`.
4. Implement the command in `src/mock.ts`, or refuse it there intentionally.
5. Update each TypeScript consumer.
6. Add a test of the argument shape and the return shape.
7. If the command changes stored state, write the compatibility.

To make or change an event, do these steps:

1. Define the payload and its owner.
2. Test the serialization in the emitter.
3. If the payload comes from a boundary that is not trusted, validate it in the
   consumer.
4. Make sure that the listeners stop when the screen closes.
5. Update the table above.

## Direction of evolution

The implemented map does not have a new dependency:

```ts
type Commands = {
  load_board: { args: undefined; result: Board };
  chat_send: { args: { session: string; text: string }; result: void };
};
```

A future step can make bindings from the Rust DTOs. An ADR must select the tool
after a small proof. The proof must examine these items:

- Enums with `serde(rename_all)`.
- `Option` and default fields.
- Serialized errors.
- Events, and also commands.
- The integration with Tauri 2 and the Rust version of the project.

The goal is not to make types for full internal structs. Only boundary DTOs
must be in the binding.

## Feedback

`feedback_capture` is additive and has no arguments. It returns a base64 PNG or
`null`. See [capture and limits](feedback.md). The mock returns a fictional
image.

`feedback_send` receives `{ report }` and returns no value. The backend sends
the report to the Cloud with the account credential. The credential does not go
into the webview. The backend returns an error with an i18n code:

- `feedback.needAccount` if there is no account, or on 401.
- `feedback.rateLimit` at the limit.
- `feedback.uncertain` with `{id}` if the delivery is not sure.
- `feedback.sendError` for all other errors.

The mock records the report, and no data goes out of the machine.

## Board recovery

`board_recovery` has no arguments. It returns `null`, or `{source, folder, error}`
when the last load did not come from a `board.json` that the app can read:

- `source` is `backup` or `reset`.
- `folder` is the absolute path of the private recovery folder, after it
  contains copies of the files that the app cannot read.
- `error` is the cause while the copy has a failure and saves stay blocked.

`board_recovery_open` opens that folder, which does not change, in the file manager of the
system. If it cannot, it returns an `err.session.openFailed` error. The two
commands are only local. The browser mock returns the value in
`localStorage["mock:boardRecovery"]`. See
[unreadable state](persistence.md#unreadable-state).

## Built-in MCP catalog entry

`mcp_hub`, `mcp_save` and `mcp_remove` return the virtual `prometeu` entry with
`config: {type: "stdio", builtin: true}` and the registry items that a person
can change. A person cannot write over the built-in entry or remove it
(`err.mcp.builtin`). It does not contain an executable path or a credential.
`load_board` includes the additive `delegations`. In Rust, its default is an
empty list. There are no new IPC commands. The browser mock makes the same
catalog and selection behavior, and it does not run MCP agents. See
[embedded MCP](embedded-mcp.md).

`workspace-preview` is additive and only local. The emitter examines the
delegation ownership and the availability of the workspace. The listener, which
operates for the life of the application, loads the local board again. It
examines the exact workspace and conversation again before it opens the current
preview. The browser mock can send the same event for E2E tests. The event does
not contain a URL or a different action, and it does not go through the relay.

## Local telemetry

`telemetry_summary`, `telemetry_insights`, `telemetry_turns`, `telemetry_events`,
`telemetry_export` and `telemetry_clear` are only local commands. They query,
export and erase data. `src/telemetry.ts` gives the types of the filters, the
cursor, the coverage and the return values. The Rust handlers and the browser
mock share the command names. No SQL from the caller and no native provider
objects go through this boundary. The export destination comes from the native
save dialog and is a `.jsonl` file. Erasure always clears the full telemetry
dataset. See the [telemetry contract](telemetry.md) for the query cohorts and
the privacy guarantees.

`telemetry_insights({ filter })` returns the current summary and also the
consumption and the groups by conversation, observed model and source. It gives
the PR attribution with its own qualification. The app does not add the context
occupancy and window values. The peak occupancy is a maximum.

`telemetry_turns({ conversation, messageIds })` looks for a maximum of 500
requested assistant IDs. It returns only exact completed matches, with the
duration and the canonical usage. Unknown and legacy replies do not get an incorrect
measurement. The native query calculates a hash of the conversation and reply
pair. The app does not store raw provider identifiers as telemetry content.
Frontend consumers ignore previous results after navigation or after a confirmed
deletion of history.

`telemetry-changed` is a local invalidation event without content. The
completion of a conversation sends it also when child activity keeps the tab
running. The naming function and the plugin maker send it after the capture is
complete. This includes calls with a failure. The PR discovery sends it one time
after it commits the full observed snapshot and releases the capture mutex.
Workspace consumers join the events and query their current scope. It is not a
relay event, and it does not send records or identifiers. The turn capture
commits before its current board publication. The refresh occurs also without a
status change on the screen.

## Delegated conversation admission

`new_tab` and the agent `action_start` reject delegated workspaces with
`err.session.delegated` before they make a conversation. The signatures and the
return formats do not change. The current delegated conversations and the
legacy extra tabs stay available. See
[sub-workspaces](embedded-mcp.md#sub-workspace-navigation). The browser mock and
the native entry points apply the same limit.
