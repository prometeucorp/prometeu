---
type: Contract
title: "Plugin hub contract"
description: "The hub, the portable package and the adaptation for each provider."
status: stable
---

# Plugin hub contract

This contract is in force. ADR 0005 records the decision about the portable
adaptation.

This contract tells how one Prometeu catalog supplies plugins to Claude Code and
Codex sessions. The hub is product state. The manifests, arguments and caches of
each CLI belong to the adapters.

The Tauri-free `prometeu-tools` crate materializes packages, adds them to the registry,
imports Git repositories and makes skills. It does this work behind ports that
the desktop composes and supplies. The captured profile, the
configuration scope, the canonical IDs and the hook IDs keep their meaning. See
[tools and packages](application-core.md#tools-and-packages).

## Hub

The registry is in `<root>/plugins.json`. It keeps the current shape:

```ts
type Plugin = {
  id: string;
  source: string;
  note: string;
  made: boolean;
  from: string;
};
```

`id` is the stable identity that the workspace stores. `source` can be a local
folder, a local `.zip` or the URL of a `.zip`. Each adapter selects which of
these sources it can materialize. `made` only tells if Prometeu made or cloned
the folder. Thus it tells if the app can delete the folder.

Git clones of GitHub URLs go into `<root>/plugins/<owner>-<repo>`. Thus two
repositories with the same basename (for example `caldoai/skills` and
`typesafe-ai/skills`) do not go into the same folder. Other Git hosts continue
to use the basename of the repository.

When the importer installs a marketplace repository, it looks for these items in
this sequence:

- A compatible plugin at the root.
- Entries of `.agents/plugins/marketplace.json` and
  `.claude-plugin/marketplace.json` with a local `source` that has
  `.claude-plugin/plugin.json`.
- The same marketplace entries when they have a `skills` array of directories
  with `SKILL.md` and no plugin manifest (the Anthropic skill-bundle shape, or
  the `strict: false` shape). Prometeu makes a portable package in
  `<clone>/.prometeu/packages/<id>/`. This package has the Claude manifest, the
  Codex manifest and a copy of each skill tree
  ([ADR 0098](../decisions/0098-skill-bundle-install.md)).
- Compatible plugins one level below the root or in `plugins/`.
- If the importer finds nothing before, each `skills/*/SKILL.md` in the clone.
  Prometeu also makes a package in `.prometeu/packages/<id>/` for each of these
  skills.

A local entry of the Codex marketplace uses
`{"source":{"source":"local","path":"./plugins/x"}}`. The Claude form uses
`{"source":"./plugins/x"}`. The importer follows only paths in the clone. A
remote entry is a different installation. It does not cause the importer to go
out of the repository. A plugin update does this procedure again on the root of
the clone. Thus the made packages change after `git pull`.

## Layered selection

Three axes — `mcp`, `plugins` and `skills` — each have their own selection in
three layers. The resolution sequence is global → project → workspace
([ADR 0045](../decisions/0045-layered-tool-selection.md)). For each axis and
layer, the value is missing or `{ base, add, remove }`. If the value is
missing, the layer inherits. The `base` value is one of these:

- `"none"`: `add` replaces the inherited set.
- `"inherit"`: the resolution applies `add` and `remove` to the inherited set.

`selection.rs` resolves the chain. It does not use Tauri, the DOM or the
network. [`persistence.md`](persistence.md) tells where Prometeu stores each layer.

The resolved list applies to the provider of the tab or of the action
profile that applies. This includes overrides of the workspace default (ADR 0047):

- If no layer declares the axis, the inheritance stops and the resolution adds
  nothing from the hub.
- A list adds only the items that continue to be in the universe of the axis.
- The resolution ignores an ID that is not in the universe. Thus a stored layer
  from before continues to resolve.

For the `mcp` axis of a Claude conversation, the universe is the hub plus the
**CLI-inherited base** ([ADR 0046](../decisions/0046-cli-inherited-mcp-base.md)).
This base contains:

- The servers that the Claude adapter finds, read-only, in its configured home
  (`CLAUDE_CONFIG_DIR`, with `~/.claude.json` as the default).
- The servers in the `.mcp.json` of the working directory and of its parent
  directories.
- The connectors of the active Claude account
  ([ADR 0063](../decisions/0063-account-connectors-in-the-mcp-base.md)). The
  adapter reads them from the account API with the credential of the login. Their
  name is `claude.ai <display name>`.

Local definitions come first. Then come project definitions (the nearest
first), user definitions and the account. A hub server continues to hide a
found server with the same ID. See
[ADR 0047](../decisions/0047-tool-selection-boundaries.md). In the resolution,
the base is an implicit `{ base: "inherit", add: <base> }` layer below the global
layer (`selection.rs::resolve_with_base`). Thus `base: "none"` in a layer also
replaces the base. The removal of a base ID is a usual `remove` in the workspace
layer. Base items that stay selected have the `cli` provenance. Codex does not
have a found base at this time. See
[`provider-matrix.md`](../quality/provider-matrix.md).

Skills are not on the plugin axis. They have their own axis. They continue to use
the same pipeline as plugin packages: `skills-packages/<id>`, `--plugin-dir` for
Claude and the derived marketplace for Codex. This change is about state and the
interface. It is not a new materialization method.

Prometeu captures the resolved set at spawn. A change in a layer has an effect at
the next spawn or resume. A session that operates keeps the set that it started
with. Thus the interface tells that a change applies to the next session. It does
not start the process again.

The selection controls what Prometeu adds. Plugins that the person enabled
directly in the global registry of the CLI continue to obey the rules of that
CLI. For Codex, Prometeu reads the real configuration again before each spawn.
Thus these preferences continue to follow the user. The workspace controls only
the entries of the reserved `prometeu` and `prometeu-dev` marketplaces.

For MCP in Claude, the spawn obeys these rules:

- If a layer declares the axis, the spawn gives `--strict-mcp-config` with a
  private file. This file materializes the **full set that applies**, which
  includes the kept CLI-inherited servers. Thus the CLI loads only the resolved
  list.
- If no layer declares the axis, the spawn does not give a strict flag. The CLI
  loads its own defaults, which are the same set that the picker shows (ADR 0046
  changes the Authority section of ADR 0045).

Prometeu materializes a kept account connector as
`{ "type": "claudeai-proxy", "url": …, "id": … }`. The CLI makes this same entry
for itself. The strict flag stops the CLI before it gets the account list.
Without this entry, the CLI ignores the connector and does not tell the person.

These rules apply to the account list:

- If the account list is unknown, a declared selection that continues to inherit
  the account base stops with an error before spawn.
- A `base: "none"` layer replaces that base and can start with its selected
  servers.
- A `claude.ai` connector that a layer adds explicitly keeps its ID through the
  resolution. If its configuration is missing, it stops with an error before
  spawn.
- If Prometeu cannot read or parse the credential, it uses the macOS Keychain
  when the Keychain has a correct token. If not, the list stays unknown.
- The account list is empty only when Prometeu is sure that the credential is
  missing.

To select a package is to activate it. Prometeu must enable the package from the
start of the session. If it declares hooks, the hooks must be active before the first
answer. A continuous mode can use `SessionStart` to put its instruction in the
context and `UserPromptSubmit` to make it stronger. It operates without a
command, a skill name in the prompt or a second activation.

## Portable package

A folder that must operate in the two providers contains
`.claude-plugin/plugin.json`. It can also contain `.codex-plugin/plugin.json`.
When this file is there, the native manifest is an overlay for the equivalent
fields. `name` must be the same in the two manifests. It must use lowercase
letters and hyphens, with a maximum of 64 characters.

Skills, commands, scripts, assets, MCP and hooks stay in the same folder. The
paths in a manifest are relative to the root of the plugin. When a Claude package
has `.mcp.json`, the adapter adds `mcpServers` to the derived Codex manifest.
Codex itself changes commands into its skill mechanism. Codex also gives
`CLAUDE_PLUGIN_ROOT` to compatible hooks. Thus a package can use the same scripts
with the two providers.

The two manifests accept a path to a hooks file. But their inline objects do not
have the same envelope:

- In the Claude manifest, the object is the event map.
- In Codex, the object is a complete hooks file, with that map in `hooks`.

When the adapter makes the derived overlay, it adds that envelope and does not
change the source. A `.codex-plugin/plugin.json` overlay that the package
supplies is native. The adapter does not change it.

`agents/*.md` is not part of the current Codex plugin format. These files can be
in the package and they continue to operate in Claude. But a flow that must
operate in the two providers must be a skill. Prometeu does not hide this
difference with a silent change to the subagent configuration of Codex.

The Prometeu creator must make the two manifests. Packages from before with only
the Claude manifest stay portable. The adapter writes the minimum Codex manifest only
in the derived copy and does not change the source.

## Adapters

### Claude Code

Each selected item becomes `--plugin-dir` for a folder or a local `.zip`. It
becomes `--plugin-url` for a URL. The adapter gives the source directly to the
CLI.

### Codex

Codex receives only local folders. Claude continues to accept `.zip` and URL
sources. But if a Codex session tries to use them, the session stops with an
error before it starts. The error tells which plugin you must install as a folder.

For each explicit selection, which includes `[]`, `NativePackages` in the tools
crate does these steps:

1. It gets a stable home in `<root>/codex-workspaces/<workspace-hash>/` from the
   SHA-256 of the stored workspace ID. Thus two workspaces in the same clone
   without a worktree get different homes.
2. For action tasks, it uses the session ID as the scope. This keeps their tools.
3. For a managed account, it uses the `<workspace-hash>/<account>/`
   subdirectory. This keeps the links of processes that continue to operate with
   a different account.
4. It copies into that home the entries of the account profile that the spawn
   captured. It does not copy configuration files or the native
   `app-server-control` runtime directory. It removes derived links from before
   for that directory and does not follow them.
5. Codex makes its private local directory, and the current physical endpoints
   do not change. Each account keeps its credential. Sessions, skills and the
   plugin cache stay shared with the Codex installation of the user.
6. It calculates a deterministic SHA-256 of each plugin folder, without `.git`.
   It adds a revision of the derived format. Thus an adapter repair also makes
   previous snapshots incorrect.
7. It copies the package to `<home>/marketplace/plugins/<id>`. It merges the
   compatible manifest with the native overlay. It adds the hash to the version
   as a cachebuster.
8. It writes `.agents/plugins/marketplace.json` in that workspace snapshot.
9. It makes `<home>/config.toml` again from the user configuration. It keeps the
   hook state of the workspace, disables previous Prometeu entries and enables only
   the current IDs.
10. It sends queries and installs through the stable `codex plugin` CLI, always
    with `CODEX_HOME=<home>`.
11. It starts `codex app-server` with the same `CODEX_HOME`.

The name of the derived marketplace is `prometeu` in a release and `prometeu-dev`
in a debug build. Thus the two states do not use the same name. Each workspace
has its own snapshot. The preparation and the installation occur one at a time
because the installed cache stays shared.

An installation or an update can write in the global cache of Codex and in the
derived `config.toml`. Prometeu does not open the `config.toml` of the user home
to write it. When a person removes an item from the hub, Prometeu tries to remove
these items:

- The shared installation.
- The entry in each derived configuration.
- The marketplace copies.

Each workspace must have its own home because the `plugins.<id>.enabled` state
does not accept a reliable override through `-c` at this time. The CLI reads the
argument, but the plugin loader continues to use the stored layer. Thus the
activation is in a configuration layer that Codex reads and that Prometeu can
discard. Prometeu does not use a flag that does not give the promised effect.

The default login storage of Codex is `auth.json`. The derived home links that
file to the profile of the account. If the configuration uses the default or
`auto`, the derived home sets the `file` mode. Thus a token refresh does not make
different tokens. Prometeu keeps an explicit `keyring` or `ephemeral` selection.
In these modes, Codex sees each `CODEX_HOME` as a different identity. Thus an API
key in the environment or an authentication of that home can be necessary.
Profiles that Prometeu manages use `file` from the login, in a private file for
each account. [ADR 0012](../decisions/0012-provider-accounts.md) records the
decision.

## Hooks and trust

When a person selects a plugin, this gives authorization to the code and the
hooks of that package. In Claude, `--plugin-dir` gives the same authorization.
Before the adapter opens the Codex thread, it sends a `hooks/list` query. Then it
writes `enabled = true` and the trust through `currentHash` in the derived
configuration. It does this only for hooks with a `pluginId` in the current
selection. The adapter also enables a hook that has trust but that the derived
state disables. The selection of the workspace replaces the previous derived
state.

These hooks do not get trust through this flow at any time:

- Hooks of the global configuration of the CLI.
- Hooks of the project files.
- Hooks of plugins that are not in the resolved selection.

If the content changes, Prometeu accepts the new hash only when a session that
continues to select the plugin opens. The adapter shows an error and does not
open the thread in these conditions:

- A package declares hooks and `hooks/list` does not connect them to its
  `pluginId`.
- Prometeu cannot write the activation or the trust.

Thus the session does not start as only a set of skills when the person selected
an automatic behavior.

### Project declarations

The dialog sends the hash that it shows. The backend calculates the declaration
again. It rejects changed or removed declarations before it records a decision.
An approval and a rejection must agree with the hash (ADR 0047). The prompt
follows the pending state of the declaration. This rule also applies if the
declaration only removes items or replaces an axis with an empty set. The menus
of the project and of the workspace keep the declarations available after a
decision.

The project layer comes from a versioned `.prometeu/settings.toml`. Thus it
cannot activate itself. Prometeu shows a prompt before it activates the items in
these conditions:

- The first time that the `[tools]` section of a repository declares items.
- Each time that the declared set changes. Prometeu compares the SHA-256 of that
  section.

Prometeu stores the decision in the local data of the app, on the board. It does
not store the decision in the repository at any time. The prompt gives approval
or rejection. The two options use the hash that the backend calculates at the
time of the call.

Until there is a decision, Prometeu resolves the items that the project declares
and shows them as pending. After a rejection, it continues to resolve them and
shows them as rejected. In the two conditions, Prometeu does not add these items.
Thus their hooks are not enabled and do not have trust. A rejection also stops
the prompt until the declaration changes and becomes pending again. Global and
workspace selections stay the explicit action of the person. No more approval is
necessary for them.

## Failures and compatibility

- If Prometeu cannot prepare or install a selected plugin, the session does not
  spawn.
- If Prometeu cannot prepare a selected MCP, the session also does not spawn. It
  does not start a conversation without tools silently.
- Prometeu makes the home, the marketplace and the copy in
  `<root>/codex-workspaces/` from the hub, the user configuration and the
  sources. It can make them again at any time.
- When a person removes or cleans a workspace, Prometeu deletes only its derived
  home. The links do not make the account, the sessions or the shared cache the
  property of the app.
- A change to the schema of the hub, to the meaning of the selection or to the
  made manifests is a contract change. It must update this contract and add a
  compatibility test.

## Standalone skills

Prometeu materializes the skills that the desktop adds to the hub or that come
from the account in `<root>/skills-packages/<id>/`. Each package has the two manifests
and `skills/<id>/SKILL.md`. The hub keeps them with the ID `skill-<id>`. A
person selects them on their own `skills` axis. This axis continues to
materialize through the pipeline of plugin packages above. It does not change
the global configuration of the CLIs. See the
[catalog contract](cloud-catalog.md).

## Skills inside packages

`plugin_skills` lists the skills that installed plugins contain. The *Start with*
catalog of the launcher uses this list
([ADR 0057](../decisions/0057-skill-kickoff-and-artifact-path.md)). The backend
examines each hub entry with a local folder as its source. It does not examine a
`.zip`, a URL or a standalone `skill-<id>` package, because the skill hub lists
that package before.

For each entry, the backend reads `skills/*/SKILL.md` and returns
`{ plugin, name, description }` from its YAML front matter. These rules apply:

- The backend can read plain, quoted and folded scalars. It folds the values into
  one line.
- If `name` is missing, the backend uses the folder name.
- A file or folder that the backend cannot read adds nothing.

This procedure only reads. It does not select, activate or give trust to a
package at any time. When a person selects a skill, Prometeu adds its package to
the resolved set of one conversation. It does not add the package to a selection
layer at any time.

## Bundled Prometeu MCP

The virtual `prometeu` server is in the universe that a person can select. It is
not in the implicit CLI base at any time. An installation does not select it
automatically. The app owns its definition. A person cannot edit, remove or share
it. Only selected sessions receive a private process credential. See
[embedded MCP](embedded-mcp.md).

## Shared local resource libraries

`prometeu-tools::skills::SkillLibrary` owns these items:

- The current validation of standalone skills.
- The collision checks of packages.
- The front matter and the manifests.
- The registration sequence.

Hosts supply its root, `PackageFiles` and `SkillPackages`. Its `Skills` port
gives local read, save and remove operations. The desktop keeps these items in
its adapter: the catalog guard, the optional Cloud publication and the current
cleanup of the Codex cache. It materializes selected packages through the shared
startup services.

`PluginLibrary` owns the source inspection, the registration, the Git import, the
update and the removal. If Prometeu finds one package, it adds that package to
the registry. A
marketplace returns candidates for the current selection dialog. A registration
does not activate a package at any time. The import accepts current Git URLs and
the GitHub short form. Local folders and archive sources keep the inspection
contract from before. Codex continues to reject archive and URL packages at
startup. It must have a local folder.

Hosts do changes to libraries one at a time. The desktop keeps the sequence of
its Cloud guard and publication. It also keeps its best-effort cleanup of the
Codex cache. Each Git command uses the bounded runner that the host supplies,
with these limits:

- A time limit of twenty seconds.
- A maximum of 256 KiB for each captured stream.

Thus network or credential operations that stop give an error. Long downloads
can also stop with an error.

Updates use `pull --ff-only` and keep local differences. A removal keeps manual
sources, shared directories and paths that resolve out of the plugin store of
the app. The importer does not add package directories that resolve out of the clone to
the registry. This rule applies to marketplace entries and to the folders that
the importer examines. Prometeu can discard clones that nothing uses, but only in
that store.

If the preparation or a write has a failure, Prometeu returns an error. The current sequence
writes the file system first. Thus there is no rollback between the clone or the
deletion and the writes to the registry.

The current desktop fixtures keep the compatibility of the discovery, the URLs
and the catalog. `tools/src/plugins.rs` has tests for command failures, write failures and
containment.

`McpCatalog` and `PackageCatalog` give the local hub files that are on disk. The desktop
continues to add its own virtual built-in MCP at composition. The current
catalog formats and fallback reads do not change. `tools/src/skills.rs` and
`tools/src/catalog.rs` cover the compatibility and the behavior of private files.

## Local MCP authentication

The desktop uses the discovery, check and OAuth services from `prometeu-tools`.
`Authorization` receives `AuthStorage`. The implementation with a private file
keeps `mcp-auth.json`. The key of each entry is the server ID. Each entry has
these values:

- The client ID.
- The token endpoint.
- The resource.
- The access token and the refresh token.
- The expiry.

If a refresh does not return a new refresh token, Prometeu keeps the previous
one. A
login uses a registration again only for the same endpoint and resource.
Changes to credentials occur one at a time. A write failure returns an error.
Pending PKCE verifiers stay in memory. They expire after five minutes, and
Prometeu uses each one only one time. A logout also cancels the pending
authorization for that server.

The desktop supplies native consent. At startup, the provider gets tokens through
the current `McpSources` interface and the current encoding of secrets. Active processes keep
their captured token. Local checks use the shared HTTP request and response code and a bounded
query launcher that the desktop supplies. Prometeu does not start an agent for an
inspection. The read-only import discovery from the CLI is different from the
inherited selection of each provider. They do not share code.
