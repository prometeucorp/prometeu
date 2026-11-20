//! Versioned native UI input admission. These heuristics never assert turn completion or emit
//! notifications. They inspect the helper's current painted viewport, never output silence.
use crate::state::ProviderId;
use prometeu_process::terminal_state::{
    protocol::{BufferKind, ColorMode, ViewportStyleRun},
    Snapshot,
};
use unicode_normalization::UnicodeNormalization;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Copy, Debug)]
pub struct NativeUiReadiness {
    provider: ProviderId,
}

impl NativeUiReadiness {
    /// UI structure is verified against real captures from these builds onward. Newer builds are
    /// trusted: the view parser still rejects any layout it does not recognize.
    pub fn verified(provider: ProviderId, version: &str) -> Option<Self> {
        let min = match provider {
            ProviderId::Claude => "2.1.326",
            ProviderId::Codex => "0.169.0",
            ProviderId::Antigravity => "1.5.10",
            ProviderId::Cursor => "2026.11.13",
            _ => return None,
        };
        prometeu_protocols::cli_version::at_least(version, min).then_some(Self { provider })
    }

    pub fn ready(&self, snapshot: &Snapshot) -> bool {
        view_from(snapshot).is_some_and(|view| self.ready_view(view))
    }

    /// Admits only the first input into a fresh AGY process whose activity has not started.
    /// The host must revoke this opportunity on any composer input or native activity. This
    /// does not establish native idle and cannot release another input after a submission.
    pub fn ready_for_initial_input(&self, snapshot: &Snapshot) -> bool {
        view_from(snapshot).is_some_and(|view| self.initial_input_view(&view))
    }

    fn initial_input_view(&self, view: &View<'_>) -> bool {
        self.provider == ProviderId::Antigravity && self.empty_view(view)
    }

    /// Combines authoritative native idle with the current composer layout. Native OSC status
    /// replaces the title predicate only; modal/busy/draft checks remain necessary.
    pub fn ready_with_native_idle(&self, snapshot: &Snapshot) -> bool {
        view_from(snapshot).is_some_and(|view| {
            valid_view(&view)
                && match self.provider {
                    ProviderId::Claude => claude_composer(&view),
                    ProviderId::Codex => codex_composer(&view),
                    ProviderId::Antigravity => antigravity_composer(&view),
                    ProviderId::Cursor => cursor_ready(&view),
                    _ => false,
                }
        })
    }

    /// Proves an empty focused input independently of terminal activity/title. The caller must
    /// separately require an authoritative idle status (or `ready`) before writing. Unlike the
    /// initial placeholder shape, this proof can clear a local-input dirty latch.
    pub fn empty_composer_verified(&self, snapshot: &Snapshot) -> bool {
        view_from(snapshot).is_some_and(|view| self.empty_view(&view))
    }

    fn empty_view(&self, view: &View<'_>) -> bool {
        valid_view(view)
            && match self.provider {
                ProviderId::Cursor => cursor_ready(view),
                ProviderId::Claude => claude_composer(view) && claude_empty_style(view),
                ProviderId::Codex => codex_composer(view),
                ProviderId::Antigravity => antigravity_composer(view),
                _ => false,
            }
    }

    pub fn requires_paste_ack(&self) -> bool {
        self.provider == ProviderId::Cursor
    }

    /// Acknowledges the native painted paste before Enter, not a model submission or turn.
    /// The caller must exclusively own input from an empty `ready` composer through a complete
    /// bracketed-paste write, and inspect a snapshot newer than that write. A chip is meaningful
    /// only under this ownership: Cursor binds its displayed id to the full atomic paste in memory.
    pub fn paste_applied(&self, snapshot: &Snapshot, text: &str) -> bool {
        self.requires_paste_ack()
            && view_from(snapshot).is_some_and(|view| cursor_paste_applied(&view, text))
    }

    fn ready_view(&self, view: View<'_>) -> bool {
        if !valid_view(&view) {
            return false;
        }
        match self.provider {
            ProviderId::Claude => claude_ready(&view),
            ProviderId::Cursor => cursor_ready(&view),
            _ => false,
        }
    }
}

struct View<'a> {
    title: &'a str,
    lines: &'a [String],
    styles: &'a [Vec<ViewportStyleRun>],
    character_columns: &'a [Vec<u16>],
    cols: u16,
    rows: u16,
    cursor_x: u16,
    cursor_y: u16,
    cursor_hidden: bool,
    bracketed_paste: bool,
    partial: bool,
}

fn view_from(snapshot: &Snapshot) -> Option<View<'_>> {
    let screen = match snapshot.active {
        BufferKind::Normal => &snapshot.normal,
        BufferKind::Alternate => snapshot.alternate.as_ref()?,
    };
    Some(View {
        title: &snapshot.title,
        lines: &snapshot.viewport_lines,
        styles: &snapshot.viewport_styles,
        character_columns: &snapshot.viewport_character_columns,
        cols: snapshot.cols,
        rows: snapshot.rows,
        cursor_x: screen.cursor.x,
        cursor_y: screen.cursor.y,
        cursor_hidden: snapshot.modes.cursor_hidden,
        bracketed_paste: snapshot.modes.bracketed_paste_mode,
        partial: !snapshot.pending_escape.is_empty() || !snapshot.pending_utf8.is_empty(),
    })
}

fn valid_view(view: &View<'_>) -> bool {
    view.cols >= 60
        && view.rows >= 12
        && view.lines.len() == usize::from(view.rows)
        && view.cursor_x < view.cols
        && view.cursor_y < view.rows
        && view.bracketed_paste
        && !view.partial
}

/// Cursor's verified bracketed-paste/submission normalization. NFC belongs to painting only;
/// it must not change the native payload or its causal submission hash.
pub fn normalize_cursor_submission(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "  ")
}

