---
type: Contract
title: "Native terminal state helper"
description: "The emulation in sequence, the snapshots for each generation and the ownership of terminal queries."
status: stable
---

# Native terminal state helper

`packages/terminal-state` is a private emulation service. It is the same for all
providers. It receives PTY bytes. It does not start the provider, read its
transcript, change the board or find the activity of the agent. The native macOS
transport has one helper for each native PTY. The transport applies the bytes to
the helper before it publishes the live output. It also gives the screen
snapshot of the helper in sequence. The Linux release does not use the helper.

A person can select the native conversation mode on macOS. The helper tests and
the CLI tests do not prove that the full Tauri app agrees with this contract.
See [conversation transport
identity](../decisions/0100-conversation-transport-identity.md) and [agent
runtime](agent-runtime.md).

The implementation pins `@xterm/headless` and the renderer package to `6.0.0`.
It pins `@xterm/addon-serialize` to `0.14.0` from the same upstream release. It
uses the same default Unicode provider. The code that reads the buffer, the
hyperlinks and the modes changes with the xterm version. This code stays in
`src/xterm-state.ts`. The remaining code of the host uses the protocol.

## Framing and identity

The helper reads UTF-8 NDJSON on stdin. It writes only NDJSON responses to
stdout. Each request has `v: 1`, a correlation `id`, `session`, `generation` and
`op`. An identifier has 1 to 128 characters. The permitted characters are ASCII
letters, digits, dots, underscores, colons and hyphens. The host makes a new
generation for each PTY incarnation and does not use it again. A new start of
the helper does not prove that the helper got a previous screen back.

`src/types.ts` is the typed source that a browser can use. `src/protocol.ts`
validates requests and limits. A correct response is `{v:1,id,ok:true,result}`.
A failure is `{v:1,id,ok:false,error:{code}}`. A response to malformed input can
have `id: null`. Error responses do not contain terminal contents, paths or text
from payloads. The error codes are `invalid_request`, `limit`, `not_found`,
`stale_generation`, `sequence`, `already_exists`, `snapshot_unavailable`,
`closed` and `internal`.

| Operation | More fields | Result and rule |
| --- | --- | --- |
| `create` | `cols`, `rows`, optional `scrollback` | `{seq:0}`. Refuses a session that is there, also with a different generation. |
| `write` | canonical base64 `data`, `seq` | `{seq,replies:string[]}`. `seq` must be the previous seq plus the length of the decoded bytes. |
| `resize` | `cols`, `rows`, `seq` | `{seq}`. The current byte sequence is necessary. Keeps the state of the decoder and the parser that waits. |
| `snapshot` | optional boolean `view` | The structured snapshot below, after the previous writes and resizes are complete. With `view: true`, the buffer `ansi` is empty and `links` is `[]`. |
| `dispose` | none | `{seq}`. Disposes only the generation that agrees. |

All operations use one queue in sequence. This includes the operations that make
and dispose a session, the resize operations and the snapshot barriers. A
previous generation cannot write, resize, snapshot or dispose its replacement.
The helper refuses missing bytes in the sequence or a duplicate write that is
not empty. The host must not try an uncertain write again. It must make a new
snapshot boundary or stop the generation. An `internal` error does not tell that
the operation had no effect. The host must stop that helper state.

Stdio applies backpressure. It waits for each parser operation and for the
stdout drain. It accepts a last complete JSON line without a newline. A
malformed JSON line gets an error, and the processing continues. A frame that
is too large gets a `limit` response, and the helper closes that stream. At
EOF, the helper disposes the emulators after the work in the queue is complete.
The service in the process gives the same sequence.

## Screen snapshot and replay

The snapshot contains these items:

- `v`, `session`, `generation` and `seq`.
- The dimensions and `scrollback`.
- The active buffer, and different `Screen` records for the normal buffer and
  the alternate buffer.
- `modes`, `rehydrate`, `title` and `titleTruncated`.
- The OSC8 `links`.
- `viewportLines`, `viewportCharacterColumns` and `viewportStyles`.
- `appearance`, `pendingEscape` and the base64 `pendingUtf8`.

A `Screen` contains these items:

- The ANSI contents.
- The cursor position and the wrap-pending state.
- The scroll margins.
- The saved cursor position and SGR.
- The last cell that is necessary to restore a pending wrap.

`alternate` is null while the normal buffer is active. xterm deletes the
contents of its alternate buffer when it goes out of it.

`viewportLines` contains `rows` strings, not more and not less, from the active
buffer at `baseY + row`. The helper uses `translateToString(true)`. The lines do
not include the scrollback or the empty cells at the end of a line. They keep
explicit space characters. Consumers can read a current native prompt without a
new parse of the ANSI. This content has the same privacy boundary as the screen.
It must not go into logs or telemetry.

`viewportCharacterColumns` gives the zero-based xterm column for each Unicode
scalar in each `viewportLines` string. It has `rows` arrays, not more and not
less. Each array has the same length as the number of scalars in its line. The
columns do not decrease, and they are less than `cols`. Scalars in one cell have
the same column. A wide character has one entry at its first column.

This map records the actual widths of the pinned renderer. It does not calculate
Unicode widths independently. The full map has a maximum of 262,144 entries. If
the map is larger, the snapshot stops with an explicit error. The native UI edge
can find a painted cursor between emoji or combining characters.

`viewportStyles` gives the dim and inverse flags and the foreground and
background colors. They are in continuous runs for each row of the active
viewport. Each run has a zero-based `start` column and an exclusive `end`
column. The runs cover `[0, cols)`, and the outer array has `rows` entries.
The color mode is one of these values:

- `default` with the value `-1`.
- `palette` with an index `0..255`.
- `rgb` with a value `0..0xffffff`.

The attributes come from the xterm cells. This includes the continuation columns
of wide cells. The current limit on the snapshot dimensions also applies to this
metadata. The native UI edge can find the difference between a painted
placeholder style and the same text that a person typed. It does not decode the
ANSI of the provider for this.

The normal ANSI includes the scrollback that the helper keeps. The helper
serializes links into OSC8 sequences and also gives them as ranges. Thus, an
emulator that the caller makes again continues to own the links after more
output and a resize. The helper restores an open hyperlink pen for the bytes
that follow. It does not change the implicit underline of the link into an
explicit underline, because an explicit underline continues after the link
closes. The saved cursor keeps the position and the usual SGR attributes. This
version does not include the saved charset state.

The modes include these items:

- Application cursor and application keypad.
- Bracketed paste, insertion and origin.
- Reverse wrap, focus reports and wraparound.
- The mouse protocol, and the SGR and SGR-pixel encodings.
- Cursor visibility, cursor style and cursor blink.

This agrees with the current input modes of the pinned xterm renderer. To add a
newer keyboard protocol, upgrade and validate the renderer and the helper
together.

The browser-safe `src/replay.ts` makes the restoration sequence. Its caller
must do these steps in this sequence:

1. Subscribe to the live output and keep the frames of the same generation.
2. Get the snapshot. Let the previous parser writes complete.
3. Reset xterm. Apply the snapshot dimensions and the scrollback policy.
4. Disable all writes back to the PTY while xterm parses the full replay.
5. Restore the normal and alternate buffers.
6. Restore the saved cursor, the current cursor and the modes.
7. Restore the title. Then restore the suffix of the escape sequence that is
   not complete. Do this step last.
8. Wait for the parser callback of xterm.
9. Give `pendingUtf8` to a new streaming UTF-8 decoder. Do not show a
   replacement character.
10. Apply only the live output after `seq` from the same generation.
11. If the current viewport must have different dimensions, resize again. Do
    this only at the related host boundary in the sequence.

These UTF-8 bytes that wait are part of `seq` at this time. Do not use a
sequence number as the identity of a process.

A CSI or OSC sequence that is not complete can go across a PTY chunk boundary.
The escape tracker keeps it, so that the next live byte completes the initial
sequence. The xterm parser can do C0 controls in a CSI or ESC sequence that is
not complete. The replay does not do these controls again. UTF-8 keeps a maximum
of three bytes that are not complete. For incorrect sequences, the standard
decoder puts a replacement character.

