//! Bounded model discovery over host-injected private subprocesses.
use prometeu_core::{
    agents::{CatalogError, Model},
    command::{CommandError, QueryLauncher, QueryPolicy, QueryProcess},
};
use serde_json::{json, Value};
use std::{collections::HashSet, process::Command, time::Duration};
const MAX_OUTPUT: usize = 1_048_576;

/// Provider correlation over an injected private query transport.
pub struct CatalogProcess(Box<dyn QueryProcess>);
impl CatalogProcess {
    pub fn spawn(
        launcher: &dyn QueryLauncher<Command>,
        command: &mut Command,
        timeout: Duration,
    ) -> Result<Self, CatalogError> {
        launcher
            .launch(
                command,
                QueryPolicy {
                    timeout,
                    max_output: MAX_OUTPUT,
                },
            )
            .map(Self)
            .map_err(query_error)
    }
    pub fn send(&mut self, value: Value) -> Result<(), CatalogError> {
        self.0
            .send(format!("{value}\n").as_bytes())
            .map_err(query_error)
    }
    pub fn read_line(&mut self) -> Result<Option<String>, CatalogError> {
        self.0.next().map_err(query_error)
    }
    pub fn response(&mut self, id: u64) -> Result<Value, CatalogError> {
        while let Some(line) = self.read_line()? {
            let value: Value =
                serde_json::from_str(&line).map_err(|_| CatalogError::new("invalid"))?;
            if value["id"].as_u64() != Some(id) {
                continue;
            }
            if value.get("error").is_some() {
                return Err(CatalogError::new("failed"));
            }
            return value
                .get("result")
                .filter(|v| v.is_object())
                .cloned()
                .ok_or_else(|| CatalogError::new("invalid"));
        }
        Err(CatalogError::new("failed"))
    }
}

fn query_error(error: CommandError) -> CatalogError {
    CatalogError::new(match error {
        CommandError::Unavailable => "unavailable",
        CommandError::Timeout => "timeout",
        CommandError::OutputLimit | CommandError::InvalidOutput => "invalid",
        CommandError::Io(_) => "failed",
    })
}
pub fn command_output(
    launcher: &dyn QueryLauncher<Command>,
    command: &mut Command,
    timeout: Duration,
) -> Result<String, CatalogError> {
    let mut process = CatalogProcess::spawn(launcher, command, timeout)?;
    process.0.close_input();
    let mut output = String::new();
    while let Some(line) = process.read_line()? {
        output.push_str(&line);
    }
    match process.0.finish().map_err(query_error)? {
        true => Ok(output),
        false => Err(CatalogError::new("failed")),
    }
}
pub fn query_claude(
    launcher: &dyn QueryLauncher<Command>,
    command: &mut Command,
    timeout: Duration,
) -> Result<Vec<Model>, CatalogError> {
    let mut process = CatalogProcess::spawn(launcher, command, timeout)?;
    process.send(
        json!({"type":"control_request","request_id":"models","request":{"subtype":"list_models"}}),
    )?;
    while let Some(line) = process.read_line()? {
        let value: Value = serde_json::from_str(&line).map_err(|_| CatalogError::new("invalid"))?;
        if value["type"] == "control_response" && value["response"]["request_id"] == "models" {
            return parse_claude(&value);
        }
    }
    Err(CatalogError::new("failed"))
}
fn text_field<'a>(value: &'a Value, key: &str) -> Result<&'a str, CatalogError> {
    value[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| CatalogError::new("invalid"))
}
fn strings(value: Option<&Value>) -> Result<Vec<String>, CatalogError> {
    match value {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| CatalogError::new("invalid"))
            })
            .collect(),
        _ => Err(CatalogError::new("invalid")),
    }
}
pub fn parse_claude(value: &Value) -> Result<Vec<Model>, CatalogError> {
    if value["response"]["subtype"] == "error" {
        return Err(CatalogError::new("failed"));
    }
    if value["response"]["subtype"] != "success" {
        return Err(CatalogError::new("invalid"));
    }
    let items = value["response"]["response"]["models"]
        .as_array()
        .ok_or_else(|| CatalogError::new("invalid"))?;
    items
        .iter()
        .filter(|item| item["value"] != "default" && item["disabled"] != true)
        .map(|item| {
            let id = text_field(item, "value")?.to_owned();
            Ok(Model {
                label: item
                    .get("displayName")
                    .map(|_| text_field(item, "displayName"))
                    .transpose()?
                    .unwrap_or(&id)
                    .to_owned(),
                id,
                efforts: strings(item.get("supportedEffortLevels"))?,
                additional: false,
            })
        })
        .collect()
}
pub fn parse_codex_page(value: &Value) -> Result<(Vec<Model>, Option<String>), CatalogError> {
    let items = value["data"]
        .as_array()
        .ok_or_else(|| CatalogError::new("invalid"))?;
    let models = items
        .iter()
        .map(|item| {
            let id = text_field(item, "model")?.to_owned();
            let label = text_field(item, "displayName")?.to_owned();
            let efforts = item["supportedReasoningEfforts"]
                .as_array()
                .ok_or_else(|| CatalogError::new("invalid"))?
                .iter()
                .map(|effort| text_field(effort, "reasoningEffort").map(str::to_owned))
                .collect::<Result<_, _>>()?;
            let additional = match item.get("hidden") {
                None => false,
                Some(Value::Bool(hidden)) => *hidden,
                _ => return Err(CatalogError::new("invalid")),
            };
            Ok(Model {
                id,
                label,
                efforts,
                additional,
            })
        })
        .collect::<Result<_, _>>()?;
    let next = match value.get("nextCursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 4096 => {
            Some(cursor.clone())
        }
        _ => return Err(CatalogError::new("invalid")),
    };
    Ok((models, next))
}
pub fn query_codex(
    launcher: &dyn QueryLauncher<Command>,
    command: &mut Command,
    timeout: Duration,
    version: &str,
) -> Result<Vec<Model>, CatalogError> {
    let mut process = CatalogProcess::spawn(launcher, command, timeout)?;
    process.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"prometeu","title":"Prometeu","version":version},"capabilities":{"experimentalApi":true}}}))?;
    process.response(1)?;
    process.send(json!({"method":"initialized","params":{}}))?;
    let mut id = 2;
    let mut cursor: Option<String> = None;
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    loop {
        process.send(
            json!({"id":id,"method":"model/list","params":{"includeHidden":true,"cursor":cursor}}),
        )?;
        let (page, next) = parse_codex_page(&process.response(id)?)?;
        models.extend(page);
        match next {
            None => return Ok(models),
            Some(next) if seen.insert(next.clone()) => cursor = Some(next),
            _ => return Err(CatalogError::new("invalid")),
        }
        id += 1;
    }
}
