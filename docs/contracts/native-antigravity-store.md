---
type: Contract
title: "Native Antigravity store"
description: "Versioned, scoped recovery of original AGY steps and tool results for terminal conversations."
status: stable
---

# Native Antigravity store

The macOS terminal transport uses this boundary for canonical content and
recovery. Terminal mode is selectable in Settings / General; the existing
NDJSON Chat adapter is unchanged. The [provider matrix](../quality/provider-matrix.md)
separates verified Tauri flows from store tests, CLI smokes and remaining checks.

## Source and identity

AGY 1.5.10 and newer persist the original trajectory in
`<captured-home>/.gemini/antigravity-cli/conversations/<native-session>.db`.
The captured account supplies this root; an authenticated root hook or the
private journal supplies the exact native session. No directory enumeration or
text matching selects a conversation. The single `trajectory_meta` row must
have the same `cascade_id`. Its distinct `trajectory_id` is retained as private
provenance; changing it cannot silently reuse step identities.

SQLite `user_version = 1`, the required table layouts, `step_format = 0` and
agreement between the SQL and protobuf step type/status are validated. Step
indices are native `steps.idx`, not positions in a rewritten export. The pinned
`gemini_coder.Step` payload contains user input, planner responses and generic
tool steps. Both a planner's `tool_calls` and an executed step's
`metadata.tool_call` carry the same native call ID. Results use this explicit
relationship, even when several steps intervene. A missing or conflicting call
identity withholds coverage.

The lossy `transcript_full.jsonl` summary is not the content authority in this
path. Native `user_response` is the original submitted text, so no model-facing
wrapper is stripped to guess an input alias. Programmatic aliases still require
the existing bounded, causal FIFO submission receipt.

## Read boundary

`terminal_agent_sqlite_snapshot` copies only the selected database and its WAL.
It opens original files without following descendants' symlinks, checks owner,
regular-file type and one link, and compares device, inode, length, modification
and change stamps throughout copying. The source SHM is never opened. Each poll
copies at most 4 MiB with a 64 KiB buffer; instability discards the partial copy.
SQLite opens only the private `0700` staging directory and `0600` copied files,
read-only with `query_only` and `trusted_schema = OFF`. WAL recovery therefore
cannot write the original provider files. Cursor uses the same copy boundary.

The pure protobuf decoder reads copied blobs in bounded chunks. It limits depth
to 16, total visited fields to 65,536, a required string to 1 MiB and total
retained strings per step to 4 MiB. Binary media is skipped without allocation
and remains part of the source hash. A step payload is limited to 128 MiB and
a snapshot to 65,536 steps. Exceeding a limit is explicit coverage failure; it
never establishes completion or silently discards required text.

A generic result may reference its complete output. Only a local file URL for
`<captured-brain>/<same-session>/.system_generated/steps/<same-index>/output.txt`
is accepted. It passes the same no-follow/owner checks and a before/after stamp
check. The complete UTF-8 output, limited to 1 MiB, replaces the preview. The
reader also rechecks this file when the database is unchanged, so a later write
produces a new source version. Other paths and URL hosts are rejected.

## Projection and coverage

Stable message/block anchors use the native session and step index; private
occurrence/version receipts retain rewrites without removing canonical history.
Thinking, text and tool blocks occupy fixed slots. Tools keep their native call
IDs. An argument edited in the native approval UI updates the original tool
block; a terminal execution result closes that same tool. Native model usage
is projected as receipted ephemeral metadata, with cache-aware input counts and
partial per-user totals. Replay never emits live telemetry or notifications.

Supported payloads are USER_INPUT, PLANNER_RESPONSE, system CHECKPOINT, the
explicitly hidden SYSTEM_MESSAGE form below, and GENERIC with textual or
protobuf Struct results, including the verified RunCommand payload. Auxiliary Any
metadata does not require a separate renderer when complete result text/Struct,
an authorized complete-output file or the native error message already supplies
the output. Its envelope and protobuf framing are still validated and bounded.
Missing output is never replaced with a fabricated success. Native Step.error
user/short messages set the error flag even with DONE; INVALID and ERROR are
terminal errors. RunCommand additionally checks its explicit typed exit code and
timeout; an unknown exit code does not become success. GenAI result/step forms,
trusted-CLI control outputs, nested trajectories and unknown step kinds remain
explicit coverage failures pending their own codecs.

