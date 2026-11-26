---
type: Contract
title: "Native Antigravity store"
description: "How Prometeu recovers the initial AGY steps and tool results for terminal conversations, with versions and a fixed scope."
status: stable
---

# Native Antigravity store

The macOS terminal transport uses this boundary for canonical content and for
recovery. A person can select the terminal mode in Settings / General. The
current NDJSON Chat adapter does not change. The
[provider matrix](../quality/provider-matrix.md) shows the verified Tauri flows
apart from the store tests, the CLI smokes and the checks that are not done.

## Source and identity

AGY 1.5.10 and newer versions keep the initial trajectory in
`<captured-home>/.gemini/antigravity-cli/conversations/<native-session>.db`.
The captured account gives this root. An authenticated root hook or the private
journal gives the exact native session. Prometeu does not select a conversation
from a directory list or from a text match. The one `trajectory_meta` row must
have the same `cascade_id`. Prometeu keeps its different `trajectory_id` as
private provenance. If this ID changes, Prometeu cannot use the step
identities again without a sign.

The adapter validates these items:

- SQLite `user_version = 1`.
- The necessary table layouts.
- `step_format = 0`.
- The step type and the step status agree between SQL and protobuf.

The step indices are the native `steps.idx`. They are not positions in an export
that the adapter wrote again. The pinned `gemini_coder.Step` payload contains
user input, planner responses and generic tool steps. The `tool_calls` of a
planner and the `metadata.tool_call` of a step that ran have the same native call
ID. Results use this explicit relation, also when there are other steps between
them. If a call identity is missing or is not the same in the two places, the
adapter does not give coverage.

In this path, the `transcript_full.jsonl` summary is not the authority for the
content, because it does not keep all the data. The native `user_response` is the
initial text that the person sent. Thus the adapter does not remove a wrapper for
the model to guess an input alias. For aliases from a program, the current bounded
and causal FIFO receipt for a submission continues to be necessary.

## Read boundary

`terminal_agent_sqlite_snapshot` copies only the selected database and its WAL.
It opens the initial files and does not follow the symlinks of descendants. It
makes sure that the file has the correct owner, is a regular file and has one
link. While it copies, it compares the device, the inode, the length, the
modification stamp and the change stamp. It does not open the source SHM at any
time.

Each poll copies a maximum of 4 MiB with a 64 KiB buffer. If the file changes
during the copy, the adapter discards the partial copy. SQLite opens only the
private `0700` staging directory and the `0600` copied files. It opens them
read-only, with `query_only` and `trusted_schema = OFF`. Thus the WAL recovery
cannot write the initial files of the provider. Cursor uses the same copy
boundary.

The pure protobuf decoder reads the copied blobs in bounded parts. It has these
limits:

- A depth of 16.
- A total of 65,536 visited fields.
- 1 MiB for a necessary string.
- 4 MiB for the total of the kept strings in each step.
- 128 MiB for a step payload.
- 65,536 steps for a snapshot.

The decoder does not allocate memory for binary media, but the media stays part
of the source hash. If the data is more than a limit, the result is an explicit
coverage failure. This failure does not mark the turn as complete at any time,
and it does not silently discard necessary text.

A generic result can refer to its full output. The adapter accepts only a local
file URL for
`<captured-brain>/<same-session>/.system_generated/steps/<same-index>/output.txt`.
The file must pass the same checks for symlinks and the owner. It must also pass a
check of the stamps before and after the read. The full UTF-8 output replaces the
preview. Its maximum size is 1 MiB. The reader examines this file again when the
database does not change, thus a subsequent write gives a new source version. The
adapter rejects other paths and URL hosts.

## Projection and coverage

Stable anchors for messages and blocks use the native session and the step index.
Private receipts for occurrences and versions keep the changes, and they do not
remove the canonical history. Thinking, text and tool blocks have fixed slots.
Tools keep their native call IDs. If a person edits an argument in the native
approval interface, the adapter updates the initial tool block. A terminal result
of the execution closes that same tool.

The adapter changes the native model usage into ephemeral metadata with a
receipt. This metadata includes input counts that show the cache and partial
totals for each user. A replay does not send live telemetry or notifications at
any time.

The adapter can read these payloads:

- USER_INPUT.
- PLANNER_RESPONSE.
- The system CHECKPOINT.
- The explicitly hidden SYSTEM_MESSAGE form that this section describes.
- GENERIC with text results or protobuf Struct results. This includes the
  verified RunCommand payload.

Auxiliary Any metadata does not have to have its own renderer when one of these items
gives the output:

- The full result text or Struct.
- An authorized file with the full output.
- The native error message.

The adapter continues to validate and
limit its envelope and its protobuf framing. If the output is missing, the
adapter does not show a success that did not occur.

The native Step.error user message or short message sets the error flag, also
with DONE. INVALID and ERROR are terminal errors. RunCommand also examines its
explicit typed exit code and its timeout. An unknown exit code does not become a
success. These items stay explicit coverage failures until they have their own
codecs: GenAI result and step forms, control outputs of a trusted CLI, nested
trajectories and unknown kinds of steps.

