# Native input fixtures

These emulator snapshots were captured by the opt-in tests in
`terminal_agent_hooks/smoke.rs`, using empty temporary workspaces and synthetic
prompts. Home/workspace paths are replaced with `/home/test-user` and
`/tmp/native-smoke/workspace`. They contain no account credentials or unrelated
conversation history.

The captured builds are Claude Code `2.1.326`, Cursor Agent
`2026.11.13-eacade1` and Antigravity `1.5.10`. Readiness is version-specific and only admits input; it
does not establish completion, success, or notification eligibility. Names such
as `rest` describe the capture stage, not a readiness assertion. A final native
status can coexist briefly with a busy footer.

The readiness regression also consumes the existing
`terminal_agent_controls/fixtures/antigravity-1.5.10-interrupted.json` capture.
After Escape, this build places its interruption banner directly above the
empty composer's top border, without a blank spacer. That exact native banner
is compatible with the empty-input predicate; pasted-content chips and modal
rows in the same position remain rejected. Separate native idle evidence is
still required for subsequent queued input.

Cursor `wrapped`, `multiline`, `unicode`, `scrolled`, and `chip` snapshots come
from draft-only bracketed pastes, with no Enter or model call. The `unicode`
case covers combining characters, CJK, and emoji. Cell columns come directly
from the emulator because native layout and xterm's Unicode widths can differ.
The chip follows the installed CLI's atomic pasted-content map and UTF-16
length threshold. The host must reserve the empty composer and obtain a newer
paint before this acknowledgment can authorize Enter.

Claude `empty-placeholder`, `literal-draft`, `literal-home`, and `cleared-draft`
snapshots compare the native example with identical text typed as a real draft,
then Ctrl-A and Ctrl-U. This draft-only test enables colors on its own process
to preserve the native placeholder's dim attribute. Production does not force
colors: an ambiguous unstyled example cannot clear a local-editing fence.

The source evidence for placeholder and paste behavior is in the installed
Cursor modules `src/components/text-input.tsx`, `src/hooks/use-text-input.ts`,
and the submit/paste handlers in chunk `2986.index.js`. The native helper's
required `viewportLines`, `viewportStyles`, and `viewportCharacterColumns`
represent the active viewport; no second VT parser reconstructs them here.

Cursor `app-unfocused-empty` is a real Tauri snapshot from the owned app
conformance workspace, before its queued synthetic prompt was submitted. Paths
are replaced without changing row widths. At 83 columns its footer path wraps;
the physical cursor is six rows below the composer rather than five. The native
window-focus report removes the inverse caret but leaves its empty placeholder
and arrow dim. Literal drafts have a normal arrow, including disabled drafts.
The owned-paste variants in the unit tests are synthetic changes to existing
captures; they are not a claim that the app-level send has passed.

Installed `2986.index.js` passes `notifications-context`'s window focus to
`showCursor`, separately from the input's `focus` predicate. Its prompt arrow is
dim only when the input value is empty. `8920.index.js` renders a dim empty
placeholder when `showCursor` is false and ordinary draft text without an inverse
caret. Its SHA-256 is
`a98878d47223628a27a97e67f54fa074aa1603bc5d90c6638b3c0683d17cf3ad`;
the installed `2986.index.js` hash is recorded in the Cursor contract.
Paste acknowledgment still requires exclusive ownership, a newer paint, exact
native wrapping/content and no modal footer. Footer wrapping does not determine
the paste's row count, and dimmed ordinary draft text cannot acknowledge input.

Cursor `app-small-auto` and Codex `app-wrapped-footer` are further owned Tauri
captures, at 61×17 and 80×24 respectively. Their paths are replaced at the same
row widths. They cover the native model/location/hints blocks rather than a
fixed distance from the composer. Cursor's installed `autorun-headline-label.ts`
returns `Run Everything` for the current unrestricted mode, rendered separately
on the prompt footer's right side. Its approval menu instead adds `(shift+tab)`
and other decision rows. Only the exact status label at the footer position is
exempted from the modal veto; a similarly named decision remains blocked.
Additional wrapped rows and paste cases in the tests are synthetic variants.

Cursor `app-followup-scrolled` is the same owned small-panel session after its
first response: the welcome banner is outside the viewport. The current version,
empty-input attributes, cursor and footer remain required; the banner is only a
delimiter when present. Tests retain literal-draft, decision and owned-paste
checks for this capture. Antigravity `app-auto-placeholder-uncolored` records
the corresponding post-response mode hint with `NO_COLOR=1` inherited by the
test app. Without distinctive attributes, its text alone cannot prove empty.

Antigravity `app-auto-placeholder-colored`, `app-auto-literal`, and
`app-auto-cleared` capture the same owned mode hint with colors enabled only in
the test app environment. The placeholder uses foreground palette 8; identical
text pasted without Enter and moved to Home uses default foreground at the same
cursor position. Ctrl-E/Ctrl-U then restores a plain empty input. Production
preserves an explicit `NO_COLOR` preference and refuses ambiguous hints.

In the pinned binary, `PromptModel.View` calls `EditArea.Value` at `0x10319cff0`
and takes the hint branch at `0x10319d014` only for an empty value. It selects the
exact Auto, Plan, Accept-edits or Best-of-N hint, calls `truncateToWidth` at
`0x10319d1b4`, and renders the separate hint style at `0x10319d204`. The available
width is the terminal width minus the two-column prompt and one trailing cell.
`truncateToWidth` (`0x1031a0220`) retains that width and replaces the final three
characters with `...` when needed. Narrow-width tests are synthetic variants of
the real colored capture; they retain the exact content and style checks.

Antigravity `fresh-final` and `fresh-busy` were captured during the existing
instruction smoke in a private scratch workspace. Its one-line empty composer
has a visible cursor at column 2, two full-width rules, and the idle shortcut
footer. Whitespace/Home, chips and modal variants in the unit tests are explicit
mutations of that capture, not additional native recordings. Native idle is
still required after input; the UI never completes a turn.

Antigravity `app-empty` is an owned Tauri conformance capture after dismissing
the native startup promotion. The account label and workspace path are replaced
without changing row lengths. Its 83-column, 44-row composer passes the same
empty-input predicate; this fixture does not supply an idle report or prove that
the app queue has dispatched its pending prompt. The host may admit one initial
input using this shape only for a fresh process without observed activity or
previous composer writes. The packaged-helper regression checks that a modal
rejects admission and that neither local Enter nor application input can reopen
the startup gate from a later empty repaint.

The whitespace equivalence is pinned to the macOS arm64 `agy` 1.5.10 binary,
SHA-256 `780ee763a9938a37b45c7f9d6ae31747d7c574358fd10c64d3a1cdc36faf2798`,
audited read-only on 2026-11-17. Its Go function
`google3/third_party/jetski/cli/model/model.PromptModel.handleInputWithOptions`
calls `editing.EditArea.Value` at `0x103199890`, then `strings.TrimSpace` on that
value at `0x103199bd4`. The non-slash path tests the trimmed length at
`0x103199eac`. For zero length and no `pastePlaceholderRanges`
(`0x103199eb8`), it sets an empty value with `SetValueWithUndo`
(`0x103199ef8`) and returns a nil command (`0x103199f38`–`0x103199f4c`).
The addresses are unslid Mach-O virtual addresses, resolved from its Go pclntab;
they describe this build rather than an upstream API guarantee. The readiness
predicate admits the observed plain-space input or a verified colored mode hint
in the focused single-row geometry, without chips or overlays. It does not trim application payloads, infer hook
alias normalization, or accept arbitrary invisible characters.
