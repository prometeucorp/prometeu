//! The MCP hub stores known servers and workspace selections. Explicit selections restrict
//! available tools, reducing context cost and unnecessary access; None preserves CLI defaults.
//! Claude receives a private 0600 config file with strict selection, while Codex receives a
//! configuration override and protected secrets. Never place secrets in process arguments. Discover
//! existing user/project CLI configuration for read-only import; mcp_auth owns OAuth and token
//! refresh.

use crate::i18n;
use crate::mcp_auth;
use crate::paths;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(test)]
use prometeu_tools::mcp::config_body;
use prometeu_tools::mcp::slug;
pub use prometeu_tools::mcp::Server;
use prometeu_tools::mcp_discovery::{read_json, servers_in};

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
    prometeu_tools::mcp::McpCatalog::load(&prometeu_tools::mcp::FileMcpCatalog(hub_path()))
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
    let server = prometeu_tools::mcp::validate(server, &[crate::embedded_mcp::ID])?;
    crate::catalog::save_mcp(&app, &server, revision)?;
    let mut servers = load();
    prometeu_tools::mcp::register(&mut servers, server);
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
    prometeu_tools::mcp_discovery::found(
        &crate::claude::config_file(&crate::claude::user_home()),
        &available(),
    )
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
    revision: u64,
    at: Instant,
    servers: Vec<Server>,
}

fn connector_cache() -> &'static Mutex<HashMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_connectors(account: &str, revision: u64) -> Option<(Instant, Vec<Server>)> {
    crate::lock::lock(connector_cache())
        .get(account)
        .filter(|cached| cached.revision == revision)
        .map(|cached| (cached.at, cached.servers.clone()))
}

fn remember_connectors(account: String, revision: u64, servers: Vec<Server>) {
    let mut cache = crate::lock::lock(connector_cache());
    if cache
        .get(&account)
        .is_some_and(|cached| cached.revision > revision)
    {
        return;
    }
    cache.insert(
        account,
        Cached {
            revision,
            at: Instant::now(),
            servers,
        },
    );
}

fn fetched_or_cached(
    account: &str,
    revision: u64,
    fetched: Option<Vec<Server>>,
) -> Option<Vec<Server>> {
    match fetched {
        Some(servers) => {
            remember_connectors(account.to_string(), revision, servers.clone());
            Some(servers)
        }
        None => cached_connectors(account, revision).map(|(_, servers)| servers),
    }
}

