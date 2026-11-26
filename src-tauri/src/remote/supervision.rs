//! Account-authorized provisioning during MCP setup. Only ingestion credentials reach the host.
use super::connection::Connection;
use crate::{cloud, i18n};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    name: String,
    url: String,
    webhook_protocol: String,
    signing_secret: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    account: String,
    supervisor_id: Option<String>,
    destination: Option<Destination>,
}

#[derive(Deserialize, Serialize)]
pub struct Supervisor {
    id: String,
    name: String,
    url: String,
    enabled: bool,
    webhook_protocol: String,
}

#[derive(Serialize)]
pub struct Options {
    account: Option<String>,
    supervisor: Option<Supervisor>,
}

#[derive(Deserialize, Serialize)]
pub struct TestResult {
    status: String,
    failed: bool,
}

fn command(
    connection: &Connection,
    action: &str,
    origin: Option<&str>,
    token: &[u8],
) -> Result<Value, String> {
    let binary = super::install::local_binary().ok_or_else(|| i18n::t("err.remote.noBuild"))?;
    let ready = connection
        .ready()
        .ok_or_else(|| i18n::t(super::connection::OFFLINE))?;
    let root = ready["root"]
        .as_str()
        .ok_or_else(|| i18n::t(super::connection::OFFLINE))?;
    let program = format!("{root}/host/{}/prometeu-host", binary.hash);
    let mut args = vec![program.as_str(), "supervision", action, "--root", root];
    if let Some(origin) = origin {
        args.extend_from_slice(&["--url", origin]);
    }
    let script = args
        .iter()
        .map(|arg| super::agent::quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let output = super::install::run(
        &connection.machine().ssh,
        &script,
        token,
        Duration::from_secs(65),
    )?;
    serde_json::from_str(&output).map_err(|_| i18n::t("err.cloud.response"))
}

#[tauri::command(async)]
pub fn machine_supervision_options() -> Result<Options, String> {
    let Some((account, status, value)) = cloud::api_as(
        Method::GET,
        "/api/supervisor",
        None,
        Duration::from_secs(12),
    )?
    else {
        return Ok(Options {
            account: None,
            supervisor: None,
        });
    };
    if status != 200 {
        return Err(i18n::t("err.supervision.unavailable"));
    }
    let supervisor = serde_json::from_value(value["supervisor"].clone())
        .map_err(|_| i18n::t("err.cloud.response"))?;
    Ok(Options {
        account: Some(account),
        supervisor,
    })
}

pub fn enable(connection: &Connection, setup: Setup) -> Result<(), String> {
    // The old build must support this command before Cloud issues any credential.
    let identity = command(connection, "identity", None, b"")?;
    let executor = identity["executor_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or_else(|| i18n::t("err.cloud.response"))?;
    let (origin, status, value) = cloud::connect_supervisor(
        &setup.account,
        json!({
            "executor_id": executor, "supervisor_id": setup.supervisor_id, "destination": setup.destination,
        }),
    )?;
    if status != 200 {
        return Err(i18n::t(match value["error"].as_str() {
            Some("changed") => "err.supervision.changed",
            Some("disabled") => "err.supervision.disabled",
            Some("invalid") => "err.supervision.invalid",
            _ => "err.supervision.unavailable",
        }));
    }
    let token = ingest_token(&value)?;
    let configured = command(connection, "setup", Some(&origin), token.as_bytes())?;
    if configured["enabled"] != true
        || configured["executor_id"] != executor
        || configured["supervisor"]["id"] != value["supervisor"]["id"]
    {
        return Err(i18n::t("err.cloud.response"));
    }
    Ok(())
}

fn ingest_token(value: &Value) -> Result<&str, String> {
    value["ingest_token"]
        .as_str()
        .filter(|token| {
            token.len() == 43
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .ok_or_else(|| i18n::t("err.cloud.response"))
}

#[tauri::command(async)]
pub fn machine_supervision_enable(app: AppHandle, id: String, setup: Setup) -> Result<(), String> {
    let connection = app.state::<super::Service>().connected(&id)?;
    enable(&connection, setup)
}

#[tauri::command(async)]
pub fn machine_supervision_test(app: AppHandle, id: String) -> Result<TestResult, String> {
    let connection = app.state::<super::Service>().connected(&id)?;
    let value = command(&connection, "test", None, b"")?;
    let result: TestResult =
        serde_json::from_value(value).map_err(|_| i18n::t("err.cloud.response"))?;
    if !matches!(
        result.status.as_str(),
        "pending" | "sending" | "delivered" | "cancelled"
    ) {
        return Err(i18n::t("err.cloud.response"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_accepts_only_a_bounded_token_without_forwarding_response_fields() {
        assert!(ingest_token(
            &json!({"ingest_token": "a".repeat(43), "signing_secret":"receiver-only"})
        )
        .is_ok());
        for token in [
            "short".to_string(),
            "a".repeat(8193),
            format!("{}\n", "a".repeat(42)),
        ] {
            let error = ingest_token(&json!({"ingest_token":token})).unwrap_err();
            assert!(!error.contains(&token));
        }
        let options = Options {
            account: Some("https://cloud.example#owner".into()),
            supervisor: None,
        };
        assert_eq!(
            serde_json::to_value(options).unwrap(),
            json!({"account":"https://cloud.example#owner", "supervisor":null})
        );
    }
}
