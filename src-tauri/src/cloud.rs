//! Optional Prometeu account. Passwords stay in the browser; the device token stays in a private
//! file and never crosses IPC. Conversations are not synchronized.

use crate::{i18n, oauth, paths};
use reqwest::{blocking::Client, Method, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::AppHandle;

const CLIENT: &str = "prometeu-desktop";
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);
static STORAGE: Mutex<()> = Mutex::new(());

#[derive(Clone, Deserialize, Serialize)]
pub struct User {
    id: String,
    name: String,
    email: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Saved {
    origin: String,
    token: String,
    user: User,
}

/// One relay identity per Mac, kept apart from the login so it survives sign-out. The Cloud lets the
/// first Mac keep the membership identity and connects any other Mac as a companion device.
#[derive(Clone, Deserialize, Serialize)]
struct Device {
    id: String,
    label: String,
}

#[derive(Serialize)]
pub struct Status {
    user: Option<User>,
    origin: String,
    offline: bool,
}

#[derive(Clone)]
struct Pending {
    id: String,
    origin: String,
    code: String,
    until: Instant,
    next: Instant,
    interval: u64,
}

#[derive(Serialize)]
pub struct Login {
    id: String,
    user_code: String,
    url: String,
    interval: u64,
}

#[derive(Deserialize, Serialize)]
pub struct Organization {
    id: String,
    slug: String,
    name: String,
    member: String,
    role: String,
}

#[derive(Serialize)]
pub struct Organizations {
    user: Option<User>,
    origin: String,
    organizations: Vec<Organization>,
}

fn relay_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[tauri::command]
pub async fn cloud_organizations() -> Result<Organizations, String> {
    blocking(|| {
        let _storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(saved) = load()? else {
            return Ok(Organizations {
                user: None,
                origin: default_origin()?,
                organizations: vec![],
            });
        };
        let device = device()?;
        let (code, value) = http(
            &saved.origin,
            Method::GET,
            &format!("/api/organizations?device={}", device.id),
            Some(&saved.token),
            None,
            262_144,
            Duration::from_secs(12),
        )?;
        if code != 200 {
            return Err(i18n::t("err.cloud.network"));
        }
        let organizations: Vec<Organization> =
            serde_json::from_value(value["organizations"].clone())
                .map_err(|_| i18n::t("err.cloud.response"))?;
        let mut ids = std::collections::BTreeSet::new();
        if organizations.iter().any(|org| {
            !relay_id(&org.id)
                || !relay_id(&org.member)
                || org.name.trim().is_empty()
                || org.name.len() > 320
                || org.slug.is_empty()
                || org.slug.len() > 48
                || !org
                    .slug
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                || !matches!(org.role.as_str(), "owner" | "admin" | "member")
                || !ids.insert(org.id.clone())
        }) {
            return Err(i18n::t("err.cloud.response"));
        }
        Ok(Organizations {
            user: Some(saved.user),
            origin: saved.origin,
            organizations,
        })
    })
    .await
}

fn relay_socket_url(relay: &str, organization: &str, ticket: &str) -> Result<String, String> {
    if !relay_id(organization) || ticket.len() != 43 || !relay_id(ticket) {
        return Err(i18n::t("err.cloud.response"));
    }
    let mut url = Url::parse(&origin(relay)?).map_err(|_| i18n::t("err.cloud.response"))?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|_| i18n::t("err.cloud.response"))?;
    url.set_path(&format!("/organization/{organization}"));
    url.query_pairs_mut()
        .append_pair("ticket", ticket)
        .append_pair("p", "4");
    Ok(url.to_string())
}

#[tauri::command]
pub async fn cloud_relay_ticket(
    organization: String,
    user: String,
    expected_origin: String,
) -> Result<String, String> {
    blocking(move || {
        let _storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
        let saved = load()?.ok_or_else(|| i18n::t("err.catalog.disconnected"))?;
        if saved.user.id != user || saved.origin != expected_origin || !relay_id(&organization) {
            return Err(i18n::t("err.cloud.response"));
        }
        let device = device()?;
        let (code, value) = http(
            &saved.origin,
            Method::POST,
            &format!("/api/organizations/{organization}/relay-ticket"),
            Some(&saved.token),
            Some(json!({ "device": device.id, "label": device.label })),
            16_384,
            Duration::from_secs(12),
        )?;
        if code != 200 {
            return Err(i18n::t("err.cloud.network"));
        }
        relay_socket_url(
            value["relay"].as_str().unwrap_or(""),
            &organization,
            value["ticket"].as_str().unwrap_or(""),
        )
    })
    .await
}

