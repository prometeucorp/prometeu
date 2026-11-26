---
type: Decision
title: "ADR 0045 — Layered tool selection: global, project and workspace"
description: "A selection of MCP servers, plugins and skills in three layers: global, project and workspace."
status: stable
---

# ADR 0045 — Layered tool selection: global, project and workspace

Date: 2026-09-14
[ADR 0046](0046-cli-inherited-mcp-base.md) changes the "Authority" section.
[ADR 0047](0047-tool-selection-boundaries.md) replaces the "Trust" section.

## Context

Prometeu keeps one hub of MCP servers, plugins and skills, and a selection for
each workspace. At this time, the stored data has two axes: `Workspace.mcp` and
`Workspace.plugins`. Each axis is an `Option<Vec<String>>` that replaces or
inherits a value. Standalone skills are not a separate axis. The hub stores them
as `skill-<id>` packages in the plugin selection. The same plugin-package
pipeline makes them into files (`plugin-marketplace.md`, "Standalone skills").

The application records the selection when it starts the agent process. A
change to the selection starts the agent process again. For this reason, the
interface locks the picker during a turn (`src/mcp.ts`).

A person wants to select from the interface the active MCP servers, plugins and
skills for a workspace. The person also wants two default selections:

- A **project** default that the repository shares.
- A **global** default for all of the app.

Claude Code has scopes that are almost the same: project (`.mcp.json`,
`.claude/*`) and user (`~/.claude.json`, `~/.claude/settings.json`). But the hub
of Prometeu must stay the source of truth. It must also stay portable between
Claude and Codex.

Two external facts had an effect on the design:

- Claude Code reads the project configuration from **one root** only. It starts
  at the current working directory, goes up and merges the parent folders.
  `--add-dir` gives access to files. It does not add roots for MCP
  configuration.
- Cursor does not load a project `.cursor/mcp.json` for each root in a
  workspace with many roots. The project tools apply only to the one folder that
  is open.

The current trust contract has two rules. Hooks from the global scope, the
project or a different plugin do not get trust through the workspace flow at
any time. When a person selects a package, its hooks also become active and
authorized (`plugin-marketplace.md`, "Hooks and trust"). If a **versioned**
project file controls the selection, the author of a repository can change
which packages and hooks become active. This applies to all persons who clone
and open the repository. The design must give a clear solution to this risk.

## Options considered

1. Keep only the selection for each workspace. Do not add a project layer or a
   global layer.
2. Read and write the native scopes of the CLI (`.mcp.json`,
   `.claude/settings.json`, `~/.claude/settings.json`, `~/.claude/skills`).
   Thus the interface shows exactly what the CLI loads.
3. Keep the hub and add a selection in layers (global → project → workspace).
   Prometeu resolves the layers and injects the result as it does now. Option 3
   has two choices:
   - The composition can use inheritance that only replaces, or additive
     changes with removals.
   - The project data can be in the repository (`.prometeu/settings.toml`) or in
     the board of the app.

Option 2 shows most accurately what the CLI sees. But it breaks the symmetry
with Codex, and Prometeu does not keep the portable hub of ADR 0005. It also makes Prometeu
align its registry with files that it does not own. Thus we rejected option 2.

## Decision

Adopt option 3. The hub stays the registry and the injection mechanism. Three
layers select items from the hub, and Prometeu resolves them for each
workspace.

### Layers and axes

Prometeu resolves three layers in this sequence: **global → project →
workspace**.

- **global**: the default selection for all of the app.
- **project**: a default for each repository that version control shares.
- **workspace**: the current choice for each workspace.

There are three **independent axes**: `mcp`, `plugins` and `skills`. Skills
leave the plugin axis and get their own axis. Internally, skills continue to
use the plugin-package pipeline (`skills-packages/<id>`, `--plugin-dir` for
Claude and the derived marketplace for Codex). The division applies only to the
state and the interface. It is not a new mechanism to make files.

### Composition

Each layer has a value for each axis. The value is `null` or an object:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

- `null` — **inherit**: use the result of the layer below.
- `base: "none"` — replace the inherited set with `add`. An empty `add` selects
  no item from the hub.
- `base: "inherit"` — apply `add` and `remove` to the inherited set.

