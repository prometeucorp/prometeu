//! Shared MCP authorization. Browser consent is supplied by the native host; tokens remain private.
mod i18n {
    pub use prometeu_core::error::{code as t, with_args as ta};
}
use crate::packages::PackageFiles;
use prometeu_oauth::{self as oauth, challenge, escape, form, now, random};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
const SLACK: u64 = 300;
const HTTP: Duration = Duration::from_secs(20);
const WAIT: Duration = Duration::from_secs(300);
/// Persisted login state for one server.
#[derive(Serialize, Deserialize, Clone)]
pub struct Auth {
    /// Reuse the registered OAuth client instead of creating duplicates on every login.
    pub client_id: String,
    /// Persist discovered token endpoints so refresh does not repeat endpoint discovery.
    pub token_endpoint: String,
    /// The RFC 8707 resource identifier is included in token exchange and refresh.
    pub resource: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix timestamp in seconds.
    pub expires_at: u64,
}

pub type Store = HashMap<String, Auth>;

pub trait AuthStorage: Send + Sync {
    fn load(&self) -> Store;
    fn save(&self, store: &Store) -> Result<(), String>;
}
pub struct PrivateAuthStorage {
    pub path: PathBuf,
    pub files: Arc<dyn PackageFiles>,
}
impl AuthStorage for PrivateAuthStorage {
    fn load(&self) -> Store {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save(&self, all: &Store) -> Result<(), String> {
        let body = serde_json::to_string_pretty(all).map_err(|e| e.to_string())?;
        self.files
            .write_private(&self.path, &body)
            .map_err(|cause| i18n::ta("err.mcp.auth.write", &[("cause", cause)]))
    }
}
struct Pending {
    id: String,
    ends: Endpoints,
    client: String,
    resource: String,
    verifier: String,
    expires: Instant,
}
pub trait Authorization: Send + Sync {
    fn pending(&self) -> bool;
    fn logged_in(&self) -> Vec<String>;
    fn forget(&self, id: &str) -> Result<(), String>;
    fn bearer(&self, id: &str) -> Option<String>;
    fn begin(
        &self,
        id: &str,
        url: &str,
        challenge: Option<&str>,
    ) -> Result<oauth::ConsentRequest, String>;
    fn finish(&self, state: &str, code: &str) -> Result<(), String>;
    fn cancel(&self, state: &str);
}
pub struct McpAuthorization {
    storage: Arc<dyn AuthStorage>,
    // Serialize credential mutations, including refresh versus logout.
    gate: Mutex<()>,
    pending: Mutex<HashMap<String, Pending>>,
}
impl McpAuthorization {
    pub fn new(storage: Arc<dyn AuthStorage>) -> Self {
        Self {
            storage,
            gate: Mutex::new(()),
            pending: Mutex::new(HashMap::new()),
        }
    }
}
impl Authorization for McpAuthorization {
    fn pending(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|p| p.expires > Instant::now())
    }

