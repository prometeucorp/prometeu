# ADR 0043 — Layered tool selection: global, project and workspace

Date: 2026-09-14
Status: Accepted — the "Authority" section is amended by
[ADR 0044](0044-cli-inherited-mcp-base.md)

## Context

Prometeu keeps a single hub of MCP servers, plugins and skills, and a
per-workspace selection. Today there are two persisted axes: `Workspace.mcp` and
`Workspace.plugins`, both `Option<Vec<String>>` with replace/inherit semantics.
Standalone skills are not a separate axis: the hub stores them as `skill-<id>`
packages that ride inside the plugin selection and are materialized through the
same plugin-package pipeline (`plugin-marketplace.md`, "Standalone skills").

Selection is captured at spawn. Changing it restarts the agent process, which is
why the picker is locked during a turn (`src/mcp.ts`).

The person wants to control, from the interface, which MCP servers, plugins and
skills are active for a workspace, with a **project** default shared through the
repository and a **global** default for the whole app. This mirrors how Claude
Code organizes scopes — project (`.mcp.json`, `.claude/*`) versus user
(`~/.claude.json`, `~/.claude/settings.json`) — but Prometeu must keep its own
hub as the source of truth and stay portable across Claude and Codex.

Two external facts shaped the design:

- Claude Code resolves project configuration **single-root**: from the current
  working directory upward, merging parent directories. `--add-dir` grants file
  access, not additional MCP configuration roots.
- Cursor does not load a per-root project `.cursor/mcp.json` in a multi-root
  workspace; project tools stay tied to the single opened folder.

The current trust contract states that global, project or other-plugin hooks
never gain trust through the workspace flow, and that selecting a package also
activates and authorizes its hooks (`plugin-marketplace.md`, "Hooks and trust").
Letting a **versioned** project file drive selection would allow a repository
author to influence which packages and hooks activate for anyone who clones and
opens it. That vector must be addressed explicitly.

## Options considered

1. Keep per-workspace selection only; add no project or global layer.
2. Read and write the CLI's native scopes (`.mcp.json`, `.claude/settings.json`,
   `~/.claude/settings.json`, `~/.claude/skills`) so the UI reflects exactly what
   the CLI loads.
3. Keep the hub and add layered selection (global → project → workspace)
   resolved by Prometeu and injected as today. Within option 3:
   - composition by replace-only inheritance, or by additive deltas with
     removals;
   - project persistence in the repository (`.prometeu/settings.toml`) or in the
     app-local board.

Option 2 is the most faithful to "what the CLI sees", but it breaks the symmetry
with Codex, abandons the portable hub decided in ADR 0005, and forces Prometeu to
reconcile its registry with files it does not own. It was rejected.

## Decision

Adopt option 3: the hub stays the registry and the injection mechanism; three
layers select among hub items and Prometeu resolves them per workspace.

### Layers and axes

Three layers, resolved in order **global → project → workspace**:

- **global**: the app-wide default selection.
- **project**: a per-repository default, shared through version control.
- **workspace**: the existing per-workspace choice.

Three **independent axes**: `mcp`, `plugins`, `skills`. Skills leave the plugin
axis and become their own. Under the hood they keep using the plugin-package
pipeline (`skills-packages/<id>`, `--plugin-dir` for Claude, the derived
marketplace for Codex); the split is a state and interface concern, not a new
materialization mechanism.

### Composition

Each layer, per axis, is either `null` or an object:

```ts
type Selection = null | { base: "none" | "inherit"; add: string[]; remove: string[] };
```

- `null` — **inherit**: fall through to the layer below.
- `base: "none"` — replace the inherited set with `add` (an empty `add` selects
  nothing from the hub).
- `base: "inherit"` — apply `add`/`remove` over the inherited set.

Resolution starts at global and ends at workspace. A terminal `null` injects
nothing from the hub. IDs removed from the hub are ignored so an old layer still
resolves. This combines a predictable inheritance chain with the ability to
remove an inherited item at a lower layer without relisting the rest.

### Persistence

- **global**: a new field on the board (`state.rs`), app-local under `<root>`,
  per axis.
- **project**: a `[tools]` table in `.prometeu/settings.toml`, parsed by the
  existing `scripts.rs` reader with its per-worktree inheritance from the
  original clone. For a multi-repository workspace the **primary repository**
  governs, matching Claude Code and Cursor's single-root behavior and the
  launcher's existing use of the primary repository for base and diff.
- **workspace**: the existing `Workspace` fields, migrated to the uniform type
  and extended with a `skills` axis.

Migration of `Workspace.mcp` and `Workspace.plugins` from `Option<Vec<String>>`:
`null → null`; `[] → {base:"none", add:[]}`; `[ids] → {base:"none", add:ids}`.
Entries matching `skill-<id>` move from `plugins` to `skills`. A change in the
persisted format requires a compatibility test and an update to
[`persistence.md`](../contracts/persistence.md).

### Application

The resolved set is captured at spawn. A change at **any** layer takes effect at
the next spawn or resume; a running session keeps the set it was born with,
consistent with the execution-account rule in
[`agent-runtime.md`](../contracts/agent-runtime.md). This removes the current
immediate restart when the workspace picker changes; the interface states that a
change applies on the next session.

### Authority

Prometeu **coexists** with the CLI's native configuration. It injects only its
own resolved set; servers and plugins the person enabled directly in the CLI
still load under that CLI's rules. `--strict-mcp-config` is not used. The
interface labels these as Prometeu-managed tools, not as the complete active set.

### Capabilities

No new capability. Skills selection is gated by the existing
`workspacePluginSelection` because it reuses the plugin pipeline. The project and
global layers are a core-resolution concern and do not change `AgentCapabilities`.

