use crate::mcp::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Default)]
struct Host {
    servers: Vec<Server>,
    token: Option<String>,
    failure: Option<String>,
    calls: Mutex<Vec<String>>,
    writes: Mutex<Vec<String>>,
}
impl McpSources for Host {
    fn claude_servers(
        &self,
        session: &str,
        chosen: &[String],
        workdir: &Path,
    ) -> Result<Vec<Server>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("claude:{session}:{}:{chosen:?}", workdir.display()));
        Ok(self.servers.clone())
    }
    fn codex_servers(&self, session: &str, chosen: &[String]) -> Result<Vec<Server>, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("codex:{session}:{chosen:?}"));
        Ok(self.servers.clone())
    }
    fn bearer(&self, server: &str) -> Option<String> {
        self.calls.lock().unwrap().push(format!("bearer:{server}"));
        self.token.clone()
    }
}
impl McpFiles for Host {
    fn claude_config(&self, session: &str, body: &str) -> Result<PathBuf, String> {
        self.calls.lock().unwrap().push(format!("file:{session}"));
        self.write(body, "/private/session.json")
    }
    fn codex_environment(
        &self,
        session: &str,
        server: &str,
        body: &str,
    ) -> Result<PathBuf, String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("env:{session}:{server}"));
        self.write(body, "/private/session.env")
    }
}
impl Host {
    fn materializer(&self) -> McpMaterializer<'_> {
        McpMaterializer {
            sources: self,
            files: self,
        }
    }
    fn write(&self, body: &str, path: &str) -> Result<PathBuf, String> {
        match &self.failure {
            Some(error) => Err(error.clone()),
            None => {
                self.writes.lock().unwrap().push(body.to_string());
                Ok(path.into())
            }
        }
    }
}
fn server(id: &str, config: Value) -> Server {
    Server {
        id: id.into(),
        config,
        note: String::new(),
    }
}
fn table(raw: &str) -> toml::Value {
    toml::from_str(&format!("mcp_servers={raw}")).unwrap()
}

#[test]
fn defaults_have_no_effects_but_empty_selection_overrides_the_cli() {
    let host = Host::default();
    let tools = host.materializer();
    assert!(tools
        .claude_config("tab", None, Path::new("/work"))
        .unwrap()
        .is_none());
    assert!(tools.codex_config("tab", None).unwrap().is_none());
    assert!(host.calls.lock().unwrap().is_empty());
    assert_eq!(
        tools
            .claude_config("tab", Some(&[]), Path::new("/work"))
            .unwrap(),
        Some("/private/session.json".into())
    );
    assert_eq!(
        tools.codex_config("tab", Some(&[])).unwrap(),
        Some(("{}".into(), vec![]))
    );
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["claude:tab:/work:[]", "file:tab", "codex:tab:[]"]
    );
    assert_eq!(
        serde_json::from_str::<Value>(&host.writes.lock().unwrap()[0]).unwrap(),
        json!({"mcpServers": {}})
    );
}

#[test]
fn missing_selection_fails_before_writing_or_refreshing_credentials() {
    let host = Host::default();
    let chosen = ["deleted".into()];
    let expected = prometeu_core::error::with_args("err.mcp.missing", &[("id", "deleted".into())]);
    assert_eq!(
        host.materializer()
            .claude_config("tab", Some(&chosen), Path::new("/work")),
        Err(expected.clone())
    );
    assert_eq!(
        host.materializer().codex_config("tab", Some(&chosen)),
        Err(expected)
    );
    assert!(host.writes.lock().unwrap().is_empty());
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}

#[test]
fn claude_keeps_unknown_fields_and_oauth_out_of_the_catalog() {
    let host = Host {
        servers: vec![server(
            "api",
            json!({"url":"https://example.test", "headers":{"X-Key":"secret"}, "futureOption":42}),
        )],
        token: Some("oauth-secret".into()),
        ..Host::default()
    };
    host.materializer()
        .claude_config("tab", Some(&["api".into()]), Path::new("/work"))
        .unwrap();
    let config: Value = serde_json::from_str(&host.writes.lock().unwrap()[0]).unwrap();
    assert_eq!(
        config["mcpServers"]["api"]["headers"],
        json!({"X-Key":"secret", "Authorization":"Bearer oauth-secret"})
    );
    assert_eq!(config["mcpServers"]["api"]["futureOption"], 42);
    assert!(host.servers[0].config["headers"]
        .get("Authorization")
        .is_none());
}