    fn logged_in(&self) -> Vec<String> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<_> = self.storage.load().into_keys().collect();
        ids.sort();
        ids
    }
    fn forget(&self, id: &str) -> Result<(), String> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, p| p.id != id);
        let mut all = self.storage.load();
        all.remove(id);
        self.storage.save(&all)
    }
    fn bearer(&self, id: &str) -> Option<String> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let mut all = self.storage.load();
        let auth = all.get(id)?.clone();
        if auth.expires_at > now() + SLACK {
            return Some(auth.access_token);
        }
        let fresh = renew(&auth).ok()?;
        let token = fresh.access_token.clone();
        all.insert(id.into(), fresh);
        self.storage.save(&all).ok()?;
        Some(token)
    }
    fn begin(
        &self,
        id: &str,
        url: &str,
        header: Option<&str>,
    ) -> Result<oauth::ConsentRequest, String> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let ends = discover(url, header)?;
        let client = match self
            .storage
            .load()
            .get(id)
            .filter(|a| a.token_endpoint == ends.token && a.resource == url)
        {
            Some(auth) => auth.client_id.clone(),
            None => register(&ends)?,
        };
        let verifier = random();
        let state = random();
        let scope = ends
            .scopes
            .as_ref()
            .map(|s| format!("&scope={}", escape(s)))
            .unwrap_or_default();
        let authorize=format!("{}?response_type=code&client_id={}&redirect_uri={}&state={state}&code_challenge={}&code_challenge_method=S256&resource={}{scope}",ends.authorize,escape(&client),escape(oauth::MCP_REDIRECT),challenge(&verifier),escape(url));
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, p| p.expires > Instant::now() && p.id != id);
        if pending.len() >= 8 {
            return Err(i18n::t("err.mcp.auth.timeout"));
        }
        pending.insert(
            state.clone(),
            Pending {
                id: id.into(),
                ends,
                client,
                resource: url.into(),
                verifier,
                expires: Instant::now() + WAIT,
            },
        );
        Ok(oauth::ConsentRequest {
            authorize,
            state,
            language: String::new(),
        })
    }
    fn finish(&self, state: &str, code: &str) -> Result<(), String> {
        let _gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(state)
            .ok_or_else(|| i18n::t("err.mcp.auth.noCode"))?;
        if pending.expires <= Instant::now() {
            return Err(i18n::t("err.mcp.auth.timeout"));
        }
        if code.is_empty() {
            return Err(i18n::t("err.mcp.auth.noCode"));
        }
        let auth = exchange(
            &pending.ends,
            &pending.client,
            &pending.resource,
            code,
            &pending.verifier,
        )?;
        let mut all = self.storage.load();
        all.insert(pending.id, auth);
        self.storage.save(&all)
    }
    fn cancel(&self, state: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(state);
    }
}
/* Discovery */

/// Discover authorization endpoints through the 401 resource metadata and authorization-server
/// metadata. Fall back to well-known metadata at the resource origin when advertised metadata is
/// unavailable.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub register: Option<String>,
    pub scopes: Option<String>,
}

pub fn discover(url: &str, challenge_header: Option<&str>) -> Result<Endpoints, String> {
    let client = http()?;
    let metadata = challenge_header
        .and_then(resource_metadata_url)
        .unwrap_or_else(|| well_known(url, "oauth-protected-resource"));

    // Use the resource metadata's authorization server, falling back to the resource host when it
    // serves both roles.
    let issuer = client
        .get(&metadata)
        .timeout(HTTP)
        .send()
        .ok()
        .and_then(|r| r.json::<Value>().ok())
        .and_then(|meta| {
            meta["authorization_servers"][0]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| origin(url));

    // Try OAuth and OpenID discovery metadata locations supported by the server.
    for probe in [
        well_known(&issuer, "oauth-authorization-server"),
        well_known(&issuer, "openid-configuration"),
    ] {
        let Some(meta) = client
            .get(&probe)
            .timeout(HTTP)
            .send()
            .ok()
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<Value>().ok())
        else {
            continue;
        };
        let (Some(authorize), Some(token)) = (
            meta["authorization_endpoint"].as_str(),
            meta["token_endpoint"].as_str(),
        ) else {
            continue;
        };
        return Ok(Endpoints {
            authorize: authorize.to_string(),
            token: token.to_string(),
            register: meta["registration_endpoint"].as_str().map(str::to_string),
            scopes: meta["scopes_supported"]
                .as_array()
                .map(|all| {
                    all.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty()),
        });
    }
    Err(i18n::ta("err.mcp.auth.noMetadata", &[("url", issuer)]))
}

/// Extract the resource_metadata parameter from WWW-Authenticate.
fn resource_metadata_url(header: &str) -> Option<String> {
    let at = header.find("resource_metadata=")? + "resource_metadata=".len();
    let rest = header[at..].trim_start_matches('"');
    let end = rest.find('"').unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Append the resource path after the well-known metadata name, preserving discovery for resources
/// below the origin root.
fn well_known(url: &str, name: &str) -> String {
    let base = origin(url);
    let path = url
        .strip_prefix(&base)
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();
    format!("{base}/.well-known/{name}{path}")
}

/// Extract the URL origin.
fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("https", url));
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}")
}

