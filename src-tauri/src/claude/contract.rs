//! Synthetic translation scenarios, not recordings or certification of an installed CLI version.

use super::*;

pub(crate) fn events() -> Vec<Value> {
    let mut adapter = Adapter::default();
    [
        json!({"type":"user","message":{"content":"Review the patch"}}),
        json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"answer"}}}),
        json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}),
        json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Reviewed"}}}),
        json!({"type":"assistant","message":{"id":"answer","content":[{"type":"text","text":"Reviewed"},{"type":"tool_use","id":"read","name":"Read","input":{"file_path":"README.md"}}]}}),
        json!({"type":"control_request","request_id":"approval","request":{"subtype":"can_use_tool","tool_name":"Read","tool_use_id":"read","input":{"file_path":"README.md"}}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"read","content":"Project guide","is_error":false}]}}),
        json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),
        json!({"type":"result","is_error":false,"duration_ms":10}),
    ].iter().flat_map(|frame| adapter.translate(frame)).collect()
}
