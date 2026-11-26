# Native input fixtures

The opt-in tests in `terminal_agent_hooks/smoke.rs` recorded these emulator
snapshots. The tests used empty temporary workspaces and synthetic prompts. The
home path is `/home/test-user` and the workspace path is
`/tmp/native-smoke/workspace`. These paths replace the paths of the test
computer. The snapshots do not
contain account credentials or unrelated conversation history.

The recorded builds are Claude Code `2.1.326`, Cursor Agent
`2026.11.13-eacade1` and Antigravity `1.5.10`. Readiness is different for each
version, and it only lets input in. It does not prove completion, success, or
that a notification is applicable. A name such as `rest` gives the stage of the
capture. It does not tell that the terminal can accept input. For a short time, a last
native status and a busy footer can show together.

The readiness regression also uses the current
`terminal_agent_controls/fixtures/antigravity-1.5.10-interrupted.json` capture.
After Escape, this build puts its interruption banner directly above the top
border of the empty composer, without an empty line between them. The empty-input predicate accepts only that
native banner. The predicate
continues to reject pasted-content chips and modal rows at the same position.
For subsequent queued input, different native idle evidence continues to be
necessary.

The Cursor `wrapped`, `multiline`, `unicode`, `scrolled`, and `chip` snapshots
come from bracketed pastes into the draft only, without Enter or a model call.
The `unicode` snapshot includes combining characters, CJK, and emoji. The cell
columns come directly from the emulator, because the native layout and the
Unicode widths of xterm can be different. The chip agrees with the atomic map of
pasted content and the UTF-16 length limit of the installed CLI. The host must
reserve the empty composer and get a newer paint. Only then can this
acknowledgment let Enter operate.

The Claude `empty-placeholder`, `literal-draft`, `literal-home`, and
`cleared-draft` snapshots compare the native example with the same text in a
typed draft. Then they use Ctrl-A and Ctrl-U. This test only uses the draft. It
enables colors for its own process to keep the dim attribute of the native
placeholder. Production does not force colors. An unclear example without style
cannot clear a fence for local edits.

The source evidence for the placeholder behavior and the paste behavior is in
the installed Cursor modules. These modules are `src/components/text-input.tsx`,
`src/hooks/use-text-input.ts`, and the submit and paste handlers in the
`2986.index.js` chunk. The native helper must supply `viewportLines`,
`viewportStyles`, and `viewportCharacterColumns`. These fields show the active
viewport. No second VT parser makes them again in this code.

The Cursor `app-unfocused-empty` snapshot is a Tauri snapshot from the
conformance workspace that the app owns. The app recorded it before it sent its
queued synthetic prompt. The paths have replacements of the same row widths. At
83 columns, the footer path goes onto a second row. Thus, the physical cursor is
six rows below the composer, not five.

The native window focus report removes the inverse caret. But the empty
placeholder and the arrow stay dim. Literal drafts have a usual arrow, also
when they are disabled. The owned-paste variants in the unit tests are synthetic
changes to current captures. They do not tell that the send at the app level
operated correctly.

The installed `2986.index.js` gives the window focus of `notifications-context`
to `showCursor`. This value is different from the `focus` predicate of the
input. The prompt arrow is dim only when the input value is empty.
`8920.index.js` renders a dim empty placeholder when `showCursor` is false. It
renders usual draft text without an inverse caret. Its SHA-256 is
`a98878d47223628a27a97e67f54fa074aa1603bc5d90c6638b3c0683d17cf3ad`. The Cursor
contract records the hash of the installed `2986.index.js`.

For a paste acknowledgment, these conditions continue to be necessary:

- exclusive ownership
- a newer paint
- the accurate native wrap and content
- no modal footer

The wrap of the footer does not set the number of rows of the paste. Dim usual
draft text cannot acknowledge input.

The Cursor `app-small-auto` and Codex `app-wrapped-footer` snapshots are more
Tauri captures that the app owns, at 61×17 and 80×24. Their paths have
replacements of the same row widths. They examine the native blocks for the
model, the location and the hints, not a constant distance from the composer.
The installed `autorun-headline-label.ts` of Cursor returns `Run Everything` for
the current mode without limits. The app renders this text independently on the
right side of the prompt footer.

