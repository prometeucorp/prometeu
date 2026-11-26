---
type: Contract
title: "Cursor CLI"
description: "The ACP transport, the external account, the conversation lifecycle and the capability limits."
status: stable
---

# Cursor CLI

The implementation is complete on the macOS and Linux desktop. Decision:
[ADR 0095](../decisions/0095-cursor-acp.md).

## Installation and accounts

The discovery runs the `cursor-agent acp --help` command with a time limit. The launcher
and Settings use the current provider descriptor and account controls. To use
Cursor, attach the external CLI account. Then select its card.

`agent login` and Cursor control the login and the change of the identity
below the account. The quota queries read the current access token of the CLI.
They do not copy, refresh or change the credential store of the CLI. When a
person removes the account, the app removes the selection and does not log out.
The stored provider and the external account ID are `cursor`.

## Transport and lifecycle

The child process runs `cursor-agent acp` in the workspace folder. There is a
newline after each JSON-RPC 2.0 frame. The startup sends these requests:

1. `initialize` (protocol 1).
2. `authenticate` (`cursor_login`).
3. `session/new` or `session/load`, with the full client `mcpServers` set.
4. An optional `session/set_model`.
5. `session/set_mode`.

A resume keeps the mode that the native history reports. The initial plan flag
applies to new sessions. The first prompt waits until the startup is complete.
Each `session/prompt` contains text. Attachments continue to use the shared
path-mention contract. The adapter adds the instructions at the start one time
for each process. When the agent is busy, the shared session core keeps the
input in a queue. `session/cancel` stops the active prompt.

The app saves the native session ID in `Tab.agent_session`. Prometeu keeps the
V1 presentation in `chats/<session>.jsonl`. The adapter ignores the native
replay during `session/load`, because the replay makes duplicate messages.
An authentication, model, mode or resume error stops the work that is not
complete with a translated error. It also closes the input.

The desktop thinks that the connection is stopped also when the child process
continues to operate. The next user message replaces the connection through the
usual shutdown and resume lifecycle. The authentication runs again, and the app
keeps a saved native session ID. The native stderr diagnostics do not go into
the shared transcript. The app keeps the text that comes as an ACP assistant
chunk, and this includes errors that Cursor puts in that text. If a resume has a
failure, the app does not start a new session automatically. It is possible
that a native session without a prompt does not stay after the process exits.

The app does not parse ACP model IDs, which include bracket parameters. The model discovery
makes a temporary ACP session without a prompt through the private query
transport, which has limits. It reads `models.availableModels` and closes the session. The app does
not make models or effort levels from provider names or CLI aliases.

## Translation

Text and thinking chunks become streaming assistant blocks. The adapter
completes these blocks at tool boundaries and at the end of the turn. Tool calls
keep their native IDs. Updates supply the inputs, the output and the last
status. Tool output accepts ACP content blocks and the native `rawOutput` of
Cursor. Only the updates for the active session go into V1. A native last
response completes the turn one time. A cancel completes the turn with an
interrupted outcome. Unknown notifications do not change the conversation state.

Permission requests keep their JSON-RPC IDs for the replies. The presentation
IDs apply only to the turn. Thus, when the CLI starts again, a previous request with
the same native counter cannot hide a new card. Ask mode uses the current
approval card. Automatic execution uses the `allow_once` option that the agent
shows. A denial uses `reject_once`. If there is no reject option, a denial uses
a cancel. No approval changes the permanent allowlist of Cursor. Explicit CLI
deny rules stay in Cursor. Stop cancels the client requests that are not
complete, and also the prompt.

`cursor/ask_question` changes the prompts and options into the shared question
card. It changes the selected labels back to native IDs. The click sequence in a
multi-select does not change the result. The extension accepts only option IDs
as structured answers. Thus the app returns free text in the text field of a
skipped response. `cursor/create_plan` renders the plan text and waits for the current
plan approval controls. Unknown client methods receive `-32601`. They do not
stay pending.

## Capability limits

These capabilities are available:

- Streaming, tools, resume and stop.
- The initial plan mode, approvals and questions.
- Local path mentions and hub MCP.

These capabilities are not available:

- Plugins and skills.
- More than one managed account.
- Token and cost reports for each turn.
- Context reports and explicit compaction.

If a workspace does not select plugins or skills, the hub selections are empty.
Thus global packages and packages of a trusted project cannot stop the startup
or the resume. An explicit plugin or skill selection that is not empty
continues to cause a failure. The app does not discard it without a notice.