fn cursor_paste_applied(view: &View<'_>, text: &str) -> bool {
    if text.is_empty()
        || view.cols < 60
        || view.rows < 12
        || view.lines.len() != usize::from(view.rows)
        || view.styles.len() != usize::from(view.rows)
        || !view.bracketed_paste
        || view.partial
        || !view.cursor_hidden
        || view.cursor_x != 0
        || view.cursor_y >= view.rows
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return false;
    }
    let banner = view
        .lines
        .iter()
        .rposition(|line| line.trim() == "Cursor Agent");
    let Some(first) = view.lines.iter().rposition(|line| line.starts_with("  → ")) else {
        return false;
    };
    if first == 0
        || banner.is_some_and(|banner| first <= banner)
        || !view.lines[first - 1].trim().is_empty()
    {
        return false;
    }
    let native = normalize_cursor_submission(text);
    let pasted_chip = native.encode_utf16().count() > 800;
    let expected = if pasted_chip {
        let Some(display) = view.lines[first].trim_end().strip_prefix("  → ") else {
            return false;
        };
        let Some(chip) = display
            .strip_prefix("[Pasted text #")
            .and_then(|text| text.strip_suffix(" lines]"))
        else {
            return false;
        };
        let Some((id, lines)) = chip.split_once(" +") else {
            return false;
        };
        if id.parse::<u64>().ok().filter(|id| *id > 0).is_none()
            || lines.parse::<usize>().ok()
                != Some(native.bytes().filter(|byte| *byte == b'\n').count() + 1)
        {
            return false;
        }
        vec![display.to_owned()]
    } else {
        let width = usize::from(view.cols - 7);
        // Cursor's six-line virtual window uses its UTF-16 width model before Ink paints NFC.
        let virtual_lines = wrap_native(&native, width, native_utf16_width);
        let visible = if virtual_lines.len() > 6 {
            virtual_lines[virtual_lines.len() - 6..].join("\n")
        } else {
            native
        };
        let normalized = visible.nfc().collect::<String>();
        // Ink's wrap renderer removes ASCII spaces from soft continuation lines. Explicit
        // newlines keep their leading spaces; trimming the whole draft would lose that distinction.
        normalized
            .split('\n')
            .flat_map(|line| {
                wrap_native(line, width, UnicodeWidthStr::width)
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| {
                        if index == 0 {
                            line
                        } else {
                            line.trim_start_matches(' ').to_owned()
                        }
                    })
            })
            .collect()
    };
    // Footer paths can wrap at the real window width. They do not determine the height of
    // the owned paste; use its expected native layout, then require the separate footer.
    if expected.is_empty() || expected.len() > 6 {
        return false;
    }
    let last = first + expected.len() - 1;
    if !cursor_footer(view, last) {
        return false;
    }
    let actual = view.lines[first..=last]
        .iter()
        .enumerate()
        .map(|(index, line)| {
            line.strip_prefix(if index == 0 { "  → " } else { "    " })
                .map(str::trim_end)
        })
        .collect::<Option<Vec<_>>>();
    let Some(actual) = actual else {
        return false;
    };
    if expected
        .iter()
        .zip(&actual)
        .any(|(expected, actual)| expected.trim_end() != *actual)
    {
        return false;
    }
    // Ink and xterm can use different Unicode width tables. Locate the caret by the helper's
    // scalar-to-cell map instead of recomputing where the native paint placed the final glyph.
    let caret_scalar = 4 + expected
        .last()
        .map(|text| text.chars().count())
        .unwrap_or(0);
    let Some(caret) = view
        .character_columns
        .get(last)
        .and_then(|row| row.get(caret_scalar))
    else {
        return false;
    };
    let caret = usize::from(*caret);
    if caret >= usize::from(view.cols) {
        return false;
    }
    let mut found = false;
    for (row, styles) in view.styles.iter().enumerate().take(last + 1).skip(first) {
        for run in styles.iter().filter(|run| run.inverse) {
            if row != last
                || usize::from(run.start) != caret
                || run.end != run.start + 1
                || run.dim
                || found
            {
                return false;
            }
            found = true;
        }
    }
    found || cursor_unfocused_draft_style(view, first, &expected, pasted_chip)
}

fn cursor_unfocused_draft_style(
    view: &View<'_>,
    first: usize,
    expected: &[String],
    pasted_chip: bool,
) -> bool {
    // Window focus controls only showCursor in the pinned CLI. An owned paste can therefore
    // paint without an inverse caret. Disabled ordinary input is dim; a real draft's arrow
    // is normal, unlike the empty placeholder arrow. Native pasted-content chips stay dim.
    let style = |row: usize, scalar: usize| {
        let column = view.character_columns.get(row)?.get(scalar)?;
        view.styles
            .get(row)?
            .iter()
            .find(|run| run.start <= *column && *column < run.end)
    };
    style(first, 2).is_some_and(|run| !run.dim && !run.inverse)
        && expected.iter().enumerate().all(|(offset, text)| {
            text.chars().enumerate().all(|(scalar, ch)| {
                ch.is_whitespace()
                    || style(first + offset, 4 + scalar)
                        .is_some_and(|run| !run.inverse && run.dim == pasted_chip)
            })
        })
}

fn native_utf16_width(text: &str) -> usize {
    text.encode_utf16()
        .map(|unit| match unit {
            0x0300..=0x036f | 0x200b..=0x200d | 0xfeff => 0,
            0x1100..=0x115f
            | 0x2e80..=0x9fff
            | 0xac00..=0xd7af
            | 0xf900..=0xfaff
            | 0xfe30..=0xfe6f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6 => 2,
            _ => 1,
        })
        .sum()
}

fn wrap_native(text: &str, width: usize, measure: impl Fn(&str) -> usize) -> Vec<String> {
    let mut output = Vec::new();
    for logical in text.split('\n') {
        if logical.is_empty() {
            output.push(String::new());
            continue;
        }
        let mut rest = logical;
        while !rest.is_empty() {
            let mut cut = rest.len();
            let mut space = None;
            for (offset, ch) in rest.char_indices() {
                let end = offset + ch.len_utf8();
                if offset > 0 && measure(&rest[..end]) > width {
                    cut = if ch != ' ' {
                        space.unwrap_or(offset)
                    } else {
                        offset
                    };
                    break;
                }
                if ch == ' ' {
                    space = Some(end);
                }
            }
            if cut == 0 {
                return Vec::new();
            }
            output.push(rest[..cut].to_owned());
            rest = &rest[cut..];
        }
    }
    output
}

fn claude_ready(view: &View<'_>) -> bool {
    // A title can be disabled by native configuration. A nonempty title must announce rest;
    // a shell's or conversation's unqualified name alone is not startup readiness.
    if !view.title.is_empty() && !view.title.starts_with('✳') && !view.title.starts_with("* ") {
        return false;
    }
    claude_composer(view)
}

fn claude_composer(view: &View<'_>) -> bool {
    let row = usize::from(view.cursor_y);
    if view.cursor_hidden
        || view.cursor_x != 2
        || row == 0
        || row + 1 >= view.lines.len()
        || !claude_empty_composer(&view.lines[row])
        || !horizontal_rule(&view.lines[row - 1], view.cols)
        || !horizontal_rule(&view.lines[row + 1], view.cols)
        || view.lines[(row + 8).min(view.lines.len())..]
            .iter()
            .any(|line| !line.trim().is_empty())
    {
        return false;
    }
    // A working turn leaves the same empty composer visible. Its footer and pending Stop-hook
    // row must veto that shape; dialog selectors cannot satisfy the cursor-and-border checks.
    !veto(&view.lines[row.saturating_sub(4)..])
}

fn claude_empty_style(view: &View<'_>) -> bool {
    let row = usize::from(view.cursor_y);
    let line = &view.lines[row];
    if line
        .trim()
        .strip_prefix('❯')
        .is_some_and(|text| text.trim().is_empty())
    {
        return true;
    }
    dim_placeholder(view, row)
}

