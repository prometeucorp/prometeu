//! The MCP hub stores known servers and workspace selections. Explicit selections restrict
//! available tools, reducing context cost and unnecessary access; None preserves CLI defaults.
//! Claude receives a private 0600 config file with strict selection, while Codex receives a
//! configuration override and protected secrets. Never place secrets in process arguments. Discover
//! existing user/project CLI configuration for read-only import; mcp_auth owns OAuth and token
//! refresh.

use crate::i18n;
use crate::mcp_auth;
use crate::paths;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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

/// Persist the registry privately because configurations may contain API keys.
fn hub_path() -> PathBuf {
    paths::root().join("mcp.json")
}

/// Use one generated config file per tab so simultaneous launches cannot overwrite each other's
/// input.
fn session_path(id: &str) -> PathBuf {
    paths::root().join("mcp").join(format!("{id}.json"))
}

/// Private environment file for a Codex stdio server; see codex_config.
fn codex_env_path(id: &str, server: &str) -> PathBuf {
    paths::root()
        .join("mcp")
        .join(format!("{id}.{}.env", slug(server)))
}

pub fn load() -> Vec<Server> {
    std::fs::read_to_string(hub_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Server>>(&raw).ok())
        .unwrap_or_default()
}

/// Built-ins are virtual catalog entries, never copied into the mutable or cloud registry.
pub fn available() -> Vec<Server> {
    let mut servers = load();
    servers.retain(|s| s.id != crate::embedded_mcp::ID);
    servers.insert(0, crate::embedded_mcp::builtin());
    servers
}

fn for_session(
    mut servers: Vec<Server>,
    id: &str,
    chosen: &[String],
) -> Result<Vec<Server>, String> {
    if chosen.iter().any(|name| name == crate::embedded_mcp::ID) {
        let server = crate::embedded_mcp::materialize(id)?;
        servers.retain(|s| s.id != server.id);
        servers.push(server);
    }
    Ok(servers)
}

pub(crate) fn store(servers: &[Server]) -> Result<(), String> {
    let body = serde_json::to_string_pretty(servers).map_err(|e| e.to_string())?;
    paths::write_private(&hub_path(), &body)
        .map_err(|cause| i18n::ta("err.mcp.save", &[("cause", cause)]))
}

/// Expose the registry to settings and selectors.
#[tauri::command(async)]
pub fn mcp_hub() -> Vec<Server> {
    let _sync = crate::catalog::guard();
    available()
}

/// Save or replace a server by name, which is also its tool-prefix identity within a session.
#[tauri::command(async)]
pub fn mcp_save(
    app: tauri::AppHandle,
    server: Server,
    revision: Option<u64>,
) -> Result<Vec<Server>, String> {
    let _sync = crate::catalog::guard();
    let id = server.id.trim().to_string();
    if id == crate::embedded_mcp::ID {
        return Err(i18n::t("err.mcp.builtin"));
    }
    if id.is_empty() {
        return Err(i18n::t("err.mcp.noName"));
    }
    if !server.config.is_object() {
        return Err(i18n::t("err.mcp.badConfig"));
    }
    let server = Server { id, ..server };
    crate::catalog::save_mcp(&app, &server, revision)?;
    let mut servers = load();
    match servers.iter_mut().find(|s| s.id == server.id) {
        Some(old) => *old = server,
        None => servers.push(server),
    }
    servers.sort_by_key(|s| s.id.to_lowercase());
    store(&servers)?;
    Ok(available())
}

#[tauri::command(async)]
pub fn mcp_remove(app: tauri::AppHandle, id: String) -> Result<Vec<Server>, String> {
    let _sync = crate::catalog::guard();
    if id == crate::embedded_mcp::ID {
        return Err(i18n::t("err.mcp.builtin"));
    }
    crate::catalog::remove_shared(&app, "mcp", &id)?;
    let mut servers = load();
    servers.retain(|s| s.id != id);
    store(&servers)?;
    Ok(available())
}

/// Discover importable CLI configuration absent from the hub without modifying user files.
#[tauri::command]
pub fn mcp_found() -> Vec<Server> {
    let known = available();
    let mut found: Vec<Server> = Vec::new();
    for server in from_claude_json().into_iter().chain(from_repo_files()) {
        // Deduplicate identical named configurations across registered entries and discovered
        // projects.
        if known.iter().any(|s| s.id == server.id)
            || found
                .iter()
                .any(|s| s.id == server.id && s.config == server.config)
        {
            continue;
        }
        // Keep different configurations with the same original name by assigning distinct import
        // names.
        let clash = found.iter().any(|s| s.id == server.id);
        let id = match (clash, server.note.trim()) {
            (true, origin) if !origin.is_empty() => format!("{}-{}", server.id, slug(origin)),
            (true, _) => format!("{}-2", server.id),
            _ => server.id.clone(),
        };
        found.push(Server { id, ..server });
    }
    found
}

