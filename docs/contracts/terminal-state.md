---
type: Contract
title: "Native terminal state helper"
description: "Ordered emulation, generation-scoped snapshots and terminal query ownership."
status: stable
---

# Native terminal state helper

`packages/terminal-state` implements a private, provider-neutral emulation
service. It consumes PTY bytes; it does not spawn the provider, read its
transcript, change the board or infer agent activity. The macOS native transport
owns one helper per native PTY, applies its bytes before publishing live output,
and exposes its ordered screen snapshot. Linux release and WSL do not consume
the helper. Native conversation mode is selectable on macOS; helper and CLI
tests remain distinct from full Tauri app conformance. See
[conversation transport identity](../decisions/0100-conversation-transport-identity.md)
and [agent runtime](agent-runtime.md).

The implementation pins `@xterm/headless` and the renderer package to `6.0.0`,
with `@xterm/addon-serialize` `0.14.0` from the same upstream release. It uses the
same default Unicode provider. Version-specific buffer, hyperlink and mode
access stays in `src/xterm-state.ts`; the rest of the host uses the protocol.

## Framing and identity

The helper reads UTF-8 NDJSON on stdin and writes only NDJSON responses to
stdout. Requests carry `v: 1`, a correlation `id`, `session`, `generation` and
`op`. Identifiers are 1–128 ASCII letters, digits, dots, underscores, colons or
hyphens. The host generates a fresh, never-reused generation for each PTY
incarnation. A helper restart does not create evidence that an old screen was
recovered.

`src/types.ts` is the browser-safe typed source; `src/protocol.ts` validates requests and limits. Successful responses are
`{v:1,id,ok:true,result}`; failures are
`{v:1,id,ok:false,error:{code}}`. Malformed input may have `id: null`. Error
responses contain no terminal contents, paths or payload excerpts. The error
vocabulary is `invalid_request`, `limit`, `not_found`, `stale_generation`,
`sequence`, `already_exists`, `snapshot_unavailable`, `closed` and `internal`.

| Operation | Additional fields | Result and rule |
| --- | --- | --- |
| `create` | `cols`, `rows`, optional `scrollback` | `{seq:0}`; refuses an existing session, including a different generation |
| `write` | canonical base64 `data`, `seq` | `{seq,replies:string[]}`; `seq` must equal previous seq plus decoded byte length |
| `resize` | `cols`, `rows`, `seq` | `{seq}`; requires the current byte sequence and preserves pending decoder/parser state |
| `snapshot` | optional boolean `view` | the structured snapshot below, after preceding writes and resizes finish; with `view: true`, buffer `ansi` is empty and `links` is `[]` |
| `dispose` | none | `{seq}`; disposes only the matching generation |

All operations share one ordered queue, including create/dispose, resize and
snapshot barriers. A stale generation cannot write, resize, snapshot or dispose
its replacement. A byte gap or duplicate nonempty write is rejected. The host
must not retry uncertain writes: reestablish a snapshot boundary or retire the
generation. An `internal` error is not an assertion that an operation had no
effect; the host must retire that helper state.

Stdio applies backpressure by awaiting each parser operation and stdout drain.
It accepts a final complete JSON line without a newline. A malformed JSON line
gets an error and processing continues; an oversized frame gets a `limit`
response and closes that helper stream. EOF disposes the emulators after
queued work completes. The in-process service exposes the same ordering.

## Screen snapshot and replay

The snapshot carries `v`, `session`, `generation`, `seq`, dimensions,
`scrollback`, active buffer, separate normal/alternate `Screen` records,
`modes`, `rehydrate`, `title`, `titleTruncated`, OSC8 `links`, `viewportLines`,
`viewportCharacterColumns`, `viewportStyles`, `appearance`, `pendingEscape`
and base64 `pendingUtf8`. A `Screen` contains ANSI contents, cursor position,
wrap-pending state, scroll margins, saved cursor position/SGR, and the final
cell needed to restore a pending wrap. `alternate` is null while normal is
active; xterm clears its alternate buffer when leaving it.

`viewportLines` contains exactly `rows` strings from the active buffer at
`baseY + row`, using `translateToString(true)`. It excludes scrollback and
trailing unoccupied cells while preserving explicit spaces. Consumers can
inspect a current native prompt without reparsing ANSI. This content has the
same privacy boundary as the screen and must not enter logs or telemetry.

`viewportCharacterColumns` maps every Unicode scalar in each `viewportLines`
string to its zero-based xterm column. It has exactly `rows` arrays, each with
the same length as its line's scalar count; columns are nondecreasing and below
`cols`. Scalars combined into one cell repeat its column; a wide character has
one entry at its starting column. This records the pinned renderer's actual
widths, rather than a separate Unicode-width approximation. The total map is
bounded to 262,144 entries; exceeding it fails the snapshot explicitly. The
native UI edge can locate a painted cursor amid emoji or combining text.

