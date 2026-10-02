//! The plugin hub stores packages and workspace selections for Claude and Codex. Plugins can
//! provide skills, commands, agents, and hooks that maintain behavior across turns. Claude receives
//! session flags; Codex receives a derived workspace home and local marketplace while preserving
//! shared account data. None leaves CLI defaults intact. Installation clones repositories and
//! imports root plugins or marketplace entries; updates use the same clone. Creation generates a
//! package in application-owned storage. Manually registered directories remain owned by the person
//! who created them.
use crate::i18n;
use crate::lock::lock;
use crate::paths;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// A persisted hub plugin.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct Plugin {
    /// The manifest name is the CLI's deduplication identity and the displayed plugin name.
    pub id: String,
    /// A local directory or ZIP path, or a remote ZIP URL, translated into provider configuration.
    pub source: String,
    /// Free-form source or purpose displayed below the name.
    #[serde(default)]
    pub note: String,
    /// Only application-created or cloned directories may be deleted on removal. Manually
    /// registered directories remain user-owned.
    #[serde(default)]
    pub made: bool,
    /// The original repository address supplies the displayed source and update target.
    #[serde(default)]
    pub from: String,
}

/// Keep the path-and-URL registry private alongside other application state, even though it
/// contains no credentials.
fn hub_path() -> PathBuf {
    paths::root().join("plugins.json")
}

pub fn load() -> Vec<Plugin> {
    std::fs::read_to_string(hub_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Plugin>>(&raw).ok())
        .unwrap_or_default()
}

pub(crate) fn write_hub(plugins: &[Plugin]) -> Result<(), String> {
    let body = serde_json::to_string_pretty(plugins).map_err(|e| e.to_string())?;
    paths::write_private(&hub_path(), &body)
        .map_err(|cause| i18n::ta("err.plugin.save", &[("cause", cause)]))
}

/// Expose the complete registry to settings and selectors.
#[tauri::command(async)]
pub fn plugin_hub() -> Vec<Plugin> {
    let _sync = crate::catalog::guard();
    load()
}

/// Save or replace a plugin by name, matching the CLI's own deduplication identity.
#[tauri::command(async)]
pub fn plugin_save(
    app: AppHandle,
    plugin: Plugin,
    revision: Option<u64>,
) -> Result<Vec<Plugin>, String> {
    let _sync = crate::catalog::guard();
    let plugin = trim(plugin);
    if plugin.id.is_empty() {
        return Err(i18n::t("err.plugin.noName"));
    }
    check_source(&plugin.source)?;
    // Publish changes only for explicitly shared items.
    crate::catalog::save_plugin(&app, &plugin, revision)?;
    save_local(plugin)
}

fn trim(plugin: Plugin) -> Plugin {
    Plugin {
        id: plugin.id.trim().to_string(),
        source: plugin.source.trim().to_string(),
        note: plugin.note.trim().to_string(),
        made: plugin.made,
        from: plugin.from.trim().to_string(),
    }
}

/// Save local installation, update, and test changes without publishing them.
pub(crate) fn save_local(plugin: Plugin) -> Result<Vec<Plugin>, String> {
    let plugin = Plugin {
        id: plugin.id.trim().to_string(),
        source: plugin.source.trim().to_string(),
        note: plugin.note.trim().to_string(),
        made: plugin.made,
        from: plugin.from.trim().to_string(),
    };
    if plugin.id.is_empty() {
        return Err(i18n::t("err.plugin.noName"));
    }
    check_source(&plugin.source)?;
    let mut plugins = load();
    match plugins.iter_mut().find(|p| p.id == plugin.id) {
        Some(old) => *old = plugin,
        None => plugins.push(plugin),
    }
    plugins.sort_by_key(|p| p.id.to_lowercase());
    write_hub(&plugins)?;
    Ok(plugins)
}

/// Remove the registry entry and delete its directory only when the app owns it. Manually
/// registered directories remain untouched.
#[tauri::command(async)]
pub fn plugin_remove(app: AppHandle, id: String) -> Result<Vec<Plugin>, String> {
    let _sync = crate::catalog::guard();
    crate::catalog::remove_shared(&app, "plugins", &id)?;
    remove_hub(&id)
}

pub(crate) fn remove_hub(id: &str) -> Result<Vec<Plugin>, String> {
    let plugins = remove_local(load(), id);
    write_hub(&plugins)?;
    Ok(plugins)
}

/// Remove this Mac's registration and any application-owned directory; the caller persists the hub.
pub(crate) fn remove_local(mut plugins: Vec<Plugin>, id: &str) -> Vec<Plugin> {
    let mut removed = false;
    if let Some(gone) = plugins.iter().find(|p| p.id == id) {
        removed = true;
        let dir = PathBuf::from(expand(&gone.source));
        if gone.made && dir.starts_with(store()) && dir != store() {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
    plugins.retain(|p| p.id != id);
    if removed && slug(id) == id && !cfg!(test) {
        codex_remove_everywhere(&format!("{}@{}", id, codex_marketplace_name()));
    }
    plugins
}

/// Reject sources the CLI cannot load before starting a session that would silently omit the
/// selected plugin.
fn check_source(source: &str) -> Result<(), String> {
    if source.is_empty() {
        return Err(i18n::t("err.plugin.noSource"));
    }
    if remote(source) {
        return Ok(());
    }
    let path = PathBuf::from(expand(source));
    if !path.exists() {
        return Err(i18n::ta(
            "err.plugin.noPath",
            &[("path", path.display().to_string())],
        ));
    }
    // The CLI opens ZIP sources directly; directories require a plugin manifest.
    if path.is_dir() && !manifest_path(&path).exists() {
        return Err(i18n::ta(
            "err.plugin.notPlugin",
            &[("path", path.display().to_string())],
        ));
    }
    Ok(())
}

/// Read a directory's manifest to prefill its name and description. For ZIP paths or URLs, suggest
/// the filename without downloading it. Apply the same source validation used during saving.
#[tauri::command]
pub fn plugin_look(source: String) -> Result<Plugin, String> {
    let source = source.trim().to_string();
    check_source(&source)?;
    let path = PathBuf::from(expand(&source));
    let manifest = (!remote(&source) && path.is_dir())
        .then(|| read_json(&manifest_path(&path)))
        .flatten();
    let text = |key: &str| {
        manifest
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let id = match text("name") {
        name if !name.is_empty() => name,
        _ => guessed_name(&source),
    };
    Ok(Plugin {
        id,
        source,
        note: text("description"),
        made: false,
        from: String::new(),
    })
}

/// Suggest a ZIP filename without its extension; the enclosed manifest remains authoritative.
fn guessed_name(source: &str) -> String {
    source
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".zip")
        .to_string()
}

/// Generate flags for selected IDs still present in the hub, skipping deleted entries for
/// compatibility. None leaves CLI defaults intact.
pub fn args_for(chosen: Option<&Vec<String>>) -> Vec<String> {
    match chosen {
        Some(chosen) => args_from(&load(), chosen),
        None => vec![],
    }
}

/// Inject the hub contents so flag translation tests do not require filesystem state.
fn args_from(hub: &[Plugin], chosen: &[String]) -> Vec<String> {
    chosen
        .iter()
        .filter_map(|name| hub.iter().find(|p| &p.id == name))
        .flat_map(flags)
        .collect()
}

/// Use plugin-url for remote sources and plugin-dir for local ones. Expand tilde at launch time
/// using this machine's home without rewriting the stored source.
fn flags(plugin: &Plugin) -> [String; 2] {
    let source = plugin.source.trim();
    if remote(source) {
        ["--plugin-url".to_string(), source.to_string()]
    } else {
        ["--plugin-dir".to_string(), expand(source)]
    }
}

pub(crate) fn remote(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(".claude-plugin").join("plugin.json")
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub(crate) fn expand(source: &str) -> String {
    match source.strip_prefix("~/") {
        Some(rest) => paths::home().join(rest).display().to_string(),
        None => source.to_string(),
    }
}

/* Codex adaptation */

/// Return the derived home, selected canonical plugin IDs, and the subset declaring hooks. The
/// handshake may trust only selected IDs and must discover every required hook before opening the
/// thread.
pub struct CodexPlugins {
    pub home: Option<PathBuf>,
    pub ids: Vec<String>,
    pub hook_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreparedPlugin {
    id: String,
    canonical: String,
    version: String,
    hooks: bool,
}

#[derive(Clone, Debug, Default)]
struct InstalledPlugin {
    version: String,
}

/// Use a reserved marketplace namespace because Codex's shared cache must not collide with
/// user-registered marketplaces.
fn codex_marketplace_name() -> &'static str {
    if cfg!(debug_assertions) {
        "prometeu-dev"
    } else {
        "prometeu"
    }
}

/// Include the adapter revision in cache versions so corrected materialization reinstalls unchanged
/// upstream packages.
const CODEX_PACKAGE_REVISION: &str = "2";

fn codex_workspaces_root() -> PathBuf {
    paths::root().join("codex-workspaces")
}

fn codex_marketplace_root(home: &Path) -> PathBuf {
    home.join("marketplace")
}

/// Derive the home from the persisted workspace ID, not cwd or an ephemeral tab ID. Workspaces
/// sharing a clone can still have different selections.
fn codex_workspace_home(workspace: &str) -> PathBuf {
    let fingerprint = format!("{:x}", Sha256::digest(workspace.as_bytes()));
    codex_workspaces_root().join(&fingerprint[..24])
}

/// Remove this disposable configuration with its workspace. Shared installed payloads and other
/// workspaces' configurations remain available.
pub fn forget_codex_workspace(workspace: &str) {
    let root = codex_workspaces_root();
    let home = codex_workspace_home(workspace);
    remove_codex_home(&root, &home);
}

fn remove_codex_home(root: &Path, home: &Path) {
    if home.starts_with(root) && home != root {
        std::fs::remove_dir_all(home).ok();
    }
}

/// Serialize marketplace materialization, configuration, and shared-cache installation so
/// simultaneous tabs cannot observe a partial plugin version.
pub fn codex_for(
    workspace: &str,
    chosen: Option<&Vec<String>>,
    profile: &crate::accounts::Profile,
) -> Result<CodexPlugins, String> {
    let Some(chosen) = chosen else {
        return Ok(CodexPlugins {
            home: None,
            ids: vec![],
            hook_ids: vec![],
        });
    };
    let hub = load();
    let mut seen = HashSet::new();
    let selected: Vec<Plugin> = chosen
        .iter()
        .filter_map(|id| hub.iter().find(|plugin| &plugin.id == id).cloned())
        .filter(|plugin| seen.insert(plugin.id.clone()))
        .collect();

    static PREPARE: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = lock(PREPARE.get_or_init(|| Mutex::new(())));
    // Keep separate account homes so new account selections cannot redirect authentication links
    // used by running processes.
    let home = if profile.managed {
        codex_workspace_home(workspace).join(&profile.id)
    } else {
        codex_workspace_home(workspace)
    };
    let marketplace = codex_marketplace_root(&home);
    let prepared = prepare_marketplace(&marketplace, codex_marketplace_name(), &selected)?;
    let ids = prepared
        .iter()
        .map(|plugin| plugin.canonical.clone())
        .collect::<Vec<_>>();
    let hook_ids = prepared
        .iter()
        .filter(|plugin| plugin.hooks)
        .map(|plugin| plugin.canonical.clone())
        .collect::<Vec<_>>();
    let base = &profile.home;
    prepare_codex_home(base, &home, &marketplace, &ids)?;

    if prepared.is_empty() {
        return Ok(CodexPlugins {
            home: Some(home),
            ids,
            hook_ids,
        });
    }

    let installed = codex_installed(&home)?;
    for plugin in &prepared {
        let current = installed.get(&plugin.canonical);
        let needs_install = current.is_none_or(|found| found.version != plugin.version);
        if needs_install {
            if let Err(error) = codex_install(&home, &plugin.canonical) {
                codex_remove(&home, &plugin.canonical);
                return Err(error);
            }
        }
    }
    // Rebuild derived configuration after codex plugin add enables entries, restoring the exact
    // selection and preserving previously trusted workspace hook hashes.
    write_codex_config(base, &home, &marketplace, &ids)?;
    Ok(CodexPlugins {
        home: Some(home),
        ids,
        hook_ids,
    })
}

/// Share native login, rollouts, skills, and databases through links. Only configuration and
/// marketplace contents are disposable application-owned data.
fn prepare_codex_home(
    base: &Path,
    home: &Path,
    marketplace: &Path,
    selected: &[String],
) -> Result<(), String> {
    if base == home {
        return Err(i18n::ta(
            "err.plugin.codex.config",
            &[(
                "cause",
                "derived CODEX_HOME collides with the user home".into(),
            )],
        ));
    }
    std::fs::create_dir_all(base)
        .and_then(|()| std::fs::create_dir_all(base.join("plugins")))
        .map_err(|error| i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())]))?;
    paths::ensure_private_dir(home)
        .map_err(|cause| i18n::ta("err.plugin.codex.config", &[("cause", cause)]))?;
    mirror_codex_home(base, home)?;
    write_codex_config(base, home, marketplace, selected)
}

