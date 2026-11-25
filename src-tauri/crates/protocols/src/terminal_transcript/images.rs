//! Bounded JSON string filtering for native inline images. All non-binary JSON remains intact.
use serde_json::Value;

const DEPTH_LIMIT: usize = 64;
const KEY_LIMIT: usize = 4096;
const MEDIA_LIMIT: usize = 256;
const FALLBACK_LIMIT: usize = super::TRANSCRIPT_LINE_LIMIT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineImageFormat {
    Claude,
    Codex,
}

struct Container {
    path: Vec<String>,
    object: bool,
    key: Option<String>,
    expecting_key: bool,
    index: usize,
}

#[derive(Default)]
struct Base64 {
    count: u64,
    padding: u8,
    last: u8,
    invalid: bool,
}
impl Base64 {
    fn push(&mut self, byte: u32) {
        if byte == b'=' as u32 {
            self.padding = self.padding.saturating_add(1);
            self.invalid |= self.padding > 2;
            return;
        }
        let decoded = match byte {
            65..=90 => Some((byte - 65) as u8),
            97..=122 => Some((byte - 97 + 26) as u8),
            48..=57 => Some((byte - 48 + 52) as u8),
            43 => Some(62),
            47 => Some(63),
            _ => None,
        };
        self.invalid |= decoded.is_none() || self.padding != 0;
        self.last = decoded.unwrap_or_default();
        self.count = self.count.saturating_add(1);
    }
    fn valid(&self) -> bool {
        if self.invalid {
            return false;
        }
        match (self.count % 4, self.padding) {
            (0, 0) => true,
            (2, 0 | 2) => self.last & 15 == 0,
            (3, 0 | 1) => self.last & 3 == 0,
            _ => false,
        }
    }
}

struct Candidate {
    path: Vec<String>,
    kind: InlineImageFormat,
    selected: bool,
    header: Vec<u8>,
    binary: Base64,
    original: Option<Vec<u8>>,
}
impl Candidate {
    /// False means an ordinary URL: keep its original bytes, with normal JSON validation.
    fn decoded(&mut self, byte: u32) -> bool {
        if self.selected {
            self.binary.push(byte);
            return true;
        }
        if byte > 127 || self.header.len() >= 128 {
            return false;
        }
        self.header.push(byte as u8);
        const PREFIX: &[u8] = b"data:image/";
        let n = self.header.len().min(PREFIX.len());
        if !self.header[..n].eq_ignore_ascii_case(&PREFIX[..n]) {
            return false;
        }
        if byte == b',' as u32 {
            let Some(mime) = self.header.strip_suffix(b";base64,") else {
                return false;
            };
            let subtype = &mime[PREFIX.len()..];
            if subtype.is_empty()
                || !subtype
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || b".+-".contains(byte))
            {
                return false;
            }
            self.selected = true;
        }
        true
    }
}

struct StringToken {
    key: bool,
    key_raw: Vec<u8>,
    candidate: Option<Candidate>,
    start: usize,
    escaped: bool,
    unicode_left: u8,
    unicode: u32,
}

pub(super) struct ImageFilter {
    format: InlineImageFormat,
    stack: Vec<Container>,
    string: Option<StringToken>,
    omitted: Vec<Candidate>,
    fallback_bytes: usize,
}

impl ImageFilter {
    pub fn new(format: InlineImageFormat) -> Self {
        Self {
            format,
            stack: Vec::new(),
            string: None,
            omitted: Vec::new(),
            fallback_bytes: 0,
        }
    }

    fn value_path(&self) -> Vec<String> {
        let Some(parent) = self.stack.last() else {
            return Vec::new();
        };
        let mut path = parent.path.clone();
        if parent.object {
            if let Some(key) = &parent.key {
                path.push(key.clone());
            }
        } else {
            path.push(parent.index.to_string());
        }
        path
    }

    fn candidate(&self, path: Vec<String>) -> Option<Candidate> {
        let matches = match self.format {
            InlineImageFormat::Claude => {
                matches!(path.len(), 5 | 7)
                    && path[0] == "message"
                    && path[1] == "content"
                    && path[2].parse::<usize>().is_ok()
                    && (path.len() == 5
                        || (path[3] == "content" && path[4].parse::<usize>().is_ok()))
                    && path[path.len() - 2] == "source"
                    && path[path.len() - 1] == "data"
            }
            InlineImageFormat::Codex => {
                (path.len() == 3
                    && path[0] == "payload"
                    && path[1] == "images"
                    && path[2].parse::<usize>().is_ok())
                    || (path.len() == 4
                        && path[0] == "payload"
                        && path[1] == "content"
                        && path[2].parse::<usize>().is_ok()
                        && path[3] == "image_url")
            }
        };
        matches.then(|| Candidate {
            path,
            kind: self.format,
            selected: self.format == InlineImageFormat::Claude,
            header: Vec::new(),
            binary: Base64::default(),
            original: Some(vec![b'"']),
        })
    }

