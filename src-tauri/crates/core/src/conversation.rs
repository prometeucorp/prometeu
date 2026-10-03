//! Canonical conversation primitives for backend adapters. This module defines V1 events without
//! knowing transports, processes, or external protocols.

use serde_json::{json, Value};

pub fn event(kind: &str, at: u64, fields: Value) -> Value {
    let mut out = fields.as_object().cloned().unwrap_or_default();
    out.insert("v".into(), json!(1));
    out.insert("type".into(), json!(kind));
    out.insert("at".into(), json!(at));
    Value::Object(out)
}

/// The main agent producing output. Adapters keep native subagent content isolated, so these events
/// always come from the primary turn: they mark a turn active again and invalidate a completion that
/// background tasks were holding. `src/alert.ts` mirrors this list for the notice.
pub fn agent_activity(event: &Value) -> bool {
    matches!(
        event["type"].as_str(),
        Some(
            "assistant.started"
                | "assistant.block.started"
                | "assistant.block"
                | "assistant.delta"
                | "tool.input.delta"
                | "tool.completed"
        )
    )
}

/// Time supplied by the execution host, in Unix milliseconds.
pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

pub mod stream;
pub mod work;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_envelope_preserves_event_fields() {
        let value = event("system.notice", 12, json!({ "detail": "hello" }));
        assert_eq!(value["v"], 1);
        assert_eq!(value["type"], "system.notice");
        assert_eq!(value["at"], 12);
        assert_eq!(value["detail"], "hello");
    }
}

pub mod usage;
