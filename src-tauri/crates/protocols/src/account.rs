//! Identity metadata from the official Codex account response; credentials stay private.
use prometeu_core::{accounts::Identity, error::code};
use serde_json::Value;
pub fn parse_account(value: &Value) -> Result<Identity, String> {
    let account = value
        .get("account")
        .ok_or_else(|| code("err.account.status"))?;
    Ok(Identity {
        connected: !account.is_null(),
        email: account["email"].as_str().map(str::to_string),
        plan: account["planType"].as_str().map(str::to_string),
    })
}
