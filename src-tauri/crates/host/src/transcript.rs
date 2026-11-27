//! Compact messages for agent clients, from Claude's native transcript and completed Codex RPC
//! items: what was said and which tools ran, without the native envelope.
use serde_json::{json, Value};

/// Tool inputs and results are cut to this many characters; the full text stays in the file.
const TOOL_TEXT: usize = 2_000;
/// Message text is cut to this many characters.
const MESSAGE_TEXT: usize = 20_000;

fn cut(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

fn block_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The messages one transcript line contributes. Codex records contain the compact projection
/// made by this host. Claude meta and subagent records contribute none.
pub fn messages(line: &Value) -> Vec<Value> {
    let kind = line.get("type").and_then(Value::as_str).unwrap_or_default();
    if kind == "codex" {
        return line["messages"].as_array().cloned().unwrap_or_default();
    }
    if !matches!(kind, "user" | "assistant")
        || line.get("isMeta").and_then(Value::as_bool) == Some(true)
        || line.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return Vec::new();
    }
    let at = line.get("timestamp").cloned().unwrap_or(Value::Null);
    let Some(content) = line.pointer("/message/content") else {
        return Vec::new();
    };
    let blocks = match content {
        Value::String(text) => vec![json!({"type": "text", "text": text})],
        Value::Array(blocks) => blocks.clone(),
        _ => Vec::new(),
    };
    blocks
        .iter()
        .filter_map(|block| {
            let message = match block.get("type").and_then(Value::as_str)? {
                "text" => {
                    let text = block.get("text").and_then(Value::as_str)?.trim();
                    if text.is_empty() {
                        return None;
                    }
                    json!({"role": kind, "kind": "text", "text": cut(text, MESSAGE_TEXT)})
                }
                "tool_use" => json!({
                    "role": kind, "kind": "tool_use",
                    "tool": block.get("name").cloned().unwrap_or(Value::Null),
                    "text": cut(&block.get("input").map(Value::to_string).unwrap_or_default(), TOOL_TEXT),
                }),
                "tool_result" => json!({
                    "role": kind, "kind": "tool_result",
                    "error": block.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    "text": cut(&block.get("content").map(block_text).unwrap_or_default(), TOOL_TEXT),
                }),
                _ => return None,
            };
            let mut message = message;
            message["at"] = at.clone();
            Some(message)
        })
        .collect()
}

