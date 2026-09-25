//! The account catalog contains shared definitions. Local hubs also retain private items; only
//! explicit links publish changes.
use crate::lock::lock;
use crate::state::publish;
use crate::{actions, cloud, i18n, mcp, paths, plugins, skills, AppState};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub(crate) mod projects;

static SYNC: Mutex<()> = Mutex::new(());

/// Coordinate synchronization with commands that modify local hubs.
pub(crate) fn guard() -> std::sync::MutexGuard<'static, ()> {
    lock(&SYNC)
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Portable {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub note: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct Doc {
    #[serde(default)]
    pub plugins: Vec<Portable>,
    #[serde(default)]
    pub projects: Vec<Portable>,
    #[serde(default)]
    pub mcp: Vec<mcp::Server>,
    #[serde(default)]
    pub skills: Vec<skills::Skill>,
    #[serde(default)]
    pub actions: Option<actions::Catalog>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Cache {
    revision: Option<u64>,
    doc: Doc,
    // Absence identifies the old cache format, where names served as links.
    #[serde(default)]
    links: Option<BTreeMap<String, String>>,
    #[serde(default)]
    organizations: Vec<OrganizationCatalog>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct OrganizationCatalog {
    id: String,
    name: String,
    revision: Option<u64>,
    doc: Doc,
    // Installation creates a local snapshot, never a personal or organization write link.
    #[serde(default)]
    links: BTreeMap<String, String>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            revision: None,
            doc: Doc::default(),
            links: Some(BTreeMap::new()),
            organizations: Vec::new(),
        }
    }
}

impl Cache {
    fn migrate(&mut self) {
        if self.links.is_none() {
            self.links = Some(
                self.doc
                    .keys()
                    .into_iter()
                    .map(|(kind, id)| (key(kind, &id), id))
                    .collect(),
            );
        }
    }
    fn local_id(&self, kind: &str, id: &str) -> Option<&str> {
        self.links.as_ref()?.get(&key(kind, id)).map(String::as_str)
    }
    fn cloud_id(&self, kind: &str, id: &str) -> Option<String> {
        self.links.as_ref()?.iter().find_map(|(k, local)| {
            (local == id)
                .then(|| k.strip_prefix(&format!("{kind}:")))
                .flatten()
                .map(str::to_string)
        })
    }
}

impl Doc {
    fn keys(&self) -> Vec<(&'static str, String)> {
        self.plugins
            .iter()
            .map(|p| ("plugins", p.id.clone()))
            .chain(self.mcp.iter().map(|s| ("mcp", s.id.clone())))
            .chain(self.skills.iter().map(|s| ("skills", s.id.clone())))
            .chain(self.projects.iter().map(|p| ("projects", p.id.clone())))
            .collect()
    }
}

#[derive(Serialize)]
pub struct CatalogPlugin {
    #[serde(flatten)]
    item: Portable,
    local_id: String,
    installed: bool,
    source_changed: bool,
}
#[derive(Serialize)]
pub struct CatalogSkill {
    #[serde(flatten)]
    item: skills::Skill,
    local_id: String,
    installed: bool,
}
#[derive(Serialize)]
pub struct CatalogState {
    connected: bool,
    revision: Option<u64>,
    plugins: Vec<CatalogPlugin>,
    projects: Vec<projects::CatalogProject>,
    mcp: Vec<String>,
    skills: Vec<CatalogSkill>,
    shared: BTreeMap<String, String>,
    organization_items: Vec<OrganizationItem>,
}

#[derive(Serialize)]
struct OrganizationItem {
    organization: String,
    organization_name: String,
    revision: Option<u64>,
    kind: String,
    id: String,
    description: String,
    installed: bool,
    // The installed local item that satisfies this definition, linked or equivalent.
    local_id: Option<String>,
}

fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}
fn invalid() -> String {
    i18n::t("err.catalog.invalid")
}
fn conflict() -> String {
    i18n::t("err.catalog.conflict")
}
fn cache_path() -> std::path::PathBuf {
    paths::root().join("catalog.json")
}
fn load_cache() -> Cache {
    let mut cache: Cache = std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    cache.migrate();
    cache
}
fn save_cache(cache: &Cache) -> Result<(), String> {
    paths::write_private(&cache_path(), &serde_json::to_string(cache).unwrap())
        .map_err(|_| i18n::t("err.cloud.storage"))
}

/// Disconnecting removes links while preserving hubs and files.
pub fn forget() {
    let _sync = guard();
    let _ = std::fs::remove_file(cache_path());
}

pub fn portable(p: &plugins::Plugin) -> Option<Portable> {
    let source = if !p.from.is_empty() {
        &p.from
    } else if plugins::remote(&p.source) {
        &p.source
    } else {
        return None;
    };
    Some(Portable {
        id: p.id.clone(),
        source: source.clone(),
        note: p.note.clone(),
    })
}

/// GitHub owners and repositories are case-insensitive; other Git hosts may not be.
fn source_key(source: &str) -> String {
    let url = plugins::git_url(source);
    let lower = url.to_lowercase();
    if lower.starts_with("https://github.com/") {
        lower.strip_suffix(".git").unwrap_or(&lower).to_string()
    } else {
        url
    }
}

fn same_plugin(item: &Portable, local: &plugins::Plugin) -> bool {
    portable(local).is_some_and(|local| {
        local.id.eq_ignore_ascii_case(&item.id)
            && source_key(&local.source) == source_key(&item.source)
    })
}

/// The linked local copy, or else an unlinked equivalent that already satisfies the item.
fn installed_plugin<'a>(
    cache: &Cache,
    item: &Portable,
    hub: &'a [plugins::Plugin],
) -> Option<&'a plugins::Plugin> {
    let linked = cache.local_id("plugins", &item.id).unwrap_or(&item.id);
    hub.iter().find(|p| p.id == linked).or_else(|| {
        hub.iter()
            .find(|p| cache.cloud_id("plugins", &p.id).is_none() && same_plugin(item, p))
    })
}

