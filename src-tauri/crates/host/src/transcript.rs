//! A compact reading of Claude's native transcript (`~/.claude/projects/<slug>/<id>.jsonl`) for
//! agent clients: what was said and which tools ran, without the native envelope.
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

/// The messages one transcript line contributes. Meta lines, subagent lines and records other
/// than user and assistant turns contribute none.
pub fn messages(line: &Value) -> Vec<Value> {
    let kind = line.get("type").and_then(Value::as_str).unwrap_or_default();
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
}