CHECKPOINT uses step type 23 and payload field 30. Its inline summaries become
`system.summary`. These summaries are about the user intent, the session, the
code changes and the memory. If the step is DONE and there is no unrecovered
failure of the summary, the adapter also records `context.compacted`. The adapter
does not guess the token counts before and after, or a manual or automatic
trigger. A referenced summary without inline text stays not supported. The usage
of a checkpoint stays isolated from the usage of a model turn. These records are
metadata, also when there is no model turn or user ancestor.

When Antigravity resumes a session, its records can put a SYSTEM_MESSAGE (type
101, payload field 114) between a user and the next planner response. Only these
three conditions together authorize this hidden form:

- The SYSTEM origin (5).
- `render_info.hidden` (field 4) is true.
- `agent_message.hide_from_user` (field 11) is true.

The decoder validates its bounded text, its identity and the nested fields that
it can read. Media, embedded step payloads and unknown forms stay not supported.
The projection keeps only a private source checkpoint with the same trajectory and
causal user. It does not send a public system message, assistant content or a turn
completion. Thus the subsequent answer stays with the new user and not with the
previous completed turn.

The exact native USER_INPUT `/compact` has a private maintenance marker. Its
history has a receipt without attribution to the model input. The observed
execution can become idle when these three conditions occur together:

- A terminal CHECKPOINT is connected to that same native user.
- The source is current and Prometeu received the acknowledgment.
- The current root PSP is Idle, Done or Error.

This change does not make `turn.completed` or a response notification. New
accepted input discards this maintenance state. Active children continue to hold
it. Other slash commands do not get a maintenance classification by inference.

A DONE **step** closes only that source record. It does not make
`turn.completed`, a request card or a routing for a response at any time.
`sourceScope: record` and `sourceUser` make sure that late changes do not change
the execution of a different input. For a correct completion, these items continue
to be necessary:

- The authenticated Stop.
- The root PSP Done.
- The related final assistant message in storage.
- Durable acknowledgments of the batches.
- A subsequent poll that is current.

The observer processes one native step in each batch. It waits for the
acknowledgment of the receipt before it continues. When Prometeu resumes a
session, it completes the replay from storage only, and then it starts the
canonical Lines. Native errors and content that is not supported cannot release
that barrier when they send nothing.

## Evidence and verification

The field metadata of the independent decoder comes from the FileDescriptorProto
records of the installed 1.5.10 binary.
[The schema fixture](../../fixtures/antigravity-store-schema.json) contains the
selected field numbers and the SHA-256 of the binary.
[The extraction helper](../../scripts/extract-antigravity-store-schema.py) makes
this fixture again. The helper reads only an executable that a person explicitly
gives. It does not read account data or conversations at any time.

A read-only examination of the Go function metadata and the ARM64 branches of the
same binary found these facts:

- `cli/steps.genericToolOutput` reads the generic result string.
- `ExtractToolInfo` independently applies `Step.GetError` and `ErrorMessage`.
- `DeriveStepState` changes status 4 and status 7 to ERROR.
- The first function only makes the text of a file read shorter for the
  presentation. It does not decode auxiliary Any metadata.

These observations show that Prometeu can keep the full result content without a
renderer for each metadata message. The independent Rust decoder copies no
implementation from the provider.

The same binary identifies manual compaction through `render.isManualCheckpoint`
and `precedingCompactUserStep`. `stepcompactor.buildCheckpointStep` makes a
CHECKPOINT (23), DONE (3), with a SYSTEM (5) origin. The schema fixture pins its
functional field numbers. Tests examine these items: the metadata projection, a
replay without a model turn, the causal maintenance identity and the idle state
without an assistant completion.

Synthetic tests in `crates/protocols/src/antigravity_store/` examine these items:

- The framing and the identity.
- Tool results that are not adjacent to their calls.
- Repeated text and versions that change.
- Binary media, limits and usage.

`terminal_agent_antigravity_store/tests.rs` examines these items:

- Reads of committed WAL data that do not change the initial files.
- Updates of the external output.
- Paths that go outside the permitted folder.
- Rotation, Fresh and Resume, and receipt barriers.

Completion regression tests make sure that a snapshot that the adapter wrote
again must contain its final candidate again. They also make sure that a planner
that continues to request tools cannot become final. The current snapshot tests
of Cursor examine the shared copy budget and the restart after a WAL change.

The regression for the hidden system message is in
`terminal_agent_antigravity_store/tests.rs`. It starts from a resumed user and
answer. Then it reads a second user, a hidden system step and an answer with
repeated visible text. It makes sure of these results:

- There are two different users and two different answers.
- The new user is the causal user.
- There is no hidden public event.
- A repeated cold replay gives the same bytes.

A verification in the macOS Tauri app recovered that subsequent turn after the
native TUI answered. The canonical history contained two users and two answers,
without copies. The recovery kept the one previous completion. Nobody examined
independently a new live completion after this repair.

An authorized scratch smoke ran exactly `printf PROMETEU_TERMINAL_TOOL_SMOKE`
with the native Ask approval. The exact authenticated database and the output file
of the completed CLI passed the optional reader and projection test. The result had
one tool, one related result and the full marker output. The smoke did not read
other sessions. It does not give coverage for tool payloads that nobody examined.
The previous AGY smoke for the success fence, without tools, stays different
evidence for the native completion protocol.