The resolution starts at the global layer and stops at the workspace layer. If
the last value is `null`, Prometeu injects no item from the hub. Prometeu
ignores IDs that are not in the hub at this time. Thus a previous layer continues
to resolve. This design gives a chain of inheritance that you can predict. A
lower layer can also remove one inherited item without a new list of all
the other items.

### Persistence

- **global**: a new field on the board (`crates/core/src/board.rs`), local to the
  app under `<root>`, with one value for each axis.
- **project**: a `[tools]` table in `.prometeu/settings.toml`. The current
  `scripts.rs` reader parses it. Each worktree inherits the file from the
  original clone. For a workspace with many repositories, the **primary
  repository** controls the selection. Claude Code and Cursor also read only
  one root. The launcher already uses the primary repository for the base and
  the diff.
- **workspace**: the current `Workspace` fields. The migration changes them to
  the same type and adds a `skills` axis.

The migration of `Workspace.mcp` and `Workspace.plugins` from
`Option<Vec<String>>` is:

- `null → null`
- `[] → {base:"none", add:[]}`
- `[ids] → {base:"none", add:ids}`

Entries that agree with `skill-<id>` move from `plugins` to `skills`. A change
in the stored format must have a compatibility test and an update to
[`persistence.md`](../contracts/persistence.md).

### Application

Prometeu records the resolved set when it starts the agent process. A change in
**any** layer applies at the next start or resume. A session that operates
keeps the set that it started with. This agrees with the rule for the execution
account in [`agent-runtime.md`](../contracts/agent-runtime.md). Thus a change
in the workspace picker does not start the agent again immediately. The
interface tells the person that the change applies to the next session.

### Authority

Prometeu operates together with the native configuration of the CLI. It
injects only its own resolved set. The servers and plugins that a person
enabled directly in the CLI continue to load under the rules of that CLI.
Prometeu does not use `--strict-mcp-config`. The interface shows these tools as
tools that Prometeu controls. It does not show them as the full active set.

### Capabilities

This decision adds no capability. The current `workspacePluginSelection`
capability controls the skills selection, because skills use the plugin
pipeline. The core resolves the project and global layers. Thus these layers do
not change `AgentCapabilities`.

### Trust

Items that a project declares must get **trust on first use** for each project.
This is the same as the approval in Claude Code of a project `.mcp.json`.
Prometeu asks the person before it activates the items in these conditions:

- The `[tools]` table of a project declares items for the first time.
- The declared set changes. Prometeu finds the change through a hash.

Prometeu stores the decision on the board, local to the app. The key of the
decision is the project identity and the hash of the declared `[tools]` section.
Prometeu does not write the decision into the repository at any time.

The prompt gives two choices, approval or rejection. Each choice is a decision:

- An approval activates the declaration.
- A rejection keeps the items resolved but does not inject them. The picker
  shows them as rejected. The prompt stops until the declaration changes and
  becomes pending again.

Before a person makes a decision, Prometeu resolves the project items but does
not inject them. The picker shows them as pending. The decision command uses the
hash that the backend calculates again at the time of the call. Thus a decision
always applies to the current declaration. The global and workspace choices stay
explicit actions of the person, and they do not get an approval step. This
keeps the current rule: project hooks do not get trust without the knowledge of
the person.

### Interface

The controls are in different locations, each in its context:

- The global defaults are in Settings → Tools.
- The project defaults are in a project area (the sidebar or the project menu).
- The workspace selection is in the current pickers.

The workspace picker shows the **effective** resolved set. For each item, it
shows the source: inherited, added, removed, pending or rejected project
declarations, and `cli` for the inherited MCP base of
[ADR 0046](0046-cli-inherited-mcp-base.md). Thus a person can see the result
in the picker and does not open each layer.

## Implementation plan

The accepted sequence is: write the document, add tested contracts, then move
the implementations (`ARCHITECTURE.md`, "Known pressures"). All checks must pass
at the end of each phase.

1. **Contracts (with the first code phase, not before).** Update these
   contracts, then change the status of this ADR to Accepted:
   - [`persistence.md`](../contracts/persistence.md): the global selection on
     the board, the project trust approvals and the workspace migration.
   - [`plugin-marketplace.md`](../contracts/plugin-marketplace.md): the three
     axes, the resolution in layers and the project trust.
   - [`agent-runtime.md`](../contracts/agent-runtime.md): `SessionLaunch` gets a
     `skills` list, and the core does the resolution.
   - [`ipc.md`](../contracts/ipc.md): the new commands.
