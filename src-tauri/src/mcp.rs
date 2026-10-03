//! The MCP hub stores known servers and workspace selections. Explicit selections restrict
//! available tools, reducing context cost and unnecessary access; None preserves CLI defaults.
//! Claude receives a private 0600 config file with strict selection, while Codex receives a
//! configuration override and protected secrets. Never place secrets in process arguments. Discover
//! existing user/project CLI configuration for read-only import; mcp_auth owns OAuth and token
//! refresh.

use crate::i18n;
use crate::mcp_auth;
use crate::paths;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;

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

/// The servers the CLI itself loads for one working directory (ADR 0046): the user-scope
/// `mcpServers` of `~/.claude.json`, the project-scope entry keyed by the directory, and the
/// directory's `.mcp.json` plus every ancestor directory's, as the CLI walks the tree upward from
/// the working directory (ADR 0045). They form the inherited base of the mcp axis, visible in the
/// picker without importing. IDs keep their original names; the first occurrence of a repeated
/// name wins: local scope precedes project scope, then user scope. Within project scope the
/// nearest repository file precedes its ancestors (ADR 0047).
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
    inherited_from(claude.as_ref(), workdir, &files)
}

/// The pure core of `inherited`, injectable so tests need no home directory or file tree. `files`
/// holds the repository `.mcp.json` values nearest first; the note carries the origin: empty for
/// user scope, the directory name otherwise, mirroring `from_claude_json`.
fn inherited_from(
    claude: Option<&Value>,
    workdir: &Path,
    files: &[(String, Value)],
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
        let got = inherited_from(Some(&claude), workdir, &files);
        let get = |id: &str| got.iter().find(|s| s.id == id).unwrap();
        assert_eq!(got.len(), 4);
        assert_eq!(get("do-projeto").config["command"], "p");
        assert_eq!(get("do-projeto").note, "projeto");
        assert_eq!(get("do-usuario").config["command"], "conflito");
        assert_eq!(get("do-repo").config["command"], "r");
        assert_eq!(get("do-ancestral").note, "dev");
        // Without configuration files the base is empty.
        assert!(inherited_from(None, workdir, &[]).is_empty());
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
        let hub = inherited_from(Some(&claude), workdir, &files);
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
