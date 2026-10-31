---
type: Contract
title: "Cursor CLI"
description: "ACP transport, external account, conversation lifecycle and capability limits."
status: stable
---

# Cursor CLI

Implemented on the macOS/Linux desktop; [ADR 0095](../decisions/0095-cursor-acp.md).

## Installation and accounts

Discovery runs the bounded `cursor-agent acp --help` command. The launcher and
Settings use the existing provider descriptor and account controls. Attach the
external CLI account, then select its card. Login and switching the underlying
identity remain owned by `agent login` and Cursor. Quota queries read the CLI's
existing access token without copying, refreshing or changing its credential store.
Removing the account clears selection without logging out.
The persisted provider and external account ID are both `cursor`.

## Transport and lifecycle

The child runs `cursor-agent acp` in the workspace directory. JSON-RPC 2.0 frames
are newline-delimited. Startup sends `initialize` (protocol 1), `authenticate`
(`cursor_login`), `session/new` or `session/load`, optional `session/set_model`,
and `session/set_mode`. Resume preserves the mode reported by native history;
the initial plan flag applies to new sessions. The first prompt waits until startup finishes. Each
`session/prompt` contains text; attachments remain the common path-mention
contract. Instructions are prepended once per process. Busy input remains queued
by the shared session core. `session/cancel` interrupts the active prompt.

The native session ID is saved in `Tab.agent_session`. Prometeu retains V1
presentation in `chats/<session>.jsonl`; native replay during `session/load` is
ignored to avoid duplicate messages. Authentication, model, mode or resume
errors end pending work with a translated error and close input. The desktop
treats the connection as stopped even if the child has not exited yet; the next
user message replaces it through the normal shutdown and resume lifecycle.
Authentication runs again, and a saved native session ID is retained. Native
stderr diagnostics do not enter the shared transcript; text delivered as an ACP assistant chunk is preserved,
including errors embedded by Cursor itself. There is no automatic new-session fallback on resume
failure. An unprompted native session may not survive process exit.

ACP model IDs, including bracket parameters, are opaque. Model discovery creates
an unprompted ephemeral ACP session under the bounded private query transport,
reads `models.availableModels`, and closes it. No models or effort levels are
invented from provider names or CLI aliases.

## Translation

Text and thinking chunks become streaming assistant blocks, finalized at tool
boundaries and completion. Tool calls retain native IDs; updates supply inputs,
output and terminal status. Tool output accepts both ACP content blocks and
Cursor's native `rawOutput`. Only updates for the active session enter V1.
Native terminal responses settle the turn once; cancellation settles it as
interrupted. Unknown notifications do not alter conversation state.

Permission requests retain their JSON-RPC IDs for replies. Presentation IDs are
scoped to the turn so restarting the CLI cannot hide a new card behind an old
request with the same native counter. Ask mode uses the existing
approval card; automatic execution uses the advertised `allow_once` option.
Denial uses `reject_once`, or cancellation if no rejection option exists.
No approval rewrites Cursor's persistent allowlist. Explicit CLI deny rules stay
in Cursor. Stop cancels pending client requests as well as the prompt.

`cursor/ask_question` maps prompts/options to the common question card and maps
selected labels back to native IDs independently of multi-select click order.
Free text is returned in a skipped response's reason, since the extension only accepts option IDs as structured answers.
`cursor/create_plan` renders the plan text and waits for existing plan approval
controls. Unknown client methods receive `-32601` rather than remaining pending.

## Capability limits

Streaming, tools, resume, stop, initial plan mode, approvals, questions and local
path mentions are available. Hub MCP/plugins/skills, managed multiple accounts,
per-turn token/cost reporting, context reports and explicit compaction are not.
Absent workspace selections resolve to empty hub selections for unsupported axes,
so global and trusted-project tools cannot block startup or resume. Explicit
nonempty workspace selections still fail rather than being silently dropped.
Cursor may load its own project/user rules, skills, hooks and MCP configuration; Prometeu
makes no isolated-configuration claim. Team-level MCP support is limited by
Cursor itself. Windows/WSL registration is outside this integration.