/// Sanitize source labels into suffixes valid in server and tool names.
fn slug(text: &str) -> String {
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

/* Account connectors */

/// The note that marks a connector's origin in the picker, mirroring the CLI's own label.
const CONNECTOR_NOTE: &str = "claude.ai";
/// The connector list changes when the person edits it on claude.ai, not while a session runs, so a
/// few minutes of staleness costs nothing and keeps the picker and the spawn off the network.
const CONNECTOR_TTL: Duration = Duration::from_secs(300);
/// The account API this endpoint requires; see ADR 0063.
const CONNECTOR_BETA: &str = "mcp-servers-2025-12-04";

/// A list belongs to the login that produced it: the connectors of one account say nothing about
/// another, and a new login on the same account bumps its revision.
struct Cached {
    at: Instant,
    servers: Vec<Server>,
}

fn connector_cache() -> &'static Mutex<HashMap<(String, u64), Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, u64), Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_connectors(key: &(String, u64)) -> Option<(Instant, Vec<Server>)> {
    crate::lock::lock(connector_cache())
        .get(key)
        .map(|cached| (cached.at, cached.servers.clone()))
}

fn remember_connectors(key: (String, u64), servers: Vec<Server>) {
    crate::lock::lock(connector_cache()).insert(
        key,
        Cached {
            at: Instant::now(),
            servers,
        },
    );
}

/// The connectors of the person's Claude account (ADR 0063). Claude Code keeps them in no
/// configuration file: it fetches them from the account at every start and loads them under its own
/// `claudeai` scope. Discovery has to ask the same endpoint, or the picker would hide what the CLI
/// shows and a strict spawn would drop every connector without saying so.
///
/// `None` means the list of a logged-in account is unknown — credential read or fetch failed, with
/// nothing cached for that login — which is not the same as an account with no connectors: the
/// picker degrades to the file base, while a spawn that inherits it refuses to start rather than
/// drop connectors in silence. A failure after a successful fetch keeps the last known list.
pub fn connectors() -> Option<Vec<Server>> {
    // Without an account, the CLI loads no connectors.
    let Ok(profile) = crate::accounts::active(crate::state::ProviderId::Claude) else {
        return Some(Vec::new());
    };
    let key = (profile.id.clone(), profile.revision);
    let cached = cached_connectors(&key);
    // No credential is a confirmed empty account. A read or parse error leaves its list unknown.
    let token = match crate::usage::claude_token(&profile) {
        Ok(Some(token)) => token,
        Ok(None) => return Some(Vec::new()),
        Err(()) => return cached.map(|(_, servers)| servers),
    };
    if let Some((at, servers)) = &cached {
        if at.elapsed() < CONNECTOR_TTL {
            return Some(servers.clone());
        }
    }
    let Some(fetched) = fetch_connectors(&token).map(|body| connectors_from(&body)) else {
        return cached.map(|(_, servers)| servers);
    };
    remember_connectors(key, fetched.clone());
    Some(fetched)
}

/// Warm the cache off the interface thread so the first picker or spawn finds the list ready.
pub fn warm_connectors() {
    std::thread::spawn(|| {
        connectors();
    });
}

/// Ask the account API with the active Claude login's own credential, the same one the CLI uses.
/// The failure is silent here and answered by the caller, which knows whether an absent list may be
/// tolerated.
fn fetch_connectors(token: &str) -> Option<Value> {
    let base = std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.anthropic.com".into());
    reqwest::blocking::Client::new()
        .get(format!(
            "{}/v1/mcp_servers?limit=1000",
            base.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", CONNECTOR_BETA)
        .timeout(Duration::from_secs(8))
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .ok()
}

/// Name each connector the way Claude Code does — `claude.ai <display name>`, with a numeric suffix
/// on a repeated name — so a persisted layer keeps referring to the same server, and materialize the
/// `claudeai-proxy` entry the CLI itself would create, which a strict configuration accepts as a
/// dynamic server. Pure, so the naming and the entry shape are tested without the network.
fn connectors_from(body: &Value) -> Vec<Server> {
    let mut out: Vec<Server> = Vec::new();
    for item in body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(url), Some(name)) = (
            item.get("id").and_then(Value::as_str),
            item.get("url").and_then(Value::as_str),
            item.get("display_name").and_then(Value::as_str),
        ) else {
            continue;
        };
        let mut server_id = format!("{CONNECTOR_NOTE} {name}");
        let mut repeat = 1;
        while out.iter().any(|s| s.id == server_id) {
            repeat += 1;
            server_id = format!("{CONNECTOR_NOTE} {name} ({repeat})");
        }
        out.push(Server {
            id: server_id,
            config: json!({ "type": "claudeai-proxy", "url": url, "id": id }),
            note: CONNECTOR_NOTE.into(),
        });
    }
    out
}

