//! GitHub App user credential through GitHub's device flow (ADR 0088). The public client ID needs no
//! secret, and device-flow tokens refresh without one. The credential stays in a private file in
//! Rust and never crosses IPC. The token reaches only repositories where the App is installed and
//! the person has access.

use crate::{i18n, lock::lock, oauth, paths};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// The App's public client ID; it grants nothing without the person's approval on github.com.
pub const CLIENT_ID: &str = "Iv23li1eKmR5B0BpsHV2";
pub const INSTALL_URL: &str = "https://github.com/apps/prometeu-app/installations/new";
const DEVICE: &str = "https://github.com/login/device/code";
const TOKEN: &str = "https://github.com/login/oauth/access_token";
const API: &str = "https://api.github.com";
const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// Refresh early enough that an API call does not outlive its token.
const SLACK: u64 = 5 * 60;

#[derive(Serialize, Deserialize, Clone)]
struct Auth {
    access_token: String,
    refresh_token: Option<String>,
    /// Unix seconds; zero means the token does not expire.
    expires_at: u64,
    login: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Status {
    pub connected: bool,
    pub login: Option<String>,
    /// While the device flow waits, the code the person types on github.com and where.
    pub busy: bool,
    pub code: Option<String>,
    pub url: Option<String>,
    pub install_url: String,
}

struct Pending {
    code: String,
    url: String,
    cancelled: bool,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
/// Refresh tokens are single use: concurrent refreshes would invalidate each other.
static CREDENTIAL: Mutex<()> = Mutex::new(());

/* Commands */

#[tauri::command]
pub fn github_status() -> Status {
    status()
}

#[tauri::command]
pub async fn github_connect(app: AppHandle) -> Result<Status, String> {
    let handle = app.clone();
    let result = crate::linear::blocking(move || connect(&handle)).await;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    let now = status();
    let _ = app.emit("github", &now);
    result.map(|_| now)
}

/// Cancels a waiting device flow, or forgets the stored credential.
#[tauri::command]
pub fn github_disconnect(app: AppHandle) -> Result<Status, String> {
    if let Some(pending) = lock(&PENDING).as_mut() {
        pending.cancelled = true;
    } else {
        let _guard = lock(&CREDENTIAL);
        match std::fs::remove_file(path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(i18n::io(error)),
        }
        crate::github_issues::forget();
    }
    let now = status();
    let _ = app.emit("github", &now);
    Ok(now)
}

pub fn status() -> Status {
    let pending = lock(&PENDING);
    let login = load().map(|auth| auth.login);
    Status {
        connected: login.is_some(),
        login,
        busy: pending.is_some(),
        code: pending.as_ref().map(|p| p.code.clone()),
        url: pending.as_ref().map(|p| p.url.clone()),
        install_url: INSTALL_URL.into(),
    }
}

/* Device flow */

fn connect(app: &AppHandle) -> Result<(), String> {
    if lock(&PENDING).is_some() {
        return Err(i18n::t("err.github.waiting"));
    }
    let reply = token_endpoint(DEVICE, &[("client_id", CLIENT_ID)])?;
    let text = |key: &str| {
        reply[key]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
            .map(str::to_owned)
            .ok_or_else(|| i18n::t("err.github.response"))
    };
    let device = text("device_code")?;
    let code = text("user_code")?;
    let url = text("verification_uri")?;
    // GitHub returns its own device page; never open anything else.
    if !url.starts_with("https://github.com/") {
        return Err(i18n::t("err.github.response"));
    }
    let mut interval = reply["interval"].as_u64().unwrap_or(5).clamp(5, 60);
    let until =
        Instant::now() + Duration::from_secs(reply["expires_in"].as_u64().unwrap_or(900).min(900));
    {
        let mut pending = lock(&PENDING);
        if pending.is_some() {
            return Err(i18n::t("err.github.waiting"));
        }
        *pending = Some(Pending {
            code,
            url: url.clone(),
            cancelled: false,
        });
    }
    // Only the flow that registered the pending code clears it, on every exit path.
    struct Owned;
    impl Drop for Owned {
        fn drop(&mut self) {
            *lock(&PENDING) = None;
        }
    }
    let _owned = Owned;
    let _ = app.emit("github", &status());
    oauth::browse(&url).map_err(|_| i18n::t("err.github.browser"))?;

    loop {
        std::thread::sleep(Duration::from_secs(interval));
        if lock(&PENDING).as_ref().is_none_or(|p| p.cancelled) {
            return Ok(());
        }
        if Instant::now() >= until {
            return Err(i18n::t("err.github.expired"));
        }
        let reply = token_endpoint(
            TOKEN,
            &[
                ("client_id", CLIENT_ID),
                ("device_code", &device),
                ("grant_type", GRANT),
            ],
        )?;
        match reply["error"].as_str() {
            None => {}
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = (interval + 5).min(60);
                continue;
            }
            Some("access_denied") => return Err(i18n::t("err.github.denied")),
            Some(_) => return Err(i18n::t("err.github.expired")),
        }
        let mut auth = credential(&reply, None)?;
        let user = get_with(&auth.access_token, "/user", &[])?;
        auth.login = user["login"]
            .as_str()
            .filter(|login| valid_login(login))
            .ok_or_else(|| i18n::t("err.github.response"))?
            .into();
        let _guard = lock(&CREDENTIAL);
        save(&auth)?;
        crate::github_issues::forget();
        return Ok(());
    }
}

fn token_endpoint(url: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
    reqwest::blocking::Client::new()
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(oauth::form(fields))
        .timeout(Duration::from_secs(20))
        .send()
        .map_err(|e| i18n::ta("err.github.unreachable", &[("cause", e.to_string())]))?
        .json()
        .map_err(|_| i18n::t("err.github.response"))
}

fn credential(reply: &Value, previous: Option<&Auth>) -> Result<Auth, String> {
    let access_token = reply["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .ok_or_else(|| i18n::t("err.github.response"))?;
    Ok(Auth {
        access_token: access_token.into(),
        // A refresh response that omits the refresh token keeps the previous one.
        refresh_token: reply["refresh_token"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| previous.and_then(|p| p.refresh_token.clone())),
        expires_at: reply["expires_in"]
            .as_u64()
            .map_or(0, |seconds| oauth::now() + seconds),
        login: previous.map(|p| p.login.clone()).unwrap_or_default(),
    })
}

pub(crate) fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 39
        && login
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && !login.starts_with('-')
}

/* Credential */

/// The connected login and a token valid for at least five minutes.
pub fn token() -> Result<(String, String), String> {
    let _guard = lock(&CREDENTIAL);
    let auth = load().ok_or_else(|| i18n::t("err.github.off"))?;
    if auth.expires_at == 0 || auth.expires_at > oauth::now() + SLACK {
        return Ok((auth.login, auth.access_token));
    }
    let refresh = auth
        .refresh_token
        .as_deref()
        .ok_or_else(|| i18n::t("err.github.expired"))?;
    let reply = token_endpoint(
        TOKEN,
        &[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
        ],
    )?;
    if reply["error"].is_string() {
        // An expired or revoked refresh token cannot recover; the person connects again.
        let _ = std::fs::remove_file(path());
        return Err(i18n::t("err.github.expired"));
    }
    let fresh = credential(&reply, Some(&auth))?;
    save(&fresh)?;
    Ok((fresh.login, fresh.access_token))
}

fn path() -> PathBuf {
    paths::root().join("github.json")
}

fn load() -> Option<Auth> {
    serde_json::from_slice(&std::fs::read(path()).ok()?).ok()
}

fn save(auth: &Auth) -> Result<(), String> {
    paths::write_private(&path(), &serde_json::to_string(auth).map_err(i18n::io)?).map_err(i18n::io)
}

/* API */

/// Calls the REST API with the stored credential. Returns the status and JSON body so callers can
/// treat 404 (App not installed, or no access) as absence instead of failure.
pub fn api(
    method: Method,
    path: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let (_, token) = token()?;
    request(&token, method, path, query, body)
}

/// GET that fails on any non-success status.
pub fn get(path: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let (_, token) = token()?;
    get_with(&token, path, query)
}

fn get_with(token: &str, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
    match request(token, Method::GET, path, query, None)? {
        (200, value) => Ok(value),
        (404, _) => Err(i18n::t("err.github.notFound")),
        _ => Err(i18n::t("err.github.response")),
    }
}

pub fn graphql(query: &str, variables: Value) -> Result<Value, String> {
    let (_, token) = token()?;
    let (status, reply) = request(
        &token,
        Method::POST,
        "/graphql",
        &[],
        Some(serde_json::json!({ "query": query, "variables": variables })),
    )?;
    if status != 200 {
        return Err(i18n::t("err.github.response"));
    }
    // Missing or inaccessible nodes come back as null data with errors; callers treat null as absent.
    Ok(reply["data"].clone())
}

fn request(
    token: &str,
    method: Method,
    path: &str,
    query: &[(&str, String)],
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let url = reqwest::Url::parse_with_params(&format!("{API}{path}"), query)
        .map_err(|_| i18n::t("err.github.response"))?;
    let mut builder = reqwest::blocking::Client::new()
        .request(method, url)
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", "Prometeu")
        .timeout(Duration::from_secs(30));
    if let Some(body) = body {
        builder = builder.json(&body);
    }
    let response = builder
        .send()
        .map_err(|e| i18n::ta("err.github.unreachable", &[("cause", e.to_string())]))?;
    let status = response.status().as_u16();
    match status {
        401 => return Err(i18n::t("err.github.auth")),
        403 | 429
            if response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0") =>
        {
            return Err(i18n::t("err.github.slowDown"))
        }
        _ => {}
    }
    let bytes = response
        .bytes()
        .map_err(|_| i18n::t("err.github.response"))?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(i18n::t("err.github.response"));
    }
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).map_err(|_| i18n::t("err.github.response"))?
    };
    Ok((status, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn device_flow_tokens_expire_and_refresh_keeps_identity_and_refresh_token() {
        let first = credential(
            &json!({"access_token": "ghu_1", "refresh_token": "ghr_1", "expires_in": 28800}),
            None,
        )
        .unwrap();
        assert!(first.expires_at > oauth::now());
        let previous = Auth {
            login: "octo".into(),
            ..first
        };
        let fresh = credential(
            &json!({"access_token": "ghu_2", "expires_in": 28800}),
            Some(&previous),
        )
        .unwrap();
        assert_eq!(fresh.login, "octo");
        assert_eq!(fresh.refresh_token.as_deref(), Some("ghr_1"));
        let lasting = credential(&json!({"access_token": "ghu_3"}), None).unwrap();
        assert_eq!(lasting.expires_at, 0);
        assert!(credential(&json!({"error": "bad_refresh_token"}), None).is_err());
    }

    #[test]
    fn logins_follow_github_rules() {
        assert!(valid_login("octo-cat"));
        for bad in ["", "-octo", "octo cat", "a/b", &"a".repeat(40)] {
            assert!(!valid_login(bad), "{bad}");
        }
    }
}
