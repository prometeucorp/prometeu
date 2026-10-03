//! Provider-specific MCP encoding with injected catalogs, OAuth and private persistence.
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// Store each server's full mcpServers configuration object so new CLI-supported fields do not
/// require an app release.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct Server {
    /// The server name is its mcpServers key and the prefix of exported tool names.
    pub id: String,
    pub config: Value,
    /// Free-form source or purpose shown below the server name.
    #[serde(default)]
    pub note: String,
}

/// Read-only local hub; startup materialization and OAuth remain separate ports.
pub trait McpCatalog: Send + Sync {
    fn load(&self) -> Vec<Server>;
}
pub struct FileMcpCatalog(pub PathBuf);
impl McpCatalog for FileMcpCatalog {
    fn load(&self) -> Vec<Server> {
        std::fs::read_to_string(&self.0)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}

/// Sources resolve the effective catalog and materialize session-owned built-ins on demand.
/// Bearer lookup may refresh OAuth credentials, but must not persist them in the catalog.
pub trait McpSources: Send + Sync {
    fn claude_servers(
        &self,
        session: &str,
        chosen: &[String],
        workdir: &Path,
    ) -> Result<Vec<Server>, String>;
    fn codex_servers(&self, session: &str, chosen: &[String]) -> Result<Vec<Server>, String>;
    fn bearer(&self, server: &str) -> Option<String>;
}

/// Hosts own paths and private atomic writes (0700 directories, 0600 files).
/// Failures are already encoded application errors and must stop preparation.
pub trait McpFiles: Send + Sync {
    fn claude_config(&self, session: &str, body: &str) -> Result<PathBuf, String>;
    fn codex_environment(&self, session: &str, server: &str, body: &str)
        -> Result<PathBuf, String>;
}

/// The table references credentials through environment variables or private files, never argv.
pub type CodexMcp = (String, Vec<(String, String)>);

pub struct McpMaterializer<'a> {
    pub sources: &'a dyn McpSources,
    pub files: &'a dyn McpFiles,
}
impl McpMaterializer<'_> {
    pub fn claude_config(
        &self,
        id: &str,
        chosen: Option<&[String]>,
        workdir: &Path,
    ) -> Result<Option<PathBuf>, String> {
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        // Refresh OAuth tokens while materializing session configuration, without persisting them in
        // the registry.
        let body = serde_json::to_string_pretty(&config_body(
            &self.sources.claude_servers(id, chosen, workdir)?,
            chosen,
            |id| self.sources.bearer(id),
        )?)
        .map_err(|e| e.to_string())?;
        self.files.claude_config(id, &body).map(Some)
    }

    pub fn codex_config(
        &self,
        id: &str,
        chosen: Option<&[String]>,
    ) -> Result<Option<CodexMcp>, String> {
        let Some(chosen) = chosen else {
            return Ok(None);
        };
        let hub = self.sources.codex_servers(id, chosen)?;
        let mut env: Vec<(String, String)> = Vec::new();
        let mut entries: Vec<String> = Vec::new();
        for name in chosen {
            let Some(server) = hub.iter().find(|s| &s.id == name) else {
                return Err(prometeu_core::error::with_args(
                    "err.mcp.missing",
                    &[("id", name.clone())],
                ));
            };
            let entry = match server.config.get("url").and_then(Value::as_str) {
                Some(url) => remote_entry(server, url, &mut env, self.sources),
                None => local_entry(id, server, self.files)?,
            };
            entries.push(format!("{}={entry}", toml_key(&server.id)));
        }
        Ok(Some((format!("{{{}}}", entries.join(",")), env)))
    }
}

/// Materialize every chosen server. Resolution already filtered the chosen ids against the
/// universe, so a miss here means the hub or the CLI configuration changed between resolve and
/// spawn; starting without a requested server is not a valid fallback
/// (docs/contracts/agent-runtime.md), so the spawn fails with the missing id. Inject token lookup
/// so configuration tests do not require disk state.
pub fn config_body(
    hub: &[Server],
    chosen: &[String],
    bearer: impl Fn(&str) -> Option<String>,
) -> Result<Value, String> {
    let mut servers = Map::new();
    for name in chosen {
        let Some(server) = hub.iter().find(|s| &s.id == name) else {
            return Err(prometeu_core::error::with_args(
                "err.mcp.missing",
                &[("id", name.clone())],
            ));
        };
        let mut config = server.config.clone();
        // Inject the OAuth bearer header so the CLI can use an authenticated server without owning
        // the login flow.
        if let Some(token) = bearer(&server.id) {
            if let Some(object) = config.as_object_mut() {
                let mut headers = object
                    .get("headers")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                headers.insert("Authorization".into(), json!(format!("Bearer {token}")));
                object.insert("headers".into(), Value::Object(headers));
            }
        }
        servers.insert(server.id.clone(), config);
    }
    Ok(json!({ "mcpServers": servers }))
}

/// Sanitize source labels into suffixes valid in server and tool names.
pub fn slug(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    cleaned.trim_matches('-').replace("--", "-")
}

