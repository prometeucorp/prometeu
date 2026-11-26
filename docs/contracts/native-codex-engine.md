---
type: Contract
title: "Native Codex engine"
description: "The private app-server that the app owns, the accurate configuration of the thread and the attachment of the native TUI."
status: stable
---

# Native Codex engine

The macOS terminal transport uses this private engine for `codex-cli 0.169.0`
and newer versions. We examined version 0.169.0. The hook bridge refuses older
releases. We do not stop newer releases, because Codex makes new releases
frequently. The bootstrap that follows does a check of the socket, the peer, the
identity of the thread and the worktree. If the private protocol changed, the
bootstrap stops with a failure.

A person can select the terminal mode in Settings / General. Ask and Plan use
native settings of the thread that we examined. The tests of the bootstrap and
of the consumer had no failures. The native CLI evidence in this document is
different from the macOS Tauri conditions in the provider matrix. These
conditions are the response, the history, the cold resume and the shutdown of
the engine.
Refer to the [provider matrix](../quality/provider-matrix.md) and the
[decision about the transport identity](../decisions/0100-conversation-transport-identity.md).

## Session ownership

`terminal_agent_codex::NativeEnginePlan` owns an app-server command that did not
start. The preparation records these items:

- The account and the worktree.
- The selected model and effort.
- The instructions and the MCP environment.
- The derived plugin home.

The hook bridge configures this command before it starts. The TUI gets the same
recorded environment and worktree. The semantic settings of the invocation
belong to the server.

The plan starts `codex app-server --listen unix://<private>/rpc` in its own
process group. The plan makes the directory with mode `0700`. Codex publishes an
alias to an endpoint with a SHA-256 name in its own private socket directory.
The client does a check of these items:

- The map from the alias to the endpoint.
- The owner and the mode of the directory.
- The owner and the mode of the socket.
- The peer PID that the kernel reports. It must be the PID of the owned server.

The client uses WebSocket framing through Unix. There is no TCP listener, no
compression and no TLS configuration.

The bootstrap occurs in a specified sequence, in a limit of 20 seconds. It
completes before the TUI process can get input:

1. Initialize one `prometeu-native` RPC client. The name tells the correct client.
2. Start a new thread, or resume the accurate UUID that the app saved. If the
   resume does not operate, the bootstrap does not use a different thread.
   Examine the returned identity and worktree.
3. Give a new thread the native title `Prometeu`. Without a title, Codex does not
   keep an empty thread, and a native remote resume cannot occur. The title
   sends no request to the model.
4. Apply the selected approval policy, sandbox, model, reasoning effort and
   collaboration mode through `thread/settings/update`.
5. Read the current settings and start the subscription through a correlated
   `thread/resume` with `excludeTurns: true`. Make sure that each requested
   setting is correct. A settings update that does not change a value sends no
   notification. Thus, a notification alone cannot show that this step is
   complete.
6. Read `thread/turns/list` in pages with full items, from the first turn to the
   last turn. Before the app writes the canonical stream, commit the missing
   native occurrences to the private ingestion journal. This recovery does not
   cause live changes.
7. Start `codex --remote unix://<private>/rpc resume <exact UUID>` in the PTY.

The TUI gets no overrides of permission or sandbox, because a remote resume
refuses them. The public configuration of the app-server starts with a policy
that Codex accepts. The RPC for each thread sets these values:

- `untrusted` for Prometeu Ask.
- `never` for Auto and for the previous bypass behavior.

The two values use `dangerFullAccess`. This agrees with the current structured
adapter. Plan uses the native collaboration mode and the native plan
instructions of Codex. There is no synthetic `/plan` prompt from the user and no
dummy inference turn.

## Remote machines

On a remote machine ([ADR 0105](../decisions/0105-codex-on-remote-machines.md)),
the plan holds an engine host. It does not start a child process. The host opens
the engine and a relay as host terminals. The relay does the checks of the
socket, the alias and the peer on the machine. In this check, the peer can also
be a descendant of the engine process. The relay sends the same messages as
JSON lines.

The bootstrap, its time limit and its other limits do not change. The TUI gets
the `unix://` address of the relay on the machine. If the relay stops, the app
releases the conversation, but the engine and the TUI on the machine continue to
operate. The `~/.codex` of the machine applies. The app sends no account, plugin
or MCP configuration.

## Lifetime and explicit control

`NativeEngine` keeps one owner for the child process, the socket, the reader and
the cleanup. The host keeps it until the host reaps the PTY. The shutdown does
these operations:

1. It closes the connection.
2. It stops the owned process group.
3. If necessary, it uses a stronger signal after one second.
4. It waits for the child process.
5. It removes only its private alias and its endpoint and lock, after a check
   that they are the physical items.

A failure at startup uses the same cleanup. If the socket disconnects or the
event queue is full, the app discards the engine. The host must then also
discard its PTY.

Before the process stops, the Tauri `RunEvent::Exit` calls the explicit shutdown
of the engine. The contract of the shutdown is not the drop of AppState. A
private weak registry keeps access to the worker that owns each engine. This
access stays while a conversation or an input port continues to refer to the
engine. The exit does these operations before it returns:

1. It refuses new engine starts.
2. It waits until each bootstrap that it let start is in the registry or stops
   with a failure. Each bootstrap has a time limit.
3. It sends a signal to all workers and waits until they stop.

A concurrent stop and exit use the same gate. Thus, the two calls do not return
while a different caller continues to reap that engine. A subprocess regression
test keeps the engine open and keeps a bootstrap that continues. It then
examines the reap of the child process, the removal of the endpoint, a second
shutdown and the refusal of a new start.

Derived `CODEX_HOME` configurations do not share `app-server-control`. Codex 0.169
refuses a symbolic link at this path of the runtime directory. The
materialization removes a previous derived alias, but does not follow the link.
Then Codex makes its private directory. The materialization keeps the physical
control directories and the endpoints of the initial home that are there. The
authentication and the durable history keep the same shared paths as before.
The regression test for the compatibility of the tools examines the migration of
previous links and a second preparation.

The connection accepts frames and messages that come in with a maximum of
64 MiB. This agrees with the byte limit of the history, because native user
items contain inline image data in live events and in cold replay. Control
messages that go out stay at a maximum of 1 MiB. During the bootstrap, the
client keeps a maximum of 64 startup messages and 8 MiB. The live channel holds
64 messages, with a total limit of 64 MiB of serialized JSON.

When the consumer receives or deletes queued events, their reserved bytes become
free. Thus, a stopped consumer cannot keep 64 images at the maximum limit. If
the byte limit or the message limit is full, the app discards the engine and its
PTY through the current explicit overflow path. Control that goes out has a
queue of 32 entries and a limit of five seconds. An overflow causes an explicit
failure. A frame that comes in above the limit causes a limit failure.

The history also has a limit of 64 MiB and 10,000 turns, with 16 turns for each
page. It uses the time limit of the startup. If a native ID occurs again, or if
the pagination has an incorrect format, the startup stops with a failure before
the TUI starts.
These limits prevent too much use of the memory of the process. They do not tell
that all provider payloads fit. The host reads the live channel continuously. If
the app cannot read the channel, it discards the engine and the PTY.

The attached protocol input lets the app send only these messages:

- `account/rateLimits/read`.
- An interrupt request for this thread only.
- An explicit response to a native request that the app saw and that waits
  for an answer.

It refuses initialization, the start or resume of a thread, and `turn/start`.
The TUI owns the text, the slash commands and the native interactions. The app
sends no automatic error or decision for a server request that it does not know.
The server can close a request through the TUI first. Then
`serverRequest/resolved` cancels the ID that waits in the app. A correct write
shows that the connection received the message. It does not show that the
server accepted a decision that a different client closed before.

The attached decoder keeps a request that it sent until the native resolution.
It refuses more answers and a second delivery of the same request. Thus, the
subsequent `serverRequest/resolved` closes the same card after an answer from
the app or from the TUI. The delivery alone does not close the card.

`prometeu_protocols::codex::Link::attached` uses the current event decoder
again, but does not run its startup handshake. It refuses canonical message
sends. It keeps the native IDs of assistant items across streamed blocks and
last blocks. It sends explicit request responses and interrupt requests. The
native resolution closes the card with the outcome `resolved`. The notification
does not tell if the person accepted, refused or answered the request. The app
does not guess a decision outcome.

Public request IDs contain a new engine nonce and a typed digest of the native
ID. Thus, a reply from an earlier engine cannot answer a new request that uses
the same JSON-RPC ID after a resume.

The desktop consumer `chat/terminal/codex.rs` uses RPC as the only canonical
source. The last records of the user, the items and the turns have native
occurrence receipts. Deltas and notifications without content go through the
usual event loop. The paginated history of the bootstrap uses the same IDs and
the same projection. The rollout is evidence. It does not publish content a
second time. Paginated Codex rollouts keep `item_completed` and not the legacy
`user_message` event. Thus, the legacy transcript decoder cannot give this
recovery path.

The app prepares the durable receipt of its input before it writes to the PTY.
The app compares the native user item with that FIFO causal boundary. This
boundary includes the native user item before it and a checked content hash. The
identity of the item is not equal text at any time. The app records manual
native input through the usual path for app input and attribution. While an app
input waits for its receipt, the app does not publish RPC content. The app
keeps the native `clientId` private. The TUI makes this ID, and the app does
not make one.

Last assistant blocks and tools keep their native item IDs. A completed turn has
a receipt for the thread and the turn. Thus, live delivery and cold replay
cannot each count the same completion or send a notification for it. A tool
that was active in the history does not get a synthetic result. Assistant text
that is not complete is only a snapshot of the content. While a turn is active
in RPC, the app does not let input in until it commits the ordered completion of
the turn. This rule applies also when PSP reports Done first. Hooks and the
rollout do not publish Codex content or completion independently.

Request answers and interrupt requests for this thread only go through the
attached decoder. Text and slash commands stay native terminal input. Quota
reads and updates use the current canonical path for account usage. A native
settings update changes the model and the effort that the decoder saw. On a
resume, the app applies an explicit selection again. Before the app lets input
in, it makes sure that the selection is correct.