/// Mirror existing and future Codex entries except writable configuration files. Preserve real
/// entries already created in the derived home; replace only obsolete links to another home.
fn mirror_codex_home(base: &Path, home: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(base)
        .map_err(|error| i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())]))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())])
        })?;
        let name = entry.file_name();
        let text = name.to_string_lossy();
        if text.starts_with("config.toml")
            || text.starts_with(".config.toml")
            || text == "marketplace"
        {
            continue;
        }
        let target = home.join(&name);
        replace_with_shared_entry(&entry.path(), &target, home).map_err(|error| {
            i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())])
        })?;
    }
    Ok(())
}

fn replace_with_shared_entry(source: &Path, target: &Path, home: &Path) -> std::io::Result<()> {
    if !target.starts_with(home) || target == home {
        return Err(std::io::Error::other("invalid derived CODEX_HOME target"));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(target) {
        #[cfg(unix)]
        if metadata.file_type().is_symlink()
            && std::fs::read_link(target).ok().as_deref() == Some(source)
        {
            return Ok(());
        }
        if metadata.file_type().is_symlink() {
            std::fs::remove_file(target)?;
        } else {
            return Ok(());
        }
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(source, target)?;
    #[cfg(not(unix))]
    if source.is_dir() {
        copy_tree(source, target)?;
    } else {
        std::fs::copy(source, target)?;
    }
    Ok(())
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => raw
            .parse::<toml::Value>()
            .map_err(|error| i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())])),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::map::Map::new()))
        }
        Err(error) => Err(i18n::ta(
            "err.plugin.codex.config",
            &[("cause", error.to_string())],
        )),
    }
}

fn child_table<'a>(
    parent: &'a mut toml::map::Map<String, toml::Value>,
    key: &str,
) -> &'a mut toml::map::Map<String, toml::Value> {
    let value = parent
        .entry(key.to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !value.is_table() {
        *value = toml::Value::Table(toml::map::Map::new());
    }
    value.as_table_mut().expect("table inserted above")
}

fn prometeu_plugin(id: &str) -> bool {
    id.rsplit_once('@')
        .is_some_and(|(_, marketplace)| matches!(marketplace, "prometeu" | "prometeu-dev"))
}

/// Rebuild derived configuration from the real home plus retained workspace hook and plugin
/// settings. Disable all reserved marketplace entries before enabling only the current selection.
fn write_codex_config(
    base: &Path,
    home: &Path,
    marketplace: &Path,
    selected: &[String],
) -> Result<(), String> {
    // Treat malformed derived TOML as disposable cache. Rebuild from real configuration and request
    // hook trust again instead of permanently blocking the workspace.
    let previous = read_toml(&home.join("config.toml"))
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()));
    let previous_hook_state = previous
        .get("hooks")
        .and_then(|value| value.get("state"))
        .and_then(toml::Value::as_table)
        .cloned();
    let previous_plugins = previous
        .get("plugins")
        .and_then(toml::Value::as_table)
        .cloned()
        .unwrap_or_default();

    let mut config = read_toml(&base.join("config.toml"))?;
    if !config.is_table() {
        return Err(i18n::ta(
            "err.plugin.codex.config",
            &[("cause", "Codex config root is not a TOML table".into())],
        ));
    }
    let root = config.as_table_mut().expect("checked above");

    // Pin default and auto credential storage to file so refresh uses the shared auth.json link.
    // Respect explicit keyring or ephemeral settings.
    let auth_store = root
        .get("cli_auth_credentials_store")
        .and_then(toml::Value::as_str);
    if base.join("auth.json").exists() && !matches!(auth_store, Some("keyring" | "ephemeral")) {
        root.insert(
            "cli_auth_credentials_store".into(),
            toml::Value::String("file".into()),
        );
    }

    if let Some(previous_state) = previous_hook_state {
        let state = child_table(child_table(root, "hooks"), "state");
        state.extend(previous_state);
    }

    let marketplaces = child_table(root, "marketplaces");
    marketplaces.remove("prometeu");
    marketplaces.remove("prometeu-dev");
    let mut source = toml::map::Map::new();
    source.insert("source_type".into(), toml::Value::String("local".into()));
    source.insert(
        "source".into(),
        toml::Value::String(marketplace.display().to_string()),
    );
    marketplaces.insert(codex_marketplace_name().into(), toml::Value::Table(source));

    let plugins = child_table(root, "plugins");
    for (id, value) in previous_plugins {
        if prometeu_plugin(&id) {
            plugins.insert(id, value);
        }
    }
    for (id, value) in plugins.iter_mut() {
        if prometeu_plugin(id) {
            if !value.is_table() {
                *value = toml::Value::Table(toml::map::Map::new());
            }
            value
                .as_table_mut()
                .expect("table inserted above")
                .insert("enabled".into(), toml::Value::Boolean(false));
        }
    }
    for id in selected {
        child_table(plugins, id).insert("enabled".into(), toml::Value::Boolean(true));
    }

    let body = toml::to_string_pretty(&config)
        .map_err(|error| i18n::ta("err.plugin.codex.config", &[("cause", error.to_string())]))?;
    paths::write_private(&home.join("config.toml"), &body)
        .map_err(|cause| i18n::ta("err.plugin.codex.config", &[("cause", cause)]))
}

/// Copy packages without modifying their Claude sources. Add a content hash to the derived version
/// so upstream changes invalidate cache even without a version bump.
fn prepare_marketplace(
    root: &Path,
    marketplace: &str,
    plugins: &[Plugin],
) -> Result<Vec<PreparedPlugin>, String> {
    let plugin_root = root.join("plugins");
    paths::ensure_private_dir(&plugin_root)
        .map_err(|cause| i18n::ta("err.plugin.codex.prepare", &[("cause", cause)]))?;
    let mut prepared = Vec::new();
    let mut entries = Vec::new();
    for plugin in plugins {
        if plugin.id.len() > 64 || slug(&plugin.id) != plugin.id {
            return Err(i18n::ta(
                "err.plugin.codex.name",
                &[("name", plugin.id.clone())],
            ));
        }
        if remote(&plugin.source) {
            return Err(i18n::ta(
                "err.plugin.codex.source",
                &[("name", plugin.id.clone())],
            ));
        }
        let source = PathBuf::from(expand(&plugin.source));
        if !source.is_dir() {
            return Err(i18n::ta(
                "err.plugin.codex.source",
                &[("name", plugin.id.clone())],
            ));
        }
        let source = source.canonicalize().map_err(|error| {
            i18n::ta("err.plugin.codex.prepare", &[("cause", error.to_string())])
        })?;
        if root.starts_with(&source) {
            return Err(i18n::ta(
                "err.plugin.codex.prepare",
                &[("cause", "plugin source contains the adapter cache".into())],
            ));
        }
        let fingerprint = codex_package_fingerprint(&source)?;
        let version = portable_version(&source, &fingerprint);
        let target = plugin_root.join(&plugin.id);
        stage_plugin(&source, &target, &plugin.id, &version, &fingerprint)?;
        let canonical = format!("{}@{marketplace}", plugin.id);
        entries.push(serde_json::json!({
            "name": plugin.id,
            "source": { "source": "local", "path": format!("./plugins/{}", plugin.id) },
            "policy": { "installation": "AVAILABLE", "authentication": "ON_USE" },
            "category": "Productivity",
        }));
        prepared.push(PreparedPlugin {
            id: plugin.id.clone(),
            canonical,
            version,
            hooks: plugin_has_hooks(&target),
        });
    }
    let marketplace_path = root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let body = serde_json::to_string_pretty(&serde_json::json!({
        "name": marketplace,
        "interface": { "displayName": "Prometeu" },
        "plugins": entries,
    }))
    .map_err(|error| error.to_string())?;
    paths::write_private(&marketplace_path, &body)
        .map_err(|cause| i18n::ta("err.plugin.codex.prepare", &[("cause", cause)]))?;
    Ok(prepared)
}