Hub MCP uses the same layered resolution as Codex. `None` sends no client
servers. A declared axis sends the resolved set through ACP `mcpServers` on
`session/new` and on `session/load`. Stdio, HTTP and SSE hub entries use the
encoding of the ACP transport table. Connectors that only Claude can use cause a
startup failure. Cursor can continue to load its own project and user rules,
skills, hooks and MCP configuration. Prometeu does not claim an isolated
configuration. Cursor sets the limits of MCP for teams.

## Account quotas

`cursor/quota.rs` queries the internal dashboard API of Cursor. It does not use
ACP and does not cause inference. It uses the current account scheduler, the
cache of previous values, the backoff and the generation checks for removal and
reconnection. The current `{ windows, at }` quota contract and the IPC do not
change. It is not necessary that the account sends a message
first.

The authentication uses `CURSOR_AUTH_TOKEN` when a person sets it explicitly. An
API key in the environment or a CLI credential store in memory does not use a
saved subscription. If not, the app reads these locations:

- On macOS, the `cursor-access-token` Keychain service for `cursor-user`. A
  private subprocess does this read, with a limit of five seconds.
- If `AGENT_CLI_CREDENTIAL_STORE=file` is set explicitly, `~/.cursor/auth.json`.
- On Linux, `$XDG_CONFIG_HOME/cursor/auth.json` or `~/.config/cursor/auth.json`.
  The app does not use relative Linux configuration roots.

The app does not read browser cookies or the database of the Cursor editor.
Tokens stay in memory. The app does not refresh, log or store them.

The subject and the expiry of the JWT make a `WorkosCursorSessionToken` cookie.
The app sends this cookie only to `https://cursor.com`. HTTP redirects are
disabled. Each request has a timeout of ten seconds and a response limit of 1
MiB. If a credential is expired, malformed or rejected, the quota data stays
unavailable or keeps the previous value. Cursor owns the renewal of the login.
As with other external profiles, a change of the CLI login that Prometeu does not make does
not give a continuous managed identity.

`POST /api/dashboard/get-sand-usage-status`, with an empty JSON body and a
the same Origin, identifies the quota type. An explicit `isUnified: true` uses
the shared weekly `usagePercent` and `nextResetTimestampUtc`.
The app uses `includedLimitZero` first, before the previous
`hasNonZeroIncludedLimit` allowance flag. The app continues to show paid allowances that are fully used. If a weekly
value is missing, malformed, expired or has a failure, the app does not show an
incorrect monthly value of 0.

Only an explicit `isUnified: false` enables `GET /api/usage-summary`. The app can show some
individual included plans. For these plans, it uses the start of the next
billing cycle and the reported total, Cursor and API percentages. Pools that the
response does not include stay missing. The app does not guess team budgets,
unlimited plans or trial grants that do not occur again. On-demand spend is not
part of the quota meter. For an unknown response, the app keeps the previous
value. It does not show 0%.