The app accepts input when PSP tells so, or when one of these conditions occurs:

- The native `thread/status/changed` tells that the thread is idle.
- The app committed the completion of the current turn.

An unknown, active or permission state cancels this condition. An empty composer
on the screen is also necessary.

## Evidence and limits

`terminal_agent_codex/tests.rs` uses a fake WebSocket server process that the
test owns. The test examines these items:

- The sequence of the startup.
- The refusal of an incorrect identity or configuration.
- The queue failure.
- Inline images larger than 1 MiB in live delivery and in cold replay of the
  history.
- Replies that are only explicit.
- IDs from an earlier engine.
- The refusal of a turn start.
- The reap of the child process.

`terminal_agent_codex/events.rs` examines a byte overflow with a stopped
consumer. It also examines the release of reserved bytes in three conditions:
the consumer receives an event, the receiver stops, or a delivery has a failure.
`crates/protocols/src/codex/attached_tests.rs` examines the attached behavior of
the current decoder. It also keeps the compatibility tests of the structured
adapter.

The ignored tests in `terminal_agent_codex/smoke.rs` had no failures on Codex
0.169.0. They used an empty private worktree and a private copy of the current
external authentication. Two conditions showed the native Plan: a new start,
and a cold stop, start and resume of the server. They kept Ask, the effort and
the accurate identity. The same TUI continued to operate, and there was no model
turn or tool request.

A different smoke test with one turn and no tool operates the native compositor.
It sends the RPC frames from Codex through the runtime consumer. Then it compares
the cold replay of the native history. The result is one user item, one
completion and canonical bytes that did not change.

The app agreement test does not use inference. It accepts an explicit fixture
root that it owns, through `PROMETEU_CODEX_ENGINE_APP_PROOF`. It also accepts an
optional ID of a saved tab, through `PROMETEU_CODEX_ENGINE_APP_TAB`. With this
ID, the test finds the workspace settings, the tools and the account of that tab
through the production preparation and the hook bridge. Then it makes sure that
a cold resume operates without a model turn. The test does not use a new thread
as an alternative, and it does not examine unrelated native sessions.

`chat/terminal/codex/tests.rs` also examines these items:

- The sequence of the receipts.
- Input text that occurs again.
- The identity of tools.
- The end of a transcript after a crash with an active tool.

The sanitized views and the evidence of the identity are in
`terminal_agent_codex/fixtures/`.

The different ignored test `terminal_agent_codex/decision_smoke.rs` had no
failures on one Plan question to the model. These are the results:

1. The screen showed the native numbered options and the footer to send the
   answer.
2. Then the attached adapter answered Blue.
3. The native resolution closed the canonical card.
4. The completed model response was accurately Blue.
5. The adapter refused a second answer from the app.
6. The test closed and reaped the engine, the TUI and the screen helper.

The fixture of this test records only agreement booleans and the specified
answer.

The command condition also had no failures with Ask (`untrusted`) and with Plan
disabled. After the native approval heading, the screen showed the accurate
command and the selection for one time only. The attached adapter explicitly
let `printf PROMETEU_TERMINAL_TOOL_SMOKE > native-approval.txt` operate one
time. The test validated these items:

- The scratch working directory.
- The resolution of the related native request.
- The completion of the command with exit code zero.
- The accurate contents of the marker file.
- The last assistant response.
- The refusal of a second answer.

The allowlist of the test refuses other shell commands. It also refuses writes
to a file that is not that file in the scratch directory. The test reaped the
two native processes and the screen helper.

These tests do not make sure that these items operate correctly:

- The native approval of file changes.
- The behavior of plugins and MCP.
- The shared sessions.
- The quota accounting.
- The full startup of the Tauri application.

For native inputs that are not text, other than image metadata, an explicit
conversion is necessary. An unknown input variant causes a failure. Thus, the
app does not delete content without a message. The test does not change the
global native configuration and does not use a login flow.

The upstream remote TUI does not enable its special `userVerification` procedure
for devices. Upstream gives that extension only to its embedded TUI and its
local desktop connection. Prometeu does not use those client names or origins
falsely. Thus, this native feature stays different, also when the usual
approvals are available through the remote TUI.

We examined the implementation against these upstream sources at
[`rust-v0.169.0`](https://github.com/openai/codex/tree/rust-v0.169.0):

- The [Unix socket transport](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server-transport/src/transport/unix_socket.rs)
  gives the alias with its guard and the WebSocket endpoint.
- The [RPC contracts of the thread](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server-protocol/src/protocol/v2/thread.rs)
  give the overrides of the settings, the resume and the collaboration mode.
- The [thread processor](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/request_processors/thread_processor.rs)
  gives the native materialization and the semantics of the subscription.
- The [app-server session of the TUI](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/tui/src/app_server_session.rs)
  keeps the permission settings of the server on a remote resume.
- The [requests that go out](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/outgoing_message.rs)
  and the [initialization](https://github.com/openai/codex/blob/rust-v0.169.0/codex-rs/app-server/src/request_processors/initialize_processor.rs)
  tell which requests can get a decision and which client owns the device
  verification.
