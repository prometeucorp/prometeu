//! Shared local plugin registration and repository import. Hosts own publication and cache cleanup.
use crate::packages::{self, remote, slug, PackageCatalog, PackageFiles, Plugin};
use prometeu_core::{
    command::{CommandOutput, CommandPolicy, CommandRunner, OutputPolicy},
    error::{code, with_args},
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

#[derive(serde::Serialize)]
pub struct Found {
    pub dir: String,
    pub plugins: Vec<Plugin>,
    pub saved: bool,
}

pub trait Plugins: Send + Sync {
    fn look(&self, source: String) -> Result<Plugin, String>;
    fn save(&self, plugin: Plugin) -> Result<Vec<Plugin>, String>;
    fn remove(&self, id: &str) -> Result<Vec<Plugin>, String>;
    fn install(&self, source: String) -> Result<Found, String>;
    fn update(&self, id: String) -> Result<Vec<Plugin>, String>;
    fn scrap(&self, dir: String);
}
pub struct PluginLibrary {
    pub root: PathBuf,
    pub home: PathBuf,
    pub catalog: Arc<dyn PackageCatalog>,
    pub files: Arc<dyn PackageFiles>,
    pub runner: Arc<dyn CommandRunner<Command>>,
}
impl PluginLibrary {
    pub fn store(&self) -> PathBuf {
        self.root.join("plugins")
    }
    fn owned(&self, path: &Path) -> bool {
        let (Ok(root), Ok(path)) = (self.store().canonicalize(), path.canonicalize()) else {
            return false;
        };
        path != root && path.starts_with(root)
    }
    fn expand(&self, source: &str) -> String {
        packages::expand(source, &self.home)
    }
    fn load(&self) -> Vec<Plugin> {
        self.catalog.load()
    }
    fn write_hub(&self, plugins: &[Plugin]) -> Result<(), String> {
        self.files
            .write_private(
                &self.root.join("plugins.json"),
                &serde_json::to_string_pretty(plugins).map_err(|e| e.to_string())?,
            )
            .map_err(|cause| with_args("err.plugin.save", &[("cause", cause)]))
    }
    fn run(&self, command: &mut Command, error: &str) -> Result<CommandOutput, String> {
        self.runner
            .run(
                command,
                &[],
                CommandPolicy {
                    timeout: Duration::from_secs(20),
                    stdout: OutputPolicy::Capture { limit: 256 * 1024 },
                    stderr: OutputPolicy::Capture { limit: 256 * 1024 },
                },
            )
            .map_err(|cause| with_args(error, &[("cause", format!("{cause:?}"))]))
    }
    fn clone(&self, url: &str, dir: &Path) -> Result<(), String> {
        let mut command = Command::new("git");
        command
            .args(["clone", "--depth", "1", "-q", "--", url])
            .arg(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
        let result = self.run(&mut command, "err.plugin.clone");
        match result {
            Ok(out) if out.success => Ok(()),
            other => {
                let _ = std::fs::remove_dir_all(dir);
                match other {
                    Ok(out) => Err(with_args(
                        "err.plugin.clone",
                        &[("cause", last_line(&String::from_utf8_lossy(&out.stderr)))],
                    )),
                    Err(error) => Err(error),
                }
            }
        }
    }
    pub fn git(&self, root: &Path, args: &[&str]) -> Result<CommandOutput, String> {
        let mut command = Command::new("git");
        command
            .current_dir(root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -oBatchMode=yes");
        self.run(&mut command, "err.plugin.pull")
    }
    pub fn check_source(&self, source: &str) -> Result<(), String> {
        if source.is_empty() {
            return Err(code("err.plugin.noSource"));
        }
        if remote(source) {
            return Ok(());
        }
        let path = PathBuf::from(self.expand(source));
        if !path.exists() {
            return Err(with_args(
                "err.plugin.noPath",
                &[("path", path.display().to_string())],
            ));
        }
        // The CLI opens ZIP sources directly; directories require a plugin manifest.
        if path.is_dir() && !manifest_path(&path).exists() {
            return Err(with_args(
                "err.plugin.notPlugin",
                &[("path", path.display().to_string())],
            ));
        }
        Ok(())
    }
    pub fn look_source(&self, source: String) -> Result<Plugin, String> {
        let source = source.trim().to_string();
        self.check_source(&source)?;
        let path = PathBuf::from(self.expand(&source));
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
    pub fn save_local(&self, plugin: Plugin) -> Result<Vec<Plugin>, String> {
        let plugin = Plugin {
            id: plugin.id.trim().to_string(),
            source: plugin.source.trim().to_string(),
            note: plugin.note.trim().to_string(),
            made: plugin.made,
            from: plugin.from.trim().to_string(),
        };
        if plugin.id.is_empty() {
            return Err(code("err.plugin.noName"));
        }
        self.check_source(&plugin.source)?;
        let mut plugins = self.load();
        packages::register(&mut plugins, plugin);
        self.write_hub(&plugins)?;
        Ok(plugins)
    }
    pub fn install_into(&self, source: String, register: bool) -> Result<Found, String> {
        let url = git_url(&source);
        if url.is_empty() {
            return Err(code("err.plugin.noSource"));
        }
        let dir = self.store().join(repo_name(&url));
        if dir.exists() {
            // An occupied clone used by hub entries requires updating. An unused clone left by an
            // abandoned selection may be replaced.
            if !self.owned(&dir) || self.lives_in(&dir) {
                return Err(with_args("err.plugin.exists", &[("name", repo_name(&url))]));
            }
            std::fs::remove_dir_all(&dir).ok();
        }
        std::fs::create_dir_all(self.store())
            .map_err(|e| with_args("err.plugin.clone", &[("cause", e.to_string())]))?;
        self.clone(&url, &dir)?;
        let plugins = plugins_in(&dir, &url);
        if plugins.is_empty() {
            std::fs::remove_dir_all(&dir).ok();
            return Err(with_args("err.plugin.noPluginIn", &[("url", url)]));
        }
        // A single discovered plugin is registered; activation remains a workspace choice.
        let saved = plugins.len() == 1;
        if saved && register {
            if self.load().iter().any(|p| p.id == plugins[0].id) {
                return Err(code("err.catalog.conflict"));
            }
            self.save_local(plugins[0].clone())?;
        }
        Ok(Found {
            dir: dir.display().to_string(),
            plugins,
            saved,
        })
    }
    pub fn scrap_directory(&self, dir: String) {
        let dir = PathBuf::from(self.expand(&dir));
        if self.owned(&dir) && !self.lives_in(&dir) {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
    pub fn update_plugin(&self, id: String) -> Result<Vec<Plugin>, String> {
        let plugin = self
            .load()
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| code("err.plugin.gone"))?;
        let dir = PathBuf::from(self.expand(&plugin.source));
        let root = self
            .git_root(&dir)
            .ok_or_else(|| code("err.plugin.noGit"))?;
        let out = self.git(&root, &["pull", "--ff-only", "-q"])?;
        if !out.success {
            return Err(with_args(
                "err.plugin.pull",
                &[("cause", last_line(&String::from_utf8_lossy(&out.stderr)))],
            ));
        }
        // Re-run discovery so skill-bundle packages under `.prometeu/packages/` are rebuilt from
        // the updated tree; ordinary root manifests still resolve through the same path.
        let mut fresh = plugins_in(&root, &plugin.from)
            .into_iter()
            .find(|entry| entry.id == plugin.id)
            .ok_or_else(|| code("err.plugin.gone"))?;
        fresh.made = plugin.made;
        self.save_local(fresh)
    }
    pub fn lives_in(&self, dir: &Path) -> bool {
        self.load().iter().any(|p| {
            let path = PathBuf::from(self.expand(&p.source));
            path.canonicalize()
                .unwrap_or(path)
                .starts_with(dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()))
        })
    }
    pub fn git_root(&self, dir: &Path) -> Option<PathBuf> {
        let mut at = dir;
        loop {
            if at.join(".git").exists() {
                return Some(at.to_path_buf());
            }
            at = at.parent()?;
            if !at.starts_with(self.store()) {
                return None;
            }
        }
    }
    pub fn remove_local(&self, mut plugins: Vec<Plugin>, id: &str) -> Vec<Plugin> {
        if let Some(gone) = plugins.iter().find(|p| p.id == id) {
            let dir = PathBuf::from(self.expand(&gone.source));
            if gone.made && self.owned(&dir) {
                let root = self.git_root(&dir).unwrap_or_else(|| dir.clone());
                let others_in_root = plugins.iter().filter(|p| p.id != id).any(|p| {
                    PathBuf::from(self.expand(&p.source))
                        .canonicalize()
                        .is_ok_and(|path| {
                            root.canonicalize()
                                .is_ok_and(|root| path.starts_with(root))
                        })
                });
                if !others_in_root {
                    // Drop the whole clone when this was the last hub entry from it.
                    std::fs::remove_dir_all(&root).ok();
                } else if dir
                    .canonicalize()
                    .ok()
                    .is_some_and(|path| path != root.canonicalize().unwrap_or(root.clone()))
                {
                    // Sibling packages still use the clone; remove only this package directory.
                    std::fs::remove_dir_all(&dir).ok();
                }
            }
        }
        plugins.retain(|p| p.id != id);
        plugins
    }
}
impl Plugins for PluginLibrary {
    fn look(&self, source: String) -> Result<Plugin, String> {
        self.look_source(source)
    }
    fn save(&self, plugin: Plugin) -> Result<Vec<Plugin>, String> {
        self.save_local(plugin)
    }
    fn remove(&self, id: &str) -> Result<Vec<Plugin>, String> {
        let hub = self.remove_local(self.load(), id);
        self.write_hub(&hub)?;
        Ok(hub)
    }
    fn install(&self, source: String) -> Result<Found, String> {
        self.install_into(source, true)
    }
    fn update(&self, id: String) -> Result<Vec<Plugin>, String> {
        self.update_plugin(id)
    }
    fn scrap(&self, dir: String) {
        self.scrap_directory(dir)
    }
}
pub fn guessed_name(source: &str) -> String {
    source
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".zip")
        .to_string()
}
pub fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(".claude-plugin").join("plugin.json")
}
pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}
pub fn git_url(source: &str) -> String {
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
pub fn repo_name(url: &str) -> String {
    if let Some((owner, repo)) = github_owner_repo(url) {
        return slug(&format!("{owner}-{repo}"));
    }
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    slug(name)
}

/// Prefer `owner-repo` clone folders for GitHub URLs so two `*/skills` repositories do not collide.
pub fn github_owner_repo(url: &str) -> Option<(String, String)> {
    let text = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = if let Some(rest) = text.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = text
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .strip_prefix("github.com/")
    {
        rest
    } else if let Some(rest) = text
        .strip_prefix("ssh://git@github.com/")
        .or_else(|| text.strip_prefix("ssh://git@github.com:"))
    {
        rest
    } else {
        return None;
    };
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}
pub fn last_line(text: &str) -> String {
    text.trim()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}
pub fn plugins_in(dir: &Path, from: &str) -> Vec<Plugin> {
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
            let Some(at) = within(dir, rel) else {
                continue;
            };
            if manifest_path(&at).exists() {
                found.push(read_plugin(&at, from));
                continue;
            }
            // Anthropic skill-bundle entries declare skills without a plugin.json (strict: false).
            if let Some(plugin) = synthesize_marketplace_skills(dir, entry, &at, from) {
                found.push(plugin);
            }
        }
    }
    if found.is_empty() {
        found = scan(dir, from);
    }
    if found.is_empty() {
        found = bare_skill_packages(dir, from);
    }
    found.sort_by_key(|p| p.id.to_lowercase());
    found.dedup_by(|a, b| a.id == b.id || a.source == b.source);
    found
}
pub fn marketplace_local_source(entry: &Value) -> Option<&str> {
    match entry.get("source")? {
        Value::String(path) => Some(path),
        Value::Object(source) if source.get("source")?.as_str()? == "local" => {
            source.get("path")?.as_str()
        }
        _ => None,
    }
}

