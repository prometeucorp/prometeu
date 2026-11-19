//! Read-only decoding of the installed Cursor CLI's resumable store. No filesystem, clock or
//! publication lives here. Unknown protobuf fields are skipped only after validating framing.
use serde_json::Value;
use sha2::{Digest, Sha256};

pub mod projection;
mod tools;
pub use tools::{tool, CursorTool, CursorToolResult};

const TEXT_LIMIT: usize = 1024 * 1024;
const FIELD_LIMIT: usize = 65_536;
const REFERENCE_LIMIT: usize = 65_536;
const META_LIMIT: usize = 65_536;
const BUFFER: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStoreSchema {
    Cli20261113,
}
impl CursorStoreSchema {
    pub fn for_version(version: &str) -> Option<Self> {
        (version == "2026.11.13-eacade1").then_some(Self::Cli20261113)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Identity,
    Malformed,
    Missing,
    Limit,
    Integrity,
    Read,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId(pub [u8; 32]);
impl BlobId {
    pub fn hex(&self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
    pub fn from_hex(value: &str) -> Result<Self, StoreError> {
        if value.len() != 64 {
            return Err(StoreError::Malformed);
        }
        let bytes = decode_hex(value)?;
        Self::from_bytes(&bytes)
    }
    fn from_bytes(value: &[u8]) -> Result<Self, StoreError> {
        Ok(Self(value.try_into().map_err(|_| StoreError::Malformed)?))
    }
}

/// Reads only a blob already referenced by the authenticated session's root. Implementations
/// must bound each read to the supplied destination and retain one consistent source snapshot.
pub trait BlobSource {
    fn len(&self, id: BlobId) -> Result<u64, StoreError>;
    fn read_at(&self, id: BlobId, offset: u64, destination: &mut [u8]) -> Result<(), StoreError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorMetadata {
    pub native_session: String,
    pub root: Option<BlobId>,
}

/// The actual SQLite metadata value is hex-encoded UTF-8 JSON. Credentials and unrelated
/// metadata are neither returned nor used as authority to select another conversation.
pub fn metadata(value: &str, expected_session: &str) -> Result<CursorMetadata, StoreError> {
    if value.len() > META_LIMIT * 2 || !valid_id(expected_session) {
        return Err(StoreError::Limit);
    }
    let raw = decode_hex(value)?;
    let value: Value = serde_json::from_slice(&raw).map_err(|_| StoreError::Malformed)?;
    if value["agentId"].as_str() != Some(expected_session)
        || value
            .get("subagentInfo")
            .is_some_and(|value| !value.is_null())
    {
        return Err(StoreError::Identity);
    }
    let root = value["latestRootBlobId"]
        .as_str()
        .ok_or(StoreError::Malformed)?;
    Ok(CursorMetadata {
        native_session: expected_session.into(),
        root: if root.is_empty() {
            None
        } else {
            Some(BlobId::from_hex(root)?)
        },
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorRoot {
    pub version: BlobId,
    pub turns: Vec<BlobId>,
    pub last_run: Option<CursorRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorTurnKey {
    pub user_message_id: String,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorRunStatus {
    Unknown,
    Running,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorRun {
    pub id: String,
    pub status: CursorRunStatus,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorUser {
    pub version: BlobId,
    pub message_id: String,
    pub text: String,
    pub simulated: bool,
    pub sent_by_agent: Option<String>,
    pub started_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorTurn {
    Agent {
        version: BlobId,
        key: CursorTurnKey,
        user: Box<CursorUser>,
        steps: Vec<BlobId>,
        run: Option<CursorRun>,
    },
    /// Shell-only turns are not agent answers. References remain private for an adapter.
    Shell {
        version: BlobId,
        command: BlobId,
        output: BlobId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlobRange {
    pub blob: BlobId,
    pub offset: u64,
    pub length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorStepContent {
    Assistant(String),
    Thinking(String),
    /// Vendor tool payload remains at the protocol edge until its particular schema is adapted.
    /// The kind's wire number is not an application tool name or an answerable request.
    Tool {
        id: Option<String>,
        kind: u32,
        payload: BlobRange,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorStep {
    pub version: BlobId,
    pub content: CursorStepContent,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
}

pub fn root(
    source: &dyn BlobSource,
    id: BlobId,
    _: CursorStoreSchema,
) -> Result<CursorRoot, StoreError> {
    let wire = Wire::verified(source, id)?;
    Ok(CursorRoot {
        version: id,
        turns: wire.references(8)?,
        last_run: run(&wire, 52)?,
    })
}

fn run(wire: &Wire<'_>, number: u32) -> Result<Option<CursorRun>, StoreError> {
    wire.field(number)?
        .map(|field| {
            let run = wire.nested(field)?;
            Ok(CursorRun {
                id: run.id(1)?.ok_or(StoreError::Identity)?,
                status: match run.integer(2)?.unwrap_or(0) {
                    0 => CursorRunStatus::Unknown,
                    1 => CursorRunStatus::Running,
                    2 => CursorRunStatus::Completed,
                    3 => CursorRunStatus::Failed,
                    4 => CursorRunStatus::Interrupted,
                    _ => return Err(StoreError::Unsupported),
                },
                started_at_ms: run.integer(3)?,
                finished_at_ms: run.integer(4)?,
            })
        })
        .transpose()
}

pub fn turn(
    source: &dyn BlobSource,
    id: BlobId,
    _: CursorStoreSchema,
) -> Result<CursorTurn, StoreError> {
    let wire = Wire::verified(source, id)?;
    let selected = wire.oneof(&[1, 2])?;
    let content = wire.nested(selected)?;
    if selected.number == 2 {
        return Ok(CursorTurn::Shell {
            version: id,
            command: content.reference(1)?.ok_or(StoreError::Malformed)?,
            output: content.reference(2)?.ok_or(StoreError::Malformed)?,
        });
    }
    let user_blob = content.reference(1)?.ok_or(StoreError::Malformed)?;
    let user_wire = Wire::verified(source, user_blob)?;
    let message_id = user_wire.id(2)?.ok_or(StoreError::Identity)?;
    let turn_message_id = content.id(10)?;
    if turn_message_id.as_ref().is_some_and(|id| id != &message_id) {
        return Err(StoreError::Identity);
    }
    let text = match user_wire.reference(18)? {
        Some(id) => verified_text(source, id)?,
        None => user_wire.text(1)?.unwrap_or_default(),
    };
    let run = run(&content, 17)?;
    Ok(CursorTurn::Agent {
        version: id,
        key: CursorTurnKey {
            user_message_id: message_id.clone(),
            request_id: content.id(3)?,
            run_id: run.as_ref().map(|run| run.id.clone()),
        },
        user: Box::new(CursorUser {
            version: user_blob,
            message_id,
            text,
            simulated: match user_wire.integer(5)? {
                None | Some(0) => false,
                Some(1) => true,
                _ => return Err(StoreError::Malformed),
            },
            sent_by_agent: user_wire.id(27)?,
            started_at_ms: user_wire.integer(25)?,
        }),
        steps: content.references(2)?,
        run,
    })
}

pub fn step(
    source: &dyn BlobSource,
    id: BlobId,
    _: CursorStoreSchema,
) -> Result<CursorStep, StoreError> {
    let wire = Wire::verified(source, id)?;
    let selected = wire.oneof(&[1, 2, 3])?;
    let content = wire.nested(selected)?;
    let (body, started, completed) = match selected.number {
        1 => (
            CursorStepContent::Assistant(content.text(1)?.unwrap_or_default()),
            2,
            3,
        ),
        3 => (
            CursorStepContent::Thinking(content.text(1)?.unwrap_or_default()),
            3,
            4,
        ),
        2 => {
            let tool = content
                .fields
                .iter()
                .filter(|field| ![54, 57, 59, 60].contains(&field.number))
                .collect::<Vec<_>>();
            if tool.len() != 1 || tool[0].wire != 2 {
                return Err(StoreError::Unsupported);
            }
            let field = tool[0];
            (
                CursorStepContent::Tool {
                    id: content
                        .text(57)?
                        .map(|id| {
                            if valid_tool_id(&id) {
                                Ok(id)
                            } else {
                                Err(StoreError::Identity)
                            }
                        })
                        .transpose()?,
                    kind: field.number,
                    payload: BlobRange {
                        blob: id,
                        offset: field.offset,
                        length: field.length,
                    },
                },
                59,
                60,
            )
        }
        _ => unreachable!(),
    };
    Ok(CursorStep {
        version: id,
        content: body,
        started_at_ms: content.integer(started)?,
        completed_at_ms: content.integer(completed)?,
    })
}

fn verified_text(source: &dyn BlobSource, id: BlobId) -> Result<String, StoreError> {
    let length = verify(source, id)?;
    let bytes = read_bytes(source, id, 0, length, TEXT_LIMIT)?;
    String::from_utf8(bytes).map_err(|_| StoreError::Malformed)
}

fn verify(source: &dyn BlobSource, id: BlobId) -> Result<u64, StoreError> {
    let length = source.len(id)?;
    let mut offset = 0;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; BUFFER];
    while offset < length {
        let count = (length - offset).min(BUFFER as u64) as usize;
        source.read_at(id, offset, &mut buffer[..count])?;
        hash.update(&buffer[..count]);
        offset += count as u64;
    }
    if hash.finalize().as_slice() != id.0 {
        return Err(StoreError::Integrity);
    }
    Ok(length)
}

#[derive(Clone, Copy)]
struct Field {
    number: u32,
    wire: u8,
    offset: u64,
    length: u64,
    integer: u64,
}
struct Wire<'a> {
    source: &'a dyn BlobSource,
    id: BlobId,
    fields: Vec<Field>,
    depth: u8,
}
impl<'a> Wire<'a> {
    fn verified(source: &'a dyn BlobSource, id: BlobId) -> Result<Self, StoreError> {
        let length = verify(source, id)?;
        Self::parse(source, id, 0, length, 0)
    }
    fn parse(
        source: &'a dyn BlobSource,
        id: BlobId,
        start: u64,
        length: u64,
        depth: u8,
    ) -> Result<Self, StoreError> {
        if depth > 16 {
            return Err(StoreError::Limit);
        }
        let end = start.checked_add(length).ok_or(StoreError::Malformed)?;
        let mut cursor = start;
        let mut fields = Vec::new();
        while cursor < end {
            if fields.len() == FIELD_LIMIT {
                return Err(StoreError::Limit);
            }
            let tag = varint(source, id, &mut cursor, end)?;
            let number = u32::try_from(tag >> 3).map_err(|_| StoreError::Malformed)?;
            if number == 0 || number >= 1 << 29 {
                return Err(StoreError::Malformed);
            }
            let wire = (tag & 7) as u8;
            let integer = if wire == 0 {
                varint(source, id, &mut cursor, end)?
            } else {
                0
            };
            let length = match wire {
                0 => 0,
                1 => 8,
                2 => varint(source, id, &mut cursor, end)?,
                5 => 4,
                _ => return Err(StoreError::Unsupported),
            };
            let offset = cursor;
            cursor = cursor
                .checked_add(length)
                .filter(|next| *next <= end)
                .ok_or(StoreError::Malformed)?;
            fields.push(Field {
                number,
                wire,
                offset,
                length,
                integer,
            });
        }
        Ok(Self {
            source,
            id,
            fields,
            depth,
        })
    }
    fn field(&self, number: u32) -> Result<Option<Field>, StoreError> {
        let mut fields = self.fields.iter().filter(|field| field.number == number);
        let first = fields.next().copied();
        if fields.next().is_some() {
            return Err(StoreError::Malformed);
        }
        Ok(first)
    }
    fn oneof(&self, numbers: &[u32]) -> Result<Field, StoreError> {
        let selected = self
            .fields
            .iter()
            .filter(|field| numbers.contains(&field.number))
            .collect::<Vec<_>>();
        if selected.len() != 1 {
            return Err(StoreError::Malformed);
        }
        Ok(*selected[0])
    }
    fn nested(&self, field: Field) -> Result<Self, StoreError> {
        if field.wire != 2 {
            return Err(StoreError::Malformed);
        }
        Self::parse(
            self.source,
            self.id,
            field.offset,
            field.length,
            self.depth + 1,
        )
    }
    fn integer(&self, number: u32) -> Result<Option<u64>, StoreError> {
        self.field(number)?
            .map(|field| {
                if field.wire == 0 {
                    Ok(field.integer)
                } else {
                    Err(StoreError::Malformed)
                }
            })
            .transpose()
    }
    fn text(&self, number: u32) -> Result<Option<String>, StoreError> {
        self.field(number)?
            .map(|field| {
                if field.wire != 2 {
                    return Err(StoreError::Malformed);
                }
                String::from_utf8(read_bytes(
                    self.source,
                    self.id,
                    field.offset,
                    field.length,
                    TEXT_LIMIT,
                )?)
                .map_err(|_| StoreError::Malformed)
            })
            .transpose()
    }
    fn id(&self, number: u32) -> Result<Option<String>, StoreError> {
        self.text(number)?
            .map(|id| {
                if valid_id(&id) {
                    Ok(id)
                } else {
                    Err(StoreError::Identity)
                }
            })
            .transpose()
    }
    fn reference(&self, number: u32) -> Result<Option<BlobId>, StoreError> {
        self.field(number)?
            .map(|field| self.reference_field(field))
            .transpose()
    }
    fn reference_field(&self, field: Field) -> Result<BlobId, StoreError> {
        if field.wire != 2 || field.length != 32 {
            return Err(StoreError::Malformed);
        }
        BlobId::from_bytes(&read_bytes(
            self.source,
            self.id,
            field.offset,
            field.length,
            32,
        )?)
    }
    fn references(&self, number: u32) -> Result<Vec<BlobId>, StoreError> {
        let mut result = Vec::new();
        for field in self.fields.iter().filter(|field| field.number == number) {
            if result.len() == REFERENCE_LIMIT {
                return Err(StoreError::Limit);
            }
            result.push(self.reference_field(*field)?);
        }
        Ok(result)
    }
}

fn varint(
    source: &dyn BlobSource,
    id: BlobId,
    cursor: &mut u64,
    end: u64,
) -> Result<u64, StoreError> {
    let mut result = 0;
    for shift in (0..70).step_by(7) {
        if *cursor >= end {
            return Err(StoreError::Malformed);
        }
        let mut byte = [0];
        source.read_at(id, *cursor, &mut byte)?;
        *cursor += 1;
        if shift == 63 && byte[0] > 1 {
            return Err(StoreError::Malformed);
        }
        result |= u64::from(byte[0] & 127) << shift;
        if byte[0] & 128 == 0 {
            return Ok(result);
        }
    }
    Err(StoreError::Malformed)
}
fn read_bytes(
    source: &dyn BlobSource,
    id: BlobId,
    offset: u64,
    length: u64,
    limit: usize,
) -> Result<Vec<u8>, StoreError> {
    let length = usize::try_from(length).map_err(|_| StoreError::Limit)?;
    if length > limit {
        return Err(StoreError::Limit);
    }
    let mut bytes = vec![0; length];
    source.read_at(id, offset, &mut bytes)?;
    Ok(bytes)
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
// Native tool IDs can contain an LF separating two IDs. They are opaque JSON values and
// never select files, sessions, commands or approval routes; path identities stay stricter.
fn valid_tool_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
}
fn decode_hex(value: &str) -> Result<Vec<u8>, StoreError> {
    if !value.len().is_multiple_of(2) {
        return Err(StoreError::Malformed);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err(StoreError::Malformed),
            };
            Ok((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect()
}

#[cfg(test)]
mod tests;
