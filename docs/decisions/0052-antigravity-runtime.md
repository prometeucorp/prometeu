---
type: Decision
title: "ADR 0052 — Replace Gemini CLI with Antigravity"
description: "Replace Gemini CLI with Antigravity."
status: stable
---

# ADR 0052 — Replace Gemini CLI with Antigravity

Date: 2026-09-19

## Context

The installed Gemini CLI rejects the consumer Google account of the user. The
cause is that Google stopped that client for individuals. Antigravity CLI
(`agy` 1.2.7) accepts the same account, and it answered a headless prompt in a
test with the account. A new name for the ACP adapter is not sufficient, because agy has a
different NDJSON protocol. This decision replaces the retired ADR 0051.

## Decision

Remove the Gemini CLI adapter, the login through a private PTY and the IPC for
API keys. In `antigravity.rs`, adapt the native stream input and output of agy
only to V1. Keep `conversation_id` in `agent_session`. To resume, use
`--conversation`. The native history is in agy, and the app owns the V1
transcript for the presentation. Each time Prometeu starts or resumes the
process, send the absolute worktree path through `--add-dir`. The cause is that
agy gets the workspace state from its native conversation, not from the cwd of
the process. Read the model catalog from `agy models`. Do not translate
Gemini CLI aliases, and do not add effort values that agy does not have. To stop
the child process group, send SIGINT.

Show only the capabilities that Prometeu examined and found correct. Headless
agy does not accept interactive approval responses. Its own policy rejects the
tools that must have an approval, but the turn continues. Usual conversations
and Auto task profiles use `--dangerously-skip-permissions`. The owner of the
product gave approval for this default after agy rejected a command in the
default mode during a test with the account. Headless agy cannot get an interactive approval. Task profiles
with the "Ask" permission continue to keep the CLI policy. The setting applies when Prometeu
starts or resumes the process. Prometeu does not change the global agy
preferences.

These items are not available:

- changes to and from plan mode
- MCP servers, plugins and skills that the app selects
- compaction and context reports
- structured questions

If the hub selection is not empty, the launch stops with an error that the
person can see.

Keep these shared account functions in Settings and in the footer:

- the account cards
- the explicit selection
- the focus on the new card
- the confirmation before the removal of the active account

Antigravity has one account that agy manages, not Prometeu. Prometeu attaches it explicitly as `antigravity` with the method
`external`. The attachment records the account name only. It does not
prove the authentication. The app does not read tokens, guess the email address or copy
the global keyring.

The quotas come from the native TSV report of `agy -p /usage`. Prometeu reads
this report one time each minute. It changes the percentages that are
available into canonical percentages that the person used. Each model group
keeps its own label and the time when its quota starts again. If a read gives
an error or incorrect data, Prometeu keeps the previous snapshot. The person logs in and changes accounts
in the official agy UI. Its authentication has no documented interface for an
isolated profile or for a login that Prometeu starts. A different HOME value alone does not
isolate the keyring. As a result, Prometeu does not manage many Google accounts
and does not start a login in the browser.

When Prometeu writes the registry again, it keeps the account values and the
selections of unknown providers. It does not show them through IPC. Retired
Gemini board identities stay as tombstones that Prometeu can read, but they
refuse execution. Prometeu does not use native IDs as agy or Claude IDs without
a message. Prometeu does not delete the credentials, CLI installations or
history from before this change. Binaries that Prometeu released before the
registry repair continue to stop with an error when they find unknown
accounts.

## Alternatives and consequences

ACP does not operate with this account. Model APIs without the CLI make a second copy of the tools,
the authentication and the context. A wrapper around the terminal UI must have a
parser for permissions and input that is not stable. Native NDJSON gives text
and tools that operate correctly. But it cannot give the full account and
approval scope of the initial issue. One external account cannot isolate the
identity from a change that a person makes in a different program. The relay format
and the relay implementation do not change.

## Evidence

- [Google transition notice](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/).
- [Official headless protocol](https://www.antigravity.google/docs/cli/headless/).
- `antigravity.rs`: tests of the adapter, native fixtures and the launch
  policy. These tests include the explicit worktree scope when Prometeu starts
  and resumes the process.
- `src-tauri/src/accounts.rs` and `src-tauri/crates/core/src/board.rs`: compatibility tests for retired accounts and board identities.
- `e2e/accounts.spec.ts`: external attachment, selection, focus and removal.
- [Provider matrix](../quality/provider-matrix.md): the limits of the
  verification.
