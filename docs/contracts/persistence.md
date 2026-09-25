# Local persistence

Status: current contract.

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
| board | `<root>/board.json` | `state.rs` |
| external MCP clients | `<root>/mcp-clients/<uuid>.json` | `mcp_access.rs`; private versioned credentials and project scope; see [embedded MCP](embedded-mcp.md) |
| MCP socket discovery | `<root>/mcp-socket` | `embedded_mcp.rs`; private ephemeral socket path, replaced on startup |
| board backup | next to `board.json` | `state.rs` |
| team and credential | `<root>/team.json` | `team.rs` |
| E2EE identities, TOFU links and replay | `<root>/team-security.json` | `team.rs` (file), `team-security.ts` (internal schema) |
| optional Prometeu account | `<root>/cloud.json` | `cloud.rs`; see the [contract](cloud-account.md) |
| cloud catalog cache and links | `<root>/catalog.json` (`catalog.local.json` is a legacy backup) | `catalog.rs`; see the [contract](cloud-catalog.md) |
| installed skills and packages | `<root>/skills.json`, `<root>/skills-packages/<id>/` | `skills.rs`; see the [catalog](cloud-catalog.md) |
| accounts and per-provider selection | `<root>/accounts.json` | `accounts.rs` |
| additional authenticated profiles | `<root>/accounts/<uuid>/` | provider adapters |
| last quota snapshot per account | `<root>/usage.json` | `usage.rs` |
| optional TypeSafe key and enablement | `<root>/typesafe.json` | `typesafe.rs`; private, disabled by default; see [context evaluation](context-evaluation.md) |
| Codex and Antigravity V1 transcript | `<root>/chats/<tab>.jsonl` | `chat.rs` |
| files received through a native promise | `<root>/attachments/<uuid>/<name>` | `file_drop.rs`; private `0700` directory, `0600` file |
| image pasted from the clipboard | `<root>/attachments/<uuid>/pasted.png` | `file_drop.rs`; same folder and permissions, TIFF converted to PNG on macOS, GTK image converted to PNG on Linux |
| plugin hub | `<root>/plugins.json` | `plugins.rs` |
| derived Codex home/marketplace | `<root>/codex-workspaces/<workspace-hash>/[<account>/]` | `plugins.rs`; rebuildable |

Catalog project installation reuses board Project records and stores local paths
in `catalog.json` installation links. No credentials or paths enter the Cloud
document; see [Git projects](cloud-catalog.md#git-projects).

Worktrees live in `~/prometeu/worktrees[-dev]/...`, outside the state root.

Transcript lookup uses the tab's resolved provider, including a frozen task
profile, before falling back to the workspace provider. This is also the rule
for snapshots after the process stops; mixed-provider tabs retain their own
transcript paths without a format migration.

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

Identity creation, a member link — first or replaced —, a revision and a receipt
are written before the corresponding use. Corruption, an unknown version and a
read/write failure block collaboration; they do not silently regenerate keys.
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
`state.rs::tabs_without_a_kickoff_keep_the_previous_format`,
`scripts.rs::method_artifacts_are_inherited_normalized_and_never_invented` and
`src/kickoff.test.ts`. See
[ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md).

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

`Board` contains projects, stages, workspaces and the optional `actions`
catalog. `Tab.task` stores the resolved configuration and the tasks' cursors.
The absence of those fields keeps previous sessions working. The catalog
receives Code review exactly once, recorded in `actions.defaults_initialized`;
see [actions](actions.md). `Workspace` contains repositories, branch, worktree,
agent configuration, the MCP/plugins/skills selection, sharing and tabs.
`Tab.tokens` stores an incremental estimate of the tokens used in the
conversation; `Tab.context_tokens` stores the last observed context so only the
growth is added. When the context drops after compaction, the new value starts
another segment and adds to the total. Old boards without `context_tokens` treat
`tokens` as the total and the initial cursor, without duplicating the value. The
tab also contains identity, status, pending message, model override and the
external identity used for resume when needed. An empty `Tab.title` is an
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

## Ordered board publication

`Saver::publish` serializes snapshot creation, enqueueing, and the `board`
event under one publication mutex. It briefly locks the current board to clone
it, then releases the board lock before enqueueing and emission. Competing
publishers cannot enqueue or emit an older captured snapshot after a newer one.
The worker still coalesces writes and performs disk I/O outside the board lock.

`save_now` uses the same publication mutex and waits for a flush acknowledgment.
Flushing does not terminate the worker: action transitions also flush while
the app remains running. Later changes must still be persisted. Regression
coverage lives in `state.rs` (`concurrent_publications_keep_snapshot_and_emission_order`
and `flush_keeps_saver_available_for_runtime_publications`).

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
| global | `Board.tools` in `<root>/board.json` | `state.rs` |
| project | the `[tools]` table of `.prometeu/settings.toml`, in the repository | `scripts.rs` |
| workspace | `Workspace.mcp`, `Workspace.plugins` and `Workspace.skills` | `state.rs` |

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
and falls back to the board backup, then to a default board. The resolved set is
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

### Claude

Claude Code writes in `~/.claude/projects/<cwd-slug>/<tab>.jsonl`, or in the
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
removed. Compatibility tests live in `state.rs` and `accounts.rs`.

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
clears `shared`, `share_team` and `audience`.

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
