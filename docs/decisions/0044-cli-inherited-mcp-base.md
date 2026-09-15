# ADR 0044 — The CLI's MCP set is the visible inherited base

Date: 2026-09-15
Status: Accepted

Amends the "Authority" section of
[ADR 0043](0043-layered-tool-selection.md); the rest of that decision stands.

## Context

ADR 0043 layered MCP, plugin and skill selection over the Prometeu hub and
resolved the layers at spawn. Testing the real app exposed an incoherence in
the `mcp` axis:

- the hub starts empty — it is a Prometeu-owned registry, and servers the
  person already uses live in Claude Code's own configuration;
- when no layer declared the axis, the spawn passed no `--strict-mcp-config`,
  so Claude Code loaded its own servers (`~/.claude.json` user scope, the
  project entry for the working directory, and the repository's `.mcp.json`);
- the picker listed only hub rows, and the composer hid the MCP button when
  the hub was empty and no layer existed.

The result: sessions were born with an effective set the interface never
showed and could not adjust. Removing one CLI server required either editing
CLI files by hand or declaring a layer, which then dropped *all* CLI defaults
because the strict configuration carried only hub picks. ADR 0043 accepted
coexistence but required the interface to be honest about it, and it was not.

## Options considered

1. Auto-import the discovered CLI servers into the hub on first sight.
2. Keep coexistence invisible and document that the picker manages only the
   Prometeu portion.
3. Treat the CLI's effective set as an implicit base layer below global:
   resolve it with the existing chain, show it in the picker with its own
   provenance, and materialize the whole effective set when a layer declares
   the axis.

Option 1 copies configuration Prometeu does not own, creates duplicate
identities that drift from the CLI, and writes to the hub without the person's
action. Option 2 keeps the dishonesty that motivated this ADR. Option 3 was
adopted.

## Decision

For the `mcp` axis of Claude workspaces, the servers discovered from the CLI
configuration form a **visible inherited base**, and the picker's universe is
`hub ∪ inherited`:

- **Discovery.** Per workspace working directory: the `mcpServers` of
  `~/.claude.json` (user scope), the project entry matching the working
  directory, and the directory's `.mcp.json` plus every ancestor directory's,
  nearest first, as Claude Code walks the tree upward. The first occurrence of
  an ID wins; a hub server shadows a discovered one with the same ID. Discovery
  is read-only and requires no import.
- **Resolution.** `selection.rs` gains `resolve_with_base`, which seeds the
  chain with an implicit `{ base: "inherit", add: <base> }` layer below
  global. `base: "none"` at any layer therefore also replaces the CLI base.
  With an empty base it reduces to the existing `resolve`, so persisted
  selections and their tests are unchanged.
- **Provenance.** A base ID that stays on is classified `cli` — active because
  the person's CLI configuration loads it. Removing one is an ordinary
  workspace-layer removal (`base: "inherit"`, ID in `remove`). An ID wiped by
  an upper `base: "none"` is simply absent, with no story to tell.
- **Spawn.** When any layer declares the axis, the private strict
  configuration materializes the **whole effective set**, including kept
  CLI-inherited servers, so the resolved list is exactly what Claude Code
  loads. When no layer declares it, nothing changes: no strict flag, and the
  CLI loads its own defaults — now the same set the picker shows.
- **Interface.** The picker lists base rows with the `cli` badge, and the
  composer's MCP button no longer hides when the hub is empty but a base
  exists. The button label for a null workspace layer keeps reading
  "MCP do CLI", which is now literally true.

Scope: Claude only. Codex reads its configuration from `~/.codex/config.toml`
and its adapter re-reads it per spawn; discovering that file is a follow-up,
and the gap is recorded in
[`provider-matrix.md`](../quality/provider-matrix.md). Plugins and skills have
no CLI-inherited base in this version.

## Consequences

Positive:

- the effective set is visible and adjustable without importing anything;
- removal of a single CLI server is expressible and survives across spawns;
- declaring a layer no longer silently drops the CLI defaults the person
  meant to keep;
- the empty hub no longer hides the control from a first-time user who already
  has servers in Claude Code.

Negative:

- spawn and picker read CLI configuration files, adding a filesystem coupling
  to `~/.claude.json` and repository `.mcp.json` formats Prometeu does not
  own;
- the ID universe is no longer the hub, so persisted layers may reference IDs
  that come and go with the CLI configuration (unknown IDs stay tolerated);
- `base: "none"` now also wipes the CLI base, which can surprise someone who
  thought it only replaced hub picks;
- discovery follows the workspace's working directory single-root, so a
  secondary repository's `.mcp.json` in a multi-repo workspace is not seen —
  the same limitation Claude Code has.

## Evidence

- `selection.rs`: `a_base_do_cli_participa_da_cadeia` — the base flows through
  the chain, a workspace removal drops one ID, `base: "none"` replaces the
  base, and an empty base equals `resolve`.
- `mcp.rs`: `a_base_herdada_vem_do_usuario_do_projeto_e_do_repositorio` and
  `o_hub_vence_colisao_no_universo` — discovery scopes, ancestor `.mcp.json`
  files with the nearest winning a clash, first-occurrence wins, hub shadows a
  clash. `escolhido_ausente_impede_a_materializacao` — a chosen id the registry
  lost fails the materialization instead of being dropped.
- `session.rs`: `provenance_classifica_a_base_herdada_do_cli` — `cli`,
  `removed`, an add over the base, and the global replacement clearing it.
- `claude.rs`: the strict configuration carries the whole effective set for
  the workspace directory.
- Web: `src/mcp.test.ts` for the per-workspace inherited cache; the mock
  mirror (`composeAxis`, `resolveWithBase`, `axisProvenance`) stays identical
  to the Rust rules.
- E2E: the picker shows the base badged as inherited from the CLI and a
  removal persists as a workspace-layer delta.