fn http() -> Result<reqwest::blocking::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::blocking::Client::builder()
        .timeout(HTTP)
        .build()
        .map_err(|e| i18n::ta("err.mcp.auth.http", &[("cause", e.to_string())]))
}

/// Register a client dynamically. Servers without registration support require separately
/// configured clients, which the UI explains.
fn register(ends: &Endpoints) -> Result<String, String> {
    let Some(endpoint) = &ends.register else {
        return Err(i18n::t("err.mcp.auth.noRegister"));
    };
    let body = serde_json::json!({
        "client_name": "Prometeu",
        "redirect_uris": [oauth::MCP_REDIRECT],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        // Use PKCE without a client secret because a desktop bundle cannot keep an embedded secret
        // private.
        "token_endpoint_auth_method": "none"
    });
    let reply: Value = http()?
        .post(endpoint)
        .json(&body)
        .send()
        .map_err(|e| i18n::ta("err.mcp.auth.unreachable", &[("cause", e.to_string())]))?
        .json()
        .map_err(|e| i18n::ta("err.mcp.auth.garbled", &[("cause", e.to_string())]))?;
    reply["client_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| {
            i18n::ta(
                "err.mcp.auth.noClient",
                &[("why", reply.to_string().chars().take(200).collect())],
            )
        })
}

fn exchange(
    ends: &Endpoints,
    client_id: &str,
    resource: &str,
    code: &str,
    verifier: &str,
) -> Result<Auth, String> {
    let got = token_request(
        &ends.token,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", oauth::MCP_REDIRECT),
            ("client_id", client_id),
            ("code_verifier", verifier),
            ("resource", resource),
        ],
    )?;
    Ok(Auth {
        client_id: client_id.to_string(),
        token_endpoint: ends.token.clone(),
        resource: resource.to_string(),
        access_token: got.0,
        refresh_token: got.1,
        expires_at: now() + got.2,
    })
}

fn renew(auth: &Auth) -> Result<Auth, String> {
    let refresh = auth
        .refresh_token
        .as_deref()
        .ok_or_else(|| i18n::t("err.mcp.auth.expired"))?;
    let got = token_request(
        &auth.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("client_id", &auth.client_id),
            ("resource", &auth.resource),
        ],
    )?;
    Ok(Auth {
        access_token: got.0,
        // Preserve the current refresh token if the server omits a replacement.
        refresh_token: got.1.or_else(|| auth.refresh_token.clone()),
        expires_at: now() + got.2,
        ..auth.clone()
    })
}

