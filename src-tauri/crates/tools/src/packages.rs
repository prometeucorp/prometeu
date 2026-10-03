//! Native package materialization with explicit roots and injected catalog, installer and files.
use crate::CodexPlugins;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PreparedPlugin {
    id: String,
    canonical: String,
    version: String,
    hooks: bool,
}

#[derive(Clone, Debug, Default)]
pub struct InstalledPlugin {
    pub version: String,
}

/// Include the adapter revision in cache versions so corrected materialization reinstalls unchanged
/// upstream packages.
const CODEX_PACKAGE_REVISION: &str = "2";

/// Catalog reads retain the existing empty-on-missing-or-invalid behavior.
pub trait PackageCatalog: Send + Sync {
    fn load(&self) -> Vec<Plugin>;
}
pub struct FilePackageCatalog {
    pub path: PathBuf,
}
impl PackageCatalog for FilePackageCatalog {
    fn load(&self) -> Vec<Plugin> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
}
/// Registry identity and ordering shared by local package registration adapters.
pub fn register(hub: &mut Vec<Plugin>, plugin: Plugin) {
    match hub.iter_mut().find(|entry| entry.id == plugin.id) {
        Some(existing) => *existing = plugin,
        None => hub.push(plugin),
    }
    hub.sort_by_key(|entry| entry.id.to_lowercase());
}

/// Private persistence returns raw causes; the materializer applies the operation's error code.
pub trait PackageFiles: Send + Sync {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String>;
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String>;
}
pub trait PackageInstaller: Send + Sync {
    fn installed(&self, home: &Path) -> Result<HashMap<String, InstalledPlugin>, String>;
    fn install(&self, home: &Path, canonical: &str) -> Result<(), String>;
    fn remove(&self, home: &Path, canonical: &str);
}
pub trait PackageBackend: Send + Sync {
    fn claude(&self, chosen: Option<&[String]>) -> Vec<String>;
    fn codex(
        &self,
        scope: &str,
        chosen: Option<&[String]>,
        profile: &prometeu_profiles::Profile,
    ) -> Result<CodexPlugins, String>;
}

