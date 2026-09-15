# Plugin hub contract

Status: contract in force; portable adaptation decided by ADR 0005.

This contract defines how a single Prometeu catalog feeds Claude Code and Codex
sessions. The hub is product state; each CLI's manifests, arguments and caches
are adapter details.

## Hub

The registry lives in `<root>/plugins.json` and keeps the existing shape:

```ts
type Plugin = {
  id: string;
  source: string;
  note: string;
  made: boolean;
  from: string;
};
```

`id` is the stable identity stored in the workspace. `source` can be a local
folder, a local `.zip` or the URL of a `.zip`; each adapter decides which of
those sources it can materialize. `made` only says whether the folder was
created or cloned by Prometeu and therefore whether the app may delete it.

When installing a marketplace repository, the importer looks, in this order,
for:

- a compatible plugin at the root;
- `.agents/plugins/marketplace.json`;
- `.claude-plugin/marketplace.json`;
- compatible plugins one level below the root or inside `plugins/`.

A local entry of the Codex marketplace uses
`{"source":{"source":"local","path":"./plugins/x"}}`; the Claude form uses
`{"source":"./plugins/x"}`. Only paths internal to the clone are followed.
Remote entries are another installation and do not make the importer cross the
repository boundary.

## Layered selection

Three independent axes — `mcp`, `plugins` and `skills` — are selected in three
layers, resolved in the order global → project → workspace
([ADR 0043](../decisions/0043-layered-tool-selection.md)). Per axis and layer
the value is either absent, meaning inherit, or
`{ base, add, remove }`, where `base` is `"none"` — the inherited set is
replaced by `add` — or `"inherit"` — `add` and `remove` are applied over it.
`selection.rs` resolves the chain and stays free of Tauri, DOM and network;
where each layer is persisted is in
[`persistence.md`](persistence.md).

The resolved list applies to whichever provider is chosen for that workspace:

- a terminal inheritance, when no layer declares the axis, injects nothing from
  the hub;
- a list injects only items still present in the axis universe;
- an ID removed from the universe is ignored so that an old layer still
  resolves.

For the `mcp` axis of a Claude workspace the universe is the hub plus the
**CLI-inherited base** ([ADR 0044](../decisions/0044-cli-inherited-mcp-base.md)):
the servers discovered read-only from `~/.claude.json` (user scope and the
project entry for the working directory) and from the working directory's
`.mcp.json`. A hub server shadows a discovered ID; the first discovered
occurrence wins. The base participates in resolution as an implicit
`{ base: "inherit", add: <base> }` layer below global
(`selection.rs::resolve_with_base`), so `base: "none"` at any layer also
replaces it, and a removal of a base ID is an ordinary workspace-layer
`remove`. Base items that stay on carry the `cli` provenance. Codex has no
discovered base yet; see
[`provider-matrix.md`](../quality/provider-matrix.md).

Skills leave the plugin axis and become their own, and keep using the same
plugin-package pipeline: `skills-packages/<id>`, `--plugin-dir` for Claude and
the derived marketplace for Codex. The split is a state and interface concern,
not a new materialization mechanism.

The resolved set is captured at spawn. A change at any layer takes effect at the
next spawn or resume; a running session keeps the set it was born with, so the
interface states that a change applies on the next session instead of restarting
the process.

The selection controls what Prometeu injects. Plugins the person enabled
directly in the CLI's global registry are still subject to that CLI's rules. In
Codex, the real configuration is re-read while preparing each spawn so those
preferences keep following the user; only the entries of the reserved `prometeu`
and `prometeu-dev` marketplaces are controlled by the workspace. For MCP in
Claude, when any layer declares the axis the spawn passes
`--strict-mcp-config` with a private file that materializes the **whole
effective set**, including the kept CLI-inherited servers, so the resolved list
is exactly what the CLI loads; when no layer declares it, no strict flag is
passed and the CLI loads its own defaults — the same set the picker shows
(ADR 0044 amends the Authority section of ADR 0043).

Selecting is activating. The package must be enabled from the start of the
session and, when it declares hooks, they must be active before the first
answer. A continuous mode may use `SessionStart` to put its instruction in the
context and `UserPromptSubmit` to reinforce it; it does not depend on a command,
a mention of the skill or a second activation.

## Portable package

A folder that must work in both providers contains
`.claude-plugin/plugin.json`. It may also carry `.codex-plugin/plugin.json`;
when it exists, the native manifest works as an overlay for the equivalent
fields. `name` must be the same in both and use lowercase letters and hyphens,
with at most 64 characters.

Skills, commands, scripts, assets, MCP and hooks stay inside the same folder.
Manifest paths are relative to the plugin's root. When a Claude package brings
`.mcp.json`, the adapter adds `mcpServers` to the derived Codex manifest;
commands are migrated by Codex itself to its skill mechanism. Codex also
provides `CLAUDE_PLUGIN_ROOT` to compatible hooks, so the package does not need
to duplicate scripts just to switch providers.

Both manifests accept a path to a hooks file, but their inline objects do not
have exactly the same envelope: in the Claude manifest the object is the event
map; in Codex it is a complete hooks file, with that map under `hooks`. When
creating the derived overlay, the adapter adds that envelope without touching
the source. A `.codex-plugin/plugin.json` overlay supplied by the package itself
is already native and does not get that conversion.

`agents/*.md` is not part of the current Codex plugin format. It may coexist in
the package and keeps working in Claude, but a flow that needs both providers
must be modeled as a skill. That difference is not hidden by a silent conversion
to Codex's subagent configuration.