fn codex_package_fingerprint(root: &Path) -> Result<String, String> {
    let source = fingerprint(root)?;
    let mut hash = Sha256::new();
    hash.update(b"prometeu-codex-package\0");
    hash.update(CODEX_PACKAGE_REVISION.as_bytes());
    hash.update(b"\0");
    hash.update(source.as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

/// A declared hook remains required even when its path is broken; fail startup rather than treating
/// the package as skills only. Without an explicit field, check the native hooks/hooks.json
/// convention.
fn plugin_has_hooks(root: &Path) -> bool {
    let declared = read_json(&root.join(".codex-plugin").join("plugin.json"))
        .and_then(|manifest| manifest.get("hooks").cloned());
    match declared {
        Some(Value::String(path)) => !path.trim().is_empty(),
        Some(Value::Array(paths)) => !paths.is_empty(),
        Some(Value::Object(hooks)) => !hooks.is_empty(),
        Some(Value::Null) | None => root.join("hooks").join("hooks.json").is_file(),
        Some(_) => true,
    }
}

fn fingerprint(root: &Path) -> Result<String, String> {
    fn visit(root: &Path, at: &Path, hash: &mut Sha256) -> std::io::Result<()> {
        let mut entries = std::fs::read_dir(at)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            hash.update(rel.to_string_lossy().as_bytes());
            let kind = entry.file_type()?;
            if kind.is_dir() {
                visit(root, &path, hash)?;
            } else if kind.is_symlink() {
                hash.update(std::fs::read_link(&path)?.to_string_lossy().as_bytes());
            } else if kind.is_file() {
                let mut file = std::fs::File::open(path)?;
                let mut chunk = [0_u8; 16 * 1024];
                loop {
                    let read = file.read(&mut chunk)?;
                    if read == 0 {
                        break;
                    }
                    hash.update(&chunk[..read]);
                }
            }
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    visit(root, root, &mut hash)
        .map_err(|error| i18n::ta("err.plugin.codex.prepare", &[("cause", error.to_string())]))?;
    Ok(format!("{:x}", hash.finalize()))
}

fn portable_version(source: &Path, fingerprint: &str) -> String {
    let native = read_json(&source.join(".codex-plugin").join("plugin.json"));
    let claude = read_json(&manifest_path(source));
    let version = [native.as_ref(), claude.as_ref()]
        .into_iter()
        .flatten()
        .find_map(|manifest| manifest.get("version").and_then(Value::as_str))
        .unwrap_or("0.0.0")
        .trim();
    let candidate = version
        .split('+')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or("0.0.0");
    let base = semver::Version::parse(candidate)
        .map(|version| version.to_string())
        .unwrap_or_else(|_| "0.0.0".into());
    format!("{base}+prometeu.{}", &fingerprint[..16])
}

fn stage_plugin(
    source: &Path,
    target: &Path,
    id: &str,
    version: &str,
    fingerprint: &str,
) -> Result<(), String> {
    let marker = target.with_file_name(format!(".{id}.source-hash"));
    if target.is_dir() && std::fs::read_to_string(&marker).ok().as_deref() == Some(fingerprint) {
        return Ok(());
    }
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let temporary = parent.join(format!(".{id}.{}.tmp", uuid::Uuid::new_v4()));
    if let Err(error) = copy_tree(source, &temporary) {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(i18n::ta(
            "err.plugin.codex.prepare",
            &[("cause", error.to_string())],
        ));
    }
    if let Err(error) = write_portable_manifest(&temporary, id, version) {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if target.exists() {
        std::fs::remove_dir_all(target).map_err(|error| {
            i18n::ta("err.plugin.codex.prepare", &[("cause", error.to_string())])
        })?;
    }
    std::fs::rename(&temporary, target).map_err(|error| {
        let _ = std::fs::remove_dir_all(&temporary);
        i18n::ta("err.plugin.codex.prepare", &[("cause", error.to_string())])
    })?;
    paths::write_private(&marker, fingerprint)
        .map_err(|cause| i18n::ta("err.plugin.codex.prepare", &[("cause", cause)]))
}

fn copy_tree(source: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        let from = entry.path();
        let to = target.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to)?;
        } else if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
            #[cfg(not(unix))]
            if from.is_dir() {
                copy_tree(&from, &to)?;
            } else {
                std::fs::copy(from, to)?;
            }
        } else if kind.is_file() {
            std::fs::copy(from, to)?;
        }
    }
    Ok(())
}

/// Overlay native Codex manifest fields onto the compatible Claude manifest. Generate missing
/// native fields only in the derived copy.
fn write_portable_manifest(root: &Path, id: &str, version: &str) -> Result<(), String> {
    let claude = read_json(&manifest_path(root));
    let native_path = root.join(".codex-plugin").join("plugin.json");
    let native = read_json(&native_path);
    let mut merged = serde_json::Map::new();
    if let Some(fields) = claude.as_ref().and_then(Value::as_object) {
        for (key, value) in fields {
            // Wrap Claude's inline event map in Codex's HooksFile envelope. String paths use the
            // same representation in both formats.
            let value = if key == "hooks" {
                codex_hooks(value)
            } else {
                value.clone()
            };
            merged.insert(key.clone(), value);
        }
    }
    if let Some(fields) = native.as_ref().and_then(Value::as_object) {
        for (key, value) in fields {
            // An explicit native overlay already has the required Codex shape.
            merged.insert(key.clone(), value.clone());
        }
    }
    merged.insert("name".into(), Value::String(id.to_string()));
    merged.insert("version".into(), Value::String(version.to_string()));
    if !merged.contains_key("skills") && root.join("skills").is_dir() {
        merged.insert("skills".into(), Value::String("./skills/".into()));
    }
    if !merged.contains_key("commands") && root.join("commands").is_dir() {
        merged.insert("commands".into(), Value::String("./commands/".into()));
    }
    if !merged.contains_key("mcpServers") {
        if let Some(name) = [".mcp.json", "mcp.json"]
            .into_iter()
            .find(|name| root.join(name).is_file())
        {
            merged.insert("mcpServers".into(), Value::String(format!("./{name}")));
        }
    }
    let body =
        serde_json::to_string_pretty(&Value::Object(merged)).map_err(|error| error.to_string())?;
    paths::write_private(&native_path, &body)
        .map_err(|cause| i18n::ta("err.plugin.codex.prepare", &[("cause", cause)]))
}

fn codex_hooks(hooks: &Value) -> Value {
    match hooks {
        Value::Object(fields) if !fields.contains_key("hooks") => {
            serde_json::json!({ "hooks": hooks })
        }
        _ => hooks.clone(),
    }
}

fn codex_command(home: &Path) -> Command {
    let mut command = Command::new("codex");
    command
        .env("CODEX_HOME", home)
        .args(["--enable", "plugins", "--enable", "hooks"]);
    command
}

fn codex_installed(home: &Path) -> Result<HashMap<String, InstalledPlugin>, String> {
    let output = codex_command(home)
        .args([
            "plugin",
            "list",
            "--marketplace",
            codex_marketplace_name(),
            "--available",
            "--json",
        ])
        .output()
        .map_err(|error| i18n::ta("err.plugin.codex.cli", &[("cause", error.to_string())]))?;
    if !output.status.success() {
        return Err(i18n::ta(
            "err.plugin.codex.cli",
            &[("cause", last_line(&String::from_utf8_lossy(&output.stderr)))],
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| i18n::ta("err.plugin.codex.cli", &[("cause", error.to_string())]))?;
    Ok(value["installed"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some((
                entry["pluginId"].as_str()?.to_string(),
                InstalledPlugin {
                    version: entry["version"].as_str().unwrap_or_default().to_string(),
                },
            ))
        })
        .collect())
}

fn codex_install(home: &Path, canonical: &str) -> Result<(), String> {
    let output = codex_command(home)
        .args(["plugin", "add", canonical, "--json"])
        .output()
        .map_err(|error| i18n::ta("err.plugin.codex.cli", &[("cause", error.to_string())]))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(i18n::ta(
            "err.plugin.codex.cli",
            &[("cause", last_line(&String::from_utf8_lossy(&output.stderr)))],
        ))
    }
}

