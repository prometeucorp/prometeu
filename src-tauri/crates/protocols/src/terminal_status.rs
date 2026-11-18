//! Bounded observations of Program Status Protocol (OSC 7501) output, independent of rendering.
mod state;
pub use state::{
    ProgramStatusSnapshot, ProgramStatusStore, ProgramStatusTransition, PROGRAM_STATUS_RECORD_LIMIT,
};

use base64::{
    alphabet,
    engine::{general_purpose::GeneralPurpose, DecodePaddingMode, GeneralPurposeConfig},
    Engine,
};

const MAX_SEQUENCE: usize = 4096;
const MAX_PAYLOAD: usize = MAX_SEQUENCE - 3;
const TEXT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Send only for a live query when the host implements the reported states and record lifecycle.
/// Replaying a terminal snapshot must never send this reply to the agent again.
pub const PROGRAM_STATUS_QUERY_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramStatusState {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramStatusKind {
    Permission,
    Question,
    Auth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramStatusReport {
    /// Absence addresses the root; an invalid child id never falls back to the root.
    pub id: Option<String>,
    pub state: ProgramStatusState,
    pub app: Option<String>,
    pub kind: Option<ProgramStatusKind>,
    pub progress: Option<u8>,
    /// Untrusted, single-line text; consumers must not interpret it as markup or execution state.
    pub message: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramStatusEvent {
    Query,
    Report(ProgramStatusReport),
}

#[derive(Clone, Copy, Default)]
enum Scan {
    #[default]
    Text,
    Escape,
    Osc,
    OscEscape,
    String,
    StringEscape,
}

/// One parser belongs to one PTY incarnation. Feeding bytes neither changes nor consumes the
/// original output. Record storage, process exit, shell prompts and notifications belong to hosts.
#[derive(Default)]
pub struct ProgramStatusParser {
    scan: Scan,
    payload: Vec<u8>,
    overflow: bool,
}

impl ProgramStatusParser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ProgramStatusEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            match self.scan {
                Scan::Text => {
                    if byte == 0x1b {
                        self.scan = Scan::Escape;
                    }
                }
                Scan::Escape => self.escape(byte),
                Scan::Osc => match byte {
                    0x07 => self.finish(1, &mut events),
                    0x1b => self.scan = Scan::OscEscape,
                    0x18 | 0x1a => self.scan = Scan::Text,
                    _ if self.payload.len() < MAX_PAYLOAD => self.payload.push(byte),
                    _ => self.overflow = true,
                },
                Scan::OscEscape => {
                    if byte == b'\\' {
                        self.finish(2, &mut events);
                    } else {
                        self.escape(byte);
                    }
                }
                // DCS, SOS, PM and APC contain opaque text, not nested status reports.
                Scan::String => match byte {
                    0x1b => self.scan = Scan::StringEscape,
                    0x18 | 0x1a => self.scan = Scan::Text,
                    _ => {}
                },
                Scan::StringEscape => {
                    self.scan = match byte {
                        b'\\' | 0x18 | 0x1a => Scan::Text,
                        0x1b => Scan::StringEscape,
                        _ => Scan::String,
                    };
                }
            }
        }
        events
    }

    fn escape(&mut self, byte: u8) {
        self.scan = match byte {
            b']' => {
                self.payload.clear();
                self.overflow = false;
                Scan::Osc
            }
            b'P' | b'X' | b'^' | b'_' => Scan::String,
            0x1b => Scan::Escape,
            _ => Scan::Text,
        };
    }

    fn finish(&mut self, terminator: usize, events: &mut Vec<ProgramStatusEvent>) {
        if !self.overflow && self.payload.len() + 2 + terminator <= MAX_SEQUENCE {
            if let Some(event) = parse(&self.payload) {
                events.push(event);
            }
        }
        self.payload.clear();
        self.scan = Scan::Text;
    }
}