/// The connectors of the person's Claude account (ADR 0063). Claude Code keeps them in no
/// configuration file: it fetches them from the account at every start and loads them under its own
/// `claudeai` scope. Discovery has to ask the same endpoint, or the picker would hide what the CLI
/// shows and a strict spawn would drop every connector without saying so.
///
/// `None` means the list of a logged-in account is unknown — credential read or fetch failed, with
/// nothing cached for that login — which differs from an account with no connectors. The picker
/// shows the file base; a spawn that inherits the account base refuses to start. A failed fetch
/// keeps the last known list.
pub fn connectors() -> Option<Vec<Server>> {
    // Without an account, the CLI loads no connectors.
    let Ok(profile) = crate::accounts::active(crate::state::ProviderId::Claude) else {
        return Some(Vec::new());
    };
    let token = match crate::usage::claude_token(&profile) {
        Ok(Some(token)) => token,
        Ok(None) => return Some(Vec::new()),
        Err(()) => {
            return cached_connectors(&profile.id, profile.revision).map(|(_, servers)| servers)
        }
    };
    if let Some((at, servers)) = cached_connectors(&profile.id, profile.revision) {
        if at.elapsed() < CONNECTOR_TTL {
            return Some(servers);
        }
    }
    // Another request may populate this login while the fetch is in flight.
    fetched_or_cached(
        &profile.id,
        profile.revision,
        fetch_connectors(&token).map(|body| connectors_from(&body)),
    )
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

/// The strict flag stops the CLI from fetching the account connectors, so materializing a selection
/// while their list is unknown would drop every one of them in silence. A preparation error prevents
/// the spawn instead (ADR 0063 and docs/contracts/agent-runtime.md). Pure, so the policy is tested
/// without an account or the network.
pub(crate) fn requires_connectors(known: bool) -> Result<(), String> {
    known
        .then_some(())
        .ok_or_else(|| i18n::t("err.mcp.connectors"))
}

pub(crate) struct Sources;
impl prometeu_tools::mcp::McpSources for Sources {
    fn claude_servers(
        &self,
        session: &str,
        chosen: &[String],
        workdir: &Path,
    ) -> Result<Vec<Server>, String> {
        for_session(universe(workdir), session, chosen)
    }
    fn codex_servers(&self, session: &str, chosen: &[String]) -> Result<Vec<Server>, String> {
        for_session(available(), session, chosen)
    }
    fn bearer(&self, server: &str) -> Option<String> {
        mcp_auth::bearer(server)
    }
}
pub(crate) struct PrivateFiles;
impl prometeu_tools::mcp::McpFiles for PrivateFiles {
    fn claude_config(&self, session: &str, body: &str) -> Result<PathBuf, String> {
        write_session_file(session_path(session), body)
    }
    fn codex_environment(
        &self,
        session: &str,
        server: &str,
        body: &str,
    ) -> Result<PathBuf, String> {
        write_session_file(codex_env_path(session, server), body)
    }
}
fn write_session_file(path: PathBuf, body: &str) -> Result<PathBuf, String> {
    paths::write_private(&path, body)
        .map_err(|cause| i18n::ta("err.mcp.session", &[("cause", cause)]))?;
    Ok(path)
}
#[cfg(test)]
fn materializer() -> prometeu_tools::mcp::McpMaterializer<'static> {
    prometeu_tools::mcp::McpMaterializer {
        sources: &Sources,
        files: &PrivateFiles,
    }
}
#[cfg(test)]
pub fn config_for(
    id: &str,
    chosen: Option<&[String]>,
    workdir: &Path,
) -> Result<Option<PathBuf>, String> {
    materializer().claude_config(id, chosen, workdir)
}
#[cfg(test)]
pub fn codex_config(
    id: &str,
    chosen: Option<&[String]>,
) -> Result<Option<prometeu_tools::mcp::CodexMcp>, String> {
    materializer().codex_config(id, chosen)
}

/* Server inspection */

pub use prometeu_tools::mcp_probe::{Check, Probe};
fn inspection() -> prometeu_tools::mcp_probe::Inspection {
    prometeu_tools::mcp_probe::Inspection {
        auth: mcp_auth::service(),
        query: std::sync::Arc::new(prometeu_process::query::UnixQueryLauncher),
    }
}
fn check(server: &Server) -> Check {
    inspection().check(server)
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
        let request = inspection().begin(&server)?;
        mcp_auth::consent(request)
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
        assert!(config_for("owner", None, Path::new("/tmp"))
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
        let entry = prometeu_tools::mcp::local_entry("owner", &server, &PrivateFiles).unwrap();
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
        assert!(config_for("aba", None, Path::new("/tmp"))
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
    fn account_cache_keeps_current_revision_and_observes_overlapping_fetch() {
        let account = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let server = |id: &str| Server {
            id: id.into(),
            config: json!({ "type": "http", "url": "https://example.com" }),
            note: String::new(),
        };
        remember_connectors(account.clone(), 2, vec![server("new")]);
        remember_connectors(account.clone(), 1, vec![server("old")]);
        remember_connectors(other.clone(), 1, vec![server("other")]);
        assert_eq!(cached_connectors(&account, 2).unwrap().1[0].id, "new");
        assert!(cached_connectors(&account, 1).is_none());
        assert_eq!(fetched_or_cached(&account, 2, None).unwrap()[0].id, "new");
        assert_eq!(fetched_or_cached(&other, 1, None).unwrap()[0].id, "other");
        assert_eq!(
            connector_cache()
                .lock()
                .unwrap()
                .keys()
                .filter(|id| *id == &account)
                .count(),
            1
        );
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