fn codex_remove(home: &Path, canonical: &str) {
    let _ = codex_command(home)
        .args(["plugin", "remove", canonical, "--json"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn forget_codex_config(home: &Path, canonical: &str) {
    let Ok(mut config) = read_toml(&home.join("config.toml")) else {
        return;
    };
    let Some(root) = config.as_table_mut() else {
        return;
    };
    if let Some(plugins) = root.get_mut("plugins").and_then(toml::Value::as_table_mut) {
        plugins.remove(canonical);
    }
    if let Ok(body) = toml::to_string_pretty(&config) {
        let _ = paths::write_private(&home.join("config.toml"), &body);
    }
}

fn remove_marketplace_entry(home: &Path, id: &str) {
    let marketplace = codex_marketplace_root(home);
    let target = marketplace.join("plugins").join(id);
    if target.starts_with(&marketplace) && target != marketplace {
        std::fs::remove_dir_all(&target).ok();
        std::fs::remove_file(
            marketplace
                .join("plugins")
                .join(format!(".{id}.source-hash")),
        )
        .ok();
    }
    let catalogue = marketplace
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let Some(mut value) = read_json(&catalogue) else {
        return;
    };
    let Some(entries) = value.get_mut("plugins").and_then(Value::as_array_mut) else {
        return;
    };
    entries.retain(|entry| entry.get("name").and_then(Value::as_str) != Some(id));
    if let Ok(body) = serde_json::to_string_pretty(&value) {
        let _ = paths::write_private(&catalogue, &body);
    }
}

/// Best-effort removal clears the shared installation and references in derived homes. Later
/// startup rebuilds configuration from the updated hub if cleanup fails.
fn codex_remove_everywhere(canonical: &str) {
    let id = canonical.split_once('@').map_or(canonical, |(id, _)| id);
    for home in codex_homes_at(&codex_workspaces_root()) {
        codex_remove(&home, canonical);
        forget_codex_config(&home, canonical);
        remove_marketplace_entry(&home, id);
    }
}

fn codex_homes_at(root: &Path) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    for workspace in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if !workspace.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let home = workspace.path();
        if home.join("config.toml").is_file() {
            homes.push(home.clone());
        }
        for account in std::fs::read_dir(home).into_iter().flatten().flatten() {
            if account.file_type().is_ok_and(|kind| kind.is_dir())
                && uuid::Uuid::parse_str(&account.file_name().to_string_lossy()).is_ok()
                && account.path().join("config.toml").is_file()
            {
                homes.push(account.path());
            }
        }
    }
    homes
}

/* Installation */

/// Report the clone path and discovered plugins. Automatically register a single plugin; a
/// marketplace requires explicit selection before registration.
#[derive(serde::Serialize)]
pub struct Found {
    pub dir: String,
    pub plugins: Vec<Plugin>,
    pub saved: bool,
}

/// Clone and inspect asynchronously so repository downloads do not block the window.
#[tauri::command(async)]
pub fn plugin_install(source: String) -> Result<Found, String> {
    let _sync = crate::catalog::guard();
    install(source)
}

fn install(source: String) -> Result<Found, String> {
    install_into(source, true)
}

fn install_into(source: String, register: bool) -> Result<Found, String> {
    let url = git_url(&source);
    if url.is_empty() {
        return Err(i18n::t("err.plugin.noSource"));
    }
    let dir = store().join(repo_name(&url));
    if dir.exists() {
        // An occupied clone used by hub entries requires updating. An unused clone left by an
        // abandoned selection may be replaced.
        if lives_in(&dir) {
            return Err(i18n::ta("err.plugin.exists", &[("name", repo_name(&url))]));
        }
        std::fs::remove_dir_all(&dir).ok();
    }
    std::fs::create_dir_all(store())
        .map_err(|e| i18n::ta("err.plugin.clone", &[("cause", e.to_string())]))?;
    clone(&url, &dir)?;
    let plugins = plugins_in(&dir, &url);
    if plugins.is_empty() {
        std::fs::remove_dir_all(&dir).ok();
        return Err(i18n::ta("err.plugin.noPluginIn", &[("url", url)]));
    }
    // Installing a repository containing one plugin already selects that plugin.
    let saved = plugins.len() == 1;
    if saved && register {
        if load().iter().any(|p| p.id == plugins[0].id) {
            return Err(i18n::t("err.catalog.conflict"));
        }
        save_local(plugins[0].clone())?;
    }
    Ok(Found {
        dir: dir.display().to_string(),
        plugins,
        saved,
    })
}

/// Install only the selected catalog item while preserving local names.
pub(crate) fn install_catalog(
    source: &str,
    expected_id: &str,
    local_id: &str,
    note: &str,
) -> Result<(), String> {
    if source.to_lowercase().ends_with(".zip") && remote(source) {
        return save_local(Plugin {
            id: local_id.into(),
            source: source.into(),
            note: note.into(),
            made: false,
            from: String::new(),
        })
        .map(|_| ());
    }
    // Reuse an existing clone belonging to another item in the same catalog.
    let dir = store().join(repo_name(&git_url(source)));
    let candidates = if dir.exists() && lives_in(&dir) {
        let root = git_root(&dir).ok_or_else(|| i18n::t("err.catalog.conflict"))?;
        let origin = git(&root, &["remote", "get-url", "origin"])?;
        if !origin.status.success()
            || String::from_utf8_lossy(&origin.stdout).trim() != git_url(source)
        {
            return Err(i18n::t("err.catalog.conflict"));
        }
        plugins_in(&dir, &git_url(source))
    } else {
        install_into(source.into(), false)?.plugins
    };
    let mut plugin = candidates
        .iter()
        .find(|p| p.id == expected_id)
        .or_else(|| {
            if candidates.len() == 1 {
                candidates.first()
            } else {
                None
            }
        })
        .cloned()
        .ok_or_else(|| i18n::t("err.catalog.invalid"))?;
    plugin.id = local_id.into();
    plugin.note = note.into();
    // The clone may contain other plugins, so removing this entry must not delete it.
    plugin.made = false;
    save_local(plugin).map(|_| ())
}

/// Discard abandoned selection clones only inside application-owned storage and only when no hub
/// entry uses them.
#[tauri::command]
pub fn plugin_scrap(dir: String) {
    let dir = PathBuf::from(expand(&dir));
    if dir.starts_with(store()) && dir != store() && !lives_in(&dir) {
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Update application-owned clones with git pull --ff-only, preserving manual edits on divergence.
/// Reread manifest descriptions after updates.
#[tauri::command(async)]
pub fn plugin_update(id: String) -> Result<Vec<Plugin>, String> {
    let _sync = crate::catalog::guard();
    let plugin = load()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| i18n::t("err.plugin.gone"))?;
    let dir = PathBuf::from(expand(&plugin.source));
    let root = git_root(&dir).ok_or_else(|| i18n::t("err.plugin.noGit"))?;
    let out = git(&root, &["pull", "--ff-only", "-q"])?;
    if !out.status.success() {
        return Err(i18n::ta(
            "err.plugin.pull",
            &[("cause", last_line(&String::from_utf8_lossy(&out.stderr)))],
        ));
    }
    let fresh = read_plugin(&dir, &plugin.from);
    if fresh.id == plugin.id {
        return save_local(fresh);
    }
    Ok(load())
}

/// Check whether any hub plugin uses this clone directory.
fn lives_in(dir: &Path) -> bool {
    load()
        .iter()
        .any(|p| PathBuf::from(expand(&p.source)).starts_with(dir))
}

/// Expand owner/repo as GitHub shorthand and remove browser tree/branch suffixes. Preserve other
/// Git URLs for GitLab, Bitbucket, and SSH transports.
pub(crate) fn git_url(source: &str) -> String {
    let mut text = source.trim().trim_end_matches('/');
    if let Some(cut) = text.find("/tree/") {
        text = &text[..cut];
    }
    let bare = text
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    if text.contains("://") || text.contains('@') {
        return text.to_string();
    }
    let path = bare.strip_prefix("github.com/").unwrap_or(bare);
    // Accept only the exact owner/repo shorthand shape.
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [owner, repo] => format!("https://github.com/{owner}/{repo}"),
        _ => String::new(),
    }
}

/// Use the repository name for its clone directory.
fn repo_name(url: &str) -> String {
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    slug(name)
}

/// Use a shallow noninteractive clone so plugins do not download unnecessary history or wait
/// indefinitely for terminal credentials.
fn clone(url: &str, dir: &Path) -> Result<(), String> {
    let out = Command::new("git")
        .args(["clone", "--depth", "1", "-q", url])
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes")
        .output()
        .map_err(|e| i18n::ta("err.plugin.clone", &[("cause", e.to_string())]))?;
    if out.status.success() {
        return Ok(());
    }
    std::fs::remove_dir_all(dir).ok();
    Err(i18n::ta(
        "err.plugin.clone",
        &[("cause", last_line(&String::from_utf8_lossy(&out.stderr)))],
    ))
}

fn git(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes")
        .output()
        .map_err(|e| i18n::ta("err.plugin.pull", &[("cause", e.to_string())]))
}

/// Find the clone root for marketplace plugins stored below it; updates must run where .git
/// belongs.
fn git_root(dir: &Path) -> Option<PathBuf> {
    let mut at = dir;
    loop {
        if at.join(".git").exists() {
            return Some(at.to_path_buf());
        }
        at = at.parent()?;
        if !at.starts_with(store()) {
            return None;
        }
    }
}

/// Use the final Git error line as a compact diagnostic.
fn last_line(text: &str) -> String {
    text.trim()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// Discover a root plugin, local marketplace entries, or one level of child/plugin directories.
/// Remote marketplace entries require their own installation and must not escape this clone.
fn plugins_in(dir: &Path, from: &str) -> Vec<Plugin> {
    if manifest_path(dir).exists() {
        return vec![read_plugin(dir, from)];
    }
    let mut found: Vec<Plugin> = Vec::new();
    for market in [
        dir.join(".agents").join("plugins").join("marketplace.json"),
        dir.join(".claude-plugin").join("marketplace.json"),
    ]
    .iter()
    .filter_map(|path| read_json(path))
    {
        for entry in market
            .get("plugins")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(rel) = marketplace_local_source(entry) else {
                continue;
            };
            if let Some(at) = within(dir, rel) {
                if manifest_path(&at).exists() {
                    found.push(read_plugin(&at, from));
                }
            }
        }
    }
    if found.is_empty() {
        found = scan(dir, from);
    }
    found.sort_by_key(|p| p.id.to_lowercase());
    found.dedup_by(|a, b| a.source == b.source);
    found
}

/// Accept Claude string sources and Codex local-source objects, following only paths inside the
/// clone.
fn marketplace_local_source(entry: &Value) -> Option<&str> {
    match entry.get("source")? {
        Value::String(path) => Some(path),
        Value::Object(source) if source.get("source")?.as_str()? == "local" => {
            source.get("path")?.as_str()
        }
        _ => None,
    }
}

/// Inspect immediate children and plugins/ without recursively scanning the entire repository.
fn scan(dir: &Path, from: &str) -> Vec<Plugin> {
    let mut found = Vec::new();
    for root in [dir.to_path_buf(), dir.join("plugins")] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let at = entry.path();
            if at.is_dir() && manifest_path(&at).exists() {
                found.push(read_plugin(&at, from));
            }
        }
    }
    found
}

/// Confine marketplace paths to the clone; reject traversal to unrelated disk locations.
fn within(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        return Some(dir.to_path_buf());
    }
    let at = dir.join(rel);
    (!rel.starts_with('/') && !at.components().any(|c| c.as_os_str() == "..")).then_some(at)
}