Prometeu's creator must generate both manifests. Old packages with only the
Claude manifest stay portable: the adapter writes the minimal Codex manifest
only in the derived copy, without changing the source.

## Adapters

### Claude Code

Each chosen item becomes `--plugin-dir` for a folder or a local `.zip` and
`--plugin-url` for a URL. The original source is handed directly to the CLI.

### Codex

Codex receives only local folders. `.zip` and URL are still supported by Claude,
but an attempt to use them in a Codex session fails before starting and explains
which plugin must be installed as a folder.

For every explicit selection, including `[]`, `plugins.rs`:

1. derives a stable home in `<root>/codex-workspaces/<workspace-hash>/` from the
   SHA-256 of the workspace's persisted ID; using the ID keeps even two
   workspaces running in the same clone without a worktree separate. Action
   tasks use the session ID as the scope, preserving their tools. For a managed
   account, it uses the `<workspace-hash>/<account>/` subdirectory, preserving
   the links of processes still working with another account;
2. mirrors in that home the entries of the account profile captured at spawn,
   except configuration files. Each account keeps its credential; sessions,
   skills and the plugin cache stay shared with the original installation;
3. computes a deterministic SHA-256 of each plugin folder, without `.git`, and
   combines a revision of the derived format so that adapter fixes also
   invalidate old snapshots;
4. copies the package to `<home>/marketplace/plugins/<id>`, merges the
   compatible manifest with the native overlay and adds the hash to the version
   as a cachebuster;
5. writes `.agents/plugins/marketplace.json` in that workspace snapshot;
6. rebuilds `<home>/config.toml` from the real configuration, preserves the
   workspace's hook state, turns off old Prometeu entries and turns on only the
   current IDs;
7. queries and installs through the stable `codex plugin` CLI, always with
   `CODEX_HOME=<home>`;
8. starts `codex app-server` with the same `CODEX_HOME`.

The derived marketplace is named `prometeu` in release and `prometeu-dev` in
debug, avoiding a collision between the two states. Each workspace has its own
snapshot, and preparation and installation are serialized because the installed
cache stays shared.

Installing or updating may write in Codex's global cache and in the derived
`config.toml`. The real home's `config.toml` is not opened for writing. Removing
an item from the hub attempts to remove the shared installation, the entry in
each derived config and the marketplace copies.

Per-home isolation is necessary because the `plugins.<id>.enabled` state does
not currently accept a reliable override through `-c`: the CLI consumes the
argument, but the plugin loader keeps using the persisted layer. That is why
activation lives in a real, yet disposable, configuration layer instead of
depending on a flag that does not produce the promised effect.

Codex's default login storage is `auth.json`. The derived home links that file
to the account's profile and pins the `file` mode when the configuration uses
the default or `auto`, so that refreshes do not create divergent tokens. An
explicit choice of `keyring` or `ephemeral` is preserved; since Codex itself
treats each `CODEX_HOME` as an independent identity in those modes, it may
require an API key in the environment or authentication specific to that home.
Profiles managed by Prometeu use `file` from login on, in a private per-account
file. The decision is in [ADR 0012](../decisions/0012-provider-accounts.md).

## Hooks and trust

Choosing a plugin authorizes that package's code and hooks, just as passing
`--plugin-dir` already authorizes in Claude. Before opening the Codex thread,
the adapter queries `hooks/list` and writes `enabled = true` and the trust
through `currentHash` in the derived config, only for hooks whose `pluginId`
belongs to the current selection. A hook that is already trusted but disabled is
also re-enabled: the workspace's selection prevails over the previous derived
state.

Hooks of the CLI's own global configuration, of the project's files and of
plugins outside the resolved selection never gain trust through that flow. If
the content changes, the new hash is accepted only when a session that still
selects the plugin opens. If a package declares hooks and `hooks/list` does not
assign them to its `pluginId`, or if writing the activation/trust fails, the
adapter shows an error and does not open the thread. That way the session cannot
be born as a collection of skills when the person chose an automatic behavior.

### Project declarations

The project layer comes from a versioned `.prometeu/settings.toml`, so it cannot
activate on its own. The first time a repository's `[tools]` declares items —
and again whenever the declared set changes, compared by the SHA-256 of that
section — Prometeu asks before activating them, and the decision is stored
app-local on the board, never in the repository. The prompt offers approval or
rejection, and both bind to the hash the backend recomputes at call time. Until
a decision exists, project-declared items are resolved and shown as pending; a
rejection keeps them resolved and shown as rejected. Either way they are not
injected, so their hooks are neither enabled nor trusted, and a rejection also
quiets the prompt until the declaration changes and re-pends. Global and
workspace choices stay the person's explicit action and need no extra approval.

## Failures and compatibility

- a failure to prepare or install a chosen plugin prevents the session's spawn;
- a failure to prepare a chosen MCP also prevents the spawn, instead of silently
  starting a conversation without tools;
- the home, the marketplace and the copy in `<root>/codex-workspaces/` are
  derived and can be rebuilt from the hub, the real config and the sources;
- removing or cleaning a workspace deletes only its derived home; the links do
  not turn the account, sessions or shared cache into the app's ownership;
- a change in the hub's schema, in the selection semantics or in the generated
  manifests requires updating this contract and a compatibility test.

## Standalone skills

Skills registered on the desktop or installed from the account are materialized
in `<root>/skills-packages/<id>/`, with both manifests and `skills/<id>/SKILL.md`.
The hub keeps them under the ID `skill-<id>`, and they are chosen on their own
`skills` axis, which still materializes through the plugin-package pipeline
above, without changing the CLIs' global configuration. See the
[catalog contract](cloud-catalog.md).