impl Server {
    fn args(&self) -> Vec<String> {
        self.config
            .get("args")
            .and_then(Value::as_array)
            .map(|args| {
                args.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn env_pairs(&self) -> Vec<(String, String)> {
        pairs(self.config.get("env"))
    }
}

fn pairs(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Generate portable uppercase environment-variable names for header values.
fn env_var_name(server: &str, header: &str) -> String {
    format!(
        "PROMETEU_MCP_{}_{}",
        slug(server).to_uppercase().replace('-', "_"),
        slug(header).to_uppercase().replace('-', "_")
    )
}

fn remote_entry(
    server: &Server,
    url: &str,
    env: &mut Vec<(String, String)>,
    sources: &dyn McpSources,
) -> String {
    let mut headers = pairs(server.config.get("headers"));
    if let Some(token) = sources.bearer(&server.id) {
        headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
        headers.push(("Authorization".into(), format!("Bearer {token}")));
    }
    let mut fields = vec![format!("url={}", toml_str(url))];
    if !headers.is_empty() {
        let mapped: Vec<String> = headers
            .into_iter()
            .map(|(header, value)| {
                let var = env_var_name(&server.id, &header);
                let line = format!("{}={}", toml_key(&header), toml_str(&var));
                env.push((var, value));
                line
            })
            .collect();
        fields.push(format!("env_http_headers={{{}}}", mapped.join(",")));
    }
    format!("{{{}}}", fields.join(","))
}

/// Encode a Unix stdio entry, delegating secret persistence to the host.
pub fn local_entry(id: &str, server: &Server, files: &dyn McpFiles) -> Result<String, String> {
    let command = server
        .config
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let args = server.args();
    let vars = server.env_pairs();
    let (command, args) = if vars.is_empty() {
        (command.to_string(), args)
    } else {
        let body = vars
            .iter()
            .map(|(k, v)| format!("export {k}='{}'\n", v.replace('\'', "'\\''")))
            .collect::<String>();
        let path = files.codex_environment(id, &server.id, &body)?;
        // Pass the original command and arguments through "$@" without additional interpolation;
        // exec replaces the wrapper shell.
        let script = format!(". '{}' && exec \"$@\"", path.display());
        let mut wrapped = vec![
            "-c".to_string(),
            script,
            "prometeu-mcp".to_string(),
            command.to_string(),
        ];
        wrapped.extend(args);
        ("/bin/sh".to_string(), wrapped)
    };
    let args = args
        .iter()
        .map(|a| toml_str(a))
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!("{{command={},args=[{args}]}}", toml_str(&command)))
}

/// Quote TOML strings for configuration values without introducing another dependency.
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Always quote TOML keys so server and header names containing dots or hyphens remain literal.
fn toml_key(s: &str) -> String {
    toml_str(s)
}

/// Shared local registration validation. Reserved entries belong to their execution host.
pub fn validate(mut server: Server, reserved: &[&str]) -> Result<Server, String> {
    server.id = server.id.trim().into();
    if reserved.contains(&server.id.as_str()) {
        return Err(prometeu_core::error::code("err.mcp.builtin"));
    }
    if server.id.is_empty() {
        return Err(prometeu_core::error::code("err.mcp.noName"));
    }
    if !server.config.is_object() {
        return Err(prometeu_core::error::code("err.mcp.badConfig"));
    }
    Ok(server)
}
pub fn register(hub: &mut Vec<Server>, server: Server) {
    match hub.iter_mut().find(|s| s.id == server.id) {
        Some(existing) => *existing = server,
        None => hub.push(server),
    }
    hub.sort_by_key(|s| s.id.to_lowercase());
}
pub trait McpLibrary: McpCatalog {
    fn save(&self, server: Server) -> Result<Vec<Server>, String>;
    fn remove(&self, id: &str) -> Result<Vec<Server>, String>;
}
pub struct LocalMcpLibrary {
    pub path: PathBuf,
    pub files: std::sync::Arc<dyn crate::packages::PackageFiles>,
    pub reserved: Vec<String>,
}
impl McpCatalog for LocalMcpLibrary {
    fn load(&self) -> Vec<Server> {
        FileMcpCatalog(self.path.clone()).load()
    }
}
impl LocalMcpLibrary {
    fn store(&self, hub: &[Server]) -> Result<(), String> {
        self.files
            .write_private(
                &self.path,
                &serde_json::to_string_pretty(hub).map_err(|e| e.to_string())?,
            )
            .map_err(|cause| prometeu_core::error::with_args("err.mcp.save", &[("cause", cause)]))
    }
}
impl McpLibrary for LocalMcpLibrary {
    fn save(&self, server: Server) -> Result<Vec<Server>, String> {
        let server = validate(
            server,
            &self.reserved.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        let mut hub = self.load();
        register(&mut hub, server);
        self.store(&hub)?;
        Ok(hub)
    }
    fn remove(&self, id: &str) -> Result<Vec<Server>, String> {
        if self.reserved.iter().any(|reserved| reserved == id) {
            return Err(prometeu_core::error::code("err.mcp.builtin"));
        }
        let mut hub = self.load();
        hub.retain(|server| server.id != id);
        self.store(&hub)?;
        Ok(hub)
    }
}
