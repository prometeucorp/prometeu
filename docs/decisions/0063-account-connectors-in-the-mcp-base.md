# ADR 0063 — Account connectors belong to the inherited MCP base

Date: 2026-09-28
Status: Accepted

Extends the Discovery section of
[ADR 0046](0046-cli-inherited-mcp-base.md); the rest of that decision stands.

## Context

ADR 0046 made the CLI's own MCP set a visible inherited base and defined
discovery as a read of configuration files: the `mcpServers` of
`~/.claude.json` (user and project scope) and the `.mcp.json` of the working
directory and its ancestors.

Claude Code since then loads a second source that is in no file. The
connectors the person enables on claude.ai are fetched from the account at
every start (`GET /v1/mcp_servers`, header
`anthropic-beta: mcp-servers-2025-12-04`) and mounted under the CLI's own
`claudeai` scope; `~/.claude.json` keeps only a list of names ever connected.
On this checkout's machine `/mcp` in the terminal lists 44 servers — one from
the user scope and 42 from the account — while the Prometeu picker listed one.

Two consequences follow, and both were observed:

- the picker hides what the CLI shows, so the base is no longer "the same set
  the picker shows" that ADR 0046 promised;
- `--strict-mcp-config` makes the CLI skip the account fetch entirely, so the
  first declared selection drops every connector without saying so. A headless
  session with an empty strict file reports zero MCP servers and zero MCP
  tools, against 43 servers and 206 tools without the flag.

## Options considered

1. Keep discovery file-only and warn in the picker that choosing any MCP turns
   the account connectors off.
2. Never pass `--strict-mcp-config` while the selection only adds to the CLI's
   set, so the connectors survive.
3. Discover the connectors from the account with the active login's own
   credential, treat them as part of the inherited base, and materialize them
   in the strict file as the `claudeai-proxy` entries the CLI itself creates.

Option 1 documents the loss instead of repairing it, and leaves a selection
that silently costs 42 servers. Option 2 keeps the base incomplete: the picker
still cannot show or remove a connector, and any removal on another axis
reintroduces the loss. Option 3 was adopted: it restores the invariant that the
picker's universe is what the session will load.

## Decision

The connectors of the active Claude account join the `mcp` axis's inherited
base, alongside the configuration files of ADR 0046:

- **Discovery.** `mcp.rs::connectors` asks the account API with the credential
  of the active Claude profile — the same one `usage.rs` reads, file first and
  macOS Keychain second — and never asks the person to log in again. The
  request honors `ANTHROPIC_BASE_URL`. Discovery stays read-only: nothing is
  imported and nothing is written to the CLI's files.
- **Identity.** A connector's ID is `claude.ai <display name>`, with a numeric
  suffix on a repeated name, so it matches what the CLI calls the same server
  and a persisted layer keeps resolving. Its note is `claude.ai`, which the
  picker shows as the row's origin.
- **Precedence.** The account comes last in the base: a configuration file the
  person controls shadows a connector of the same name, as local scope already
  shadows user scope. A hub server still shadows both.
- **Materialization.** A kept connector enters the strict file as
  `{ "type": "claudeai-proxy", "url": …, "id": … }`, which the CLI accepts as a
  dynamic server and connects with the account's own authentication; Prometeu
  never handles the connector's token.
- **Freshness.** The list is cached for five minutes and warmed at startup, so
  neither the picker nor a spawn waits on the network in the common case. The
  cache belongs to the login that produced it — the account id and its revision
  — so switching accounts or logging in again never shows another account's
  connectors. Concurrent fetches update only their own login's entry. A failed
  fetch keeps that login's last known list, so a network blip changes nothing.
- **Unknown is not empty.** Without an account or without its credential the
  list is empty and complete: the CLI would load no connector either. An
  unreadable or malformed credential, or a failed fetch with no successful
  result for that login, leaves the list unknown. The picker degrades to the
  file base. A declared selection that still inherits the account base refuses
  to spawn while the list is unknown: strict mode would silently drop connectors
  the person kept. A `base: "none"` replacement at any layer excludes that base,
  so its strict configuration can start even when the account lookup fails.

Scope: Claude only, like ADR 0046. Codex has no account connector source.

## Consequences

Positive:

- the picker lists every server the CLI would load, and a connector can be
  removed for one workspace as an ordinary layer delta;
- a declared selection no longer costs the person their account connectors;
- no import, no duplicated configuration and no second login.

Negative:

- Prometeu now depends on an account endpoint and its beta header, neither of
  which it owns; a shape change degrades discovery to the file-only base;
- the base varies with the active account and with edits made on claude.ai, so
  a layer may reference an ID that comes and goes — tolerated, as ADR 0046
  already tolerates for CLI files;
- the credential is read for a second purpose, widening the reasons Prometeu
  touches the Claude login;
- a five-minute cache can show a connector removed on claude.ai minutes ago;
- a conversation that inherits account connectors does not start while their
  list is unknown, which trades an offline start for never losing connectors in
  silence.

## Evidence

- `mcp.rs`: `account_connectors_join_the_inherited_base` — CLI naming with the
  numbered repeat, the entry an incomplete server does not produce, a
  configuration file shadowing a connector, and the `claudeai-proxy` entry in
  the strict configuration.
- `mcp.rs`: `unknown_account_connectors_prevent_materialization` — inherited
  connectors require a known list; `older_account_fetch_cannot_replace_newer_account_cache`
  checks concurrent login fetches.
- `session.rs`: `replacing_mcp_base_does_not_require_account_connectors` — a
  replacement can start without the account base.
- `usage.rs`: `malformed_and_unreadable_claude_credentials_are_unknown` — a
  credential read failure is distinct from a missing credential.
- `mcp.rs`: the existing `inherited_base_combines_user_project_and_repository_configuration`
  still describes the file scopes, now with an explicit empty account.
- Web: `src/mock.ts` mirrors the base with a connector row that the import menu
  does not offer, since connectors live in no file.
- `mcp.rs`: `lists_real_account_connectors`, ignored like the server probe, asks
  the real account and prints the base the picker will show, so a change in the
  endpoint or in its naming is caught by hand.
- Manual, against the real CLI 2.1.272: a strict file carrying one
  `claudeai-proxy` entry reports the server `connected` with its tools, while
  an empty strict file reports no servers at all.
