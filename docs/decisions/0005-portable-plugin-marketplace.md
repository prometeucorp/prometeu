---
type: Decision
title: "ADR 0005 — One portable marketplace for Claude and Codex"
description: "One plugin marketplace for Claude and Codex, with a derived Codex configuration for each workspace."
status: stable
---

# ADR 0005 — One portable marketplace for Claude and Codex

Date: 2026-09-04

## Context

Prometeu had a plugin hub and a selection for each workspace. But the adapter
gave the selection only to Claude Code, through `--plugin-dir` or
`--plugin-url`. Prometeu disabled the capability for Codex.

Codex does not have an equivalent flag for a folder that a person selects. The
Codex runtime finds plugins through a marketplace. It installs one version in
the global cache, and it keeps the activation state in `config.toml`. These two
alternatives cause problems:

- If the adapter writes that marketplace in the worktree, it adds Codex
  files to the repository.
- If the adapter enables the installation globally, the selection of one
  workspace goes to all other sessions.

Also, Codex does not trust installed hooks only because they are in the cache.
Codex connects the trust to the current hash. The trust must keep the boundary
between the selected package and the global hooks or the hooks of other
projects.

## Options considered

1. Keep plugins only for Claude and hide the selector in Codex.
2. Make two hubs and tell the person to add different versions of the same
   plugin.
3. Write `.agents/plugins/marketplace.json` and the Codex configuration in each
   worktree.
4. Keep one shared hub and make a derived `CODEX_HOME` configuration for each
   workspace, out of the repository.

## Decision

Use option 4. The hub and `Workspace.plugins` do not change for each provider.
`plugins.rs` contains two adapters:

- Claude gets the source through session flags.
- Codex gets a local marketplace in the private root of Prometeu. It also gets
  a copy with a version that comes from the content, and a home for each
  workspace (or task session) and each selected account.

The installation lifecycle uses `codex plugin list/add/remove`. It does not use
the experimental plugin CRUD methods of the app-server. All of those commands
and the app-server get the derived home. Its `config.toml` starts as a copy of
the configuration of the person. Prometeu writes its marketplace, the
activation and the hook trust only in that copy. The authentication points to
the account that the application recorded when the process started. A change to
the selection does not change the links of a process that operates at that
time. Sessions, skills, memory and the plugin cache stay shared, as
[ADR 0012](0012-provider-accounts.md) and the
[accounts contract](../contracts/accounts.md) specify.

Do not use a `-c` override for activation. The CLI version that the project
examined accepts `plugins."id@marketplace".enabled=true` on the command line.
But the loader does not apply that layer. The official issue
[openai/codex#35289](https://github.com/openai/codex/issues/35289) also records
this behavior. A stored, isolated layer gives behavior that a person can
examine. It does not change the global `config.toml`, and it does not add files
to the worktree.

When a person selects the package, the adapter also activates and trusts, by
hash, only the hooks that Codex connects to that `pluginId`. Before the adapter
opens the thread, Codex must find each package that has hooks. Then the adapter
writes `enabled = true` with the hash. If this step has a failure, the
conversation does not open. The adapter does not silently use only the skills
of the package. The app-server writes in the derived home. The adapter can make
the derived home again if a protocol version changes or removes configuration
fields.

The portable format uses `.claude-plugin/plugin.json` as its base. It accepts
`.codex-plugin/plugin.json` as an overlay. The plugin creator makes the two
files. The adapter changes previous packages only in the derived copy. In that
change, the inline hook objects of Claude get the `hooks` envelope that the
Codex manifest must have. The revision of the format is part of the
cachebuster. Thus, the adapter also updates the copies that are there before
the change.

## Consequences

Positive:

- The marketplace, the installation and the selection stay one experience.
- When a person changes from Claude to Codex in the workspace, the selection
  stays the same.
- The adaptation does not write in the worktree, and it does not change the
  source package.
- If an update does not change the version, the hash continues to make the
  adapter replace the cache.
- The plugins of Prometeu do not apply out of Prometeu.
- The adapter does not write to the `config.toml` of the person at any time.
- The hook trust stays only on the selected package and the current revision.
- A mode that uses `SessionStart` applies on the first answer.

Negative:

- The first Codex session with a plugin must copy and install the package.
- The Codex cache stays global, but the activation and the marketplace are for
  each workspace.
- Each workspace keeps a copy of the configuration and of the marketplace. The
  adapter can make this copy again.
- If a person explicitly configures `keyring` as the login storage, the login
  continues to follow the independent identity of each `CODEX_HOME`.
- If the plugin format or the commands of Codex change, the adapter must
  change.
- If a managed policy stops the activation of a selected hook, the
  conversation also does not open.
- A local `.zip` and a URL stay sources only for Claude. This continues until
  Codex has an equivalent safe installation.
- The events or components of the two CLIs are different. Thus, a package can
  continue to have conditional content.

## Evidence

- Tests in `plugins.rs` examine the discovery of the two marketplace formats.
  They also examine how the adapter makes the Codex package and marketplace from a
  compatible plugin.
- An opt-in test installs a temporary skill and a `SessionStart` hook with the
  installed CLI. It runs the app-server handshake and shows that the two items are in
  the derived runtime. It compares the global `config.toml` before and after.
- Tests in `codex.rs` examine that only hooks of the selected `pluginId` get the activation
  and the trust hash before the thread opens. They also examine that the thread does not open if
  the discovery or the write is not complete.
- `agents.rs` and the launcher E2E show the capability with the two providers.
- [`plugin-marketplace.md`](../contracts/plugin-marketplace.md) records the
  contract of the operation.
- The official documentation accepts `.agents` and `.claude-plugin`
  marketplaces and compatible packages:
  [OpenAI — Plugin management](https://learn.chatgpt.com/pt-BR/docs/enterprise/plugin-management).
- The native structure and the hooks follow
  [OpenAI — Build plugins](https://developers.openai.com/codex/plugins/build)
  and [OpenAI — Hooks](https://developers.openai.com/codex/hooks).
- The independent authentication for each `CODEX_HOME` is intentional in
  [openai/codex#15410](https://github.com/openai/codex/issues/15410). The
  adapter shares the default file mode through a link.
- Because the trust write stays in the derived home, the risk of key loss in
  [openai/codex#42116](https://github.com/openai/codex/issues/42116) stays in
  that home.