/// Read hub metadata from the manifest, falling back to the folder name when no name is declared.
fn read_plugin(dir: &Path, from: &str) -> Plugin {
    let manifest = read_json(&manifest_path(dir));
    let text = |key: &str| {
        manifest
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let id = match text("name") {
        name if !name.is_empty() => name,
        _ => dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
    };
    Plugin {
        id,
        source: dir.display().to_string(),
        note: text("description"),
        made: true,
        from: from.to_string(),
    }
}

/* Plugin creation */

/// Only plugins under application-owned creation storage may be deleted with their registration.
/// Manually maintained repository directories stay outside that ownership.
pub fn store() -> PathBuf {
    paths::root().join("plugins")
}

/// Use the stable sonnet alias for package creation because valid frontmatter and manifests require
/// more than a cheap title-generation model.
const MAKER_MODEL: &str = "sonnet";

/// Bound creation time so an abandoned process cannot keep writing indefinitely.
const MAKER_TIMEOUT: Duration = Duration::from_secs(600);

/// Limit creation requests to a plugin-sized scope.
const MAX_ASK: usize = 4000;

/// Keep progress messages short enough for a single line.
const MAX_STEP: usize = 140;

/// Specify required manifests, frontmatter, and portable hook paths in the system instructions.
/// Append the person's request separately so creation stays scoped to a plugin package.
const MAKER: &str = r#"Você escreve um plugin portátil para Claude Code e Codex, do zero, dentro da pasta em que está — e nada além disso.

O formato compartilhado, que os dois CLIs vão ler:

- `.claude-plugin/plugin.json`, obrigatório: {"name": "<NOME>", "description": "…", "version": "0.1.0", "hooks": "./hooks/hooks.json"}. O `name` tem que ser exatamente <NOME>. Omita `hooks` se o plugin não tiver hook.
- `.codex-plugin/plugin.json`, obrigatório, com o mesmo `name`, `description` e `version`. Aponte recursos existentes com caminhos relativos iniciados por `./`: `"skills": "./skills/"`, `"commands": "./commands/"`, `"mcpServers": "./.mcp.json"` e `"hooks": "./hooks/hooks.json"`; omita o que não existir.
- `skills/<assunto>/SKILL.md`: a instrução que o agente carrega quando o assunto aparece. Frontmatter YAML com `name` e `description`; é a `description` que decide se a skill é carregada, então diga nela quando usar.
- `commands/<nome>.md`: um comando de barra. Frontmatter opcional com `description` e `argument-hint`; o corpo é o prompt, e `$ARGUMENTS` recebe o que a pessoa escreveu depois do comando.
- `.mcp.json`: servidores empacotados, no formato `{"mcpServers":{"nome":{"command":"…","args":[]}}}`. Use só quando o plugin realmente precisa iniciar um servidor.
- `agents/<nome>.md`: subagente exclusivo do Claude. Frontmatter com `name`, `description` e, se for o caso, `tools`. Quando o mesmo fluxo precisar existir nos dois CLIs, escreva uma skill em vez de um agent.
- `hooks/hooks.json`: o que roda sozinho, turno após turno, sem depender da atenção do modelo. Formato:
  {"hooks": {"UserPromptSubmit": [{"hooks": [{"type": "command", "command": "sh ${CLAUDE_PLUGIN_ROOT}/hooks/nome.sh"}]}]}}
  Os eventos comuns aos dois são PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Stop, SubagentStop e PreCompact; PreToolUse e PostToolUse aceitam `matcher` com o nome da ferramenta. O script recebe um JSON no stdin, e o que ele escreve no stdout de UserPromptSubmit e de SessionStart entra na conversa como contexto. Chame todo script por `sh` ou por `python3` — nunca conte com bit de execução. `${CLAUDE_PLUGIN_ROOT}` é preenchido pelos dois CLIs com a pasta do plugin em qualquer máquina: nunca escreva caminho absoluto.

Selecionar o plugin já é ativá-lo. Se o pedido descreve um modo contínuo — estilo, persona, política ou comportamento para toda a conversa — crie um hook `SessionStart` que escreva a instrução completa no stdout desde a primeira resposta. Use também `UserPromptSubmit` quando for importante reforçá-la a cada turno. Não exija comando de barra, menção à skill ou uma segunda ativação para iniciar esse modo, e não crie opção desligada por padrão salvo se a pessoa pedir isso explicitamente.

Escreva só o que o pedido pede: um plugin de uma skill é uma skill, e não um pacote de exemplos. Nada de README, LICENSE, .gitignore, teste ou CHANGELOG. Não rode comando, não instale nada, não use a rede. Ao terminar, responda em uma linha só o que o plugin faz."#;

/// Progress is either a written file or agent text. The frontend localizes file-event framing;
/// agent output remains unchanged.
#[derive(Clone, serde::Serialize)]
pub struct Step {
    pub kind: String,
    pub text: String,
}

/// Return the creation event identity and generated directory name to the UI.
#[derive(serde::Serialize)]
pub struct Make {
    pub run: u64,
    pub slug: String,
}

/// Track active creation processes by run ID so cancellation or an old timeout cannot stop a newer
/// run.
fn running() -> &'static Mutex<HashMap<u64, Child>> {
    static RUNS: OnceLock<Mutex<HashMap<u64, Child>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

fn next_run() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Return immediately and report creation progress through plugin-make and completion or failure
/// through plugin-made.
#[tauri::command]
pub fn plugin_make(app: AppHandle, name: String, ask: String) -> Result<Make, String> {
    let slug = slug(&name);
    let ask: String = ask.trim().chars().take(MAX_ASK).collect();
    if slug.is_empty() {
        return Err(i18n::t("err.plugin.noName"));
    }
    if ask.is_empty() {
        return Err(i18n::t("err.plugin.noAsk"));
    }
    let dir = store().join(&slug);
    if dir.exists() || load().iter().any(|p| p.id == slug) {
        return Err(i18n::ta("err.plugin.exists", &[("name", slug)]));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| i18n::ta("err.plugin.make", &[("cause", e.to_string())]))?;
    let run = next_run();
    let generation = lock(&app.state::<crate::AppState>().telemetry).generation;
    let (slug, place) = (slug, dir.clone());
    let mine = slug.clone();
    std::thread::spawn(move || {
        let end = make(&app, run, &place, &mine, &ask, generation);
        // Remove incomplete package directories that cannot appear as valid hub entries.
        if end.is_err() {
            std::fs::remove_dir_all(&place).ok();
        }
        let _ = app.emit("plugin-made", (run, end.err().unwrap_or_default()));
    });
    Ok(Make { run, slug })
}

/// Cancellation stops the process, closes its output, and uses normal failure cleanup to remove the
/// partial package.
#[tauri::command]
pub fn plugin_make_stop(run: u64) {
    stop(run);
}

fn stop(run: u64) {
    if let Some(mut child) = lock(running()).remove(&run) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Create with claude -p inside the new directory, allowing only file tools and disabling user
/// hooks, plugins, MCP, and shell execution. Outside-directory writes remain subject to CLI
/// approval, unavailable in this headless flow.
fn make(
    app: &AppHandle,
    run: u64,
    dir: &Path,
    slug: &str,
    ask: &str,
    generation: u64,
) -> Result<(), String> {
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "--model",
        MAKER_MODEL,
        "--setting-sources",
        "",
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        "--permission-mode",
        "acceptEdits",
        "--allowedTools",
        "Read Write Edit Glob Grep",
        "--output-format",
        "stream-json",
        "--verbose",
        "--system-prompt",
    ]);
    cmd.arg(MAKER.replace("<NOME>", slug));
    cmd.arg(ask);
    cmd.current_dir(dir);
    // Remove inherited Claude child-session settings belonging to the parent process.
    for (k, _) in std::env::vars() {
        if k.starts_with("CLAUDE") {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let profile = crate::accounts::active(crate::state::ProviderId::Claude)?;
    profile.prepare()?;
    profile.apply(&mut cmd)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| i18n::ta("err.plugin.make", &[("cause", e.to_string())]))?;
    let state = app.state::<crate::AppState>();
    let mut capture = crate::telemetry::AppCapture::start(
        &state.telemetry,
        generation,
        crate::telemetry::Scope {
            provider: Some("claude".into()),
            ..Default::default()
        },
        crate::telemetry::AppSource::PluginMaker,
        Some(MAKER_MODEL.into()),
    );
    let mut adapter = crate::claude::Adapter::fresh();
    let out = child.stdout.take();
    lock(running()).insert(run, child);
    watch(run);
    if let Some(out) = out {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                for frame in adapter.translate(&value) {
                    capture.observe(&frame);
                }
            }
            if let Some(step) = step(dir, &line) {
                let _ = app.emit("plugin-make", (run, step));
            }
        }
    }
    let ended = lock(running())
        .remove(&run)
        .and_then(|mut child| child.wait().ok())
        .map(|status| status.success())
        .unwrap_or(false);
    crate::telemetry::notify_changed(app, || capture.finish(&state.telemetry, ended));
    if !ended {
        return Err(i18n::t("err.plugin.make.failed"));
    }
    born(dir, slug)
}

/// Apply the timeout only while this run ID remains active; completed runs have already left the
/// map.
fn watch(run: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(MAKER_TIMEOUT);
        stop(run);
    });
}

/// Register only packages with a readable manifest, taking their name and description from that
/// manifest.
fn born(dir: &Path, slug: &str) -> Result<(), String> {
    let manifest =
        read_json(&manifest_path(dir)).ok_or_else(|| i18n::t("err.plugin.made.empty"))?;
    let native = read_json(&dir.join(".codex-plugin").join("plugin.json"))
        .ok_or_else(|| i18n::t("err.plugin.made.empty"))?;
    let text = |key: &str| {
        manifest
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let id = match text("name") {
        name if !name.is_empty() => name,
        _ => slug.to_string(),
    };
    if native.get("name").and_then(Value::as_str) != Some(id.as_str()) {
        return Err(i18n::t("err.plugin.made.empty"));
    }
    save_local(Plugin {
        id,
        source: dir.display().to_string(),
        note: text("description"),
        made: true,
        from: String::new(),
    })?;
    Ok(())
}

/// Translate stream-json progress into written-file or agent-message events. Reads, usage, and
/// result frames are not creation progress.
fn step(dir: &Path, line: &str) -> Option<Step> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str)? != "assistant" {
        return None;
    }
    for item in value.get("message")?.get("content")?.as_array()? {
        match item.get("type").and_then(Value::as_str) {
            Some("tool_use") => {
                let wrote = matches!(
                    item.get("name").and_then(Value::as_str),
                    Some("Write") | Some("Edit")
                );
                let path = item
                    .get("input")
                    .and_then(|input| input.get("file_path"))
                    .and_then(Value::as_str);
                if let (true, Some(path)) = (wrote, path) {
                    return Some(Step {
                        kind: "file".into(),
                        text: inside(dir, path),
                    });
                }
            }
            Some("text") => {
                let said = item.get("text").and_then(Value::as_str).unwrap_or_default();
                if let Some(first) = said.lines().map(str::trim).find(|l| !l.is_empty()) {
                    return Some(Step {
                        kind: "say".into(),
                        text: first.chars().take(MAX_STEP).collect(),
                    });
                }
            }
            _ => {}
        }
    }
    None
}

/// Display written paths relative to the plugin, such as skills/x/SKILL.md.
fn inside(dir: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(dir)
        .unwrap_or(Path::new(path))
        .display()
        .to_string()
}