fn dim_placeholder(view: &View<'_>, row: usize) -> bool {
    let line = &view.lines[row];
    let Some(columns) = view.character_columns.get(row) else {
        return false;
    };
    let Some(styles) = view.styles.get(row) else {
        return false;
    };
    let length = line.trim_end().chars().count();
    // The verified placeholders are dim. A literal identical draft remains normal even at
    // Home's same physical cursor position. Missing attributes cannot prove an empty input.
    length > 2
        && columns.len() >= length
        && columns[2] == 2
        && columns[2..length].iter().all(|column| {
            styles
                .iter()
                .find(|run| run.start <= *column && *column < run.end)
                .is_some_and(|run| run.dim && !run.inverse)
        })
}

fn codex_composer(view: &View<'_>) -> bool {
    let row = usize::from(view.cursor_y);
    let Some(hints) = view
        .lines
        .iter()
        .rposition(|line| line.contains("? for shortcuts"))
    else {
        return false;
    };
    if view.cursor_hidden
        || view.cursor_x != 2
        || row == 0
        || hints < row + 3
        || !matches!(
            view.lines[row].as_str(),
            "› Ask Codex to do anything" | "» Ask Codex to do anything"
        )
        || !view.lines[row - 1].trim().is_empty()
        || !view.lines[row + 1].trim().is_empty()
        || view.lines[row + 2..hints]
            .iter()
            .any(|line| !line.starts_with("  ") || line.trim().is_empty())
        || view.lines[hints + 1..]
            .iter()
            .any(|line| !line.trim().is_empty())
        || veto(&view.lines[row.saturating_sub(2)..row])
        || veto(&view.lines[hints..])
    {
        return false;
    }
    // rust-v0.169.0 ChatComposer renders this dim placeholder only for an empty textarea;
    // disabled input uses a different label and a dim arrow. Ultra changes only the arrow.
    // Native idle remains an independent requirement: this frame never completes a turn.
    view.styles.get(row).is_some_and(|styles| {
        styles
            .iter()
            .find(|run| run.start == 0 && run.end > 0)
            .is_some_and(|run| !run.dim && !run.inverse)
    }) && dim_placeholder(view, row)
}

fn antigravity_composer(view: &View<'_>) -> bool {
    let row = usize::from(view.cursor_y);
    let spaces = |line: &str| line.bytes().all(|byte| byte == b' ');
    let border = |line: &str| {
        line.chars().count() == usize::from(view.cols) && line.chars().all(|ch| ch == '─')
    };
    if view.cursor_hidden
        || view.cursor_x != 2
        || row < 2
        || row + 2 >= view.lines.len()
        || !(view.lines[row].strip_prefix('>').is_some_and(spaces)
            || antigravity_mode_hint(view, row))
        || view.lines[row].len() > usize::from(view.cols)
        || !border(&view.lines[row - 1])
        || !border(&view.lines[row + 1])
        // After Escape the pinned native UI places this history banner directly above the
        // composer border. It is not a draft or modal; other adjacent content stays ambiguous.
        || (!spaces(&view.lines[row - 2])
            && view.lines[row - 2].trim()
                != "⎿  Interrupted · What should Antigravity CLI do instead?")
        || view.lines[row + 3..].iter().any(|line| !spaces(line))
        || veto(&view.lines[row - 2..row])
        || veto(&view.lines[row + 1..])
    {
        return false;
    }
    let Some((hint, model)) = view.lines[row + 2].split_once("  ") else {
        return false;
    };
    // The pinned Go submit handler TrimSpace's EditArea.Value and returns without a command
    // for an empty result with no paste placeholders. Thus a single plain-space row at Home
    // is equivalent to empty; visible chips, other text, multiline drafts and overlays fail
    // the exact composer geometry. This does not normalize application submissions or prove idle.
    hint == "? for shortcuts"
        && !model.trim_matches(' ').is_empty()
        && !model.chars().any(char::is_control)
        && view.styles.get(row).is_some_and(|styles| {
            styles.first().is_some_and(|run| run.start == 0)
                && styles.last().is_some_and(|run| run.end == view.cols)
                && styles.iter().all(|run| !run.dim && !run.inverse)
        })
        && view.character_columns.get(row).is_some_and(|columns| {
            columns.len() == view.lines[row].len()
                && columns
                    .iter()
                    .enumerate()
                    .all(|(index, column)| usize::from(*column) == index)
        })
}

fn antigravity_mode_hint(view: &View<'_>, row: usize) -> bool {
    let Some(content) = view.lines[row].strip_prefix("> ") else {
        return false;
    };
    let width = usize::from(view.cols.saturating_sub(3));
    // PromptModel.View renders these hints only when EditArea.Value is empty. It uses
    // truncateToWidth before the dedicated palette-8 style; a literal at Home has ordinary
    // input styling. NO_COLOR removes that distinction and therefore cannot prove empty.
    let known = [
        "Auto mode: edits & safe commands auto-approved (shift+tab to cycle)",
        "Plan mode: research & plan only (shift+tab to cycle)",
        "Accept-edits mode: file edits auto-approved (shift+tab to cycle)",
        "Best-of-N mode: fork into parallel arms and pick a winner (shift+tab to cycle)",
    ]
    .iter()
    .any(|hint| {
        if hint.len() <= width {
            content == *hint
        } else if width >= 3 {
            content.len() == width && content.strip_suffix("...") == Some(&hint[..width - 3])
        } else {
            false
        }
    });
    known
        && view.styles.get(row).is_some_and(|styles| {
            (2..view.lines[row].len()).all(|column| {
                styles
                    .iter()
                    .find(|run| usize::from(run.start) <= column && column < usize::from(run.end))
                    .is_some_and(|run| {
                        !run.dim
                            && !run.inverse
                            && run.foreground.mode == ColorMode::Palette
                            && run.foreground.value == 8
                    })
            })
        })
}

fn claude_empty_composer(line: &str) -> bool {
    let Some(content) = line.trim().strip_prefix('❯').map(str::trim) else {
        return false;
    };
    if content.is_empty() {
        return true;
    }
    // kBi in the verified installed build paints one of these native examples at the empty
    // input cursor. The host must also veto a dirty local draft, even if it mimics a placeholder.
    let Some(example) = content
        .strip_prefix("Try \"")
        .and_then(|text| text.strip_suffix('"'))
    else {
        return false;
    };
    matches!(
        example,
        "fix lint errors"
            | "fix typecheck errors"
            | "how do I log an error?"
            | "create a util logging.py that…"
    ) || example
        .strip_prefix("how does ")
        .is_some_and(|path| path.ends_with(" work?"))
        || example.starts_with("refactor ")
        || example
            .strip_prefix("edit ")
            .is_some_and(|path| path.ends_with(" to…"))
        || example.starts_with("write a test for ")
}

