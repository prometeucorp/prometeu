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
(`cursor_login`), `session/new` or `session/load` with the complete client
`mcpServers` set, optional `session/set_model`,
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

Streaming, tools, resume, stop, initial plan mode, approvals, questions, local
path mentions and hub MCP are available. Plugins, skills, managed multiple
accounts, per-turn token/cost reporting, context reports and explicit compaction
are not. Absent plugin/skill workspace selections resolve to empty hub
selections, so global and trusted-project packages cannot block startup or
resume. Explicit nonempty plugin or skill selections still fail rather than being
silently dropped. Hub MCP follows the same layered resolution as Codex: `None`
sends no client servers; a declared axis materializes the resolved set through
ACP `mcpServers` on both `session/new` and `session/load`. Stdio, HTTP and SSE
hub entries encode through the ACP transport table; Claude-only connectors fail
startup. Cursor may still load its own project/user rules, skills, hooks and MCP
configuration; Prometeu makes no isolated-configuration claim. Team-level MCP
support is limited by Cursor itself.

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

## Native interruption evidence

The native terminal host's versioned control uses the current `ctrl+c to stop`
composer hint and observed active execution. It sends Ctrl+C only once per
active cycle, because another Ctrl+C can exit the CLI. The startup banner may
scroll away; current composer alignment, footer rows and physical cursor must
still match. A draft that hides the hint does not authorize a synthetic Stop.

Local keyboard input keeps Cursor's own bindings. In the pinned
`2026.11.13-eacade1` source, `src/components/prompt-bar.tsx` also stops on Escape
when the composer is empty and no overlay, history or queued-message action
consumes it. That path and Ctrl+C reach the same cancellation callback; the
application's Stop control continues to use the advertised Ctrl+C binding.

The spinner can start before Cursor has persisted the new turn. A native Enter
written while idle records an input boundary but does not itself mark the agent
working. Only a newer, complete paint matching the pinned busy composer,
`ctrl+c to stop` footer and physical caret publishes observed Working. This
creates no user message; durable source capture later claims the same execution
cycle. Paste, Enter without a busy paint and an old screen cannot start work.
After success or interruption, a late busy paint cannot reopen that cycle.
`cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`,
`cursor_working_observation_requires_its_current_complete_busy_composer` and
`observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once` cover
the host, versioned screen profile and execution projector respectively.

The opt-in `terminal_agent_controls/smoke.rs` passed on 2026.11.13-eacade1 with
an owned scratch workspace and private writable CLI settings. It waited for the
submitted prompt, model-generated counting output and live Working/footer,
then sent one Ctrl+C. The authenticated root Stop included `aborted`; any later
Stop reasons are preserved separately. The process remained alive, restored the
interrupted prompt as a native draft, and was reaped after the proof. This is
an interruption check, not a completion proof or a new OSC status claim.
Captured busy, scrolled and interrupted screens exercise the control profile in
`terminal_agent_controls/tests.rs`; no shell command or file tool was requested.

The authenticated root `stop` / `aborted` hook settles the canonical turn with
outcome `interrupted` only after its `generation_id` matches the current durable
source tail's `request_id` and ingestion is caught up. A newer prompt or app
submission invalidates an older proof, including while the new prompt has not
yet reached the store. The prompt-hook generation is only a barrier for input
not yet captured: durable acknowledgment of its exact request clears it. A
subsequent source-authorized request can then settle even if no redundant
`beforeSubmitPrompt` hook arrived. Stop candidates are bounded and retained by
request identity, so a delayed hook for an older request cannot replace the
current request's evidence. A later `completed` hook for the cancelled generation
cannot replace its interruption. The native CLI can emit both: its abort path
starts the `turn_ended` / `aborted` write before calling the hook, while its
normal-return path can subsequently call `stop` / `completed` but retain the
aborted marker. Interruption therefore uses the authenticated cancellation and
source identity, not the success path's post-ack file-change fence or optional
run statuses.

The host journals the interrupted outcome before publication and settles the
spinner even when Cursor restores a nonempty prompt draft. It does not require
an empty composer, send that draft or infer cancellation from an Escape byte.
Late source content and tool hooks from the cancelled request cannot reopen
work; the content remains in history. The interrupted outcome creates no
successful-reply notification. Observer cancellation tests and
`journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet` exercise
these boundaries with the real portable pump.

## Native decision-channel limit

