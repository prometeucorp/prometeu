---
type: Decision
title: "ADR 0046 — The CLI's MCP set is the visible inherited base"
description: "The MCP set of the CLI is an inherited base that the picker shows."
status: stable
---

# ADR 0046 — The CLI's MCP set is the visible inherited base

Date: 2026-09-15
[ADR 0047](0047-tool-selection-boundaries.md) replaces the discovery section of
this decision. [ADR 0063](0063-account-connectors-in-the-mcp-base.md) adds to
it.

This ADR changes the "Authority" section of
[ADR 0045](0045-layered-tool-selection.md). The other sections of that decision
stay in effect.

## Context

ADR 0045 put the selection of MCP servers, plugins and skills in layers above
the Prometeu hub. Prometeu resolved the layers when it started the agent
process. A test in the application showed a contradiction in the `mcp` axis:

- The hub is empty at the start. It is a registry that Prometeu controls. The
  servers that the person used before are in the configuration of Claude Code.
- When no layer declared the axis, the start command did not give
  `--strict-mcp-config`. Thus Claude Code loaded its own servers. These servers
  came from the user scope of `~/.claude.json`, the project entry for the
  working directory and the `.mcp.json` file of the repository.
- The picker showed only the rows of the hub. The composer did not show the MCP
  button when the hub was empty and there was no layer.

As a result, a session started with an active set that the interface did not
show. The person was not able to change this set. To remove one CLI server, the
person had to edit the CLI files manually or declare a layer. A declared layer
removed *all* the CLI defaults, because the strict configuration had only the
hub selections. ADR 0045 accepted the two sources together, but it told the
interface to show them correctly. The interface did not do this.

## Options considered

1. Import the CLI servers into the hub automatically when Prometeu finds them
   for the first time.
2. Do not show the CLI servers. Write in the documentation that the picker
   controls only the Prometeu part.
3. Use the active set of the CLI as an implicit base layer below the global
   layer. Resolve it with the current chain. Show it in the picker with its own
   provenance. When a layer declares the axis, write the full active set to the
   configuration.

Option 1 copies configuration that Prometeu does not control. It makes a second
identity for the same server, and the two identities become different. It also
writes to the hub without an action from the person. Option 2 keeps the
incorrect interface that caused this ADR. Prometeu selected option 3.

## Decision

For the `mcp` axis of Claude workspaces, the servers from the CLI configuration
are a **visible inherited base**. The set of IDs in the picker is
`hub ∪ inherited`:

- **Discovery.** Prometeu reads these sources for the working directory of each
  workspace, in this sequence:
  - the `mcpServers` of `~/.claude.json` (user scope)
  - the project entry that agrees with the working directory
  - the `.mcp.json` file of the directory and of each parent directory,
    nearest first. Claude Code reads the parent directories in
    the same sequence.

  Prometeu uses the first occurrence of an ID. If a hub server and a CLI server
  have the same ID, Prometeu uses the hub server. Discovery only reads files. It does not import
  data. After this decision, the CLI added a source that is not in a file: the
  claude.ai connectors of the account.
  [ADR 0063](0063-account-connectors-in-the-mcp-base.md) adds this source to
  the base.
- **Resolution.** `selection.rs` has a new function, `resolve_with_base`. This
  function starts the chain with an implicit `{ base: "inherit", add: <base> }`
  layer below the global layer. Thus `base: "none"` in a layer also replaces
  the CLI base. With an empty base, the function gives the same result as the
  current `resolve`. As a result, the saved selections and their tests do not
  change.
- **Provenance.** If a base ID stays on, its provenance is `cli`. The ID is
  active because the CLI configuration of the person loads it. To remove a base
  ID, the person does a usual removal in the workspace layer
  (`base: "inherit"`, with the ID in `remove`). If a higher layer with
  `base: "none"` removes an ID, the picker does not show that ID.
- **Start of the agent.** When a layer declares the axis, the private strict
  configuration contains the **full active set**. This set includes the CLI
  servers that the person kept. Thus Claude Code loads the resolved list and
  only that list. When no layer declares the axis, Prometeu does not change the
  start command. There is no strict flag, and the CLI loads its own defaults.
  The picker shows this same set.
- **Interface.** The picker shows the base rows below their own header. The
  header shows the `cli` provenance one time. A row shows the provenance as a
  badge only when the list has no headers. A removed row keeps its own badge,
  so that the person can see it easily. The MCP button of the composer stays
  on the screen when the hub is empty and there is a base. When the workspace
  layer is null, the button label continues to show "MCP do CLI". With this
  decision, this label is correct.

Scope: only Claude. Codex reads its configuration from `~/.codex/config.toml`,
and its adapter reads that file again each time it starts the agent. In this
version, Prometeu does not find the servers in that file.
[`provider-matrix.md`](../quality/provider-matrix.md) records this difference.
In this version, plugins and skills do not have a base from the CLI.

## Consequences

Positive:

- The person can see and change the active set without an import.
- The person can remove one CLI server. The removal stays for each subsequent
  start of the agent.
- When the person declares a layer, the CLI defaults that the person wants to
  keep stay in the set.
- When the hub is empty, a new user who has servers in Claude Code can see the
  control.

Negative:

- The agent start and the picker read the CLI configuration files. This adds a
  dependency on the formats of `~/.claude.json` and of the repository
  `.mcp.json` file. Prometeu does not control these formats.
- The hub is no longer the full set of IDs. Saved layers can thus refer to IDs
  that the CLI configuration adds and removes. Prometeu continues to accept
  unknown IDs.
- `base: "none"` also removes the CLI base. A person can think that it replaces
  only the hub selections, and this is not correct.
- Discovery uses only the working directory of the workspace as its root. In a
  workspace with many repositories, Prometeu does not see the `.mcp.json` file
  of a secondary repository. Claude Code has the same limit.

## Evidence

- `selection.rs`: `cli_base_participates_in_the_chain`. The base goes through
  the chain. A workspace removal removes one ID. `base: "none"` replaces the
  base. An empty base gives the same result as `resolve`.
- `mcp.rs`: `inherited_base_combines_user_project_and_repository_configuration` and
  `hub_wins_name_collisions_in_the_universe`. These tests examine the discovery
  scopes and the `.mcp.json` files of parent directories. When two files have
  the same ID, the nearest file wins. The tests also show that the first
  occurrence wins. If a hub server and a CLI server have the same ID, the hub
  server wins. `missing_selected_servers_prevent_materialization` shows that
  when the registry does not have a selected id, the configuration step stops
  with an error. The step does not remove the server without a message.
- `session.rs`: `provenance_classifies_inherited_cli_configuration`. This test
  examines `cli`, `removed`, an added ID above the base, and a global
  replacement that removes the base.
- `claude.rs`: the strict configuration contains the full active set for the
  workspace directory.
- Web: `src/mcp.test.ts` examines the inherited cache for each workspace. The
  mock copy (`composeAxis`, `resolveWithBase`, `axisProvenance`) stays the same
  as the Rust rules. `src/tool-picker.test.ts` examines the header that shows
  the `cli` provenance and the removal badge. It also examines how the picker
  opens again at the same scroll position and keyboard selection after each
  selection.
- E2E: the picker shows the base below its header. The picker stays in the
  window and scrolls when account connectors make the list long. It keeps its
  position after pointer and keyboard selections. A removal stays as a change in
  the workspace layer.