`viewportStyles` exposes dim/inverse flags and foreground/background colors as
contiguous runs per active viewport row. Each run has zero-based `start` and
exclusive `end` columns; runs cover `[0, cols)` and the outer array has `rows`
entries. Color mode is `default` with value `-1`, `palette` with index `0..255`,
or `rgb` with value `0..0xffffff`. Attributes come from xterm cells, including
wide-cell continuation columns. The existing snapshot size limit bounds this
metadata too. The native UI edge can distinguish painted placeholder styling
from literal input with identical text without decoding provider ANSI itself.

Normal ANSI includes retained scrollback. Links are serialized into OSC8
sequences as well as exposed as ranges, so a reconstructed emulator still owns
the links after subsequent output and resize. An open hyperlink pen is restored
for following bytes. Its implicit underline is not converted into an explicit
underline that would leak after link closure. The saved cursor keeps position
and ordinary SGR attributes; saved charset state is outside this version.

Modes cover application cursor/keypad, bracketed paste, insertion, origin,
reverse wrap, focus reporting, wraparound, mouse protocol and SGR/SGR-pixel encoding,
cursor visibility, style and blink. This matches the pinned xterm renderer's
current input modes; adding a newer keyboard protocol requires upgrading and
validating the renderer and helper together.

The browser-safe `src/replay.ts` builds the restoration sequence. Its caller
must perform these steps in order:

1. Subscribe to live output and hold matching-generation frames.
2. Obtain the snapshot, drain old parser writes, reset xterm and apply the
   snapshot dimensions and scrollback policy.
3. Disable all writes back to the PTY while parsing the complete replay. Restore
   normal/alternate buffers, saved/current cursor and modes, then title and the
   incomplete escape suffix last. Await xterm's parser callback.
4. Seed a fresh streaming UTF-8 decoder with `pendingUtf8`, without displaying
   a replacement character. These pending bytes already count toward `seq`.
5. Apply only live output after `seq` from the same generation; never treat a
   sequence number as a process identity. Resize again only at the corresponding
   ordered host boundary if the current viewport requires another size.

An incomplete CSI/OSC can cross any PTY chunk boundary. The escape tracker
retains it so the next live byte completes the original sequence. C0 controls
already executed inside an incomplete CSI/ESC are not executed again during
replay. UTF-8 keeps at most three incomplete bytes; invalid sequences use the
standard decoder replacement behavior.

The helper preserves the screen represented by this contract, rather than
exporting every private xterm object. Custom charset registers, custom tab
stops, arbitrary addon state, indexed palette mutations and newer protocols absent from
the pinned renderer are not reconstructed here. Restoration guarantees are
limited to the state represented by this contract and the replay tests below;
they do not certify every native CLI interaction.

## Terminal query ownership

The headless emulator is the single query-response authority. Only live
`write` operations capture xterm's `onData`; their result contains base64
`replies`. Snapshot and resize never send input themselves. The host forwards
each returned reply once to the matching generation's PTY, after accepting the
write result. No unsolicited reply event exists.

Both replayed and live renderer-generated automatic replies must be suppressed
when this helper owns replies. User keyboard/paste input still reaches the PTY
through the existing authority. Keeping the old generic xterm `onData` sink
unchanged would duplicate live replies and is therefore not a valid integration.
Native foreground, background and cursor defaults come from the browser-safe
`src/appearance.ts`, shared with the native renderer. The helper consumes xterm's
parsed OSC10/11/12 requests, answers reports with those current colors and
tracks set/reset operations. `appearance` records the resulting colors and
replay restores them before the screen. Indexed palette and pixel-geometry
queries remain unsupported; the helper does not invent renderer measurements.

Native conversation presentation uses a neutral dark background (`#282c34`),
white foreground and cursor, and a complete ANSI theme with blue selection
(`#5a7898`). Its macOS font starts with SF Mono at 14 px, normal/bold weights
500/700 and line height 1, with system monospace fallbacks. These are visual
defaults over the pinned xterm 6.0.0 renderer, drawn by its WebGL addon with a
DOM fallback. Explicit native OSC color changes remain authoritative through replay.
A helper already running retains its current colors until its normal
replacement; rebuilding the packaged executable does not rewrite live state or
restart a provider to apply a theme.

In the pinned renderer, keyboard, IME, paste and SGR mouse data are marked as
user input by `CoreService.triggerDataEvent`. Focus notifications are not marked
that way and must still be forwarded as view events. Default mouse encoding uses
`onBinary`, whose byte-valued string requires a byte-preserving IPC path rather
than UTF-8 encoding. `src/terminal-input.ts` wraps the pinned core's synchronous
user-input marker; `Term` suppresses automatic replies, forwards focus and raw
binary input, and serializes input promises with captured session/generation.
An attachment change rejects queued obsolete input. Blocking all `onData` while
live output parses would lose concurrent input.