The helper keeps the screen that this contract specifies. It does not export
all private xterm objects. The helper does not restore these items:

- Custom charset registers.
- Custom tab stops.
- The state of other addons.
- Changes to the indexed palette.
- Newer protocols that the pinned renderer does not have.

The restoration guarantees apply only to the state in this contract and the
replay tests below. They do not prove that all native CLI interactions operate
correctly.

## Terminal query ownership

The headless emulator is the only authority that answers queries. Only live
`write` operations capture `onData` of xterm. Their result contains base64
`replies`. Snapshot and resize do not send input. After the host accepts the
write result, it sends each returned reply one time to the PTY of the same
generation. There is no event for a reply that the host did not request.

When this helper owns the replies, the renderer must not send automatic replies.
This applies to the replay and to the live output. The keyboard input and the
paste input of the person continue to go to the PTY through the current
authority. If the previous generic xterm `onData` sink does not change, it sends
the live replies two times. Thus, that integration is not correct.

The native foreground, background and cursor defaults come from the browser-safe
`src/appearance.ts`. The native renderer uses the same file. The helper receives
the OSC10, OSC11 and OSC12 requests that xterm parses. It answers the reports
with the current colors. It also records the operations that set and reset the
colors. `appearance` records the colors that result. The replay restores them
before the screen. The helper does not answer queries for the indexed palette or
the pixel geometry. It does not make renderer measurements.

The native conversation presentation uses these defaults:

- A neutral dark background (`#282c34`).
- A white foreground and a white cursor.
- A full ANSI theme with a blue selection (`#5a7898`).
- On macOS, the font list starts with SF Mono at 14 px. The regular weight is
  500 and the bold weight is 700. The line height is 1. System monospace fonts
  are the fallbacks.

These visual defaults apply to the pinned xterm 6.0.0 renderer. Its WebGL addon
shows the terminal, with a DOM fallback. Explicit native OSC color changes
continue to apply first through the replay. A helper that operates keeps its
current colors until its usual replacement. When you build the packaged
executable again, the live state does not change. The provider does not start
again to apply a theme.

In the pinned renderer, `CoreService.triggerDataEvent` identifies keyboard, IME,
paste and SGR mouse data as user input. It does not identify focus
notifications. The terminal must continue to send focus notifications as view
events. The default mouse encoding uses `onBinary`. Its string has byte values,
so it must go through an IPC path that keeps the bytes. It must not go through a
UTF-8 encoder.

`src/terminal-input.ts` puts a wrapper around the synchronous user-input marker
of the pinned core. `Term` does these operations:

- It does not send automatic replies.
- It sends the focus input and the raw binary input.
- It serializes the input promises with the session and generation that it
  captured.

An attachment change refuses input in the queue that is not current. If the
terminal stops all `onData` while it parses live output, the terminal does not
get the input that occurs at the same time.

## Desktop lifecycle

`chat/terminal/screen.rs` puts writes, resizes and snapshots in sequence under
one emulator mutex. This mutex is not one of the locks for the conversation
capture or the PTY output. A native resize operates in the same boundary. The
helpers use byte offsets. The desktop wrapper also keeps the related sequence
of complete PTY chunks for IPC. The empty chunk at the end of the process also
uses a chunk number.

The readiness checks and the controls read view snapshots. The serialization of
the full history uses tens of milliseconds. During this time, it holds the mutex
that the PTY reader must have. Thus, only the restoration of a view and the
freeze request the full snapshot. A full snapshot in the cache can also answer a
request for a view. A view snapshot cannot answer a request for a full snapshot.

After the process stops, the host freezes the last snapshot. Then it disposes
and reaps the helper. Thus, closed conversation tabs do not each keep one Node
process. If the helper has a failure, the host refuses the restoration and stops
that native process. It does not show the raw tail as a full screen that it
restored correctly. The app can resume its native transcript.

A release build uses the packaged executable adjacent to the app binary. A
debug build accepts the explicit `PROMETEU_TERMINAL_STATE_HELPER` path from the
development script. These two paths do not look for a Node that the customer
installed. Node is not necessary on the computer of the customer. An ignored
test of the desktop screen lifecycle runs the actual packaged helper. It
examines resize, query replies, UTF-8 continuation and frozen snapshots.