/// The servers the CLI itself loads for one working directory (ADR 0046): the user-scope
/// `mcpServers` of `~/.claude.json`, the project-scope entry keyed by the directory, the
/// directory's `.mcp.json` plus every ancestor directory's, as the CLI walks the tree upward from
/// the working directory (ADR 0045), and the account connectors of the active login (ADR 0063).
/// They form the inherited base of the mcp axis, visible in the picker without importing. IDs keep
/// their original names; the first occurrence of a repeated name wins: local scope precedes project
/// scope, then user scope, then the account. Within project scope the nearest repository file
/// precedes its ancestors (ADR 0047).
pub fn inherited(workdir: &Path) -> Vec<Server> {
    let claude = read_json(&crate::claude::config_file(&crate::claude::user_home()));
    let mut files: Vec<(String, Value)> = Vec::new();
    let mut dir = Some(workdir);
    while let Some(current) = dir {
        if let Some(value) = read_json(&current.join(".mcp.json")) {
            let origin = current
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| current.to_string_lossy().into_owned());
            files.push((origin, value));
        }
        dir = current.parent();
    }
    // The picker and the button gating survive an unknown account: they show the file base, and the
    // spawn is the one that refuses to materialize a selection without the connectors.
    inherited_from(
        claude.as_ref(),
        workdir,
        &files,
        &connectors().unwrap_or_default(),
    )
}

/// The pure core of `inherited`, injectable so tests need no home directory, file tree or account.
/// `files` holds the repository `.mcp.json` values nearest first; the note carries the origin: empty
/// for user scope, the directory name for a repository, `claude.ai` for an account connector,
/// mirroring `from_claude_json` and `connectors_from`.
fn inherited_from(
    claude: Option<&Value>,
    workdir: &Path,
    files: &[(String, Value)],
    connectors: &[Server],
) -> Vec<Server> {
    let origin = workdir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| workdir.to_string_lossy().into_owned());
    let mut out: Vec<Server> = Vec::new();
    if let Some(root) = claude {
        // Claude Code keys projects by the resolved working directory; try the given path and its
        // canonical form so a symlinked worktree still matches.
        let projects = root.get("projects").and_then(Value::as_object);
        let key = workdir.to_string_lossy().into_owned();
        let canonical = workdir
            .canonicalize()
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let project = projects.and_then(|p| {
            p.get(&key)
                .or_else(|| canonical.as_deref().and_then(|c| p.get(c)))
        });
        if let Some(project) = project {
            out.extend(servers_in(project, &origin));
        }
    }
    for (file_origin, file) in files {
        out.extend(servers_in(file, file_origin));
    }
    if let Some(root) = claude {
        out.extend(servers_in(root, ""));
    }
    // The account comes last: a file the person controls shadows a connector of the same name, the
    // same precedence the local scopes already have over the user scope.
    out.extend(connectors.iter().cloned());
    let mut unique: Vec<Server> = Vec::new();
    for server in out {
        if !unique.iter().any(|s| s.id == server.id) {
            unique.push(server);
        }
    }
    unique
}

/// The mcp universe of one working directory: hub servers plus the CLI-inherited ones (ADR 0046).
/// A hub entry wins an ID clash, so an imported server stays Prometeu-managed with its own config.
pub fn universe(workdir: &Path) -> Vec<Server> {
    merge_universe(available(), inherited(workdir))
}

fn merge_universe(hub: Vec<Server>, inherited: Vec<Server>) -> Vec<Server> {
    let mut all = hub;
    for server in inherited {
        if !all.iter().any(|s| s.id == server.id) {
            all.push(server);
        }
    }
    all
}

/// The CLI-inherited servers absent from the hub, for the picker's rows and the composer's button
/// gating. Importing stays optional: these rows are visible and adjustable without it.
pub fn inherited_missing_hub(workdir: &Path) -> Vec<Server> {
    let hub = available();
    inherited(workdir)
        .into_iter()
        .filter(|s| !hub.iter().any(|h| h.id == s.id))
        .collect()
}

/// Read user and project mcpServers from ~/.claude.json. Use the final project path component for
/// source labels and name collisions; an empty source represents user configuration for the
/// frontend to label.
fn from_claude_json() -> Vec<Server> {
    let path = crate::claude::config_file(&crate::claude::user_home());
    let Some(root) = read_json(&path) else {
        return vec![];
    };
    let mut out = servers_in(&root, "");
    if let Some(projects) = root.get("projects").and_then(Value::as_object) {
        for (path, project) in projects {
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            out.extend(servers_in(project, &name));
        }
    }
    out
}