/// Build a portable package from a marketplace entry that lists skill directories instead of a manifest.
fn synthesize_marketplace_skills(
    clone: &Path,
    entry: &Value,
    source_root: &Path,
    from: &str,
) -> Option<Plugin> {
    let skills = marketplace_skill_dirs(entry, source_root);
    if skills.is_empty() {
        return None;
    }
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| skills.first().map(|(name, _)| name.clone()))?;
    let description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let description = if description.is_empty() {
        skills
            .first()
            .and_then(|(_, path)| skill_meta(path).1)
            .unwrap_or_default()
    } else {
        description
    };
    synthesize_skill_package(clone, &name, &description, &skills, from)
}

fn marketplace_skill_dirs(entry: &Value, source_root: &Path) -> Vec<(String, PathBuf)> {
    entry
        .get("skills")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let rel = value.as_str()?;
            let at = within(source_root, rel)?;
            skill_dir_entry(&at)
        })
        .collect()
}

fn bare_skill_packages(dir: &Path, from: &str) -> Vec<Plugin> {
    let Ok(entries) = std::fs::read_dir(dir.join("skills")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let at = entry.path();
        let Some((name, path)) = skill_dir_entry(&at) else {
            continue;
        };
        let (front_name, description) = skill_meta(&path);
        let id = front_name.unwrap_or_else(|| name.clone());
        if let Some(plugin) = synthesize_skill_package(
            dir,
            &id,
            &description.unwrap_or_default(),
            &[(name, path)],
            from,
        ) {
            found.push(plugin);
        }
    }
    found
}

