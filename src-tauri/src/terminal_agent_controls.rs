//! Native interruption is a versioned UI control, not a Unix process signal. A verified version
//! alone never authorizes a default key: the current native footer must advertise that binding.
use crate::state::ProviderId;
use prometeu_process::terminal_state::{protocol::BufferKind, Snapshot};

#[derive(Clone, Copy, Debug)]
pub struct NativeControls {
    provider: ProviderId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    UnknownVersion,
    Inactive,
    IncompleteSnapshot,
    UnsupportedLayout,
    MissingObservedHint,
    ConflictingBinding,
}

impl NativeControls {
    pub fn verified(provider: ProviderId, version: &str) -> Result<Self, Unavailable> {
        let (at_least, min): (fn(&str, &str) -> bool, _) = match provider {
            ProviderId::Claude => (prometeu_protocols::cli_version::at_least, "2.1.326"),
            ProviderId::Codex => (prometeu_protocols::cli_version::at_least, "0.169.0"),
            ProviderId::Antigravity => (prometeu_protocols::cli_version::at_least, "1.5.9"),
            ProviderId::Cursor => (
                prometeu_protocols::cli_version::build_date_at_least,
                "2026.11.13",
            ),
            ProviderId::RetiredGemini => return Err(Unavailable::UnknownVersion),
        };
        at_least(version, min)
            .then_some(Self { provider })
            .ok_or(Unavailable::UnknownVersion)
    }

    /// Send at most once per observed active cycle, through the matching generation's input gate.
    /// The host owns duplicate suppression: repeated Ctrl+C can exit Cursor after it stops.
    pub fn interrupt(&self, snapshot: &Snapshot, active: bool) -> Option<&'static [u8]> {
        self.decision(snapshot, active).ok()
    }

    /// The pinned Cursor composer advertises Stop only while its native turn is running.
    /// This observation never sends input or establishes a successful completion.
    pub fn observes_working(&self, snapshot: &Snapshot) -> bool {
        self.provider == ProviderId::Cursor && self.decision(snapshot, true).is_ok()
    }

    pub fn decision(
        &self,
        snapshot: &Snapshot,
        active: bool,
    ) -> Result<&'static [u8], Unavailable> {
        if !active {
            return Err(Unavailable::Inactive);
        }
        let screen = match snapshot.active {
            BufferKind::Normal => &snapshot.normal,
            BufferKind::Alternate => snapshot
                .alternate
                .as_ref()
                .ok_or(Unavailable::IncompleteSnapshot)?,
        };
        if snapshot.v != 1
            || snapshot.cols < 60
            || snapshot.rows < 12
            || snapshot.viewport_lines.len() != usize::from(snapshot.rows)
            || screen.cursor.x >= snapshot.cols
            || screen.cursor.y >= snapshot.rows
            || !snapshot.modes.bracketed_paste_mode
            || !snapshot.pending_escape.is_empty()
            || !snapshot.pending_utf8.is_empty()
        {
            return Err(Unavailable::IncompleteSnapshot);
        }
        let lines = &snapshot.viewport_lines;
        match self.provider {
            ProviderId::Claude => {
                claude_hint(lines, snapshot.cols, screen.cursor.x, screen.cursor.y)
            }
            ProviderId::Cursor => cursor_hint(
                lines,
                snapshot.cols,
                screen.cursor.x,
                screen.cursor.y,
                snapshot.modes.cursor_hidden,
            ),
            ProviderId::Codex => codex_hint(
                lines,
                screen.cursor.x,
                screen.cursor.y,
                snapshot.modes.cursor_hidden,
            ),
            ProviderId::Antigravity => antigravity_hint(
                lines,
                snapshot.cols,
                screen.cursor.x,
                screen.cursor.y,
                snapshot.modes.cursor_hidden,
            ),
            ProviderId::RetiredGemini => Err(Unavailable::UnsupportedLayout),
        }
    }
}

fn codex_hint(
    lines: &[String],
    x: u16,
    y: u16,
    hidden: bool,
) -> Result<&'static [u8], Unavailable> {
    let Some(row) = lines.iter().rposition(|line| line.starts_with("› ")) else {
        return Err(Unavailable::UnsupportedLayout);
    };
    if hidden
        || x < 2
        || usize::from(y) != row
        || row < 3
        || row < lines.len().saturating_sub(6)
        || row + 2 >= lines.len()
        || !lines[row + 1].trim().is_empty()
        || lines[row + 2].trim().is_empty()
    {
        return Err(Unavailable::UnsupportedLayout);
    }
    // The native status is above the live composer. Startup MCP cancellation has the same
    // binding but is not interruption of a model turn; transcript mentions are not controls.
    let Some(status_row) = (row.saturating_sub(5)..row).rev().find(|index| {
        let text = lines[*index].trim();
        text.starts_with(['◦', '•'])
    }) else {
        return Err(Unavailable::MissingObservedHint);
    };
    if lines[status_row + 1..row].iter().any(|line| {
        let text = line.trim();
        !(text.is_empty() || text.starts_with("⚠ weekly limit:") && text.ends_with("· /status"))
    }) {
        return Err(Unavailable::UnsupportedLayout);
    }
    let status = lines[status_row].trim();
    let Some(body) = status
        .strip_prefix(['◦', '•'])
        .map(str::trim_start)
        .and_then(|text| text.strip_prefix("Working ("))
        .and_then(|text| text.strip_suffix(')'))
    else {
        return Err(Unavailable::MissingObservedHint);
    };
    let Some((elapsed, hint)) = body.rsplit_once(" • ") else {
        return Err(Unavailable::MissingObservedHint);
    };
    if !elapsed.ends_with('s')
        || !elapsed
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, 's' | 'm' | 'h' | ' '))
    {
        return Err(Unavailable::UnsupportedLayout);
    }
    if hint == "esc to interrupt" {
        return Ok(b"\x1b");
    }
    if hint.contains(" to interrupt") {
        return Err(Unavailable::ConflictingBinding);
    }
    Err(Unavailable::MissingObservedHint)
}

