---
type: Decision
title: "ADR 0100 — Persist conversation transport independently of provider"
description: "A default for new conversations does not change the transport and the history of the current tabs."
status: stable
---

# ADR 0100 — Persist conversation transport independently of provider

Date: 2026-11-16

## Context

The native CLIs have interactive features that the structured conversation
adapters do not show. When a person selects a native terminal, Prometeu must
continue to identify a usual Prometeu conversation. The transcript of that
conversation stays after its process stops. A change to a default must not
change the meaning of current transcripts. It must not resume the session of a
different provider. The shell sessions that the app owns have a different
lifetime. They are not part of agent notifications, delegation or sharing
(ADR 0096).

## Options considered

1. Use the shell records that the app owns again. This option gives a terminal
   quickly. But an exit removes the record, and the record does not use the
   ownership and reactions of a conversation.
2. Make the terminal a model or a provider. This option puts the identity of
   the presentation and transport together with the current account and model
   catalog.
3. Keep the conversation mode independently on the board and on each tab. The
   launch finds the default one time. A resume keeps the saved selection.

## Decision

Use option 3. `ConversationMode` has the values `chat` and `terminal`. When the
board or the tab does not have a value, the value is `chat`. Serialization does
not write that default. A launch can replace the default of the board, and
Prometeu saves the value that it found on the tab. A resume keeps the mode of
the tab. This rule includes the resume of an action profile.

**Interface.** The saved mode also selects the local interface. Chat uses the
structured conversation, and Terminal uses the native TUI. A terminal tab does
not have a second transcript screen or a local control to change the screen.
The terminal fills its area. It does not have a permanent composer, a footer
with controls or controls above the native output.

- The workspace menu at the right of the central tab bar opens a dialog. The
  context menus of tabs and the menus of the desk header also open it. This
  dialog contains the shared product actions, MCP, plugins, skills, context,
  dictation and remote access.
- Quotation and manual resume use those menus when they are available.
- Pending context and queued messages have an indicator in the action group of
  the tab bar or in the desk header. This indicator is separate from the agent
  status that Prometeu sees. The person can open their content.
- The person can see collaboration notices that are necessary for an action
  without that dialog.
  Thus the owner sees each approval.

Thus the product capabilities stay available, and it is not necessary for Prometeu to
keep a second input surface. The CLI owns the only local message input that a
person can edit. It also owns the live model and effort controls. The app keeps
the launch defaults, but it does not show those selectors again in the
conversation.

**Files and context.** When a person selects, drops or pastes files, Prometeu
puts their paths into that native input. It does not send the input. Browser
context, review notes and text previews stay in the tools dialog until the
person inserts them explicitly. Asynchronous preparation records the initial
tab and the native process generation. If a failure occurs, that tab keeps its
draft. The insertion does not resume a process or send text to a different
process at any time.

The canonical history does not depend on the presentation. It continues to
operate for persistence, status, notifications and sharing. Remote members use
that canonical history, because the native PTY runs only on the machine of the
owner.

**Automatic resume.** When a local terminal tab that the person sees attaches,
and its process does not operate, Prometeu resumes its saved session automatically. This also
occurs after the app starts again. Thus the selected interface comes back
without one more click or a synthetic message. The resume stays bound to the
current native identity and to the launch checks. Surfaces that operate at the
same time do not make two processes. A failure shows a manual retry. Prometeu
does not start the process again and again without a limit. Terminal tabs that
the person did not open do not launch only because they are on the board.

**Native decisions of Cursor and Antigravity.** In the first terminal version,
some decisions of Cursor and Antigravity stay in their native TUIs. These are
the approvals, the questions and the plan decisions.

- The pre-execution hooks of Cursor cannot answer a dialog that is open and also
  keep the local interaction.
- The parallel Remote Control of Antigravity has no public local attachment API
  with documentation.
- In the two products, ACP owns a different session.