### Trust

Project-declared items require **trust on first use**, per project, mirroring
Claude Code's approval of a project `.mcp.json`. The first time a project's
`[tools]` declares items — and again whenever the declared set changes, by hash —
Prometeu asks before activating them. The decision is stored app-local on the
board, keyed by project identity plus the hash of the declared `[tools]` section;
it is never written into the repository. The prompt offers approval or rejection,
and both are decisions: an approval activates the declaration, while a rejection
keeps its items resolved but not injected — labeled as rejected in the picker —
and quiets the prompt until the declaration changes and re-pends. Until a
decision exists, project-declared items are resolved but not injected and show
as pending. The decision command binds to the hash the backend recomputes at
call time, so a verdict always covers the declaration as it stands now. Global
and workspace choices stay the person's explicit action and need no extra
approval. This keeps the existing rule that project hooks never gain trust
silently.

### Interface

Distributed and contextual. Global defaults live in Settings → Tools; project
defaults in a project surface (sidebar or project menu); workspace selection in
the existing pickers. The workspace picker shows the **effective** resolved set
with per-item provenance — inherited, added, removed, pending or rejected
project declarations, and `cli` for the inherited MCP base of
[ADR 0044](0044-cli-inherited-mcp-base.md) — so the result is visible without
opening each layer.

## Implementation plan

The accepted sequence is document, introduce tested contracts, then move
implementations (`ARCHITECTURE.md`, "Known pressures"). Each phase ends green.

1. **Contracts (with the first code phase, not before).** Update
   [`persistence.md`](../contracts/persistence.md) (board global selection,
   project trust approvals, workspace migration),
   [`plugin-marketplace.md`](../contracts/plugin-marketplace.md) (three axes,
   layered resolution, project trust),
   [`agent-runtime.md`](../contracts/agent-runtime.md) (`SessionLaunch` gains a
   `skills` list; resolution is a core concern) and
   [`ipc.md`](../contracts/ipc.md) (new commands). Move this ADR to Accepted.
2. **Pure resolution.** Add `src-tauri/src/selection.rs` with
   `resolve(global, project, workspace) -> ids` per axis, free of DOM, Tauri and
   network. Unit-test inherit, `none`, `inherit` deltas, fall-through, removal
   and unknown-ID tolerance.
3. **State and migration.** Add the board global field; migrate `Workspace.mcp`
   and `Workspace.plugins` to `Selection` and add `Workspace.skills`; cover old
   board JSON in a `session.rs` compatibility test. Extend the `scripts.rs`
   `File` with a `[tools]` table and select the primary repository for
   multi-repo workspaces.
4. **Project trust.** Hash the declared `[tools]`, store approvals on the board,
   gate injection of project-declared items on approval, and expose the prompt
   and decision through IPC.
5. **Adapters.** `claude.rs::launch_args` and `plugins.rs` materialize the
   resolved `mcp`, `plugins` and `skills`; `session.rs` resolves the layers into
   `SessionLaunch` and replaces the per-workspace selection plumbing.
6. **IPC and interface.** Register the new commands in `src/ipc.ts` with
   `src/mock.ts` parity; extend `src/mcp.ts`, `src/plugins.ts` and add a skills
   picker to show the effective set and provenance; add the global subsection in
   Settings → Tools, the project surface and the trust dialog; add the
   Portuguese and English keys.
7. **Remove the immediate restart** in the workspace picker and replace it with
   an "applies next session" state.
8. **Validation.** `npm run check`; record skills-axis and layered-selection
   evidence for both providers in
   [`provider-matrix.md`](../quality/provider-matrix.md).

## Consequences

Positive:

- one hub and one resolution rule serve both providers;
- a repository can standardize tools for the team through a versioned file;
- a project or global default can be adjusted per workspace with a removal,
  without relisting the whole set;
- skills become discoverable and controllable on their own;
- project-declared packages cannot activate silently for someone who clones the
  repository;
- the workspace picker shows the effective result, not just the local delta.

Negative:

- the persisted board format changes and needs migration and a compatibility
  test;
- three layers by three axes increase the interface and resolution surface;
- coexistence means the displayed set is the Prometeu-managed portion, not every
  tool the CLI loads, which the interface must make honest;
- a change no longer applies to a running session, so the person must start a
  new session or resume to see it;
- the `[tools]` table and the trust hash become a new repository-facing contract
  to keep stable;
- multi-repository workspaces follow only the primary repository, so a secondary
  repository's `[tools]` is ignored by design.

## Evidence

In place:

- the contracts of the first phase:
  [`persistence.md`](../contracts/persistence.md),
  [`plugin-marketplace.md`](../contracts/plugin-marketplace.md),
  [`agent-runtime.md`](../contracts/agent-runtime.md) and
  [`ipc.md`](../contracts/ipc.md);
- `selection.rs` unit tests for the inheritance chain, the two base modes,
  fall-through, removal and unknown-ID tolerance.

Pending, following the implementation plan:

- a `session.rs` compatibility test loading a pre-migration board and asserting
  the resolved `SessionLaunch` for Claude and Codex;
- a `scripts.rs` test for the `[tools]` table and primary-repository selection;
- an adapter test proving skills materialize through the plugin-package path for
  both providers;
- an interface test for the project trust prompt on first declaration and on a
  changed hash;
- [`provider-matrix.md`](../quality/provider-matrix.md) updated with the layered
  behavior and its coverage.

The native scope behavior referenced in Context comes from
[Claude Code — MCP](https://code.claude.com/docs/en/mcp) and
[Claude Code — settings](https://code.claude.com/docs/en/settings).