fn skill_dir_entry(at: &Path) -> Option<(String, PathBuf)> {
    if !at.is_dir() || !at.join("SKILL.md").is_file() {
        return None;
    }
    let name = at.file_name()?.to_str()?.to_string();
    if name.is_empty() || name.starts_with('.') {
        return None;
    }
    Some((name, at.to_path_buf()))
}

fn skill_meta(skill_dir: &Path) -> (Option<String>, Option<String>) {
    let text = std::fs::read_to_string(skill_dir.join("SKILL.md")).unwrap_or_default();
    skill_frontmatter(&text)
}

/// Extract `name` and `description` from YAML frontmatter without a YAML dependency.
pub fn skill_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.trim_start_matches('\u{feff}').lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return (None, None);
    }
    let body: Vec<&str> = lines.take_while(|line| line.trim_end() != "---").collect();
    let value = |key: &str| -> Option<String> {
        let at = body.iter().position(|line| {
            line.strip_prefix(key)
                .is_some_and(|rest| rest.trim_start().starts_with(':'))
        })?;
        let raw = body[at][key.len()..].trim_start()[1..].trim();
        let folded = if raw.starts_with('"') {
            let end = raw[1..]
                .find('"')
                .map(|index| index + 2)
                .unwrap_or(raw.len());
            let quoted = &raw[..end];
            serde_json::from_str::<String>(quoted)
                .unwrap_or_else(|_| quoted.trim_matches('"').to_string())
        } else if raw.starts_with('\'') {
            let end = raw[1..]
                .find('\'')
                .map(|index| index + 2)
                .unwrap_or(raw.len());
            let inner = &raw[1..end.saturating_sub(1)];
            inner.replace("''", "'")
        } else if raw.is_empty() || raw.starts_with('>') || raw.starts_with('|') {
            body[at + 1..]
                .iter()
                .take_while(|line| line.trim().is_empty() || line.starts_with([' ', '\t']))
                .map(|line| line.trim())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            raw.split('#').next().unwrap_or(raw).trim().to_string()
        };
        let line = folded.split_whitespace().collect::<Vec<_>>().join(" ");
        (!line.is_empty()).then_some(line)
    };
    (value("name"), value("description"))
}