fn origin(value: &str) -> Result<String, String> {
    let url = Url::parse(value).map_err(|_| i18n::t("err.cloud.url"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(i18n::t("err.cloud.url"));
    }
    Ok(url.origin().ascii_serialization())
}

fn default_origin() -> Result<String, String> {
    origin(
        &std::env::var("PROMETEU_CLOUD_URL").unwrap_or_else(|_| "https://app.prometeu.co".into()),
    )
}

fn device() -> Result<Device, String> {
    let path = paths::root().join("device.json");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(device) = serde_json::from_slice::<Device>(&bytes) {
            if relay_id(&device.id) {
                return Ok(device);
            }
        }
    }
    let label = crate::platform::device_name();
    let device = Device {
        id: oauth::random()[..32].to_string(),
        label,
    };
    paths::write_private(&path, &serde_json::to_string(&device).unwrap())
        .map_err(|_| i18n::t("err.cloud.storage"))?;
    Ok(device)
}

fn load() -> Result<Option<Saved>, String> {
    match std::fs::read(paths::root().join("cloud.json")) {
        Ok(bytes) => {
            let saved: Saved =
                serde_json::from_slice(&bytes).map_err(|_| i18n::t("err.cloud.storage"))?;
            origin(&saved.origin)?;
            Ok(Some(saved))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(i18n::t("err.cloud.storage")),
    }
}

fn save(saved: &Saved) -> Result<(), String> {
    paths::write_private(
        &paths::root().join("cloud.json"),
        &serde_json::to_string(saved).unwrap(),
    )
    .map_err(|_| i18n::t("err.cloud.storage"))
}

fn clear() -> Result<(), String> {
    match std::fs::remove_file(paths::root().join("cloud.json")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(i18n::t("err.cloud.storage")),
    }
}

fn request(
    origin: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Result<(u16, Value), String> {
    let method = if body.is_some() {
        Method::POST
    } else {
        Method::GET
    };
    http(
        origin,
        method,
        &format!("/api/auth/{path}"),
        token,
        body,
        65_536,
        Duration::from_secs(12),
    )
}

/// Call the API with the stored credential, or return None without an account. Callers receive only
/// status and JSON, never the token. Uploads forwarded to a third party need more than the default
/// timeout, so the caller chooses it.
pub(crate) fn api(
    method: Method,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<Option<(u16, Value)>, String> {
    Ok(api_as(method, path, body, timeout)?.map(|(_, status, value)| (status, value)))
}

/// Like `api`, also naming the account (`origin#user id`) whose credential made the request, so
/// per-account state survives switching accounts between calls.
pub(crate) fn api_as(
    method: Method,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<Option<(String, u16, Value)>, String> {
    let Some(saved) = load()? else {
        return Ok(None);
    };
    let (status, value) = http(
        &saved.origin,
        method,
        path,
        Some(&saved.token),
        body,
        512 * 1024,
        timeout,
    )?;
    Ok(Some((
        format!("{}#{}", saved.origin, saved.user.id),
        status,
        value,
    )))
}

pub(crate) fn connected() -> bool {
    load().ok().flatten().is_some()
}

fn http(
    origin: &str,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    max: usize,
    timeout: Duration,
) -> Result<(u16, Value), String> {
    let client = Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("Prometeu Desktop")
        .build()
        .map_err(|_| i18n::t("err.cloud.network"))?;
    let mut request = client.request(method, format!("{origin}{path}"));
    if let Some(body) = body {
        request = request.json(&body);
    }
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|_| i18n::t("err.cloud.network"))?;
    let status = response.status().as_u16();
    // No response body, URL with secrets, or token is included in an error.
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| i18n::t("err.cloud.network"))?;
    if bytes.len() > max {
        return Err(i18n::t("err.cloud.response"));
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| i18n::t("err.cloud.response"))?;
    Ok((status, value))
}

fn user(value: &Value) -> Result<User, String> {
    let user: User =
        serde_json::from_value(value["user"].clone()).map_err(|_| i18n::t("err.cloud.response"))?;
    if user.id.is_empty()
        || user.id.len() > 128
        || user.name.trim().is_empty()
        || user.name.len() > 320
        || user.email.len() > 320
    {
        return Err(i18n::t("err.cloud.response"));
    }
    Ok(user)
}

fn status(refresh: bool) -> Result<Status, String> {
    let _storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
    let saved = load()?;
    let Some(mut saved) = saved else {
        return Ok(Status {
            user: None,
            origin: default_origin()?,
            offline: false,
        });
    };
    let mut offline = false;
    if refresh {
        match request(&saved.origin, "get-session", Some(&saved.token), None) {
            Ok((401, _)) | Ok((200, Value::Null)) => {
                clear()?;
                return Ok(Status {
                    user: None,
                    origin: default_origin()?,
                    offline: false,
                });
            }
            Ok((200, value)) => {
                saved.user = user(&value)?;
                save(&saved)?;
            }
            _ => offline = true,
        }
    }
    Ok(Status {
        user: Some(saved.user),
        origin: saved.origin,
        offline,
    })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| i18n::t("err.cloud.network"))?
}

#[tauri::command]
pub async fn cloud_status(app: AppHandle, refresh: bool) -> Result<Status, String> {
    blocking(move || {
        let status = status(refresh)?;
        // Load the catalog with the account; catalog failures must not invalidate the account.
        if refresh && status.user.is_some() && !status.offline {
            crate::catalog::pull(&app)?;
        }
        Ok(status)
    })
    .await
}

#[tauri::command]
pub async fn cloud_login_start(signup: bool) -> Result<Login, String> {
    blocking(move || {
        let origin = default_origin()?;
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let (code, value) = request(
            &origin,
            "device/code",
            None,
            Some(json!({"client_id": CLIENT})),
        )?;
        if code != 200 {
            return Err(i18n::t("err.cloud.network"));
        }
        let read = |key: &str| {
            value[key]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .map(str::to_owned)
                .ok_or_else(|| i18n::t("err.cloud.response"))
        };
        let device_code = read("device_code")?;
        let user_code = read("user_code")?;
        let interval = value["interval"].as_u64().unwrap_or(5).clamp(5, 60);
        let expires = value["expires_in"].as_u64().unwrap_or(300).clamp(1, 900);
        let id = uuid::Uuid::new_v4().to_string();
        let mut url = Url::parse(&format!("{origin}/device")).unwrap();
        url.query_pairs_mut()
            .append_pair("user_code", &user_code)
            .append_pair("mode", if signup { "signup" } else { "login" })
            .append_pair("lang", &i18n::lang());
        // Construct the URL from our configured origin, never trust a redirect from the server.
        oauth::browse(url.as_str()).map_err(|_| i18n::t("err.cloud.browser"))?;
        *pending = Some(Pending {
            id: id.clone(),
            origin,
            code: device_code,
            until: Instant::now() + Duration::from_secs(expires),
            next: Instant::now() + Duration::from_secs(interval),
            interval,
        });
        Ok(Login {
            id,
            user_code,
            url: url.to_string(),
            interval,
        })
    })
    .await
}

#[tauri::command]
pub async fn cloud_login_poll(app: AppHandle, id: String) -> Result<Option<Status>, String> {
    blocking(move || {
        let attempt = {
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            let p = pending.as_mut().filter(|p| p.id == id).ok_or_else(|| i18n::t("err.cloud.expired"))?;
            if Instant::now() >= p.until { *pending = None; return Err(i18n::t("err.cloud.expired")); }
            if Instant::now() < p.next { return Ok(None); }
            p.next = Instant::now() + Duration::from_secs(p.interval + 12);
            p.clone()
        };
        let (code, value) = request(&attempt.origin, "device/token", None, Some(json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": attempt.code, "client_id": CLIENT,
        })))?;
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if pending.as_ref().is_none_or(|p| p.id != id) {
            drop(pending);
            if let Some(token) = value["access_token"].as_str() { let _ = request(&attempt.origin, "sign-out", Some(token), Some(json!({}))); }
            return Err(i18n::t("err.cloud.expired"));
        }
        if code != 200 {
            match value["error"].as_str() {
                Some("authorization_pending" | "slow_down") => {
                    let p = pending.as_mut().unwrap();
                    if value["error"] == "slow_down" { p.interval = (p.interval + 5).min(60); }
                    p.next = Instant::now() + Duration::from_secs(p.interval);
                    return Ok(None);
                }
                _ => { *pending = None; return Err(i18n::t("err.cloud.expired")); }
            }
        }
        let token = value["access_token"].as_str().filter(|s| !s.is_empty() && s.len() <= 4096)
            .ok_or_else(|| i18n::t("err.cloud.response"))?;
        drop(pending);
        let result = (|| {
            let (code, value) = request(&attempt.origin, "get-session", Some(token), None)?;
            if code != 200 { return Err(i18n::t("err.cloud.response")); }
            let saved = Saved { origin: attempt.origin.clone(), token: token.into(), user: user(&value)? };
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            if pending.as_ref().is_none_or(|p| p.id != id) { return Err(i18n::t("err.cloud.expired")); }
            let _storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
            // Changing identity must not reuse links from the previous account's catalog.
            if load()?.is_none_or(|old| old.user.id != saved.user.id || old.origin != saved.origin) {
                crate::catalog::forget();
            }
            save(&saved)?;
            *pending = None;
            drop(pending);
            drop(_storage);
            if let Err(error) = crate::catalog::pull(&app) {
                eprintln!("catálogo não sincronizado: {error}");
            }
            Ok(Some(Status { user: Some(saved.user), origin: saved.origin, offline: false }))
        })();
        if result.is_err() {
            // A consumed code cannot be retried. Revoke any session whose credential could not be
            // saved.
            let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            if pending.as_ref().is_some_and(|p| p.id == id) { *pending = None; }
            drop(pending);
            let _ = request(&attempt.origin, "sign-out", Some(token), Some(json!({})));
        }
        result
    }).await
}

