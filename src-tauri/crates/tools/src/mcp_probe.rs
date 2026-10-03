//! Shared MCP inspection through native HTTP and an injected bounded subprocess query.
use crate::{
    mcp::Server,
    mcp_auth::{self, Authorization},
};
mod i18n {
    pub use prometeu_core::error::code as t;
}
use prometeu_core::command::{QueryLauncher, QueryPolicy};
use serde_json::{json, Value};
use std::{process::Command, sync::Arc, time::Duration};
/// Inspect the server directly to validate its configuration without adding an agent process to the
/// diagnostic path.
#[derive(serde::Serialize, Default)]
pub struct Probe {
    /// The connection succeeded and tools were listed.
    pub ok: bool,
    /// HTTP 401 distinguishes a server requiring login from a failed connection.
    pub auth: bool,
    pub tools: usize,
    /// The returned server identity confirms which server answered.
    pub name: String,
    /// Raw system or server error details; the frontend supplies the surrounding localized message.
    pub detail: String,
}

/// Record each inspection stage so the UI can distinguish startup, handshake, tools, and OAuth
/// discovery failures. Keys are translated codes; notes hold observed data; details preserve raw
/// errors.
#[derive(serde::Serialize)]
pub struct Step {
    pub key: &'static str,
    pub ok: bool,
    pub note: String,
    pub detail: String,
}

impl Step {
    fn ok(key: &'static str, note: impl Into<String>) -> Self {
        Step {
            key,
            ok: true,
            note: note.into(),
            detail: String::new(),
        }
    }

    fn bad(key: &'static str, detail: impl Into<String>) -> Self {
        Step {
            key,
            ok: false,
            note: String::new(),
            detail: detail.into(),
        }
    }
}

/// Return all inspection steps and a summary used to offer saving, login, or configuration repair.
#[derive(serde::Serialize, Default)]
pub struct Check {
    pub steps: Vec<Step>,
    pub probe: Probe,
}

/// Results of a remote-server exchange.
struct Http {
    probe: Probe,
    /// Preserve WWW-Authenticate from HTTP 401 to start OAuth discovery.
    challenge: Option<String>,
    steps: Vec<Step>,
}

/// Bound inspection time while allowing initial npx downloads and remote requests.
const PROBE_WAIT: Duration = Duration::from_secs(25);

/// Advertise a fixed MCP revision; the handshake negotiates an older revision when needed.
const PROTOCOL: &str = "2025-06-18";

fn hello() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": { "name": "Prometeu", "version": env!("CARGO_PKG_VERSION") }
        }
    })
}

pub struct Inspection {
    pub auth: Arc<dyn Authorization>,
    pub query: Arc<dyn QueryLauncher<Command>>,
}
impl Inspection {
    pub fn begin(&self, server: &Server) -> Result<prometeu_oauth::ConsentRequest, String> {
        let url = server.config["url"]
            .as_str()
            .ok_or_else(|| i18n::t("err.mcp.auth.notRemote"))?;
        let response = probe_http(url, &server.config, None);
        self.auth
            .begin(&server.id, url, response.challenge.as_deref())
    }
    /// For servers requiring login, also inspect OAuth endpoint discovery and dynamic client
    /// registration support before offering a browser login that cannot succeed.
    pub fn check(&self, server: &Server) -> Check {
        let Some(url) = server.config.get("url").and_then(Value::as_str) else {
            let (probe, steps) = probe_stdio(&server.config, self.query.as_ref());
            return Check { steps, probe };
        };
        // Use stored authentication during inspection so a successful login does not keep reporting
        // that login is required.
        let mut http = probe_http(url, &server.config, self.auth.bearer(&server.id).as_deref());
        if http.probe.auth {
            match mcp_auth::discover(url, http.challenge.as_deref()) {
                Ok(ends) => {
                    http.steps.push(Step::ok("oauth", String::new()));
                    http.steps.push(match ends.register {
                        Some(_) => Step::ok("client", String::new()),
                        None => Step::bad("client", i18n::t("err.mcp.auth.noRegister")),
                    });
                }
                Err(why) => http.steps.push(Step::bad("oauth", why)),
            }
        }
        Check {
            steps: http.steps,
            probe: http.probe,
        }
    }
}
/// POST initialize, then tools/list with the returned session. Accept both JSON and event-stream
/// response envelopes.
fn probe_http(url: &str, config: &Value, token: Option<&str>) -> Http {
    /// The initial connection failed before a protocol exchange began.
    fn broke(key: &'static str, detail: String) -> Http {
        Http {
            probe: Probe {
                detail: detail.clone(),
                ..Probe::default()
            },
            challenge: None,
            steps: vec![Step::bad(key, detail)],
        }
    }

    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = match reqwest::blocking::Client::builder()
        .timeout(PROBE_WAIT)
        .build()
    {
        Ok(client) => client,
        Err(e) => return broke("connect", e.to_string()),
    };
    let headers = config
        .get("headers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let post = |body: &Value, session: Option<&str>| {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            // Streamable HTTP permits either JSON or event-stream responses.
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL);
        for (key, value) in &headers {
            if let Some(value) = value.as_str() {
                req = req.header(key.as_str(), value);
            }
        }
        if let Some(token) = token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(id) = session {
            req = req.header("Mcp-Session-Id", id);
        }
        req.json(body).send()
    };

    let first = match post(&hello(), None) {
        Ok(response) => response,
        Err(e) => return broke("connect", e.to_string()),
    };
    let code = first.status().as_u16().to_string();
    // Treat HTTP 401 as an authentication requirement rather than a malformed server configuration.
    if first.status() == reqwest::StatusCode::UNAUTHORIZED {
        let challenge = first
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        return Http {
            probe: Probe {
                auth: true,
                ..Probe::default()
            },
            challenge,
            steps: vec![Step::ok("connect", code)],
        };
    }
    let session = first
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if !first.status().is_success() {
        let status = first.status();
        return broke(
            "connect",
            format!("{status} {}", short(&first.text().unwrap_or_default())),
        );
    }
    let mut steps = vec![Step::ok("connect", code)];
    let hello_body = first.text().unwrap_or_default();
    let Some(result) = frame(&hello_body) else {
        let detail = short(&hello_body);
        steps.push(Step::bad("handshake", detail.clone()));
        return Http {
            probe: Probe {
                detail,
                ..Probe::default()
            },
            challenge: None,
            steps,
        };
    };
    let name = result["result"]["serverInfo"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    steps.push(Step::ok("handshake", name.clone()));

    // Send initialized without waiting for a response; servers may require it before later
    // requests.
    let _ = post(
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        session.as_deref(),
    );
    let listed = post(
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        session.as_deref(),
    )
    .and_then(|r| r.text());
    let tools = listed
        .as_ref()
        .ok()
        .and_then(|body| frame(body))
        .and_then(|value| value["result"]["tools"].as_array().map(Vec::len))
        .unwrap_or(0);
    steps.push(tools_step(listed.err().map(|e| e.to_string()), tools));
    Http {
        probe: Probe {
            ok: true,
            auth: false,
            tools,
            name,
            detail: String::new(),
        },
        challenge: None,
        steps,
    }
}

