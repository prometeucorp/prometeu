//! Wire v1 mirrors packages/terminal-state/src/types.ts, independently of Tauri.
use serde::{Deserialize, Serialize};

pub const VERSION: u8 = 1;
pub const MAX_WRITE_BYTES: usize = 256 * 1024;
pub const MAX_REQUEST_BYTES: usize = 512 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024 + 1024;
pub const MAX_SAFE_SEQUENCE: u64 = 9_007_199_254_740_991;
pub const MAX_VIEWPORT_CHARACTERS: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Key {
    pub session: String,
    pub generation: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Serialize)]
pub(super) struct Request<'a> {
    pub v: u8,
    pub id: String,
    #[serde(flatten)]
    pub key: &'a Key,
    #[serde(flatten)]
    pub operation: Operation<'a>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(super) enum Operation<'a> {
    Create {
        #[serde(flatten)]
        size: Size,
        scrollback: u16,
    },
    Write {
        data: &'a str,
        seq: u64,
    },
    Resize {
        #[serde(flatten)]
        size: Size,
        seq: u64,
    },
    Snapshot {
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        view: bool,
    },
    Dispose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    InvalidRequest,
    Limit,
    NotFound,
    StaleGeneration,
    Sequence,
    AlreadyExists,
    SnapshotUnavailable,
    Closed,
    Internal,
}

#[derive(Deserialize)]
pub(super) struct Rejected {
    pub code: Rejection,
}

#[derive(Deserialize)]
pub(super) struct Response {
    pub v: u8,
    pub id: Option<String>,
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<Rejected>,
}

#[derive(Deserialize)]
pub(super) struct Ack {
    pub seq: u64,
}

#[derive(Deserialize)]
pub(super) struct WriteAck {
    pub seq: u64,
    pub replies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub wrap_pending: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedCursor {
    pub x: u16,
    pub y: u16,
    pub origin: bool,
    pub attributes: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WrapCell {
    pub col: u16,
    pub ansi: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub ansi: String,
    pub cursor: Cursor,
    pub scroll_top: u16,
    pub scroll_bottom: u16,
    pub saved_cursor: SavedCursor,
    pub wrap_cell: Option<WrapCell>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BufferKind {
    Normal,
    Alternate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseTracking {
    None,
    X10,
    Vt200,
    Drag,
    Any,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MouseEncoding {
    Default,
    Sgr,
    #[serde(rename = "SGR_PIXELS")]
    SgrPixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyle {
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Modes {
    pub application_cursor_keys_mode: bool,
    pub application_keypad_mode: bool,
    pub bracketed_paste_mode: bool,
    pub insert_mode: bool,
    pub origin_mode: bool,
    pub reverse_wraparound_mode: bool,
    pub send_focus_mode: bool,
    pub wraparound_mode: bool,
    pub mouse_tracking_mode: MouseTracking,
    pub mouse_encoding: MouseEncoding,
    pub cursor_hidden: bool,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OscLink {
    pub buffer: BufferKind,
    pub row: u32,
    pub start: u16,
    pub end: u16,
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    pub foreground: String,
    pub background: String,
    pub cursor: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Default,
    Palette,
    Rgb,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalColor {
    pub mode: ColorMode,
    pub value: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewportStyleRun {
    pub start: u16,
    pub end: u16,
    pub dim: bool,
    pub inverse: bool,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub v: u8,
    pub session: String,
    pub generation: String,
    /// Byte offset in the helper, not TerminalOutput's chunk sequence.
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
    pub scrollback: u16,
    pub viewport_lines: Vec<String>,
    pub viewport_character_columns: Vec<Vec<u16>>,
    pub viewport_styles: Vec<Vec<ViewportStyleRun>>,
    pub appearance: Appearance,
    pub active: BufferKind,
    pub normal: Screen,
    pub alternate: Option<Screen>,
    pub modes: Modes,
    pub rehydrate: String,
    pub title: String,
    pub title_truncated: bool,
    pub links: Vec<OscLink>,
    pub pending_escape: String,
    pub pending_utf8: String,
}