2. **Pure resolution.** Add `src-tauri/src/selection.rs` with
   `resolve(global, project, workspace) -> ids` for each axis. It must not use
   the DOM, Tauri or the network. Add unit tests for these items:
   - Inheritance, `none` and `inherit` changes.
   - The result from the lower layer, removal and IDs that are not known.
3. **State and migration.** Do these tasks:
   - Add the global field to the board.
   - Migrate `Workspace.mcp` and `Workspace.plugins` to `Selection`, and add
     `Workspace.skills`.
   - Add a compatibility test in `session.rs` for previous board JSON.
   - Add a `[tools]` table to the `File` of `scripts.rs`.
   - Select the primary repository for workspaces with many repositories.
4. **Project trust.** Calculate the hash of the declared `[tools]`. Store the
   approvals on the board. Inject the project items only after an approval.
   Give the prompt and the decision through IPC.
5. **Adapters.** `claude.rs::launch_args` and `plugins.rs` make the resolved
   `mcp`, `plugins` and `skills` into files. `session.rs` resolves the layers
   into `SessionLaunch` and replaces the selection code for each workspace.
6. **IPC and interface.** Do these tasks:
   - Register the new commands in `src/ipc.ts`, with parity in `src/mock.ts`.
   - Change `src/mcp.ts` and `src/plugins.ts`, and add a skills picker. They
     show the effective set and the source of each item.
   - Add the global subsection in Settings → Tools, the project area and the
     trust dialog.
   - Add the Portuguese and English keys.
7. **Remove the immediate restart** in the workspace picker. Replace it with an
   "applies next session" state.
8. **Validation.** Run `npm run check`. Record the evidence for the skills axis
   and the selection in layers for the two providers in
   [`provider-matrix.md`](../quality/provider-matrix.md).

## Consequences

Positive:

- One hub and one resolution rule operate for the two providers.
- A repository can give the same tools to all of the team through a versioned
  file.
- A workspace can change a project default or a global default with a removal.
  It does not write all the set again.
- A person can find and control skills separately.
- Packages that a project declares cannot become active without the knowledge
  of a person who clones the repository.
- The workspace picker shows the effective result, not only the local change.

Negative:

- The stored board format changes. It must have a migration and a
  compatibility test.
- Three layers and three axes make the interface and the resolution larger.
- Prometeu operates together with the CLI. Thus the set on the screen is only
  the part that Prometeu controls, not all the tools that the CLI loads. The
  interface must tell this clearly.
- A change does not apply to a session that operates. The person must start a
  new session or resume to see the change.
- The `[tools]` table and the trust hash become a new contract with the
  repository. This contract must stay stable.
- A workspace with many repositories uses only the primary repository. Thus
  Prometeu ignores the `[tools]` of a secondary repository on purpose.

## Evidence

In place:

- The contracts of the first phase:
  [`persistence.md`](../contracts/persistence.md),
  [`plugin-marketplace.md`](../contracts/plugin-marketplace.md),
  [`agent-runtime.md`](../contracts/agent-runtime.md) and
  [`ipc.md`](../contracts/ipc.md).
- The unit tests in `selection.rs`. They examine the chain of inheritance, the
  two base modes, the result from the lower layer, removal and unknown IDs.

Pending, in the sequence of the implementation plan:

- A compatibility test in `session.rs` that loads a board from before the
  migration. It must make sure that the resolved `SessionLaunch` is correct for
  Claude and Codex.
- A test in `scripts.rs` for the `[tools]` table and the selection of the
  primary repository.
- An adapter test that shows that skills become files through the
  plugin-package path for the two providers.
- An interface test for the project trust prompt on the first declaration and on
  a changed hash.
- An update of [`provider-matrix.md`](../quality/provider-matrix.md) with the
  behavior in layers and its test coverage.

The information about the native scopes in the "Context" section comes from
[Claude Code — MCP](https://code.claude.com/docs/en/mcp) and
[Claude Code — settings](https://code.claude.com/docs/en/settings).