/// Write Claude and Codex manifests plus copied skill trees under `.prometeu/packages/<id>/`.
fn synthesize_skill_package(
    clone: &Path,
    id: &str,
    description: &str,
    skills: &[(String, PathBuf)],
    from: &str,
) -> Option<Plugin> {
    let id = slug(id);
    if id.is_empty() || skills.is_empty() {
        return None;
    }
    let package = clone.join(".prometeu").join("packages").join(&id);
    let _ = std::fs::remove_dir_all(&package);
    std::fs::create_dir_all(package.join(".claude-plugin")).ok()?;
    std::fs::create_dir_all(package.join(".codex-plugin")).ok()?;
    let manifest = serde_json::json!({
        "name": id,
        "description": description,
        "version": "0.0.0",
    });
    let body = serde_json::to_string_pretty(&manifest).ok()?;
    std::fs::write(package.join(".claude-plugin").join("plugin.json"), &body).ok()?;
    let mut native = manifest;
    native["skills"] = serde_json::json!("./skills/");
    std::fs::write(
        package.join(".codex-plugin").join("plugin.json"),
        serde_json::to_string_pretty(&native).ok()?,
    )
    .ok()?;
    for (name, src) in skills {
        let folder = slug(name);
        let folder = if folder.is_empty() {
            name.clone()
        } else {
            folder
        };
        let dest = package.join("skills").join(&folder);
        copy_tree(src, &dest).ok()?;
    }
    Some(Plugin {
        id,
        source: package.display().to_string(),
        note: description.to_string(),
        made: true,
        from: from.to_string(),
    })
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| error.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), dest).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