fn parse(payload: &[u8]) -> Option<ProgramStatusEvent> {
    let body = payload.strip_prefix(b"7501;")?;
    if body.trim_ascii() == b"?" {
        return Some(ProgramStatusEvent::Query);
    }
    let mut state = None;
    let (mut id, mut app, mut kind, mut progress, mut message, mut title) =
        (None, None, None, None, None, None);
    for pair in body.split(|byte| *byte == b':') {
        let Some(at) = pair.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = pair[..at].trim_ascii();
        let value = pair[at + 1..].trim_ascii();
        // Check every occurrence before applying last-value-wins semantics.
        if key.len() > 16
            || (key == b"msg" && value.len() > 2732)
            || (key == b"title" && value.len() > 256)
            || (key == b"app" && value.len() > 32)
            || (key == b"id" && !valid_id(value))
        {
            return None;
        }
        if key.is_empty()
            || !key.iter().all(u8::is_ascii_lowercase)
            || !value
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(byte))
        {
            continue;
        }
        match key {
            b"state" => {
                state = match value {
                    b"idle" => Some(ProgramStatusState::Idle),
                    b"working" => Some(ProgramStatusState::Working),
                    b"blocked" => Some(ProgramStatusState::Blocked),
                    b"done" => Some(ProgramStatusState::Done),
                    b"error" => Some(ProgramStatusState::Error),
                    b"clear" => Some(ProgramStatusState::Clear),
                    _ => None,
                }
            }
            b"id" => id = Some(String::from_utf8(value.to_vec()).ok()?),
            b"app" => {
                app = valid_segment(value)
                    .then(|| String::from_utf8(value.to_vec()).expect("validated ASCII"));
            }
            b"kind" => {
                kind = match value {
                    b"permission" => Some(ProgramStatusKind::Permission),
                    b"question" => Some(ProgramStatusKind::Question),
                    b"auth" => Some(ProgramStatusKind::Auth),
                    _ => None,
                }
            }
            b"progress" => {
                progress = std::str::from_utf8(value)
                    .ok()
                    .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|text| text.parse::<u8>().ok())
                    .filter(|value| *value <= 100);
            }
            b"msg" => message = Some(decode_text(value, 2048)?),
            b"title" => title = Some(decode_text(value, 192)?),
            _ => {}
        }
    }
    let state = state?;
    Some(ProgramStatusEvent::Report(ProgramStatusReport {
        id,
        state,
        app,
        kind: (state == ProgramStatusState::Blocked)
            .then_some(kind)
            .flatten(),
        progress: matches!(
            state,
            ProgramStatusState::Working | ProgramStatusState::Blocked
        )
        .then_some(progress)
        .flatten(),
        message,
        title,
    }))
}

fn valid_segment(value: &[u8]) -> bool {
    (1..=32).contains(&value.len())
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.+-".contains(byte))
}

fn valid_id(value: &[u8]) -> bool {
    value.len() <= 128
        && value.split(|byte| *byte == b'/').count() <= 8
        && value.split(|byte| *byte == b'/').all(valid_segment)
}