## Account quotas

`cursor/quota.rs` queries Cursor's internal dashboard API independently of ACP,
without inference. It joins the existing account scheduler, stale-reading cache,
backoff and removal/reconnection generation checks. The existing `{ windows, at }`
quota contract and IPC stay unchanged. The account need not send a message first.

Authentication uses `CURSOR_AUTH_TOKEN` when explicitly set. An environment API
key or an in-memory CLI credential store does not borrow a saved subscription.
Otherwise macOS reads the CLI's `cursor-access-token` Keychain service for
`cursor-user` with a five-second private subprocess deadline. Explicit
`AGENT_CLI_CREDENTIAL_STORE=file` uses `~/.cursor/auth.json`; Linux reads
`$XDG_CONFIG_HOME/cursor/auth.json` or `~/.config/cursor/auth.json`. Relative
Linux config roots are not used. No browser cookies or Cursor editor database
are read. Tokens remain in memory and are never refreshed, logged or persisted.

The JWT subject and expiry form a `WorkosCursorSessionToken` cookie sent only to
`https://cursor.com`. HTTP redirects are disabled; each request has a ten-second
timeout and a 1 MiB response limit. Expired, malformed or rejected credentials
leave quota data unavailable or retain the previous reading. Cursor owns login
renewal. As with other external profiles, changing the CLI login outside
Prometeu does not provide managed identity continuity.

`POST /api/dashboard/get-sand-usage-status` with an empty JSON body and matching
Origin identifies the quota regime. Explicit `isUnified: true` uses the shared
weekly `usagePercent` and `nextResetTimestampUtc`. `includedLimitZero` takes
precedence over the older `hasNonZeroIncludedLimit` allowance flag. Exhausted
paid allowances remain visible. Missing, malformed, expired or failed weekly
readings never fall back to a misleading monthly zero.

Only explicit `isUnified: false` enables `GET /api/usage-summary`. Supported
individual included plans use their billing-cycle reset and the reported total,
Cursor and API percentages; omitted pools remain absent. Team budgets, unlimited
plans and nonrecurring trial grants are not inferred. On-demand spend is outside
the quota meter. Unknown responses retain the previous reading, never invent 0%.

These endpoints are internal and can change independently of ACP. The approach
is also documented by [CodexBar](https://github.com/steipete/CodexBar/blob/main/docs/cursor.md).
The live quota check verifies the current signed-in account without inference;
monthly mapping and failure handling are synthetic fixture tests.

## Native streaming diagnostics

Cursor CLI `2026.10.22-27dcf5f` can send
`Error: RetriableError: WritableIterable is closed` as an `agent_message_chunk`
after answer text, then return `stopReason: end_turn`. This was also observed
with a standalone ACP client outside Prometeu. Do not strip matching assistant
text or retry automatically: that could conceal a real failure or repeat tool
effects. Cursor support describes this as a
[known transient stream teardown bug](https://forum.cursor.com/t/cursor-cli-error-writableiterable-is-closed/165741/5).
The person can send another prompt or resume the native session.

## Evidence

`src-tauri/crates/protocols/src/cursor/tests.rs` covers translation and protocol
failures using a [sanitized recording](../../src-tauri/crates/protocols/src/cursor/fixtures/README.md).
The generated V1 fixture is consumed by `src/conversation.test.ts`. Account
compatibility lives in `src-tauri/crates/core/src/accounts/tests.rs`; model
selection persistence lives in `src/model-choice.test.ts`. Existing account and
launcher browser journeys cover reused controls; no additional provider matrix
of browser tests is introduced.

`src-tauri/src/cursor/quota/tests.rs` covers quota regimes, missing/malformed
data, expiry, credential validation and unchanged window serialization. Run its
ignored `live_cursor_quota` test for a read-only authenticated dashboard check.

For an explicitly authorized live check with paid inference:

```sh
cd src-tauri
cargo test --bin Prometeu cursor::tests::live_cursor_round_trip -- --ignored --exact
```

The test confines file work to a fresh temporary directory. Default suites do not
invoke Cursor or spend model credits.
