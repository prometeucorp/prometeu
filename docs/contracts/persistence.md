---
type: Contract
title: "Local persistence"
description: "The local formats of the board, the transcripts and the accounts, with their migrations and recovery."
status: stable
---

# Local persistence

This contract is current.

## Principles

- The logical session continues to be available after the agent process stops.
- State and credentials stay out of the worktrees.
- When you delete a worktree, the conversation history stays.
- Sensitive files are private from the start: `0700` directories and `0600`
  files on Unix platforms.
- To write state again, Prometeu writes a temporary file, writes it fully to the
  disk and then gives it the correct name. Thus, after a failure, you cannot
  see a truncated file at any time.
- Previous formats get default values and migrations. Prometeu does not discard an
  previous format because new fields are missing.

## App root

In a release build, the default root is `~/.prometeu`. In a debug build, the
root is `~/.prometeu-dev`. `PROMETEU_ROOT` can replace the root. Tests and
isolated copies of the app use it most.

| Data | Path | Ownership |
| --- | --- | --- |
| board | `<root>/board.json` | `board_store.rs` through the injected `BoardStore`. The models are in `crates/core/src/board.rs`. |
| external MCP clients | `<root>/mcp-clients/<uuid>.json` | `mcp_access.rs`. Private versioned credentials, the project scope and the optional `todos` grant. Refer to [embedded MCP](embedded-mcp.md). |
| standalone to-do list | `<root>/todos.json` | `todos.rs` through `prometeu-core::todos::TodoStore`. Private `{ version: 1, items }`. Refer to the [to-do contract](todos.md). |
| location of the to-do list | `<root>/todos-location.json` | `todos.rs`. Private `{ machine }`. A missing file means this computer. |
| MCP socket discovery | `<root>/mcp-socket` | `embedded_mcp.rs`. A private temporary socket path. The app replaces it at startup. |
| board backup | next to `board.json` | `board_store.rs` |
| copies of unreadable state | `<root>/recovery/<label>-<millis>.json` | `recovery.rs`. Refer to [Unreadable state](#unreadable-state). |
| team and credential | `<root>/team.json` | `team.rs` |
| E2EE identities, TOFU links and replay | `<root>/team-security.json` | `team.rs` (file), `team-security.ts` (internal schema) |
| optional Prometeu account | `<root>/cloud.json` | `cloud.rs`. Refer to the [contract](cloud-account.md). |
| cloud catalog cache and links | `<root>/catalog.json` (`catalog.local.json` is a legacy backup) | `catalog.rs`. Refer to the [contract](cloud-catalog.md). |
| installed skills and packages | `<root>/skills.json`, `<root>/skills-packages/<id>/` | `skills.rs`. Refer to the [catalog](cloud-catalog.md). |
| accounts and the selection for each provider | `<root>/accounts.json` | `crates/core/src/accounts.rs` through the injected `AccountStore`. `account_store.rs` keeps the private file. |
| additional authenticated profiles | `<root>/accounts/<uuid>/` | provider adapters |
| last quota snapshot for each account | `<root>/usage.json` | `usage.rs` |
| Linear OAuth credential | `<root>/linear.json` | `linear.rs`. Private access and refresh tokens, the account identity and the granted scopes. Refer to [Linear issues](linear-issues.md). |
| Linear issue cache | `<root>/linear-issues.json` | `linear.rs`. Private lists of assigned and available issues, with the fetch time. Refer to [Linear issues](linear-issues.md). |
| GitHub App credential | `<root>/github.json` | `github_auth.rs`. Private access and refresh tokens, the expiry time and the login. IPC does not send them at any time. Refer to [GitHub issues](github-issues.md). The app ignores the retired `<root>/github-issues.json`. |
| optional TypeSafe key and its enabled state | `<root>/typesafe.json` | `typesafe.rs`. Private and disabled by default. Refer to [context evaluation](context-evaluation.md). |
| consent for product analytics | `<root>/analytics-enabled.json` | `analytics.rs`. A private JSON boolean. A missing or incorrect file disables capture and causes the question at startup. A saved boolean of either value stops the question. Refer to [product analytics](product-analytics.md). |
| identity for product analytics | `<root>/analytics-id` | `analytics.rs`. A private random UUID. The app makes it only when the consent and the build key let capture occur. The file alone does not show consent. |
| V1 transcript for Codex, Antigravity, Cursor and all providers in terminal mode | `<root>/chats/<tab>.jsonl` | `transcript_store.rs`, through the core conversation stream |
| binding of the native terminal transcript | `<root>/terminal-bindings/<tab>.json` | `chat/terminal/binding.rs`. A private versioned identity and the accurate path, without conversation content. |
| ingestion journal of the native terminal | `<root>/terminal-ingestion/<tab>.sqlite3` | `terminal_agent_journal.rs` and `chat/terminal/ingestion.rs`. The private source provenance and the durable canonical append. |
| diagnostic log of the app | `<root>/logs/prometeu.log` (previous: `prometeu.log.1`) | `main.rs`. A private `0600` stderr when the app gets a discarded stderr (`/dev/null` or closed, as in a Finder launch). At startup, the app rotates the log if it is larger than 5 MiB. Terminal-host lines are `<ms> <tab or generation> terminal.<code>`, with only identifiers and step names. |
| files that the app receives through a native promise | `<root>/attachments/<uuid>/<name>` | `file_drop.rs`. A private `0700` directory and a `0600` file. |
| image pasted from the clipboard | `<root>/attachments/<uuid>/pasted.png` | `file_drop.rs`. The same folder and permissions. On macOS, the app changes TIFF to PNG. On Linux, it changes a GTK image to PNG. |
| plugin hub | `<root>/plugins.json` | `plugins.rs` |
| remote machines | `<root>/machines.json` | `remote.rs`. Private `{ v: 1, machines: [{ id, name, ssh, limits: { memory_max, cpu_quota } }] }`, without credentials. Refer to [ADR 0101](../decisions/0101-remote-machines.md). |
| identity of the remote executor | `<remote root>/supervision-executor` | `prometeu-host`. A private and stable executor ID that is available before the supervision setup. An ID in a current `supervision.json` has priority. Refer to [supervision](supervision.md#provisioning-with-the-remote-mcp). |
| mirror of remote transcripts | `<root>/remote/<machine>/mirror/<remote path>` | `remote/connection.rs`. Private append-only copies of the Claude project files of a machine. If a copy is missing, the app makes it again from offset zero. Refer to the [remote host protocol](remote-host.md#transcript-mirror). |
| remote workspaces, tabs and projects removed from the board | `<root>/remote/<machine>/dismissed.json` | `remote/registry.rs`. A private JSON array of workspace, conversation and project IDs. Adoption from the registry of the machine ignores these IDs. When you add the project again, the app removes its entry. Refer to [ADR 0103](../decisions/0103-agents-on-remote-machines.md). |
| pending remote archive actions | `<root>/remote/<machine>/archives.json` | `remote/registry.rs`. A private map from workspace IDs to archive booleans and operation IDs. Explicit actions stay until the host confirms them. Refer to [shared workspace archives](remote-host.md#shared-workspace-archives). |
| derived Codex home and marketplace | `<root>/codex-workspaces/<workspace-hash>/[<account>/]` | `plugins.rs`. The app can make it again. |

When a person installs a project from the catalog, the app uses the board
Project records again. It stores the local paths in the installation links of
`catalog.json`. No credentials or paths go into the Cloud document. Refer to
[Git projects](cloud-catalog.md#git-projects).

Worktrees are in `~/prometeu/worktrees[-dev]/...`, out of the state root.

The app loads repository scripts, tool declarations and method settings only
from `.prometeu/settings.toml`. A worktree without this file uses the file of
the original checkout. A worktree with its own file replaces the full file.
Workspace scripts receive these values:

- `PROMETEU_WORKSPACE_PATH`, `PROMETEU_ROOT_PATH` and `PROMETEU_WORKSPACE_NAME`.
- `PROMETEU_PORT` when the app reserves a port, and the same port as `PORT`.

The app does not use a different configuration file or environment prefix as a
fallback. Refer to [project scripts](../operations/project-scripts.md).

When a Linux AppImage starts, it keeps a desktop entry for the running file
out of the state root. This entry is in `$XDG_DATA_HOME` (`~/.local/share` by
default): `applications/co.prometeu.desktop.desktop` and its icon
`icons/hicolor/256x256/apps/co.prometeu.desktop.png`. They contain no app state.
`desktop_entry.rs` replaces each file through a rename, but only when its
content is different. It removes the entry when AppImageLauncher or Gear Lever
lists the same file. Refer to [ADR 0055](../decisions/0055-linux-desktop.md).

To find a transcript, the app first uses the saved conversation mode of the tab.
Terminal mode always reads the V1 log of the app. Chat mode uses the provider of
the saved choice of the tab, or the provider of its frozen task profile. A
retired provider keeps the history path of that provider. Chat mode uses the
workspace provider only when the tab has no saved choice and no task profile.
The same resolver operates for stopped snapshots and MCP history. Current chat
tabs keep their transcript paths without a format migration.

The native terminal transport keeps a native binding only after the
authenticated root hook of its process passes observation validation. Version 1
contains `version`, `session`, `provider`, `nativeSession` and the absolute
`transcript` path. It does not contain a bridge token or message content. A
read has a limit of 64 KiB. The app and native identities and the provider must
agree accurately. Unknown versions, unknown fields and differences cause a
failure. Writes use the private atomic file helper.

On resume, the permitted roots of the captured account authorize the path again.
This includes native history that accounts share on purpose. A previous account
ID does not give more access to the file system.

Resume loads this binding and completes a silent native baseline before the app
starts the CLI. Thus, the app does not wait for a new prompt to make the hook
that it must have to read the history. As an alternative, Cursor can authorize
its accurate native session database again when the CLI did not report the
derived JSONL path. If the source binding is missing or incorrect, the app
rejects the terminal resume with a clear error. It does not select a different
session or examine unrelated histories.

Current chat tabs do not read this additive format, and a migration is not
necessary for them. The tests in `chat/terminal/binding.rs` and the resume tests of the
observer cover compatibility. The format alone does not repair output that the
app did not get during an app crash. The ingestion journal below gives the
recovery path for Cursor. Each different provider must first have its
source-provenance integration. Then, the app can enable terminal mode for it.

[Remote process adoption](remote-host.md#transcript-mirror) can keep a process
when its authenticated binding comes before its first transcript file. It keeps
the journal and the identity. Before it uses the first prompt as new input, it
must have proof that the app captured no conversation work. It does not make
the history requirements less strict when the app starts a replacement process.

AppKit writes promised files (for example, the thumbnail of a capture) in a new
directory for each gesture. It does not replace previous attachments. The
backend gives to the UI only the files that are in that directory. The
transcript can refer to their paths. Thus, the app does not remove them when it
sends the message, when the session ends or when you delete the worktree. At this
time, there is no automatic cleanup. Usual Finder files continue to use their
original paths. That folder is additive. A rollback ignores the folder and keeps
the paths that the app sent before. No current format must have a migration.

Prometeu does not look for the Prometheus roots and does not write in them.
Worktrees that the earlier migration from Prometheus adopted stay in `~/prometheus/worktrees`. The
cleanup of a multi-repo workspace continues to accept that path. The two
applications must not operate on the same folder at the same time.

## Notification preferences

The app does not read or write the legacy `prometeu:som` localStorage key. If
the key is there, it has no effect. There is no migration and no change to the
board or the transcripts. The new `prometeu:notifications` version-1 record
stores the local notification selections. By default, the app disables notifications and
sound. A missing record, an incorrect record and a record from a future
version keep those default values. Refer to the
[notification contract](notifications.md) and
[ADR 0054](../decisions/0054-local-notifications.md).

## Collaboration security

`team-security.json` is additive and private (`0600` in a `0700` directory).
Its writes are atomic, and its limit is 8 MiB. The envelope is
`{ version: 1, scopes: { ... } }`. Each scope is the combination of these items:

- The origin.
- The organization or team.
- The local Cloud account.
- The membership.

A scope contains these items:

- A P-256 identity (a private JWK and a public key).
- Member links.
- The announcement sequence.
- The last owner, key, revision and ID for each share.
- Receipts for remote messages.
- The clock of the last consumption.

The limits for each scope are 64 links, 4096 shares and 4096 receipts that did
not expire. Receipts expire in two minutes or less.

Each scope also contains two fields:

- `approved`: the member → key pairs. The owner approved the remote input from these keys.
- `paused`: the member → workspace IDs where the app discarded input from a key
  that is not approved. The input stays discarded until the owner answers the
  notice.

`paused` contains only identifiers. It does not contain message content at any
time. The limits are 64 approvals and 64 paused members, with a maximum of 64
workspaces for each member.

A scope from before the approvals feature has no `approved` field. When the app
loads that scope, its links become approved. Thus, teams continue to operate
after an upgrade. A new scope starts with no approvals. A rollback to a version
without these fields removes them at its next write. A new upgrade then approves
the links that are there at that time. Refer to
[ADR 0090](../decisions/0090-approved-remote-input.md).

The app writes these items before it uses them:

- A new identity.
- A member link, first or replaced.
- An approval.
- A revision.
- A receipt.

Corruption, an unknown version and a read or write failure stop collaboration.
The app does not silently make new keys. The file stays when a person leaves
the team, changes the organization, logs out or renews a ticket. The webview
does its operations one at a time. The Rust write uses the same lock during the
read, the validation and the write.

The file contains secrets. It is not an encrypted backup, and it is not a
content key on the server. If the file is lost, the TOFU continuity
is lost, and also the access to the encrypted comments of the previous identity. A new device
joins with a new key, and peers adopt it automatically
([ADR 0042](../decisions/0042-automatic-key-rotation.md)). A rollback ignores
the file and keeps it. It does not change it into v3 credentials at any time.
The browser mock stores only fictional identities in localStorage, and it uses
the same cryptographic channel.

Tests: `src/team-security.test.ts` and the `src-tauri/src/team.rs` tests.
Network contract and limits: [relay v4](relay-v4.md).

## Optional context evaluation

`typesafe.json` is additive: `{ version: 1, enabled, key }`. Each field has a
default value. These conditions keep the integration disabled:

- The file is missing.
- The installation is from a previous version.
- The file does not have a key and `enabled: true` together.

When you save a key, the integration does not become enabled. When you remove
the key, the app deletes the file. An unreadable file stays disabled. Settings
shows the problem, and work continues. The key does not go into the board, the
transcripts, `team.json`, the relay or the Cloud catalogs at any time. No IPC
result returns it. Tests: `src-tauri/src/typesafe.rs`. Refer to the
[contract](context-evaluation.md).

## GitHub notification read state

`prometeu:github-notifications:read:<origin>#<user id>` in the localStorage of
the desktop webview stores `{ "before": "<id>", "ids": ["<id>"] }`. It applies
to one Prometeu account on this Mac only. The app does not keep notification
rows. They come back from the Cloud feed. Refer to
[GitHub notifications](github-notifications.md).

## Settings navigation preference

`prometeu:configuracoes` stays a local string in the localStorage of the desktop
webview. New selections write `geral`, `agentes`, `recursos`, `acoes` or
`trabalho`. When the app reads a historical page value, it selects the new
group for that value:

- `contas` and `padroes` open Agents.
- `notifications` and `app` open General.
- `ferramentas`, `plugins` and `skills` open Resources with the applicable
  filter.
- `projects`, `time` and `integracoes` open Work and team.
- Unknown values open General.

The read does not write the saved value or a related preference again. No
board, IPC or provider format changes. `src/settings-navigation.test.ts` contains the
compatibility tests.

## Skill kickoff

`Tab.kickoff` is an optional `<package>/<skill>` string. It records the *Start
with* skill that a conversation started from. When there is no skill, the app
does not write the field. Thus, usual tabs keep the previous shape, and previous
boards load with no kickoff. While the skill is installed, a resume adds that
package to the resolved tools of the tab. If the skill is not installed, the
resume continues without it and shows a warning in the conversation. No
selection layer stores the kickoff.

The repository tells where artifacts go with `[method] artifacts` in
`.prometeu/settings.toml`. `scripts.rs` reads it with the same clone inheritance
and primary-repository rule as `[tools]`. The app accepts only a relative path
in the repository. For a different path, it names no path.

The desktop webview keeps the last launcher selection of a person in its
localStorage as the string `prometeu:kickoff`. This selection can also be no
skill. If
the installed catalog does not have that value at this time, or the storage is not
available, the conversation starts without a skill. Tests:
`crates/core/src/board.rs::tabs_without_a_kickoff_keep_the_previous_format`,
`scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented` and
`src/kickoff.test.ts`. Refer to
[ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md).

## Automatic worktree cleanup

`prometeu:auto-cleanup-force` is a string in the localStorage of the desktop
webview. Only `true` enables automatic cleanup after a successful manual archive
or finish. A missing, unknown or inaccessible value keeps the scoped
confirmation. Thus, the behavior after an upgrade does not change. `false`
disables the automatic cleanup. The preference applies to all projects in that
webview and stays after a restart. The app does not store it on the board or
send it through the relay. No IPC or board format changes are necessary.

An enabled cleanup runs with `force`. Thus, the cleanup deletes work that is not
committed or not merged. The earlier `prometeu:auto-cleanup` key recorded
consent to a cleanup that asked before it deleted that work. For this reason,
the app does not read that key at this time, and those consents start as disabled. Refer
to the [Git contract](git.md#archived-worktree-cleanup) and
[ADR 0048](../decisions/0048-scoped-cleanup-offer.md).
Tests: `src/cleanup.test.ts`.

## Sleep preference

`prometeu:acordado` is a string in the localStorage of the desktop webview. It
has these values:

- `off`: the system can sleep as usual.
- `on`: the system and the display stay awake while the app is open.
- `agent`: the system and the display stay awake only while a local tab is
  `rodando`.
- `agent-system` (additive, macOS only): the system stays awake in that same
  state, but the display can sleep.

The three original values keep their behavior, and the app does not write them
again automatically at any time. Linux shows only the original values. The
preference is local to this Mac. It is not in the board or the relay. A missing
or unknown value becomes `off`. The native assertion is temporary and stops
when the app stops. The app does not keep a sleep assertion.

## Sidebar width

`rail-w` is an optional decimal pixel width in the localStorage of the desktop
webview. A missing, not finite, zero or negative value uses the default of the
design token (256px). When you drag the right edge of the sidebar, or use
Left/Right on its separator, the app saves this local preference. A double-click
or Home removes it and puts the default back.

During rendering, the app keeps the width between 200px and 50% of the window,
with a minimum of 200px. This rule also applies at startup and when the window size
changes. A change of the window size does not replace the preferred width.
The remaining workspace width also limits the right panel. The app keeps 320px
for the conversation, or half of the workspace when the workspace is narrower
than 640px.

This rendering limit applies to restored widths, to the two resize handles and
to changes in panel visibility. It does not write the saved `side-w` preference
again. It does not change the board, the relay or the current `side-w` key of
the right panel. Previous installations do not have to do a migration. Previous versions
ignore the new key. `e2e/sidebar-resize.spec.ts` contains the tests for compatibility
and for the browser interaction.

## Model selection preferences

These preferences are in the localStorage of the desktop webview, not in the
board or the relay:

- `prometeu:model-choice`: JSON `{ agent: ProviderId, model: string }`.
- `prometeu:model-favorites`: a JSON array of the same pairs, without two
  entries that have the same two fields. Favorites are local to this
  installation, and all desktop pickers use them.
- `prometeu:effort`: the native effort string. An empty string means the
  default of the provider.

The app does not change the previous `prometeu:model` string. When there is no new
choice, the app waits until the discovery of all installed agents is
successful. Then it does a migration only when the association of model and
provider is clear. Known historical Claude aliases can continue to identify previous
Claude preferences. For an unclear or unknown legacy value, the person must
select a model. A new installation uses the native default model of the first
installed provider. The app ignores incorrect preference JSON.

When a catalog removes a model, the app does not remove its saved default,
favorite, tab choice or profile choice. Favorites that are not available in the
current account stay stored, but you cannot select them.

The app reads the historical Codex `ultracode` effort as `ultra`. New selections
write native values. Historical efforts that the provider cannot use stay
visible. The app does not write them again only because you open or save an
unrelated profile field. An explicit model change keeps the effort only if the
new model can use it. If not, it selects the default of the provider and tells
the person about the change. `Choice` and `SessionLaunch` keep their current
wire shapes.

A saved model that is not available, or an effort that the model cannot use,
cannot start a new workspace until the person corrects it. The app does not
write its saved preference and the current conversations again. Catalogs and
their freshness state are only in memory, and an account change makes them not
valid. Tests: `src/model-choice.test.ts`, `src/agents.test.ts` and
`e2e/model-picker.spec.ts`, with the compatibility of action profiles in
`e2e/actions.spec.ts`.

## Board

`Board.conversation_mode` is the default for new conversations, `chat` or
`terminal`. Each `Tab.conversation_mode` stores the mode that the person
selected when the app made that conversation. The two fields have the default
`chat`, and the app does not write that value when it saves. Thus, a previous tab
stays in chat when the global default changes. Resume puts back the saved mode
of the tab, also for reusable actions. A change of the model or of the global
preference does not change a current transcript. `Launch.conversation_mode` is
optional. If it is missing, the launch uses the board preference, but only when
the app makes the conversation. An explicit `chat` replaces a terminal
default.

macOS shows the default in Settings / General and operates native terminal
conversations. Linux rejects terminal launches with a clear error. These tests
cover compatibility:

- The board revival and mode round-trip tests in `crates/core/src/board.rs`.
- The launch deserialization tests in `crates/core/src/session/launch/tests.rs`.
- The desktop tests for task and tab resume in `src-tauri/src/session.rs`.

Workspace stages are retired ([ADR 0102](../decisions/0102-remove-workspace-stages.md)).
The app ignores the legacy `stages`/`columns` and `stage`/`column` fields when
it loads a board, and it does not write them when it saves. Workspaces and
transcripts do not change. Versions that must have those fields cannot read a
board that the new version saved. To go back to such a version, use a backup
from before the upgrade.

`Board` contains projects, workspaces and the optional `actions` catalog. The
sequence of `projects` is the sidebar sequence that the person selected with a
drag. Previous boards keep their registration sequence. `Project.group` is the
optional sidebar folder. The app writes it only when it has a value. Thus, previous
boards load without a change, and previous versions ignore it. A folder is only
that name on its members. The sidebar shows folders first, in alphabetical
sequence, and then the projects without a folder. A drag changes the sequence
only in one folder or among the projects without a folder.

`Tab.task` stores the resolved configuration and the cursors of the tasks. When
those fields are missing, previous sessions continue to operate. The catalog
receives Code review one time only, and `actions.defaults_initialized` records
this. Refer to [actions](actions.md). `Workspace` contains the repositories, the
branch, the worktree, the agent configuration, the MCP/plugins/skills selection,
the sharing and the tabs.

`preserve_branches` lists clone paths. The current local and remote
branches of these clones must stay after worktree cleanup. The app examines each repository
independently. Thus, a new branch in a different repository continues to have
merge safety, and cleanup deletes it. Previous boards do not have the list and keep
their previous cleanup behavior.

`Tab.tokens` stores an estimate of the tokens that the conversation used. The
app adds to this estimate in increments. `Tab.context_tokens` stores the last
observed context, so that the app adds only the increase. When the context
becomes smaller after compaction, the new value starts a new segment and adds
to the total. Previous boards without `context_tokens` use `tokens` as the total and
as the initial cursor, and the app does not count the value two times. The tab
also contains the identity, the status, the pending message, the model override
and, when necessary, the external identity for resume.

The app sets `Tab.interrupted` during revival for a tab that was `rodando` or
`querendo` when the app stopped. The next accepted message removes it. The app
writes it only while it is true. When such a tab starts a process again, its
transcript ends at the cut. Thus, `Tab.cuts` records that line count. Replay
puts `session.state: stopped` at that position. When the conversation
continues, the reply at the cut does not look complete at any time.

Transcripts only become longer at the end. Thus, the position is correct for
native logs and for logs of the app. The app writes `cuts` only when it is not
empty. Thus, previous boards load and save without a change. An empty `Tab.title`
is a tab without a name, and the interface shows the model of the tab. When the
app loads previous boards with the invented name `conversa` or `conversa N`, it
changes that name to an empty name.

`Board.tools` keeps the global layer of the tool selection. `Board.tool_trust`
keeps the decisions that let the declaration of a repository become active.
[Tool selection layers](#tool-selection-layers) gives the two fields.

When you remove a project, the app removes only its registration from the
board. It does not delete the repository, the worktrees or the conversations.
The workspaces of that project show under **No project**. During a load, only
legacy workspaces without the `project` field make the registration again. An
explicit ID without an applicable project keeps the removal.

During a load, the app does these operations:

- For unknown enum values, it uses a safe state when `serde(other)` declares it.
- For added fields, it uses `serde(default)`.
- During a migration, aliases keep the previous names.
- It makes runtime states agree with reality. Processes do not stay after the
  app stops.

A change that removes a persisted field, renames it or changes its meaning must
have a test with JSON from the previous version.

### Unreadable state

Prometeu does not replace bytes that it cannot read at any time. When
`board.json` does not deserialize, the board comes from `board.json.bak`. When
the backup also does not deserialize, the board starts empty. Before the app
replaces an unreadable file, it copies the bytes of that file into the private
`<root>/recovery/` folder as `board-<millis>.json` or
`board-backup-<millis>.json`. The app does not write a copy again if an
identical copy is there. A new copy does not use the name of a current copy at
any time. The app keeps only the five most recent different copies of each file.

The app copies an unreadable `board.json` during the load. Thus, the original
stays until a save replaces it. While that copy fails, or when the app cannot
read the file at all, each save returns an error. The files on the disk do not
change.

The app copies an unreadable backup during the load when the board uses the
backup. If not, it copies the backup before the first save of the session
writes it again. While that copy fails, saves continue to write `board.json`,
but they do not change the backup.

At startup, the interface asks `board_recovery`. The board can come from the
backup or start empty. In these two conditions, the interface shows a dialog
with the recovery folder and the copy error, if there is one. Refer to [IPC](ipc.md#board-recovery).
`team.json` uses the same rule. A file that does not parse continues to read as
no team. But before the app joins or leaves a team, it copies the file to
`recovery/team-<millis>.json`. If the app cannot read the file, it stops the
join and the leave operations.

The board and `team.json` do not change their formats. The tests in
`board_store.rs`, `recovery.rs` and `team.rs` cover this behavior. Refer to
[ADR 0091](../decisions/0091-keep-unreadable-state.md).

## Ordered board publication

`BoardPublisher::publish` does these operations in sequence under one
publication mutex:

1. It makes the snapshot.
2. It puts the snapshot in the queue.
3. It sends the `board` event.

 It locks the current board for a short time to copy
it. Then it releases the board lock before it puts the snapshot in the queue and
sends the event. Thus, a publisher cannot put a previous snapshot in the queue, or
send it, after a more recent snapshot. The worker continues to combine writes and to
do disk I/O outside the board lock.

`save_now` uses the same publication mutex and waits for a flush
acknowledgment. A flush does not stop the worker. Action transitions also flush
while the app continues to operate. The app must also keep the changes that
come after the flush. `crates/core/src/publication/tests.rs` contains the
regression tests. They include concurrent publication, durable barriers,
storage failures and disconnected delivery.

Storage and event delivery use injected ports. Refer to the
[application-core contract](application-core.md). The assignment of terminal
ports uses the same persistence queue. Thus, previous snapshots cannot replace
that assignment.

No board fields or serialization change. Refer to
[ADR 0023](../decisions/0023-ordered-publication.md).

## Derived plugins

The hub is the source of truth of Prometeu. The copy and the marketplace in
`<root>/codex-workspaces/<workspace-hash>/marketplace/` are a cache. They
contain a hash of the source. The app can make them again, and they do not go
into the board. The same directory contains a derived `config.toml`. This file
uses the real configuration and keeps the trust and the active hook state of
that workspace.

The layer of the original account stays at that path. Each managed account gets
its own subdirectory. The other `CODEX_HOME` entries point to the profile that
the app captured when it started the process. Each profile keeps its own
credential, but sessions, skills and the plugin cache stay shared. The global
configuration does not receive the marketplace of Prometeu or its activation.

When you remove the workspace from the board or return its worktree, the app
deletes that derived layer. It does not follow the links to the shared state.
The full contract is in [`plugin-marketplace.md`](plugin-marketplace.md).

## Tool selection layers

MCP servers, plugins and skills are three independent axes. The app selects
each axis in three layers, in the sequence global → project → workspace
([ADR 0045](../decisions/0045-layered-tool-selection.md)). For each axis, a
layer is missing, and then it uses the layer above, or it is an object:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

A missing `base` means `"inherit"`. Thus, a manually written `[tools]` that
declares only `add` does not erase the layers above it at any time. In one
layer, the app applies `add` first, and `remove` has the last decision.

| Layer | Where it lives | Owner |
| --- | --- | --- |
| global | `Board.tools` in `<root>/board.json` | `crates/core/src/board.rs` |
| project | the `[tools]` table of `.prometeu/settings.toml`, in the repository | `scripts.rs` |
| workspace | `Workspace.mcp`, `Workspace.plugins` and `Workspace.skills` | `crates/core/src/board.rs` |

`Board.tools` is local to the app. It contains one `Selection` for each axis,
and all of them are missing by default. Thus, a previous board continues to inject
the same items as before. The project layer is the only layer that the app
writes outside the app root. The file is versioned. Without trust, a repository
author can select packages and hooks for all persons who clone it.

For a multi-repository workspace, the app reads only the `[tools]` of the
**primary** repository. This agrees with the single-root behavior of Claude Code
and Cursor. It also agrees with the current use of the primary repository in
the launcher.

`Board.tool_trust` stores one decision for each primary repository:
`{ repo, hash, approved, at }`.

- `repo` is the URL of the `origin` remote when there is one. If not, it is
  the absolute path of the clone.
- `hash` is the SHA-256 of the declared `[tools]` section. The backend
  calculates it again and compares it with the version that the person saw
  before the app records the decision (ADR 0047).

A submission for a version that is not current does not change this persisted
format or the current decisions.
`approved: false` is an explicit rejection. It stops the prompt, but it keeps
the items of the declaration out of each process start, with the label
"rejected". The hash of a declaration can be different from the stored
decision (approval or rejection). The app resolves such a declaration, but it
does not inject it. It asks the person again until the person decides. The app does not write the decision into
the repository at any time. Refer to
[`plugin-marketplace.md`](plugin-marketplace.md).

### Workspace migration

`Workspace.mcp` and `Workspace.plugins` were `Option<Vec<String>>`. During a
load, the app changes them as follows:

- `null` stays `null`.
- `[]` becomes `{base:"none", add:[]}`.
- `[ids]` becomes `{base:"none", add:ids}`.

Thus, a board after the migration resolves to the same set that it injected
before. The app moves entries with the form `skill-<id>` from `plugins` to the
new `skills` axis. The hub keeps them as packages, so only the state and the
interface change. In previous boards, `Workspace.skills` is missing, and thus it
uses the layer above.

The migration operates in one direction only. A previous version cannot
deserialize the object form. It then uses the board backup, and after that a
default board. Versions that keep [unreadable state](#unreadable-state) copy the
unreadable files before they save. The app captures the resolved set when it
starts the process. Thus, a change in a layer has an effect at the next process
start or resume ([`agent-runtime.md`](agent-runtime.md)).

## Quotas

`usage.json` stores, for each local account ID, the known windows and the
timestamp of the last change. The previous `claude` and `codex` keys continue to
identify the original profiles. The app does not write previous caches again.
The registry and the global selection are in [`accounts.md`](accounts.md). They
do not go into the board or the relay.

Each window has a kind, a percentage and a reset. `scope` and `label` are
optional. A previous snapshot without those fields continues to be one quota. New
snapshots use `scope` to keep general quotas apart from the quotas for a model
or a feature. The file is a cache. A correct read from the provider replaces
the persisted content.

## Transcripts

Terminal conversations use `<root>/chats/<tab>.jsonl` for all providers,
and also for Claude. Accepted input, observed canonical output and application
notices go at the end of that history. Native bootstrap snapshots and rewrite
snapshots do not replace published history or add to its start at any time.
Native recovery adds only source occurrences and versions that the app did not
capture, with durable identity receipts.

Each provider uses its own identities:

- Cursor uses its resumable store IDs.
- Codex uses its thread, turn and item IDs.
- Claude uses the native row UUIDs.
- Antigravity uses the native step indices.

When the app cannot use the source content, it does not give coverage. It does
not silently certify a complete baseline. Thus, the native terminal can show a
compacted provider state, while Prometeu keeps the prior conversation, the
positional comment anchors and the cut locations. Compatibility tests in
`chat.rs` cover the routing for each mode. They also make sure that canonical
notices, usage and transcript positions stay after a native rewrite.

### Native ingestion journal foundation

`terminal_agent_journal.rs` gives a private SQLite schema 4 for durable
provenance of native sources at `<root>/terminal-ingestion/<tab>.sqlite3`. The
native terminal host completes recovery and the source replay of Cursor before
it gives the first lines to Lines. The journal does not change these items:

- V1 lines and positional indices.
- Comment anchors and cut locations.
- Relay formats.
- The telemetry database.

Each journal is for one app session and one provider only. Its native session
can be missing at the start, when the CLI gives the identity after the start.
`bind_native` records the authenticated identity one time, in an immediate
transaction. It rejects a different identity. The migration from schemas 1–3
does not change receipts or canonical bytes. Schema 2 added the optional native
binding. Schema 3 adds prepared input intents. Schema 4 adds indexes for the
causal completion of native user messages and for committed input ranges.

A private, read-only identity lookup lets Resume find a native identity that
the app committed before a crash, when the crash lost the board update. A fresh
start does not adopt a previous bound journal. Before resume, the captured account
continues to authorize the accurate native source again. Cursor can validate
its original session database directly, before its derived JSONL file is there.
AGY 1.5.10 also validates its captured conversation DB and its accurate cascade
identity. Its [native-store contract](native-antigravity-store.md) gives the
provenance of steps and output files. If the board and the private evidence do
not prove a native identity, Resume fails with a clear error. It does not start
a different native conversation.

The `captures` table of the journal uses a source **occurrence and version** as
its key, not the message text. Two prompts with the same text are different
occurrences. Native steps that can change keep their occurrence while their
version changes. The app does not capture a captured version again at any time.
A private object checkpoint records the projection state that is necessary to
calculate a subsequent version. Cursor uses the authenticated source identities that
[its native-store contract](cursor.md#native-store-recovery) gives. It does
not use positions in its rewritten JSONL export.

Claude rows and Antigravity steps have `sourceScope: record`. Their sealed
status does not mean that the agent turn is complete. Their private
`sourceUser` identifies the native user occurrence that caused them. Tool
results keep the association of the call owner, also when they come after a
different input. Claude follows the known `parentUuid` ancestry. It keeps the
association of a row after the app sets it, also across rewrites. Unknown
ancestry cannot authorize the effects of live work.

The journal must have the causal user receipt. Late output from a source that
the app can use becomes historical only after one of these items:

- A durable canonical completion.
- A subsequent committed range of user messages.

Prepared inputs without a public echo do not set this limit. App submissions
with a committed echo set it, also before the native alias comes. The app
compares the original ranges, not the sequence in which it added the receipts.
Thus, replay and aliases do not start a previous turn again. Legacy receipts without
this attribution can update the current history quietly. The migration does not
invent completion evidence.

The full Antigravity store also records its native `trajectoryId` in a private
receipt without content before it accepts its first capture. The binding stays
after a restart. The app examines it before deduplication and before it accepts
new steps. The same cascade or session ID does not authorize a different
trajectory. Previous receipts without this field get a binding from the
first authorized observation of the full store. The migration does not invent a
trajectory identity.

The caller has one writer gate for native output, app echoes and notices. While
it has that gate, it records the accurate next canonical byte and line
positions. SQLite commits an intent with the accurate canonical bytes and their
SHA-256 before the app adds those bytes at the end of the file. The app writes
the canonical file fully to the disk before SQLite acknowledges the capture.
Committed rows keep the provenance, the checkpoint and the byte and line
ranges. The app removes their temporary canonical payload. When the app proves a
native input, it can make it an alias of the committed range of an app
submission. It does not add a second echo. Global deduplication by text is not
permitted.

The private checkpoint of an app submission keeps these items:

- An opaque ID.
- The hash of the proven normalized input of the provider.
- The native user occurrence that comes before it.

The canonical echo keeps the original text. After a crash, the app puts back a
maximum of 64 unbound submissions in causal sequence. The causal sequence
identifies the echo. The normalized hash only confirms that the CLI kept the
text.

If the hash of the causal candidate is different, the app writes
`terminal.history.echo_mismatch` to the log. It continues to bind the echo and
keeps the original text of the app. CLIs change the format of prompts between
versions. A rejection stops the agent and causes a failure of each subsequent
resume on the same record. Thus, the app does not reject the echo.

After a crash, a prepared input with a different hash is not certain. It is
possible that the app did not write it. It is also possible that the CLI
accepted it and changed its format. The app writes
`terminal.history.prepared_uncertain` to the log and keeps the native prompt as
a separate message. A line-free `uncertain_submission` receipt closes the input
ID. Thus, the board dispatch that the app saved counts as accepted, and the app
does not write it again at any time.

The proven paste normalization of Cursor changes CRLF/CR to LF and tabs to two
spaces, without NFC normalization. Claude Code records prompts in NFC. Thus,
its normalized input is the NFC form. On macOS, attachment paths come as NFD.
Claude Code also records a prompt that is one long paste in
`<pasted_content id="…">`. When a prompt is accurately one such block, the
Claude adapter removes the wrapper. Thus, the echo and the shown text are the
pasted text.

Two conditions are necessary before the app accepts a different app prompt or
reports that it is up to date:

- A source baseline.
- An empty queue of pending submissions.

 The app does not send an input that is not confirmed again
automatically at any time.

Before the app tries PTY input, `prepared_inputs` keeps one private intent with
these items:

- The opaque submission ID.
- The original text and its hash.
- The normalized hash.
- The timestamp.
- The native causal watermark.

The limit of its serialized value is 2 MiB, and the limit of its text is 1 MiB.
A read gets a maximum of two rows, so that extra rows from corruption cause a
clear failure. When the app prepares an intent, it does not publish the
canonical echo. When there is a durable receipt, the app deletes the temporary
original payload. If a crash occurs between the append and that deletion, the
app uses the receipt to make the state correct when it opens the journal.

An intent without a resolution does not authorize a different write at any
time, also for the same text. The app can cancel it only when the process input,
which stays reserved, proves that the app did not try to write a byte. Source
replay can prove the accurate next accepted native user occurrence. It can then
keep the original app text before it makes the alias. Without evidence, the
intent stays uncertain and stops input. Other source adapters must prove the
same correlation before they report the same recovery.

Recovery runs before a new writer starts. It compares the current tail with the
accurate prefix of the intent. It adds only the missing suffix and then
acknowledges it. A different prefix, a shorter file or a corrupt intent stops
recovery and keeps the two artifacts. Recovery changes only the storage. It
does not send the replay through the Pump. It does not send a completion,
notifications, telemetry or new input. The host loads the canonical Lines after
recovery.

Live frames use the commit port of the pump. A proven revision of finished
content updates the public lines. It does not start execution, telemetry or
reactions again. App input reserves its opaque ID before the app writes to the
PTY. Native observations wait until the canonical echo commits. If a commit
fails, the app stops that process. It does not publish an echo that is not in
the journal or try the write again.

If the commit of usual output or of a notice fails, the app marks that
incarnation of the process as failed. It stops input readiness and stops the
process on the observation worker. Thus, a subsequent successful write cannot
silently skip the missing occurrence.

The journal uses a private directory, a `0600` file, WAL with
`synchronous=FULL` and an explicit check of the identity and schema. The app
rejects unknown schemas. These limits apply:

- Each batch: 8 MiB and 4,096 canonical lines.
- Checkpoint objects: 64 KiB.
- Source keys: 1,024 bytes.

Reads of pending payloads and checkpoints keep their limits, also for damaged
journal rows. The app rejects temporary payloads and private telemetry payloads
as canonical lines. Their source occurrence and version can keep a receipt with
an empty line before live effects. Replay makes that receipt without effects.
Late revisions of temporary metadata update only receipts. Thus, previous usage or
context cannot charge or replace a more recent request. Retained summary, notice and
context report text uses the quiet revision path and keeps the earlier line
positions.

`terminal_agent_journal/tests.rs` contains tests for these items:

- Each crash window for append and acknowledgment.
- Conflicts with the accurate prefix.
- Aliases for repeated input.
- Source revisions.
- Damaged intents.
- The rejection of formats that are not compatible.

`chat/terminal/ingestion/tests.rs` uses the real journal and the portable pump.
Its tests include a restart, missing native tails, aliases for normalized input, source
revisions and a failure before publication. All native providers use durable
commits for app input and output, and the ingestion path for source
occurrences. Append batches keep their causal user cursor. Complete snapshot
batches make it again without global deduplication by text. Before the app can
enable terminal mode for a provider, that provider must agree with the recovery
and control rules.

### Claude

Current Claude history in chat mode reads
`~/.claude/projects/<cwd-slug>/<tab>.jsonl`. When that variable has a
value, it reads the `projects` of the original `CLAUDE_CONFIG_DIR`. Managed
account profiles share that directory through a link. The selection does not
change the path of the conversation. The folder name comes from the path of the
worktree. It is possible that the file is not there until the first message.

### Codex

The UI does not use the native rollout of Codex. Prometeu writes the shown V1
events in `<root>/chats/<tab>.jsonl`. `Tab.agent_session` stores the opaque
thread that `thread/resume` must have.

### Antigravity and retired Gemini sessions

Antigravity owns the native history. `Tab.agent_session` stores its opaque
`conversation_id`, and the app resumes it with `--conversation`. Prometeu
stores V1 in `<root>/chats/<tab>.jsonl`. The provider ID `antigravity` is
different from `gemini`.

Previous `gemini` board values keep that identity and read their current V1 logs,
but they cannot start a process. They do not change to Claude or get agy IDs at
any time. Legacy plan and permission fields have no effect on execution. Previous
Gemini account entries and selections stay opaque on the disk. The app does not
remove secrets or histories. The compatibility tests are in
`crates/core/src/board.rs` and `accounts.rs`.

### Compatibility

The reader accepts a truncated start and isolated incorrect lines. The buffer
in memory has a maximum size, and it truncates only at the end of a line. The
app can give a number to a temporary streaming event and not keep it, when the
complete form replaces it.

`ConversationEventV1` continues to read the legacy stream-json. There is no
destructive migration in place. The reader identifies and ignores the
historical `prometheusV1Mirror` mark in current imported logs. New logs of
Prometeu contain only the canonical V1 event. The app does not have an importer
at this time.

## Secrets and logs

Prompts, outputs, tool results and credentials can contain secrets. Do not send
transcripts to telemetry. Do not write full tokens or configurations to logs.
Errors can record the path and the cause, but not the content or a credential
at any time.

When a terminal tab fails with `err.terminalAgent.history`, the app writes the
failed step to the diagnostic log as `terminal.history.<step>`. Each forced stop
of its agent process writes `terminal.abort.<reason>`. A `terminal.launch` line
connects the generation at the start of subsequent lines to its tab.

## Sharing scope

`Workspace.share_team` is optional. It connects the consent to the organization
and the membership (`organization:<id>:<member>`), or to the legacy team
(`team:<id>`). When it is missing, the consent authorizes only the legacy path.
`team.json` accepts the `cloud` field with the identity, the origin and the
organization. The app does not write tickets. Before it replaces a legacy
configuration, `team.rs` writes a private `team-legacy-<uuid>.json`. Refer to
[migration and rollback](cloud-organizations.md).

`Workspace.remote_control` is false by default. It records consent for the
companion devices of the owner. It is independent of `audience`. An empty list
means remote control without a team audience. When the owner disables the last
kind of access, the app also removes the values of `shared`, `share_team`, `audience` and
`rights`.

`Workspace.rights` is `{ send, control }`, two lists of person IDs:

- `send`: the persons who can send messages (prompts and interrupts).
- `control`: the persons who can control the agent, that is, answer its
  requests. This is more than to view and comment.

The two lists contain only persons who can view. The audience `null` does not
give these rights alone. When the owner changes a right, the app removes the
persons who are not in the directory of the organization at this time. Thus, each list
stays in the `MEMBERS_MAX` persons that peers read from the announcement.

Each share that the app saved from
[ADR 0090](../decisions/0090-approved-remote-input.md) on writes the field, also
when it is empty. Shares from before that ADR load with `rights` null. Persons
can then only view and comment. The owner sees that change one time on each
Mac. The Mac records the dismissal in `localStorage` under
`prometeu:share-rights-notice`. Tests: `share_tests` in
`crates/core/src/workspace_lifecycle.rs` and `fixtures/backend-contract.json`.

`Workspace.confirm_messages` is true when it is missing. It records if
messages from teammates wait in the composer of the owner until the owner sends,
edits or discards them ([ADR 0090](../decisions/0090-approved-remote-input.md)).
Only `set_confirm_messages` changes it. When sharing stops, the choice stays.
The board keeps only this choice. Messages that wait are in the memory of the
frontend, and the app does not keep them at any time. Tests: `share_tests` and
`fixtures/backend-contract.json`. The fixture contains a workspace saved with
the option off and one from before the option.

## Remote machines

`Workspace.host` and `Project.host` name the machine from `machines.json` that
has the clone and operates the workspace
([ADR 0101](../decisions/0101-remote-machines.md)). Their paths are paths on
that machine. Remote project IDs are `remote:<machine>:<path>`. The app writes
the two fields only when they have a value. Thus, boards without remote entries
load and save without a change, and a previous version ignores them.

A remote workspace has no reserved `port`. Its tabs always store terminal mode.
When you remove a machine, the app removes its projects from the board. The app
refuses the removal while a workspace names the machine.

`Workspace.creator` names the agent that made a workspace, with the label that
its MCP configuration gives. This label can be empty. The app writes the field
only when it has a value. When it is missing, the person made the workspace, and
a previous version ignores it. It records authorship, not location. Thus, it is
independent of `host`. At this time, only the host of a machine sets it
([ADR 0103](../decisions/0103-agents-on-remote-machines.md)), but a local
workspace can also have it.

The interface shows the two items separately. The machine chip keeps its server
icon and connection state. A separate "Created by agent" chip shows authorship.
Tests: `src/machines.test.ts`.

`Project.origin` is the `origin` remote of the clone, normalized to
`host/owner/repo` in lowercase. Thus, all URL spellings of one repository are
equal. The app sets it for local projects at startup and when a person adds a
project. For the projects of a machine, it sets the value each time the machine
attaches. When the origin of a remote project is equal to the origin of a local
project, the app shows the two as one project. The sidebar lists its workspaces
under the local project, and the launcher shows only the local project. The app
writes the field only when it knows the value. Tests:
`remote_hosts_round_trip_and_local_entries_omit_them` in
`crates/core/src/board.rs`, `every_spelling_of_one_remote_normalizes_alike` in
`src-tauri/src/remote/workspace.rs` and `src/machines.test.ts`.

## Terminal sessions

`Board.terminals` lists the terminal sessions in the sidebar sequence
([ADR 0096](../decisions/0096-terminal-sessions.md)) as `{ id, folder, name? }`:

- `id` is `terminal-<uuid>`.
- `folder` is the absolute folder where the shell starts.
- `name` is the name that the person set.

The app writes the field only when there is a session, and `name` only when it
has a value. Thus, a board from v0.29.0 or earlier loads and saves back without
a change. A previous version ignores the field and removes it at its next save.
Only the record stays. The scrollback stays in memory, and no process stays
after the app stops. Thus, after a restart, the sidebar shows the session as
stopped. When you open it, a new shell starts in the same folder. Test:
`boards_from_v0_29_0_load_unchanged` in
`crates/core/src/terminal_sessions/tests.rs`, with the board that v0.29.0 wrote
in its contract fixture.

## Delegations

For previous boards, `Board.delegations` has the default `[]`. It keeps these items:

- The owner client ID.
- The worker conversation and workspace identities, independently.
- The initial task and the repository commits.
- Idempotency keys and hashes.
- Executions and observations.

After a process restart, the app keeps the ownership and the queued IDs with
persisted pending input. It marks other incomplete executions as stopped and
removes live background and request observations. No transcript migration
applies. Refer to the [embedded MCP contract](embedded-mcp.md) for the fields
and the retry rules.

## Local telemetry

`<root>/telemetry.sqlite3` stores canonical metadata independently of boards and
transcripts. A private `telemetry-health.json` file records failures. Database
versions and event versions are different. The app keeps future database
versions that it cannot read. Short committed transactions, deduplication by
UUID and occurrence, unlimited retention, export and explicit erasure follow
the [telemetry contract](telemetry.md).

`Board.telemetry_ids` is an additive map of domain keys to opaque UUIDs. It is
empty by default. It connects legacy project IDs that use paths, and it gives
identities to repositories and branches. It does not put names or paths in the
event store. The app uses current UUID workspace and tab IDs again. These
aliases stay as domain state after the erasure of telemetry. Previous boards and
transcripts do not need an event backfill. The current board tests and
`telemetry/tests.rs` cover compatibility.

Turn completions can include a nullable SHA-256 `messageKey`. The app
calculates it from the opaque conversation identity and the last assistant ID.
It gives accurate local lookup for the replay of native Claude transcripts. The
app does not keep the reply text or the raw provider identifier. Previous facts do
not have it, and the app can continue to read them. Additive facts about the
origin, app calls and PR observations use the same SQLite store and erasure
limit. There is no second usage ledger and no backfill of provider transcripts.

`Tab.context_window` is optional. In previous boards, its default is unknown. It
goes with the current `context_tokens`. The historical `tokens` estimate of the
increase does not get a new meaning as consumption or as current occupancy. New
canonical transcript completions can keep optional usage independently of the
local history store. When you erase telemetry, the app removes local queries
and restored footer caches. It does not remove the conversation transcript or
current exports.

## Pending session input

`SessionService` keeps the current `pending_prompt` field and its append
sequence across setup, account changes and process loss. In chat mode, when a
send fails, the app puts the original prompt before the subsequent text. Terminal
mode adds the optional `Tab.native_dispatch: { id, text }`. Previous boards do not
have it, and its default is null.

One queued item moves into this slot, which does not change after that. The app
saves the board durably before it writes to the process. Later prompts stay in
`pending_prompt` and cannot change the ID or the text of that item. While the
acceptance is not certain, the two texts stay visible in that sequence. The ID
is an internal queue identity.

The native host returns `Sent` only after its canonical receipt is durable. As
an alternative, after recovery it returns `AlreadyAccepted` for that accurate ID
and content. In that case, there is no process write, capture, telemetry or
reaction. Only then does the core remove the item from the slot, through a different
persistence barrier of the board.

A crash before this removal is safe, because the same token resolves to its previous
receipt. A crash after the write but before the receipt needs the
prepared-input reconciliation above. The app does not try again automatically
at any time. If a board barrier fails, the slot stays and the app does not
send. The usual Chat continues to use its current queue rules. Refer to
[sessions](application-core.md#sessions-launch-and-workers).

### Cursor sessions

The additive provider ID `cursor` is explicit in boards, saved model choices and
accounts. `Tab.agent_session` contains the opaque ACP session ID. Prometeu
stores the canonical V1 presentation in its current chat log. The CLI continues
to own the native Cursor history. When the app loads that history, it does not
duplicate the presentation history. A failed load does not replace the session.

A person attaches and selects the external account explicitly. The removal
stays after a restart. Account readers that do not know Cursor keep its account
and active selection as opaque data. It is possible that previous board readers
change an unknown provider to Claude. Thus, a downgrade of a Cursor board is
not supported. Refer to the [Cursor contract](cursor.md).

## Delegation navigation compatibility

The app gets the sub-workspace groups from `Board.delegations` and the IDs of
the owner tabs. It does not add a parent field or a migration. When a board has
no delegation list, the legacy flat navigation stays. Current delegated
workspaces with many tabs keep all tabs during the load, but the app rejects new
conversations in them. Refer to [ADR 0099](../decisions/0099-sub-workspaces.md)
and the [embedded MCP contract](embedded-mcp.md#sub-workspace-navigation).