fn antigravity_hint(
    lines: &[String],
    cols: u16,
    x: u16,
    y: u16,
    hidden: bool,
) -> Result<&'static [u8], Unavailable> {
    let row = usize::from(y);
    if hidden
        || x < 2
        || row == 0
        || row + 2 >= lines.len()
        || !(lines[row] == ">" || lines[row].starts_with("> "))
        || !rule(&lines[row - 1], cols)
        || !rule(&lines[row + 1], cols)
        || lines[row + 3..].iter().any(|line| !line.trim().is_empty())
    {
        return Err(Unavailable::UnsupportedLayout);
    }
    // The left footer is separated from the model/effort label on the right. A prompt or
    // assistant response above the bordered composer cannot provide this control.
    let Some((hint, model)) = lines[row + 2].split_once("  ") else {
        return Err(Unavailable::MissingObservedHint);
    };
    if model.trim().is_empty() {
        return Err(Unavailable::UnsupportedLayout);
    }
    if hint == "esc to cancel" {
        return Ok(b"\x1b");
    }
    if hint.contains(" to cancel") {
        return Err(Unavailable::ConflictingBinding);
    }
    Err(Unavailable::MissingObservedHint)
}

fn rule(line: &str, cols: u16) -> bool {
    let text = line.trim();
    text.chars().count() >= usize::from(cols.saturating_sub(4)) && text.chars().all(|ch| ch == '─')
}

fn claude_hint(lines: &[String], cols: u16, x: u16, y: u16) -> Result<&'static [u8], Unavailable> {
    let Some(border) = lines.iter().rposition(|line| rule(line, cols)) else {
        return Err(Unavailable::UnsupportedLayout);
    };
    if border < 2
        || x < 2
        || usize::from(y) != border - 1
        || !lines[border - 1].trim_start().starts_with('❯')
        || !rule(&lines[border - 2], cols)
        || lines[(border + 5).min(lines.len())..]
            .iter()
            .any(|line| !line.trim().is_empty())
    {
        return Err(Unavailable::UnsupportedLayout);
    }
    let Some(footer) = lines[border + 1..].iter().find(|line| {
        line.split('·').next().is_some_and(|part| {
            part.trim_end().ends_with("(shift+tab to cycle)") || part.trim() == "⏸ manual mode on"
        })
    }) else {
        return Err(Unavailable::MissingObservedHint);
    };
    if footer
        .split('·')
        .any(|part| part.trim() == "esc to interrupt")
    {
        return Ok(b"\x1b");
    }
    if footer.contains(" to interrupt") {
        return Err(Unavailable::ConflictingBinding);
    }
    Err(Unavailable::MissingObservedHint)
}

fn cursor_hint(
    lines: &[String],
    cols: u16,
    x: u16,
    y: u16,
    hidden: bool,
) -> Result<&'static [u8], Unavailable> {
    let Some(row) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("→ "))
    else {
        return Err(Unavailable::UnsupportedLayout);
    };
    if row == 0
        || row + 4 >= lines.len()
        || !lines[row - 1].trim().is_empty()
        || !lines[row + 1].trim().is_empty()
        || !lines[row + 2].trim().is_empty()
        || (lines[row + 3].trim().is_empty()
            && !lines[..row]
                .iter()
                .any(|line| line.trim() == "Cursor Agent"))
        || lines[row + 4].trim().is_empty()
        || !hidden
        || x != 0
        || usize::from(y) < row
        || usize::from(y) > row + 6
        || lines[usize::from(y) + 1..]
            .iter()
            .any(|line| !line.trim().is_empty())
    {
        return Err(Unavailable::UnsupportedLayout);
    }
    let text = lines[row].trim_end();
    // The native hint is right-aligned on the composer row, separate from its input text.
    let Some((_, hint)) = text.rsplit_once("  ") else {
        return Err(Unavailable::MissingObservedHint);
    };
    if hint == "ctrl+c to stop" {
        // The initial banner scrolls out during ordinary long replies. The live composer stays
        // right-aligned above the model/workspace footer and the current physical cursor.
        if text.chars().count() != usize::from(cols.saturating_sub(2)) {
            return Err(Unavailable::UnsupportedLayout);
        }
        return Ok(b"\x03");
    }
    if hint.contains(" to stop") {
        return Err(Unavailable::ConflictingBinding);
    }
    Err(Unavailable::MissingObservedHint)
}

#[cfg(test)]
mod smoke;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::snapshot as test_snapshot;
