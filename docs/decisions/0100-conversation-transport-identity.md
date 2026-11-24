---
type: Decision
title: "ADR 0100 — Persist conversation transport independently of provider"
description: "A default for new conversations preserves the transport and history of existing tabs."
status: stable
---

# ADR 0100 — Persist conversation transport independently of provider

Date: 2026-11-16

## Context

The native CLIs offer interactive features that the structured conversation
adapters do not expose. Selecting a native terminal must still identify an
ordinary Prometeu conversation, whose transcript survives its process. Changing
a default must not reinterpret existing transcripts or resume a different
provider session. App-owned shell sessions have a different lifetime and do not
participate in agent notifications, delegation or sharing (ADR 0096).

## Options considered

1. Reuse app-owned shell records. This provides a terminal quickly, but exit
   removes the record and bypasses conversation ownership and reactions.
2. Treat terminal as a model or provider. This mixes presentation/transport
   identity with the existing account and model catalog.
3. Persist conversation mode separately on the board and each tab. Launch
   resolves the default once; resume keeps the saved choice.

## Decision

Option 3. `ConversationMode` has `chat` and `terminal` values. Missing board and
tab values mean `chat`; that default is omitted when serializing. A launch may
override the board default, and its resolved value is saved on the tab. Resume,
including action-profile resume, preserves the tab's mode.

The saved mode also selects the local interface exclusively: Chat uses the
structured conversation and Terminal uses the native TUI. A terminal tab has no
second transcript screen or local screen switcher. The terminal fills its area
without a permanent composer, controls footer or controls floating over native
output. The workspace menu at the right of the central tab bar, tab context
menus and desk-header menus open a dialog for shared product actions, MCP,
plugins, skills, context, dictation and remote access.
Quotation and manual resume use those surrounding menus when available. Pending
context and queued messages have an indicator in the tab-bar action group or desk header,
separate from the agent's observed status, and their content is available on demand.
Actionable collaboration notices remain visible without opening that dialog so
the owner does not miss an approval. This keeps product capabilities available without maintaining a second
input surface. The CLI owns the only editable local message input and live model
and effort controls; the app keeps launch defaults without duplicating those
selectors in the conversation.

Selecting, dropping or pasting files inserts paths into that native input
without submitting it. Browser context, review notes and text previews remain
staged in the tools dialog for explicit insertion. Asynchronous preparation
captures the original tab and native process generation; failure retains that
tab's draft, and insertion never resumes or retargets a process. Canonical
history remains independent of presentation and continues to support persistence,
status, notifications and sharing. Remote members consume that canonical history
because the native PTY runs only on the owner's machine.

Attaching a visible local terminal tab automatically resumes its saved session
when its process is stopped, including after app restart. This restores the
selected interface without an extra click or a synthetic message. Resume remains
bound to the existing native identity and launch checks; concurrent surfaces do
not create duplicate processes, and failures expose manual retry rather than
restarting indefinitely. Unopened terminal tabs do not launch merely because
they exist on the board.

