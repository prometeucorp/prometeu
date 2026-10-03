# ADR 0052 — Replace Gemini CLI with Antigravity

Date: 2026-09-19
Status: Accepted

## Context

The installed Gemini CLI rejects the user's consumer Google account because
Google discontinued that client for individuals. Antigravity CLI (`agy` 1.2.7)
accepts the same existing account and answered a real headless prompt. Merely
renaming the ACP adapter would not work: agy has a different NDJSON protocol.
This decision replaces retired ADR 0051.

## Decision

Remove the Gemini CLI adapter, private-PTY login and API-key IPC. Adapt agy's
native streaming input/output exclusively to V1 in `antigravity.rs`. Keep
`conversation_id` in `agent_session`; resume with `--conversation`. Native
history belongs to agy, while the app owns the V1 presentation transcript.
Pass the absolute worktree through `--add-dir` on every process creation and resume
because agy restores workspace state from its native conversation instead of process cwd.
Read the model catalog from `agy models`; do not translate Gemini CLI aliases
or invent effort levels. Interrupt the child process group with SIGINT.

Expose only verified capabilities. Headless agy does not accept interactive
approval responses; tools needing approval are soft-denied by its own policy.
Ordinary conversations and Auto task profiles use `--dangerously-skip-permissions`.
This default was explicitly approved after a real default-mode command was denied:
headless agy cannot obtain interactive approval. Explicit Ask task profiles still
preserve the CLI policy. The setting applies on process creation and resume,
without rewriting global agy preferences.
Plan transitions, app-selected MCP/plugins/skills, compaction, context reports
and structured questions are unavailable. Nonempty hub selections fail visibly.

Keep shared Settings/footer account cards, explicit selection, focus on the new
card and confirmation for active removal. Antigravity exposes one externally
managed account, attached explicitly as `antigravity` with method `external`.
Attaching records a reference, not proof of authentication. The app does not
read tokens, infer email, or duplicate the global keyring. Quotas come from the
native `agy -p /usage` TSV report, polled once per minute. Its remaining
percentages become canonical used percentages; labels and reset times remain
per model group. Failed or malformed reads preserve the previous snapshot.
Login and switching accounts remain in the official agy UI. Its authentication
has no documented isolated-profile or direct-login interface; HOME redirection
alone would not isolate the keyring. Managed multiple Google accounts and a
browser-only login initiated by Prometeu are therefore not delivered.

Preserve unknown-provider account values and selections when rewriting the
registry, without exposing them through IPC. Retired Gemini board identities
remain readable tombstones and refuse execution; native IDs are never silently
reinterpreted as agy or Claude. Old credentials, CLI installations and history
are not deleted. Previously distributed binaries without the registry fix
remain incompatible with unknown accounts.

## Alternatives and consequences

Keeping ACP fails with this account. Direct model APIs would duplicate tools,
authentication and context. Wrapping the terminal UI would require an unstable
permission/input parser. Native NDJSON provides working text and tools, but
cannot offer the complete original issue's account and approval scope.
The single external account cannot promise identity isolation against a change
made outside Prometeu. No relay format or implementation change is needed.

## Evidence

- [Google transition notice](https://developers.googleblog.com/an-important-update-transitioning-gemini-cli-to-antigravity-cli/).
- [Official headless protocol](https://www.antigravity.google/docs/cli/headless/).
- `antigravity.rs`: adapter, native fixtures and launch-policy tests, including
  explicit worktree scope on creation and resume.
- `src-tauri/src/accounts.rs` and `src-tauri/crates/core/src/board.rs`: retired account/board compatibility tests.
- `e2e/accounts.spec.ts`: external attachment, selection, focus and removal.
- [Provider matrix](../quality/provider-matrix.md): verification limits.