The approval menu of Cursor adds `(shift+tab)` and other decision rows. The
modal veto does not apply only to the accurate status label at the footer position.
A decision with an equivalent name stays blocked. The other wrapped rows and the
paste conditions in the tests are synthetic variants.

The Cursor `app-followup-scrolled` snapshot is the same owned session in a small
panel after its first response. The welcome banner is not in the viewport. The
current version, the empty-input attributes, the cursor and the footer continue
to be necessary. The banner is only a delimiter when it shows. The tests keep
the literal-draft, decision and owned-paste checks for this capture.

The Antigravity `app-auto-placeholder-uncolored` snapshot records the same mode
hint after a response. The test app inherits `NO_COLOR=1`. Without special
attributes, the text alone cannot prove that the input is empty.

The Antigravity `app-auto-placeholder-colored`, `app-auto-literal`, and
`app-auto-cleared` snapshots record the same owned mode hint. Colors are enabled
only in the environment of the test app. The placeholder uses the foreground
palette color 8. The same text, pasted without Enter and moved to Home, uses the
default foreground at the same cursor position. Ctrl-E/Ctrl-U then gives a
plain empty input again. Production keeps an explicit `NO_COLOR` preference and
rejects unclear hints.

In the pinned binary, `PromptModel.View` calls `EditArea.Value` at `0x10319cff0`.
It goes to the hint branch at `0x10319d014` only for an empty value. It selects
the accurate Auto, Plan, Accept-edits or Best-of-N hint. It calls `truncateToWidth`
at `0x10319d1b4`, and renders the different hint style at `0x10319d204`. The
available width is the terminal width less the prompt of two columns and one
cell at the end. `truncateToWidth` (`0x1031a0220`) keeps that width. When it is
necessary, it replaces the last three characters with `...`. The tests for
narrow widths are synthetic variants of the colored capture from the binary. They keep the accurate checks of content and style.

The Antigravity `fresh-final` and `fresh-busy` snapshots come from the current
instruction smoke test in a private scratch workspace. The empty composer has
one line, a cursor that shows at column 2, two full-width rules and the idle
shortcut footer. The variants for spaces and Home, chips and modals in the unit
tests are explicit changes to that capture. They are not more native
recordings. After input, native idle continues to be necessary. The UI does not
complete a turn.

The Antigravity `app-empty` snapshot is a Tauri conformance capture that the app
owns. The app recorded it after it closed the native promotion at startup. The
account label and the workspace path have replacements of the same row lengths.
Its composer of 83 columns and 44 rows passes the same empty-input predicate.
This fixture does not supply an idle report. It does not prove that the app
queue sent the prompt that waits.

The host can let one initial input in with this shape only for a new process.
That process must have no activity that the host saw and no previous composer writes.
The regression for the packaged helper examines two conditions. A modal rejects
the input. Local Enter and application input cannot open the startup gate again
from a subsequent empty repaint.

The equivalence of spaces is pinned to the macOS arm64 `agy` 1.5.10 binary,
SHA-256 `780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798`. We
examined this binary in read-only mode on 2026-11-17. Its Go function
`google3/third_party/jetski/cli/model/model.PromptModel.handleInputWithOptions`
calls `editing.EditArea.Value` at `0x103199890`. Then it calls
`strings.TrimSpace` on that value at `0x103199bd4`. The path without a slash
command examines the trimmed length at `0x103199eac`. If the length is zero and
there are no `pastePlaceholderRanges` (`0x103199eb8`), it sets an empty value
with `SetValueWithUndo` (`0x103199ef8`). Then it returns a nil command
(`0x103199f38`–`0x103199f4c`).

The addresses are unslid Mach-O virtual addresses from the Go pclntab of the
binary. They apply to this build only. They are not a guarantee of an upstream
API. The readiness predicate lets in the input of plain spaces that the host saw, or a colored mode hint
that it examined. The geometry must have one focused row, without
chips or overlays. The predicate does not trim application payloads. It does
not guess the normalization of hook aliases. It does not accept random
invisible characters.
