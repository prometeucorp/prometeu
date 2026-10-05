//! Provider-only structured output. JSON-valued leaves use explicit encoded fields
//! because strict provider schemas cannot describe arbitrary operation inputs.
use serde_json::{json, Value};

fn object(properties: Value) -> Value {
    json!({"type":"object", "additionalProperties":false,
        "required":properties.as_object().unwrap().keys().collect::<Vec<_>>(),
        "properties":properties})
}
fn array(items: Value) -> Value {
    json!({"type":"array", "items":items})
}
fn nullable(value: Value) -> Value {
    json!({"anyOf":[value,{"type":"null"}]})
}

pub fn response() -> Value {
    let text = json!({"type":"string"});
    let integer = json!({"type":"integer"});
    let json_text = json!({"type":"string", "description":"A JSON-encoded value, not Markdown. Use the normal field name without Json in workflow examples as the value to encode."});
    let operation = |kind: &str| {
        object(json!({
            "type":{"type":"string","enum":[kind]}, "operation":text,
            "version":integer, "inputsJson":json_text
        }))
    };
    let config = json!({"anyOf":[
        object(json!({"type":{"type":"string","enum":["trigger"]},
            "event":{"type":"string","enum":["manual","github.authored_pr","linear.assigned_issue"]},
            "intervalSeconds":nullable(integer.clone()),
            "baseline":{"type":"string","enum":["ignoreExisting","includeExisting"]}})),
        operation("query"), operation("action"), operation("jev"),
        object(json!({"type":{"type":"string","enum":["condition"]}, "path":text,
            "operator":{"type":"string","enum":["equals","notEquals","exists","truthy"]},
            "valueJson":nullable(json_text.clone())})),
        object(json!({"type":{"type":"string","enum":["agent"]}, "prompt":text,
            "provider":nullable(json!({"type":"string","enum":["claude","codex"]})),
            "contextJson":json_text, "tools":array(json!({"type":"string","enum":["list_files","read_file","write_file","run_checks"]})),
            "checks":array(object(json!({"executable":text,"args":array(text.clone())})))})),
        object(json!({"type":{"type":"string","enum":["approval"]},"message":text})),
        object(json!({"type":{"type":"string","enum":["wait"]},"seconds":integer}))
    ]});
    let scope = object(json!({
        "projectId":nullable(text.clone()), "repository":nullable(text.clone()),
        "linearProjectId":nullable(text.clone()),
        "targets":array(object(json!({"projectId":text,"repository":nullable(text.clone())})))
    }));
    let workflow = object(json!({
        "id":text, "name":text, "revision":integer, "enabled":{"type":"boolean","enum":[false]},
        "scope":scope,
        "policy":object(json!({"maxConcurrentRuns":integer, "maxRetries":integer,
            "maxAgentTurns":integer, "maxCostUsd":nullable(json!({"type":"number"})),
            "allowWrites":{"type":"boolean","enum":[false]},
            "allowCommit":{"type":"boolean","enum":[false]},
            "allowPush":{"type":"boolean","enum":[false]},
            "requireLocalChecks":{"type":"boolean","enum":[true]},
            "requireMergeApproval":{"type":"boolean","enum":[true]}, "requirePublishApproval":{"type":"boolean","enum":[true]}})),
        "nodes":array(object(json!({"id":text,"label":text,"config":config,
            "position":object(json!({"x":{"type":"number"},"y":{"type":"number"}}))}))),
        "edges":array(object(json!({"from":text,"to":text,"port":text})))
    }));
    object(json!({"workflow":nullable(workflow),"summary":text}))
}

/// Accept old native documents as well; never reinterpret literal string inputs
/// in an existing graph as encoded JSON. Only the provider's *Json fields decode.
pub(super) fn decode(mut workflow: Value) -> Result<Value, String> {
    if let Some(nodes) = workflow.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let id = node["id"].as_str().unwrap_or("?").to_owned();
            if let Some(config) = node.get_mut("config").and_then(Value::as_object_mut) {
                for (encoded, field) in [
                    ("inputsJson", "inputs"),
                    ("contextJson", "context"),
                    ("outputSchemaJson", "outputSchema"),
                    ("valueJson", "value"),
                ] {
                    if let Some(value) = config.remove(encoded) {
                        if config.contains_key(field) {
                            return Err(format!(
                                "automation_proposal_schema: node {id}: duplicate {field}"
                            ));
                        }
                        let decoded = if value.is_null() && field == "value" {
                            Value::Null
                        } else {
                            let text = value.as_str().ok_or_else(|| format!("automation_proposal_schema: node {id}: {encoded} must be a JSON string"))?;
                            serde_json::from_str(text).map_err(|error| {
                                format!("automation_proposal_schema: node {id}: {encoded}: {error}")
                            })?
                        };
                        config.insert(field.into(), decoded);
                    }
                }
                if config.get("type").and_then(Value::as_str) == Some("agent") {
                    // The worker result is an app-owned contract. Asking the model
                    // to echo its JSON as an enum literal breaks strict output formats.
                    // Explicit legacy schemas still pass through native validation.
                    config
                        .entry("outputSchema")
                        .or_insert_with(super::super::worker::output_schema);
                }
            }
        }
    }
    Ok(workflow)
}
