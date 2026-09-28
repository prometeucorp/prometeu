# ADR 0064 — Cross-family, read-only default review

Date: 2026-09-28
Status: Accepted

Amends the profile model of [ADR 0009](0009-reusable-actions.md) and the
bundled profile of [ADR 0010](0010-default-code-review.md); the rest of those
decisions stands.

## Context

The bundled Code review profile from [ADR 0010](0010-default-code-review.md)
pinned Claude and relied on its prompt and on approvals to stay read-only. When
the workspace ran Claude, the reviewer shared the builder's family: judges
prefer their own generations, and two instances of one model tend to make
correlated mistakes. A fresh session removes the builder's transcript, not the
family's blind spots. [ADR 0009](0009-reusable-actions.md) treats profile
instructions as guidance, so "do not modify files" was a request.

## Options considered

1. Keep a fixed provider and tighten the prompt: the family and the guarantee
   stay the same.
2. Pick the reviewer in the presentation and pass it to `action_start`: the
   installation and account checks would live where they can be stale.
3. A provider rule resolved by the backend at task start, an access level each
   adapter materializes, and a capability that tells the presentation which
   providers can enforce it.

## Decision

Adopt the third option. `provider_rule` is `fixed` or `different_from_builder`;
the latter carries ordered candidates, each with its own model and effort.
Builders are the providers of the workspace's ordinary tabs. At start, the
backend takes the first installed and signed-in candidate outside the builders,
then the first usable candidate, then the first candidate; it never refuses the
review. The frozen task records the picked candidate and whether it shares a
builder's family, and the tab shows both.

`access: read_only` is enforced by the adapter, not by the prompt. Read-only
tasks run without MCP servers, plugins, skills or subagents: their tools, hooks
and tool grants act outside both envelopes. Claude runs in restricted mode with
`dontAsk`, only `Read`, `Grep`, `Glob` and `Bash`, the worktree added for
`CLAUDE.md` and an inline empty MCP configuration; settings files cannot widen
it and bypass is refused. `Skill` is absent because a skill's tool grants, bundled
ones included, pre-approve commands, and `Task` because its worktree isolation
writes a git worktree. Codex runs in the `read-only` sandbox with
`approvalPolicy: never`, with plugins, hooks, connector apps and computer or
browser control turned off, and every MCP server from `config.toml` disabled by
name, because `-c mcp_servers={}` merges into that table instead of replacing
it. Antigravity cannot enforce it and advertises `readOnlyProfile: false`.

The bundled profile becomes `different_from_builder` with Codex then Claude at
provider defaults and read-only access. A catalog upgrades a profile identical
to the previous seed exactly once, recorded by `defaults_revision`, at startup
and whenever a Cloud catalog is applied; customized, overridden and removed
profiles stay as they are.

## Consequences

A review usually comes from another family and uses that provider's quota; the
status bar keeps showing each provider's account. Without another usable
provider, the review still runs and says it shares the builder's family. A
read-only reviewer cannot run tests that write caches or build outputs, use a
code-review skill or plugin, or split work across subagents; checks belong to a
deterministic step outside the agent. Claude needs `--restricted` (verified with
2.1.283, present since at least 2.1.260) and Codex the feature names of 0.154.0;
an older CLI fails the spawn visibly instead of running unrestricted. Codex does
not report sandbox-denied commands as items; the reviewer's text reports them. Git configuration left in the repository, such as
`core.fsmonitor`, still runs for any Git read, including Prometeu's own.

An older app ignores the new fields and runs the first candidate as a fixed
profile; if it saves the catalog, the fields are lost and the profile stays
fixed.

## Evidence

- [Selection](../../src-tauri/src/actions/reviewer.rs),
  [validation, freezing and the seed upgrade](../../src-tauri/src/actions.rs)
  and [Cloud catalog application](../../src-tauri/src/catalog.rs).
- [Claude arguments and recording](../../src-tauri/src/claude.rs),
  [Codex parameters and recording](../../src-tauri/src/codex.rs) and the
  [Antigravity refusal](../../src-tauri/src/antigravity.rs).
- [Capability descriptor](../../src-tauri/src/agents.rs).
- [Mirror, seed parity and badges](../../src/actions.test.ts).
- [Contract](../contracts/actions.md) and
  [provider matrix](../quality/provider-matrix.md).