/// Share the gate between instances that access the same installed cache.
/// Roots, namespace and effects are captured by the host, never discovered globally here.
pub struct NativePackages {
    root: PathBuf,
    user_home: PathBuf,
    marketplace: String,
    catalog: Arc<dyn PackageCatalog>,
    files: Arc<dyn PackageFiles>,
    installer: Arc<dyn PackageInstaller>,
    gate: Arc<Mutex<()>>,
}
impl NativePackages {
    pub fn new(
        root: PathBuf,
        user_home: PathBuf,
        marketplace: String,
        catalog: Arc<dyn PackageCatalog>,
        files: Arc<dyn PackageFiles>,
        installer: Arc<dyn PackageInstaller>,
        gate: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            root,
            user_home,
            marketplace,
            catalog,
            files,
            installer,
            gate,
        }
    }
    /// Derive the home from the persisted workspace ID, not cwd or an ephemeral tab ID. Workspaces
    /// sharing a clone can still have different selections.
    pub fn codex_workspace_home(&self, workspace: &str) -> PathBuf {
        let fingerprint = format!("{:x}", Sha256::digest(workspace.as_bytes()));
        self.root.join(&fingerprint[..24])
    }

    /// Remove this disposable configuration with its workspace. Shared installed payloads and other
    /// workspaces' configurations remain available.
    pub fn forget_codex_workspace(&self, workspace: &str) {
        let root = &self.root;
        let home = self.codex_workspace_home(workspace);
        remove_codex_home(root, &home);
    }

    /// Serialize marketplace materialization, configuration, and shared-cache installation so
    /// simultaneous tabs cannot observe a partial plugin version.
    fn codex_for(
        &self,
        workspace: &str,
        chosen: Option<&[String]>,
        profile: &prometeu_profiles::Profile,
    ) -> Result<CodexPlugins, String> {
        let Some(chosen) = chosen else {
            return Ok(CodexPlugins {
                home: None,
                ids: vec![],
                hook_ids: vec![],
            });
        };
        let hub = self.catalog.load();
        let mut seen = HashSet::new();
        let selected: Vec<Plugin> = chosen
            .iter()
            .filter_map(|id| hub.iter().find(|plugin| &plugin.id == id).cloned())
            .filter(|plugin| seen.insert(plugin.id.clone()))
            .collect();

        let _guard = prometeu_core::lock::lock(&self.gate);
        // Keep separate account homes so new account selections cannot redirect authentication links
        // used by running processes.
        let home = if profile.managed {
            self.codex_workspace_home(workspace).join(&profile.id)
        } else {
            self.codex_workspace_home(workspace)
        };
        let marketplace = codex_marketplace_root(&home);
        let prepared = self.prepare_marketplace(&marketplace, &self.marketplace, &selected)?;
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
        self.prepare_codex_home(base, &home, &marketplace, &ids)?;

        if prepared.is_empty() {
            return Ok(CodexPlugins {
                home: Some(home),
                ids,
                hook_ids,
            });
        }

        let installed = self.installer.installed(&home)?;
        for plugin in &prepared {
            let current = installed.get(&plugin.canonical);
            let needs_install = current.is_none_or(|found| found.version != plugin.version);
            if needs_install {
                if let Err(error) = self.installer.install(&home, &plugin.canonical) {
                    self.installer.remove(&home, &plugin.canonical);
                    return Err(error);
                }
            }
        }
        // Rebuild derived configuration after codex plugin add enables entries, restoring the exact
        // selection and preserving previously trusted workspace hook hashes.
        self.write_codex_config(base, &home, &marketplace, &ids)?;
        Ok(CodexPlugins {
            home: Some(home),
            ids,
            hook_ids,
        })
    }

    /// Share native login, rollouts, skills, and databases through links. Only configuration and
    /// marketplace contents are disposable application-owned data.
    fn prepare_codex_home(
        &self,
        base: &Path,
        home: &Path,
        marketplace: &Path,
        selected: &[String],
    ) -> Result<(), String> {
        if base == home {
            return Err(prometeu_core::error::with_args(
                "err.plugin.codex.config",
                &[(
                    "cause",
                    "derived CODEX_HOME collides with the user home".into(),
                )],
            ));
        }
        std::fs::create_dir_all(base)
            .and_then(|()| std::fs::create_dir_all(base.join("plugins")))
            .map_err(|error| {
                prometeu_core::error::with_args(
                    "err.plugin.codex.config",
                    &[("cause", error.to_string())],
                )
            })?;
        self.files.ensure_private_dir(home).map_err(|cause| {
            prometeu_core::error::with_args("err.plugin.codex.config", &[("cause", cause)])
        })?;
        mirror_codex_home(base, home)?;
        self.write_codex_config(base, home, marketplace, selected)
    }

    /// Rebuild derived configuration from the real home plus retained workspace hook and plugin
    /// settings. Disable all reserved marketplace entries before enabling only the current selection.
    fn write_codex_config(
        &self,
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
            return Err(prometeu_core::error::with_args(
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
        marketplaces.insert(self.marketplace.clone(), toml::Value::Table(source));

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

        let body = toml::to_string_pretty(&config).map_err(|error| {
            prometeu_core::error::with_args(
                "err.plugin.codex.config",
                &[("cause", error.to_string())],
            )
        })?;
        self.files
            .write_private(&home.join("config.toml"), &body)
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.codex.config", &[("cause", cause)])
            })
    }

    /// Copy packages without modifying their Claude sources. Add a content hash to the derived version
    /// so upstream changes invalidate cache even without a version bump.
    fn prepare_marketplace(
        &self,
        root: &Path,
        marketplace: &str,
        plugins: &[Plugin],
    ) -> Result<Vec<PreparedPlugin>, String> {
        let plugin_root = root.join("plugins");
        self.files
            .ensure_private_dir(&plugin_root)
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.codex.prepare", &[("cause", cause)])
            })?;
        let mut prepared = Vec::new();
        let mut entries = Vec::new();
        for plugin in plugins {
            if plugin.id.len() > 64 || slug(&plugin.id) != plugin.id {
                return Err(prometeu_core::error::with_args(
                    "err.plugin.codex.name",
                    &[("name", plugin.id.clone())],
                ));
            }
            if remote(&plugin.source) {
                return Err(prometeu_core::error::with_args(
                    "err.plugin.codex.source",
                    &[("name", plugin.id.clone())],
                ));
            }
            let source = PathBuf::from(expand(&plugin.source, &self.user_home));
            if !source.is_dir() {
                return Err(prometeu_core::error::with_args(
                    "err.plugin.codex.source",
                    &[("name", plugin.id.clone())],
                ));
            }
            let source = source.canonicalize().map_err(|error| {
                prometeu_core::error::with_args(
                    "err.plugin.codex.prepare",
                    &[("cause", error.to_string())],
                )
            })?;
            if root.starts_with(&source) {
                return Err(prometeu_core::error::with_args(
                    "err.plugin.codex.prepare",
                    &[("cause", "plugin source contains the adapter cache".into())],
                ));
            }
            let fingerprint = codex_package_fingerprint(&source)?;
            let version = portable_version(&source, &fingerprint);
            let target = plugin_root.join(&plugin.id);
            self.stage_plugin(&source, &target, &plugin.id, &version, &fingerprint)?;
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
        self.files
            .write_private(&marketplace_path, &body)
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.codex.prepare", &[("cause", cause)])
            })?;
        Ok(prepared)
    }

    fn stage_plugin(
        &self,
        source: &Path,
        target: &Path,
        id: &str,
        version: &str,
        fingerprint: &str,
    ) -> Result<(), String> {
        let marker = target.with_file_name(format!(".{id}.source-hash"));
        if target.is_dir() && std::fs::read_to_string(&marker).ok().as_deref() == Some(fingerprint)
        {
            return Ok(());
        }
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let temporary = parent.join(format!(".{id}.{}.tmp", uuid::Uuid::new_v4()));
        if let Err(error) = copy_tree(source, &temporary) {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(prometeu_core::error::with_args(
                "err.plugin.codex.prepare",
                &[("cause", error.to_string())],
            ));
        }
        if let Err(error) = self.write_portable_manifest(&temporary, id, version) {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(error);
        }
        if target.exists() {
            std::fs::remove_dir_all(target).map_err(|error| {
                prometeu_core::error::with_args(
                    "err.plugin.codex.prepare",
                    &[("cause", error.to_string())],
                )
            })?;
        }
        std::fs::rename(&temporary, target).map_err(|error| {
            let _ = std::fs::remove_dir_all(&temporary);
            prometeu_core::error::with_args(
                "err.plugin.codex.prepare",
                &[("cause", error.to_string())],
            )
        })?;
        self.files
            .write_private(&marker, fingerprint)
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.codex.prepare", &[("cause", cause)])
            })
    }

    /// Overlay native Codex manifest fields onto the compatible Claude manifest. Generate missing
    /// native fields only in the derived copy.
    fn write_portable_manifest(&self, root: &Path, id: &str, version: &str) -> Result<(), String> {
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
        let body = serde_json::to_string_pretty(&Value::Object(merged))
            .map_err(|error| error.to_string())?;
        self.files
            .write_private(&native_path, &body)
            .map_err(|cause| {
                prometeu_core::error::with_args("err.plugin.codex.prepare", &[("cause", cause)])
            })
    }

    fn forget_codex_config(&self, home: &Path, canonical: &str) {
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
            let _ = self.files.write_private(&home.join("config.toml"), &body);
        }
    }

    fn remove_marketplace_entry(&self, home: &Path, id: &str) {
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
            let _ = self.files.write_private(&catalogue, &body);
        }
    }

    /// Best-effort removal clears the shared installation and references in derived homes. Later
    /// startup rebuilds configuration from the updated hub if cleanup fails.
    pub fn codex_remove_everywhere(&self, canonical: &str) {
        let id = canonical.split_once('@').map_or(canonical, |(id, _)| id);
        for home in codex_homes_at(&self.root) {
            self.installer.remove(&home, canonical);
            self.forget_codex_config(&home, canonical);
            self.remove_marketplace_entry(&home, id);
        }
    }
}
impl PackageBackend for NativePackages {
    fn claude(&self, chosen: Option<&[String]>) -> Vec<String> {
        match chosen {
            Some(chosen) => args_from(&self.catalog.load(), chosen, &self.user_home),
            None => Vec::new(),
        }
    }
    fn codex(
        &self,
        scope: &str,
        chosen: Option<&[String]>,
        profile: &prometeu_profiles::Profile,
    ) -> Result<CodexPlugins, String> {
        self.codex_for(scope, chosen, profile)
    }
}
/// Inject the hub contents so flag translation tests do not require filesystem state.
pub fn args_from(hub: &[Plugin], chosen: &[String], home: &Path) -> Vec<String> {
    chosen
        .iter()
        .filter_map(|name| hub.iter().find(|p| &p.id == name))
        .flat_map(|plugin| flags(plugin, home))
        .collect()
}