For the first terminal version, Cursor and Antigravity approvals, questions and
plan decisions remain in their native TUIs. Cursor's pre-execution hooks cannot
answer an already-open dialog while preserving local interaction. Antigravity's
parallel Remote Control has no documented public local attachment API. ACP owns
a separate session in both products. The chosen trade-off is native interaction
instead of recreating or instrumenting private UI and remote protocols. This
exception is scoped to saved Cursor and Antigravity terminal conversations;
Chat keeps its existing capabilities. Shared viewers receive an
encrypted per-tab decision-surface capability, and the owner rejects unsupported
responses independently of presentation. See the
[Cursor decision boundary](../contracts/cursor.md#native-decision-channel-limit)
and [Antigravity decision boundary](../contracts/native-antigravity-decisions.md).

The native PTY exposes a `ProcessHandle` for its existing child, allowing the
conversation host to reuse process identity and lifecycle rules. Its owning
waiter remains unique. OSC program-status parsing and record reduction live at
the protocol edge; they do not create assistant messages, approve tools or
infer completion from silence.

The native Codex engine owns a separate process group from its TUI. Tauri exit
does not guarantee dropping conversation references before terminating the app,
so an explicit exit hook stops and joins all engine owners. Startup admission
and exit share a barrier: already admitted bounded bootstraps cannot escape the
shutdown snapshot, and no new engine starts after exit begins. This preserves
exact-session resume without leaving a previous engine holding native resources.

Antigravity's empty startup does not provide the idle evidence observed when
starting with a native prompt argument. Its version-pinned empty composer may
admit one first input in a fresh process before any native activity. Any composer
write consumes this admission, including local input whose hook has not arrived
yet. Resume and later inputs keep native idle authority. The exception avoids
a first-input deadlock without synthesizing execution or completion events.

Observed execution crosses the conversation boundary as the additive ephemeral
`execution.observed` event. Reusing `assistant.started` would invent content;
reusing `request.opened` for a status observation would advertise an answer
route it does not prove. Genuine decision channels, such as the
[Claude PermissionRequest bridge](../contracts/native-claude-decisions.md), emit
requests only while an authenticated invocation can receive an explicit reply.
The separate read-only event lets the shared Work reducer, tab
status and notifications observe activity and blocking without either claim.
Root and background scope remain distinct so child activity cannot reopen a
completed primary turn. Loss of evidence is explicit `unknown`, published
before an uncertain background drain, never completion by absence.

Explicit root active-to-idle is availability evidence, distinct from a removed
or lost record. It settles the primary activity without a success outcome or
notification, as required by Antigravity's native Escape sequence. Source tails
remain readable without reopening that activity; active children and actual
requests still hold their own work. The existing Done completion barrier is
unchanged by this availability rule.

The content boundary also records root idle following Done/Error without closing
or consuming a pending completion ticket. Antigravity's file-exploration
interruption emits Done before Idle, so using cancellation closure alone to quiet
the source tail would reopen the backend tab on a late thinking block while the
frontend already showed idle. Availability and successful completion therefore
remain separate facts: late content stays readable, and success still requires
the provider's source evidence.

The pure execution projector returns a completion ticket for an explicit root
active-to-done/error transition. The host may commit it only after draining the
native transcript; canonical completion or resumed activity invalidates it.
Accepted input opens a new cycle, while the first input arriving behind its
initial activity claims that still-open provisional cycle. Closed cycles require
new accepted input or a distinct generation proven by the host; stale status
reports cannot create repeated completion. Native and canonical background
snapshots are merged without allowing one source to erase the other. This is
observation plumbing, not native approval or remote-control parity.

The Claude/Antigravity host also requires source-specific persistence
proof. A raw user boundary excludes earlier unread responses, and an ordinal
ties that proof to the exact PSP Done report that created the execution ticket.
The host previews completion without closing the projector, then revalidates
the ticket and records its source receipt under the canonical commit gate.
Only a successful durable commit settles and publishes the turn. A stale proof,
an already captured source or a failed commit cannot finish a newer input.

The pinned Cursor TUI uses a different successful-stop fence because it does
not emit PSP. A process-private witness captures the authorized transcript's
metadata before the hook listener acknowledges Stop. Completion requires a
later stable success trailer, the exact hook generation/native request match,
complete acknowledged source coverage and the current empty composer. The
trailer is emitted after the whole Stop hook chain; ordinary writes discard an
old trailer. Optional run records constrain the proof when present but are not
invented when the native store omits them. A source receipt commits before any
completion publication or execution change. This avoids treating either the
Stop callback or an early empty composer as the end of the turn.

The macOS execution host enables native terminal conversations for all four
providers. Settings / General exposes Chat or Terminal as the default for new
tabs, including action tabs. Existing tabs keep their saved mode. Linux retains
Chat and rejects `terminal` explicitly. The native transport includes
canonical source ingestion, durable input, explicit decision channels where
supported, shared transcript consumption and restorable screen snapshots.
The [provider matrix](../quality/provider-matrix.md) distinguishes verified macOS
Tauri flows from native CLI smokes, automated integration tests and the remaining
provider-specific conformance checks.

Slash commands and permission-mode changes belong to the native TUI for every
provider. The shared transcript UI omits those controls and validates manually typed
commands before clearing drafts, while user Actions expand first. Individual
Claude/Codex approval and question responses retain their integrated paths.
Encrypted per-tab mode metadata lets shared viewers apply the same command
boundary; the owner also rejects stale or legacy unsupported input.

Native CLI versions are gated by a minimum, never an exact build. Each
integration names the oldest build it was verified on and accepts every newer
one through `prometeu_protocols::cli_version`: semver CLIs order a prerelease of
the floor before it, and Cursor's dated builds ignore their hash. Provider CLIs ship
weekly; an exact pin broke terminal mode for everyone on each release and told
people to "update" a CLI that was already newer. The trade-off is that a newer
build can change a private format or layout. Readers of native stores, the
Codex engine bootstrap and the screen parsers still fail closed on content they
do not recognize, and such a break is fixed forward with a new release.

## Consequences

Existing boards and transcripts retain their meaning. Changing the
default affects newly created tabs only. Switching a saved tab's mode requires
an explicit history/identity conversion contract; it is not a settings side
effect.

Downgrades can discard the new fields, since older versions ignore unknown
fields. Native-history resume through an older structured adapter has not been
validated.

Raw terminal bytes and status observations do not replace the canonical
conversation stream. Native source projection owns retained content, the
emulator owns screen restoration, and verified channels own explicit controls.

The native transport remains inside `Chat`, sharing its pump, telemetry capture,
account context and process identity. It drains raw bytes independently of input
and publication locks. Native identity, output and exit wait until the tab has
been published; otherwise a fast CLI failure could report closure before its
registry entry exists. One waiter reaps the child before lifecycle callbacks.
Each PTY incarnation also has a transport generation: snapshot replay, live
output and queued input/resize cannot be mixed with a replacement process.

Codex control endpoints remain local to each derived configuration home. Sharing
durable account/session data does not imply sharing `app-server-control`: the
native engine rejects a symlink there and owns directory validation and creation.
Preparation migrates old derived aliases without modifying their original target
or any already physical control directory.

An authenticated root hook also records the exact native transcript path in a
private versioned binding. Resume reauthorizes that path against the captured
account and reads its silent baseline before spawning the CLI. Waiting for a
new hook would deadlock providers that report resume only on the next prompt;
searching all native histories would weaken session isolation. A missing or
invalid binding therefore rejects resume. Cursor can also reauthorize its exact
original session database before a derived transcript path exists. AGY 1.5.10
uses the same private DB/WAL copy boundary for its exact cascade and trajectory;
its truncated JSONL export cannot prove tool/result identity. The original
protobuf call IDs and scoped complete output files provide that evidence without
opening other sessions or changing native files. See the
[native-store contract](../contracts/native-antigravity-store.md). Baseline and
compaction projections never replace already published canonical lines or
repeat their reactions. Source-provenance recovery is composed for each provider;
unrecognized native content withholds source coverage instead of silently
discarding output or authorizing completion.

Recovery uses a separate private ingestion journal, instead of
adding provenance to public V1 rows or replacing the transcript with native
snapshots. Source occurrence/version keys preserve repeated prompts and mutable
native steps. A durable intent precedes canonical append; acknowledgment follows
canonical sync. Recovery can therefore finish an exact pending prefix without
moving existing line indices or replaying its reactions. The native host composes
the journal, native source decoders and single writer gate. It recovers
missing native tails before seeding canonical Lines; acknowledged live batches
follow the same durable commit order. Provider-specific coverage and native
control limits stay explicit in their contracts and the provider matrix.

The source commit port also receipts ephemeral metadata without retaining its
payload as a public line. It shares the ordinary output reduction path, so a
failed or duplicate receipt cannot repeat telemetry or context effects. Replay
is storage-only. Late revisions of ephemeral metadata update private receipts;
retained summary/notice/report text uses quiet append without execution or
reaction effects. This preserves the append-only history and the default Chat
policy while making native source capture idempotent.

AGY's source-recognized native `/compact` is maintenance rather than a model
turn. Its causally bound terminal checkpoint, durable source catch-up and PSP
Idle/Done/Error can settle `execution.observed` to idle without `turn.completed`
or a completion alert. Replay and `context.compacted` metadata do not open a
model cycle. The maintenance marker is private and optional; native user/source
identities and canonical history remain append-only.

The private journal uses schema 4 and can begin with an absent native identity.
An authenticated binding records that identity once, rejecting a different one.
This supports CLIs that allocate their identity after launch while preserving
the durable receipt for the first app input. Schemas 1–3 migrate without changing
canonical bytes or receipts. Resume can recover a committed native identity
before its board update; Fresh never adopts an old bound journal. Recovery still
reauthorizes the captured account's exact native source.

Sealed Claude rows and Antigravity steps do not prove a completed turn. Private
causal-user receipts distinguish current output from historical revisions;
completion is indexed only after its canonical bytes commit. A later committed
input range also prevents old results from affecting its successor, including
an interrupted turn that never produced a completion. Original ranges remain
stable across aliases and replay. Unknown ancestry cannot authorize live work,
and the Antigravity trajectory identity is separately pinned across restart.
These rules stay at the ingestion boundary; no provider provenance enters V1.

Native queued input also needs durable identity before process writes. An
additive `Tab.native_dispatch` holds one immutable `{id,text}` item separately
from later `pending_prompt` text. A board barrier commits that item before
dispatch. The private journal then persists a prepared input intent before any
PTY write, and the canonical acceptance receipt precedes clearing the board
slot. This covers both crash windows: write before receipt and receipt before
board clear. A restored exact receipt returns `AlreadyAccepted` without another
write or capture. An unresolved intent requires causal native acceptance proof;
silence, absent source rows and matching text elsewhere never authorize retry.
The original text stays visible while uncertain. Ordinary Chat retains its
existing queue and persistence policy.

The portable pump exposes `feed_committed` for this ordering without importing
native source formats or SQLite into the core. Its injected callback commits
sanitized public bytes before execution observations, delivery, telemetry or
reactions. A duplicate durable occurrence and a failed commit cause no live
effects. This explicit ingestion path leaves ordinary chat's existing
best-effort persistence policy intact; native recovery cannot safely publish an
occurrence whose durable ownership is still uncertain. The corresponding pump
tests verify both policies and prove that committed bytes are not appended twice.
The optional commit port also covers app echoes and notices. Proven revisions
of finished native content publish updates without rearming execution or
notifications. Causal input aliases use opaque receipts and native user
occurrences, including Cursor's proven paste normalization, rather than global
text deduplication.

## Evidence

- Core board and launch tests cover old data, explicit overrides and independent
  board/tab values; desktop session tests cover saved mode on resume.
- `crates/process/src/terminal/tests.rs` checks input, resize, process control,
  exit state and the single owning waiter with real PTYs.
- `crates/protocols/src/terminal_status.rs` and `terminal_status/state.rs` check
  bounded byte parsing, chunk boundaries, explicit lifecycle and child records.
- `crates/protocols/src/terminal_execution/tests.rs`, core Work/session tests
  and `src/alert.test.ts` check completion barriers, late first input, child
  holds, lost evidence and repeated notifications without fabricated messages.
- `src-tauri/src/terminal_agent_launch.rs` tests native argument construction;
  these are not real-provider inference or end-to-end conformance tests.
- `crates/protocols/src/cursor_store/tests.rs` and
  `src-tauri/src/terminal_agent_cursor_store/tests.rs` cover stable native IDs,
  bounded protobuf decoding and private copied DB/WAL reads. The journal tests
  cover exact-prefix recovery, source versions and distinct repeated inputs.

See the [runtime contract](../contracts/agent-runtime.md),
[persistence contract](../contracts/persistence.md) and
[provider matrix](../quality/provider-matrix.md).
