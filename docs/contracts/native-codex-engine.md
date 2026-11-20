---
type: Contract
title: "Native Codex engine"
description: "Private app-server ownership, exact thread configuration and native TUI attachment."
status: stable
---

# Native Codex engine

The macOS terminal transport uses this private engine for `codex-cli 0.169.0`
and newer. 0.169.0 is the verified version; the hook bridge rejects older
releases. Newer releases are not pinned out because Codex ships often: the
bootstrap below verifies the socket, peer, thread identity and worktree and
fails closed if the private protocol changed.
Terminal mode is selectable in Settings / General. Ask and Plan use verified
native thread settings; bootstrap and consumer tests passed. Native CLI evidence
below is separate from the verified macOS Tauri response, history, cold-resume
and engine-shutdown cases recorded in the provider matrix.
See the [provider matrix](../quality/provider-matrix.md) and
[transport identity decision](../decisions/0100-conversation-transport-identity.md).

## Session ownership

`terminal_agent_codex::NativeEnginePlan` owns an unstarted app-server command.
Preparation captures the account, worktree, selected model/effort, instructions,
MCP environment and derived plugin home. The hook bridge configures this command
before it starts. The TUI receives the same captured environment and worktree;
semantic invocation settings belong to the server.

The plan starts `codex app-server --listen unix://<private>/rpc` in its own
process group. The directory is created with mode `0700`. Codex publishes an
alias to a SHA-256 named endpoint in its own private socket directory. The client
verifies the alias mapping, directory ownership/mode, socket ownership/mode and
the kernel-reported peer PID against the owned server. It uses WebSocket framing
over Unix with no TCP listener, compression or TLS configuration.

Bootstrap is ordered, bounded by one 20-second deadline, and completes before
the TUI process can receive input:

1. Initialize one honestly named `prometeu-native` RPC client.
2. Start a fresh thread or resume the exact saved UUID. Resume never falls back
   to another thread. Check the returned identity and worktree.
3. Give a fresh thread the native title `Prometeu`. Codex otherwise leaves an
   empty thread unpersisted, preventing native remote resume. Naming sends no
   model request.
4. Apply the selected approval policy, sandbox, model, reasoning effort and
   collaboration mode through `thread/settings/update`.
5. Read the current settings and establish the subscription through a correlated
   `thread/resume` with `excludeTurns: true`. Verify every requested setting.
   A no-op settings update emits no notification, so notification arrival alone
   cannot acknowledge this step.
6. Read `thread/turns/list` in ascending pages with full items. Commit missing
   native occurrences to the private ingestion journal before seeding the
   canonical stream; this recovery emits no live effects.
7. Start `codex --remote unix://<private>/rpc resume <exact UUID>` in the PTY.

The TUI receives no permission or sandbox overrides: remote resume rejects them.
App-server public config starts with a supported policy; the per-thread RPC
sets `untrusted` for Prometeu Ask and `never` for Auto/previous bypass behavior,
both with `dangerFullAccess`, matching the existing structured adapter. Plan
uses the native collaboration mode and native built-in plan instructions.
There is no synthetic user `/plan` prompt or dummy inference turn.

## Lifetime and explicit control

`NativeEngine` retains one owner for the child, socket, reader and cleanup. The
host retains it through PTY reaping. Shutdown closes the connection, terminates
the owned process group, escalates after one second if needed, waits for the
child and removes only its private alias and verified physical endpoint/lock.
Startup failure uses the same cleanup. A lost socket or saturated event queue
invalidates the engine; the host must also invalidate its PTY.

Tauri's `RunEvent::Exit` invokes explicit engine shutdown before process exit;
dropping AppState is not the shutdown contract. A private weak registry retains
access to each engine's owning worker even while a conversation or input port
still references it. Exit denies new engine starts, waits for already admitted
bounded bootstraps to register or fail, signals all workers and joins them before
returning. Concurrent stop and exit share the same join gate, so neither returns
while another caller is still reaping that engine. A subprocess regression keeps
live handles and an in-flight bootstrap while checking child reaping, endpoint
removal, repeated shutdown and rejection of a new spawn.

Derived `CODEX_HOME` configurations do not share `app-server-control`. Codex 0.169
rejects a symlink at this runtime-directory path. Materialization removes an old
derived alias without following it and lets Codex create its private directory;
existing physical control directories and original-home endpoints are preserved.
Authentication and durable history retain their existing sharing rules. The tools
compatibility regression checks both old-link migration and repeated preparation.

The wire accepts messages up to 1 MiB, retains at most 64 startup messages and
8 MiB during bootstrap, and exposes a 64-message live channel. Outbound control
has a 32-entry queue and a five-second deadline. Overflow fails explicitly.
History additionally has a 64 MiB/10,000-turn ceiling, 16 turns per page, and
shares the startup deadline. Repeated native IDs or malformed pagination fail
before starting the TUI. These limits protect process memory; they are not claims
that arbitrary provider payloads fit. The host continuously drains the live
channel; loss invalidates both the engine and PTY.

The attached protocol input permits only `account/rateLimits/read`, an interrupt
for this exact thread, and an explicit response to an observed pending native
request. It rejects initialization, thread creation/resume and `turn/start`.
The TUI owns text, slash commands and native interactions. Unknown server
requests receive no automatic error or decision. The server can settle a request
through the TUI first; `serverRequest/resolved` invalidates the app's pending ID.
Successful writing proves delivery to the connection, not acceptance of a
decision that another client already settled.
The attached decoder retains a delivered request until native resolution, while
rejecting further answers and duplicate request delivery. This lets the eventual
`serverRequest/resolved` close the same card after an app answer as after a TUI
answer; delivery itself does not manufacture a closure.