/// Use plugin-url for remote sources and plugin-dir for local ones. Expand tilde at launch time
/// using this machine's home without rewriting the stored source.
pub fn flags(plugin: &Plugin, home: &Path) -> [String; 2] {
    let source = plugin.source.trim();
    if remote(source) {
        ["--plugin-url".to_string(), source.to_string()]
    } else {
        ["--plugin-dir".to_string(), expand(source, home)]
    }
}

pub fn remote(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(".claude-plugin").join("plugin.json")
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn expand(source: &str, home: &Path) -> String {
    match source.strip_prefix("~/") {
        Some(rest) => home.join(rest).display().to_string(),
        None => source.to_string(),
    }
}

fn codex_marketplace_root(home: &Path) -> PathBuf {
    home.join("marketplace")
}

fn remove_codex_home(root: &Path, home: &Path) {
    if home.starts_with(root) && home != root {
        std::fs::remove_dir_all(home).ok();
    }
}

/// Mirror existing and future Codex entries except writable configuration files. Preserve real
/// entries already created in the derived home; replace only obsolete links to another home.
fn mirror_codex_home(base: &Path, home: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(base).map_err(|error| {
        prometeu_core::error::with_args("err.plugin.codex.config", &[("cause", error.to_string())])
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            prometeu_core::error::with_args(
                "err.plugin.codex.config",
                &[("cause", error.to_string())],
            )
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
            prometeu_core::error::with_args(
                "err.plugin.codex.config",
                &[("cause", error.to_string())],
            )
        })?;
    }
    Ok(())
}

