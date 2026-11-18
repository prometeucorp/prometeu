//! The pinned CLI's functional protobuf descriptors stay at this edge. Only actual tool
//! arguments and results leave it; stored approvals are never answerable application requests.
use super::*;
use serde_json::{json, Map};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

#[derive(Clone, Debug, PartialEq)]
pub struct CursorTool {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub result: Option<CursorToolResult>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CursorToolResult {
    pub output: String,
    pub error: bool,
    pub background: bool,
}

#[derive(Clone)]
enum Type {
    Scalar(u8),
    Enum,
    Message(String),
}
struct Descriptor {
    number: u32,
    name: String,
    value: Type,
    repeated: bool,
    map: bool,
    group: Option<String>,
}
type Schema = BTreeMap<String, Vec<Descriptor>>;

fn schema() -> Result<&'static Schema, StoreError> {
    static SCHEMA: OnceLock<Result<Schema, StoreError>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            let document: Value = serde_json::from_str(include_str!("tool_schema.json"))
                .map_err(|_| StoreError::Malformed)?;
            if document["version"] != "2026.11.13-eacade1" {
                return Err(StoreError::Unsupported);
            }
            let mut schema = BTreeMap::new();
            for (name, message) in document["messages"]
                .as_object()
                .ok_or(StoreError::Malformed)?
            {
                let mut descriptors = Vec::new();
                for field in message["fields"].as_array().ok_or(StoreError::Malformed)? {
                    let parts = field
                        .as_str()
                        .ok_or(StoreError::Malformed)?
                        .split_whitespace()
                        .collect::<Vec<_>>();
                    if !(3..=4).contains(&parts.len()) {
                        return Err(StoreError::Malformed);
                    }
                    let raw_type = parts[2].trim_end_matches(['*', '?']);
                    let (map, raw_type) = if let Some(value) = raw_type.strip_prefix("9,") {
                        (true, value)
                    } else {
                        (false, raw_type)
                    };
                    let value = if let Some(index) = raw_type.strip_prefix('#') {
                        let index: usize = index.parse().map_err(|_| StoreError::Malformed)?;
                        match message["refs"][index]
                            .as_str()
                            .ok_or(StoreError::Malformed)?
                        {
                            "enum" => Type::Enum,
                            name => Type::Message(name.into()),
                        }
                    } else {
                        Type::Scalar(raw_type.parse().map_err(|_| StoreError::Malformed)?)
                    };
                    descriptors.push(Descriptor {
                        number: parts[0].parse().map_err(|_| StoreError::Malformed)?,
                        name: parts[1].into(),
                        value,
                        map,
                        repeated: parts[2].ends_with('*'),
                        group: parts.get(3).map(|v| (*v).into()),
                    });
                }
                schema.insert(name.clone(), descriptors);
            }
            Ok(schema)
        })
        .as_ref()
        .map_err(|error| *error)
}

#[derive(Default)]
struct Budget {
    text: usize,
    fields: usize,
}
impl Budget {
    fn text(&mut self, length: u64) -> Result<(), StoreError> {
        self.text = self
            .text
            .checked_add(length.try_into().map_err(|_| StoreError::Limit)?)
            .ok_or(StoreError::Limit)?;
        if self.text > TEXT_LIMIT {
            return Err(StoreError::Limit);
        }
        Ok(())
    }
    fn fields(&mut self, count: usize) -> Result<(), StoreError> {
        self.fields = self.fields.checked_add(count).ok_or(StoreError::Limit)?;
        if self.fields > FIELD_LIMIT {
            return Err(StoreError::Limit);
        }
        Ok(())
    }
}

pub fn tool(
    source: &dyn BlobSource,
    step: &CursorStep,
    schema: CursorStoreSchema,
) -> Result<CursorTool, StoreError> {
    decode_tool(
        source,
        step,
        schema,
        &mut BTreeSet::new(),
        &mut Budget::default(),
    )
}

