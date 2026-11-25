---
type: Contract
title: "Local persistence"
description: "Local board, transcript and account formats, migrations and recovery."
status: stable
---

# Local persistence

Current contract.

## Principles

- The logical session outlives the agent process.
- State and credentials stay outside the worktrees.
- Deleting a worktree does not delete the conversation history.
- Sensitive files are born private: `0700` directories and `0600` files on Unix
  platforms.
- State rewriting uses a temporary file, sync and rename so a truncated file is
  never exposed after a failure.
- Old formats get defaults and migrations; they are not discarded for missing
  new fields.

## App root

In release, the default root is `~/.prometeu`. In debug, `~/.prometeu-dev`.
`PROMETEU_ROOT` can replace the root, mainly in tests and isolated instances.

| Data | Path | Ownership |
| --- | --- | --- |
| board | `<root>/board.json` | `board_store.rs` via injected `BoardStore`; models in `crates/core/src/board.rs` |
| external MCP clients | `<root>/mcp-clients/<uuid>.json` | `mcp_access.rs`; private versioned credentials and project scope; see [embedded MCP](embedded-mcp.md) |
| MCP socket discovery | `<root>/mcp-socket` | `embedded_mcp.rs`; private ephemeral socket path, replaced on startup |
| board backup | next to `board.json` | `board_store.rs` |
| copies of unreadable state | `<root>/recovery/<label>-<millis>.json` | `recovery.rs`; see [Unreadable state](#unreadable-state) |
| team and credential | `<root>/team.json` | `team.rs` |
| E2EE identities, TOFU links and replay | `<root>/team-security.json` | `team.rs` (file), `team-security.ts` (internal schema) |
| optional Prometeu account | `<root>/cloud.json` | `cloud.rs`; see the [contract](cloud-account.md) |
| cloud catalog cache and links | `<root>/catalog.json` (`catalog.local.json` is a legacy backup) | `catalog.rs`; see the [contract](cloud-catalog.md) |
| installed skills and packages | `<root>/skills.json`, `<root>/skills-packages/<id>/` | `skills.rs`; see the [catalog](cloud-catalog.md) |
| accounts and per-provider selection | `<root>/accounts.json` | `crates/core/src/accounts.rs` through injected `AccountStore`; `account_store.rs` supplies private file persistence |
| additional authenticated profiles | `<root>/accounts/<uuid>/` | provider adapters |
| last quota snapshot per account | `<root>/usage.json` | `usage.rs` |
| Linear OAuth credential | `<root>/linear.json` | `linear.rs`; private access and refresh tokens, account identity and granted scopes; see [Linear issues](linear-issues.md) |
| Linear issue cache | `<root>/linear-issues.json` | `linear.rs`; private assigned and available lists with fetch time; see [Linear issues](linear-issues.md) |
| GitHub App credential | `<root>/github.json` | `github_auth.rs`; private access and refresh tokens, expiry and login, never sent over IPC; see [GitHub issues](github-issues.md). The retired `<root>/github-issues.json` is ignored |
| optional TypeSafe key and enablement | `<root>/typesafe.json` | `typesafe.rs`; private, disabled by default; see [context evaluation](context-evaluation.md) |
| product analytics consent | `<root>/analytics-enabled.json` | `analytics.rs`; private JSON boolean, missing or invalid files disable capture and trigger the startup question; either saved boolean suppresses the question; see [product analytics](product-analytics.md) |
| product analytics identity | `<root>/analytics-id` | `analytics.rs`; private random UUID, created only when consent and the build key allow capture; its presence does not imply consent |
| V1 transcript for Codex, Antigravity, Cursor and every terminal-mode provider | `<root>/chats/<tab>.jsonl` | `transcript_store.rs`, via the core conversation stream |
| native terminal transcript binding | `<root>/terminal-bindings/<tab>.json` | `chat/terminal/binding.rs`; private versioned identity and exact path, without conversation content |
| native terminal ingestion journal | `<root>/terminal-ingestion/<tab>.sqlite3` | `terminal_agent_journal.rs` and `chat/terminal/ingestion.rs`; private source provenance and durable canonical append |
| app diagnostic log | `<root>/logs/prometeu.log` (previous: `prometeu.log.1`) | `main.rs`; private `0600` stderr when the app inherits a discarded stderr (`/dev/null` or closed, as in a Finder launch), rotated at startup above 5 MiB; terminal-host lines are `<ms> <tab or generation> terminal.<code>` with identifiers and step names only |
| files received through a native promise | `<root>/attachments/<uuid>/<name>` | `file_drop.rs`; private `0700` directory, `0600` file |
| image pasted from the clipboard | `<root>/attachments/<uuid>/pasted.png` | `file_drop.rs`; same folder and permissions, TIFF converted to PNG on macOS, GTK image converted to PNG on Linux |
| plugin hub | `<root>/plugins.json` | `plugins.rs` |
| remote machines | `<root>/machines.json` | `remote.rs`; private `{ v: 1, machines: [{ id, name, ssh, limits: { memory_max, cpu_quota } }] }`, no credentials; see [ADR 0101](../decisions/0101-remote-machines.md) |
| remote transcript mirror | `<root>/remote/<machine>/mirror/<remote path>` | `remote/connection.rs`; private append-only copies of a machine's Claude project files, rebuilt from offset zero when missing; see the [remote host protocol](remote-host.md#transcript-mirror) |
| remote workspaces and tabs removed from the board | `<root>/remote/<machine>/dismissed.json` | `remote/registry.rs`; private JSON array of workspace and conversation IDs that adoption from the machine's registry skips; see [ADR 0103](../decisions/0103-agents-on-remote-machines.md) |
| derived Codex home/marketplace | `<root>/codex-workspaces/<workspace-hash>/[<account>/]` | `plugins.rs`; rebuildable |

Catalog project installation reuses board Project records and stores local paths
in `catalog.json` installation links. No credentials or paths enter the Cloud
document; see [Git projects](cloud-catalog.md#git-projects).

Worktrees live in `~/prometeu/worktrees[-dev]/...`, outside the state root.

Repository scripts, tool declarations and method settings load only from
`.prometeu/settings.toml`. A worktree without this file inherits the original
checkout's file; a worktree with its own file replaces it as a whole. Workspace
scripts receive `PROMETEU_WORKSPACE_PATH`, `PROMETEU_ROOT_PATH`,
`PROMETEU_WORKSPACE_NAME`, `PROMETEU_PORT` when a port is reserved, and the same
port as `PORT`. No other configuration file or environment prefix is used as a
fallback. See [project scripts](../operations/project-scripts.md).

A Linux AppImage launch keeps a desktop entry for the running file outside the
state root, in `$XDG_DATA_HOME` (`~/.local/share` by default):
`applications/co.prometeu.desktop.desktop` and its icon
`icons/hicolor/256x256/apps/co.prometeu.desktop.png`. They hold no app state.
`desktop_entry.rs` replaces each through a rename only when its content differs,
and removes the entry when AppImageLauncher or Gear Lever lists the same file;
see [ADR 0055](../decisions/0055-linux-desktop.md).

Transcript lookup first honors the tab's saved conversation mode: terminal mode
always reads the app-owned V1 log. Chat mode uses the tab's resolved provider,
including a frozen task profile, before falling back to the workspace provider.
The same resolver serves stopped snapshots and MCP history. Existing chat tabs
retain their transcript paths without a format migration.

The native terminal transport persists a native binding only after its process's
authenticated root hook passes observation validation. Version 1 contains
`version`, `session`, `provider`, `nativeSession` and the absolute `transcript`
path. It contains neither a bridge token nor message content. Reads are bounded
to 64 KiB and require exact app/native identity and provider; unknown versions,
unknown fields and mismatches fail. Writes use the private atomic file helper.
The captured account's permitted roots reauthorize the path on resume, including
intentional shared native history across accounts. A previous account id grants
no additional filesystem access.

Resume loads this binding and completes a silent native baseline before spawning
the CLI. This avoids waiting for a new prompt to produce the hook needed to read
its history. Cursor may instead reauthorize its exact native session database
when the derived JSONL path has not yet been reported. A missing or invalid
source binding rejects terminal resume explicitly; it never selects another
session or searches unrelated histories. Existing chat
tabs do not read this additive format and require no migration. Compatibility is
covered in `chat/terminal/binding.rs` and the observer's resume tests. The format
alone does not reconcile output missed during an app crash; the ingestion
journal below provides the Cursor recovery path. Other providers still need
their source-provenance integration before terminal mode can be enabled.

Promised files (such as a capture's thumbnail) are materialized by AppKit in a
new directory per gesture, without overwriting previous attachments. The backend
only delivers to the UI files that exist inside that directory. They are not
removed when the message is sent, when the session ends or when the worktree is
deleted, because the transcript may reference their paths. There is no automatic
cleanup at this stage. Ordinary Finder files still use their original paths.
That folder is additive: a rollback ignores the folder and preserves the paths
already sent; no existing format requires migration.

Prometeu neither looks for nor writes in the Prometheus roots. Worktrees adopted
in the old migration remain in `~/prometheus/worktrees`; cleaning up a multi-repo
workspace still accepts that path, and the two applications must not operate on
the same folder at the same time.

## Notification preferences

The legacy `prometeu:som` localStorage key is no longer read or written. If it
exists, it stays inert; there is no migration and no change to the board or
transcripts. The new `prometeu:notifications` version-1 record stores local
notification choices, with notifications and sound disabled by default. Missing,
invalid and future-version records keep those defaults. See the
[notification contract](notifications.md) and
[ADR 0054](../decisions/0054-local-notifications.md).

## Collaboration security

`team-security.json` is additive, private (`0600` in a `0700` directory), with
atomic writes and an 8 MiB limit. The envelope is
`{ version: 1, scopes: { ... } }`. Each scope combines the origin, the
organization/team, the local Cloud account and the membership. It contains a
P-256 identity (private JWK and public key), member links, the announcement
sequence, the last owner/key/revision/ID per share, receipts for remote messages
and the clock of the last consumption. Limits: 64 links, 4096 shares and 4096
unexpired receipts per scope. Receipts expire within two minutes.

Each scope also holds `approved`, the member → key pairs whose remote input the
owner approved, and `paused`, the member → workspace IDs where input from an
unapproved key was discarded until the owner answers its notice. `paused`
holds identifiers only, never message content. Limits: 64 approvals and 64
paused members with up to 64 workspaces each. A scope written before approvals
existed has no `approved` field: on load, its links become approved, so an
upgrade keeps teams working. A new scope starts with no approvals. A rollback
to a version without these fields drops them at its next write; upgrading again
approves the links present then. See
[ADR 0090](../decisions/0090-approved-remote-input.md).

Identity creation, a member link — first or replaced —, an approval, a revision
and a receipt are written before the corresponding use. Corruption, an unknown
version and a read/write failure block collaboration; they do not silently
regenerate keys.
Leaving the team, switching organizations, logging out and renewing a ticket do
not remove the file. The webview's operations are serialized; the Rust write
uses the same lock during read/validation/write.

The file contains secrets and is neither an encrypted backup nor a content key
on the server. Losing it loses TOFU continuity and access to the encrypted
comments for the old identity. A new device joins with a new key and peers adopt
it automatically ([ADR 0042](../decisions/0042-automatic-key-rotation.md)). A rollback ignores and preserves the file; it never converts it into v3
credentials. The browser mock stores only fictional identities in localStorage
and exercises the same cryptographic channel.

Tests: `src/team-security.test.ts` and the `src-tauri/src/team.rs` tests.
Network contract and limits: [relay v4](relay-v4.md).

## Optional context evaluation

`typesafe.json` is additive: `{ version: 1, enabled, key }`, every field with a
default. A missing file, an old installation or a file without both a key and
`enabled: true` keeps the integration disabled; saving a key does not enable
it, and removing the key deletes the file. An unreadable file stays disabled
and is reported in Settings without blocking work. The key never enters the
board, transcripts, `team.json`, the relay or Cloud catalogs, and no IPC result
returns it. Tests: `src-tauri/src/typesafe.rs`. See the
[contract](context-evaluation.md).

## GitHub notification read state

`prometeu:github-notifications:read:<origin>#<user id>` in desktop webview
localStorage stores `{ "before": "<id>", "ids": ["<id>"] }` for one Prometeu
account on this Mac only. Notification rows are
not persisted; they come back from the Cloud feed. See
[GitHub notifications](github-notifications.md).

## Settings navigation preference

`prometeu:configuracoes` remains a local string in desktop webview localStorage.
New selections write `geral`, `agentes`, `recursos`, `acoes`, or `trabalho`.
Reading a historical page selects its new group: `contas` and `padroes` open
Agents; `notifications` and `app` open General; `ferramentas`, `plugins` and
`skills` open Resources with the corresponding filter; `projects`, `time` and
`integracoes` open Work and team. Unknown values open General. Reading does not
rewrite the saved value or any underlying preference. No board, IPC or provider
format changes. Compatibility is covered by `src/settings-navigation.test.ts`.

## Skill kickoff

`Tab.kickoff` is an optional `<package>/<skill>` string recording the *Start
with* skill a conversation began from; it is omitted when absent, so ordinary
tabs keep the previous shape and older boards load with none. A resume adds
that package to the tab's resolved tools while the skill is still installed,
and otherwise resumes without it and warns in the conversation; no selection
layer stores it.

The repository declares where artifacts land with `[method] artifacts` in
`.prometeu/settings.toml`, read by `scripts.rs` with the same clone inheritance
and primary-repository rule as `[tools]`. Only a relative path inside the
repository is accepted; otherwise nothing is named.

The last explicit launcher choice, including none, lives in desktop webview
localStorage as the string `prometeu:kickoff`. A value the installed catalog no
longer offers, or unavailable storage, starts without a skill. Tests:
`crates/core/src/board.rs::tabs_without_a_kickoff_keep_the_previous_format`,
`scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented` and
`src/kickoff.test.ts`. See
[ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md).

## Automatic worktree cleanup

`prometeu:auto-cleanup-force` is a desktop webview localStorage string. Only `true`
enables automatic cleanup after a successful manual archive or finish. Missing,
unrecognized or inaccessible values keep the scoped confirmation, preserving
upgrade behavior. `false` disables automation. The preference applies across
projects in that webview and survives restarts; it is not stored on the board
or synchronized through the relay. No IPC or board format changes are needed.
Enabled cleanup runs with `force`, so dirty or unmerged work is deleted. The
earlier `prometeu:auto-cleanup` key recorded consent to a cleanup that asked
before deleting that work, so it is no longer read and those opt-ins start off;
see the
[Git contract](git.md#archived-worktree-cleanup) and
[ADR 0048](../decisions/0048-scoped-cleanup-offer.md).
Tests: `src/cleanup.test.ts`.

## Sleep preference

`prometeu:acordado` is a desktop webview localStorage string. `off` permits
normal sleep; `on` keeps the system and display awake while the app is open;
`agent` keeps both awake only while a local tab is `rodando`; and the additive
macOS option `agent-system` keeps the system awake during that same state but
permits display sleep. The original three values retain their behavior and are
never rewritten automatically. Linux presents the original choices only.
The preference is local to this Mac, never the board or relay. Missing or
unrecognized values resolve to `off`. The native assertion is ephemeral and
ends when the app exits; no sleep assertion is persisted.

## Sidebar width

`rail-w` is an optional decimal pixel width in desktop webview localStorage.
Missing, non-finite or non-positive values use the design token default (256px).
Dragging the sidebar's right edge or using Left/Right on its separator saves
this local preference. Double-click or Home removes it and restores the default.
Rendering clamps the width to 200px–50% of the window (at least 200px), including
on startup and window resize, without overwriting the preferred width on resize.
The right panel is additionally capped by the remaining workspace width, reserving
320px for the conversation, or half the workspace when it is narrower than 640px.
This rendering cap applies to restored widths, either resize handle and panel
visibility changes without rewriting the saved `side-w` preference.
It does not change the board, relay or the existing right-panel `side-w` key.
Older installations need no migration; older versions ignore the new key.
Compatibility and browser interaction coverage: `e2e/sidebar-resize.spec.ts`.

## Model selection preferences

These preferences live in desktop webview localStorage, not the board or relay:

- `prometeu:model-choice`: JSON `{ agent: ProviderId, model: string }`.
- `prometeu:model-favorites`: JSON array of the same pairs, deduplicated by both
  fields. Favorites are local to this installation and shared by desktop pickers.
- `prometeu:effort`: native effort string; empty means provider default.

The old `prometeu:model` string remains untouched. When the new choice is absent,
wait for successful discovery of all installed agents and migrate only an
unambiguous model/provider association;
known historical Claude aliases can still identify old Claude preferences.
An ambiguous or unrecognized legacy value requires explicit selection. A new
installation defaults to the first installed provider's native model default.
Malformed preference JSON is ignored. Removing a model from a catalog does not
remove its saved default, favorite, tab choice or profile choice. Favorites that
are unavailable in the current account remain stored but are not selectable.

Historical Codex `ultracode` effort is read as `ultra`; new selections write native
values. Historical unsupported efforts stay visible and are not rewritten merely
by opening or saving an unrelated profile field. Explicit model changes retain
only supported effort; otherwise they select the provider default and explain
the adjustment. `Choice` and `SessionLaunch` retain their existing wire shapes.
An unavailable saved model or unsupported effort cannot start a new workspace
until explicitly corrected; its saved preference and existing conversations are
not rewritten. Catalogs and freshness state are memory-only and are invalidated on account
changes. Tests: `src/model-choice.test.ts`, `src/agents.test.ts`, and
`e2e/model-picker.spec.ts`, with action profile compatibility in `e2e/actions.spec.ts`.

## Board

`Board.conversation_mode` is the default for new conversations, `chat` or
`terminal`. Each `Tab.conversation_mode` stores the mode selected when that
conversation was created. Both default to `chat` and omit that value on save,
so an older tab stays in chat when the global default changes. Resume restores
the saved tab mode, including reusable actions; changing a model or the global
preference does not convert an existing transcript. `Launch.conversation_mode`
is optional: omission inherits the board preference only on creation, while an
explicit `chat` overrides a terminal default.

macOS exposes the default in Settings / General and runs native terminal
conversations. Linux rejects terminal launches explicitly. Compatibility
is covered by the board revival and mode round-trip tests in
`crates/core/src/board.rs`, launch deserialization in
`crates/core/src/session/launch/tests.rs`, and desktop task/tab resume tests in
`src-tauri/src/session.rs`.

Workspace stages are retired ([ADR 0102](../decisions/0102-remove-workspace-stages.md)).
Legacy `stages`/`columns` and `stage`/`column` fields are ignored on load and
omitted on save. Workspaces and transcripts survive unchanged. Versions requiring
those fields cannot read a newly saved board; use a pre-upgrade backup to downgrade.

`Board` contains projects, workspaces and the optional `actions`
catalog. The order of `projects` is the sidebar order the person chose by
dragging; older boards keep their registration order. `Project.group` is the
optional sidebar folder; it is written only when set, so older boards load
unchanged and older versions ignore it. Folders exist only as that name on their
members. The sidebar shows folders first, alphabetically, then loose projects,
and a drag reorders only within one folder or among loose projects. `Tab.task` stores the resolved configuration and the tasks' cursors.
The absence of those fields keeps previous sessions working. The catalog
receives Code review exactly once, recorded in `actions.defaults_initialized`;
see [actions](actions.md). `Workspace` contains repositories, branch, worktree,
agent configuration, the MCP/plugins/skills selection, sharing and tabs.
`preserve_branches` lists clone paths whose existing local or remote branches
must survive worktree cleanup. Each repository is checked separately, so a
new branch in another repository still needs merge safety and is deleted on
cleanup. Older boards omit the list and retain their previous cleanup behavior.

`Tab.tokens` stores an incremental estimate of the tokens used in the
conversation; `Tab.context_tokens` stores the last observed context so only the
growth is added. When the context drops after compaction, the new value starts
another segment and adds to the total. Old boards without `context_tokens` treat
`tokens` as the total and the initial cursor, without duplicating the value. The
tab also contains identity, status, pending message, model override and the
external identity used for resume when needed. `Tab.interrupted` is set on
revival for a tab that was `rodando` or `querendo` when the app ended, and the
next accepted message clears it; it is written only while true. When such a tab
starts a process again, its transcript ends at the cut, so `Tab.cuts` records
that line count; replay places `session.state: stopped` there, and continuing
never makes the cut reply look finished. Transcripts only grow at the end, so
the position holds for native and app-owned logs alike. `cuts` is written only
when not empty, so older boards load and save unchanged. An empty `Tab.title` is an
unnamed tab: the interface shows the model it talks to. Old boards with the
invented name `conversa` or `conversa N` are normalized to empty on load.

`Board.tools` keeps the global layer of the tool selection and
`Board.tool_trust` the decisions that let a repository's declaration activate;
both are described in [Tool selection layers](#tool-selection-layers).

Removing a project only takes its registration out of the board. The repository,
worktrees and conversations are not deleted; workspaces linked to it appear
under **No project**. On load, only legacy workspaces without the `project`
field rebuild the registration. An explicit id without a matching project
preserves the removal.

On load:

- unknown enum values fall back to a safe state when declared by
  `serde(other)`;
- added fields use `serde(default)`;
- aliases preserve old names during migration;
- runtime states are reconciled: processes do not survive the app.

A change that removes, renames or alters the semantics of a persisted field
requires a test with JSON from the previous version.

### Unreadable state

Prometeu never replaces bytes it could not read. When `board.json` does not
deserialize, the board comes from `board.json.bak`; when the backup does not
either, it starts empty. Before anything replaces an unreadable file, its bytes
are copied into the private `<root>/recovery/` folder as `board-<millis>.json`
or `board-backup-<millis>.json`. A copy identical to one already there is not
written again, a new copy never takes an existing name, and only the newest five
distinct copies of each file remain.

An unreadable `board.json` is copied at load, so the original stays in place
until a save replaces it. While that copy fails, or when the file could not be
read at all, every save returns an error and the files on disk stay untouched.
An unreadable backup is copied at load when the board falls back, and otherwise
before the first save of the session refreshes it. While that copy fails, saves
still write `board.json` but leave the backup as it is.

At startup the interface asks `board_recovery` and, when the board came from
the backup or started empty, shows a dialog with the recovery folder and any
copy error; see [IPC](ipc.md#board-recovery). `team.json` follows the same
rule: a file that does not parse still reads as no team, but joining or leaving
first copies it to `recovery/team-<millis>.json`, and a file that cannot be read
blocks both.

Neither the board nor `team.json` changes format. Coverage lives in
`board_store.rs`, `recovery.rs` and `team.rs`. See
[ADR 0091](../decisions/0091-keep-unreadable-state.md).

## Ordered board publication

`BoardPublisher::publish` serializes snapshot creation, enqueueing, and the `board`
event under one publication mutex. It briefly locks the current board to clone
it, then releases the board lock before enqueueing and emission. Competing
publishers cannot enqueue or emit an older captured snapshot after a newer one.
The worker still coalesces writes and performs disk I/O outside the board lock.

`save_now` uses the same publication mutex and waits for a flush acknowledgment.
Flushing does not terminate the worker: action transitions also flush while
the app remains running. Later changes must still be persisted. Regression
coverage lives in `crates/core/src/publication/tests.rs`, including concurrent
publication, durable barriers, storage failures and disconnected delivery.
Storage and event delivery use injected ports; see the
[application-core contract](application-core.md). Terminal port assignment uses
the same persistence queue, preventing older snapshots from overwriting it.

No board fields or serialization change. See
[ADR 0023](../decisions/0023-ordered-publication.md).

## Derived plugins

The hub is Prometeu's source of truth. The copy and the marketplace under
`<root>/codex-workspaces/<workspace-hash>/marketplace/` are a cache: they carry
a hash of the source, can be recreated and do not enter the board. The same
directory contains a derived `config.toml` that inherits the real configuration
and keeps the trust and the active hook state of that workspace. The original
account's layer stays at that path; managed accounts get their own
subdirectory. The remaining `CODEX_HOME` entries point to the profile captured
at spawn. Each profile keeps its own credential, while sessions, skills and the
plugin cache stay shared. The global configuration receives neither Prometeu's
marketplace nor its activation. Removing the workspace from the board or
returning its worktree deletes that derived layer, without following the links
to the shared state. The complete contract is in
[`plugin-marketplace.md`](plugin-marketplace.md).

## Tool selection layers

MCP servers, plugins and skills are three independent axes, each selected in
three layers resolved in the order global → project → workspace
([ADR 0045](../decisions/0045-layered-tool-selection.md)). Per axis, a layer is
either absent — inherit from the layer above — or an object:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

An omitted `base` means `"inherit"`, so a hand-written `[tools]` that declares
only `add` never erases the layers above it. Within one layer `add` is applied
first and `remove` has the last word.

| Layer | Where it lives | Owner |
| --- | --- | --- |
| global | `Board.tools` in `<root>/board.json` | `crates/core/src/board.rs` |
| project | the `[tools]` table of `.prometeu/settings.toml`, in the repository | `scripts.rs` |
| workspace | `Workspace.mcp`, `Workspace.plugins` and `Workspace.skills` | `crates/core/src/board.rs` |

`Board.tools` is app-local and holds one `Selection` per axis, all absent by
default, so an old board keeps injecting exactly what it used to. The project
layer is the only one written outside the app root: the file is versioned, so a
repository author could otherwise choose packages and hooks for whoever clones
it. For a multi-repository workspace only the **primary** repository's `[tools]`
is read, matching the single-root behavior of Claude Code and Cursor and the
launcher's existing use of the primary repository.

`Board.tool_trust` stores one decision per primary repository:
`{ repo, hash, approved, at }`. `repo` is the `origin` remote URL when one
exists and the clone's absolute path otherwise; `hash` is the SHA-256 of the
declared `[tools]` section, recomputed by the backend and compared with the
version displayed to the person before the decision is recorded (ADR 0047).
Stale submissions leave this persisted format and existing decisions unchanged. `approved: false` is an explicit rejection: it quiets the prompt but
keeps the declaration's items out of every spawn, labeled as rejected. A
declaration whose hash differs from the stored decision — approval or rejection
— is resolved but not injected, and prompts again, until the person decides;
the decision is never written into the repository. See
[`plugin-marketplace.md`](plugin-marketplace.md).

### Workspace migration

`Workspace.mcp` and `Workspace.plugins` were `Option<Vec<String>>`. On load
`null` stays `null`, `[]` becomes `{base:"none", add:[]}` and `[ids]` becomes
`{base:"none", add:ids}`, so a migrated board resolves to the same set it used
to inject. Entries matching `skill-<id>` move from `plugins` to the new `skills`
axis; the hub keeps them as packages, so only the state and the interface change.
`Workspace.skills` is absent, therefore inherited, in old boards.

The migration is one-way: a previous version cannot deserialize the object form
and falls back to the board backup, then to a default board. Versions that keep
[unreadable state](#unreadable-state) copy the unreadable files before saving. The resolved set is
captured at spawn, so a change in any layer takes effect at the next spawn or
resume
([`agent-runtime.md`](agent-runtime.md)).

## Quotas

`usage.json` stores, per local account ID, the known windows and the timestamp
of the last change. The old `claude` and `codex` keys still identify the
original profiles, without rewriting previous caches. The registry and the
global selection are in [`accounts.md`](accounts.md); they do not enter the
board or the relay. Each window has a kind, a percentage and a reset. `scope`
and `label` are optional: old snapshots without those fields are still a single
quota; new snapshots use `scope` to keep general quotas and model or feature
buckets separate. The file is a cache: a valid read from the provider replaces
the persisted content.

## Transcripts

Terminal conversations use `<root>/chats/<tab>.jsonl` for every provider,
including Claude. Accepted input, observed canonical output and application
notices append to that history. Native bootstrap and rewrite snapshots never
replace or prepend published history. Native recovery appends only uncaptured
source occurrences and versions, with durable identity receipts. Cursor uses
its resumable store IDs, Codex its thread/turn/item IDs, Claude native row UUIDs
and Antigravity native step indices. Unsupported source content withholds
coverage rather than silently certifying a complete baseline.
The native terminal can therefore show compacted provider state while Prometeu
retains the prior conversation, positional comment anchors and cut locations.
Compatibility tests in `chat.rs` cover mode-aware routing and retention of
canonical notices, usage and transcript positions across a native rewrite.

### Native ingestion journal foundation

`terminal_agent_journal.rs` provides a private SQLite schema 4 for durable
native-source provenance at `<root>/terminal-ingestion/<tab>.sqlite3`. The native
terminal host completes recovery and Cursor source replay before seeding Lines.
It does not change V1 lines, positional indices,
comment anchors, cut locations, relay formats or the telemetry database.

Each journal binds exactly one app session and provider. Its native session may
start absent when the CLI allocates identity asynchronously; `bind_native`
records the authenticated identity once, under an immediate transaction, and
rejects a different identity. Schemas 1–3 migrate without changing receipts
or canonical bytes. Schema 2 introduced the optional native binding; schema 3
adds prepared input intents. Schema 4 indexes causal native-user completion and
committed input ranges. A read-only, private
identity lookup lets Resume recover a native identity committed before a crash
lost its board update. Fresh does not adopt an old bound journal. The captured
account still reauthorizes the exact native source before resume; Cursor can
validate its original session database directly before its derived JSONL exists.
AGY 1.5.10 similarly validates its captured conversation DB and exact cascade identity;
its [native-store contract](native-antigravity-store.md) describes step and output-file provenance.
If neither the board nor private evidence proves a native identity, Resume
fails explicitly instead of starting another native conversation.

The journal's
`captures` table keys a source **occurrence and version**, not message text.
Repeated prompts are distinct occurrences. Mutable native steps retain their
occurrence while changing version; an already captured version is never
captured again. A private object checkpoint records the projection state needed
to compute a later version. Cursor uses the authenticated source identities
described in [its native-store contract](cursor.md#native-store-recovery),
rather than ordinals in its rewritten JSONL export.

Claude rows and Antigravity steps carry `sourceScope: record`: their sealed
status is not an agent-turn completion. Their private `sourceUser` identifies
the originating native user occurrence; tool results retain the call owner's
association even when they arrive after another input. Claude follows known
`parentUuid` ancestry and preserves a row's established association across rewrites.
Unknown ancestry cannot authorize live work effects. The journal requires the
causal user receipt, and only a durable canonical completion or a later committed
user-message range makes otherwise supported late output historical. Prepared
inputs without a public echo do not establish this boundary. App submissions
with a committed echo do, even before the native alias arrives. Comparing the
original ranges rather than receipt insertion order keeps replay and aliases
from reviving an old turn. Legacy receipts without this attribution can update
existing history quietly; migration does not invent completion evidence.
The full Antigravity store also pins its native `trajectoryId` in a private,
content-free receipt before accepting its first capture. The binding survives
restart and is checked before deduplication and newly discovered steps, since
the same cascade/session ID does not authorize adopting a different trajectory.
Older receipts lacking this field acquire a binding from the first authorized
full-store observation; no trajectory identity is fabricated during migration.

The caller holds one writer gate covering native output, app echoes and notices.
Under that gate, it records the exact next canonical byte and line positions.
SQLite commits an intent with exact canonical bytes and their SHA-256 before
those bytes are appended. The canonical file is synced before SQLite acknowledges
the capture. Committed rows retain provenance, checkpoint and byte/line ranges;
their temporary canonical payload is cleared. An explicitly proven native input
may alias an app submission's committed range without appending a second echo.
No global text deduplication is permitted.

An app submission's private checkpoint retains an opaque ID, the hash of the
provider's proven normalized input and the preceding native user occurrence.
The canonical echo keeps the original text. At most 64 unbound submissions are
restored in causal order after a crash; a mismatched candidate is an explicit
conflict. Cursor's proven paste normalization changes CRLF/CR to LF and tabs to
two spaces, without NFC normalization. Claude Code records prompts in NFC, so
its normalized input is the NFC form (macOS attachment paths arrive as NFD).
A source baseline and an empty pending
submission queue are both required before admitting another app prompt or
claiming catch-up. An unconfirmed input is never resent automatically.

Before attempting any PTY input, `prepared_inputs` retains one private intent
with its opaque submission ID, original text/hash, normalized hash, timestamp
and native causal watermark. Its serialized value is bounded to 2 MiB, its text
to 1 MiB and its read to two rows so corrupt excess rows fail explicitly. The
canonical echo is not published by preparing an intent. Once a durable receipt
exists the transient original payload is deleted. A crash between append and
that deletion is reconciled from the receipt on open. An unresolved intent
never authorizes another write, even for identical text. It can be cancelled
only when the still-reserved process input proves that no byte write was
attempted. Source replay can prove the exact next accepted native user
occurrence and preserve the original app text before aliasing it; absent
evidence remains uncertain and blocks input. Other source adapters must prove
the same correlation before claiming equivalent recovery.

Recovery runs before any new writer. It compares the existing tail with the
intent's exact prefix, appends only the missing suffix and then acknowledges it.
A different prefix, shortened file or corrupt intent stops recovery and preserves
both artifacts. Recovery is storage-only: it does not pass replay through the
Pump or emit completion, notifications, telemetry or new input. The host loads
canonical Lines after recovery. Live frames use the pump's commit port; a proven
revision of finished content updates public lines without reopening execution,
telemetry or reactions. App input reserves its opaque ID before writing to the
PTY, and native observations wait until the canonical echo commits. A commit
failure retires that process instead of publishing an unjournaled echo or
retrying the write. Generic output or notice commit failures latch the same
incarnation as failed, revoke input readiness and retire it on the observation
worker; a later successful write cannot silently skip the missing occurrence.

The journal uses a private directory, a `0600` file, WAL with `synchronous=FULL`
and an explicit identity/schema check; unknown schemas are rejected. Each batch
is bounded to 8 MiB and 4,096 canonical lines, checkpoint objects to 64 KiB and
source keys to 1,024 bytes. Pending payload/checkpoint reads remain bounded even
for damaged journal rows. Ephemeral and private telemetry payloads are rejected
as canonical lines; their source occurrence/version may retain an empty-line
receipt before live effects. Replay creates that receipt without effects. Late
revisions of ephemeral metadata update only receipts, so old usage/context
cannot charge or replace a newer request. Retained summary, notice and context
report text uses the quiet revision path and keeps earlier line positions.
`terminal_agent_journal/tests.rs` covers every append/ack crash window, exact
prefix conflicts, repeated input aliases, source revisions, damaged intents and
compatibility rejection. `chat/terminal/ingestion/tests.rs` composes the real
journal and portable pump to test restart, missing native tails, normalized
input aliases, source revisions and failure-before-publication. All native
providers use durable app/output commits and the source occurrence ingestion
path. Append batches retain their causal user cursor, while complete snapshot
batches reconstruct it without global text deduplication. Provider recovery/control
conformance remains a prerequisite for enabling terminal mode.

### Claude

Existing Claude chat-mode history reads `~/.claude/projects/<cwd-slug>/<tab>.jsonl`, or the
`projects` of the original `CLAUDE_CONFIG_DIR` when configured. Managed account
profiles share that directory through a link; the selection does not change the
conversation's path. The folder derives from the worktree's path. The file may
not exist until the first message.

### Codex

Codex's native rollout is not used by the UI. Prometeu writes the displayed V1
events in `<root>/chats/<tab>.jsonl`; `Tab.agent_session` stores the opaque
thread required for `thread/resume`.

### Antigravity and retired Gemini sessions

Antigravity owns native history. `Tab.agent_session` stores its opaque
`conversation_id`, resumed with `--conversation`; Prometeu stores V1 in
`<root>/chats/<tab>.jsonl`. Provider ID `antigravity` is distinct from `gemini`.
Old `gemini` board values retain that identity and read their existing V1 logs,
but cannot start a process. They never fall back to Claude or acquire agy IDs.
Legacy plan/permission fields have no execution effect. Old Gemini account
entries and selections remain opaque on disk; no secrets or histories are
removed. Compatibility tests live in `crates/core/src/board.rs` and `accounts.rs`.

### Compatibility

Reading tolerates a truncated beginning and isolated invalid lines. The
in-memory buffer has a ceiling and truncates only at a line boundary. Ephemeral
streaming events may be numbered without being persisted when the complete form
replaces them.

`ConversationEventV1` keeps reading the legacy stream-json. There is no
destructive in-place migration. The reader recognizes and ignores the historical
`prometheusV1Mirror` mark in existing imported logs; Prometeu's new logs write
only the canonical V1 event. The app no longer offers an importer.

## Secrets and logs

Prompts, outputs, tool results and credentials may contain secrets. Do not send
transcripts to telemetry and do not print complete tokens/configurations in
logs. Errors may record the path and the cause, but never content or a
credential.

A terminal tab that fails with `err.terminalAgent.history` writes the failing
step to the diagnostic log as `terminal.history.<step>`, and every forced stop of
its agent process writes `terminal.abort.<reason>`. A `terminal.launch` line maps
the generation that prefixes later lines to its tab.

## Sharing scope

`Workspace.share_team` is optional and binds the consent to the organization and
membership (`organization:<id>:<member>`), or to the legacy team (`team:<id>`).
Its absence authorizes only the legacy path. `team.json` accepts the `cloud`
field with identity, origin and organization; tickets are not written. Before
replacing a legacy configuration, `team.rs` writes a private
`team-legacy-<uuid>.json`. See [migration and rollback](cloud-organizations.md).

`Workspace.remote_control`, false by default, records consent for the owner's
companion devices. It is independent of `audience`: an empty list represents
remote control without a team audience. Turning off the last kind of access also
clears `shared`, `share_team`, `audience` and `rights`.

`Workspace.rights` is `{ send, control }`, two lists of person IDs: who may send
messages (prompts and interrupts) and who may control (answer the agent's
requests) beyond viewing and commenting. Both lists only name people who can view;
the audience `null` never grants them by itself. Changing a right drops the people
no longer in the organization's directory, so each list stays within the
`MEMBERS_MAX` people that peers read from the announcement. Every share saved from
[ADR 0090](../decisions/0090-approved-remote-input.md) on writes the field, even
empty. Shares saved before it load with `rights` null and let people only view
and comment. The owner sees that change once on each Mac, which records the
dismissal in `localStorage` under `prometeu:share-rights-notice`. Tests:
`share_tests` in `crates/core/src/workspace_lifecycle.rs` and
`fixtures/backend-contract.json`.

`Workspace.confirm_messages`, true when absent, records whether teammates'
messages wait in the owner's composer until the owner sends, edits or discards
them ([ADR 0090](../decisions/0090-approved-remote-input.md)). Only
`set_confirm_messages` changes it; stopping sharing keeps the choice. The board
keeps only this choice: waiting messages live in the frontend's memory and are
never persisted. Tests: `share_tests` and `fixtures/backend-contract.json`,
which carries a workspace saved with the option off and one from before it.

## Remote machines

`Workspace.host` and `Project.host` name the machine from `machines.json` that
holds the clone and runs the workspace ([ADR 0101](../decisions/0101-remote-machines.md)).
Their paths are paths on that machine, and remote project IDs are
`remote:<machine>:<path>`. Both fields are written only when set, so boards
without remote entries load and save unchanged, and an older version ignores
them. A remote workspace has no reserved `port`. Its tabs always store terminal
mode. Removing a machine drops its projects from the board and is refused while a
workspace still names it.

`Workspace.creator` names the agent that created a remote workspace through its
machine's host ([ADR 0103](../decisions/0103-agents-on-remote-machines.md)), as the
label its MCP configuration gives, which may be empty. It is written only when set:
absent means the person created the workspace, and an older version ignores it.

`Project.origin` is the clone's `origin` remote normalized to `host/owner/repo`
in lowercase, so every URL spelling of one repository compares equal. The app
fills it in for local projects at start and when one is added, and for a
machine's projects each time the machine attaches. A remote project whose origin
equals a local project's is shown as that project: the sidebar lists its
workspaces under the local one, and the launcher offers only the local one. It is
written only when known. Tests: `remote_hosts_round_trip_and_local_entries_omit_them`
in `crates/core/src/board.rs`, `every_spelling_of_one_remote_normalizes_alike` in
`src-tauri/src/remote/workspace.rs` and `src/machines.test.ts`.

## Terminal sessions

`Board.terminals` lists the terminal sessions in sidebar order
([ADR 0096](../decisions/0096-terminal-sessions.md)) as `{ id, folder, name? }`:
`id` is `terminal-<uuid>`, `folder` is the absolute folder the shell starts in and
`name` is the name the person set. The field is written only when a session
exists, and `name` only when set, so a board from v0.29.0 or earlier loads and
saves back unchanged; an older version ignores the field and drops it on its next
save. Only the record persists: scrollback stays in memory and no process
survives the app, so after a restart a session is listed stopped, and opening it
starts a new shell in the same folder. Test: `boards_from_v0_29_0_load_unchanged` in
`crates/core/src/terminal_sessions/tests.rs`, against the board v0.29.0 wrote in
its contract fixture.

## Delegations

`Board.delegations` defaults to `[]` for older boards. It persists owner client ID,
worker conversation and workspace identities separately, initial task and
repository commits, idempotency keys/hashes, executions and observations.
Process restarts preserve ownership and queued IDs with persisted pending input,
mark other unfinished executions stopped and
clear live background/request observations. No transcript migration applies.
See the [embedded MCP contract](embedded-mcp.md) for fields and retry semantics.

## Local telemetry

`<root>/telemetry.sqlite3` stores canonical metadata independently of boards and
transcripts, with a private `telemetry-health.json` failure record. Database and
event versions are separate; unsupported future database versions are preserved.
Short committed transactions, UUID/occurrence deduplication, indefinite retention,
export and explicit erasure follow the [telemetry contract](telemetry.md).

`Board.telemetry_ids` is an additive, default-empty map of domain keys to opaque
UUIDs. It bridges legacy path-based project IDs and assigns repository/branch
identities without placing names or paths in the event store. Existing UUID
workspace/tab IDs are reused. These aliases remain domain state after telemetry
is erased. Old boards/transcripts require no event backfill; compatibility is
covered by existing board tests and `telemetry/tests.rs`.

Turn completions may include a nullable SHA-256 `messageKey` derived from the
opaque conversation identity and final assistant ID. It supports exact local
lookup for native Claude transcript replay without retaining reply text or the
raw provider identifier. Older facts omit it and remain readable. Additive
origin, app-call and PR-observation facts use the same SQLite store and erasure
boundary; no second usage ledger or provider transcript backfill is introduced.

`Tab.context_window` is optional and defaults to unknown in old boards. It
accompanies current `context_tokens`; the historical `tokens` growth estimate
is not repurposed as consumption or current occupancy. New canonical transcript
completions can retain optional usage independently of the local history store.
Clearing telemetry removes local queries and restored footer caches, not the
conversation transcript or existing exports.

## Pending session input

`SessionService` keeps the existing `pending_prompt` field and its append order
across setup, account changes and process loss; chat-mode failed sends put the
original prompt before later text. Terminal mode adds optional
`Tab.native_dispatch: { id, text }`, absent on old boards and defaulting to null.
One queued item moves into this immutable slot and the board is durably saved
before the process write. Later prompts remain in `pending_prompt` and cannot
change that item's ID or text. Both texts remain visible, in that order, while
acceptance is uncertain; the ID is internal queue identity.

The native host returns `Sent` only after its canonical receipt is durable, or
`AlreadyAccepted` for that exact ID/content after recovery. The latter performs
no process write, capture, telemetry or reactions. Only then does the core
clear the slot through another board persistence barrier. A crash before this
clear is safe because the same token resolves to its old receipt. A crash after
write but before the receipt requires prepared-input reconciliation above and
never retries automatically. A failed board barrier preserves the slot and
prevents sending. Ordinary Chat retains its existing queue policy. See
[sessions](application-core.md#sessions-launch-and-workers).

### Cursor sessions

The additive provider ID `cursor` is explicit in boards, saved model choices and
accounts. `Tab.agent_session` holds the opaque ACP session ID; Prometeu stores
canonical V1 presentation in its existing chat log. Native Cursor history stays
owned by the CLI. Loading it never duplicates presentation history, and a failed
load does not replace the session. The external account is attached and selected
explicitly; removal survives restart. Account readers without Cursor support
preserve its account and active selection opaquely. Older board readers may map
an unknown provider to Claude, so downgrading a Cursor board is unsupported.
See the [Cursor contract](cursor.md).

## Delegation navigation compatibility

Sub-workspace grouping derives from `Board.delegations` and the owning tab IDs;
no parent field or migration is added. Missing delegation lists keep legacy flat
navigation. Existing multi-tab delegated workspaces preserve all tabs on load,
but new conversations are rejected. See [ADR 0099](../decisions/0099-sub-workspaces.md)
and the [embedded MCP contract](embedded-mcp.md#sub-workspace-navigation).