## Desktop lifecycle

`chat/terminal/screen.rs` orders writes, resizes and snapshots under one emulator
mutex outside the conversation capture/PTY output locks. Native resize runs
inside that same boundary. Helpers consume byte offsets; the desktop wrapper
also retains the corresponding complete PTY chunk sequence for IPC. The empty
process-exit chunk consumes a chunk sequence too. Readiness and controls read
view snapshots: serializing the whole history takes tens of milliseconds and
holds the mutex the PTY reader needs, so only restoring a view and freezing ask
for the full snapshot. A cached full snapshot also answers a view, never the
reverse. After exit, the host freezes
the last snapshot and disposes/reaps the helper, so closed conversation tabs
do not retain one Node process each. A helper failure refuses restoration and
stops that native process; it never presents the raw tail as a successfully
restored full screen. Its native transcript can be resumed.

Release resolution uses the packaged executable beside the app binary. Debug
accepts the explicit `PROMETEU_TERMINAL_STATE_HELPER` path supplied by the
development script. Neither path discovers or requires a customer-installed
Node. The desktop screen lifecycle's ignored test exercises the actual packaged
helper, resize, query replies, UTF-8 continuation and frozen snapshots.
The SEA builder signs and verifies a temporary executable before atomically
replacing the destination. Subsequent launches use that executable; existing
helper processes continue with their original code and state.

## Limits and evidence

Defaults and enforced bounds are explicit in `LIMITS`: 32 sessions, 500 columns,
300 rows, 5,000 scrollback rows (default 1,000), one million nominal cells per
session and four million in total, accounting for both screen buffers. Writes
are at most 256 KiB decoded, input lines 512 KiB, and snapshots 16 MiB JSON.
The library queue accepts at most 128 pending requests. Grid admission and
resize limits are checked before mutation.

An incomplete escape over 4,096 characters makes snapshots unavailable until
the sequence terminates; emulation continues. The tail is never silently
truncated into a valid restore point. Title metadata is explicitly truncated
at 4,096 characters and marked. Large repeated OSC8 URLs are bounded before
ANSI expansion; snapshots over the result limit are refused. These are screen
retention limits, not transcript truncation policies.

`test/state.test.ts` compares cells, colors, styles, cursor, buffers, links,
modes and continued output against a second headless instance after restoration.
Cases include alternate screen exit, saved cursor, margins, wrap-pending wide
characters, hyperlinks in scrollback, split CSI/OSC/UTF-8, resize/reflow and
query replies. `test/protocol.test.ts` covers the real stdio executable,
concurrent request ordering, generation replacement, byte gaps, duplicate
writes and admission limits. Provider differences remain in the
[provider matrix](../quality/provider-matrix.md); these tests invoke no agent.

Run `npm ci --prefix packages/terminal-state`, then the package's `test` and
`typecheck` scripts. `build:sea` downloads and verifies the pinned Node 24.14.0
archive, bundles the helper and dependencies, injects its SEA blob into that
same runtime, and records archive/executable hashes. V8 keeps its JIT: every PTY
chunk waits for the helper, and `--jitless` made writes and snapshots about 3.5x
slower. On macOS the build signs it with `src-tauri/Entitlements.plist`, the same
file Tauri applies when it re-signs the bundled sidecar, which grants
`com.apple.security.cs.allow-jit` under the hardened runtime. Runtime argument
extension is disabled.
`test:sea` runs with an empty PATH and invalid injected Node options. Native
macOS arm64 is locally verified; the Linux x64 recipe is not locally verified.

On macOS, `npm run app`, `npm run build:app` and `npm run app:bundle` use
`scripts/terminal-state-app.mjs`: it installs the helper's own locked dependencies,
builds the SEA, and adds `tauri.terminal-state.conf.json` to the Tauri invocation.
The overlay bundles the executable and licenses; development supplies its explicit path.
A local debug `.app` bundle passed strict signature verification and executed
the bundled helper without installed Node. The current helper is about 113 MiB
before compression. `npm run terminal-state:check`, included after the frontend
build in `npm run check`, validates the package, SEA and exact host wire/screen
tests without invoking an agent. Linux CI runs that gate too; Linux release and
WSL do not yet consume the helper. The macOS release build uses the same wrapper,
then verifies the packaged executable and notices before notarizing the DMG.
Native conversation mode is enabled separately by the macOS execution host and
Settings capability. Packaging tests do not establish full application
conformance or authorize publication.

The incomplete-escape scanner is adapted from Orca commit
`6ee09ca1c9901c58e45df966cadd6f2a86e268f0`; its MIT license and attribution are
retained in `packages/terminal-state/THIRD_PARTY_NOTICES.md`. The sidecar overlay
includes those notices and the Node/xterm licenses.