/// Project one completed native Codex item through the shared protocol adapter. The caller owns
/// thread filtering and deduplication. Image data and reasoning are not part of the compact log.
pub fn codex_item(item: &Value, cwd: &str) -> Vec<Value> {
    let at = item.get("completedAtMs").cloned().unwrap_or(Value::Null);
    if item["type"] == "userMessage" {
        let text = item["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str(),
                Some("image" | "localImage") => Some("[image]"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return if text.trim().is_empty() {
            Vec::new()
        } else {
            vec![
                json!({"role": "user", "kind": "text", "text": cut(text.trim(), MESSAGE_TEXT), "at": at}),
            ]
        };
    }
    let thread = "host-transcript";
    let mut link = prometeu_protocols::codex::Link::attached(
        Box::new(std::io::sink()),
        prometeu_protocols::codex::Start {
            permission: None,
            instructions: String::new(),
            cwd: cwd.into(),
            resume: Some(thread.into()),
            model: String::new(),
            effort: String::new(),
            plugin_ids: Vec::new(),
            plugin_hook_ids: Vec::new(),
        },
        thread.into(),
        "host-transcript".into(),
        |_, english| english.into(),
    );
    ["item/started", "item/completed"]
        .into_iter()
        .flat_map(|method| {
            link.on_line(
                &json!({"method": method, "params": {"threadId": thread, "item": item}})
                    .to_string(),
            )
        })
        .filter_map(|event| {
            let block = &event["block"];
            let mut message = match event["type"].as_str()? {
                "assistant.block" if block["kind"] == "text" => {
                    let text = block["text"].as_str()?.trim();
                    if text.is_empty() {
                        return None;
                    }
                    json!({"role": "assistant", "kind": "text", "text": cut(text, MESSAGE_TEXT)})
                }
                "assistant.block" if block["kind"] == "tool" => json!({
                    "role": "assistant", "kind": "tool_use", "tool": block["name"],
                    "text": cut(&block["input"].to_string(), TOOL_TEXT),
                }),
                "tool.completed" => json!({
                    "role": "user", "kind": "tool_result", "error": event["error"],
                    "text": cut(event["output"].as_str().unwrap_or_default(), TOOL_TEXT),
                }),
                _ => return None,
            };
            message["at"] = at.clone();
            Some(message)
        })
        .collect()
}

/// Parse the complete lines of `bytes`, which start at a line boundary. Returns the messages and
/// how many bytes were consumed; a trailing partial line is left for the next read.
pub fn read(bytes: &[u8]) -> (Vec<Value>, usize) {
    let mut out = Vec::new();
    let mut consumed = 0;
    while let Some(end) = bytes[consumed..].iter().position(|b| *b == b'\n') {
        let line = &bytes[consumed..consumed + end];
        consumed += end + 1;
        if let Ok(value) = serde_json::from_slice::<Value>(line) {
            out.extend(messages(&value));
        }
    }
    (out, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turns_become_text_and_tool_messages() {
        let transcript = concat!(
            r#"{"type":"user","message":{"role":"user","content":"fix the test"},"timestamp":"t1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"x"},{"type":"text","text":"On it."},{"type":"tool_use","name":"Bash","input":{"command":"npm test"}}]},"timestamp":"t2"}"#,
            "\n",
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"a","content":[{"type":"text","text":"1 passed"}]}]},"timestamp":"t3"}"#,
            "\n",
            r#"{"type":"user","isMeta":true,"message":{"content":"caveat"}}"#,
            "\n",
            r#"{"type":"summary","summary":"s"}"#,
            "\n",
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"partial"#,
        );
        let (messages, consumed) = read(transcript.as_bytes());
        assert_eq!(consumed, transcript.rfind('\n').unwrap() + 1);
        let summary: Vec<(&str, &str)> = messages
            .iter()
            .map(|m| (m["kind"].as_str().unwrap(), m["text"].as_str().unwrap()))
            .collect();
        assert_eq!(
            summary,
            [
                ("text", "fix the test"),
                ("text", "On it."),
                ("tool_use", r#"{"command":"npm test"}"#),
                ("tool_result", "1 passed"),
            ]
        );
        assert_eq!(messages[2]["tool"], "Bash");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[3]["at"], "t3");
    }

    #[test]
    fn long_tool_text_is_cut() {
        let long = "x".repeat(TOOL_TEXT + 10);
        let line =
            json!({"type":"user","message":{"content":[{"type":"tool_result","content":long}]}});
        let text = messages(&line)[0]["text"].as_str().unwrap().to_string();
        assert_eq!(text.chars().count(), TOOL_TEXT + 1);
    }

    #[test]
    fn codex_projects_user_text_and_images_without_inline_data() {
        let item = json!({"type":"userMessage", "id":"u1", "completedAtMs":42,
            "content":[{"type":"text", "text":"Inspect this"},
            {"type":"image", "image_url":"data:image/png;base64,secret"},
            {"type":"localImage", "path":"/tmp/image.png"}]});
        let projected = codex_item(&item, "/repo");
        assert_eq!(
            projected,
            vec![json!({"role":"user", "kind":"text",
            "text":"Inspect this\n[image]\n[image]", "at":42})]
        );
        let record = json!({"type":"codex", "messages":projected});
        assert_eq!(messages(&record), projected);
    }

    #[test]
    fn codex_uses_shared_assistant_and_command_projection() {
        let assistant = codex_item(
            &json!({"type":"agentMessage", "id":"a1", "text":"Done.", "completedAtMs":43}),
            "/repo",
        );
        assert_eq!(
            assistant,
            vec![json!({"role":"assistant", "kind":"text", "text":"Done.", "at":43})]
        );
        let command = codex_item(
            &json!({"type":"commandExecution", "id":"t1", "command":"npm test",
                "status":"completed", "aggregatedOutput":"1 passed", "exitCode":0}),
            "/repo",
        );
        assert_eq!(
            command,
            vec![
                json!({"role":"assistant", "kind":"tool_use", "tool":"Bash", "text":r#"{"command":"npm test"}"#, "at":null}),
                json!({"role":"user", "kind":"tool_result", "error":false, "text":"1 passed", "at":null}),
            ]
        );
        assert!(codex_item(
            &json!({"type":"reasoning", "id":"r1", "summary":["Private"]}),
            "/repo"
        )
        .is_empty());
    }

    #[test]
    fn codex_bounds_unicode_message_and_tool_text() {
        let text = "界".repeat(MESSAGE_TEXT + 1);
        let assistant = codex_item(
            &json!({"type":"agentMessage", "id":"a1", "text":text}),
            "/repo",
        );
        assert_eq!(
            assistant[0]["text"].as_str().unwrap().chars().count(),
            MESSAGE_TEXT + 1
        );
        assert!(assistant[0]["text"].as_str().unwrap().ends_with('…'));
        let command = codex_item(
            &json!({"type":"commandExecution", "id":"t1",
            "command":"é".repeat(TOOL_TEXT + 1), "status":"failed", "exitCode":1,
            "aggregatedOutput":"界".repeat(TOOL_TEXT + 1)}),
            "/repo",
        );
        assert_eq!(command.len(), 2);
        for message in &command {
            assert_eq!(
                message["text"].as_str().unwrap().chars().count(),
                TOOL_TEXT + 1
            );
        }
        assert_eq!(command[1]["error"], true);
    }
}