The first terminal version intentionally uses Cursor's native TUI for approvals,
questions and plan decisions. Chat mode keeps its existing ACP request cards.
This product choice preserves Cursor's own interaction and is not an outstanding
requirement to recreate those controls in Prometeu for the current native mode.
The other terminal conformance and integration requirements still apply.

Application request cards cannot answer these requests; an explanatory hint
points to the native terminal. Shared views receive `decision_surface: "terminal"` inside
the encrypted announcement, show that the owner must answer, and do not offer
response actions even to members with Control. The owner and native backend
also reject stale or legacy decision responses. Messages, verified interruption,
notes and native keyboard interaction remain available under their existing
rights. No request is invented from a blocked status or a stored tool call.

The terminal control audit is pinned to CLI `2026.11.13-eacade1`. It found no
published channel that can answer the existing TUI's pending approvals or
questions while leaving the native dialog available. The current transport
therefore continues to reject canonical decision commands for Cursor; it must
not turn a stored tool call into an answerable request.

The native hooks are sequential pre-execution gates, unlike Claude's parallel
PermissionRequest dialog hook. In installed `4327.index.js`, the
`../hooks-exec/dist/index.js` wrapper awaits `preToolUse` and the specialized
pre-execution hook before invoking the underlying executor. `beforeShellExecution`
also waits for its hook, then turns `permission: ask` into a `FORCE_PROMPT`
requirement consumed by the native shell permission service. Waiting for a remote
answer inside either hook prevents that same operation from reaching its native
approval dialog. Returning allow merely continues native policy evaluation; it
is not a response to an already-open native prompt. The Claude compatibility
mapper explicitly maps `PermissionRequest` to unsupported. The public
[hook contract](https://cursor.com/docs/hooks) describes these policy hooks and
their output and failure behavior; no neutral timeout is adopted as an approval
by Prometeu.

The interactive implementation keeps pending approval resolvers in
`./src/pending-decision-store.ts` inside `2986.index.js`. Its React consumer calls
`approveDecision`/`rejectDecision` in the same process. Root questions and mode
switches use React-held query IDs and promise resolvers; no external endpoint
was found for these objects. The installed `index.js` implementation of
`agent persist attach` attaches a tmux terminal, which forwards input rather
than exposing typed request identities.

[ACP](https://cursor.com/docs/cli/acp) provides permission, question and plan
responses for an ACP-owned session. Its `./src/acp/run.ts` in `8104.index.js`
constructs a separate agent runner over stdio; it is not an attachment to the
running TUI. Starting a second ACP session would create another execution
authority. The [SDK Bridge](https://cursor.com/docs/sdk/bridge) similarly embeds
the SDK and owns its own agents; it is not an advertised TUI-control channel.

These findings are source and documented-interface evidence, without a new
model request or global settings change. They do not prove that a future CLI
cannot expose a suitable channel. Sending menu keys, instrumenting private
React state, or replacing the native dialog would require a separate design
and conformance proof; none is silently used as a permission response.

`conversation-capabilities.test.ts`, `team-viewer.test.ts`, `team-owner.test.ts`,
`team-channel.test.ts`, relay protocol compatibility tests and
`chat::terminal::tests::native_only_decisions_reject_application_responses_without_disabling_native_input`
cover this boundary without changing the global Cursor catalog or Chat behavior.

Reproduction uses the installed module names above and these SHA-256 values:

| Installed bundle | SHA-256 |
| --- | --- |
| `index.js` | `92ce160b560396971bf318f9e9f6c2fde8810c36e9f2aad68b7ac2b81af2ddee` |
| `2986.index.js` | `deb07a87c5c16c31c0429d41320fa1ea2f6fd29bc14250f461969940b0fb365b` |
| `4327.index.js` | `3815ff2df5531bde1f725fafbb2ffcc39b5721c6a83690c7dfb2c0ab70c4dc0c` |
| `8104.index.js` | `6605d145643e8190df2b5250bbef41a54d96473baab88c49e3dc9c0915a85e0f` |

## Native-store recovery

The macOS terminal recovery edge is separate from the Chat ACP path, as defined
by [ADR 0100](../decisions/0100-conversation-transport-identity.md).
Native instructions use a private `beforeSubmitPrompt` hook's
`additional_context` response. In the pinned CLI this carrier runs before each
native submission, including resumed sessions; `sessionStart` alone would miss
resume. The private hook returns only context, leaving other hooks and native
permission policy in force. Instructions are data in a private JSON file, read
by `/bin/cat`, and never become shell syntax or visible prompt text. The native
10,000 UTF-16-unit carrier limit is checked before launch. A controlled native
smoke verified fresh/resume with changed instructions and the same user prompt;
unit coverage checks quoting, file lifetime, permissions and the Unicode limit.
See the native [hook contract](https://cursor.com/docs/hooks) and the versioned
implementation checks in `terminal_agent_hooks.rs` and its opt-in smoke.

Cursor CLI `2026.11.13-eacade1` resumes from
`<captured-config>/chats/<md5(physical-cwd)>/<native-session>/store.db`.
The preparer resolves the existing workspace once before launch, preserving the
board's original path, and uses that same physical path for the PTY cwd,
`--workspace`, store hash and transcript slug. The native store hashes
`process.cwd()`, which resolves filesystem aliases such as macOS `/tmp` to
`/private/tmp`; hashing only the lexical board path can hide a successfully
written native response. The preparation regression covers a directory symlink
followed by `..` on both fresh and resumed launches, without searching histories.
The caller supplies this exact authorized session directory; the reader never
searches sessions or opens the editor database. Its pinned root and no-follow
opens require regular, singly linked files owned by the current user.

`terminal_agent_cursor_store.rs` copies the database and its WAL into a private
staging directory, at most 4 MiB per poll, and rejects a copy if either source
changes. SQLite opens only that copy. The original shared-memory file is never
opened; `immutable` is not used to bypass the WAL. The copy is removed on drop.
Source database `user_version: 1`, exact metadata `agentId`, and a CLI at or
above the verified `2026.11.13` decoder floor are all required. Older or
unparseable versions fail explicitly; newer ones are decoded with the same
framing validation.

`protocols::cursor_store` follows only root-referenced blobs and verifies their
SHA-256 identities. Generated protobuf fields retain user message, request, run
and tool-call identities. A step index belongs to its native turn; the referenced
blob hash is its content version, so an updated answer does not become another
answer and repeated text in separate runs remains distinct. Compaction of the
derived JSONL does not identify or remove canonical history. Bounded wire reads
skip binary fields without allocating their payload while retaining user text;
malformed framing, missing blobs, identity conflicts and oversized required text
are explicit failures. The pinned tool descriptors are functional field metadata
in `cursor_store/tool_schema.json`, reproducible with
[`extract-cursor-store-schema.py`](../../scripts/extract-cursor-store-schema.py).
The extractor reads an explicitly selected CLI source file without executing it;
the artifact records its SHA-256 and CLI version.

Tool projection preserves native call IDs and actual arguments, strips native
approval/routing metadata, and captures results as separate occurrences from
their tool blocks. A mutable result updates the same tool; a pending tool does
not fabricate completion. Tool IDs are opaque bounded JSON strings: the verified
CLI can join native IDs with an LF, which is preserved and escaped in canonical
lines. They never select files or approval routes. MCP maps retain JSON values, textual output and
structured results. Read results follow authenticated content-blob references;
truncated calls follow and verify their original step, with a bounded reference
chain and matching native call identity. Binary tool output is represented by
its byte count and explicit omission rather than retained base64. Unknown tool
fields withhold coverage. Stored tool calls never create answerable requests.

The observer's opt-in `configure_cursor_store` path receives the captured
`chats/<cwd-hash>` directory and opens only the authenticated session child.
`set_ingestion` receives private `NativeIngestionBatch` values containing source
revision, replay/live phase and bounded occurrence/version records. A record
contains a canonical frame or a content-free checkpoint; none carries a vendor
payload into the core. User occurrence is its native message ID, independently
of request/run retries. Assistant steps retain their public `(messageId,index)`
across blob revisions, so authoritative updates append to the canonical log
while preserving the visual item's position.

The host acknowledges a batch only after every supported record is durably
committed. Until then the exact same batch may be delivered again. `caught_up`
requires a final acknowledgment and a subsequent unchanged-source check.
Resume batches remain replay until their complete bootstrap is acknowledged;
they never use the legacy live or accepted-prompt callbacks. `expect_submission`
binds a new native user occurrence to an opaque app submission by a bounded,
ordered causal queue; old occurrences and repeated text cannot consume later
submissions. It does not deduplicate history by text.

Unknown tools, shell-only turns and synthetic-user kinds without a projection explicitly
withhold coverage, even if later compacted out. Their no-frame checkpoints are
not proof of complete ingestion. No root snapshot, run status or missing activity
fabricates a completion event. The native host reconciles captured
occurrence/version receipts through its private ingestion journal: bootstrap
recovery is storage-only before Lines are seeded; live publication follows
durable commit. See [persistence](persistence.md#native-ingestion-journal-foundation).

The success fence correlates a live `stop` hook's `generation_id` with the
last fully ingested turn's native `request_id`. Before acknowledging that hook,
the authenticated listener captures metadata of its exact authorized transcript
through pinned directory handles and no-follow opens. That witness is host-only:
it cannot be supplied in the helper envelope or restored from a previous process.
The observer then requires a changed, stable file revision ending in a complete
`turn_ended` / `success` record. The pinned writer emits this marker only after
the Stop hook chain returns; ordinary writes remove a previous turn's marker.
A missing file before acknowledgment may become a regular authorized file later.
Read failures, partial records and an unchanged old marker do not establish the fence.

Native run records are optional in the verified TUI store. When present, their
identities and completed states must agree; missing records are not synthesized.
The host also checks the current empty composer and pending app-input transaction,
then journals completion before publishing or changing execution state. Composer
painting alone cannot bypass the post-hooks marker. Duplicate request receipts
do not repeat notifications, telemetry or completion effects. Cancellation uses
the separate authenticated interruption path above. Native errors and unsupported
background content remain separate conformance requirements.

Synthetic tests in `protocols/src/cursor_store/tests.rs` and
`terminal_agent_cursor_store/tests.rs` cover repeated inputs, compaction, new
runs, mutable answers, large binary context, hashes, WAL recovery and source
isolation, pending/result revisions, MCP values, read blobs, truncated calls,
binary limits and malformed tool unions. `terminal_agent_observation/cursor/tests.rs` covers acknowledgment,
quiet resume, repeated submissions and incomplete coverage; `src/timeline.test.ts`
covers late authoritative updates without moving existing positional keys.
`chat/terminal/ingestion/tests.rs` composes that journal with the real portable
pump to cover restart, a missing native tail, normalized input aliases and
failure before publication. Controlled native smokes validated the copied
store decoder for hook-authorized scratch sessions, including one Shell call and
its exact output. That call was explicitly approved once in the test's native
prompt; programmatic approval controls are outside the accepted native-mode
scope. Full Tauri app conformance remains distinct from these native smokes.
`terminal_agent_observation/cursor/tests.rs` and `terminal_agent_hooks.rs` also
cover pre-ack capture, stale and partial success markers, exact request identity,
new-input invalidation, duplicate hooks, symlink/hard-link rejection and durable
completion without repeated publication. The cancellation regressions
`cursor_interruption_requires_durable_request_tail_without_a_success_marker`,
`cursor_interruption_rejects_other_requests_and_new_app_input` and
`cursor_interruption_rejects_late_old_hooks_before_new_prompt_reaches_store`
cover the interrupted outcome, duplicate acknowledgment and stale request
boundaries independently of the success marker.
`cursor_restored_prompt_retry_settles_without_repeating_before_submit_hook`
checks a later source-confirmed request without a repeated prompt hook and
with delayed old Stop hooks. It is a synthetic ordering regression, not evidence
that a particular native retry skipped its hook. The controlled identity smoke confirmed
the hook/request match and the absence of optional run records for its own session.
A second controlled native run exercised the pre-ACK witness, copied source,
durable ingestion receipts, post-hooks marker and empty composer together, then
committed exactly one completion receipt. The headless harness uses the same
observer/journal path; it does not claim a Tauri UI or shared-control test.

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

`src-tauri/crates/protocols/src/cursor/tests.rs` covers translation, the MCP
session payload and protocol failures using a [sanitized recording](../../src-tauri/crates/protocols/src/cursor/fixtures/README.md).
Hub encoding lives in `crates/tools/src/tests.rs`. The generated V1 fixture is
consumed by `src/conversation.test.ts`. Account compatibility lives in
`src-tauri/crates/core/src/accounts/tests.rs`; model selection persistence lives
in `src/model-choice.test.ts`. Existing account and launcher browser journeys
cover reused controls; no additional provider matrix of browser tests is
introduced.

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