fn horizontal_rule(line: &str, cols: u16) -> bool {
    let line = line.trim();
    line.chars().count() >= usize::from(cols.saturating_sub(4)) && line.chars().all(|ch| ch == '─')
}

fn cursor_ready(view: &View<'_>) -> bool {
    // Cursor uses an Ink-drawn input and hides the physical cursor even when ready. The banner
    // can scroll out after a response. When visible it delimits stale startup content; current
    // composer attributes, footer and the versioned process identity remain the admission proof.
    let banner = view
        .lines
        .iter()
        .rposition(|line| line.trim() == "Cursor Agent");
    let Some(row) = view.lines.iter().rposition(|line| {
        matches!(
            line.trim(),
            "→ Plan, search, build anything"
                | "→ Add a follow-up"
                | "→ Add a follow-up — /plan to review and build"
        )
    }) else {
        return false;
    };
    // Native control hints live in the composer/footer; welcome tips and prior conversation
    // prose above it can mention permissions or work without being a live blocking dialog.
    if !cursor_footer(view, row) {
        return false;
    }
    // The placeholder is the empty native input. A visible draft or an unknown placeholder
    // cannot be overwritten. Cursor's own viewport cursor rests below this input in this build.
    banner.is_none_or(|banner| row > banner)
        && row > 0
        && row + 1 < view.lines.len()
        && view.lines[row - 1].trim().is_empty()
        && view.lines[row + 1].trim().is_empty()
        && cursor_placeholder_style(view, row)
        && view.cursor_hidden
        && view.cursor_x == 0
}

fn cursor_footer(view: &View<'_>, last_input_row: usize) -> bool {
    let physical_row = usize::from(view.cursor_y);
    let headline = last_input_row + 3;
    let location = headline + 1;
    if physical_row >= view.lines.len()
        || physical_row <= location
        || !view.lines[last_input_row + 1].trim().is_empty()
        || !view.lines[last_input_row + 2].trim().is_empty()
        || view.lines[location..physical_row]
            .iter()
            .any(|line| !line.starts_with("  ") || line.trim().is_empty())
        || view.lines[physical_row..]
            .iter()
            .any(|line| !line.trim().is_empty())
    {
        return false;
    }
    // The native footer has a separate model/headline row and a wrapping location block.
    // Its right-hand autorun label is not the approval menu's "Run Everything (shift+tab)".
    // Exempt only that exact label at its structural position; all other dialog/busy hints
    // still veto both empty-input admission and the owned-paste acknowledgment.
    view.lines[last_input_row + 1..]
        .iter()
        .enumerate()
        .all(|(offset, line)| {
            if last_input_row + 1 + offset == headline {
                if let Some(left) = line.trim_end().strip_suffix("Run Everything") {
                    if left.ends_with("  ")
                        && !left.trim().is_empty()
                        && !left.contains(['→', '❯', '›'])
                    {
                        return !veto(&[left.to_owned()]);
                    }
                }
            }
            !veto(std::slice::from_ref(line))
        })
}

fn cursor_placeholder_style(view: &View<'_>, row: usize) -> bool {
    let Some(styles) = view.styles.get(row) else {
        return false;
    };
    let line = &view.lines[row];
    let Some(arrow) = line.chars().position(|ch| ch == '→') else {
        return false;
    };
    let start = arrow + 2;
    let end = line.trim_end().chars().count();
    let style = |column| {
        styles
            .iter()
            .find(|run| usize::from(run.start) <= column && column < usize::from(run.end))
    };
    // The arrow is dim only for empty native input. Window focus controls caret painting,
    // not input admission: without focus the whole placeholder is dim. Requiring the dim
    // arrow also rejects an identical literal draft, including disabled/dimmed input.
    start + 1 < end
        && style(arrow).is_some_and(|run| run.dim && !run.inverse)
        && (start..end).all(|column| {
            style(column).is_some_and(|run| {
                if column == start {
                    run.inverse != run.dim
                } else {
                    run.dim && !run.inverse
                }
            })
        })
}