#[tauri::command]
pub async fn cloud_login_cancel(id: String) -> Result<(), String> {
    blocking(move || {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if pending.as_ref().is_some_and(|p| p.id == id) {
            *pending = None;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn cloud_logout() -> Result<Status, String> {
    blocking(|| {
        let _storage = STORAGE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(saved) = load()? {
            let (code, _) = request(
                &saved.origin,
                "sign-out",
                Some(&saved.token),
                Some(json!({})),
            )?;
            if code != 200 && code != 401 {
                return Err(i18n::t("err.cloud.network"));
            }
            clear()?;
            crate::catalog::forget();
        }
        Ok(Status {
            user: None,
            origin: default_origin()?,
            offline: false,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_cloud_contract_profile_uses_the_production_decoder() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/cloud-api.json")).unwrap();
        let profile = user(&fixture["session"]).unwrap();
        assert_eq!(
            serde_json::to_value(profile).unwrap(),
            fixture["session"]["user"]
        );
        assert!(user(&fixture["signed_out"]).is_err());
    }

    #[test]
    fn organization_socket_uses_only_a_short_lived_ticket_and_validated_origin() {
        let ticket = "t".repeat(43);
        let url = relay_socket_url("https://relay.example", "organization1", &ticket).unwrap();
        let url = Url::parse(&url).unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.path(), "/organization/organization1");
        assert_eq!(url.query_pairs().count(), 2);
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "p" && value == "4"));
        assert!(url
            .query_pairs()
            .any(|(key, value)| key == "ticket" && value == ticket));
        for relay in [
            "http://outside.example",
            "https://user:password@relay.example",
            "https://relay.example/path",
            "https://relay.example?token=secret",
        ] {
            assert!(relay_socket_url(relay, "organization1", &ticket).is_err());
        }
        assert!(
            relay_socket_url("http://127.0.0.1:8787", "organization1", &ticket)
                .unwrap()
                .starts_with("ws://")
        );
        assert!(relay_socket_url("https://relay.example", "../other", &ticket).is_err());
        assert!(relay_socket_url(
            "https://relay.example",
            "organization1",
            "long-lived-bearer"
        )
        .is_err());
    }

    #[test]
    fn cloud_origin_requires_https_except_loopback() {
        for value in [
            "http://example.com",
            "https://x.test/path",
            "https://x.test?token=x",
            "https://user:secret@x.test",
            "file:///tmp/cloud",
            "https://x.test#fragment",
        ] {
            assert!(origin(value).is_err(), "{value}");
        }
        assert_eq!(
            origin("https://app.prometeu.co/").unwrap(),
            "https://app.prometeu.co"
        );
        assert!(origin("http://127.0.0.1:3100").is_ok());
    }

    #[test]
    fn cloud_status_never_serializes_credentials() {
        let status = Status {
            user: Some(User {
                id: "id".into(),
                name: "Nome".into(),
                email: "me@example.com".into(),
            }),
            origin: "https://app.prometeu.co".into(),
            offline: false,
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["user"]["name"], "Nome");
        assert!(value.get("token").is_none());
        assert!(
            user(&json!({"user": {"id": "", "name": "Nome", "email": "me@example.com"}})).is_err()
        );
    }

    #[test]
    fn cloud_http_does_not_follow_redirects_or_expose_response_secrets() {
        crate::install_crypto();
        use std::io::{Read, Write};
        use std::net::TcpListener;
        for response in [
            "HTTP/1.1 302 Found\r\nLocation: https://example.com/stolen\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nsecret-token",
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut request = [0; 4096];
                let received = stream.read(&mut request).unwrap();
                assert!(received > 0);
                stream.write_all(response.as_bytes()).unwrap();
            });
            let result = request(&origin, "get-session", Some("private-token"), None);
            if response.starts_with("HTTP/1.1 302") {
                assert_eq!(result.unwrap().0, 302);
            } else {
                let error = result.unwrap_err();
                assert!(!error.contains("secret-token"));
                assert!(!error.contains("private-token"));
                assert!(error.contains("err.cloud.response"));
            }
            server.join().unwrap();
        }
    }
}