fn installed_skill<'a>(
    cache: &Cache,
    item: &skills::Skill,
    hub: &'a [skills::Skill],
) -> Option<&'a skills::Skill> {
    let linked = cache.local_id("skills", &item.id).unwrap_or(&item.id);
    hub.iter().find(|s| s.id == linked).or_else(|| {
        hub.iter()
            .find(|s| cache.cloud_id("skills", &s.id).is_none() && same_skill(item, s))
    })
}

/// Sharing a private item that already exists in the personal catalog links it instead of
/// publishing a duplicate. MCPs are excluded because the catalog always installs its own copy.
fn link_equivalent(
    cache: &mut Cache,
    kind: &str,
    id: &str,
    hub: &[plugins::Plugin],
    skill_hub: &[skills::Skill],
) -> bool {
    let cloud = match kind {
        "plugins" => hub.iter().find(|p| p.id == id).and_then(|local| {
            cache
                .doc
                .plugins
                .iter()
                .find(|item| same_plugin(item, local))
                .map(|item| item.id.clone())
        }),
        "skills" => skill_hub.iter().find(|s| s.id == id).and_then(|local| {
            cache
                .doc
                .skills
                .iter()
                .find(|item| same_skill(item, local))
                .map(|item| item.id.clone())
        }),
        _ => None,
    };
    let Some(cloud) = cloud else {
        return false;
    };
    let installed = |local: &str| match kind {
        "plugins" => hub.iter().any(|p| p.id == local),
        _ => skill_hub.iter().any(|s| s.id == local),
    };
    // Never steal the link from a copy that is still installed.
    if cache
        .local_id(kind, &cloud)
        .is_some_and(|local| local != id && installed(local))
    {
        return false;
    }
    cache
        .links
        .as_mut()
        .unwrap()
        .insert(key(kind, &cloud), id.into());
    true
}

fn same_mcp(item: &mcp::Server, local: &mcp::Server) -> bool {
    item.id == local.id && item.config == blank(local).config
}

fn same_skill(item: &skills::Skill, local: &skills::Skill) -> bool {
    item == local
}

pub fn blank(server: &mcp::Server) -> mcp::Server {
    let mut config = server.config.clone();
    for key in ["env", "headers"] {
        if let Some(map) = config.get_mut(key).and_then(Value::as_object_mut) {
            for value in map.values_mut() {
                *value = json!("");
            }
        }
    }
    mcp::Server {
        id: server.id.clone(),
        config,
        note: server.note.clone(),
    }
}

fn merge(cloud: &Value, local: &Value) -> Value {
    let mut out = cloud.clone();
    for key in ["env", "headers"] {
        let Some(mine) = local.get(key).and_then(Value::as_object) else {
            continue;
        };
        if let Some(target) = out.get_mut(key).and_then(Value::as_object_mut) {
            for (name, value) in target.iter_mut() {
                if value.as_str() == Some("") {
                    if let Some(kept) = mine.get(name) {
                        *value = kept.clone();
                    }
                }
            }
        }
    }
    out
}