CHECKPOINT uses step type 23 and payload field 30. Its inline user intent, session,
code-change and memory summaries become `system.summary`; DONE without an
unrecovered summarization failure also records `context.compacted`. No before/after
token counts or manual/automatic trigger are guessed. A referenced summary without
inline text remains unsupported. Checkpoint usage stays separate from model-turn
usage. These records are metadata, even when no model turn or user ancestor exists.

Native resume bookkeeping can insert SYSTEM_MESSAGE (type 101, payload field
114) between a user and the next planner response. Only SYSTEM origin (5),
`render_info.hidden` (field 4) and `agent_message.hide_from_user` (field 11), both
explicitly true, authorize this hidden form. The decoder validates its bounded
text, identity and supported nested fields; media, embedded step payloads and
unrecognized forms remain unsupported. Projection retains only a private source
checkpoint with the same trajectory and causal user. It emits no public system
message, assistant content or turn completion, so the following answer remains
associated with the new user rather than the previous completed turn.

The exact native USER_INPUT `/compact` carries a private maintenance marker. Its
history is receipted without model-input attribution. A terminal CHECKPOINT tied
to that same native user, acknowledged source catch-up and current root PSP
Idle/Done/Error may settle observed execution to idle. It creates neither
`turn.completed` nor a response notification. New accepted input invalidates this
maintenance state; active children still hold it. Other slash commands acquire no
maintenance classification by inference.

A DONE **step** seals only that source record. It never manufactures
`turn.completed`, a request card or a response route. `sourceScope: record` and
`sourceUser` keep late changes from affecting another input's execution. Success
completion still requires the authenticated Stop, root PSP Done, corresponding
persisted final assistant, durable batch acknowledgments and a later caught-up
poll. The observer processes one native step per batch and waits for receipt
acknowledgment before moving forward. Resume finishes storage-only replay before
seeding canonical Lines. Native errors and unsupported content cannot release
that barrier by silence.

## Evidence and verification

The independent decoder's field metadata comes from the installed 1.5.10 binary's
FileDescriptorProto records. The selected field numbers and binary SHA-256 are
in [the schema fixture](../../fixtures/antigravity-store-schema.json), reproduced
by [the extraction helper](../../scripts/extract-antigravity-store-schema.py).
The helper reads only an explicitly supplied executable, never account data or
conversations. Read-only inspection of the same binary's Go function metadata and
ARM64 branches verified that `cli/steps.genericToolOutput` reads the generic
result string, while `ExtractToolInfo` independently applies `Step.GetError` and
`ErrorMessage`. `DeriveStepState` maps status 4/7 to ERROR. The former only shortens
read-file text for presentation; it does not decode auxiliary Any metadata.
These observations justify retaining complete result content without implementing
a renderer for every metadata message. The independent Rust decoder copies no
provider implementation.
The same binary identifies manual compaction through `render.isManualCheckpoint`
and `precedingCompactUserStep`; `stepcompactor.buildCheckpointStep` constructs
CHECKPOINT (23), DONE (3), with SYSTEM (5) origin. The schema fixture pins its
functional field numbers. Tests cover the metadata projection, no-model-turn
replay, causal maintenance identity and settlement without assistant completion.

Synthetic tests in `crates/protocols/src/antigravity_store/` cover framing,
identity, nonadjacent tool results, repeated text, mutable versions, binary media,
limits and usage. `terminal_agent_antigravity_store/tests.rs` covers committed
WAL reads without original-file mutation, external-output updates, path escapes,
rotation, Fresh/Resume and receipt barriers. Completion regressions verify that a
rewritten snapshot must contain its final candidate again and that a planner
still requesting tools cannot become final. Cursor's existing snapshot tests
exercise the shared copy budget and changed-WAL restart.

The hidden-system regression in `terminal_agent_antigravity_store/tests.rs`
starts from a resumed user/answer baseline, then ingests a second user, hidden
system step and answer with repeated visible text. It verifies two distinct
users and answers, the new causal user, no hidden public event and byte-identical
repeated cold replay. Actual macOS Tauri verification recovered that follow-up
after the native TUI had answered: canonical history contained two users and two
answers without duplication. Recovery preserved the one prior completion; a
new live completion after this fix was not separately exercised.

An authorized scratch smoke ran exactly `printf PROMETEU_TERMINAL_TOOL_SMOKE`
under native Ask approval. The completed CLI's exact authenticated database and
output file passed the opt-in reader/projection test: one tool, one corresponding
result and the full marker output. It did not read other sessions or establish
coverage for untested tool payloads. The earlier no-tool AGY success-fence smoke
remains separate evidence for the native completion protocol.