    pub fn feed(&mut self, byte: u8, output: &mut Vec<u8>) -> Result<(), ()> {
        let Some(mut token) = self.string.take() else {
            let start = output.len();
            output.push(byte);
            match byte {
                b'"' => {
                    let key = self
                        .stack
                        .last()
                        .is_some_and(|frame| frame.object && frame.expecting_key);
                    let candidate = (!key).then(|| self.candidate(self.value_path())).flatten();
                    if candidate.is_some() {
                        self.fallback_bytes += 1;
                    }
                    self.string = Some(StringToken {
                        key,
                        key_raw: if key { vec![b'"'] } else { Vec::new() },
                        candidate,
                        start,
                        escaped: false,
                        unicode_left: 0,
                        unicode: 0,
                    });
                }
                b'{' | b'[' => {
                    if self.stack.len() >= DEPTH_LIMIT {
                        return Err(());
                    }
                    self.stack.push(Container {
                        path: self.value_path(),
                        object: byte == b'{',
                        key: None,
                        expecting_key: byte == b'{',
                        index: 0,
                    });
                }
                b'}' | b']' => {
                    self.stack.pop().ok_or(())?;
                }
                b',' => {
                    if let Some(frame) = self.stack.last_mut() {
                        if frame.object {
                            frame.key = None;
                            frame.expecting_key = true;
                        } else {
                            frame.index = frame.index.checked_add(1).ok_or(())?;
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        };
        if token.key {
            if token.key_raw.len() >= KEY_LIMIT {
                return Err(());
            }
            token.key_raw.push(byte);
        }
        if let Some(candidate) = &mut token.candidate {
            if let Some(original) = &mut candidate.original {
                if self.fallback_bytes < FALLBACK_LIMIT {
                    original.push(byte);
                    self.fallback_bytes += 1;
                } else {
                    candidate.original = None;
                }
            }
        }
        let closing = byte == b'"' && !token.escaped && token.unicode_left == 0;
        if closing
            || !token
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.selected)
        {
            output.push(byte);
        }
        if closing {
            if token.key {
                let key = serde_json::from_slice::<String>(&token.key_raw).map_err(|_| ())?;
                let frame = self.stack.last_mut().ok_or(())?;
                frame.key = Some(key);
                frame.expecting_key = false;
            }
            if let Some(candidate) = token.candidate.filter(|candidate| candidate.selected) {
                if self.omitted.len() >= MEDIA_LIMIT {
                    return Err(());
                }
                self.omitted.push(candidate);
            }
            return Ok(());
        }
        let decoded = if token.unicode_left > 0 {
            let hex = (byte as char).to_digit(16).ok_or(())?;
            token.unicode = token.unicode * 16 + hex;
            token.unicode_left -= 1;
            (token.unicode_left == 0).then_some(token.unicode)
        } else if token.escaped {
            token.escaped = false;
            match byte {
                b'u' => {
                    token.unicode_left = 4;
                    token.unicode = 0;
                    None
                }
                b'"' | b'\\' | b'/' => Some(byte as u32),
                b'b' => Some(8),
                b'f' => Some(12),
                b'n' => Some(10),
                b'r' => Some(13),
                b't' => Some(9),
                _ => return Err(()),
            }
        } else if byte == b'\\' {
            token.escaped = true;
            None
        } else {
            if byte < 32 {
                return Err(());
            }
            Some(byte as u32)
        };
        if let (Some(candidate), Some(decoded)) = (&mut token.candidate, decoded) {
            if !candidate.decoded(decoded) {
                token.candidate = None;
            } else if candidate.selected {
                output.truncate(token.start + 1);
            }
        }
        self.string = Some(token);
        Ok(())
    }

    pub fn finish(&self, value: &mut Value) -> Result<usize, ()> {
        if self.string.is_some() || !self.stack.is_empty() {
            return Err(());
        }
        let mut omitted = 0;
        for candidate in &self.omitted {
            let pointer = format!("/{}", candidate.path.join("/"));
            let image = match candidate.kind {
                InlineImageFormat::Claude => {
                    let block =
                        format!("/{}", candidate.path[..candidate.path.len() - 2].join("/"));
                    // Read-tool images are nested inside a tool_result content array.
                    (candidate.path.len() == 5
                        || value
                            .pointer(&format!("/message/content/{}/type", candidate.path[2]))
                            .is_some_and(|v| v == "tool_result"))
                        && value
                            .pointer(&format!("{block}/type"))
                            .is_some_and(|v| v == "image")
                        && value
                            .pointer(&format!("{block}/source/type"))
                            .is_some_and(|v| v == "base64")
                        && value
                            .pointer(&format!("{block}/source/media_type"))
                            .and_then(Value::as_str)
                            .is_some_and(|mime| mime.starts_with("image/"))
                }
                InlineImageFormat::Codex if candidate.path.len() == 3 => {
                    value["type"] == "event_msg" && value["payload"]["type"] == "user_message"
                }
                InlineImageFormat::Codex => {
                    value["type"] == "response_item"
                        && value["payload"]["type"] == "message"
                        && value
                            .pointer(&format!("/payload/content/{}/type", candidate.path[2]))
                            .is_some_and(|v| v == "input_image")
                }
            };
            let replacement = if image {
                if !candidate.binary.valid() {
                    return Err(());
                }
                omitted += 1;
                // Null explicitly marks unavailable binary content. Adjacent native metadata
                // remains intact, and TranscriptRecord records this intentional omission.
                Value::Null
            } else {
                let original = candidate.original.as_deref().ok_or(())?;
                Value::String(serde_json::from_slice(original).map_err(|_| ())?)
            };
            *value.pointer_mut(&pointer).ok_or(())? = replacement;
        }
        Ok(omitted)
    }
}