/// Token response with refresh credential and lifetime.
fn token_request(
    endpoint: &str,
    fields: &[(&str, &str)],
) -> Result<(String, Option<String>, u64), String> {
    let reply: Value = http()?
        .post(endpoint)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(fields))
        .send()
        .map_err(|e| i18n::ta("err.mcp.auth.unreachable", &[("cause", e.to_string())]))?
        .json()
        .map_err(|e| i18n::ta("err.mcp.auth.garbled", &[("cause", e.to_string())]))?;
    if let Some(error) = reply["error"].as_str() {
        let why = reply["error_description"].as_str().unwrap_or_default();
        return Err(i18n::ta(
            "err.mcp.auth.refused",
            &[("why", format!("{error} {why}").trim().to_string())],
        ));
    }
    let token = reply["access_token"]
        .as_str()
        .ok_or_else(|| i18n::t("err.mcp.auth.noToken"))?
        .to_string();
    Ok((
        token,
        reply["refresh_token"].as_str().map(str::to_string),
        reply["expires_in"].as_u64().unwrap_or(3600),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_metadata_in_the_header() {
        let header = r#"Bearer realm="OAuth", resource_metadata="https://x.dev/.well-known/oauth-protected-resource/mcp""#;
        assert_eq!(
            resource_metadata_url(header).as_deref(),
            Some("https://x.dev/.well-known/oauth-protected-resource/mcp")
        );
        assert!(resource_metadata_url(r#"Bearer realm="OAuth""#).is_none());
    }

    /// RFC 9728 places a nested resource path after the well-known metadata name.
    #[test]
    fn well_known_preserves_the_path() {
        assert_eq!(
            well_known("https://mcp.example.com/mcp", "oauth-protected-resource"),
            "https://mcp.example.com/.well-known/oauth-protected-resource/mcp"
        );
        assert_eq!(
            well_known("https://mcp.example.com", "oauth-authorization-server"),
            "https://mcp.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn origin_contains_only_scheme_and_host() {
        assert_eq!(origin("https://a.b/c/d?x=1"), "https://a.b");
        assert_eq!(origin("http://127.0.0.1:3000/mcp"), "http://127.0.0.1:3000");
    }

    /// Ignored integration test discovers endpoints and registers a client without opening browser
    /// consent: cargo test -- --ignored descobre.
    #[test]
    #[ignore]
    fn discovers_and_registers_with_a_real_server() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let ends = discover("https://mcp.apps.capim.tech/mcp", None).expect("descoberta");
        println!(
            "authorize={} token={} register={:?}",
            ends.authorize, ends.token, ends.register
        );
        assert!(ends.authorize.starts_with("https://"));
        let client = register(&ends).expect("registro dinâmico");
        println!("client_id={client}");
        assert!(!client.is_empty());
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };
    #[derive(Default)]
    struct Memory {
        values: Mutex<Store>,
        fail: bool,
    }
    impl AuthStorage for Memory {
        fn load(&self) -> Store {
            self.values.lock().unwrap().clone()
        }
        fn save(&self, values: &Store) -> Result<(), String> {
            if self.fail {
                return Err("disk full".into());
            }
            *self.values.lock().unwrap() = values.clone();
            Ok(())
        }
    }
    fn pending(token: String) -> Pending {
        Pending {
            id: "docs".into(),
            ends: Endpoints {
                authorize: String::new(),
                token,
                register: None,
                scopes: None,
            },
            client: "client".into(),
            resource: "https://example.invalid/mcp".into(),
            verifier: "private".into(),
            expires: Instant::now() + WAIT,
        }
    }
    #[test]
    fn failed_commit_never_publishes_a_login_and_completion_cannot_be_replayed() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let token = format!("http://{}/token", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    if key.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
            }
            assert!(length <= 8192);
            reader.read_exact(&mut vec![0; length]).unwrap();
            drop(reader);
            let reply = r#"{"access_token":"private-token","expires_in":3600}"#;
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",reply.len()).unwrap();
        });
        let storage = Arc::new(Memory {
            fail: true,
            ..Memory::default()
        });
        let service = McpAuthorization::new(storage.clone());
        service
            .pending
            .lock()
            .unwrap()
            .insert("state".into(), pending(token));
        assert_eq!(service.finish("state", "code"), Err("disk full".into()));
        assert!(service.logged_in().is_empty());
        assert!(service.finish("state", "code").is_err());
        worker.join().unwrap();
    }
    #[test]
    fn wrong_state_cancellation_expiry_and_logout_never_exchange_a_code() {
        let service = McpAuthorization::new(Arc::new(Memory::default()));
        service
            .pending
            .lock()
            .unwrap()
            .insert("state".into(), pending(String::new()));
        assert!(service.finish("wrong", "code").is_err());
        assert!(service.pending());
        assert!(service.pending.lock().unwrap().contains_key("state"));
        service.cancel("state");
        assert!(!service.pending());
        assert!(service.finish("state", "code").is_err());
        let mut expired = pending(String::new());
        expired.expires = Instant::now();
        service
            .pending
            .lock()
            .unwrap()
            .insert("expired".into(), expired);
        assert!(!service.pending());
        assert!(service.finish("expired", "code").is_err());
        service
            .pending
            .lock()
            .unwrap()
            .insert("other".into(), pending(String::new()));
        service.forget("docs").unwrap();
        assert!(service.finish("other", "code").is_err());
        assert!(service.logged_in().is_empty());
    }
}