#[test]
fn codex_remote_credentials_only_reach_environment_and_oauth_replaces_old_authorization() {
    let host = Host {
        servers: vec![server(
            "api.test",
            json!({"url":"https://example.test/path", "headers":{"authorization":"old-secret", "X-Key":"header-secret"}}),
        )],
        token: Some("oauth-secret".into()),
        ..Host::default()
    };
    let (raw, env) = host
        .materializer()
        .codex_config("tab", Some(&["api.test".into()]))
        .unwrap()
        .unwrap();
    let parsed = table(&raw);
    let headers = parsed["mcp_servers"]["api.test"]["env_http_headers"]
        .as_table()
        .unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(
        headers["Authorization"].as_str(),
        Some("PROMETEU_MCP_API_TEST_AUTHORIZATION")
    );
    assert!(env.contains(&(
        "PROMETEU_MCP_API_TEST_AUTHORIZATION".into(),
        "Bearer oauth-secret".into()
    )));
    assert!(env.contains(&("PROMETEU_MCP_API_TEST_X_KEY".into(), "header-secret".into())));
    assert!(!raw.contains("secret"));
    assert!(!env.iter().any(|(_, value)| value == "old-secret"));
    assert!(host.writes.lock().unwrap().is_empty());
}

#[test]
fn codex_stdio_secrets_use_the_private_writer_and_plain_commands_stay_direct() {
    let host = Host {
        servers: vec![
            server(
                "local",
                json!({"command":"runner", "args":["a\"b", "line\nbreak"], "env":{"API_KEY":"it's secret"}}),
            ),
            server("plain", json!({"command":"direct", "args":[]})),
        ],
        ..Host::default()
    };
    let (raw, env) = host
        .materializer()
        .codex_config("tab", Some(&["local".into(), "plain".into()]))
        .unwrap()
        .unwrap();
    let parsed = table(&raw);
    assert_eq!(
        parsed["mcp_servers"]["local"]["command"].as_str(),
        Some("/bin/sh")
    );
    let args = parsed["mcp_servers"]["local"]["args"].as_array().unwrap();
    assert_eq!(
        args[1].as_str(),
        Some(". '/private/session.env' && exec \"$@\"")
    );
    assert_eq!(args[3].as_str(), Some("runner"));
    assert_eq!(args[4].as_str(), Some("a\"b"));
    assert_eq!(args[5].as_str(), Some("line\nbreak"));
    assert_eq!(
        parsed["mcp_servers"]["plain"]["command"].as_str(),
        Some("direct")
    );
    assert_eq!(
        *host.writes.lock().unwrap(),
        ["export API_KEY='it'\\''s secret'\n"]
    );
    assert!(!raw.contains("secret"));
    assert!(env.is_empty());
    assert_eq!(
        *host.calls.lock().unwrap(),
        ["codex:tab:[\"local\", \"plain\"]", "env:tab:local"]
    );
}

#[test]
fn private_write_errors_propagate_without_reencoding() {
    let error = prometeu_core::error::with_args("err.mcp.session", &[("cause", "denied".into())]);
    let host = Host {
        failure: Some(error.clone()),
        servers: vec![server(
            "local",
            json!({"command":"runner", "env":{"TOKEN":"secret"}}),
        )],
        ..Host::default()
    };
    assert_eq!(
        host.materializer()
            .claude_config("tab", Some(&[]), Path::new("/work")),
        Err(error.clone())
    );
    assert_eq!(
        host.materializer()
            .codex_config("tab", Some(&["local".into()])),
        Err(error)
    );
    assert!(host.writes.lock().unwrap().is_empty());
}
