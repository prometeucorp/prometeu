//! Stable application errors retain the existing IPC encoding.

pub fn code(code: &str) -> String {
    format!("i18n:{}", serde_json::json!({ "code": code }))
}

/// Encode named arguments without translating application data.
pub fn with_args(code: &str, args: &[(&str, String)]) -> String {
    let map: std::collections::BTreeMap<&str, &str> =
        args.iter().map(|(k, v)| (*k, v.as_str())).collect();
    format!("i18n:{}", serde_json::json!({ "code": code, "args": map }))
}
