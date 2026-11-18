//! Pure bounded decoding of the pinned Antigravity resumable SQLite step payloads.
//! Native step indices and explicit tool-call IDs are identities; text and adjacency are not.
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{cell::Cell, rc::Rc};

pub mod projection;

const TEXT_LIMIT: usize = 1024 * 1024;
const FIELD_LIMIT: usize = 65_536;
const DEPTH_LIMIT: usize = 16;
const CHUNK: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schema {
    Cli1510,
}
impl Schema {
    pub fn for_version(version: &str) -> Option<Self> {
        (version == "1.5.10").then_some(Self::Cli1510)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Read,
    Malformed,
    Limit,
    Identity,
    Unsupported,
}

/// Reads one immutable copied SQLite blob. Binary fields are skipped without allocating them.
pub trait Source {
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Error>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
impl Source for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<(), Error> {
        let start = usize::try_from(offset).map_err(|_| Error::Read)?;
        bytes.copy_from_slice(
            self.get(start..start.checked_add(bytes.len()).ok_or(Error::Read)?)
                .ok_or(Error::Read)?,
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Unknown,
    Generating,
    Queued,
    Pending,
    Running,
    Waiting,
    Done,
    Invalid,
    Cleared,
    Canceled,
    Error,
    Interrupted,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Invalid | Self::Error | Self::Canceled | Self::Interrupted
        )
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    User {
        text: String,
        client_id: Option<String>,
    },
    Assistant {
        text: String,
        thinking: String,
        calls: Vec<ToolCall>,
        stop_reason: Option<u64>,
    },
    Tool {
        call: ToolCall,
        result: Option<Value>,
        full_output_uri: Option<String>,
        error: bool,
    },
    Checkpoint {
        summary: String,
        failed: bool,
    },
    /// Native resume bookkeeping explicitly hidden by both its render and agent-message flags.
    /// Its source hash is receipted, but its internal text is not public conversation content.
    HiddenSystemMessage,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub index: u64,
    pub version: String,
    pub status: Status,
    pub source: u64,
    pub content: Content,
    pub complete: bool,
    pub usage: Option<prometeu_core::conversation::usage::Usage>,
    pub model: Option<String>,
}

/// The copied SQLite header must agree with the protobuf header. It is not a turn outcome.
pub fn step(
    source: &dyn Source,
    index: u64,
    expected_type: u64,
    expected_status: u64,
    _: Schema,
) -> Result<Step, Error> {
    let wire = Wire::new(source, 0, source.len(), 0, Rc::new(Budget::default()))?;
    let kind = wire.integer(1)?.unwrap_or(0);
    let raw_status = wire.integer(4)?.unwrap_or(0);
    if kind != expected_type || raw_status != expected_status {
        return Err(Error::Identity);
    }
    let status = match raw_status {
        0 => Status::Unknown,
        1 => Status::Pending,
        2 => Status::Running,
        3 => Status::Done,
        4 => Status::Invalid,
        5 => Status::Cleared,
        6 => Status::Canceled,
        7 => Status::Error,
        8 => Status::Generating,
        9 => Status::Waiting,
        11 => Status::Queued,
        12 => Status::Interrupted,
        _ => return Err(Error::Unsupported),
    };
    let metadata = wire.message(5)?.ok_or(Error::Malformed)?;
    let origin = metadata.integer(3)?.unwrap_or(0);
    let expected_payload = match kind {
        14 => 19,
        15 => 20,
        23 => 30,
        101 => 114,
        132 => 140,
        _ => return Err(Error::Unsupported),
    };
    if wire.fields.iter().any(|field| {
        ![1, 4, 5, 31, 133, 148, 158, 56, 147, 155, expected_payload].contains(&field.number)
    }) {
        return Err(Error::Unsupported);
    }
    let mut complete = true;
    let content = match kind {
        14 => {
            if origin != 4 {
                return Err(Error::Unsupported);
            }
            let user = wire.message(19)?.ok_or(Error::Malformed)?;
            let text = user.text(2)?.or(user.text(1)?).ok_or(Error::Malformed)?;
            Content::User {
                text,
                client_id: user.id(19)?,
            }
        }
        15 => {
            if origin != 2 {
                return Err(Error::Identity);
            }
            let answer = wire.message(20)?.ok_or(Error::Malformed)?;
            let mut calls = Vec::new();
            for field in answer.repeated(7) {
                calls.push(tool_call(&answer.nested(field)?)?);
            }
            Content::Assistant {
                text: answer.text(8)?.or(answer.text(1)?).unwrap_or_default(),
                thinking: answer.text(3)?.unwrap_or_default(),
                calls,
                stop_reason: answer.integer(12)?,
            }
        }
        132 => {
            if origin != 2 {
                return Err(Error::Identity);
            }
            let original = tool_call(&metadata.message(4)?.ok_or(Error::Identity)?)?;
            let call = if let Some(modified) = metadata.message(29)? {
                let modified = tool_call(&modified)?;
                if modified.id != original.id || modified.name != original.name {
                    return Err(Error::Identity);
                }
                modified
            } else {
                original
            };
            let generic = wire.message(140)?.ok_or(Error::Malformed)?;
            let result = generic.message(2)?;
            let mut full_output_uri = None;
            let native_error = wire
                .message(31)?
                .map(|details| {
                    let user = details.text(1)?.filter(|text| !text.trim().is_empty());
                    let short = details.text(2)?.filter(|text| !text.trim().is_empty());
                    Ok::<_, Error>(user.or(short))
                })
                .transpose()?
                .flatten();
            let mut error = native_error.is_some()
                || matches!(
                    status,
                    Status::Invalid | Status::Error | Status::Canceled | Status::Interrupted
                );
            let mut result = match result {
                None => None,
                Some(result) => {
                    full_output_uri = result.text(7)?.filter(|text| !text.is_empty());
                    // External output references need their own authorized reader. The decoder
                    // retains the reference but cannot certify that a possibly truncated preview
                    // contains the complete tool result.
                    complete &= full_output_uri.is_none();
                    if result.field(11)?.is_some()
                        || result.field(8)?.is_some()
                        || result.field(9)?.is_some()
                    {
                        return Err(Error::Unsupported);
                    }
                    let output = if let Some(structure) = result.message(10)? {
                        Some(struct_value(&structure)?)
                    } else {
                        result.text(1)?.map(Value::String)
                    };
                    if let Some(payload) = result.message(6)? {
                        let type_url = payload
                            .text(1)?
                            .filter(|name| {
                                !name.is_empty()
                                    && name.len() <= 1024
                                    && !name.chars().any(char::is_control)
                            })
                            .ok_or(Error::Malformed)?;
                        // The native generic renderer reads result text and Step.error, not
                        // auxiliary Any metadata. Its wire framing is still bounded/validated.
                        let body = payload.message(2)?;
                        if type_url == "type.googleapis.com/exa.cortex.core.tools.RunCommand" {
                            if call.name != "run_command" {
                                return Err(Error::Identity);
                            }
                            let command = body.ok_or(Error::Malformed)?;
                            error |= command.integer(1)?.unwrap_or(0) as u32 as i32 != 0
                                || command.integer(2)?.unwrap_or(0) != 0;
                            if command.integer(8)?.unwrap_or(0) != 0
                                || command.repeated(9).next().is_some()
                            {
                                return Err(Error::Unsupported);
                            }
                        } else if call.name == "run_command" {
                            return Err(Error::Identity);
                        } else if output.is_none()
                            && full_output_uri.is_none()
                            && native_error.is_none()
                        {
                            return Err(Error::Unsupported);
                        }
                    }
                    output
                }
            };
            if result
                .as_ref()
                .is_none_or(|value| value.as_str() == Some(""))
            {
                if let Some(message) = native_error {
                    result = Some(Value::String(message));
                }
            }
            complete &= result.is_some() || !status.terminal();
            Content::Tool {
                call,
                result,
                full_output_uri,
                error,
            }
        }
        23 => {
            if origin != 5 {
                return Err(Error::Identity);
            }
            let checkpoint = wire.message(30)?.ok_or(Error::Malformed)?;
            let mut parts = Vec::new();
            for field in [4, 5, 6, 8] {
                if let Some(text) = checkpoint.text(field)?.filter(|text| !text.is_empty()) {
                    parts.push(text);
                }
            }
            let summary = parts.join("\n\n");
            if summary.is_empty() && checkpoint.id(29)?.is_some() {
                return Err(Error::Unsupported);
            }
            let native_error = if let Some(error) = wire.message(31)? {
                error.text(1)?.is_some_and(|text| !text.is_empty())
                    || error.text(2)?.is_some_and(|text| !text.is_empty())
            } else {
                false
            };
            Content::Checkpoint {
                summary,
                failed: (checkpoint.integer(16)?.unwrap_or(0) != 0
                    && checkpoint.integer(17)?.unwrap_or(0) == 0)
                    || native_error,
            }
        }
        101 => {
            if origin != 5 {
                return Err(Error::Identity);
            }
            hidden_system_message(&wire.message(114)?.ok_or(Error::Malformed)?)?;
            Content::HiddenSystemMessage
        }
        _ => return Err(Error::Unsupported),
    };
    let usage = metadata
        .message(9)?
        .map(|stats| {
            let read = stats.integer(5)?.unwrap_or(0);
            let input = stats
                .integer(2)?
                .unwrap_or(0)
                .checked_add(read)
                .ok_or(Error::Limit)?;
            let output = stats.integer(3)?.unwrap_or(0);
            let usage = prometeu_core::conversation::usage::Usage {
                input_tokens: Some(input),
                output_tokens: Some(output),
                cache_read_tokens: Some(read),
                cache_write_tokens: Some(stats.integer(4)?.unwrap_or(0)),
                reasoning_tokens: Some(stats.integer(9)?.unwrap_or(0))
                    .filter(|tokens| *tokens <= output),
                ..Default::default()
            };
            if !usage.valid() {
                return Err(Error::Malformed);
            }
            Ok(usage)
        })
        .transpose()?;
    let model = metadata
        .message(24)?
        .map(|info| info.text(8))
        .transpose()?
        .flatten()
        .filter(|name| !name.is_empty());
    let mut digest = Sha256::new();
    let mut offset = 0;
    let mut bytes = vec![0; CHUNK];
    while offset < source.len() {
        let count = (source.len() - offset).min(CHUNK as u64) as usize;
        source.read_at(offset, &mut bytes[..count])?;
        digest.update(&bytes[..count]);
        offset += count as u64;
    }
    let version = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(Step {
        index,
        version,
        status,
        source: origin,
        content,
        complete,
        usage,
        model,
    })
}

/// The verified resume notice has plain text plus two explicit native visibility flags. Other
/// SYSTEM_MESSAGE forms (visible messages, media, nested steps) need their own content projection.
fn hidden_system_message(message: &Wire<'_>) -> Result<(), Error> {
    if message
        .fields
        .iter()
        .any(|field| ![1, 2, 3, 4, 5].contains(&field.number))
    {
        return Err(Error::Unsupported);
    }
    if message.text(1)?.is_none_or(|text| text.is_empty()) {
        return Err(Error::Malformed);
    }
    message.text(3)?;
    let render = message.message(2)?.ok_or(Error::Unsupported)?;
    if render.integer(4)? != Some(1)
        || render
            .fields
            .iter()
            .any(|field| ![1, 2, 4, 5, 6, 7, 10, 11].contains(&field.number))
    {
        return Err(Error::Unsupported);
    }
    for field in [1, 2, 5, 6, 7] {
        render.text(field)?;
    }
    render.integer(11)?;
    for block in render.repeated(10) {
        let block = render.nested(block)?;
        if block.fields.iter().any(|field| field.number != 1) {
            return Err(Error::Unsupported);
        }
        block.text(1)?.ok_or(Error::Malformed)?;
    }
    let agent = message.message(4)?.ok_or(Error::Unsupported)?;
    if agent.integer(11)? != Some(1)
        || agent
            .fields
            .iter()
            .any(|field| ![1, 2, 3, 4, 5, 6, 11].contains(&field.number))
    {
        return Err(Error::Unsupported);
    }
    agent.id(1)?.ok_or(Error::Identity)?;
    agent.id(2)?;
    agent.id(3)?;
    if agent.text(4)?.is_none_or(|text| text.is_empty()) {
        return Err(Error::Malformed);
    }
    agent.integer(5)?;
    if let Some(timestamp) = agent.message(6)? {
        if timestamp
            .fields
            .iter()
            .any(|field| ![1, 2].contains(&field.number))
        {
            return Err(Error::Unsupported);
        }
        timestamp.integer(1)?;
        timestamp.integer(2)?;
    }
    for entry in message.repeated(5) {
        let entry = message.nested(entry)?;
        if entry
            .fields
            .iter()
            .any(|field| ![1, 2].contains(&field.number))
        {
            return Err(Error::Unsupported);
        }
        entry.text(1)?;
        entry.text(2)?;
    }
    Ok(())
}

fn tool_call(wire: &Wire<'_>) -> Result<ToolCall, Error> {
    let id = wire.id(1)?.ok_or(Error::Identity)?;
    let name = wire.id(2)?.ok_or(Error::Identity)?;
    let input = wire.text(3)?.ok_or(Error::Malformed)?;
    let input: Value = serde_json::from_str(&input).map_err(|_| Error::Malformed)?;
    if !input.is_object() {
        return Err(Error::Malformed);
    }
    Ok(ToolCall { id, name, input })
}
fn struct_value(wire: &Wire<'_>) -> Result<Value, Error> {
    let mut values = Map::new();
    for field in wire.repeated(1) {
        let entry = wire.nested(field)?;
        let key = entry.text(1)?.ok_or(Error::Malformed)?;
        let value = value(&entry.message(2)?.ok_or(Error::Malformed)?)?;
        if values.insert(key, value).is_some() {
            return Err(Error::Malformed);
        }
    }
    Ok(Value::Object(values))
}
fn value(wire: &Wire<'_>) -> Result<Value, Error> {
    let fields = wire
        .fields
        .iter()
        .filter(|field| (1..=6).contains(&field.number))
        .collect::<Vec<_>>();
    if fields.len() != 1 {
        return Err(Error::Malformed);
    }
    let field = fields[0];
    match field.number {
        1 if field.kind == 0 && wire.integer(1)? == Some(0) => Ok(Value::Null),
        2 if field.kind == 1 => {
            let mut bytes = [0; 8];
            wire.source.read_at(field.offset, &mut bytes)?;
            let value = f64::from_le_bytes(bytes);
            if !value.is_finite() {
                return Err(Error::Malformed);
            }
            Ok(json!(value))
        }
        3 => Ok(json!(wire.text(3)?.ok_or(Error::Malformed)?)),
        4 => match wire.integer(4)? {
            Some(0) => Ok(json!(false)),
            Some(1) => Ok(json!(true)),
            _ => Err(Error::Malformed),
        },
        5 => struct_value(&wire.nested(field)?),
        6 => {
            let list = wire.nested(field)?;
            list.repeated(1)
                .map(|field| value(&list.nested(field)?))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => Err(Error::Malformed),
    }
}

#[derive(Clone, Copy)]
struct Field {
    number: u32,
    kind: u8,
    offset: u64,
    length: u64,
}
#[derive(Default)]
struct Budget {
    fields: Cell<usize>,
    text: Cell<usize>,
}
struct Wire<'a> {
    source: &'a dyn Source,
    fields: Vec<Field>,
    depth: usize,
    budget: Rc<Budget>,
}
impl<'a> Wire<'a> {
    fn new(
        source: &'a dyn Source,
        start: u64,
        length: u64,
        depth: usize,
        budget: Rc<Budget>,
    ) -> Result<Self, Error> {
        if depth > DEPTH_LIMIT {
            return Err(Error::Limit);
        }
        let end = start
            .checked_add(length)
            .filter(|end| *end <= source.len())
            .ok_or(Error::Malformed)?;
        let mut fields = Vec::new();
        let mut at = start;
        while at < end {
            if budget.fields.get() >= FIELD_LIMIT {
                return Err(Error::Limit);
            }
            budget.fields.set(budget.fields.get() + 1);
            let tag = varint(source, &mut at, end)?;
            let number = u32::try_from(tag >> 3).map_err(|_| Error::Malformed)?;
            if number == 0 || number > 0x1fff_ffff {
                return Err(Error::Malformed);
            }
            let kind = (tag & 7) as u8;
            let (offset, length) = match kind {
                0 => {
                    let offset = at;
                    varint(source, &mut at, end)?;
                    (offset, at - offset)
                }
                1 | 5 => {
                    let length = if kind == 1 { 8 } else { 4 };
                    let offset = at;
                    at = at.checked_add(length).ok_or(Error::Malformed)?;
                    (offset, length)
                }
                2 => {
                    let length = varint(source, &mut at, end)?;
                    let offset = at;
                    at = at.checked_add(length).ok_or(Error::Malformed)?;
                    (offset, length)
                }
                _ => return Err(Error::Malformed),
            };
            if at > end {
                return Err(Error::Malformed);
            }
            fields.push(Field {
                number,
                kind,
                offset,
                length,
            });
        }
        Ok(Self {
            source,
            fields,
            depth,
            budget,
        })
    }
    fn repeated(&self, number: u32) -> impl Iterator<Item = &Field> {
        self.fields
            .iter()
            .filter(move |field| field.number == number)
    }
    fn field(&self, number: u32) -> Result<Option<&Field>, Error> {
        let mut fields = self.repeated(number);
        let first = fields.next();
        if fields.next().is_some() {
            return Err(Error::Malformed);
        }
        Ok(first)
    }
    fn nested(&self, field: &Field) -> Result<Self, Error> {
        if field.kind != 2 {
            return Err(Error::Malformed);
        }
        Self::new(
            self.source,
            field.offset,
            field.length,
            self.depth + 1,
            self.budget.clone(),
        )
    }
    fn message(&self, number: u32) -> Result<Option<Self>, Error> {
        self.field(number)?
            .map(|field| self.nested(field))
            .transpose()
    }
    fn integer(&self, number: u32) -> Result<Option<u64>, Error> {
        self.field(number)?
            .map(|field| {
                if field.kind != 0 {
                    return Err(Error::Malformed);
                }
                let mut offset = field.offset;
                varint(self.source, &mut offset, field.offset + field.length)
            })
            .transpose()
    }
    fn text(&self, number: u32) -> Result<Option<String>, Error> {
        self.field(number)?
            .map(|field| {
                if field.kind != 2 {
                    return Err(Error::Malformed);
                }
                if field.length > TEXT_LIMIT as u64
                    || field.length > (4 * TEXT_LIMIT).saturating_sub(self.budget.text.get()) as u64
                {
                    return Err(Error::Limit);
                }
                self.budget
                    .text
                    .set(self.budget.text.get() + field.length as usize);
                let mut bytes = vec![0; field.length as usize];
                self.source.read_at(field.offset, &mut bytes)?;
                String::from_utf8(bytes).map_err(|_| Error::Malformed)
            })
            .transpose()
    }
    fn id(&self, number: u32) -> Result<Option<String>, Error> {
        let value = self.text(number)?;
        if value.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        }) {
            return Err(Error::Identity);
        }
        Ok(value)
    }
}
fn varint(source: &dyn Source, at: &mut u64, end: u64) -> Result<u64, Error> {
    let mut value = 0;
    for index in 0..10 {
        if *at >= end {
            return Err(Error::Malformed);
        }
        let mut byte = [0];
        source.read_at(*at, &mut byte)?;
        *at += 1;
        if index == 9 && byte[0] > 1 {
            return Err(Error::Malformed);
        }
        value |= u64::from(byte[0] & 127) << (index * 7);
        if byte[0] & 128 == 0 {
            return Ok(value);
        }
    }
    Err(Error::Malformed)
}

#[cfg(test)]
mod tests;