fn decode_tool(
    source: &dyn BlobSource,
    value: &CursorStep,
    version: CursorStoreSchema,
    visited: &mut BTreeSet<BlobId>,
    budget: &mut Budget,
) -> Result<CursorTool, StoreError> {
    if visited.len() >= 8 || !visited.insert(value.version) {
        return Err(StoreError::Limit);
    }
    let CursorStepContent::Tool { id, kind, payload } = &value.content else {
        return Err(StoreError::Malformed);
    };
    let definitions = schema()?;
    let descriptor = definitions["agent.v1.ToolCall"]
        .iter()
        .find(|field| field.number == *kind)
        .ok_or(StoreError::Unsupported)?;
    let Type::Message(name) = &descriptor.value else {
        return Err(StoreError::Unsupported);
    };
    // The step decoder already verified this content-addressed blob in the same immutable copy.
    let wire = Wire::parse(source, payload.blob, payload.offset, payload.length, 0)?;
    if name == "agent.v1.TruncatedToolCall" {
        let original = wire.reference(1)?.ok_or(StoreError::Malformed)?;
        let mut original = decode_tool(
            source,
            &step(source, original, version)?,
            version,
            visited,
            budget,
        )?;
        if id.as_ref().is_some_and(|id| id != &original.id) {
            return Err(StoreError::Identity);
        }
        if let Some(id) = id {
            original.id.clone_from(id);
        }
        return Ok(original);
    }
    let decoded = message(&wire, name, budget)?;
    let mut input = decoded.get("args").cloned().unwrap_or_else(|| json!({}));
    let supplied_id = input["tool_call_id"].as_str().filter(|id| !id.is_empty());
    if supplied_id.zip(id.as_deref()).is_some_and(|(a, b)| a != b) {
        return Err(StoreError::Identity);
    }
    let id = id
        .as_deref()
        .or(supplied_id)
        .filter(|id| valid_tool_id(id))
        .ok_or(StoreError::Identity)?
        .to_owned();
    let kind = descriptor
        .name
        .strip_suffix("_tool_call")
        .ok_or(StoreError::Unsupported)?;
    let name = if kind == "mcp" {
        input["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .or_else(|| input["tool_name"].as_str())
            .unwrap_or("MCP")
            .to_owned()
    } else {
        kind.split('_')
            .map(|part| {
                let mut chars = part.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            })
            .collect()
    };
    let wrapper = definitions
        .get(match &descriptor.value {
            Type::Message(name) => name,
            _ => return Err(StoreError::Unsupported),
        })
        .ok_or(StoreError::Unsupported)?;
    let result_type = wrapper
        .iter()
        .find(|field| field.name == "result")
        .ok_or(StoreError::Unsupported)?;
    let Type::Message(result_type) = &result_type.value else {
        return Err(StoreError::Unsupported);
    };
    let result_descriptor = definitions
        .get(result_type)
        .ok_or(StoreError::Unsupported)?;
    let result = decoded
        .get("result")
        .map(|result| {
            result_value(
                result,
                kind,
                input["is_background"].as_bool().unwrap_or(false),
                result_descriptor,
            )
        })
        .transpose()?
        .flatten();
    if kind == "mcp" {
        input = input.get("args").cloned().unwrap_or_else(|| json!({}));
    } else if let Some(input) = input.as_object_mut() {
        for name in [
            "tool_call_id",
            "requested_sandbox_policy",
            "skip_approval",
            "smart_mode_approval",
            "smart_mode_approval_only",
            "classifier_result",
            "smart_mode_classifier_allowed",
            "hook_approval_requirement",
            "secret_scope_id",
            "admin_command_denylist",
            "conversation_id",
            "request_id",
            "refuse_saved_secrets",
        ] {
            input.remove(name);
        }
    }
    Ok(CursorTool {
        id,
        name,
        input,
        result,
    })
}

fn result_value(
    value: &Value,
    kind: &str,
    background: bool,
    descriptor: &[Descriptor],
) -> Result<Option<CursorToolResult>, StoreError> {
    let selected = descriptor
        .iter()
        .filter(|field| {
            field.group.as_deref() == Some("result") && value.get(&field.name).is_some()
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(None);
    }
    if selected.len() != 1 {
        return Err(StoreError::Malformed);
    }
    let selected = selected[0].name.as_str();
    let body = &value[selected];
    let error = matches!(
        selected,
        "error"
            | "failure"
            | "file_busy"
            | "file_not_found"
            | "needs_confirmation"
            | "not_file"
            | "not_found"
            | "permission_denied"
            | "read_permission_denied"
            | "rejected"
            | "spawn_error"
            | "timeout"
            | "write_permission_denied"
    ) || body["is_error"] == true;
    let output =
        if kind == "shell" && (body.get("stdout").is_some() || body.get("stderr").is_some()) {
            body["interleaved_output"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "{}{}",
                        body["stdout"].as_str().unwrap_or(""),
                        body["stderr"].as_str().unwrap_or("")
                    )
                })
        } else if kind == "read" && body["content"].is_string() {
            body["content"].as_str().unwrap_or("").to_owned()
        } else if kind == "mcp" {
            let text = body["content"]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item["text"]["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let structured = body
                .get("structured_content")
                .filter(|value| !value.is_null());
            match (text.is_empty(), structured) {
                (false, None) => text,
                (false, Some(value)) => format!("{text}\n{value}"),
                (true, _) => body.to_string(),
            }
        } else {
            body.to_string()
        };
    Ok(Some(CursorToolResult {
        output,
        error,
        background: background
            || value["is_background"] == true
            || matches!(selected, "async" | "still_running"),
    }))
}

fn message(wire: &Wire<'_>, name: &str, budget: &mut Budget) -> Result<Value, StoreError> {
    budget.fields(wire.fields.len())?;
    if name == "google.protobuf.Value" {
        return google_value(wire, budget);
    }
    if name == "google.protobuf.Struct" {
        if wire.fields.iter().any(|field| field.number != 1) {
            return Err(StoreError::Unsupported);
        }
        return map(
            wire,
            &Type::Message("google.protobuf.Value".into()),
            1,
            budget,
        )
        .map(Value::Object);
    }
    let fields = schema()?.get(name).ok_or(StoreError::Unsupported)?;
    let mut output = Map::new();
    let mut groups = BTreeSet::new();
    for field in &wire.fields {
        let descriptor = fields
            .iter()
            .find(|item| item.number == field.number)
            .ok_or(StoreError::Unsupported)?;
        if descriptor.map || descriptor.repeated {
            continue;
        }
        if output.contains_key(&descriptor.name)
            || descriptor
                .group
                .as_ref()
                .is_some_and(|group| !groups.insert(group))
        {
            return Err(StoreError::Malformed);
        }
        let (key, value) = if name == "agent.v1.ReadToolSuccess"
            && matches!(descriptor.name.as_str(), "content_blob_id" | "data_blob_id")
        {
            let id = wire.reference_field(*field)?;
            let length = verify(wire.source, id)?;
            if descriptor.name == "content_blob_id" {
                budget.text(length)?;
                (
                    "content",
                    Value::String(
                        String::from_utf8(read_bytes(wire.source, id, 0, length, TEXT_LIMIT)?)
                            .map_err(|_| StoreError::Malformed)?,
                    ),
                )
            } else {
                ("data", json!({"bytes":length,"omitted":true}))
            }
        } else {
            (
                descriptor.name.as_str(),
                scalar(wire, *field, &descriptor.value, budget)?,
            )
        };
        output.insert(key.into(), value);
    }
    for descriptor in fields.iter().filter(|field| field.map || field.repeated) {
        if !wire
            .fields
            .iter()
            .any(|field| field.number == descriptor.number)
        {
            continue;
        }
        let value = if descriptor.map {
            Value::Object(map(wire, &descriptor.value, descriptor.number, budget)?)
        } else {
            let mut values = Vec::new();
            for field in wire
                .fields
                .iter()
                .filter(|field| field.number == descriptor.number)
            {
                if field.wire == 2
                    && matches!(descriptor.value,Type::Scalar(1..=8|13..=18)|Type::Enum)
                {
                    packed(wire, *field, &descriptor.value, budget, &mut values)?;
                } else {
                    values.push(scalar(wire, *field, &descriptor.value, budget)?);
                }
            }
            Value::Array(values)
        };
        output.insert(descriptor.name.clone(), value);
    }
    Ok(Value::Object(output))
}

fn map(
    wire: &Wire<'_>,
    value_type: &Type,
    number: u32,
    budget: &mut Budget,
) -> Result<Map<String, Value>, StoreError> {
    let mut output = Map::new();
    for field in &wire.fields {
        if field.number != number {
            continue;
        }
        let entry = wire.nested(*field)?;
        budget.fields(entry.fields.len())?;
        if entry
            .fields
            .iter()
            .any(|field| ![1, 2].contains(&field.number))
        {
            return Err(StoreError::Unsupported);
        }
        let key = entry
            .field(1)?
            .map(|field| scalar(&entry, field, &Type::Scalar(9), budget))
            .transpose()?
            .unwrap_or_else(|| json!(""));
        let key = key.as_str().ok_or(StoreError::Malformed)?.to_owned();
        let value = entry
            .field(2)?
            .map(|field| scalar(&entry, field, value_type, budget))
            .transpose()?
            .unwrap_or_else(|| match value_type {
                Type::Scalar(9) => json!(""),
                Type::Scalar(8) => json!(false),
                Type::Scalar(_) | Type::Enum => json!(0),
                Type::Message(_) => Value::Null,
            });
        if output.insert(key, value).is_some() {
            return Err(StoreError::Malformed);
        }
    }
    Ok(output)
}

fn google_value(wire: &Wire<'_>, budget: &mut Budget) -> Result<Value, StoreError> {
    if wire.fields.is_empty() {
        return Ok(Value::Null);
    }
    if wire.fields.len() != 1 {
        return Err(StoreError::Malformed);
    }
    let field = wire.fields[0];
    match field.number {
        1 if field.wire == 0 && field.integer == 0 => Ok(Value::Null),
        2 => scalar(wire, field, &Type::Scalar(1), budget),
        3 => scalar(wire, field, &Type::Scalar(9), budget),
        4 => scalar(wire, field, &Type::Scalar(8), budget),
        5 => message(&wire.nested(field)?, "google.protobuf.Struct", budget),
        6 => {
            let list = wire.nested(field)?;
            budget.fields(list.fields.len())?;
            let mut values = Vec::new();
            for item in &list.fields {
                if item.number != 1 {
                    return Err(StoreError::Unsupported);
                }
                values.push(message(
                    &list.nested(*item)?,
                    "google.protobuf.Value",
                    budget,
                )?);
            }
            Ok(Value::Array(values))
        }
        _ => Err(StoreError::Unsupported),
    }
}

fn scalar(
    wire: &Wire<'_>,
    field: Field,
    value_type: &Type,
    budget: &mut Budget,
) -> Result<Value, StoreError> {
    let code = match value_type {
        Type::Message(name) => return message(&wire.nested(field)?, name, budget),
        Type::Enum => 5,
        Type::Scalar(code) => *code,
    };
    let expected = match code {
        1 | 6 | 16 => 1,
        2 | 7 | 15 => 5,
        3..=5 | 8 | 13 | 17 | 18 => 0,
        9 | 12 => 2,
        _ => return Err(StoreError::Unsupported),
    };
    if field.wire != expected {
        return Err(StoreError::Malformed);
    }
    Ok(match code {
        1 | 2 => {
            let bytes = read_bytes(wire.source, wire.id, field.offset, field.length, 8)?;
            let value = if code == 1 {
                f64::from_le_bytes(bytes.try_into().map_err(|_| StoreError::Malformed)?)
            } else {
                f32::from_le_bytes(bytes.try_into().map_err(|_| StoreError::Malformed)?) as f64
            };
            Value::Number(serde_json::Number::from_f64(value).ok_or(StoreError::Malformed)?)
        }
        3 => json!(field.integer as i64),
        4 => json!(field.integer),
        5 => json!(field.integer as i32),
        8 => match field.integer {
            0 => json!(false),
            1 => json!(true),
            _ => return Err(StoreError::Malformed),
        },
        9 => {
            budget.text(field.length)?;
            Value::String(
                String::from_utf8(read_bytes(
                    wire.source,
                    wire.id,
                    field.offset,
                    field.length,
                    TEXT_LIMIT,
                )?)
                .map_err(|_| StoreError::Malformed)?,
            )
        }
        12 => json!({"bytes":field.length,"omitted":true}),
        13 => json!(u32::try_from(field.integer).map_err(|_| StoreError::Malformed)?),
        17 => {
            let n = u32::try_from(field.integer).map_err(|_| StoreError::Malformed)?;
            json!(((n >> 1) as i32) ^ (-((n & 1) as i32)))
        }
        _ => return Err(StoreError::Unsupported),
    })
}

fn packed(
    wire: &Wire<'_>,
    field: Field,
    value_type: &Type,
    budget: &mut Budget,
    values: &mut Vec<Value>,
) -> Result<(), StoreError> {
    let mut cursor = field.offset;
    let end = cursor
        .checked_add(field.length)
        .ok_or(StoreError::Malformed)?;
    while cursor < end {
        budget.fields(1)?;
        let length = match value_type {
            Type::Scalar(1 | 6 | 16) => 8,
            Type::Scalar(2 | 7 | 15) => 4,
            _ => 0,
        };
        let integer = if length == 0 {
            varint(wire.source, wire.id, &mut cursor, end)?
        } else {
            0
        };
        let next = Field {
            number: field.number,
            wire: if length == 8 {
                1
            } else if length == 4 {
                5
            } else {
                0
            },
            offset: cursor,
            length,
            integer,
        };
        cursor = cursor
            .checked_add(length)
            .filter(|cursor| *cursor <= end)
            .ok_or(StoreError::Malformed)?;
        values.push(scalar(wire, next, value_type, budget)?);
    }
    Ok(())
}
