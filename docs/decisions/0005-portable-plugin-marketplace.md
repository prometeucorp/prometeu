---
type: Decision
title: "ADR 0005 — One portable marketplace for Claude and Codex"
description: "One portable marketplace for Claude and Codex."
status: stable
---

# ADR 0005 — One portable marketplace for Claude and Codex

Date: 2026-09-04

## Context

Prometeu already had a plugin hub and per-workspace selection, but the adapter
delivered the selection only to Claude Code through `--plugin-dir` or
`--plugin-url`. The capability was disabled for Codex.

Codex has no equivalent flag for an arbitrary folder. Its runtime discovers
plugins through a marketplace, installs one version in the global cache and
keeps the activation state in `config.toml`. Registering that marketplace
directly in the worktree would contaminate the repository; enabling the
installation globally would let a workspace choice escape to every other
session.

Besides, hooks installed in Codex are not trusted just for being in the cache.
Trust is tied to the current hash and must preserve the boundary between the
chosen package and global or other-project hooks.

## Options considered

1. Keep plugins exclusive to Claude and hide the selector in Codex.
2. Create two hubs and ask the person to register distinct versions of the same
   plugin.
3. Write `.agents/plugins/marketplace.json` and the Codex configuration in each
   worktree.
4. Keep a common hub and materialize a derived configuration `CODEX_HOME` per
   workspace, outside the repository.

## Decision

Adopt option 4. The hub and `Workspace.plugins` stay provider-independent.
`plugins.rs` now contains two adapters:

- Claude receives the source through session flags;
- Codex receives a local marketplace generated under Prometeu's private root, a
  copy with a version derived from the content and a home separated by workspace
  (or task session) and selected account.

The installation lifecycle uses `codex plugin list/add/remove`, instead of the
app-server's experimental plugin CRUD methods. All of those commands and the
app-server receive the derived home. Its `config.toml` starts from the real
configuration, but Prometeu's marketplace, activation and hook trust are written
only in that copy. Authentication points to the account captured when the
process starts; changing selection never repoints a running process's links.
Sessions, skills, memory and plugin cache stay shared as defined in
[ADR 0012](0012-provider-accounts.md) and the
[accounts contract](../contracts/accounts.md).

Do not use a `-c` override for activation. The studied CLI version accepts
`plugins."id@marketplace".enabled=true` on the command line, but the loader does
not apply that layer; the behavior is also recorded in the official issue
[openai/codex#35289](https://github.com/openai/codex/issues/35289). A persisted,
isolated layer produces verifiable behavior without changing the global
`config.toml` or contaminating the worktree.

Selecting the package also activates and authorizes, by hash, only the hooks
that Codex assigns to that `pluginId`. Before opening the thread, the adapter
requires that every package that declared hooks has been discovered and writes
`enabled = true` along with the hash. A failure at that step ends the opening
instead of silently degrading the package to skills. The app-server's write
happens in the derived home, which can be rebuilt if a protocol version changes
or loses configuration fields.

The portable format has `.claude-plugin/plugin.json` as its base and accepts
`.codex-plugin/plugin.json` as an overlay. The creator generates both; adapting
old packages happens only in the derived copy. In that adaptation, Claude's
inline hook objects get the `hooks` envelope required by the Codex manifest; the
format's revision takes part in the cachebuster to migrate copies that already
exist.

## Consequences

Positive:

- marketplace, installation and selection stay a single experience;
- swapping Claude for Codex in the workspace does not lose the selection;
- the adaptation does not write in the worktree and does not modify the source
  package;
- an update without a version bump still invalidates the cache through the hash;
- Prometeu's plugins do not start applying outside it;
- the real `config.toml` never enters the adapter's write cycle;
- hook trust stays limited to the chosen package and the current revision;
- a mode based on `SessionStart` already applies on the first answer.

Negative:

- the first Codex session with a plugin has to copy and install the package;
- Codex's cache stays global, even though activation and marketplace are per
  workspace;
- each workspace keeps a rebuildable config and marketplace copy;
- login storage explicitly configured as `keyring` still follows the independent
  identity of each `CODEX_HOME`;
- the adapter depends on Codex's plugin format and commands;
- a managed policy that prevents activating a selected hook also prevents
  opening the conversation;
- a local `.zip` and a URL remain sources exclusive to Claude until Codex offers
  an equivalent safe installation;
- real differences between the two CLIs' events or components may still require
  conditional content inside the package.

## Evidence

- `plugins.rs` tests discovery of both marketplace formats and generation of the
  Codex package/marketplace from a compatible plugin;
- an opt-in test installs a temporary skill and `SessionStart` hook with the
  real CLI, runs the app-server handshake, proves both reached the derived
  runtime and compares the global `config.toml` before and after;
- `codex.rs` tests that only hooks of the chosen `pluginId` receive the
  activation and the trust hash before the thread is opened, and that incomplete
  discovery or writing prevents the thread;
- `agents.rs` and the launcher E2E demonstrate the capability in both providers;
- [`plugin-marketplace.md`](../contracts/plugin-marketplace.md) records the
  operational contract;
- the official documentation accepts `.agents` and `.claude-plugin` marketplaces
  and compatible packages:
  [OpenAI — Plugin management](https://learn.chatgpt.com/pt-BR/docs/enterprise/plugin-management);
- the native structure and hooks follow
  [OpenAI — Build plugins](https://developers.openai.com/codex/plugins/build)
  and [OpenAI — Hooks](https://developers.openai.com/codex/hooks);
- the independent authentication behavior per `CODEX_HOME` is deliberate in
  [openai/codex#15410](https://github.com/openai/codex/issues/15410); the
  default file mode is shared through a link in the adapter;
- keeping the trust write in the derived home also contains the key-loss risk
  reported in
  [openai/codex#42116](https://github.com/openai/codex/issues/42116).
