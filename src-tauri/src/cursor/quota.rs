//! Dashboard quotas share the CLI's login, independently of ACP. Credentials stay in memory.
use crate::{
    paths,
    usage::{rfc3339, Window},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::blocking::{Client, RequestBuilder};
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DASHBOARD: &str = "https://cursor.com";

/// Called by the existing account scheduler; failure retains its last successful reading.
pub fn quota() -> Option<Vec<Window>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let cookie = session_cookie(&access_token()?, now)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Prometeu/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let weekly = fetch(
        client
            .post(format!("{DASHBOARD}/api/dashboard/get-sand-usage-status"))
            .header("Cookie", &cookie)
            .header("Origin", DASHBOARD)
            .json(&serde_json::json!({})),
    )?;
    windows(&weekly, now, || {
        fetch(
            client
                .get(format!("{DASHBOARD}/api/usage-summary"))
                .header("Cookie", &cookie),
        )
    })
}

fn windows(
    weekly: &Value,
    now: u64,
    monthly: impl FnOnce() -> Option<Value>,
) -> Option<Vec<Window>> {
    // An unavailable weekly endpoint cannot authorize falling back to a stale monthly zero.
    match weekly.get("isUnified")?.as_bool()? {
        true => weekly_windows(weekly, now),
        false => monthly_windows(&monthly()?, now),
    }
}

fn fetch(request: RequestBuilder) -> Option<Value> {
    let response = request
        .header("Accept", "application/json")
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let mut bytes = Vec::new();
    response.take(1_048_577).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1_048_576 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

fn access_token() -> Option<String> {
    if let Ok(token) = std::env::var("CURSOR_AUTH_TOKEN") {
        return Some(token);
    }
    // An API key or in-memory login must not borrow another account's saved subscription.
    if std::env::var("CURSOR_API_KEY").is_ok_and(|key| !key.is_empty()) {
        return None;
    }
    let store = std::env::var("AGENT_CLI_CREDENTIAL_STORE").unwrap_or_default();
    if store == "memory" {
        return None;
    }
    #[cfg(target_os = "macos")]
    if store != "file" {
        return prometeu_protocols::catalog::command_output(
            &prometeu_process::query::UnixQueryLauncher,
            std::process::Command::new("/usr/bin/security").args([
                "find-generic-password",
                "-a",
                "cursor-user",
                "-s",
                "cursor-access-token",
                "-w",
            ]),
            Duration::from_secs(5),
        )
        .ok()
        .map(|token| token.trim().to_string());
    }
    let path = auth_path(
        &paths::home(),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    )?;
    let file = std::fs::File::open(path).ok()?;
    let auth: Value = serde_json::from_reader(file.take(65_536)).ok()?;
    auth["accessToken"].as_str().map(str::to_string)
}

fn auth_path(home: &std::path::Path, config: Option<PathBuf>) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(home.join(".cursor/auth.json"))
    } else {
        let config = config.unwrap_or_else(|| home.join(".config"));
        config
            .is_absolute()
            .then(|| config.join("cursor/auth.json"))
    }
}

fn session_cookie(token: &str, now: u64) -> Option<String> {
    if token.len() > 65_536
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return None;
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).ok()?).ok()?;
    if claims["exp"].as_u64()? <= now.saturating_add(60) {
        return None;
    }
    let user = claims["sub"].as_str()?.rsplit('|').next()?;
    if user.is_empty()
        || !user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return None;
    }
    Some(format!("WorkosCursorSessionToken={user}%3A%3A{token}"))
}

fn percent(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|pct| pct.is_finite() && (0.0..=100.0).contains(pct))
}

fn weekly_windows(value: &Value, now: u64) -> Option<Vec<Window>> {
    let has_limit = match value.get("includedLimitZero").filter(|v| !v.is_null()) {
        Some(zero) => !zero.as_bool()?,
        None => value["hasNonZeroIncludedLimit"].as_bool()?,
    };
    let resets = rfc3339(value["nextResetTimestampUtc"].as_str()?)?;
    if !has_limit || resets <= now {
        return None;
    }
    Some(vec![Window {
        kind: "weekly".into(),
        pct: percent(&value["usagePercent"])?,
        resets,
        scope: None,
        label: None,
    }])
}

fn monthly_windows(value: &Value, now: u64) -> Option<Vec<Window>> {
    // The verified mapping is an individual included allowance, not a shared team budget.
    if value["limitType"].as_str()? != "user" || value["isUnlimited"].as_bool()? {
        return None;
    }
    let start = rfc3339(value["billingCycleStart"].as_str()?)?;
    let resets = rfc3339(value["billingCycleEnd"].as_str()?)?;
    if start > now || resets <= now || resets <= start {
        return None;
    }
    let plan = &value["individualUsage"]["plan"];
    if !plan["enabled"].as_bool()? || plan["limit"].as_f64()? <= 0.0 {
        return None;
    }
    let mut windows = vec![];
    for (field, scope, label) in [
        ("totalPercentUsed", "general", None),
        ("autoPercentUsed", "cursor", Some("Cursor")),
        ("apiPercentUsed", "api", Some("API")),
    ] {
        // Optional pools stay absent when the dashboard does not report them.
        let Some(value) = plan.get(field).filter(|v| !v.is_null()) else {
            continue;
        };
        windows.push(Window {
            kind: format!("duration:{}", resets - start),
            pct: percent(value)?,
            resets,
            scope: Some(scope.into()),
            label: label.map(str::to_string),
        });
    }
    (!windows.is_empty()).then_some(windows)
}

#[cfg(test)]
mod tests;