/// A successful handshake without any tools is not a usable inspection result.
fn tools_step(failed: Option<String>, tools: usize) -> Step {
    match (failed, tools) {
        (Some(why), _) => Step::bad("tools", why),
        (None, 0) => Step::bad("tools", String::new()),
        (None, n) => Step::ok("tools", n.to_string()),
    }
}

/// Unwrap plain JSON or the first event-stream data line containing the request result.
fn frame(body: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return Some(value);
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
}

fn probe_stdio(config: &Value, launcher: &dyn QueryLauncher<Command>) -> (Probe, Vec<Step>) {
    let mut probe = Probe::default();
    let Some(command) = config["command"].as_str() else {
        probe.detail = "Missing command or URL".into();
        return (probe, vec![Step::bad("spawn", "Missing command or URL")]);
    };
    let mut cmd = Command::new(command);
    if let Some(args) = config["args"].as_array() {
        cmd.args(args.iter().filter_map(Value::as_str));
    }
    if let Some(env) = config["env"].as_object() {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                cmd.env(key, value);
            }
        }
    }
    let mut child = match launcher.launch(
        &mut cmd,
        QueryPolicy {
            timeout: PROBE_WAIT,
            max_output: 1024 * 1024,
        },
    ) {
        Ok(child) => child,
        Err(error) => {
            let detail = format!("{error:?}");
            probe.detail = detail.clone();
            return (probe, vec![Step::bad("spawn", detail)]);
        }
    };
    let mut answered = false;
    let result = (|| {
        child.send(format!("{}\n", hello()).as_bytes())?;
        // Wait for initialize before sending initialized and tools/list.
        while let Some(line) = child.next()? {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["id"] != 1 {
                continue;
            }
            if let Some(error) = value.get("error") {
                probe.detail = short(&error.to_string());
                return Ok(());
            }
            if !value["result"].is_object() {
                continue;
            }
            probe.name = value["result"]["serverInfo"]["name"]
                .as_str()
                .unwrap_or_default()
                .into();
            probe.ok = true;
            child.send(
                format!(
                    "{}\n{}\n",
                    json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
                )
                .as_bytes(),
            )?;
            break;
        }
        while let Some(line) = child.next()? {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value["id"] != 2 {
                continue;
            }
            if let Some(tools) = value["result"]["tools"].as_array() {
                probe.tools = tools.len();
                answered = true;
                break;
            }
            if let Some(error) = value.get("error") {
                probe.detail = short(&error.to_string());
                break;
            }
        }
        Ok::<_, prometeu_core::command::CommandError>(())
    })();
    if let Err(error) = result {
        probe.detail = format!("{error:?}");
    }
    // The injected query owns process-group termination, drain bounds and reaping on drop.
    drop(child);
    let mut steps = vec![Step::ok("spawn", String::new())];
    if !probe.ok {
        steps.push(Step::bad("handshake", probe.detail.clone()));
    } else {
        steps.push(Step::ok("handshake", probe.name.clone()));
        steps.push(tools_step(
            (!answered).then(|| probe.detail.clone()),
            probe.tools,
        ));
    }
    (probe, steps)
}
/// Truncate diagnostic text to fit a single UI line.
fn short(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or_default().trim();
    if line.chars().count() > 200 {
        line.chars().take(199).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// JSON and event-stream envelopes expose the same server identity object.
    #[test]
    fn unwraps_json_and_event_streams() {
        let plain = frame(r#"{"result":{"serverInfo":{"name":"x"}}}"#).expect("json");
        assert_eq!(plain["result"]["serverInfo"]["name"], "x");
        let stream =
            frame("event: message\ndata: {\"result\":{\"serverInfo\":{\"name\":\"y\"}}}\n\n")
                .expect("fluxo");
        assert_eq!(stream["result"]["serverInfo"]["name"], "y");
        assert!(frame("not JSON").is_none());
    }

    #[test]
    fn errors_fit_on_one_line() {
        assert_eq!(short("  failed\nmore details  "), "failed");
        assert_eq!(short(&"a".repeat(300)).chars().count(), 200);
    }
}