fn replace_with_shared_entry(source: &Path, target: &Path, home: &Path) -> std::io::Result<()> {
    if !target.starts_with(home) || target == home {
        return Err(std::io::Error::other("invalid derived CODEX_HOME target"));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(target) {
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
    std::os::unix::fs::symlink(source, target)?;
    Ok(())
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    match std::fs::read_to_string(path) {
        Ok(raw) => raw.parse::<toml::Value>().map_err(|error| {
            prometeu_core::error::with_args(
                "err.plugin.codex.config",
                &[("cause", error.to_string())],
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(toml::Value::Table(toml::map::Map::new()))
        }
        Err(error) => Err(prometeu_core::error::with_args(
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
    visit(root, root, &mut hash).map_err(|error| {
        prometeu_core::error::with_args("err.plugin.codex.prepare", &[("cause", error.to_string())])
    })?;
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
            std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
        } else if kind.is_file() {
            std::fs::copy(from, to)?;
        }
    }
    Ok(())
}

fn codex_hooks(hooks: &Value) -> Value {
    match hooks {
        Value::Object(fields) if !fields.contains_key("hooks") => {
            serde_json::json!({ "hooks": hooks })
        }
        _ => hooks.clone(),
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

/// Normalize names for directories and CLI identities: lowercase, no spaces or accents.
/// Transliterate supported accented letters rather than replacing them with separators.
pub fn slug(name: &str) -> String {
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
mod tests;