`prometeu_protocols::codex::Link::attached` reuses the existing event decoder
without running its startup handshake. It rejects canonical message sends,
preserves native assistant item IDs across streaming and final blocks, and
handles explicit request responses and interrupts. Native resolution closes the
card with outcome `resolved`; the notification does not disclose whether the
person allowed, denied or answered it. No decision outcome is inferred. Public request IDs include a fresh engine nonce
and a typed native-ID digest. A reply from an earlier engine cannot answer a
new request that happens to reuse the same JSON-RPC ID after resume.

The desktop `chat/terminal/codex.rs` consumer uses RPC as the single canonical
source. Final user/item/turn records have native occurrence receipts; deltas and
non-content notifications pass through the ordinary pump. Bootstrap paginated
history uses the same IDs and projection. The rollout is evidence, not a second
publisher. Paginated Codex rollouts persist `item_completed` rather than the
legacy `user_message` event, so the legacy transcript decoder cannot supply this
recovery path.

App input prepares its durable receipt before PTY writes. Its native user item is
matched against that FIFO causal boundary, including the preceding native user
and checked content hash; text equality is never its identity. Manual native
input is captured through the ordinary app input/attribution path. A pending app
capture holds RPC publication until its receipt commits. Native `clientId` is
preserved privately; the TUI generates it and the app does not invent it.

Final assistant blocks and tools retain native item IDs. A completed turn has a
thread/turn receipt, so live delivery and cold replay cannot independently charge
or notify the same completion. Historical active tools never synthesize a result;
partial assistant text is only a content snapshot. RPC turn activity holds app
input admission until its ordered completion has committed, even if PSP reports
Done first. Hooks/rollout do not independently publish Codex content or completion.

Request answers and exact-thread interrupts route through the attached decoder;
text and slash commands remain native terminal input. Quota reads and updates use
the existing canonical account-usage path. A native settings update changes the
decoder's observed model/effort; explicit app selection is reapplied and verified
on resume before input is admitted. Native `thread/status/changed` idle and a
committed completion of the current turn provide input-readiness authority in
addition to PSP; unknown/active/permission states invalidate it. A painted empty
composer is still required.

## Evidence and limits

`terminal_agent_codex/tests.rs` uses an owned fake WebSocket server process to
check startup ordering, identity/configuration rejection, queue failure,
explicit-only replies, stale IDs, forbidden turn creation and child reaping.
`crates/protocols/src/codex/attached_tests.rs` checks the existing decoder's
attached behavior while retaining the structured adapter's compatibility suite.

The ignored `terminal_agent_codex/smoke.rs` tests passed on Codex 0.169.0 using
an empty private worktree and a private copy of existing external authentication.
Fresh start and cold server restart/resume both rendered native Plan, preserved
Ask/effort/exact identity and left the same TUI alive, with no model turn or tool
request. A separate one-turn, no-tool smoke drives the native compositor, feeds
the actual RPC frames through the runtime consumer and compares cold native
history replay: one user, one completion and unchanged canonical bytes.
The no-inference app-conformance test accepts an explicit owned fixture root
through `PROMETEU_CODEX_ENGINE_APP_PROOF` and an optional exact persisted tab ID
through `PROMETEU_CODEX_ENGINE_APP_TAB`. The latter resolves that board tab's
workspace settings, tools and account through the production preparation and
hook bridge, then verifies cold resume without sending a model turn. It does not
fall back to a new thread or scan unrelated native sessions.
`chat/terminal/codex/tests.rs` also checks receipt ordering, repeated input text,
tool identity and active-tool crash tails. Sanitized views and identity evidence
are in `terminal_agent_codex/fixtures/`. The separate ignored
`terminal_agent_codex/decision_smoke.rs` test passed one real Plan question:
the native numbered options and submit footer were visible before the attached
adapter answered Blue; native resolution closed the canonical card and the
completed model response was exactly Blue. A repeated app answer was rejected.
The engine, TUI and screen helper were then closed and reaped. Its fixture records
only conformance booleans and the fixed answer. The command case also passed
with Ask (`untrusted`) and Plan disabled: after the native approval heading,
exact command and one-shot choice were visible, the attached adapter explicitly
allowed `printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt` once. The
test validated the scratch working directory, matching native request resolution,
command completion with exit code zero, exact marker file contents and final
assistant response; a repeated answer was rejected. Its allowlist rejects extra
shell commands and writes outside that scratch-relative file. Both native
processes and the screen helper were reaped. This does not
verify native file-change approval, plugins/MCP behavior, sharing,
quota accounting or full Tauri application startup. Non-text native inputs beyond
image metadata still require explicit conversion; unknown input variants fail
instead of silently losing content. No global native configuration changes
or login flow are part of the test.

The upstream remote TUI does not enable its special device `userVerification`
ceremony. Upstream grants that extension only to its embedded TUI and its local
desktop connection; Prometeu does not spoof those client names or origins.
This remains a native feature difference even when ordinary approvals are
available through the remote TUI.

The implementation was checked against these upstream sources at
[`rust-v0.169.0`](https://github.com/openai/codex/tree/rust-v0.169.0):

- [Unix socket transport](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server-transport/src/transport/unix_socket.rs)
  defines the guarded alias and WebSocket endpoint.
- [Thread RPC contracts](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server-protocol/src/protocol/v2/thread.rs)
  define settings, resume and collaboration mode overrides.
- [Thread processor](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_processor.rs)
  defines native materialization and subscription semantics.
- [TUI app-server session](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/app_server_session.rs)
  preserves the server's permission settings on remote resume.
- [Outgoing requests](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/outgoing_message.rs)
  and [initialization](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/request_processors/initialize_processor.rs)
  define decision eligibility and device-verification ownership.