/// Normalize names for directories and CLI identities: lowercase, no spaces or accents.
/// Transliterate supported accented letters rather than replacing them with separators.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for ch in name.trim().to_lowercase().chars().map(fold) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Transliterate common Portuguese and Spanish accents; this is not full Unicode normalization.
fn fold(ch: char) -> char {
    match ch {
        'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_install_keeps_private_names_and_reuses_clone() {
        // Use an isolated process so PROMETEU_ROOT changes cannot race parallel tests.
        if std::env::var("PROMETEU_CATALOG_TEST_CHILD").as_deref() != Ok("1") {
            let root = std::env::temp_dir()
                .join(format!("prometeu-catalog-install-{}", uuid::Uuid::new_v4()));
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "plugins::tests::catalog_install_keeps_private_names_and_reuses_clone",
                    "--nocapture",
                ])
                .env("PROMETEU_CATALOG_TEST_CHILD", "1")
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
        let root = paths::root();
        let source = root.join("source-review");
        let private = root.join("private-review");
        for directory in [&source, &private] {
            paths::write_private(
                &directory.join(".claude-plugin/plugin.json"),
                r#"{"name":"review","description":"fixture"}"#,
            )
            .unwrap();
        }
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            // The fixture identity has no signing key; a contributor who signs every commit by
            // default would otherwise fail here.
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
        ] {
            assert!(Command::new("git")
                .current_dir(&source)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        save_local(plugin("review", private.to_str().unwrap())).unwrap();
        let url = format!("file://{}", source.display());
        install_catalog(&url, "review", "cloud-review-1", "from cloud").unwrap();
        let first = load();
        assert_eq!(first.len(), 2);
        assert_eq!(
            first.iter().find(|p| p.id == "review").unwrap().source,
            private.to_string_lossy()
        );
        let installed = first.iter().find(|p| p.id == "cloud-review-1").unwrap();
        assert_eq!(installed.from, url);
        assert!(!installed.made);
        let installed_path = PathBuf::from(&installed.source);
        // Reinstallation reuses the clone and preserves a private entry's local name.
        install_catalog(&url, "review", "cloud-review-1", "updated note").unwrap();
        assert_eq!(load().len(), 2);
        assert_eq!(
            load()
                .iter()
                .find(|p| p.id == "cloud-review-1")
                .unwrap()
                .note,
            "updated note"
        );
        remove_hub("cloud-review-1").unwrap();
        assert!(installed_path.exists());
        assert!(private.exists());
        assert_eq!(load().len(), 1);
    }

    #[test]
    fn cleanup_finds_all_accounts_without_following_links() {
        let root =
            std::env::temp_dir().join(format!("prometeu-account-plugins-{}", uuid::Uuid::new_v4()));
        let home = root.join("workspace");
        let account = home.join(uuid::Uuid::new_v4().to_string());
        paths::ensure_private_dir(&account).unwrap();
        std::fs::write(home.join("config.toml"), "").unwrap();
        std::fs::write(account.join("config.toml"), "").unwrap();
        std::os::unix::fs::symlink(&account, home.join(uuid::Uuid::new_v4().to_string())).unwrap();
        let homes = codex_homes_at(&root);
        assert_eq!(homes.len(), 2);
        assert!(homes.contains(&home));
        assert!(homes.contains(&account));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn plugin(id: &str, source: &str) -> Plugin {
        Plugin {
            id: id.into(),
            source: source.into(),
            note: String::new(),
            made: false,
            from: String::new(),
        }
    }

    /// Directories use plugin-dir; remote sources use plugin-url.
    #[test]
    fn source_determines_the_flag() {
        assert_eq!(
            flags(&plugin("caveman", "/opt/caveman")),
            ["--plugin-dir".to_string(), "/opt/caveman".to_string()]
        );
        assert_eq!(
            flags(&plugin("x", "https://example.com/x.zip")),
            [
                "--plugin-url".to_string(),
                "https://example.com/x.zip".to_string()
            ]
        );
    }

    /// Expand tilde into this machine's absolute home path before invoking Claude.
    #[test]
    fn expands_tilde_in_paths() {
        let [_, path] = flags(&plugin("x", "~/plugins/x"));
        assert_eq!(path, paths::home().join("plugins/x").display().to_string());
        assert!(!path.starts_with('~'));
    }

    /// Preserve selection order and emit one argument pair per existing selected plugin. Skip IDs
    /// removed from the hub rather than preventing conversation startup.
    #[test]
    fn selection_becomes_command_line_arguments() {
        let hub = vec![
            plugin("caveman", "/opt/caveman"),
            plugin("ponytail", "https://example.com/ponytail.zip"),
        ];
        let chosen = ["ponytail".to_string(), "apagado".into(), "caveman".into()];
        assert_eq!(
            args_from(&hub, &chosen),
            [
                "--plugin-url",
                "https://example.com/ponytail.zip",
                "--plugin-dir",
                "/opt/caveman",
            ]
        );
        // An explicit empty selection emits no plugin flags.
        assert!(args_from(&hub, &[]).is_empty());
    }

    /// Reject empty names because the CLI deduplicates plugins by identity.
    #[test]
    fn does_not_save_unnamed_plugins() {
        assert!(save_local(plugin("  ", "/opt/x")).is_err());
    }

    /// Reject directories without a valid plugin before registration instead of silently losing
    /// behavior during startup.
    #[test]
    fn rejects_directories_without_manifests() {
        let dir = std::env::temp_dir().join(format!("prometeu-plug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_source(&dir.display().to_string()).is_err());

        std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            dir.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"example","description":"what it does"}"#,
        )
        .unwrap();
        assert!(check_source(&dir.display().to_string()).is_ok());

        // Use manifest metadata to prefill the form.
        let looked = plugin_look(dir.display().to_string()).unwrap();
        assert_eq!(looked.id, "example");
        assert_eq!(looked.note, "what it does");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Reject paths that do not exist.
    #[test]
    fn rejects_missing_paths() {
        assert!(check_source("/does/not/exist/plugin").is_err());
        assert!(check_source("https://example.com/x.zip").is_ok());
    }

    /// Suggest a ZIP filename when its manifest is not directly readable.
    #[test]
    fn zip_uses_the_archive_filename() {
        assert_eq!(guessed_name("https://example.com/caveman.zip"), "caveman");
        assert_eq!(guessed_name("/tmp/meu-plugin.zip"), "meu-plugin");
    }

    /// Normalize browser URLs, owner/repo shorthand, and Git addresses into clone targets.
    #[test]
    fn pasted_urls_become_clone_sources() {
        let git = "https://github.com/JuliusBrussee/caveman";
        assert_eq!(git_url("JuliusBrussee/caveman"), git);
        assert_eq!(git_url("github.com/JuliusBrussee/caveman"), git);
        assert_eq!(git_url("https://github.com/JuliusBrussee/caveman/"), git);
        assert_eq!(
            git_url("https://github.com/JuliusBrussee/caveman/tree/main"),
            git
        );
        // Preserve already valid Git addresses.
        assert_eq!(
            git_url("git@github.com:dietrichgebert/ponytail.git"),
            "git@github.com:dietrichgebert/ponytail.git"
        );
        assert_eq!(
            git_url("https://gitlab.com/time/x.git"),
            "https://gitlab.com/time/x.git"
        );
        // Reject inputs that cannot identify a repository.
        assert!(git_url("  ").is_empty());
        assert!(git_url("caveman").is_empty());
    }

    /// Derive the clone directory from the repository name, with or without .git.
    #[test]
    fn directory_uses_the_repository_name() {
        assert_eq!(
            repo_name("https://github.com/JuliusBrussee/caveman"),
            "caveman"
        );
        assert_eq!(
            repo_name("git@github.com:dietrichgebert/ponytail.git"),
            "ponytail"
        );
    }

    /// Discover root plugins, local marketplace entries, and plugins/ children. Ignore entries
    /// pointing to other repositories, which require separate installation.
    #[test]
    fn clone_lists_discovered_plugins() {
        let root = std::env::temp_dir().join(format!("prometeu-inst-{}", uuid::Uuid::new_v4()));
        let manifest = |at: &Path, name: &str| {
            std::fs::create_dir_all(at.join(".claude-plugin")).unwrap();
            std::fs::write(
                at.join(".claude-plugin").join("plugin.json"),
                format!(r#"{{"name":"{name}","description":"what it does"}}"#),
            )
            .unwrap();
        };

        // The repository itself is a plugin.
        let one = root.join("one");
        manifest(&one, "caveman");
        let found = plugins_in(&one, "https://example/caveman");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "caveman");
        assert_eq!(found[0].note, "what it does");
        assert!(found[0].made);
        assert_eq!(found[0].from, "https://example/caveman");

        // The marketplace contains one local plugin and one external entry.
        let many = root.join("muitos");
        manifest(&many.join("plugins").join("a"), "a");
        manifest(&many.join("plugins").join("b"), "b");
        std::fs::create_dir_all(many.join(".claude-plugin")).unwrap();
        std::fs::write(
            many.join(".claude-plugin").join("marketplace.json"),
            r#"{"plugins":[{"name":"a","source":"./plugins/a"},{"name":"outside","source":{"source":"git-subdir","url":"https://example.com/other.git"}}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(many.join(".agents").join("plugins")).unwrap();
        std::fs::write(
            many.join(".agents").join("plugins").join("marketplace.json"),
            r#"{"name":"nativo","plugins":[{"name":"b","source":{"source":"local","path":"./plugins/b"}}]}"#,
        )
        .unwrap();
        let found = plugins_in(&many, "https://example/muitos");
        assert_eq!(
            found.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );

        // Without a root manifest or marketplace, inspect one directory level.
        let loose = root.join("solto");
        manifest(&loose.join("plugins").join("c"), "c");
        assert_eq!(
            plugins_in(&loose, "")
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["c"]
        );

        // Return no entries when no plugins exist.
        std::fs::create_dir_all(root.join("vazio")).unwrap();
        assert!(plugins_in(&root.join("vazio"), "").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Generate a native marketplace entry and Codex manifest while retaining compatible hooks.
    /// Content-hashed versions invalidate unchanged upstream version numbers.
    #[test]
    fn codex_marketplace_uses_the_same_plugin() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-market-{}", uuid::Uuid::new_v4()));
        let source = root.join("origem");
        let market = root.join("mercado");
        std::fs::create_dir_all(source.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(source.join("skills").join("curta")).unwrap();
        std::fs::create_dir_all(source.join("commands")).unwrap();
        std::fs::write(
            source.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"curta","description":"responde curto","hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"sh ${CLAUDE_PLUGIN_ROOT}/hooks/curta.sh"}]}]}}"#,
        )
        .unwrap();
        std::fs::write(
            source.join("skills/curta/SKILL.md"),
            "---\nname: curta\n---\n",
        )
        .unwrap();
        std::fs::write(
            source.join(".mcp.json"),
            r#"{"mcpServers":{"curta":{"command":"node","args":["server.js"]}}}"#,
        )
        .unwrap();

        let prepared = prepare_marketplace(
            &market,
            "prometeu-test",
            &[plugin("curta", &source.display().to_string())],
        )
        .unwrap();
        assert_eq!(prepared[0].id, "curta");
        assert_eq!(prepared[0].canonical, "curta@prometeu-test");
        assert!(prepared[0].version.starts_with("0.0.0+prometeu."));
        assert!(prepared[0].hooks);

        let native = read_json(
            &market
                .join("plugins/curta")
                .join(".codex-plugin/plugin.json"),
        )
        .unwrap();
        assert_eq!(native["name"], "curta");
        assert_eq!(native["skills"], "./skills/");
        assert_eq!(native["commands"], "./commands/");
        assert_eq!(native["mcpServers"], "./.mcp.json");
        assert!(native["hooks"]["hooks"]["UserPromptSubmit"].is_array());
        assert_eq!(native["version"], prepared[0].version);

        let catalogue = read_json(&market.join(".agents/plugins/marketplace.json")).unwrap();
        assert_eq!(catalogue["name"], "prometeu-test");
        assert_eq!(catalogue["plugins"][0]["source"]["source"], "local");
        assert_eq!(catalogue["plugins"][0]["source"]["path"], "./plugins/curta");

        // An adapter revision must regenerate older snapshots whose source hash and inline hook
        // envelope used the previous format.
        let staged = market.join("plugins/curta");
        std::fs::write(
            staged.join(".codex-plugin/plugin.json"),
            r#"{"name":"curta","hooks":{"UserPromptSubmit":[]}}"#,
        )
        .unwrap();
        std::fs::write(
            market.join("plugins/.curta.source-hash"),
            fingerprint(&source).unwrap(),
        )
        .unwrap();
        prepare_marketplace(
            &market,
            "prometeu-test",
            &[plugin("curta", &source.display().to_string())],
        )
        .unwrap();
        let migrated = read_json(&staged.join(".codex-plugin/plugin.json")).unwrap();
        assert!(migrated["hooks"]["hooks"]["UserPromptSubmit"].is_array());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detects_hooks_that_must_start_enabled() {
        let root = std::env::temp_dir().join(format!(
            "prometeu-codex-hook-detect-{}",
            uuid::Uuid::new_v4()
        ));
        let inline = root.join("inline");
        let conventional = root.join("conventional");
        let declared_but_broken = root.join("declared-broken");
        let plain = root.join("plain");

        for plugin in [&inline, &conventional, &declared_but_broken, &plain] {
            std::fs::create_dir_all(plugin.join(".codex-plugin")).unwrap();
        }
        std::fs::write(
            inline.join(".codex-plugin/plugin.json"),
            r#"{"name":"inline","hooks":{"SessionStart":[{"hooks":[]}]}}"#,
        )
        .unwrap();
        std::fs::write(
            conventional.join(".codex-plugin/plugin.json"),
            r#"{"name":"conventional"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(conventional.join("hooks")).unwrap();
        std::fs::write(conventional.join("hooks/hooks.json"), r#"{"hooks":{}}"#).unwrap();
        std::fs::write(
            declared_but_broken.join(".codex-plugin/plugin.json"),
            r#"{"name":"declared-broken","hooks":"./missing.json"}"#,
        )
        .unwrap();
        std::fs::write(
            plain.join(".codex-plugin/plugin.json"),
            r#"{"name":"plain"}"#,
        )
        .unwrap();

        assert!(plugin_has_hooks(&inline));
        assert!(plugin_has_hooks(&conventional));
        assert!(plugin_has_hooks(&declared_but_broken));
        assert!(!plugin_has_hooks(&plain));
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn freeform_claude_versions_do_not_break_the_codex_cache() {
        let root =
            std::env::temp_dir().join(format!("prometeu-plugin-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/plugin.json"),
            r#"{"name":"x","version":"v-next"}"#,
        )
        .unwrap();
        assert_eq!(
            portable_version(&root, "0123456789abcdef0123456789abcdef"),
            "0.0.0+prometeu.0123456789abcdef"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn codex_home_belongs_to_the_workspace_not_the_working_directory() {
        assert_eq!(
            codex_workspace_home("workspace-a"),
            codex_workspace_home("workspace-a")
        );
        assert_ne!(
            codex_workspace_home("workspace-a"),
            codex_workspace_home("workspace-b")
        );
    }

    #[cfg(unix)]
    #[test]
    fn deleting_derived_home_does_not_follow_links_to_the_real_home() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-remove-{}", uuid::Uuid::new_v4()));
        let homes = root.join("homes");
        let home = homes.join("workspace");
        let real = root.join("real");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("auth.json"), "account").unwrap();
        std::os::unix::fs::symlink(real.join("auth.json"), home.join("auth.json")).unwrap();

        remove_codex_home(&homes, &home);

        assert!(!home.exists());
        assert_eq!(
            std::fs::read_to_string(real.join("auth.json")).unwrap(),
            "account"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn derived_home_preserves_state_without_changing_global_configuration() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-home-{}", uuid::Uuid::new_v4()));
        let base = root.join("base");
        let home = root.join("workspace");
        let marketplace = home.join("marketplace");
        std::fs::create_dir_all(base.join("plugins")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(base.join("auth.json"), "account").unwrap();
        let global = r#"
[projects."/tmp/project"]
trust_level = "trusted"

[plugins."global@other"]
enabled = true

[hooks.state.global]
trusted_hash = "sha256:global"
"#;
        std::fs::write(base.join("config.toml"), global).unwrap();
        let selected = format!("novo@{}", codex_marketplace_name());
        let old = format!("antigo@{}", codex_marketplace_name());
        std::fs::write(
            home.join("config.toml"),
            format!(
                r#"
[hooks.state.workspace]
trusted_hash = "sha256:workspace"

[plugins."{old}"]
enabled = true
opcao = "preservada"
"#
            ),
        )
        .unwrap();

        prepare_codex_home(&base, &home, &marketplace, std::slice::from_ref(&selected)).unwrap();

        assert_eq!(
            std::fs::read_to_string(base.join("config.toml")).unwrap(),
            global
        );
        let config = read_toml(&home.join("config.toml")).unwrap();
        assert_eq!(
            config["projects"]["/tmp/project"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(
            config["plugins"]["global@other"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("file"));
        assert_eq!(
            config["plugins"][&selected]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(config["plugins"][&old]["enabled"].as_bool(), Some(false));
        assert_eq!(
            config["plugins"][&old]["opcao"].as_str(),
            Some("preservada")
        );
        assert_eq!(
            config["hooks"]["state"]["global"]["trusted_hash"].as_str(),
            Some("sha256:global")
        );
        assert_eq!(
            config["hooks"]["state"]["workspace"]["trusted_hash"].as_str(),
            Some("sha256:workspace")
        );
        assert_eq!(
            config["marketplaces"][codex_marketplace_name()]["source"].as_str(),
            Some(marketplace.to_string_lossy().as_ref())
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::read_link(home.join("auth.json")).unwrap(),
            base.join("auth.json")
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// Opt-in CLI integration verifies the marketplace, installed copy, visible skill, and active
    /// SessionStart hook. It temporarily writes to and cleans up the real Codex cache.
    #[test]
    #[ignore]
    fn codex_installs_a_real_portable_plugin() {
        let root = std::env::temp_dir().join(format!(
            "prometeu-codex-plugin-live-{}",
            uuid::Uuid::new_v4()
        ));
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let id = format!("prometeu-smoke-{}", &suffix[..8]);
        let marker = format!("runtime-marker-{}", &suffix[8..16]);
        let hook_marker = format!("session-hook-{}", &suffix[24..32]);
        let updated_marker = format!("runtime-updated-{}", &suffix[16..24]);
        let hook_ran = root.join("session-start-ran");
        let source = root.join("source");
        std::fs::create_dir_all(source.join(".claude-plugin")).unwrap();
        std::fs::create_dir_all(source.join("skills").join(&id)).unwrap();
        std::fs::create_dir_all(source.join("hooks")).unwrap();
        std::fs::write(
            source.join(".claude-plugin/plugin.json"),
            format!(
                r#"{{"name":"{id}","version":"0.1.0","hooks":{{"SessionStart":[{{"hooks":[{{"type":"command","command":"sh ${{CLAUDE_PLUGIN_ROOT}}/hooks/activate.sh"}}]}}]}}}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            source.join("hooks/activate.sh"),
            format!(
                "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{hook_marker}'\nprintf '%s\\n' '{hook_marker}' > '{}'\n",
                hook_ran.display()
            ),
        )
        .unwrap();
        std::fs::write(
            source.join("skills").join(&id).join("SKILL.md"),
            format!("---\nname: {id}\ndescription: {marker}\n---\n"),
        )
        .unwrap();

        let previous = std::env::var_os("PROMETEU_ROOT");
        let global_config = crate::codex::user_home().join("config.toml");
        let global_before = std::fs::read(&global_config).ok();
        std::env::set_var("PROMETEU_ROOT", &root);
        let canonical = format!("{id}@{}", codex_marketplace_name());
        let workspace = format!("workspace-{suffix}");
        let home = codex_workspace_home(&workspace);
        let result = (|| -> Result<(), String> {
            save_local(plugin(&id, &source.display().to_string()))?;
            let chosen = vec![id.clone()];
            let selected = codex_for(
                &workspace,
                Some(&chosen),
                &crate::accounts::active(crate::state::ProviderId::Codex)?,
            )?;
            if selected.ids != [canonical.clone()] {
                return Err(format!("ids inesperados: {:?}", selected.ids));
            }
            if selected.hook_ids != [canonical.clone()] {
                return Err(format!("hooks inesperados: {:?}", selected.hook_ids));
            }
            if selected.home.as_ref() != Some(&home) {
                return Err(format!("home inesperado: {:?}", selected.home));
            }
            let first_version = codex_installed(&home)?
                .get(&canonical)
                .ok_or_else(|| "plugin did not appear in the Codex cache".to_string())?
                .version
                .clone();
            let listed = codex_command(&home)
                .args(["plugin", "list", "--json"])
                .output()
                .map_err(|error| error.to_string())?;
            if !listed.status.success() {
                return Err(last_line(&String::from_utf8_lossy(&listed.stderr)));
            }
            let listed: Value = serde_json::from_slice(&listed.stdout)
                .map_err(|error| format!("unreadable list: {error}"))?;
            let active = listed["installed"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| entry["pluginId"].as_str() == Some(&canonical));
            if active.and_then(|entry| entry["enabled"].as_bool()) != Some(true) {
                return Err(format!("plugin was not active in the home: {active:?}"));
            }
            let output = Command::new("codex")
                .env("CODEX_HOME", &home)
                .args(["--enable", "plugins", "--enable", "hooks"])
                .args(["debug", "prompt-input", "smoke"])
                .output()
                .map_err(|error| error.to_string())?;
            if !output.status.success() {
                return Err(last_line(&String::from_utf8_lossy(&output.stderr)));
            }
            if !String::from_utf8_lossy(&output.stdout).contains(&marker) {
                return Err("the Codex session did not receive the installed skill".into());
            }

            // debug prompt-input proves skill discovery without opening a session. Use the real
            // app-server to observe SessionStart before any model turn.
            use std::io::Write as _;
            let mut server = Command::new("codex")
                .env("CODEX_HOME", &home)
                .arg("app-server")
                .args(["--enable", "plugins", "--enable", "hooks"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| error.to_string())?;
            let live = (|| -> Result<(), String> {
                let mut input = server
                    .stdin
                    .take()
                    .ok_or_else(|| "app-server missing stdin".to_string())?;
                let output = server
                    .stdout
                    .take()
                    .ok_or_else(|| "app-server missing stdout".to_string())?;
                let (send, receive) = std::sync::mpsc::channel();
                std::thread::spawn(move || {
                    for line in BufReader::new(output).lines().map_while(Result::ok) {
                        if let Ok(message) = serde_json::from_str::<Value>(&line) {
                            if send.send(message).is_err() {
                                break;
                            }
                        }
                    }
                });
                let response = |id: u64| -> Result<Value, String> {
                    loop {
                        let message =
                            receive
                                .recv_timeout(Duration::from_secs(5))
                                .map_err(|error| {
                                    format!("app-server missing response {id}: {error}")
                                })?;
                        if message["id"].as_u64() == Some(id) {
                            if let Some(error) = message["error"]["message"].as_str() {
                                return Err(format!("app-server rejected {id}: {error}"));
                            }
                            return Ok(message);
                        }
                    }
                };

                writeln!(
                    input,
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "clientInfo": { "name": "prometeu-smoke", "title": "Prometeu smoke", "version": "0" },
                            "capabilities": { "experimentalApi": true },
                        },
                    })
                )
                .map_err(|error| error.to_string())?;
                input.flush().map_err(|error| error.to_string())?;
                response(1)?;
                writeln!(
                    input,
                    "{}",
                    serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })
                )
                .map_err(|error| error.to_string())?;
                writeln!(
                    input,
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "hooks/list",
                        "params": { "cwds": [root.display().to_string()] },
                    })
                )
                .map_err(|error| error.to_string())?;
                input.flush().map_err(|error| error.to_string())?;
                let listed = response(2)?;
                let hook = listed["result"]["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
                    .find(|hook| hook["pluginId"].as_str() == Some(&canonical))
                    .ok_or_else(|| format!("hook not discovered: {}", listed["result"]))?;
                let key = hook["key"]
                    .as_str()
                    .ok_or_else(|| format!("hook missing key: {hook}"))?;
                let hash = hook["currentHash"]
                    .as_str()
                    .ok_or_else(|| format!("hook missing hash: {hook}"))?;
                let state = serde_json::Map::from_iter([(
                    key.to_string(),
                    serde_json::json!({ "trusted_hash": hash, "enabled": true }),
                )]);
                writeln!(
                    input,
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 3,
                        "method": "config/batchWrite",
                        "params": {
                            "edits": [{
                                "keyPath": "hooks.state",
                                "value": state,
                                "mergeStrategy": "upsert",
                            }],
                            "reloadUserConfig": true,
                        },
                    })
                )
                .map_err(|error| error.to_string())?;
                input.flush().map_err(|error| error.to_string())?;
                response(3)?;
                writeln!(
                    input,
                    "{}",
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 4,
                        "method": "thread/start",
                        "params": {
                            "cwd": root.display().to_string(),
                            "approvalPolicy": "never",
                            "sandbox": "danger-full-access",
                        },
                    })
                )
                .map_err(|error| error.to_string())?;
                input.flush().map_err(|error| error.to_string())?;
                let opened = response(4)?;
                if !hook_ran.is_file() {
                    let thread = opened["result"]["thread"]["id"]
                        .as_str()
                        .ok_or_else(|| "thread/start missing id".to_string())?;
                    writeln!(
                        input,
                        "{}",
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 5,
                            "method": "turn/start",
                            "params": {
                                "threadId": thread,
                                "input": [{ "type": "text", "text": "smoke", "text_elements": [] }],
                                "summary": "auto",
                            },
                        })
                    )
                    .map_err(|error| error.to_string())?;
                    input.flush().map_err(|error| error.to_string())?;
                }
                for _ in 0..100 {
                    if hook_ran.is_file() {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err("the plugin SessionStart hook did not start active".into())
            })();
            server.kill().ok();
            server.wait().ok();
            live?;
            if std::fs::read_to_string(&hook_ran).ok().as_deref()
                != Some(format!("{hook_marker}\n").as_str())
            {
                return Err("the plugin SessionStart hook did not start active".into());
            }

            std::fs::write(
                source.join("skills").join(&id).join("SKILL.md"),
                format!("---\nname: {id}\ndescription: {updated_marker}\n---\n"),
            )
            .map_err(|error| error.to_string())?;
            codex_for(
                &workspace,
                Some(&chosen),
                &crate::accounts::active(crate::state::ProviderId::Codex)?,
            )?;
            let updated_version = codex_installed(&home)?
                .get(&canonical)
                .ok_or_else(|| "updated plugin disappeared from the Codex cache".to_string())?
                .version
                .clone();
            if updated_version == first_version {
                return Err("the new hash did not invalidate the installed version".into());
            }
            let updated = Command::new("codex")
                .env("CODEX_HOME", &home)
                .args(["--enable", "plugins", "--enable", "hooks"])
                .args(["debug", "prompt-input", "smoke atualizado"])
                .output()
                .map_err(|error| error.to_string())?;
            if !updated.status.success() {
                return Err(last_line(&String::from_utf8_lossy(&updated.stderr)));
            }
            if !String::from_utf8_lossy(&updated.stdout).contains(&updated_marker) {
                return Err("the Codex session did not receive the updated version".into());
            }
            if std::fs::read(&global_config).ok() != global_before {
                return Err("a config global do Codex foi alterada".into());
            }
            Ok(())
        })();
        codex_remove(&home, &canonical);
        match previous {
            Some(value) => std::env::set_var("PROMETEU_ROOT", value),
            None => std::env::remove_var("PROMETEU_ROOT"),
        }
        std::fs::remove_dir_all(root).ok();
        result.unwrap();
    }

    /// Ignored GitHub integration clones, discovers, and registers a real plugin: cargo test --
    /// --ignored installs_a_real_plugin.
    #[test]
    #[ignore]
    fn installs_a_real_plugin() {
        let root = std::env::temp_dir().join(format!("prometeu-net-{}", uuid::Uuid::new_v4()));
        // The ignored test owns its process environment.
        std::env::set_var("PROMETEU_ROOT", &root);

        let found = install("JuliusBrussee/caveman".into()).unwrap();
        assert!(found.saved);
        assert_eq!(found.plugins.len(), 1);
        assert_eq!(found.plugins[0].id, "caveman");
        assert!(found.plugins[0].note.len() > 10);
        assert_eq!(
            found.plugins[0].from,
            "https://github.com/JuliusBrussee/caveman"
        );
        assert!(manifest_path(Path::new(&found.plugins[0].source)).exists());

        // Verify the hub entry and the session arguments it produces.
        assert_eq!(
            args_from(&load(), &["caveman".to_string()]),
            ["--plugin-dir", &found.plugins[0].source]
        );

        // Reinstallation reports that an update is needed instead of overwriting the existing
        // clone.
        assert!(install("https://github.com/JuliusBrussee/caveman".into()).is_err());
        plugin_update("caveman".into()).unwrap();

        // Removal also deletes the application-owned directory.
        remove_hub("caveman").unwrap();
        assert!(!store().join("caveman").exists());
        std::env::remove_var("PROMETEU_ROOT");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Marketplace paths must remain inside the clone regardless of external manifest contents.
    #[test]
    fn marketplace_paths_cannot_escape_the_clone() {
        let dir = Path::new("/tmp/clone");
        assert_eq!(within(dir, "./plugins/a"), Some(dir.join("plugins/a")));
        assert_eq!(within(dir, "./"), Some(dir.to_path_buf()));
        assert!(within(dir, "../../etc").is_none());
        assert!(within(dir, "/etc").is_none());
    }

    /// Normalize names without accents, spaces, or repeated hyphens for CLI use.
    #[test]
    fn plugin_names_become_directory_names() {
        assert_eq!(slug("Revisão de front"), "revisao-de-front");
        assert_eq!(slug("  Caveman!!  "), "caveman");
        assert_eq!(slug("padrões — do time"), "padroes-do-time");
        assert_eq!(slug("!!!"), "");
    }

    /// Display written files and the first agent sentence as progress, with paths relative to the
    /// plugin. Reads do not count as progress.
    #[test]
    fn stream_becomes_progress() {
        let dir = Path::new("/tmp/plug");
        let wrote = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/tmp/plug/skills/x/SKILL.md"}}]}}"#;
        let wrote = step(dir, wrote).unwrap();
        assert_eq!(
            (wrote.kind.as_str(), wrote.text.as_str()),
            ("file", "skills/x/SKILL.md")
        );

        let read = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/plug/x"}}]}}"#;
        assert!(step(dir, read).is_none());

        let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"\n  I will start with the manifest.\nThen the skills."}]}}"#;
        let said = step(dir, said).unwrap();
        assert_eq!(
            (said.kind.as_str(), said.text.as_str()),
            ("say", "I will start with the manifest.")
        );

        // Ignore unrelated stream frames and malformed JSON without aborting progress parsing.
        assert!(step(dir, r#"{"type":"result","subtype":"success"}"#).is_none());
        assert!(step(dir, "not JSON").is_none());
    }

    /// A response without a manifest is not a created plugin and must not enter the hub.
    #[test]
    fn directories_without_manifests_do_not_become_plugins() {
        let dir = std::env::temp_dir().join(format!("prometeu-made-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(born(&dir, "example").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