These endpoints are internal, and they can change independently of ACP.
[CodexBar](https://github.com/steipete/CodexBar/blob/main/docs/cursor.md) also
documents this method. The live quota check examines the current signed-in
account without inference. Synthetic fixture tests examine the monthly map and
the failure processing.

## Native interruption evidence

The versioned control of the native terminal host uses the current
`ctrl+c to stop` composer hint and the active execution that the host sees. It sends
Ctrl+C only one time in each active cycle, because a second Ctrl+C can stop the
CLI. The startup banner can scroll out of the screen. The current composer
position, the footer rows and the physical cursor must continue to agree. A
draft that hides the hint does not let the host send a synthetic Stop.

Local keyboard input keeps the bindings of Cursor. In the pinned
`2026.11.13-eacade1` source, `src/components/prompt-bar.tsx` also stops on
Escape in this condition. The composer is empty, and no overlay, history or
queued-message action uses the key. That path and Ctrl+C go to the same cancel
callback. The Stop control of the application continues to use the Ctrl+C
binding that Cursor shows.

The spinner can start before Cursor stores the new turn. A native Enter that the
host writes when the agent does not operate records an input boundary. It does not mark
the agent as Working. Only a newer complete paint publishes the Working state
that the host sees. That paint must agree with the pinned busy composer, the
`ctrl+c to stop` footer and the physical caret. This does not make a user
message. Subsequently, the durable source capture claims the same execution cycle.

Paste, Enter without a busy paint, and a previous screen cannot start work. After a
success or an interruption, a late busy paint cannot open that cycle again.
These tests examine the host, the versioned screen profile and the execution
projector:

- `cursor_busy_paint_starts_before_source_and_restored_retry_reopens_after_escape`
- `cursor_working_observation_requires_its_current_complete_busy_composer`
- `observed_native_input_reopens_a_settled_cycle_and_source_claims_it_once`

The opt-in `terminal_agent_controls/smoke.rs` had a correct result on
2026.11.13-eacade1. It
used a scratch workspace that it owned and private CLI settings that it can
write. It waited for the prompt that it sent, a counting output from the model and
the live Working state and footer. Then it sent one Ctrl+C. The authenticated
root Stop included `aborted`. The test keeps the values of subsequent Stop hooks in a
different record. The
process stayed alive and put the interrupted prompt back as a native draft. The
test stopped the process after the proof.

This smoke test is an interruption check. It is not a completion proof or a new
OSC status claim. Captured busy, scrolled and interrupted screens examine the
control profile in `terminal_agent_controls/tests.rs`. The test did not request
a shell command or a file tool.

The authenticated root `stop` / `aborted` hook completes the canonical turn with
the `interrupted` outcome. It does this only in this condition:

- Its `generation_id` agrees with the `request_id` of the current durable
  source tail.
- The ingestion has no data that is not processed.

A newer prompt or app submission cancels an older proof. This also applies
while the new prompt is not in the store at that time. The generation of the prompt hook
is only a barrier for input that the host did not capture at that time. The durable
acknowledgment of its request removes the barrier. Then a subsequent
request that the source authorizes can complete, also if no new
`beforeSubmitPrompt` hook came.

The host keeps a maximum number of Stop candidates by request identity. Thus a
late hook for a previous request cannot replace the evidence of the current
request. A subsequent `completed` hook for the cancelled generation cannot replace
its interruption. The native CLI can send the two hooks. Its abort path starts
the `turn_ended` / `aborted` write before it calls the hook. Its usual return
path can then call `stop` / `completed`, but keep the aborted marker. Thus the
interruption uses the authenticated cancel and the source identity. It does not
use the post-ack file-change fence of the success path or the optional run
statuses.

The host records the interrupted outcome in its journal before the publication.
It stops the spinner also when Cursor puts back a prompt draft that is not empty.
An empty composer is not necessary for the host. It does not send that draft, and it
does not think that an Escape byte is a cancel. Late source content and tool
hooks from the cancelled request cannot open the work again. The content stays
in the history. The interrupted outcome does not send a notification of a
correct reply. The observer cancel tests and
`journal_cursor_interruption_stops_spinner_and_keeps_late_source_quiet` examine
these boundaries with the production portable pump.

## Native decision-channel limit

The first terminal version uses the native TUI of Cursor for approvals,
questions and plan decisions. This is intentional. Chat mode keeps its current
ACP request cards. This product decision keeps the interaction of Cursor. For
the current native mode, Prometeu must not make those controls again.
The other terminal conformance and integration requirements continue to apply.

The request cards of the application cannot answer these requests. A hint tells
the person to use the native terminal. Shared views receive
`decision_surface: "terminal"` in the encrypted announcement. They show that the
owner must answer. They do not show response actions, also to members with
Control. The owner and the native backend also reject decision responses that
are not current or that use a previous format. Messages, interruption with a
proof, notes
and native keyboard interaction stay available with their current rights. The
app does not make a request from a blocked status or a stored tool call.

The audit of the terminal control is pinned to CLI `2026.11.13-eacade1`. The
audit found no published channel that can answer the pending approvals or
questions of the current TUI and keep the native dialog available. Thus the
current transport continues to reject canonical decision commands for Cursor.
It must not change a stored tool call into a request that a person can answer.

The native hooks are sequential gates before the execution. Claude has a
different design: its PermissionRequest dialog hook operates in parallel. In the
installed `4327.index.js`, the `../hooks-exec/dist/index.js` wrapper waits for
`preToolUse` and the special pre-execution hook. Then it calls the executor.
`beforeShellExecution` also waits for its hook. Then it changes
`permission: ask` into a `FORCE_PROMPT` requirement for the native shell
permission service.

If a hook waits for a remote answer, that operation cannot open its native
approval dialog. An answer that lets the operation continue only continues to
apply the native policy. It is not a response to a native prompt that is open. The Claude
compatibility mapper explicitly maps `PermissionRequest` to unsupported. The
public [hook contract](https://cursor.com/docs/hooks) gives these policy hooks,
their output and their failure behavior. Prometeu does not use a neutral
timeout as an approval.

The interactive implementation keeps the resolvers for pending approvals in
`./src/pending-decision-store.ts` in `2986.index.js`. Its React consumer calls
`approveDecision`/`rejectDecision` in the same process. Root questions and mode
changes use query IDs and promise resolvers that React holds. The audit found
no external endpoint for these objects. The installed `index.js` implementation
of `agent persist attach` attaches a tmux terminal. This terminal sends input.
It does not show typed request identities to other processes.

[ACP](https://cursor.com/docs/cli/acp) gives permission, question and plan
responses for a session that ACP owns. Its `./src/acp/run.ts` in
`8104.index.js` makes a separate agent runner through stdio. It is not an
attachment to the TUI that operates. A second ACP session makes a
different execution authority. The [SDK Bridge](https://cursor.com/docs/sdk/bridge)
also contains the SDK and owns its own agents. It is not a published channel to
control the TUI.

These results are evidence from the source and from the documented interfaces.
The audit did not send a new model request or change global settings. The
results do not prove that a future CLI cannot show a channel that operates for
this function. These methods must have their own design and a conformance
proof:

- Send menu keys.
- Add instrumentation to the private React state.
- Replace the native dialog.

The app does not use one of these methods as a permission response.

These tests examine this boundary, and they do not change the global Cursor
catalog or the Chat behavior:

- `conversation-capabilities.test.ts`, `team-viewer.test.ts`,
  `team-owner.test.ts` and `team-channel.test.ts`.
- The compatibility tests of the relay protocol.
- `chat::terminal::tests::native_only_decisions_reject_application_responses_without_disabling_native_input`.

To do the audit again, use the installed module names above and these SHA-256
values:

| Installed bundle | SHA-256 |
| --- | --- |
| `index.js` | `92ce160b560396971bf318f9e9f6c2fde8810c36e9f2aad68b7ac2b81af2ddee` |
| `2986.index.js` | `deb07a87c5c16c31c0429d41320fa1ea2f6fd29bc14250f461969940b0fb365b` |
| `4327.index.js` | `3815ff2df5531bde1f725fafbb2ffcc39b5721c6a83690c7dfb2c0ab70c4dc0c` |
| `8104.index.js` | `6605d145643e8190df2b5250bbef41a54d96473baab88c49e3dc9c0915a85e0f` |

## Native-store recovery

The macOS terminal recovery edge is different from the Chat ACP path.
[ADR 0100](../decisions/0100-conversation-transport-identity.md) gives this rule.
The native instructions use the `additional_context` response of a private
`beforeSubmitPrompt` hook. In the pinned CLI, this carrier operates before each
native submission. This includes resumed sessions. `sessionStart` alone does not
operate on a resume.

The private hook returns only context. The other hooks and the native permission
policy stay in force. The instructions are data in a private JSON file that
`/bin/cat` reads. They do not become shell syntax or prompt text that a person can see. The app examines the native carrier limit of 10,000 UTF-16 units before
the launch.

A controlled native smoke test examined a new session and a resume with changed
instructions and the same user prompt. The unit tests examine the quotes, the
life of the file, the permissions and the Unicode limit. See the native
[hook contract](https://cursor.com/docs/hooks) and the versioned implementation
checks in `terminal_agent_hooks.rs` and its opt-in smoke test.

Cursor CLI `2026.11.13-eacade1` resumes from
`<captured-config>/chats/<md5(physical-cwd)>/<native-session>/store.db`. The
preparer resolves the current workspace one time before the launch. It keeps the
initial path of the board. It uses the same physical path for these items:

- The PTY cwd.
- `--workspace`.
- The store hash.
- The transcript slug.

The native store calculates the hash of `process.cwd()`. This value resolves
file system aliases, for example macOS `/tmp` to `/private/tmp`. If the app
calculates the hash of only the lexical board path, it can hide a native
response that the CLI wrote correctly. The preparation regression test examines a
folder symlink followed by `..`, on new and resumed launches. It does not search
histories.

The caller supplies this authorized session folder. The reader does not
search sessions or open the editor database. Its pinned root and
no-follow opens accept only regular files with one link that the current user
owns.

`terminal_agent_cursor_store.rs` copies the database and its WAL into a private
staging folder, with a maximum of 4 MiB for each poll. It rejects a copy if one
of the two sources changes. SQLite opens only that copy. The app does not open
the initial shared-memory file. It does not use `immutable` to
bypass the WAL. The app deletes the copy on drop.

The reader must have all of these items:

- The source database `user_version: 1`.
- The metadata `agentId` with the same value.
- A CLI version that is at or above the examined `2026.11.13` decoder floor.

Previous versions, or versions that the reader cannot parse, cause an explicit
failure. The reader decodes newer versions with the same framing validation.

`protocols::cursor_store` follows only the blobs that the root refers to. It
examines their SHA-256 identities. The protobuf fields keep the identities
of the user message, the request, the run and the tool call. A step index
belongs to its native turn. The hash of the blob that the step refers to is its
content version. Thus an updated answer does not become a different answer.
The same text in different runs stays as different items.

The compaction of the derived JSONL does not identify or remove canonical
history. The reads of the protobuf data have limits. They skip binary fields and do not
use memory for their payload. They keep the user text. These conditions cause explicit
failures:

- Malformed framing.
- Missing blobs.
- Identity conflicts.
- Mandatory text that is too large.

The pinned tool descriptors are functional field metadata in
`cursor_store/tool_schema.json`.
[`extract-cursor-store-schema.py`](../../scripts/extract-cursor-store-schema.py)
can make them again. The extractor reads an explicitly selected CLI source file
and does not run it. The artifact records its SHA-256 and the CLI version.

The tool projection keeps the native call IDs and the arguments that the agent
used. It removes the native approval and routing metadata. It captures results
as occurrences that are different from their tool blocks. A mutable result
updates the same tool. A pending tool does not make an incorrect completion.

Tool IDs are JSON strings with a limit, and the app does not parse them. The
examined CLI can put an LF between native IDs. The app keeps the LF and writes
it as an escape sequence in canonical lines. Tool IDs do not select files or
approval paths. MCP maps keep JSON values,
text output and structured results. Read results follow authenticated
links to content blobs. Truncated calls follow and examine their initial
step. They use a chain of links with a limit and the same native call identity.

The app shows binary tool output as its byte count and an explicit omission. It
does not keep the base64 data. Unknown tool fields stop the coverage. Stored
tool calls do not make requests that a person can answer.

The opt-in `configure_cursor_store` path of the observer receives the captured
`chats/<cwd-hash>` folder. It opens only the authenticated session child.
`set_ingestion` receives private `NativeIngestionBatch` values. These values
contain the source revision, the replay or live phase, and occurrence and version
records with limits. A record contains a canonical frame or a checkpoint without
content. No record sends a vendor payload into the core.

The native message ID is the user occurrence. It does not change with request and
run retries. Assistant steps keep their public `(messageId,index)` across blob
revisions. Thus authoritative updates add to the canonical log, and the visual
item keeps its position.

The host acknowledges a batch only after it commits all supported records to
durable storage. Until then, the same batch can come again. `caught_up` must
have a last acknowledgment and a subsequent check that shows that the source did
not change. Resume batches stay replay until the host acknowledges their full
bootstrap. They do not use the previous live or accepted-prompt callbacks.

`expect_submission` connects a new native user occurrence to an app submission
identifier that the core does not parse. It uses a causal queue in sequence,
with a limit. Previous occurrences and the same text cannot use subsequent
submissions. It does not remove duplicate
history by text.

Unknown tools, turns with only shell work and synthetic-user kinds without a
projection explicitly stop the coverage. This also applies when a subsequent
compaction removes them. Their checkpoints without frames are not a proof that
the ingestion is complete. No root snapshot, run status or missing activity
makes an incorrect completion event. The native host compares the captured
occurrence and version receipts through its private ingestion journal. Before
the host fills the Lines, the bootstrap recovery uses only the storage. The live
publication follows the durable commit. See
[persistence](persistence.md#native-ingestion-journal-foundation).

The success fence compares the `generation_id` of a live `stop` hook with the
native `request_id` of the last turn with a complete ingestion. Before the
authenticated listener acknowledges that hook, it captures the metadata of its
authorized transcript. It uses pinned folder handles and no-follow opens.
Only the host has that witness. The helper envelope cannot supply it, and a
previous process cannot give it.

Then the observer must see a changed and stable file revision. This revision
must end in a complete `turn_ended` / `success` record. The pinned writer sends
this marker only after the Stop hook chain returns. Usual writes remove the
marker of a previous turn. If the file is missing before the acknowledgment, it
can become a regular authorized file subsequently. Read failures, records that are not complete
and a previous marker that did not change do not make the fence.

Native run records are optional in the examined TUI store. If they are there,
their identities and completed states must agree. The host does not make
missing records. The host also examines the current empty composer and the
pending transaction for app input. Then it records the completion in the
journal before it publishes or changes the execution state. A composer paint
alone cannot bypass the post-hooks marker. Duplicate request receipts do not
send again the notifications, telemetry or completion effects. A cancel uses the
different authenticated interruption path above. Native errors and unsupported
background content stay different conformance requirements.

Synthetic tests in `protocols/src/cursor_store/tests.rs` and
`terminal_agent_cursor_store/tests.rs` examine these items:

- The same input more than one time, compaction, new runs and mutable answers.
- Large binary context, hashes, WAL recovery and a source that stays isolated.
- Pending and result revisions, MCP values and read blobs.
- Truncated calls, binary limits and malformed tool unions.

`terminal_agent_observation/cursor/tests.rs` examines the acknowledgment, the
quiet resume, duplicate submissions and coverage that is not complete. `src/timeline.test.ts`
tests late authoritative updates that do not move the current positional keys.
`chat/terminal/ingestion/tests.rs` uses that journal with the production portable
pump. It examines a new start of the host, a missing native tail, normalized input aliases and a
failure before the publication.

Controlled native smoke tests validated the copied store decoder for scratch
sessions that the hook authorized. This included one Shell call and its full
output. The test explicitly approved that call one time in the native prompt.
Programmatic approval controls are not in the accepted scope of the native
mode. The full conformance of the Tauri app is different from these native
smoke tests.

`terminal_agent_observation/cursor/tests.rs` and `terminal_agent_hooks.rs` also
examine these items:

- The capture before the acknowledgment.
- Success markers that are not current or not complete.
- The request identity.
- The cancel of a proof after new input.
- Duplicate hooks.
- Symlinks and hard links, which the host rejects.
- A durable completion without a second publication.

These cancel regression tests examine the interrupted outcome, the duplicate
acknowledgment and the boundaries of previous requests. They do not use the success
marker:

- `cursor_interruption_requires_durable_request_tail_without_a_success_marker`
- `cursor_interruption_rejects_other_requests_and_new_app_input`
- `cursor_interruption_rejects_late_old_hooks_before_new_prompt_reaches_store`

`cursor_restored_prompt_retry_settles_without_repeating_before_submit_hook`
examines a subsequent request that the source shows, without a second prompt
hook and with previous Stop hooks that come late. It is a synthetic regression test for the
sequence. It is not evidence that a specified native retry did not send its
hook. The controlled identity smoke test showed that the hook and the request
agree. It also showed that its own session did not have optional run records.

A second controlled native run examined these items together:

- The witness before the acknowledgment.
- The copied source and the durable ingestion receipts.
- The post-hooks marker and the empty composer.

Then the run committed only one
completion receipt. The headless harness uses the same observer and journal
path. It does not claim a test of the Tauri UI or of shared control.

## Native streaming diagnostics

Cursor CLI `2026.10.22-27dcf5f` can send
`Error: RetriableError: WritableIterable is closed` as an `agent_message_chunk`
after the answer text. Then it returns `stopReason: end_turn`. A standalone ACP
client without Prometeu also showed this behavior. Do not remove that
assistant text. Do not try again automatically. These actions can hide a
failure or do the tool effects again. Cursor support tells that this is a
[known transient stream teardown bug](https://forum.cursor.com/t/cursor-cli-error-writableiterable-is-closed/165741/5).
The person can send a different prompt or resume the native session.

## Evidence

`src-tauri/crates/protocols/src/cursor/tests.rs` examines the translation, the
MCP session payload and the protocol failures. It uses a
[sanitized recording](../../src-tauri/crates/protocols/src/cursor/fixtures/README.md).
The hub encoding tests are in `crates/tools/src/tests.rs`. `src/conversation.test.ts`
uses the V1 fixture that the recording gives. The account compatibility tests are in
`src-tauri/crates/core/src/accounts/tests.rs`. The tests for the storage of the
model selection are in `src/model-choice.test.ts`. The current account and
launcher browser journeys examine the controls that this feature uses again. This
feature does not add a provider matrix of browser tests.

`src-tauri/src/cursor/quota/tests.rs` examines the quota types, missing or
malformed data, the expiry, the credential validation and the window
serialization that does not change. For a read-only authenticated dashboard
check, run its ignored `live_cursor_quota` test.

For a live check with paid inference, get an explicit authorization. Then run:

```sh
cd src-tauri
cargo test --bin Prometeu cursor::tests::live_cursor_round_trip -- --ignored --exact
```

The test keeps file work in a new temporary folder. The default suites do not
call Cursor or use model credits.