fn decode_text(value: &[u8], limit: usize) -> Option<String> {
    let decoded = TEXT_BASE64.decode(value).ok()?;
    if decoded.len() > limit {
        return None;
    }
    let text = String::from_utf8(decoded).ok()?;
    (!text.chars().any(char::is_control)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str) -> Option<ProgramStatusReport> {
        let mut parser = ProgramStatusParser::default();
        match parser
            .feed(format!("\x1b]7501;{body}\x1b\\").as_bytes())
            .pop()?
        {
            ProgramStatusEvent::Report(report) => Some(report),
            ProgramStatusEvent::Query => None,
        }
    }

    #[test]
    fn terminal_status_survives_every_chunk_boundary_without_changing_output() {
        let bytes = b"screen \xf0\x9f\xa6\x80\x1b]7501;?\x07\x1b]7501;state=blocked:id=worker/one:kind=permission:app=claude-code:progress=40:msg=QXBwcm92ZT8=:title=V29ya2Vy\x1b\\tail";
        let expected = vec![
            ProgramStatusEvent::Query,
            ProgramStatusEvent::Report(ProgramStatusReport {
                id: Some("worker/one".into()),
                state: ProgramStatusState::Blocked,
                app: Some("claude-code".into()),
                kind: Some(ProgramStatusKind::Permission),
                progress: Some(40),
                message: Some("Approve?".into()),
                title: Some("Worker".into()),
            }),
        ];
        for split in 0..=bytes.len() {
            let mut parser = ProgramStatusParser::default();
            let mut actual = parser.feed(&bytes[..split]);
            actual.extend(parser.feed(&bytes[split..]));
            assert_eq!(actual, expected, "split at {split}");
        }
        let mut parser = ProgramStatusParser::default();
        let actual: Vec<_> = bytes
            .chunks(1)
            .flat_map(|chunk| parser.feed(chunk))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn terminal_status_ignores_other_control_strings_and_recovers_after_aborts() {
        let mut parser = ProgramStatusParser::default();
        assert!(parser
            .feed(b"\x1bPtext\x1b]7501;state=error\x07\x1b\\")
            .is_empty());
        assert!(parser.feed(b"\x1b]0;state=done\x07").is_empty());
        assert!(parser.feed(b"\x1b]7501;state=working\x18").is_empty());
        assert_eq!(
            parser.feed(b"\x1b]7501;state=error\x1b]7501;?\x1b\\"),
            vec![ProgramStatusEvent::Query]
        );
        assert!(parser.feed(PROGRAM_STATUS_QUERY_REPLY).is_empty());
        assert!(parser.feed(b"plain text and no terminal signal").is_empty());
    }

    #[test]
    fn terminal_status_bounds_sequences_including_their_terminator() {
        let mut parser = ProgramStatusParser::default();
        let body = format!("7501;state=working:x={}", "a".repeat(4072));
        assert_eq!(body.len(), 4093);
        assert_eq!(parser.feed(format!("\x1b]{body}\x07").as_bytes()).len(), 1);
        assert!(parser
            .feed(format!("\x1b]{body}\x1b\\").as_bytes())
            .is_empty());
        assert!(parser.feed(b"\x1b]7501;").is_empty());
        for _ in 0..100 {
            assert!(parser.feed(&[b'a'; 4096]).is_empty());
            assert!(parser.payload.len() <= MAX_PAYLOAD);
        }
        assert_eq!(
            parser.feed(b"\x07\x1b]7501;?\x07"),
            vec![ProgramStatusEvent::Query]
        );
    }

    #[test]
    fn terminal_status_validates_every_field_before_accepting_a_record() {
        for body in [
            "id=worker",
            "state=unknown",
            "state=working:state=unknown",
            "state=done:id=",
            "state=done:id=worker//one",
            "state=done:id=/worker",
            "state=done:id=a/b/c/d/e/f/g/h/i",
            "state=done:msg=___",
            "state=done:msg=/w==",
            "state=done:msg=Cg==",
            "state=done:msg=woA=",
            "state=done:msg=Cg==:msg=T0s=",
            "state=done:abcdefghijklmnopq=x",
        ] {
            assert!(report(body).is_none(), "accepted {body}");
        }
        for body in [
            format!("state=done:id={}", "a".repeat(33)),
            format!("state=done:app={}", "a".repeat(33)),
            format!("state=done:title={}", "a".repeat(257)),
            format!("state=done:msg={}", "a".repeat(2733)),
            format!("state=done:msg={}", TEXT_BASE64.encode("x".repeat(2049))),
        ] {
            assert!(report(&body).is_none());
        }
    }

    #[test]
    fn terminal_status_keeps_native_state_and_child_identity_without_inference() {
        for (name, expected) in [
            ("idle", ProgramStatusState::Idle),
            ("working", ProgramStatusState::Working),
            ("blocked", ProgramStatusState::Blocked),
            ("done", ProgramStatusState::Done),
            ("error", ProgramStatusState::Error),
            ("clear", ProgramStatusState::Clear),
        ] {
            let result = report(&format!("state={name}:id=worker/one")).unwrap();
            assert_eq!(result.state, expected);
            assert_eq!(result.id.as_deref(), Some("worker/one"));
        }
        let result = report(
            "state=working:bad pair:x=!:unknown=value:progress=00040:app=bad/app:kind=auth:msg=T0s",
        )
        .unwrap();
        assert_eq!(result.progress, Some(40));
        assert_eq!(result.message.as_deref(), Some("OK"));
        assert_eq!(result.app, None);
        assert_eq!(result.kind, None);
        assert_eq!(
            report("state=done:progress=40:kind=question")
                .unwrap()
                .progress,
            None
        );
        assert_eq!(
            report("state=blocked:progress=101:kind=unknown")
                .unwrap()
                .kind,
            None
        );
        assert_eq!(
            report("state=working:progress=0").unwrap().progress,
            Some(0)
        );
        assert_eq!(report("state=working:progress=+1").unwrap().progress, None);
        assert_eq!(
            report("state=working:state=done").unwrap().state,
            ProgramStatusState::Done
        );
        assert_eq!(
            report(" state = blocked : kind = question ").unwrap().kind,
            Some(ProgramStatusKind::Question)
        );
        assert_eq!(
            report("state=idle:msg=4pyT").unwrap().message.as_deref(),
            Some("✓")
        );
    }
}