The SEA builder signs and examines a temporary executable. Then it replaces the
destination in one atomic operation. Subsequent launches use that executable.
Helper processes that operate at that time continue with their initial code
and state.

## Limits and evidence

`LIMITS` gives the defaults and the maximum values:

- 32 sessions.
- 500 columns and 300 rows.
- 5,000 scrollback rows. The default is 1,000.
- One million nominal cells for each session and four million in total. This
  includes the two screen buffers.
- 256 KiB of decoded data for each write.
- 512 KiB for each input line.
- 16 MiB of JSON for each snapshot.
- 128 requests in the library queue.

The helper examines the grid limits and the resize limits before it changes
the state.

If an escape sequence that is not complete has more than 4,096 characters,
snapshots are not available until the sequence is complete. The emulation
continues. The helper does not silently cut the tail to make a restore point.
The helper cuts the title metadata at 4,096 characters, and the snapshot tells
that it cut the title. The helper limits large OSC8 URLs that occur many times
before it expands the ANSI. It refuses a snapshot that is more than the result
limit. These are limits on what the screen keeps. They are not rules that cut
the transcript.

`test/state.test.ts` makes a second headless instance after the restoration. It
compares these items with that instance: cells, colors, styles, the cursor,
buffers, links, modes and the subsequent output. The tests include these
conditions:

- An exit from the alternate screen.
- A saved cursor and margins.
- Wide characters with a wrap that waits.
- Hyperlinks in the scrollback.
- CSI, OSC and UTF-8 sequences that are divided between chunks.
- Resize and reflow.
- Query replies.

`test/protocol.test.ts` examines the actual stdio executable and the sequence of
requests that occur at the same time. It also examines the replacement of a
generation, missing bytes, duplicate writes and the admission limits. The
differences between providers stay in the [provider
matrix](../quality/provider-matrix.md). These tests do not start an agent.

Run `npm ci --prefix packages/terminal-state`. Then run the `test` and
`typecheck` scripts of the package.

`build:sea` downloads and examines the pinned Node 24.14.0 archive. It bundles
the helper and its dependencies. It puts the SEA blob into the same runtime.
It records the hashes of the archive and the executable. V8 keeps its JIT,
because each PTY chunk waits for the helper. With `--jitless`, writes and
snapshots were approximately 3.5x slower.

On macOS, the build signs the executable with `src-tauri/Entitlements.plist`.
Tauri applies the same file when it signs the bundled sidecar again. This file
grants `com.apple.security.cs.allow-jit` under the hardened runtime. The build
disables the runtime argument extension. `test:sea` runs with an empty PATH and
with incorrect Node options in the environment. The native macOS arm64 build has
a local verification. The Linux x64 procedure does not have a local
verification.

On macOS, `npm run app`, `npm run build:app` and `npm run app:bundle` use
`scripts/terminal-state-app.mjs`. This script installs the locked dependencies
of the helper. Then it builds the SEA and adds `tauri.terminal-state.conf.json`
to the Tauri command. The overlay bundles the executable and the licenses. In
development, the script gives the explicit path of the executable.

A local debug `.app` bundle had no errors in the strict signature verification.
It ran the bundled helper without an installed Node. The current helper is
approximately 113 MiB before compression.

`npm run terminal-state:check` runs after the frontend build in `npm run check`.
It validates the package and the SEA. It also runs the host tests for the
protocol format and the screen. It does not start an agent. Linux CI also runs
that check. The Linux release does not use the helper at this time. The macOS
release build uses the same wrapper. Then it examines the packaged executable
and the notices before it notarizes the DMG.

The macOS execution host and the Settings capability enable the native
conversation mode. The packaging tests do not enable it. These tests do not
prove that the full application agrees with this contract. They do not authorize
publication.

The scanner for escape sequences that are not complete comes from Orca commit
`6ee09ca1c9901c58e45df966cadd6f2a86e268f0`. Its MIT license and attribution
are in `packages/terminal-state/THIRD_PARTY_NOTICES.md`. The sidecar overlay
includes those notices and the Node and xterm licenses.