pub fn scan(dir: &Path, from: &str) -> Vec<Plugin> {
    let mut found = Vec::new();
    for root in [dir.to_path_buf(), dir.join("plugins")] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let at = entry.path();
            if at.is_dir()
                && manifest_path(&at).exists()
                && at
                    .canonicalize()
                    .is_ok_and(|at| dir.canonicalize().is_ok_and(|dir| at.starts_with(dir)))
            {
                found.push(read_plugin(&at, from));
            }
        }
    }
    found
}
pub fn within(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        return Some(dir.to_path_buf());
    }
    let at = dir.join(rel);
    if rel.starts_with('/') || at.components().any(|c| c.as_os_str() == "..") {
        return None;
    }
    if at.exists()
        && !at
            .canonicalize()
            .ok()?
            .starts_with(dir.canonicalize().ok()?)
    {
        return None;
    }
    Some(at)
}
pub fn read_plugin(dir: &Path, from: &str) -> Plugin {
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
pub fn trim(plugin: Plugin) -> Plugin {
    Plugin {
        id: plugin.id.trim().to_string(),
        source: plugin.source.trim().to_string(),
        note: plugin.note.trim().to_string(),
        made: plugin.made,
        from: plugin.from.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Files;
    impl PackageFiles for Files {
        fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
            std::fs::create_dir_all(path).map_err(|e| e.to_string())
        }
        fn write_private(&self, _: &Path, _: &str) -> Result<(), String> {
            Err("fixture write refused".into())
        }
    }
    struct Timeout;
    impl CommandRunner<Command> for Timeout {
        fn run(
            &self,
            command: &mut Command,
            _: &[u8],
            policy: CommandPolicy,
        ) -> Result<CommandOutput, prometeu_core::command::CommandError> {
            assert_eq!(policy.timeout, Duration::from_secs(20));
            assert_eq!(policy.stdout, OutputPolicy::Capture { limit: 256 * 1024 });
            assert_eq!(command.get_program(), "git");
            Err(prometeu_core::command::CommandError::Timeout)
        }
    }
    #[test]
    fn failed_commands_and_registry_writes_never_report_a_registered_plugin() {
        let root = std::env::temp_dir().join(format!("plugin-library-{}", uuid::Uuid::new_v4()));
        let library = PluginLibrary {
            root: root.clone(),
            home: root.clone(),
            catalog: Arc::new(packages::FilePackageCatalog {
                path: root.join("plugins.json"),
            }),
            files: Arc::new(Files),
            runner: Arc::new(Timeout),
        };
        let error = library
            .install("https://example.invalid/package".into())
            .err()
            .unwrap();
        assert!(error.contains("err.plugin.clone") && error.contains("Timeout"));
        assert!(!library.store().join("package").exists());
        let source = root.join("user-package");
        std::fs::create_dir_all(source.join(".claude-plugin")).unwrap();
        std::fs::write(manifest_path(&source), r#"{"name":"user-package"}"#).unwrap();
        let entry = library.look(source.display().to_string()).unwrap();
        assert!(library
            .save(entry)
            .err()
            .unwrap()
            .contains("fixture write refused"));
        assert!(library.load().is_empty());
        let escape = library.store().join("../user-package");
        library.scrap(escape.display().to_string());
        assert!(source.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn github_clone_directories_include_the_owner() {
        assert_eq!(
            repo_name("https://github.com/JuliusBrussee/caveman"),
            "juliusbrussee-caveman"
        );
        assert_eq!(
            repo_name("git@github.com:dietrichgebert/ponytail.git"),
            "dietrichgebert-ponytail"
        );
        assert_eq!(
            repo_name("https://github.com/caldoai/skills"),
            "caldoai-skills"
        );
        assert_eq!(
            repo_name("https://github.com/typesafe-ai/skills"),
            "typesafe-ai-skills"
        );
        assert_eq!(repo_name("https://gitlab.com/time/x.git"), "x");
    }

    #[test]
    fn skill_bundle_marketplace_becomes_a_portable_package() {
        let root = std::env::temp_dir().join(format!("skill-bundle-{}", uuid::Uuid::new_v4()));
        let skill = root.join("skills/untangle-codebase");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: untangle-codebase\ndescription: Untangle a codebase.\n---\n\nDo the work.\n",
        )
        .unwrap();
        std::fs::write(
            skill.join("notes.md"),
            "supporting asset\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{
              "name": "caldo-skills",
              "plugins": [{
                "name": "untangle-codebase",
                "description": "Untangle a whole codebase.",
                "source": "./",
                "strict": false,
                "skills": ["./skills/untangle-codebase"]
              }]
            }"#,
        )
        .unwrap();

        let found = plugins_in(&root, "https://github.com/caldoai/skills");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "untangle-codebase");
        assert_eq!(found[0].note, "Untangle a whole codebase.");
        assert!(found[0].made);
        let package = PathBuf::from(&found[0].source);
        assert!(package.ends_with(".prometeu/packages/untangle-codebase"));
        assert!(manifest_path(&package).exists());
        let native: Value = serde_json::from_str(
            &std::fs::read_to_string(package.join(".codex-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(native["skills"], "./skills/");
        assert_eq!(
            std::fs::read_to_string(package.join("skills/untangle-codebase/notes.md")).unwrap(),
            "supporting asset\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bare_skill_directories_become_portable_packages() {
        let root = std::env::temp_dir().join(format!("bare-skills-{}", uuid::Uuid::new_v4()));
        let skill = root.join("skills/review");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Review the diff.\n---\n\nRead carefully.\n",
        )
        .unwrap();

        let found = plugins_in(&root, "https://example.com/skills.git");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "review");
        assert_eq!(found[0].note, "Review the diff.");
        assert!(PathBuf::from(&found[0].source)
            .join("skills/review/SKILL.md")
            .exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
