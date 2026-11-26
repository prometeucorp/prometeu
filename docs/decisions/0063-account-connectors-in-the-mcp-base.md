---
type: Decision
title: "ADR 0063 — Account connectors belong to the inherited MCP base"
description: "The claude.ai connectors of the account are part of the inherited MCP base."
status: stable
---

# ADR 0063 — Account connectors belong to the inherited MCP base

Date: 2026-09-28

This ADR adds to the Discovery section of
[ADR 0046](0046-cli-inherited-mcp-base.md). The other sections of that decision
stay in effect.

## Context

ADR 0046 made the MCP set of the CLI an inherited base that the picker shows.
It specified discovery as a read of configuration files: the `mcpServers` of
`~/.claude.json` (user and project scope) and the `.mcp.json` of the working
directory and its parent directories.

After that decision, Claude Code started to load a second source that is not in
a file. Each time the CLI starts, it gets the connectors that the person enabled
on claude.ai from the account (`GET /v1/mcp_servers`, header
`anthropic-beta: mcp-servers-2025-12-04`). The CLI puts them under its own
`claudeai` scope. `~/.claude.json` keeps only a list of the names that the
person connected before. On the machine of this checkout, `/mcp` in the
terminal lists 44 servers: one from the user scope and 42 from the account. The
Prometeu picker listed one.

This causes two problems. Prometeu saw the two problems:

- The picker does not show the servers that the CLI shows. Thus the base is not
  "the same set the picker shows", as ADR 0046 promised.
- With `--strict-mcp-config`, the CLI does not get the connectors from the
  account. Thus the first declared selection removes all the connectors without
  a message. A headless session with an empty strict file reports 0 MCP
  servers and 0 MCP tools. Without the flag, it reports 43 servers and 206
  tools.

## Options considered

1. Keep discovery only for files. Show a warning in the picker that a selection
   of an MCP server turns off the account connectors.
2. Do not send `--strict-mcp-config` while the selection only adds to the set of
   the CLI. Thus the connectors stay.
3. Find the connectors from the account with the credential of the active
   login. Use them as part of the inherited base. Write them in the strict file
   as the `claudeai-proxy` entries that the CLI makes.

Option 1 tells about the loss but does not repair it. A selection then removes
42 servers without a message. Option 2 keeps a base that is not complete. The
picker continues to be not able to show or remove a connector. Also, a removal on a
different axis causes the loss again. Prometeu selected option 3. With this
option, the set in the picker is again the set that the session will load.

## Decision

The connectors of the active Claude account are part of the inherited base of
the `mcp` axis, together with the configuration files of ADR 0046:

- **Discovery.** `mcp.rs::connectors` sends a request to the account API with
  the credential of the active Claude profile. This is the same credential that
  `usage.rs` reads: from the file first, and from the macOS Keychain second.
  Prometeu does not tell the person to log in again. The request obeys
  `ANTHROPIC_BASE_URL`. Discovery only reads data. It does not import data, and
  it does not write to the files of the CLI.
- **Identity.** The ID of a connector is `claude.ai <display name>`. If a name
  occurs again, the ID gets a number at its end. Thus the ID agrees with the
  name that the CLI gives to the same server, and a saved layer continues to
  resolve. Its note is `claude.ai`. The picker shows this note as the origin of
  the row.
- **Precedence.** The account is the last source in the base. If a
  configuration file that the person controls has a server with the same name
  as a connector, Prometeu uses the server from the file. The local scope
  replaces the user scope in the same manner. A hub server continues to replace
  the two.
- **Configuration.** A kept connector goes into the strict file as
  `{ "type": "claudeai-proxy", "url": …, "id": … }`. The CLI accepts this entry
  as a dynamic server. It connects the server with the authentication of the
  account. Prometeu does not use the token of the connector.
- **Cache.** Prometeu keeps the list in a cache for five minutes. It fills the
  cache when the app starts. Thus, usually, the picker and the agent start do
  not wait for the network. The cache is for the login that made it, which is
  the account id and its revision. Thus, when the person changes the account or
  logs in again, Prometeu does not show the connectors of a different account.
  The cache keeps one revision for each account. A late response for an older
  revision cannot replace the newer revision. If a request has a failure,
  Prometeu reads the cache again. Thus a correct response from a parallel request stays
  available. After a network failure, Prometeu keeps the last known list of that
  login.
- **Unknown is not empty.** Without an account or without its credential, the
  list is empty and complete. In that condition, the CLI also loads no
  connector. These conditions make the list unknown:
  - Prometeu cannot read the credential, or it has an incorrect format, and there is no
    correct Keychain fallback.
  - A request has a failure, and there is no correct result for that login.

  When the list is unknown, the picker shows only the file base. If a declared
  selection continues to inherit the account base, Prometeu does not start the
  agent while the list is unknown. The cause is that the strict mode removes
  the connectors that the person kept, without a message. A `base: "none"`
  replacement in a layer removes that base. Thus its strict configuration can
  start also when the account lookup has a failure. An account connector that the
  person added explicitly keeps its ID through the resolution. If discovery
  cannot give its configuration, the configuration step does not start the
  agent. It does not remove that selection without a message.

Scope: only Claude, as in ADR 0046. Codex has no source of account connectors.

## Consequences

Positive:

- The picker lists all the servers that the CLI loads. The person can remove a
  connector for one workspace as a usual change in the layer.
- A declared selection does not remove the account connectors of the person.
- There is no import, no duplicate configuration and no second login.

Negative:

- With this decision, Prometeu uses an account endpoint and its beta header. Prometeu
  does not own them. If the format changes, discovery uses only the file base.
- The base changes with the active account and with edits on claude.ai. Thus a
  layer can refer to an ID that the account adds and removes. Prometeu accepts this,
  as ADR 0046 accepts it for CLI files.
- Prometeu reads the credential for a second purpose. Thus it uses the Claude
  login for more purposes.
- A cache of five minutes can show a connector that the person removed on
  claude.ai a short time before.
- A conversation that inherits account connectors does not start while their
  list is unknown. Thus an offline start is not possible, but Prometeu does not
  remove connectors without a message.

## Evidence

- `mcp.rs`: `account_connectors_join_the_inherited_base`. This test examines the
  CLI names with the number for a name that occurs again. It also examines
  these items: a server that is not complete makes no entry, and a
  configuration file replaces a connector. It also examines the
  `claudeai-proxy` entry in the strict configuration.
- `mcp.rs`: `unknown_account_connectors_prevent_materialization` shows that
  inherited connectors must have a known list.
  `account_cache_keeps_current_revision_and_observes_overlapping_fetch`
  examines revisions and parallel requests.
- `session.rs`: `explicit_account_connector_survives_replacement_until_materialization`
  examines a replacement without the base, and the failure for a missing
  explicit connector.
- `usage.rs`: `unreadable_claude_file_uses_keychain_token` and
  `malformed_and_unreadable_claude_credentials_are_unknown` show the difference
  between the Keychain fallback, read failures and a missing credential.
- `mcp.rs`: the current `inherited_base_combines_user_project_and_repository_configuration`
  continues to examine the file scopes, with an explicit empty account.
- Web: `src/mock.ts` has a copy of the base with a connector row. The import
  menu does not show this row, because connectors are not in a file.
- `mcp.rs`: `lists_real_account_connectors` has the ignore mark, as the server
  probe test has. It sends a request to the account of the person and prints the base that the picker
  will show. Thus a person can find a change in the endpoint or in its names
  manually.
- Manual test with the installed CLI 2.1.272: a strict file with one
  `claudeai-proxy` entry reports the server as `connected` with its tools. An
  empty strict file reports no servers.