/// IDs from different sources must never overwrite a private item with the same name.
fn available(id: &str, used: &BTreeSet<String>) -> String {
    if !used.contains(id) {
        return id.into();
    }
    let stem: String = id
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
        .take(40)
        .collect();
    let stem = if stem.is_empty() { "item" } else { &stem };
    for n in 1.. {
        let candidate = format!("cloud-{stem}-{n}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn bind(cache: &mut Cache, new: &Doc, hubs: [BTreeSet<String>; 3]) {
    let keys: BTreeSet<String> = new.keys().iter().map(|(kind, id)| key(kind, id)).collect();
    let links = cache.links.as_mut().unwrap();
    links.retain(|k, _| keys.contains(k));
    for (kind, mut used) in ["plugins", "mcp", "skills"].into_iter().zip(hubs) {
        used.extend(
            links
                .iter()
                .filter(|(k, _)| k.starts_with(&format!("{kind}:")))
                .map(|(_, id)| id.clone()),
        );
        for (_, id) in new.keys().into_iter().filter(|(k, _)| *k == kind) {
            links.entry(key(kind, &id)).or_insert_with(|| {
                let local = available(&id, &used);
                used.insert(local.clone());
                local
            });
        }
    }
}

fn apply(app: &AppHandle, cache: &mut Cache, new: Doc) -> Result<(), String> {
    let mut hub = plugins::load();
    let mut servers = mcp::load();
    let local_skills = skills::load();
    bind(
        cache,
        &new,
        [
            hub.iter().map(|p| p.id.clone()).collect(),
            servers.iter().map(|s| s.id.clone()).collect(),
            local_skills.iter().map(|s| s.id.clone()).collect(),
        ],
    );
    for item in &new.plugins {
        let id = cache.local_id("plugins", &item.id).unwrap();
        if let Some(local) = hub.iter_mut().find(|p| p.id == id) {
            local.note = item.note.clone();
        }
    }
    plugins::write_hub(&hub)?;
    servers = merge_servers(servers, cache, &new);
    mcp::store(&servers)?;
    for item in &new.skills {
        let id = cache.local_id("skills", &item.id).unwrap();
        if local_skills.iter().any(|s| s.id == id) {
            skills::save_local(skills::Skill {
                id: id.into(),
                ..item.clone()
            })?;
        }
    }
    // Preserve existing actions; logging in never exports the local catalog.
    if let Some(actions) = &new.actions {
        actions::validate(actions)?;
        lock(&app.state::<AppState>().board).actions = actions.clone();
        publish(app);
    }
    cache.doc = new;
    Ok(())
}

fn merge_servers(mut servers: Vec<mcp::Server>, cache: &Cache, new: &Doc) -> Vec<mcp::Server> {
    for item in &new.mcp {
        let id = cache.local_id("mcp", &item.id).unwrap().to_string();
        let mut server = item.clone();
        server.id = id.clone();
        if let Some(local) = servers.iter_mut().find(|s| s.id == id) {
            server.config = merge(&item.config, &local.config);
            *local = server;
        } else {
            servers.push(server);
        }
    }
    servers.sort_by_key(|s| s.id.to_lowercase());
    servers
}

fn validate(doc: &Doc) -> Result<(), String> {
    let keys = doc.keys();
    if keys
        .iter()
        .any(|(_, id)| id.trim().is_empty() || id.len() > 256 || id.contains('\0'))
        || keys.iter().collect::<BTreeSet<_>>().len() != keys.len()
    {
        return Err(invalid());
    }
    for item in &doc.projects {
        projects::validate(item)?;
    }
    for item in &doc.skills {
        skills::validate(item)?;
    }
    for server in &doc.mcp {
        if !server.config.is_object()
            || (!server.config["url"].is_string() && !server.config["command"].is_string())
        {
            return Err(invalid());
        }
        for key in ["env", "headers"] {
            if let Some(value) = server.config.get(key) {
                if !value
                    .as_object()
                    .is_some_and(|map| map.values().all(|v| v.as_str() == Some("")))
                {
                    return Err(invalid());
                }
            }
        }
    }
    for item in &doc.plugins {
        if item.source.trim().is_empty()
            || item.source.starts_with(['/', '~', '.'])
            || item.source.contains('\0')
        {
            return Err(invalid());
        }
    }
    if let Some(actions) = &doc.actions {
        actions::validate(actions)?;
    }
    Ok(())
}

fn parse(value: &Value) -> Result<(Option<u64>, Doc), String> {
    let revision = value["revision"].as_u64();
    let doc = if value["catalog"].is_null() {
        Doc::default()
    } else {
        serde_json::from_value(value["catalog"].clone())
            .map_err(|_| i18n::t("err.cloud.response"))?
    };
    if value["catalog"].is_null() != revision.is_none() {
        return Err(invalid());
    }
    validate(&doc)?;
    Ok((revision, doc))
}

fn pull_locked(app: &AppHandle) -> Result<(), String> {
    let Some((code, value)) =
        cloud::api(Method::GET, "/api/catalog", None, Duration::from_secs(12))?
    else {
        return Ok(());
    };
    if code != 200 {
        return Err(i18n::t("err.cloud.network"));
    }
    let (revision, doc) = parse(&value)?;
    let mut cache = load_cache();
    let organizations = pull_organizations(&cache.organizations)?;
    if cache.revision == revision && cache.doc == doc && cache.organizations == organizations {
        return Ok(());
    }
    if cache.revision != revision || cache.doc != doc {
        apply(app, &mut cache, doc)?;
    }
    cache.organizations = organizations;
    cache.revision = revision;
    save_cache(&cache)?;
    let _ = app.emit("catalog", ());
    Ok(())
}

fn pull_organizations(
    previous: &[OrganizationCatalog],
) -> Result<Vec<OrganizationCatalog>, String> {
    let Some((code, value)) = cloud::api(
        Method::GET,
        "/api/organizations",
        None,
        Duration::from_secs(12),
    )?
    else {
        return Err(i18n::t("err.catalog.disconnected"));
    };
    if code == 404 {
        return Ok(Vec::new());
    }
    if code != 200 {
        return Err(i18n::t("err.cloud.network"));
    }
    let entries = value["organizations"].as_array().ok_or_else(invalid)?;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in entries {
        let id = entry["id"].as_str().ok_or_else(invalid)?;
        let name = entry["name"].as_str().ok_or_else(invalid)?;
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || !seen.insert(id)
        {
            return Err(invalid());
        }
        let Some((code, value)) = cloud::api(
            Method::GET,
            &format!("/api/organizations/{id}/catalog"),
            None,
            Duration::from_secs(12),
        )?
        else {
            return Err(i18n::t("err.catalog.disconnected"));
        };
        // An older Cloud has no endpoint; a revoked membership also returns 404.
        if code == 404 {
            continue;
        }
        if code != 200 {
            return Err(i18n::t("err.cloud.network"));
        }
        let (revision, doc) = parse(&value)?;
        let links = previous
            .iter()
            .find(|org| org.id == id)
            .map(|org| org.links.clone())
            .unwrap_or_default();
        result.push(OrganizationCatalog {
            id: id.into(),
            name: name.into(),
            revision,
            doc,
            links,
        });
    }
    Ok(result)
}

pub fn pull(app: &AppHandle) -> Result<(), String> {
    let _sync = lock(&SYNC);
    pull_locked(app)
}

fn write(app: &AppHandle, mut cache: Cache, doc: Doc) -> Result<(), String> {
    validate(&doc)?;
    let body = json!({ "catalog": doc, "revision": cache.revision });
    let (code, value) = cloud::api(
        Method::PUT,
        "/api/catalog",
        Some(body),
        Duration::from_secs(12),
    )?
    .ok_or_else(|| i18n::t("err.catalog.disconnected"))?;
    if code == 409 {
        // A stale revision must not overwrite edits from another desktop or browser.
        pull_locked(app)?;
        return Err(conflict());
    }
    if code != 200 {
        return Err(i18n::t(if code == 422 || code == 400 {
            "err.catalog.invalid"
        } else {
            "err.cloud.network"
        }));
    }
    let (revision, doc) = parse(&value)?;
    apply(app, &mut cache, doc)?;
    cache.revision = revision;
    save_cache(&cache)?;
    let _ = app.emit("catalog", ());
    Ok(())
}

pub fn mutate(app: &AppHandle, change: impl Fn(&mut Doc)) -> Result<(), String> {
    if !cloud::connected() {
        return Ok(());
    }
    let _sync = lock(&SYNC);
    let cache = load_cache();
    let mut doc = cache.doc.clone();
    change(&mut doc);
    if doc == cache.doc {
        return Ok(());
    }
    write(app, cache, doc)
}

pub fn save_plugin(
    app: &AppHandle,
    plugin: &plugins::Plugin,
    revision: Option<u64>,
) -> Result<(), String> {
    if !cloud::connected() {
        return Ok(());
    }
    let cache = load_cache();
    let Some(id) = cache.cloud_id("plugins", &plugin.id) else {
        return Ok(());
    };
    if cache.revision != revision {
        return Err(conflict());
    }
    let mut doc = cache.doc.clone();
    let target = doc
        .plugins
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(invalid)?;
    // The remote source can change without replacing this Mac's installed clone. Editing its local
    // description must not restore the old remote address.
    target.note = plugin.note.clone();
    if doc == cache.doc {
        return Ok(());
    }
    write(app, cache, doc)
}
pub fn save_mcp(
    app: &AppHandle,
    server: &mcp::Server,
    revision: Option<u64>,
) -> Result<(), String> {
    if !cloud::connected() {
        return Ok(());
    }
    let cache = load_cache();
    let Some(id) = cache.cloud_id("mcp", &server.id) else {
        return Ok(());
    };
    if cache.revision != revision {
        return Err(conflict());
    }
    let mut item = blank(server);
    item.id = id.clone();
    let mut doc = cache.doc.clone();
    let target = doc
        .mcp
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(invalid)?;
    *target = item;
    // Credential-only changes remain local and work offline.
    if doc == cache.doc {
        return Ok(());
    }
    write(app, cache, doc)
}
pub fn save_skill(
    app: &AppHandle,
    skill: &skills::Skill,
    revision: Option<u64>,
) -> Result<(), String> {
    if !cloud::connected() {
        return Ok(());
    }
    let cache = load_cache();
    let Some(id) = cache.cloud_id("skills", &skill.id) else {
        return Ok(());
    };
    if cache.revision != revision {
        return Err(conflict());
    }
    let mut doc = cache.doc.clone();
    let target = doc
        .skills
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(invalid)?;
    *target = skills::Skill {
        id,
        ..skill.clone()
    };
    if doc == cache.doc {
        return Ok(());
    }
    write(app, cache, doc)
}
pub fn remove_shared(app: &AppHandle, kind: &str, local_id: &str) -> Result<(), String> {
    if !cloud::connected() {
        return Ok(());
    }
    let cache = load_cache();
    let Some(id) = cache.cloud_id(kind, local_id) else {
        return Ok(());
    };
    let mut doc = cache.doc.clone();
    match kind {
        "plugins" => doc.plugins.retain(|p| p.id != id),
        "mcp" => doc.mcp.retain(|s| s.id != id),
        _ => return Err(invalid()),
    }
    write(app, cache, doc)
}

#[tauri::command(async)]
pub fn catalog_share(app: AppHandle, kind: String, id: String) -> Result<(), String> {
    if !cloud::connected() {
        return Err(i18n::t("err.catalog.disconnected"));
    }
    let _sync = lock(&SYNC);
    let mut cache = load_cache();
    if link_equivalent(&mut cache, &kind, &id, &plugins::load(), &skills::load()) {
        save_cache(&cache)?;
        let _ = app.emit("catalog", ());
        return Ok(());
    }
    if cache.doc.keys().iter().any(|(k, i)| *k == kind && i == &id) {
        return Err(conflict());
    }
    let mut doc = cache.doc.clone();
    match kind.as_str() {
        "plugins" => {
            let plugin = plugins::load()
                .into_iter()
                .find(|p| p.id == id)
                .ok_or_else(invalid)?;
            doc.plugins
                .push(portable(&plugin).ok_or_else(|| i18n::t("err.catalog.portable"))?);
        }
        "mcp" => doc.mcp.push(blank(
            &mcp::load()
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(invalid)?,
        )),
        "skills" => doc.skills.push(
            skills::load()
                .into_iter()
                .find(|s| s.id == id)
                .ok_or_else(invalid)?,
        ),
        _ => return Err(invalid()),
    }
    cache.links.as_mut().unwrap().insert(key(&kind, &id), id);
    write(&app, cache, doc)
}

#[tauri::command(async)]
pub fn catalog_copy(
    app: AppHandle,
    kind: String,
    id: String,
    new_id: String,
) -> Result<(), String> {
    let _sync = lock(&SYNC);
    let new_id = new_id.trim().to_string();
    if new_id.is_empty() || new_id.len() > 128 || new_id.contains('\0') {
        return Err(invalid());
    }
    let cache = load_cache();
    if cache
        .links
        .as_ref()
        .unwrap()
        .iter()
        .any(|(k, v)| k.starts_with(&format!("{kind}:")) && v == &new_id)
    {
        return Err(conflict());
    }
    match kind.as_str() {
        "plugins" => {
            let mut hub = plugins::load();
            if hub.iter().any(|p| p.id == new_id) {
                return Err(conflict());
            }
            let item = hub
                .iter()
                .find(|p| p.id == id)
                .cloned()
                .ok_or_else(invalid)?;
            // ponytail: copies share the clone; copy files when independent editing is required.
            for plugin in hub.iter_mut().filter(|p| p.source == item.source) {
                plugin.made = false;
            }
            hub.push(plugins::Plugin {
                id: new_id,
                made: false,
                ..item
            });
            plugins::write_hub(&hub)?;
        }
        "mcp" => {
            let mut hub = mcp::load();
            if hub.iter().any(|s| s.id == new_id) {
                return Err(conflict());
            }
            let item = hub
                .iter()
                .find(|s| s.id == id)
                .cloned()
                .ok_or_else(invalid)?;
            hub.push(mcp::Server { id: new_id, ..item });
            mcp::store(&hub)?;
        }
        "skills" => {
            let hub = skills::load();
            if hub.iter().any(|s| s.id == new_id) {
                return Err(conflict());
            }
            let item = hub.into_iter().find(|s| s.id == id).ok_or_else(invalid)?;
            skills::save_local(skills::Skill { id: new_id, ..item })?;
        }
        _ => return Err(invalid()),
    }
    let _ = app.emit("catalog", ());
    Ok(())
}

#[tauri::command(async)]
pub fn catalog_install_plugin(app: AppHandle, id: String) -> Result<(), String> {
    let _sync = lock(&SYNC);
    let cache = load_cache();
    let item = cache
        .doc
        .plugins
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(invalid)?;
    let local = cache.local_id("plugins", &id).ok_or_else(invalid)?;
    plugins::install_catalog(&item.source, &item.id, local, &item.note)?;
    let _ = app.emit("catalog", ());
    Ok(())
}
#[tauri::command(async)]
pub fn catalog_install_skill(app: AppHandle, id: String) -> Result<(), String> {
    let _sync = lock(&SYNC);
    let cache = load_cache();
    let item = cache
        .doc
        .skills
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(invalid)?;
    let local = cache.local_id("skills", &id).ok_or_else(invalid)?;
    skills::save_local(skills::Skill {
        id: local.into(),
        ..item.clone()
    })?;
    let _ = app.emit("catalog", ());
    Ok(())
}

#[tauri::command(async)]
pub fn catalog_install_organization_item(
    app: AppHandle,
    organization: String,
    kind: String,
    id: String,
    revision: Option<u64>,
) -> Result<(), String> {
    let _sync = guard();
    let mut cache = load_cache();
    let index = cache
        .organizations
        .iter()
        .position(|org| org.id == organization)
        .ok_or_else(invalid)?;
    // Recheck membership and the displayed definition before installing executable content.
    let Some((code, value)) = cloud::api(
        Method::GET,
        &format!("/api/organizations/{organization}/catalog"),
        None,
        Duration::from_secs(12),
    )?
    else {
        return Err(i18n::t("err.catalog.disconnected"));
    };
    if code == 404 {
        cache.organizations.remove(index);
        save_cache(&cache)?;
        let _ = app.emit("catalog", ());
        return Err(invalid());
    }
    if code != 200 {
        return Err(i18n::t("err.cloud.network"));
    }
    let (current_revision, doc) = parse(&value)?;
    if revision != current_revision || cache.organizations[index].doc != doc {
        cache.organizations[index].doc = doc;
        cache.organizations[index].revision = current_revision;
        save_cache(&cache)?;
        let _ = app.emit("catalog", ());
        return Err(conflict());
    }
    install_organization_item(&mut cache, index, &kind, &id)?;
    save_cache(&cache)?;
    let _ = app.emit("catalog", ());
    Ok(())
}

fn install_organization_item(
    cache: &mut Cache,
    index: usize,
    kind: &str,
    id: &str,
) -> Result<(), String> {
    let item_key = key(kind, id);
    let existing = match kind {
        "plugins" => {
            let item = cache.organizations[index]
                .doc
                .plugins
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(invalid)?;
            plugins::load()
                .into_iter()
                .find(|local| same_plugin(item, local))
                .map(|local| local.id)
        }
        "mcp" => {
            let item = cache.organizations[index]
                .doc
                .mcp
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(invalid)?;
            mcp::load()
                .into_iter()
                .find(|local| same_mcp(item, local))
                .map(|local| local.id)
        }
        "skills" => {
            let item = cache.organizations[index]
                .doc
                .skills
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(invalid)?;
            skills::load()
                .into_iter()
                .find(|local| same_skill(item, local))
                .map(|local| local.id)
        }
        _ => return Err(invalid()),
    };
    if let Some(local) = existing {
        cache.organizations[index].links.insert(item_key, local);
        return Ok(());
    }
    let mut used: BTreeSet<String> = match kind {
        "plugins" => plugins::load().into_iter().map(|p| p.id).collect(),
        "mcp" => mcp::load().into_iter().map(|s| s.id).collect(),
        "skills" => skills::load().into_iter().map(|s| s.id).collect(),
        _ => return Err(invalid()),
    };
    if cache.organizations[index]
        .links
        .get(&item_key)
        .is_some_and(|local| used.contains(local))
    {
        return Ok(());
    }
    // Reserve uninstalled personal and organization names too; no catalog owns another's item.
    for links in cache
        .links
        .iter()
        .chain(cache.organizations.iter().map(|org| &org.links))
    {
        used.extend(
            links
                .iter()
                .filter(|(k, _)| k.starts_with(&format!("{kind}:")))
                .map(|(_, local)| local.clone()),
        );
    }
    let org = &mut cache.organizations[index];
    let local = org
        .links
        .get(&item_key)
        .cloned()
        .unwrap_or_else(|| available(id, &used));
    match kind {
        "plugins" => {
            let item = org
                .doc
                .plugins
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(invalid)?;
            plugins::install_catalog(&item.source, &item.id, &local, &item.note)?;
        }
        "mcp" => {
            let item = org
                .doc
                .mcp
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(invalid)?;
            let mut servers = mcp::load();
            servers.push(mcp::Server {
                id: local.clone(),
                ..item.clone()
            });
            servers.sort_by_key(|s| s.id.to_lowercase());
            mcp::store(&servers)?;
        }
        "skills" => {
            let item = org
                .doc
                .skills
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(invalid)?;
            skills::save_local(skills::Skill {
                id: local.clone(),
                ..item.clone()
            })?;
        }
        _ => unreachable!(),
    }
    org.links.insert(item_key, local);
    Ok(())
}

#[tauri::command(async)]
pub fn catalog_state(app: AppHandle) -> CatalogState {
    let _sync = guard();
    let connected = cloud::connected();
    let cache = if connected {
        load_cache()
    } else {
        Cache::default()
    };
    let hub = plugins::load();
    let skill_hub = skills::load();
    let servers = mcp::load();
    drop(_sync);
    let mut organization_items = Vec::new();
    for org in &cache.organizations {
        for (kind, id) in org
            .doc
            .keys()
            .into_iter()
            .filter(|(kind, _)| *kind != "projects")
        {
            let linked = org.links.get(&key(kind, &id));
            let (description, local_id) = match kind {
                "plugins" => {
                    let item = org.doc.plugins.iter().find(|p| p.id == id).unwrap();
                    (
                        format!("{} · {}", item.source, item.note),
                        hub.iter()
                            .find(|p| Some(&p.id) == linked)
                            .or_else(|| hub.iter().find(|plugin| same_plugin(item, plugin)))
                            .map(|p| p.id.clone()),
                    )
                }
                "mcp" => {
                    let item = org.doc.mcp.iter().find(|s| s.id == id).unwrap();
                    (
                        format!(
                            "{} · {}",
                            item.config["url"]
                                .as_str()
                                .or_else(|| item.config["command"].as_str())
                                .unwrap_or_default(),
                            item.note
                        ),
                        servers
                            .iter()
                            .find(|s| Some(&s.id) == linked)
                            .or_else(|| servers.iter().find(|server| same_mcp(item, server)))
                            .map(|s| s.id.clone()),
                    )
                }
                "skills" => {
                    let item = org.doc.skills.iter().find(|s| s.id == id).unwrap();
                    (
                        item.description.clone(),
                        skill_hub
                            .iter()
                            .find(|s| Some(&s.id) == linked)
                            .or_else(|| skill_hub.iter().find(|skill| same_skill(item, skill)))
                            .map(|s| s.id.clone()),
                    )
                }
                _ => unreachable!(),
            };
            organization_items.push(OrganizationItem {
                organization: org.id.clone(),
                organization_name: org.name.clone(),
                revision: org.revision,
                kind: kind.into(),
                id,
                description,
                installed: local_id.is_some(),
                local_id,
            });
        }
    }
    let plugins = cache
        .doc
        .plugins
        .iter()
        .map(|item| {
            let local = installed_plugin(&cache, item, &hub);
            CatalogPlugin {
                item: item.clone(),
                local_id: local.map_or_else(
                    || {
                        cache
                            .local_id("plugins", &item.id)
                            .unwrap_or(&item.id)
                            .into()
                    },
                    |p| p.id.clone(),
                ),
                installed: local.is_some(),
                source_changed: local.is_some_and(|p| {
                    portable(p).is_none_or(|p| source_key(&p.source) != source_key(&item.source))
                }),
            }
        })
        .collect();
    let skills = cache
        .doc
        .skills
        .iter()
        .map(|item| {
            let local = installed_skill(&cache, item, &skill_hub);
            CatalogSkill {
                item: item.clone(),
                local_id: local.map_or_else(
                    || {
                        cache
                            .local_id("skills", &item.id)
                            .unwrap_or(&item.id)
                            .into()
                    },
                    |s| s.id.clone(),
                ),
                installed: local.is_some(),
            }
        })
        .collect();
    let shared = cache
        .links
        .as_ref()
        .unwrap()
        .iter()
        .filter_map(|(k, local)| {
            let (kind, id) = k.split_once(':')?;
            Some((key(kind, local), id.into()))
        })
        .collect();
    let registered = lock(&app.state::<AppState>().board).projects.clone();
    let projects = projects::state(&cache, &registered);
    CatalogState {
        connected,
        projects,
        revision: cache.revision,
        plugins,
        skills,
        shared,
        organization_items,
        mcp: cache
            .doc
            .mcp
            .iter()
            .filter_map(|s| cache.local_id("mcp", &s.id).map(str::to_string))
            .collect(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_cloud_contract_catalogs_use_the_production_parser() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/cloud-api.json")).unwrap();
        for name in ["empty_catalog", "catalog", "legacy_catalog"] {
            let payload = &fixture[name];
            let (revision, doc) = parse(payload).unwrap();
            assert_eq!(revision, payload["revision"].as_u64(), "{name}");
            let mut expected = if payload["catalog"].is_null() {
                json!({ "plugins": [], "mcp": [], "skills": [], "actions": null })
            } else {
                payload["catalog"].clone()
            };
            if expected.get("projects").is_none() {
                expected["projects"] = json!([]);
            }
            if expected.get("skills").is_none() {
                expected["skills"] = json!([]);
            }
            assert_eq!(serde_json::to_value(doc).unwrap(), expected, "{name}");
        }
        let mut invalid_revision = fixture["catalog"].clone();
        invalid_revision["revision"] = Value::Null;
        assert!(parse(&invalid_revision).is_err());
        let mut private_payload = fixture["catalog"].clone();
        private_payload["catalog"]["mcp"][0]["config"]["headers"]["Authorization"] =
            json!("synthetic-local-secret");
        assert!(parse(&private_payload).is_err());
    }

    #[test]
    fn organization_installation_preserves_private_items_and_personal_catalog() {
        if std::env::var_os("PROMETEU_ORGANIZATION_CATALOG_TEST").is_none() {
            let root =
                std::env::temp_dir().join(format!("prometeu-org-catalog-{}", uuid::Uuid::new_v4()));
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "catalog::tests::organization_installation_preserves_private_items_and_personal_catalog", "--nocapture"])
                .env("PROMETEU_ORGANIZATION_CATALOG_TEST", "1")
                .env("PROMETEU_ROOT", &root)
                .output().unwrap();
            std::fs::remove_dir_all(root).ok();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let mut cache: Cache = serde_json::from_value(
            json!({"revision": 0, "doc": {}, "links": {"mcp:issues":"cloud-issues-1"}}),
        )
        .unwrap();
        assert!(cache.organizations.is_empty());
        let private = mcp::Server {
            id: "issues".into(),
            note: "private".into(),
            config: json!({"url":"https://private.test", "headers":{"Authorization":"private-token"}}),
        };
        let local_mcp = mcp::Server {
            id: "notion".into(),
            note: "local".into(),
            config: json!({"url":"https://same.test/mcp", "headers":{"Authorization":"local-token"}}),
        };
        mcp::store(&[private.clone(), local_mcp.clone()]).unwrap();
        let local_plugin = plugins::Plugin {
            id: "caveman".into(),
            source: "https://github.com/JuliusBrussee/caveman".into(),
            note: "local".into(),
            made: false,
            from: String::new(),
        };
        plugins::save_local(local_plugin.clone()).unwrap();
        let doc = Doc {
            plugins: vec![Portable {
                id: "Caveman".into(),
                source: "https://github.com/JuliusBrussee/caveman/".into(),
                note: "team".into(),
            }],
            mcp: vec![
                mcp::Server {
                    id: "issues".into(),
                    note: "team".into(),
                    config: json!({"url":"https://team.test", "headers":{"Authorization":""}}),
                },
                mcp::Server {
                    id: "notion".into(),
                    note: "team".into(),
                    config: json!({"url":"https://same.test/mcp", "headers":{"Authorization":""}}),
                },
            ],
            skills: vec![
                skills::Skill {
                    id: "review".into(),
                    description: "Review".into(),
                    content: "Read changes".into(),
                },
                skills::Skill {
                    id: "local-skill".into(),
                    description: "Local skill".into(),
                    content: "Keep this content".into(),
                },
            ],
            ..Doc::default()
        };
        for id in ["one", "two"] {
            cache.organizations.push(OrganizationCatalog {
                id: id.into(),
                name: id.into(),
                revision: Some(0),
                doc: doc.clone(),
                links: BTreeMap::new(),
            });
        }
        let personal = cache.doc.clone();
        let personal_links = cache.links.clone();
        install_organization_item(&mut cache, 0, "plugins", "Caveman").unwrap();
        assert_eq!(cache.organizations[0].links["plugins:Caveman"], "caveman");
        assert!(plugins::load() == [local_plugin]);
        let local_skill = skills::Skill {
            id: "local-skill".into(),
            description: "Local skill".into(),
            content: "Keep this content".into(),
        };
        skills::save_local(local_skill.clone()).unwrap();
        install_organization_item(&mut cache, 0, "mcp", "notion").unwrap();
        install_organization_item(&mut cache, 0, "skills", "local-skill").unwrap();
        assert_eq!(cache.organizations[0].links["mcp:notion"], "notion");
        assert_eq!(
            cache.organizations[0].links["skills:local-skill"],
            "local-skill"
        );
        assert!(mcp::load().contains(&local_mcp));
        assert!(skills::load().contains(&local_skill));
        install_organization_item(&mut cache, 0, "mcp", "issues").unwrap();
        install_organization_item(&mut cache, 1, "mcp", "issues").unwrap();
        assert_eq!(cache.organizations[0].links["mcp:issues"], "cloud-issues-2");
        assert_eq!(cache.organizations[1].links["mcp:issues"], "cloud-issues-3");
        let mut servers = mcp::load();
        assert_eq!(servers.len(), 4);
        assert!(servers.iter().any(|s| s == &private));
        let installed = servers
            .iter_mut()
            .find(|s| s.id == "cloud-issues-2")
            .unwrap();
        assert_eq!(installed.config["headers"]["Authorization"], "");
        installed.config["headers"]["Authorization"] = json!("local-token");
        mcp::store(&servers).unwrap();
        install_organization_item(&mut cache, 0, "mcp", "issues").unwrap();
        assert!(mcp::load() == servers);
        install_organization_item(&mut cache, 0, "skills", "review").unwrap();
        assert!(skills::load()
            .iter()
            .any(|skill| skill.content == "Read changes"));
        assert!(cache.doc == personal);
        assert_eq!(cache.links, personal_links);
        assert_eq!(cache.revision, Some(0));
        save_cache(&cache).unwrap();
        assert!(load_cache().organizations == cache.organizations);
        assert!(install_organization_item(&mut cache, 0, "mcp", "missing").is_err());
        assert!(mcp::load() == servers);
        // Losing membership forgets availability, while installed snapshots and credentials remain.
        cache.organizations.clear();
        save_cache(&cache).unwrap();
        assert!(mcp::load() == servers);
    }

    #[test]
    fn explicit_links_preserve_private_items_and_legacy_catalogs() {
        let local = mcp::Server {
            id: "linear".into(),
            note: "private".into(),
            config: json!({"url":"https://local.test/mcp", "headers":{"Authorization":"private"}}),
        };
        let cloud = mcp::Server {
            id: "linear".into(),
            note: "compartilhado".into(),
            config: json!({"url":"https://cloud.test/mcp", "headers":{"Authorization":""}}),
        };
        let doc = Doc {
            mcp: vec![cloud.clone()],
            ..Doc::default()
        };
        let mut fresh = Cache::default();
        bind(
            &mut fresh,
            &doc,
            [
                BTreeSet::new(),
                BTreeSet::from(["linear".into()]),
                BTreeSet::new(),
            ],
        );
        assert_eq!(fresh.local_id("mcp", "linear"), Some("cloud-linear-1"));
        let servers = merge_servers(vec![local.clone()], &fresh, &doc);
        assert_eq!(servers.len(), 2);
        assert!(servers.iter().any(|s| s == &local));
        assert_eq!(servers[0].config["headers"]["Authorization"], "");
        // Remote deletion removes the link, never the private definition or secrets.
        bind(
            &mut fresh,
            &Doc::default(),
            [
                BTreeSet::new(),
                servers.iter().map(|s| s.id.clone()).collect(),
                BTreeSet::new(),
            ],
        );
        assert!(fresh.local_id("mcp", "linear").is_none());
        assert!(merge_servers(servers, &fresh, &Doc::default())
            .iter()
            .any(|s| s == &local));
        let mut legacy: Cache = serde_json::from_value(json!({"revision":3, "doc":doc})).unwrap();
        legacy.migrate();
        assert_eq!(legacy.cloud_id("mcp", "linear"), Some("linear".into()));
        let servers = merge_servers(vec![local], &legacy, &doc);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].config["url"], "https://cloud.test/mcp");
        assert_eq!(servers[0].config["headers"]["Authorization"], "private");
        assert!(Cache::default().doc.keys().is_empty());
        assert!(Cache::default().links.unwrap().is_empty());
    }

    #[test]
    fn equivalent_private_items_satisfy_the_personal_catalog_until_linked() {
        assert_eq!(
            source_key("https://github.com/JuliusBrussee/caveman.git"),
            source_key("juliusbrussee/caveman/")
        );
        assert_ne!(
            source_key("https://gitlab.test/Team/tool"),
            source_key("https://gitlab.test/team/tool")
        );
        assert_eq!(
            source_key("HTTPS://GitHub.com/A/b"),
            source_key("https://github.com/a/B")
        );
        assert_ne!(
            source_key("https://gitlab.test/team/tool.git"),
            source_key("https://gitlab.test/team/tool")
        );
        let local = plugins::Plugin {
            id: "caveman".into(),
            source: "~/.prometeu/plugins/caveman".into(),
            note: "local".into(),
            made: true,
            from: "https://github.com/JuliusBrussee/caveman".into(),
        };
        let skill = skills::Skill {
            id: "review".into(),
            description: "Review".into(),
            content: "Read changes".into(),
        };
        let mut cache: Cache = serde_json::from_value(json!({
            "revision": 1,
            "doc": {
                "plugins": [{"id": "caveman", "source": "https://github.com/juliusbrussee/caveman"}],
                "skills": [skill],
                "mcp": [{"id": "notion", "config": {"url": "https://same.test/mcp"}}],
            },
            "links": {"plugins:caveman": "cloud-caveman-1", "skills:review": "cloud-review-1", "mcp:notion": "cloud-notion-1"},
        }))
        .unwrap();
        let hub = vec![local.clone()];
        let skill_hub = vec![skill.clone()];
        let item = cache.doc.plugins[0].clone();
        // Equivalence reports availability without publishing future local edits.
        assert!(installed_plugin(&cache, &item, &hub) == Some(&local));
        assert!(installed_skill(&cache, &skill, &skill_hub) == Some(&skill));
        assert!(cache.cloud_id("plugins", "caveman").is_none());

        assert!(!link_equivalent(
            &mut cache, "mcp", "notion", &hub, &skill_hub
        ));
        let installed_copy = plugins::Plugin {
            id: "cloud-caveman-1".into(),
            ..local.clone()
        };
        assert!(!link_equivalent(
            &mut cache,
            "plugins",
            "caveman",
            &[local.clone(), installed_copy],
            &skill_hub
        ));
        assert!(link_equivalent(
            &mut cache, "plugins", "caveman", &hub, &skill_hub
        ));
        assert!(link_equivalent(
            &mut cache, "skills", "review", &hub, &skill_hub
        ));
        assert_eq!(cache.local_id("plugins", "caveman"), Some("caveman"));
        assert_eq!(cache.local_id("skills", "review"), Some("review"));
        assert!(installed_plugin(&cache, &item, &hub) == Some(&local));
        let other = plugins::Plugin {
            from: "https://github.com/someone/else".into(),
            ..local
        };
        assert!(!link_equivalent(
            &mut cache,
            "plugins",
            "caveman",
            &[other],
            &skill_hub
        ));
    }

    #[test]
    fn malformed_cloud_documents_cannot_overwrite_local_data() {
        assert!(parse(&json!({"catalog":null,"revision":null})).is_ok());
        assert!(parse(&json!({"catalog":{},"revision":null})).is_err());
        assert!(parse(&json!({"catalog":{"skills":[{"id":"../escape","description":"d","content":"c"}]},"revision":0})).is_err());
        assert!(parse(&json!({"catalog":{"mcp":[{"id":"x","config":{"command":"npx","env":{"TOKEN":"secret"}}}]},"revision":0})).is_err());
        assert!(parse(
            &json!({"catalog":{"plugins":[{"id":"x","source":"/private"}]},"revision":0})
        )
        .is_err());
        assert!(parse(&json!({"catalog":{"plugins":[{"id":"x","source":"org/repo"},{"id":"x","source":"org/repo"}]},"revision":0})).is_err());
    }

    #[test]
    fn mcp_equivalence_ignores_object_order_but_preserves_command_args_and_secrets() {
        let local = mcp::Server {
            id: "order-test".into(),
            note: "Local".into(),
            config: json!({"type":"stdio", "command":"npx", "args":["-y", "server"], "env":{"FIRST":"local-secret", "SECOND":"another-secret"}, "headers":{"Authorization":"Bearer local"}}),
        };
        let mut item = mcp::Server {
            id: local.id.clone(),
            note: "Team".into(),
            config: json!({"headers":{"Authorization":""}, "env":{"SECOND":"", "FIRST":""}, "args":["-y", "server"], "command":"npx", "type":"stdio"}),
        };
        assert!(same_mcp(&item, &local));
        assert_eq!(merge(&item.config, &local.config), local.config);
        item.config["args"] = json!(["server", "-y"]);
        assert!(!same_mcp(&item, &local));
        item.config["args"] = local.config["args"].clone();
        item.config["command"] = json!("other");
        assert!(!same_mcp(&item, &local));
    }

    #[test]
    fn secrets_stay_on_the_mac_and_return_during_merge() {
        let local = mcp::Server {
            id: "x".into(),
            config: json!({"command": "npx", "env": {"TOKEN": "s3cret", "MODE": "dev"}, "headers": {"Authorization": "Bearer t"}}),
            note: "n".into(),
        };
        let cloud = blank(&local);
        assert_eq!(cloud.config["env"]["TOKEN"], "");
        assert_eq!(cloud.config["headers"]["Authorization"], "");
        assert_eq!(cloud.config["command"], "npx");
        let mut changed = cloud.config.clone();
        changed["command"] = json!("bunx");
        let merged = merge(&changed, &local.config);
        assert_eq!(merged["command"], "bunx");
        assert_eq!(merged["env"]["TOKEN"], "s3cret");
        assert_eq!(merged["headers"]["Authorization"], "Bearer t");
        assert_eq!(
            merge(&json!({"url": "u"}), &local.config),
            json!({"url": "u"})
        );
    }

    #[test]
    fn only_items_with_addresses_are_portable() {
        let p = |source: &str, from: &str| plugins::Plugin {
            id: "p".into(),
            source: source.into(),
            note: "".into(),
            made: !from.is_empty(),
            from: from.into(),
        };
        assert!(portable(&p("~/dev/x", "")).is_none());
        assert_eq!(
            portable(&p("~/.prometeu/plugins/x", "https://github.com/a/x"))
                .unwrap()
                .source,
            "https://github.com/a/x"
        );
        assert_eq!(
            portable(&p("https://x.test/p.zip", "")).unwrap().source,
            "https://x.test/p.zip"
        );
    }
}
