# Cross-family, read-only review

Implement issue #141: `/review` runs on a provider family other than the one that
built the workspace whenever another installed and signed-in provider exists,
and falls back visibly to the same family otherwise; it never blocks. A profile
can be read-only by construction: the adapter removes the ability to write
instead of asking the agent not to, and reads run without approval prompts. The
review tab shows the resolved provider and model, whether read-only is enforced
and whether the fallback was used.

## Boundaries and decisions

- `actions::Profile` and its TypeScript twin gain three additive fields with
  defaults: `provider_rule: fixed | different_from_builder`, ordered
  `candidates: Choice[]` and `access: default | read_only`. `fixed` keeps
  `choice` and requires no candidates. `different_from_builder` requires one to
  three candidates with distinct, runnable providers and `choice ==
  candidates[0]`, so an older app that ignores the new fields runs the first
  candidate as a fixed profile. `permission` keeps its meaning only under
  `default` access.
- A `read_only` profile is valid only when every provider it can resolve to
  advertises `AgentCapabilities.readOnlyProfile`, its MCP and plugin selections
  are `null` or empty and it names no skills. Resolution freezes empty MCP and
  plugin selections: MCP tools, plugin hooks and servers, and skill tool grants
  act outside both providers' write envelopes, so read-only means no MCP
  servers, plugins or skills.
- Builder providers are the providers of the workspace's non-task tabs, each
  tab's `choice.agent` or `Workspace.agent` when it inherits; a workspace
  without such tabs contributes `Workspace.agent`. Delegation workers are
  ordinary tabs and count.
- `pick(profile, builders, usable)` is pure. `fixed` returns `choice`.
  Otherwise it returns the first usable candidate outside the builders, then
  the first usable candidate, then the first candidate, so the spawn reports an
  unavailable provider as it does today. `same_family` is true when the rule is
  `different_from_builder` and the picked provider is a builder. Rust owns the
  real resolution in `action_start`; a TypeScript mirror serves the mock, with
  parity tests, like the existing default initialization.
- `usable` means installed and signed in: Claude and Codex are found on the
  adopted PATH and their active account's last known identity is connected;
  Antigravity passes its version check and has an attached account. It is
  evaluated lazily, only for `different_from_builder` profiles.
- The provider is picked before tool resolution, because the MCP base depends
  on the provider. The frozen `Tab.task.profile.choice` and `Tab.choice` record
  the picked candidate; `Run.same_family` records the fallback.
- `Launch.access` carries the frozen access to the adapters, which materialize
  it and refuse combinations they cannot honor:
  - Claude: `--restricted --permission-mode dontAsk --tools
    Read,Grep,Glob,Bash --add-dir <worktree>` with
    `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`, plus `--strict-mcp-config
    --mcp-config {"mcpServers":{}}` inline, never the bypass or plan flags and
    no `--plugin-dir`. Restricted mode ignores user, project and local settings
    files, so no `permissions.allow` rule, CLI-enabled plugin, project skill or
    setting can widen the envelope; write tools are absent; `Skill` is absent
    because skill grants, bundled ones included, pre-approve commands; `Task` is
    absent because its worktree isolation writes a git worktree; commands the
    CLI does not classify as read-only are denied without a prompt; switching to
    `bypassPermissions` is refused by the CLI. The project `CLAUDE.md` loads
    through the added directory; web tools are absent.
  - Codex: `sandbox: "read-only"` with `approvalPolicy: "never"` on
    `thread/start` and `thread/resume`; `--disable` for `plugins`, `hooks`,
    `apps`, `computer_use` and `browser_use`; and every MCP server from the
    session's `config.toml` disabled by name, since `-c mcp_servers={}` merges
    into that table. The operating system sandbox blocks writes and network for
    commands; nothing prompts.
  - Antigravity: `readOnlyProfile: false`; its adapter refuses read-only
    launches with `err.antigravity.unsupported`.
- The bundled profile moves to seed revision 2: `different_from_builder` with
  candidates `[codex, claude]` at provider defaults and `access: read_only`,
  keeping the prompt. `Catalog.defaults_revision` upgrades a profile identical
  to the revision 1 seed exactly once; customized, overridden or removed
  profiles stay as they are. The upgrade also runs when a Cloud catalog is
  applied, so an older document cannot undo it. Both seeds live in
  `action-defaults.json`, shared by the backend and the mock.
- The profile editor offers the provider rule, an ordered candidate list with
  per-row model and effort, and the access level. Read-only hides the
  permission field, disables MCP with an explanation and offers only providers
  whose capability allows it, using `AgentCapabilities` rather than provider
  ids. The task tab's footer shows `provider · model`, a read-only badge and a
  same-family badge, each with an explanatory tooltip; the account stays in the
  status bar.
- Out of scope: running checks outside the agent, structured line-anchored
  findings, ordinary workspace permissions, OS sandboxing for Claude's shell and
  Git configuration a builder may leave in the repository (`core.fsmonitor`,
  pagers, external diff drivers), which any Git read, including Prometeu's own,
  already executes.

## Evidence required

Rust unit tests cover builders, `pick` (single-provider, mixed-provider,
uninstalled, signed-out, nothing usable and fixed cases), validation, MCP
freezing, the one-time seed upgrade, launch arguments and thread parameters for
both access levels on start and resume, the Antigravity refusal and the
capability descriptor. Sanitized recordings from Claude 2.1.283 and Codex
0.154.0 show a failed write with no approval request and pass through the
adapters without an approval event. TypeScript tests cover the mirror and the
seed parity; the contract fixture carries the new capability. No new browser
scenario is needed under the E2E scope policy. ADR 0064 amends ADRs 0009 and
0010; the actions and agent-runtime contracts, the provider matrix, the README
and English/Portuguese copy change with the code.