This decision keeps the native interaction. As a result, Prometeu does not make
again or instrument the private UI and remote protocols. This special rule applies
only to saved terminal conversations of Cursor and Antigravity. Chat keeps its
current capabilities. Shared viewers receive an encrypted decision-surface
capability for each tab. The owner rejects responses that the tab cannot
accept, and the presentation has no effect on this rule. See the
[Cursor decision boundary](../contracts/cursor.md#native-decision-channel-limit)
and [Antigravity decision boundary](../contracts/native-antigravity-decisions.md).

**Process identity.** The native PTY gives a `ProcessHandle` for its current
child process. Thus the conversation host can use the process identity and the
lifecycle rules again. Only one waiter owns the process. The parse of the OSC
program status and the record reduction are at the protocol edge. They do not
make assistant messages, approve tools or find a completion from silence.

**Codex engine.** The native Codex engine owns a process group that is
different from the process group of its TUI. When Tauri exits, it does not
always drop the conversation references before it stops the app. Thus an
explicit exit hook stops all engine owners and waits for them. The startup admission and
the exit share a barrier. Bounded bootstraps that Prometeu accepted cannot go
out of the shutdown snapshot. No new engine starts after the exit starts. Thus
the resume of the accurate session operates, and no previous engine keeps
native resources.

**Antigravity startup.** When Antigravity starts empty, it does not give the
idle evidence that Prometeu sees when it starts with a native prompt argument.
Its empty composer, for the pinned version, can accept one first input in a new
process before native activity occurs. Each composer write uses this admission.
This includes local input whose hook did not come at that time. A resume and
subsequent inputs keep the native idle authority. This special rule prevents a
deadlock at the first input. It does not make synthetic execution or
completion events.

**Execution events.** Execution that Prometeu sees goes through the
conversation boundary as the additive ephemeral `execution.observed` event.

- If Prometeu uses `assistant.started` again, it makes content that is not
  there.
- If Prometeu uses `request.opened` for a status, it advertises an answer
  path that it cannot prove.

Correct decision channels, for example the
[Claude PermissionRequest bridge](../contracts/native-claude-decisions.md),
send requests only while an authenticated invocation can receive an explicit
reply. The separate read-only event lets the shared Work reducer, the tab status
and notifications see activity and blocks without one of those claims. The
root scope and the background scope stay different. Thus child activity cannot
open a completed primary turn again. When the evidence is lost, the status is an explicit
`unknown`. Prometeu publishes it before an uncertain background drain. An
absence of evidence is not a completion at any time.

**Idle after activity.** An explicit change of the root from active to idle is
evidence of availability. It is different from a record that Prometeu removed
or lost. It stops the primary activity without a success outcome or a
notification. The native Escape sequence of Antigravity makes this necessary.
The source tails stay readable, and they do not open that activity again.
Active children and requests that are there continue to hold their own work. This
availability rule does not change the current Done completion barrier.

The content boundary also records a root idle that follows Done or Error. It
does not close or use a pending completion ticket. When Antigravity interrupts
a file exploration, it sends Done before Idle. Prometeu does not use only the
closure of the cancellation to stop the source tail. If it did, a late thinking
block can open the backend tab again while the frontend shows idle. Thus availability and
correct completion stay separate facts. Late content stays readable, and a
success continues to have the source evidence as a condition of the provider.

**Completion tickets.** The pure execution projector returns a completion
ticket for an explicit change of the root from active to done or error. The
host can commit the ticket only after it drains the native transcript. A
canonical completion or a resumed activity makes the ticket not valid.

- Accepted input opens a new cycle. The first input that comes after its
  initial activity takes that provisional cycle that is open.
- A closed cycle must have new accepted input or a different generation that the
  host proves. Status reports that are not current cannot make a second
  completion.
- Prometeu merges native and canonical background snapshots. One source cannot
  erase the other.

This is plumbing for observation. It does not give native approval or the same
functions as remote control.

**Claude and Antigravity proof.** The host for Claude and Antigravity also
must have a persistence proof for each source. A raw user boundary does not include
earlier responses that the person did not read. An ordinal connects that proof
to the accurate PSP Done report that made the execution ticket. The host shows
the completion before it closes the projector. Then it validates the ticket
again and records its source receipt under the canonical commit gate. Only a
durable commit that is correct settles and publishes the turn. A proof that is
not current, a source that Prometeu captured before, or a commit with a failure
cannot complete a newer input.

**Cursor proof.** The pinned Cursor TUI uses a different fence for a correct
stop, because it does not send PSP. A witness that is private to the process
records the metadata of the authorized transcript before the hook listener
acknowledges Stop. A completion must have these items:

- A subsequent stable success trailer.
- The hook generation and the native request agree accurately.
- Full acknowledged source coverage.
- The current empty composer.

Cursor writes the trailer after the full Stop hook chain. Usual writes discard
an earlier trailer. Optional run records limit the proof when they are there.
Prometeu does not make them when the native store does not have them. A source
receipt commits before the publication of a completion or a change of the
execution. Thus Prometeu does not use the Stop callback or an early empty
composer as the end of the turn.

**Platforms.** The macOS execution host enables native terminal conversations
for all four providers. Settings / General shows Chat or Terminal as the
default for new tabs. This includes action tabs. Current tabs keep their saved
mode. Linux keeps Chat and rejects `terminal` explicitly. The native transport
includes these items:

- Canonical source ingestion and durable input.
- Explicit decision channels where they are available.
- Shared transcript consumption and screen snapshots that Prometeu can
  restore.

The [provider matrix](../quality/provider-matrix.md) shows the difference
between verified macOS Tauri flows, native CLI smokes, automated integration
tests and the provider-specific conformance checks that stay.

**Slash commands.** Slash commands and permission-mode changes are part of the
native TUI for all providers. The shared transcript UI does not show those
controls. It validates commands that the person types manually before it
clears drafts. User Actions expand first. The approval and question responses
of Claude and Codex keep their integrated paths. Encrypted mode metadata for
each tab lets shared viewers apply the same command boundary. The owner also
rejects input that is not current or that a legacy version cannot accept.

**CLI versions.** A minimum version controls the native CLIs, not an accurate
build. Each integration names the earliest build that the team verified. It
accepts all newer builds through `prometeu_protocols::cli_version`:

- For semver CLIs, a prerelease of the minimum comes before the minimum.
- For the dated builds of Cursor, Prometeu ignores the hash.

Providers publish new CLIs each week. An accurate pin stopped the terminal mode
for all persons at each release. It also told persons to "update" a CLI that was
newer at that time. As a result of this decision, a newer build can change a
private format or layout. The readers of native stores, the Codex engine
bootstrap and the screen parsers continue to fail closed on content that they
do not know. The team repairs such a break with a new release.

## Consequences

The meaning of current boards and transcripts does not change. A change to the
default has an effect only on new tabs. To change the mode of a saved tab, an
explicit conversion contract for the history and the identity is necessary. It
is not a side effect of a setting.

A downgrade can discard the new fields, because earlier versions ignore unknown
fields. The team did not validate a native-history resume through an earlier
structured adapter.

Raw terminal bytes and status observations do not replace the canonical
conversation stream. The native source projection owns the content that
Prometeu keeps. The emulator owns the screen restoration. Verified channels own
the explicit controls.

**Transport in the chat.** The native transport stays in `Chat`. It shares the
pump, the telemetry capture, the account context and the process identity of
the chat. It drains raw bytes independently of the input locks and the
publication locks. The native identity, output and exit wait until Prometeu
publishes the tab. If not, a fast CLI failure can report a closure before its
registry entry is there. One waiter reaps the child process before the
lifecycle callbacks. Each PTY incarnation also has a transport generation.
Thus snapshot replay, live output and queued input or resize cannot go to a
replacement process.

**Codex control endpoints.** Codex control endpoints stay local to each derived
configuration home. When Prometeu shares durable account and session data, it
does not share `app-server-control`. The native engine rejects a symlink at
that location. It owns the validation and the creation of the folder. The
preparation migrates derived aliases from earlier versions. It does not change
their initial target or a physical control folder that is there before.

**Transcript binding.** An authenticated root hook also records the accurate
native transcript path in a private versioned binding. A resume authorizes that
path again against the captured account. It reads its silent baseline before it
starts the CLI.

- Prometeu does not wait for a new hook. If it did, providers that report a
  resume only on the next prompt stop in a deadlock.
- Prometeu does not examine all native histories. If it did, the session
  isolation becomes less strong.

Thus a binding that is not there or that is not valid rejects the resume.
Cursor can also authorize again its accurate initial session database before a
derived transcript path is there. AGY 1.5.10 uses the same private DB/WAL copy
boundary for its accurate cascade and trajectory. Its truncated JSONL export
cannot prove the identity of a tool or a result. The initial protobuf call IDs
and the full output files in the scope give that evidence. They do not open
other sessions or change native files. See the
[native-store contract](../contracts/native-antigravity-store.md). Baseline and
compaction projections do not replace canonical lines that Prometeu published
before, and they do not repeat their reactions. The source-provenance recovery
has a composition for each provider. Native content that Prometeu does not know
does not give source coverage. Prometeu does not discard the output or
authorize a completion without a message.

**Ingestion journal.** The recovery uses a separate private ingestion journal.
Prometeu does not add provenance to public V1 rows. It does not replace the
transcript with native snapshots. Source occurrence keys and version keys keep
repeated prompts and native steps that can change.

- A durable intent comes before the canonical append.
- The acknowledgment comes after the canonical sync.

Thus the recovery can complete an accurate pending prefix. It does not move
current line indices or replay their reactions. The native host composes the
journal, the native source decoders and the single writer gate. It collects
the missing native tails before it seeds canonical Lines. Acknowledged live batches
follow the same durable commit order. The contracts and the provider matrix
show the provider-specific coverage and the native control limits explicitly.

**Ephemeral metadata.** The source commit port also records receipts for
ephemeral metadata. It does not keep the payload of the metadata as a public
line. It shares the usual output reduction path. Thus a receipt with a failure
or a duplicate receipt cannot repeat telemetry or context effects. A replay
only writes to the storage. Late revisions of ephemeral metadata update private
receipts. Summary, notice and report text that Prometeu keeps uses a quiet
append without execution or reaction effects. Thus the history stays
append-only, the default Chat policy stays, and the native source capture is
idempotent.

**AGY compaction.** A native `/compact` of AGY that the source identifies is
maintenance, not a model turn. Its terminal checkpoint, which has a causal
binding, the durable source catch-up and PSP Idle, Done or Error can settle
`execution.observed` to idle. This occurs without `turn.completed` and without
a completion alert. Replay and `context.compacted` metadata do not open a model
cycle. The maintenance marker is private and optional. Native user and source
identities and the canonical history stay append-only.

**Journal schema.** The private journal uses schema 4. It can start without a
native identity. An authenticated binding records that identity one time and
rejects a different identity. Thus Prometeu can operate with CLIs that make
their identity after the launch, and it keeps the durable receipt for the first
app input. Schemas 1–3 migrate without a change to canonical bytes or
receipts. A resume can recover a committed native identity before the board
update for it. Fresh does not use a bound journal from an earlier session at
any time. The recovery continues to authorize again the accurate native source
of the captured account.

**Completed turns.** Sealed Claude rows and Antigravity steps do not prove a
completed turn. Private causal-user receipts show the difference between
current output and historical revisions. Prometeu indexes a completion only
after its canonical bytes commit. A subsequent committed input range also
prevents earlier results from an effect on the next turn. This includes an
interrupted turn that did not make a completion. Initial ranges stay stable
through aliases and replay. An unknown ancestry cannot authorize live work.
Prometeu pins the Antigravity trajectory identity independently through a restart.
These rules stay at the ingestion boundary. No provider provenance goes into
V1.

**Queued native input.** Native queued input must also have a durable
identity before the process writes. An additive `Tab.native_dispatch` holds one
immutable `{id,text}` item, independently from the subsequent `pending_prompt`
text.

1. A board barrier commits that item before the dispatch.
2. The private journal then keeps a prepared input intent before a PTY write.
3. The canonical acceptance receipt comes before Prometeu clears the board
   slot.

This sequence covers the two crash windows: a write before the receipt, and a
receipt before Prometeu clears the board. A restored accurate receipt returns
`AlreadyAccepted` without one more write or capture. An unresolved intent must have
a causal proof that the native agent accepted the input. Silence, missing
source rows and the same text in a different location do not authorize a retry
at any time. The person continues to see the initial text while the status is uncertain. The
usual Chat keeps its current queue and persistence policy.

**Committed feed.** The portable pump gives `feed_committed` for this sequence.
It does not import native source formats or SQLite into the core. Its injected
callback commits sanitized public bytes before execution observations,
delivery, telemetry or reactions. A duplicate durable occurrence and a commit
with a failure cause no live effects. This explicit ingestion path does not
change the best-effort persistence policy of the usual chat. Native recovery
cannot safely publish an occurrence while its durable ownership is uncertain.
The related pump tests make sure that the two policies operate. They also prove
that Prometeu does not append committed bytes two times. The optional commit
port also covers app echoes and notices. Proven revisions of completed native
content publish updates, but they do not start the execution or the
notifications again. Causal input aliases use opaque receipts and native user
occurrences, for example the proven paste normalization of Cursor. They do not
use a global deduplication of text.

## Evidence

- Core board and launch tests cover old data, explicit overrides and
  independent values for the board and the tab. Desktop session tests cover the
  saved mode on resume.
- `crates/process/src/terminal/tests.rs` examines input, resize, process
  control, exit state and the single owner waiter with real PTYs.
- `crates/protocols/src/terminal_status.rs` and `terminal_status/state.rs`
  examine the parse of bounded bytes, chunk boundaries, the explicit lifecycle
  and child records.
- `crates/protocols/src/terminal_execution/tests.rs`, the core Work and session
  tests and `src/alert.test.ts` examine completion barriers and a late first
  input. They also examine child holds, lost evidence, and notifications that
  occur again without synthetic messages.
- `src-tauri/src/terminal_agent_launch.rs` tests the construction of native
  arguments. These tests are not real-provider inference tests or end-to-end
  conformance tests.
- `crates/protocols/src/cursor_store/tests.rs` and
  `src-tauri/src/terminal_agent_cursor_store/tests.rs` cover stable native IDs,
  bounded protobuf decoding and private copied DB/WAL reads. The journal tests
  cover the recovery of an accurate prefix, source versions and different
  repeated inputs.

See the [runtime contract](../contracts/agent-runtime.md),
[persistence contract](../contracts/persistence.md) and
[provider matrix](../quality/provider-matrix.md).