fn veto(lines: &[String]) -> bool {
    lines.iter().any(|line| {
        if line
            .chars()
            .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
        {
            return true;
        }
        let lower = line.to_lowercase();
        [
            "esc to interrupt",
            "ctrl+c to stop",
            "running stop hooks",
            "working",
            "accessing workspace",
            "trust this",
            "trust the",
            "trusting workspace",
            "trust required",
            "run this command?",
            "run (once)",
            "run everything",
            "skip & tell the agent",
            "allowlist?",
            "permission",
            "approve",
            "approval",
            "sign in",
            "log in",
            "login",
            "authentication",
            "enter to confirm",
            "esc to cancel",
            "press enter",
            "select an option",
            "waiting for",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Capture {
        version: String,
        title: String,
        cols: u16,
        rows: u16,
        cursor: prometeu_process::terminal_state::protocol::Cursor,
        cursor_hidden: bool,
        bracketed_paste: bool,
        pending_escape: bool,
        pending_utf8: bool,
        lines: Vec<String>,
        #[serde(default)]
        styles: Vec<Vec<ViewportStyleRun>>,
        #[serde(default)]
        character_columns: Vec<Vec<u16>>,
    }
    impl Capture {
        fn view(&self) -> View<'_> {
            View {
                title: &self.title,
                lines: &self.lines,
                styles: &self.styles,
                character_columns: &self.character_columns,
                cols: self.cols,
                rows: self.rows,
                cursor_x: self.cursor.x,
                cursor_y: self.cursor.y,
                cursor_hidden: self.cursor_hidden,
                bracketed_paste: self.bracketed_paste,
                partial: self.pending_escape || self.pending_utf8,
            }
        }
        fn ready(&self, provider: ProviderId) -> bool {
            NativeUiReadiness::verified(provider, &self.version)
                .is_some_and(|check| check.ready_view(self.view()))
        }
    }

    fn capture(text: &str) -> Capture {
        serde_json::from_str(text).unwrap()
    }

    fn codex_capture(text: &str) -> (NativeUiReadiness, Snapshot) {
        #[derive(Deserialize)]
        struct EngineCapture {
            version: String,
            snapshot: Snapshot,
        }
        let capture: EngineCapture = serde_json::from_str(text).unwrap();
        (
            NativeUiReadiness::verified(ProviderId::Codex, &capture.version).unwrap(),
            capture.snapshot,
        )
    }

    #[test]
    fn antigravity_empty_or_plain_spaces_at_home_require_independent_native_idle() {
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, "1.5.10").unwrap();
        for body in [
            include_str!("native_ui_readiness/fixtures/antigravity-fresh-final.json"),
            include_str!("native_ui_readiness/fixtures/antigravity-fresh-busy.json"),
        ] {
            let frame = capture(body);
            assert!(
                !check.ready_view(frame.view()),
                "paint is not activity authority"
            );
        }
        let body = include_str!("native_ui_readiness/fixtures/antigravity-fresh-final.json");
        for spaces in ["", " ", "    "] {
            let mut frame = capture(body);
            let row = usize::from(frame.cursor.y);
            frame.lines[row] = format!(">{spaces}");
            frame.character_columns[row] = (0..frame.lines[row].len() as u16).collect();
            assert!(check.empty_view(&frame.view()), "{spaces:?}");
            frame.cursor.x = 6;
            assert!(
                !check.empty_view(&frame.view()),
                "requires verified Home position"
            );
            frame.cursor.x = 2;
            frame.lines[row] = "> A local unsent draft".into();
            assert!(!check.empty_view(&frame.view()));
        }
        let busy = capture(include_str!(
            "native_ui_readiness/fixtures/antigravity-fresh-busy.json"
        ));
        assert!(!check.empty_view(&busy.view()));
        for version in ["1.5.9", "unverified-build"] {
            assert!(NativeUiReadiness::verified(ProviderId::Antigravity, version).is_none());
        }
        assert!(NativeUiReadiness::verified(ProviderId::Antigravity, "1.5.11").is_some());
    }

    #[test]
    fn antigravity_empty_app_composer_admits_initial_input_without_claiming_idle() {
        let frame = capture(include_str!(
            "native_ui_readiness/fixtures/antigravity-app-empty.json"
        ));
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, &frame.version).unwrap();
        assert!(valid_view(&frame.view()));
        assert!(antigravity_composer(&frame.view()));
        assert!(check.empty_view(&frame.view()));
        assert!(check.initial_input_view(&frame.view()));
        assert!(!check.ready_view(frame.view()));
        let mut busy = capture(include_str!(
            "native_ui_readiness/fixtures/antigravity-fresh-busy.json"
        ));
        assert!(!check.initial_input_view(&busy.view()));
        busy = frame;
        let footer = usize::from(busy.cursor.y) + 2;
        busy.lines[footer] = "Enter to confirm          Gemini 3.8 Flash".into();
        assert!(!check.initial_input_view(&busy.view()));
    }

    #[test]
    fn antigravity_captured_interruption_keeps_empty_composer_available() {
        let mut frame = capture(include_str!(
            "terminal_agent_controls/fixtures/antigravity-1.5.10-interrupted.json"
        ));
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, &frame.version).unwrap();
        // This earlier native capture predates column metadata. Its composer contains only
        // the recorded ASCII prompt, so each scalar has the identical xterm cell column.
        let row = usize::from(frame.cursor.y);
        assert_eq!(frame.lines[row], ">");
        frame.character_columns = vec![Vec::new(); usize::from(frame.rows)];
        frame.character_columns[row] = vec![0];
        assert!(antigravity_composer(&frame.view()));
        assert!(
            check.empty_view(&frame.view()),
            "a newer paint can release Escape's local input fence"
        );
        assert!(
            !check.ready_view(frame.view()),
            "the banner does not replace native idle authority"
        );
        for adjacent in ["[Pasted text #1]", "Select a model", "Interrupted draft"] {
            frame.lines[row - 2] = adjacent.into();
            assert!(!check.empty_view(&frame.view()));
        }
    }

    #[test]
    fn antigravity_colored_mode_hints_distinguish_empty_input_from_literal_and_uncolored_text() {
        let body = include_str!(
            "native_ui_readiness/fixtures/antigravity-app-auto-placeholder-colored.json"
        );
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, "1.5.10").unwrap();
        let original = capture(body);
        let row = usize::from(original.cursor.y);
        assert!(check.empty_view(&original.view()));
        assert!(!check.ready_view(original.view()));
        let uncolored = capture(include_str!(
            "native_ui_readiness/fixtures/antigravity-app-auto-placeholder-uncolored.json"
        ));
        assert!(!check.empty_view(&uncolored.view()));
        for (body, empty) in [
            (
                include_str!("native_ui_readiness/fixtures/antigravity-app-auto-literal.json"),
                false,
            ),
            (
                include_str!("native_ui_readiness/fixtures/antigravity-app-auto-cleared.json"),
                true,
            ),
        ] {
            let frame = capture(body);
            assert_eq!(check.empty_view(&frame.view()), empty);
        }

        let mut literal = capture(body);
        for run in &mut literal.styles[row] {
            if run.start >= 2 {
                run.foreground.mode = ColorMode::Default;
                run.foreground.value = -1;
            }
        }
        assert!(!check.empty_view(&literal.view()));
        let mut partial = capture(body);
        partial.pending_escape = true;
        assert!(!check.empty_view(&partial.view()));
        let mut dialog = capture(body);
        dialog.lines[row + 2] = "Enter to confirm         auto · Gemini 3.8 Flash".into();
        assert!(!check.empty_view(&dialog.view()));

        let hint = original.lines[row].strip_prefix("> ").unwrap();
        let mut narrow = capture(body);
        narrow.cols = 61;
        let width = usize::from(narrow.cols - 3);
        narrow.lines[row] = format!("> {}...", &hint[..width - 3]);
        narrow.lines[row - 1] = "─".repeat(usize::from(narrow.cols));
        narrow.lines[row + 1] = narrow.lines[row - 1].clone();
        narrow.character_columns[row] = (0..narrow.lines[row].len() as u16).collect();
        narrow.styles[row][2].end = narrow.lines[row].len() as u16;
        narrow.styles[row][3].start = narrow.styles[row][2].end;
        narrow.styles[row][3].end = narrow.cols;
        assert!(check.empty_view(&narrow.view()));
        narrow.lines[row].replace_range(2..3, "X");
        assert!(!check.empty_view(&narrow.view()));
        narrow.lines[row] = format!("> {}...", "é".repeat((width - 3) / 2));
        assert!(!check.empty_view(&narrow.view()));
    }

    #[test]
    fn antigravity_chips_modals_non_ascii_spaces_and_multiline_drafts_cannot_release_input() {
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, "1.5.10").unwrap();
        let body = include_str!("native_ui_readiness/fixtures/antigravity-fresh-final.json");
        for text in [
            "> [Pasted text #1]",
            "> @synthetic-file.txt",
            "> /model",
            "> \t",
            "> \u{00a0}",
            "> \u{200b}",
            "> A local unsent draft",
        ] {
            let mut frame = capture(body);
            let row = usize::from(frame.cursor.y);
            frame.lines[row] = text.into();
            assert!(!check.empty_view(&frame.view()), "{text:?}");
        }
        for (offset, text) in [
            (-2, "[Pasted text #1]"),
            (-2, "Select a model"),
            (-1, "A previous draft line"),
            (1, "Another draft line"),
            (
                2,
                "Enter to confirm                      Gemini 3.8 Flash · high",
            ),
            (
                2,
                "esc to cancel                         Gemini 3.8 Flash · high",
            ),
            (3, "Select an option"),
            (4, "Allow this command?"),
        ] {
            let mut frame = capture(body);
            let row = (i32::from(frame.cursor.y) + offset) as usize;
            frame.lines[row] = text.into();
            assert!(!check.empty_view(&frame.view()), "{offset}: {text}");
        }
    }

    #[test]
    fn antigravity_missing_metadata_partial_paints_and_detached_focus_are_not_empty_proofs() {
        let check = NativeUiReadiness::verified(ProviderId::Antigravity, "1.5.10").unwrap();
        let body = include_str!("native_ui_readiness/fixtures/antigravity-fresh-final.json");
        let mut frame = capture(body);
        frame.pending_escape = true;
        assert!(!check.empty_view(&frame.view()));
        frame.pending_escape = false;
        frame.pending_utf8 = true;
        assert!(!check.empty_view(&frame.view()));
        frame.pending_utf8 = false;
        frame.cursor_hidden = true;
        assert!(!check.empty_view(&frame.view()));
        frame.cursor_hidden = false;
        frame.cursor.y -= 1;
        assert!(!check.empty_view(&frame.view()));
        let mut frame = capture(body);
        frame.styles[usize::from(frame.cursor.y)][0].inverse = true;
        assert!(!check.empty_view(&frame.view()));
        frame.styles.clear();
        assert!(!check.empty_view(&frame.view()));
        let mut frame = capture(body);
        frame.character_columns.clear();
        assert!(!check.empty_view(&frame.view()));
        let mut frame = capture(body);
        frame.cols += 1;
        assert!(
            !check.empty_view(&frame.view()),
            "borders belong to this geometry"
        );
        frame.cols -= 1;
        frame.bracketed_paste = false;
        assert!(!check.empty_view(&frame.view()));
    }

    #[test]
    fn codex_native_placeholder_releases_a_cleared_draft_only_with_separate_idle_authority() {
        for (body, empty) in [
            (
                include_str!("terminal_agent_codex/fixtures/codex-remote-fresh-ready.json"),
                true,
            ),
            (
                include_str!("terminal_agent_codex/fixtures/codex-remote-cold-resume-ready.json"),
                true,
            ),
            (
                include_str!("terminal_agent_codex/fixtures/codex-remote-literal-home.json"),
                false,
            ),
            (
                include_str!("terminal_agent_codex/fixtures/codex-remote-cleared.json"),
                true,
            ),
        ] {
            let (readiness, snapshot) = codex_capture(body);
            assert_eq!(readiness.empty_composer_verified(&snapshot), empty);
            assert_eq!(readiness.ready_with_native_idle(&snapshot), empty);
            assert!(
                !readiness.ready(&snapshot),
                "paint is not activity authority"
            );
        }
    }

    #[test]
    fn codex_empty_input_rejects_missing_styles_partial_paints_modals_and_whitespace_drafts() {
        let (readiness, original) = codex_capture(include_str!(
            "terminal_agent_codex/fixtures/codex-remote-fresh-ready.json"
        ));
        let row = usize::from(original.alternate.as_ref().unwrap().cursor.y);
        for blocker in [
            "esc to interrupt",
            "Enter to confirm",
            "Sign in to continue",
            "Waiting for approval",
        ] {
            let mut snapshot = original.clone();
            snapshot.viewport_lines[row + 3].push_str(blocker);
            assert!(!readiness.empty_composer_verified(&snapshot), "{blocker}");
        }
        let mut snapshot = original.clone();
        snapshot.viewport_styles.clear();
        assert!(!readiness.empty_composer_verified(&snapshot));
        let mut snapshot = original.clone();
        snapshot.viewport_character_columns.clear();
        assert!(!readiness.empty_composer_verified(&snapshot));
        let mut snapshot = original.clone();
        snapshot.pending_escape = "\u{1b}[".into();
        assert!(!readiness.empty_composer_verified(&snapshot));
        let mut snapshot = original.clone();
        snapshot.viewport_lines[row] = "› ".into();
        assert!(!readiness.empty_composer_verified(&snapshot));
        let mut snapshot = original.clone();
        snapshot.modes.cursor_hidden = true;
        assert!(!readiness.empty_composer_verified(&snapshot));
        let mut snapshot = original;
        snapshot.viewport_lines[row] = "» Ask Codex to do anything".into();
        assert!(readiness.empty_composer_verified(&snapshot));
        assert!(NativeUiReadiness::verified(ProviderId::Codex, "codex-cli 0.170.0").is_some());
        assert!(NativeUiReadiness::verified(ProviderId::Codex, "codex-cli 0.168.0").is_none());
    }

    #[test]
    fn codex_app_footer_can_wrap_without_relaxing_empty_input_or_dialog_checks() {
        let body = include_str!("native_ui_readiness/fixtures/codex-app-wrapped-footer.json");
        let check = NativeUiReadiness::verified(ProviderId::Codex, "codex-cli 0.169.0").unwrap();
        let mut frame = capture(body);
        assert!(check.empty_view(&frame.view()));
        assert!(!check.ready_view(frame.view()));
        // A narrower viewport or a longer path adds metadata rows without moving the input.
        for _ in 0..4 {
            let hints = frame.lines.len() - 1;
            frame
                .lines
                .insert(hints, "  wrapped workspace metadata".into());
            frame.styles.insert(hints, frame.styles[hints].clone());
            frame.character_columns.insert(hints, Vec::new());
            frame.rows += 1;
        }
        assert!(check.empty_view(&frame.view()));
        let mut literal = capture(body);
        let row = usize::from(literal.cursor.y);
        literal.styles[row]
            .iter_mut()
            .for_each(|run| run.dim = false);
        assert!(!check.empty_view(&literal.view()));
        let mut dialog = capture(body);
        dialog
            .lines
            .last_mut()
            .unwrap()
            .push_str(" · Enter to confirm");
        assert!(!check.empty_view(&dialog.view()));
        let mut partial = capture(body);
        partial.pending_escape = true;
        assert!(!check.empty_view(&partial.view()));
    }

    #[test]
    fn cursor_small_app_footer_distinguishes_autorun_status_from_approval_options() {
        let body = include_str!("native_ui_readiness/fixtures/cursor-app-small-auto.json");
        let mut frame = capture(body);
        assert!(frame.ready(ProviderId::Cursor));
        // The location may occupy additional rows; the block ends at the physical cursor.
        for _ in 0..4 {
            let location = usize::from(frame.cursor.y) - 1;
            frame
                .lines
                .insert(location, "  wrapped workspace path".into());
            frame
                .styles
                .insert(location, frame.styles[location].clone());
            frame.character_columns.insert(location, Vec::new());
            frame.rows += 1;
            frame.cursor.y += 1;
        }
        assert!(frame.ready(ProviderId::Cursor));
        for label in [
            "  Auto                         Run Everything (shift+tab)",
            "  →                            Run Everything",
            "  Run (once) (y)",
            "  Enter to confirm",
        ] {
            let mut dialog = capture(body);
            dialog.lines[13] = label.into();
            assert!(!dialog.ready(ProviderId::Cursor), "{label}");
        }
        let mut literal = capture(body);
        literal.styles[10]
            .iter_mut()
            .for_each(|run| run.dim = false);
        assert!(!literal.ready(ProviderId::Cursor));
        let mut pasted = capture(body);
        let text = "Keep working on the fixture";
        pasted.lines[10] = format!("  → {text} ");
        pasted.character_columns[10] = (0..pasted.lines[10].chars().count() as u16).collect();
        pasted.styles[10].iter_mut().for_each(|run| run.dim = false);
        assert!(cursor_paste_applied(&pasted.view(), text));
        pasted.lines[13].push_str(" (shift+tab)");
        assert!(!cursor_paste_applied(&pasted.view(), text));
    }

    #[test]
    fn cursor_scrolled_app_followup_preserves_empty_and_owned_paste_guards() {
        let body = include_str!("native_ui_readiness/fixtures/cursor-app-followup-scrolled.json");
        let frame = capture(body);
        assert!(frame.ready(ProviderId::Cursor));
        assert!(!frame.lines.iter().any(|line| line.trim() == "Cursor Agent"));
        let mut literal = capture(body);
        literal.styles[10]
            .iter_mut()
            .for_each(|run| run.dim = false);
        assert!(!literal.ready(ProviderId::Cursor));
        let mut dialog = capture(body);
        dialog.lines[13] = "  Run (once) (y)".into();
        assert!(!dialog.ready(ProviderId::Cursor));
        let mut pasted = capture(body);
        let text = "Synthetic follow-up";
        pasted.lines[10] = format!("  → {text} ");
        pasted.character_columns[10] = (0..pasted.lines[10].chars().count() as u16).collect();
        pasted.styles[10].iter_mut().for_each(|run| run.dim = false);
        assert!(cursor_paste_applied(&pasted.view(), text));
        pasted.lines[13].push_str(" (shift+tab)");
        assert!(!cursor_paste_applied(&pasted.view(), text));
    }

    #[test]
    fn claude_compact_composer_requires_current_focus_and_no_later_dialog() {
        let original = capture(include_str!(
            "native_ui_readiness/fixtures/claude-fresh-initial-ready.json"
        ));
        let mut compact = capture(include_str!(
            "native_ui_readiness/fixtures/claude-fresh-initial-ready.json"
        ));
        let start = usize::from(original.cursor.y) - 1;
        compact.lines.fill(String::new());
        for (target, source) in (12..).zip(start..original.lines.len()) {
            compact.lines[target] = original.lines[source].clone();
        }
        compact.cursor.y = 13;
        assert!(compact.ready(ProviderId::Claude));
        compact.cursor.y = original.cursor.y;
        assert!(!compact.ready(ProviderId::Claude));
        compact.cursor.y = 13;
        compact.lines[30] = "Run this command?".into();
        assert!(!compact.ready(ProviderId::Claude));
    }

    #[test]
    fn verified_native_captures_admit_empty_composers_and_reject_busy_and_trust_frames() {
        for (provider, body, expected) in [
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-fresh-initial-ready.json"),
                true,
            ),
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-resume-initial-ready.json"),
                true,
            ),
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-fresh-busy.json"),
                false,
            ),
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-resume-busy.json"),
                false,
            ),
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-fresh-trust.json"),
                false,
            ),
            (
                ProviderId::Cursor,
                include_str!("native_ui_readiness/fixtures/cursor-fresh-empty-placeholder.json"),
                true,
            ),
            (
                ProviderId::Cursor,
                include_str!("native_ui_readiness/fixtures/cursor-fresh-busy.json"),
                false,
            ),
            (
                ProviderId::Cursor,
                include_str!("native_ui_readiness/fixtures/cursor-fresh-trust.json"),
                false,
            ),
        ] {
            assert_eq!(capture(body).ready(provider), expected);
        }
    }

    #[test]
    fn drafts_unknown_builds_partial_paints_and_modals_never_admit_input() {
        for (provider, body) in [
            (
                ProviderId::Claude,
                include_str!("native_ui_readiness/fixtures/claude-fresh-initial-ready.json"),
            ),
            (
                ProviderId::Cursor,
                include_str!("native_ui_readiness/fixtures/cursor-fresh-empty-placeholder.json"),
            ),
        ] {
            for blocker in [
                "Run this command?",
                "Sign in to continue",
                "Enter to confirm",
                "Esc to cancel",
                "Running Stop hooks",
            ] {
                let mut view = capture(body);
                let end = view.lines.len() - 1;
                view.lines[end] = blocker.into();
                assert!(!view.ready(provider), "{blocker}");
            }
            let mut view = capture(body);
            view.version = "unverified-build".into();
            assert!(!view.ready(provider));
            let mut view = capture(body);
            view.pending_escape = true;
            assert!(!view.ready(provider));
            let mut view = capture(body);
            let prompt = view
                .lines
                .iter_mut()
                .rfind(|line| line.trim().starts_with(['❯', '→']))
                .unwrap();
            *prompt = "❯ An unsent local draft".into();
            assert!(!view.ready(provider));
        }
    }

    #[test]
    fn cursor_literal_placeholder_draft_is_distinct_from_empty_and_cleared_input() {
        for (body, ready) in [
            (
                include_str!("native_ui_readiness/fixtures/cursor-fresh-empty-placeholder.json"),
                true,
            ),
            (
                include_str!("native_ui_readiness/fixtures/cursor-fresh-literal-draft.json"),
                false,
            ),
            (
                include_str!("native_ui_readiness/fixtures/cursor-fresh-cleared-draft.json"),
                true,
            ),
        ] {
            assert_eq!(capture(body).ready(ProviderId::Cursor), ready);
        }
        let mut empty = capture(include_str!(
            "native_ui_readiness/fixtures/cursor-fresh-empty-placeholder.json"
        ));
        empty.styles.clear();
        assert!(!empty.ready(ProviderId::Cursor));
    }

    #[test]
    fn cursor_unfocused_app_placeholder_is_empty_but_literal_and_modal_frames_are_not() {
        let body = include_str!("native_ui_readiness/fixtures/cursor-app-unfocused-empty.json");
        let frame = capture(body);
        assert!(frame.ready(ProviderId::Cursor));
        let check = NativeUiReadiness::verified(ProviderId::Cursor, &frame.version).unwrap();
        assert!(check.empty_view(&frame.view()));
        let row = 28;
        let mut literal = capture(body);
        // Literal input has a normal arrow even when the text itself is dimmed/disabled.
        let arrow = literal.styles[row]
            .iter()
            .position(|run| run.start == 2)
            .unwrap();
        let mut text = literal.styles[row][arrow].clone();
        text.start = 4;
        literal.styles[row][arrow].end = 4;
        literal.styles[row][arrow].dim = false;
        literal.styles[row].insert(arrow + 1, text);
        assert!(!literal.ready(ProviderId::Cursor));
        let mut modal = capture(body);
        modal.lines[row + 3] = "Enter to confirm".into();
        assert!(!modal.ready(ProviderId::Cursor));
        let mut partial = capture(body);
        partial.pending_escape = true;
        assert!(!partial.ready(ProviderId::Cursor));
    }

    #[test]
    fn cursor_owned_background_paste_uses_its_layout_instead_of_wrapped_footer_height() {
        let body = include_str!("native_ui_readiness/fixtures/cursor-app-unfocused-empty.json");
        let text = "Synthetic background paste";
        let mut frame = capture(body);
        let row = 28;
        frame.lines[row] = format!("  → {text}");
        let padding = usize::from(frame.cols) - 1 - frame.lines[row].len();
        frame.lines[row].push_str(&" ".repeat(padding));
        frame.character_columns[row] = (0..frame.lines[row].chars().count() as u16).collect();
        for style in &mut frame.styles[row] {
            style.dim = false;
            style.inverse = false;
        }
        assert!(cursor_paste_applied(&frame.view(), text));
        assert!(!cursor_paste_applied(&frame.view(), "Another draft"));
        assert!(
            !frame.ready(ProviderId::Cursor),
            "owned paste is not an empty composer"
        );
        frame.styles[row]
            .iter_mut()
            .find(|run| run.start == 2)
            .unwrap()
            .dim = true;
        assert!(
            !cursor_paste_applied(&frame.view(), text),
            "disabled text is not a paint acknowledgment"
        );
        for style in &mut frame.styles[row] {
            style.dim = false;
        }
        frame.lines[row + 3] = "Run this command?".into();
        assert!(!cursor_paste_applied(&frame.view(), text));
    }

    #[test]
    fn cursor_acknowledges_native_wrapping_multiline_unicode_window_and_owned_paste_chip() {
        for (name, body, text) in [
            (
                "word-wrap",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-word-wrap.json"),
                "Use the shell tool once to run exactly `printf PROMETEU_TERMINAL_TOOL_SMOKE` in the current scratch workspace. Do not run any other command or use another tool. Then reply exactly PROMETEU_TERMINAL_SMOKE.".into(),
            ),
            (
                "wrapped",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-wrapped.json"),
                "PROMETEU_WRAPPED_".repeat(24),
            ),
            (
                "multiline",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-multiline.json"),
                "First synthetic line.\nSecond synthetic line.\nThird synthetic line.".into(),
            ),
            (
                "unicode",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-unicode.json"),
                "São Paulo 中文 🐣 e\u{301} ".repeat(8),
            ),
            (
                "scrolled",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-scrolled.json"),
                (0..14)
                    .map(|n| format!("Synthetic line {n:02}.\n"))
                    .collect(),
            ),
            (
                "chip",
                include_str!("native_ui_readiness/fixtures/cursor-fresh-chip.json"),
                "PROMETEU_PASTE_CHIP\n".repeat(70),
            ),
        ] {
            let capture = capture(body);
            let view = || View {
                title: &capture.title,
                lines: &capture.lines,
                styles: &capture.styles,
                character_columns: &capture.character_columns,
                cols: capture.cols,
                rows: capture.rows,
                cursor_x: capture.cursor.x,
                cursor_y: capture.cursor.y,
                cursor_hidden: capture.cursor_hidden,
                bracketed_paste: capture.bracketed_paste,
                partial: capture.pending_escape || capture.pending_utf8,
            };
            assert!(cursor_paste_applied(&view(), &text), "{name}");
            assert!(
                !cursor_paste_applied(&view(), "An unrelated draft"),
                "{name}"
            );
            assert!(!cursor_paste_applied(&view(), "Bad\x1binput"), "{name}");
            let mut partial = view();
            partial.partial = true;
            assert!(!cursor_paste_applied(&partial, &text));
            let mut unfocused: Capture = serde_json::from_str(body).unwrap();
            for row in &mut unfocused.styles {
                for run in row { run.inverse = false; }
            }
            assert!(cursor_paste_applied(&unfocused.view(), &text), "unfocused {name}");
        }
        assert!(
            NativeUiReadiness::verified(ProviderId::Cursor, "2026.11.13-eacade1")
                .unwrap()
                .requires_paste_ack()
        );
        assert!(
            !NativeUiReadiness::verified(ProviderId::Claude, "2.1.326 (Claude Code)")
                .unwrap()
                .requires_paste_ack()
        );
    }

    #[test]
    fn claude_colored_placeholder_and_clear_prove_empty_but_identical_home_draft_does_not() {
        let check =
            NativeUiReadiness::verified(ProviderId::Claude, "2.1.326 (Claude Code)").unwrap();
        for (body, empty) in [
            (
                include_str!("native_ui_readiness/fixtures/claude-fresh-empty-placeholder.json"),
                true,
            ),
            (
                include_str!("native_ui_readiness/fixtures/claude-fresh-literal-draft.json"),
                false,
            ),
            (
                include_str!("native_ui_readiness/fixtures/claude-fresh-literal-home.json"),
                false,
            ),
            (
                include_str!("native_ui_readiness/fixtures/claude-fresh-cleared-draft.json"),
                true,
            ),
        ] {
            let capture = capture(body);
            assert_eq!(check.empty_view(&capture.view()), empty);
        }
        let mut cleared = capture(include_str!(
            "native_ui_readiness/fixtures/claude-fresh-cleared-draft.json"
        ));
        for run in &mut cleared.styles[usize::from(cleared.cursor.y)] {
            run.dim = false;
        }
        assert!(
            !check.empty_view(&cleared.view()),
            "NO_COLOR cannot distinguish the literal placeholder"
        );
        cleared.lines[usize::from(cleared.cursor.y)] = "❯ ".into();
        assert!(check.empty_view(&cleared.view()));
        cleared
            .lines
            .last_mut()
            .unwrap()
            .push_str("Run this command?");
        assert!(!check.empty_view(&cleared.view()));
    }

    #[test]
    fn cursor_submission_normalization_preserves_unicode_payload() {
        assert_eq!(
            normalize_cursor_submission("A\r\nB\rC\tD e\u{301}"),
            "A\nB\nC  D e\u{301}"
        );
        assert_ne!(normalize_cursor_submission("e\u{301}"), "é");
    }

    #[test]
    fn a_busy_title_or_detached_cursor_vetoes_a_claude_composer() {
        let body = include_str!("native_ui_readiness/fixtures/claude-fresh-initial-ready.json");
        let mut view = capture(body);
        view.title = "Working on smoke".into();
        assert!(!view.ready(ProviderId::Claude));
        view.title = "✳ Native smoke".into();
        assert!(view.ready(ProviderId::Claude));
        view.cursor.y -= 1;
        assert!(!view.ready(ProviderId::Claude));
    }
}