/// Read versioned .mcp.json files from repositories listed in ~/.claude.json.
fn from_repo_files() -> Vec<Server> {
    let path = crate::claude::config_file(&crate::claude::user_home());
    let Some(root) = read_json(&path) else {
        return vec![];
    };
    let Some(projects) = root.get("projects").and_then(Value::as_object) else {
        return vec![];
    };
    let mut out = Vec::new();
    for path in projects.keys() {
        let Some(file) = read_json(&Path::new(path).join(".mcp.json")) else {
            continue;
        };
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        out.extend(servers_in(&file, &name));
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Extract mcpServers from a user, project, or repository configuration object.
fn servers_in(value: &Value, origin: &str) -> Vec<Server> {
    value
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|servers| {
            servers
                .iter()
                .filter(|(_, config)| config.is_object())
                .map(|(id, config)| Server {
                    id: id.clone(),
                    config: config.clone(),
                    note: origin.to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Generate a Claude session config only for explicit selections. None preserves CLI defaults; an
/// empty list still generates an empty file for strict MCP exclusion. The strict file must carry
/// the whole effective set, so CLI-inherited servers the layers kept are materialized from their
/// discovered configuration too (ADR 0046); otherwise the first selection would silently drop
/// what the picker shows as inherited from the CLI.
pub fn config_for(
    id: &str,
    chosen: Option<&Vec<String>>,
    workdir: &Path,
    inherit_account_connectors: bool,
) -> Result<Option<PathBuf>, String> {
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    requires_connectors(!inherit_account_connectors || connectors().is_some())?;
    let path = session_path(id);
    // Refresh OAuth tokens while materializing session configuration, without persisting them in
    // the registry; see mcp_auth.rs.
    let body = serde_json::to_string_pretty(&config_body(
        &for_session(universe(workdir), id, chosen)?,
        chosen,
        mcp_auth::bearer,
    )?)
    .map_err(|e| e.to_string())?;
    paths::write_private(&path, &body)
        .map_err(|cause| i18n::ta("err.mcp.session", &[("cause", cause)]))?;
    Ok(Some(path))
}

/// The strict flag stops the CLI from fetching the account connectors, so materializing a selection
/// while their list is unknown would drop every one of them in silence. A preparation error prevents
/// the spawn instead (ADR 0063 and docs/contracts/agent-runtime.md). Pure, so the policy is tested
/// without an account or the network.
fn requires_connectors(known: bool) -> Result<(), String> {
    known
        .then_some(())
        .ok_or_else(|| i18n::t("err.mcp.connectors"))
}

/// Materialize every chosen server. Resolution already filtered the chosen ids against the
/// universe, so a miss here means the hub or the CLI configuration changed between resolve and
/// spawn; starting without a requested server is not a valid fallback
/// (docs/contracts/agent-runtime.md), so the spawn fails with the missing id. Inject token lookup
/// so configuration tests do not require disk state.
fn config_body(
    hub: &[Server],
    chosen: &[String],
    bearer: impl Fn(&str) -> Option<String>,
) -> Result<Value, String> {
    let mut servers = Map::new();
    for name in chosen {
        let Some(server) = hub.iter().find(|s| &s.id == name) else {
            return Err(i18n::ta("err.mcp.missing", &[("id", name.clone())]));
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

/* Codex configuration */

/// Override Codex's complete mcp_servers table for explicit selections, preserving its defaults for
/// None. Remote headers reference process environment variables. Stdio secrets use a private 0600
/// environment file sourced by a shell wrapper before exec. Commands without environment overrides
/// run directly. Secrets must never appear in process arguments.
pub type CodexMcp = (String, Vec<(String, String)>);

pub fn codex_config(id: &str, chosen: Option<&Vec<String>>) -> Result<Option<CodexMcp>, String> {
    let Some(chosen) = chosen else {
        return Ok(None);
    };
    let hub = for_session(available(), id, chosen)?;
    let mut env: Vec<(String, String)> = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    for name in chosen {
        let Some(server) = hub.iter().find(|s| &s.id == name) else {
            return Err(i18n::ta("err.mcp.missing", &[("id", name.clone())]));
        };
        let entry = match server.config.get("url").and_then(Value::as_str) {
            Some(url) => remote_entry(server, url, &mut env),
            None => local_entry(id, server)?,
        };
        entries.push(format!("{}={entry}", toml_key(&server.id)));
    }
    Ok(Some((format!("{{{}}}", entries.join(",")), env)))
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

fn remote_entry(server: &Server, url: &str, env: &mut Vec<(String, String)>) -> String {
    let mut headers = pairs(server.config.get("headers"));
    if let Some(token) = mcp_auth::bearer(&server.id) {
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

fn local_entry(id: &str, server: &Server) -> Result<String, String> {
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
        let path = codex_env_path(id, &server.id);
        let body = vars
            .iter()
            .map(|(k, v)| format!("export {k}='{}'\n", v.replace('\'', "'\\''")))
            .collect::<String>();
        paths::write_private(&path, &body)
            .map_err(|cause| i18n::ta("err.mcp.session", &[("cause", cause)]))?;
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

/* Server inspection */

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

/// Inspect the unsaved configuration directly from the form.
#[tauri::command]
pub async fn mcp_check(server: Server) -> Check {
    // Run synchronous HTTP and subprocess work outside async runtime workers; see linear::blocking.
    tauri::async_runtime::spawn_blocking(move || check(&server))
        .await
        .unwrap_or_else(|e| Check {
            probe: Probe {
                detail: e.to_string(),
                ..Probe::default()
            },
            ..Check::default()
        })
}

/// Start MCP OAuth discovery with an unauthenticated 401 response; mcp_auth.rs owns the login flow.
#[tauri::command]
pub async fn mcp_login(server: Server) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let url = server
            .config
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| i18n::t("err.mcp.auth.notRemote"))?
            .to_string();
        // Omit the token deliberately to obtain the OAuth challenge.
        let http = probe_http(&url, &server.config, None);
        mcp_auth::login(&server.id, &url, http.challenge.as_deref())
    })
    .await
    .map_err(|e| i18n::ta("err.mcp.auth.taskDied", &[("cause", e.to_string())]))?
}

#[tauri::command]
pub fn mcp_logout(id: String) -> Result<(), String> {
    mcp_auth::forget(&id)
}

/// Expose servers with stored login state for connection indicators.
#[tauri::command]
pub fn mcp_logins() -> Vec<String> {
    mcp_auth::logged_in()
}

/// For servers requiring login, also inspect OAuth endpoint discovery and dynamic client
/// registration support before offering a browser login that cannot succeed.
fn check(server: &Server) -> Check {
    let Some(url) = server.config.get("url").and_then(Value::as_str) else {
        let (probe, steps) = probe_stdio(&server.config);
        return Check { steps, probe };
    };
    // Use stored authentication during inspection so a successful login does not keep reporting
    // that login is required.
    let mut http = probe_http(url, &server.config, mcp_auth::bearer(&server.id).as_deref());
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

/// Start a local server, complete its stdio handshake, and list tools. Always stop the inspection
/// process afterward.
fn probe_stdio(config: &Value) -> (Probe, Vec<Step>) {
    let Some(command) = config.get("command").and_then(Value::as_str) else {
        let detail = "sem command nem url".to_string();
        return (
            Probe {
                detail: detail.clone(),
                ..Probe::default()
            },
            vec![Step::bad("spawn", detail)],
        );
    };
    let mut cmd = Command::new(command);
    if let Some(args) = config.get("args").and_then(Value::as_array) {
        cmd.args(args.iter().filter_map(Value::as_str));
    }
    if let Some(env) = config.get("env").and_then(Value::as_object) {
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                cmd.env(key, value);
            }
        }
    }
    // Use a separate process group so npx descendants cannot survive inspection cleanup.
    cmd.process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return (
                Probe {
                    detail: e.to_string(),
                    ..Probe::default()
                },
                vec![Step::bad("spawn", e.to_string())],
            )
        }
    };

    let mut probe = Probe::default();
    // Distinguish a tools/list response from an empty list or a missing response.
    let mut answered = false;
    if let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut write = |value: &Value| writeln!(stdin, "{value}").and_then(|()| stdin.flush());
        let sent = write(&hello())
            .and_then(|()| {
                write(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            })
            .and_then(|()| write(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })));
        match sent {
            Ok(()) => {
                let until = Instant::now() + PROBE_WAIT;
                // Accept logs, notifications, and requested responses in any order.
                while let Ok(line) =
                    rx.recv_timeout(until.saturating_duration_since(Instant::now()))
                {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(name) = value["result"]["serverInfo"]["name"].as_str() {
                        probe.name = name.to_string();
                        probe.ok = true;
                    }
                    if let Some(tools) = value["result"]["tools"].as_array() {
                        probe.tools = tools.len();
                        probe.ok = true;
                        answered = true;
                        break;
                    }
                    if let Some(error) = value["error"]["message"].as_str() {
                        probe.detail = error.to_string();
                    }
                }
            }
            Err(e) => probe.detail = e.to_string(),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if !probe.ok && probe.detail.is_empty() {
        // When no valid response arrives, retain stderr as the diagnostic for missing commands,
        // packages, or variables.
        probe.detail = child
            .stderr
            .take()
            .map(|err| {
                let mut text = String::new();
                let _ = std::io::BufReader::new(err).read_to_string(&mut text);
                short(&text)
            })
            .unwrap_or_default();
    }

    // Finalize inspection steps after process exit so stderr can explain failures.
    let mut steps = vec![Step::ok("spawn", String::new())];
    if !probe.ok {
        steps.push(Step::bad("handshake", probe.detail.clone()));
        return (probe, steps);
    }
    steps.push(Step::ok("handshake", probe.name.clone()));
    steps.push(tools_step(
        (!answered).then(|| probe.detail.clone()),
        probe.tools,
    ));
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

    #[test]
    fn slug_is_valid_as_a_suffix() {
        assert_eq!(slug("capim-backend"), "capim-backend");
        assert_eq!(slug("Meu Projeto!"), "meu-projeto");
    }

    #[test]
    fn reads_servers_from_an_object() {
        let value = json!({
            "mcpServers": {
                "notion": { "type": "http", "url": "https://mcp.notion.com/mcp" },
                "quebrado": "not an object"
            }
        });
        let found = servers_in(&value, "origem");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "notion");
        assert_eq!(found[0].note, "origem");
    }

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

    /// Ignored, like the server probe: it reaches the real account of the active Claude login and
    /// prints what the picker will show, so a change in the endpoint or in its naming is caught by
    /// hand: cargo test -- --ignored lists_real_account_connectors --nocapture.
    #[test]
    #[ignore]
    fn lists_real_account_connectors() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let got = connectors().expect("the account list must be readable in this run");
        for server in &got {
            println!("{} -> {}", server.id, server.config);
        }
        assert!(got.iter().all(|s| s.config["type"] == "claudeai-proxy"));
    }

    /// Ignored integration tests contact real MCP servers and may download packages: cargo test --
    /// --ignored sonda. Install the crypto provider explicitly because main does not run here.
    #[test]
    #[ignore]
    fn probes_real_servers() {
        let stdio = Server {
            id: "eco".into(),
            config: json!({ "command": "npx", "args": ["-y", "@modelcontextprotocol/server-everything"], "env": {} }),
            note: String::new(),
        };
        let got = check(&stdio).probe;
        println!(
            "stdio: ok={} tools={} name={} detail={}",
            got.ok, got.tools, got.name, got.detail
        );
        assert!(got.ok && got.tools > 0);

        // Test a remote server without authentication using an event-stream handshake response.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let remote = Server {
            id: "deepwiki".into(),
            config: json!({ "type": "http", "url": "https://mcp.deepwiki.com/mcp" }),
            note: String::new(),
        };
        let got = check(&remote).probe;
        println!(
            "http: ok={} auth={} tools={} name={} detail={}",
            got.ok, got.auth, got.tools, got.name, got.detail
        );
        assert!(got.ok && got.tools > 0);

        // Distinguish a reachable server requiring login from a failed connection.
        let requires_login = Server {
            id: "notion".into(),
            config: json!({ "type": "http", "url": "https://mcp.notion.com/mcp" }),
            note: String::new(),
        };
        let got = check(&requires_login).probe;
        println!("login: auth={} detail={}", got.auth, got.detail);
        assert!(got.auth);

        let missing = Server {
            id: "fantasma".into(),
            config: json!({ "command": "command-that-does-not-exist", "args": [], "env": {} }),
            note: String::new(),
        };
        let got = check(&missing).probe;
        assert!(!got.ok && !got.detail.is_empty());
    }

    /// Verify Codex header environment references and stdio wrappers keep secrets out of process
    /// arguments.
    #[test]
    fn codex_table_does_not_contain_secrets() {
        // Keep the fixture root in a child process so parallel tests cannot redirect hub reads.
        if std::env::var("PROMETEU_MCP_TEST_CHILD").as_deref() != Ok("1") {
            let root =
                std::env::temp_dir().join(format!("prometeu-codex-{}", uuid::Uuid::new_v4()));
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "mcp::tests::codex_table_does_not_contain_secrets",
                    "--nocapture",
                ])
                .env("PROMETEU_MCP_TEST_CHILD", "1")
                .env("PROMETEU_ROOT", &root)
                .output()
                .unwrap();
            std::fs::remove_dir_all(&root).ok();
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        let hub = vec![
            Server {
                id: "remoto".into(),
                config: json!({ "type": "http", "url": "https://x/mcp", "headers": { "X-Key": "abracadabra" } }),
                note: String::new(),
            },
            Server {
                id: "aqui".into(),
                config: json!({ "command": "npx", "args": ["-y", "coisa"], "env": { "TOKEN": "abracadabra" } }),
                note: String::new(),
            },
            Server {
                id: "simples".into(),
                config: json!({ "command": "node", "args": ["s.js"], "env": {} }),
                note: String::new(),
            },
        ];
        let body = serde_json::to_string(&hub).expect("hub");
        paths::write_private(&hub_path(), &body).expect("gravar hub");

        let chosen = vec![
            "remoto".to_string(),
            "aqui".to_string(),
            "simples".to_string(),
        ];
        let (table, env) = codex_config("aba", Some(&chosen))
            .expect("no error")
            .expect("selection exists");

        // No secret may appear anywhere in the command line.
        assert!(!table.contains("abracadabra"), "{table}");
        // The header references an environment variable carrying its value.
        assert!(table.contains("env_http_headers"), "{table}");
        assert!(env.iter().any(|(_, v)| v == "abracadabra"));
        // Only commands with environment overrides need a shell wrapper.
        assert!(table.contains("/bin/sh"), "{table}");
        assert!(table.contains("\"node\",args=[\"s.js\"]"), "{table}");
    }

    #[test]
    fn builtin_materializes_for_both_providers_without_persisting_registry_secrets() {
        if std::env::var_os("PROMETEU_BUILTIN_TEST_CHILD").is_none() {
            let root =
                std::env::temp_dir().join(format!("prometeu-builtin-{}", uuid::Uuid::new_v4()));
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "mcp::tests::builtin_materializes_for_both_providers_without_persisting_registry_secrets"])
                .env("PROMETEU_BUILTIN_TEST_CHILD", "1").env("PROMETEU_ROOT", &root).output().unwrap();
            std::fs::remove_dir_all(&root).ok();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        assert!(load().is_empty());
        assert!(available().iter().any(|s| s.id == "prometeu"));
        assert!(!hub_path().exists());
        assert!(codex_config("owner", None).unwrap().is_none());
        assert!(config_for("owner", None, Path::new("/tmp"), false)
            .unwrap()
            .is_none());
        let server = crate::embedded_mcp::server_config(
            "/app/Prometeu".into(),
            "/tmp/local/socket".into(),
            "test-credential".into(),
        );
        let body = config_body(std::slice::from_ref(&server), &["prometeu".into()], |_| {
            None
        })
        .unwrap();
        assert_eq!(
            body["mcpServers"]["prometeu"]["env"]["PROMETEU_MCP_TOKEN"],
            "test-credential"
        );
        let entry = local_entry("owner", &server).unwrap();
        assert!(entry.contains("--prometeu-mcp"));
        assert!(!entry.contains("test-credential"));
        let private = codex_env_path("owner", "prometeu");
        assert!(std::fs::read_to_string(&private)
            .unwrap()
            .contains("test-credential"));
        assert_eq!(
            std::fs::metadata(private).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!hub_path().exists());
    }

    /// Without an explicit selection, do not generate a configuration file or change legacy startup
    /// behavior.
    #[test]
    fn no_selection_creates_no_file() {
        assert!(config_for("aba", None, Path::new("/tmp"), false)
            .expect("no error")
            .is_none());
    }

    /// The inherited base of one working directory: user scope, the project entry keyed by the
    /// directory, and the repository's .mcp.json plus its ancestors' nearest first, deduplicated
    /// by name with the first origin winning (ADR 0046).
    #[test]
    fn inherited_base_combines_user_project_and_repository_configuration() {
        let workdir = Path::new("/dev/projeto");
        let claude = json!({
            "mcpServers": { "do-usuario": { "type": "http", "url": "https://u/mcp" } },
            "projects": {
                "/dev/projeto": { "mcpServers": { "do-projeto": { "command": "p" } } },
                "/dev/other": { "mcpServers": { "estranho": { "command": "x" } } }
            }
        });
        let files = vec![
            (
                "projeto".to_string(),
                json!({
                    "mcpServers": {
                        "do-repo": { "command": "r" },
                        "do-usuario": { "command": "conflito" }
                    }
                }),
            ),
            (
                "dev".to_string(),
                json!({
                    "mcpServers": {
                        "do-ancestral": { "command": "a" },
                        "do-repo": { "command": "conflito-ancestral" }
                    }
                }),
            ),
        ];
        let got = inherited_from(Some(&claude), workdir, &files, &[]);
        let get = |id: &str| got.iter().find(|s| s.id == id).unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!(get("do-projeto").config["command"], "p");
        assert_eq!(get("do-projeto").note, "projeto");
        assert_eq!(get("do-usuario").config["command"], "conflito");
        assert_eq!(get("do-repo").config["command"], "r");
        assert_eq!(get("do-ancestral").note, "dev");
        // Without configuration files the base is empty.
        assert!(inherited_from(None, workdir, &[], &[]).is_empty());
    }

    /// The account connectors join the base with the CLI's own naming, and a configuration file the
    /// person controls shadows a connector of the same name (ADR 0063).
    #[test]
    fn account_connectors_join_the_inherited_base() {
        let body = json!({ "data": [
            { "type": "mcp_server", "id": "mcpsrv_1", "display_name": "Linear", "url": "https://linear/mcp" },
            { "type": "mcp_server", "id": "mcpsrv_2", "display_name": "Linear", "url": "https://other/mcp" },
            { "type": "mcp_server", "id": "mcpsrv_3", "display_name": "Notion" },
        ]});
        let connectors = connectors_from(&body);
        // A repeated display name is numbered, and an entry without a URL cannot be materialized.
        assert_eq!(
            connectors.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["claude.ai Linear", "claude.ai Linear (2)"]
        );
        assert_eq!(connectors[0].note, "claude.ai");
        assert_eq!(
            connectors[0].config,
            json!({ "type": "claudeai-proxy", "url": "https://linear/mcp", "id": "mcpsrv_1" })
        );

        let workdir = Path::new("/dev/project");
        let claude = json!({ "mcpServers": { "claude.ai Linear": { "command": "own" } } });
        let base = inherited_from(Some(&claude), workdir, &[], &connectors);
        let get = |id: &str| base.iter().find(|s| s.id == id).unwrap();
        assert_eq!(base.len(), 2);
        assert_eq!(get("claude.ai Linear").config["command"], "own");
        assert_eq!(get("claude.ai Linear (2)").note, "claude.ai");

        // The strict configuration materializes the proxy entry the CLI would have created.
        let body = config_body(&base, &["claude.ai Linear (2)".into()], |_| None).unwrap();
        assert_eq!(
            body["mcpServers"]["claude.ai Linear (2)"]["type"],
            "claudeai-proxy"
        );
        assert_eq!(body["mcpServers"]["claude.ai Linear (2)"]["id"], "mcpsrv_2");
    }

    #[test]
    fn local_mcp_shadows_project_and_user_and_materializes_the_same_definition() {
        let workdir = Path::new("/review/project");
        let claude = json!({
            "mcpServers": {"db": {"command":"user"}, "user-only": {"command":"user-only"}},
            "projects": {"/review/project": {"mcpServers": {"db": {"command":"local"}}}}
        });
        let files = vec![(
            "project".into(),
            json!({"mcpServers":{"db":{"command":"project"}}}),
        )];
        let hub = inherited_from(Some(&claude), workdir, &files, &[]);
        let body = config_body(&hub, &["db".into(), "user-only".into()], |_| None).unwrap();
        assert_eq!(body["mcpServers"]["db"]["command"], "local");
        assert_eq!(body["mcpServers"]["user-only"]["command"], "user-only");
    }

    /// A hub entry wins an ID clash, so an imported server stays Prometeu-managed.
    #[test]
    fn hub_wins_name_collisions_in_the_universe() {
        let hub = vec![Server {
            id: "notion".into(),
            config: json!({ "url": "https://hub" }),
            note: String::new(),
        }];
        let inherited = vec![
            Server {
                id: "notion".into(),
                config: json!({ "url": "https://cli" }),
                note: String::new(),
            },
            Server {
                id: "n8n".into(),
                config: json!({ "command": "npx" }),
                note: String::new(),
            },
        ];
        let universe = merge_universe(hub, inherited);
        assert_eq!(universe.len(), 2);
        assert_eq!(universe[0].config["url"], "https://hub");
        assert_eq!(universe[1].id, "n8n");
    }

    #[test]
    fn includes_only_selected_servers() {
        let hub = vec![
            Server {
                id: "notion".into(),
                config: json!({ "type": "http", "url": "https://mcp.notion.com/mcp" }),
                note: String::new(),
            },
            Server {
                id: "drive".into(),
                config: json!({ "type": "http", "url": "https://drive" }),
                note: String::new(),
            },
        ];
        let body = config_body(&hub, &["notion".to_string()], |_| None).expect("materializa");
        let servers = body["mcpServers"].as_object().expect("objeto");
        assert_eq!(servers.len(), 1);
        assert!(servers.contains_key("notion"));
    }

    /// An unknown account list fails the spawn too: under the strict flag the CLI no longer fetches
    /// the connectors, so materializing without them would drop what the person kept (ADR 0063).
    #[test]
    fn unknown_account_connectors_prevent_materialization() {
        assert!(requires_connectors(true).is_ok());
        let err = requires_connectors(false).expect_err("failure");
        assert!(err.contains("err.mcp.connectors"), "{err}");
    }

    #[test]
    fn older_account_fetch_cannot_replace_newer_account_cache() {
        let old = (uuid::Uuid::new_v4().to_string(), 1);
        let new = (uuid::Uuid::new_v4().to_string(), 1);
        let server = |id: &str| Server {
            id: id.into(),
            config: json!({ "type": "http", "url": "https://example.com" }),
            note: String::new(),
        };
        remember_connectors(new.clone(), vec![server("new")]);
        remember_connectors(old, vec![server("old")]);
        assert_eq!(cached_connectors(&new).unwrap().1[0].id, "new");
    }

    /// A chosen id the universe no longer has fails the spawn instead of silently shrinking the
    /// effective set (docs/contracts/agent-runtime.md).
    #[test]
    fn missing_selected_servers_prevent_materialization() {
        let got = config_body(&[], &["missing".to_string()], |_| None);
        let err = got.expect_err("failure");
        assert!(err.contains("missing"), "{err}");
    }

    /// An empty selection still generates strict empty configuration. Inject OAuth authorization
    /// without discarding other configured headers.
    #[test]
    fn token_becomes_a_header_in_the_session_file() {
        let hub = vec![Server {
            id: "capisce".into(),
            config: json!({ "type": "http", "url": "https://x/mcp", "headers": { "X-Id": "7" } }),
            note: String::new(),
        }];
        let body = config_body(&hub, &["capisce".to_string()], |id| {
            (id == "capisce").then(|| "abc123".to_string())
        })
        .expect("materializa");
        let headers = &body["mcpServers"]["capisce"]["headers"];
        assert_eq!(headers["Authorization"], "Bearer abc123");
        assert_eq!(headers["X-Id"], "7");
    }

    #[test]
    fn explicitly_empty_selection_creates_an_empty_file() {
        let body = config_body(&[], &[], |_| None).expect("materializa");
        assert_eq!(body["mcpServers"].as_object().expect("objeto").len(), 0);
    }
}
