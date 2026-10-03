//! Repository declarations shared by native hosts; reading never starts scripts.
use prometeu_core::selection::Tools;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub trait RepositorySettings: Send + Sync {
    fn read(&self, worktree: &Path, repository: &Path) -> Scripts;
}
pub struct NativeSettings;
impl RepositorySettings for NativeSettings {
    fn read(&self, worktree: &Path, repository: &Path) -> Scripts {
        read_for(worktree, repository)
    }
}

/// Search Prometeu settings first so local overrides do not require changing Conductor
/// configuration.
pub const FILES: [&str; 2] = [".prometeu/settings.toml", ".conductor/settings.toml"];

/// The commented example documents the repository/app contract when no settings file exists.
pub const TEMPLATE: &str = r#"# Scripts Prometeu runs in this repository.
#
# `setup`   runs when a worktree is created; the first agent message waits for it
# `run`     powers the Run button
# `archive` runs before archiving the workspace
#
# Scripts run through `/bin/sh -lc` in the worktree directory and receive:
#
#   $PROMETEU_WORKSPACE_PATH  the worktree where the script runs
#   $PROMETEU_ROOT_PATH       the source repository
#   $PROMETEU_WORKSPACE_NAME  this workspace's name
#   $PROMETEU_PORT            its reserved port, with nine more through +9
#   $PORT                     the same port for tools using this convention
#
# Fixed ports conflict across worktrees; use $PROMETEU_PORT.

[scripts]
setup = "npm install"
run = "npm run dev -- --port $PROMETEU_PORT"

# Files copied from the source clone before setup: ignored files that scripts
# cannot recreate. Without this list, copy the root .env and its variants;
# `copy = []` disables copying. Never overwrite files already in the worktree.
#
# [worktree]
# copy = [".env", "config/master.key"]

# Where a conversation started from a skill writes its artifacts, relative to
# the repository. Without it Prometeu names no path.
#
# [method]
# artifacts = "docs/specs"
"#;

#[derive(Deserialize, Default)]
struct Table {
    setup: Option<String>,
    /// Parse run as a raw Value to support both a command string and named script tables without
    /// invalidating the entire configuration on a shape mismatch.
    run: Option<toml::Value>,
    archive: Option<String>,
}

/// Worktree settings describe preparation performed before scripts, so they are separate from
/// scripts.
#[derive(Deserialize, Default)]
struct WorktreeTable {
    /// None enables automatic copying; an explicit empty list disables copying.
    copy: Option<Vec<String>>,
}

/// Method settings shared by the team through the versioned file (ADR 0057).
#[derive(Deserialize, Default)]
struct MethodTable {
    /// Where artifacts of a conversation started from a skill land, relative to the repository.
    artifacts: Option<String>,
}

#[derive(Deserialize, Default)]
struct File {
    #[serde(default)]
    scripts: Table,
    #[serde(default)]
    worktree: WorktreeTable,
    /// The project layer of the tool selection (ADR 0045). Versioned, so it never activates on its
    /// own; project-declared items are gated on trust before injection.
    #[serde(default)]
    tools: Tools,
    #[serde(default)]
    method: MethodTable,
}

#[derive(Serialize, Clone)]
pub struct Run {
    /// Use the scripts.run.<name> key, or run for the single-string form, as the menu identity.
    pub name: String,
    pub command: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Scripts {
    /// Record the selected settings file, or None when the UI should show missing configuration.
    pub file: Option<String>,
    /// Inherited settings come from the original clone. Opening them for worktree editing requires
    /// a local copy.
    pub inherited: bool,
    pub setup: Option<String>,
    pub runs: Vec<Run>,
    pub archive: Option<String>,
    /// Resolved files to copy from the clone determine the Setup header and can create a Setup tab
    /// without a command.
    pub copy: Vec<String>,
    /// The project layer of the tool selection read from the authoritative settings file, inherited
    /// from the clone like the scripts. Absent axes inherit the layers above (ADR 0045).
    pub tools: Tools,
    /// The `[method] artifacts` path, relative to the repository and inherited from the clone like
    /// the scripts. `None` when undeclared or unsafe, so the app injects no path (ADR 0057).
    pub artifacts: Option<String>,
    /// Retain the raw optional copy declaration for read_for; the frontend receives the resolved
    /// list.
    #[serde(skip)]
    declared: Option<Vec<String>>,
}

impl Scripts {
    /// Choose the explicitly default run, otherwise the first entry.
    pub fn run(&self, name: Option<&str>) -> Option<&Run> {
        match name {
            Some(n) => self.runs.iter().find(|r| r.name == n),
            None => self.runs.first(),
        }
    }
}

/// Use the entire worktree configuration or the entire clone fallback; never merge partial script
/// sets. Sessions in the clone need only one read.
pub fn read_for(worktree: &Path, repo: &Path) -> Scripts {
    let mut found = read(worktree);
    if found.file.is_none() && worktree != repo {
        found = read(repo);
        found.inherited = found.file.is_some();
    }
    // Resolve copies after selecting the authoritative settings file, including inherited clone
    // settings.
    found.copy = copies(worktree, repo, found.declared.as_deref());
    found
}

pub fn read(root: &Path) -> Scripts {
    for file in FILES {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        // Malformed TOML yields no scripts while leaving the file available for repair.
        let parsed: File = toml::from_str(&text).unwrap_or_default();
        return Scripts {
            file: Some(file.to_string()),
            inherited: false,
            setup: trimmed(parsed.scripts.setup),
            runs: runs(parsed.scripts.run),
            archive: trimmed(parsed.scripts.archive),
            copy: Vec::new(),
            tools: parsed.tools,
            artifacts: artifacts(parsed.method.artifacts),
            declared: parsed.worktree.copy,
        };
    }
    Scripts::default()
}

/// Normalize the declared artifact path: relative, confined to the repository, without a leading
/// `./` or trailing slash. Anything else is ignored rather than guessed.
fn artifacts(value: Option<String>) -> Option<String> {
    let raw = trimmed(value)?;
    let rel = raw.trim_start_matches("./").trim_end_matches('/');
    safe(rel).map(|path| path.to_string_lossy().into_owned())
}

fn trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn runs(spec: Option<toml::Value>) -> Vec<Run> {
    match spec {
        // The string run form defines one unnamed script.
        Some(toml::Value::String(command)) => trimmed(Some(command))
            .map(|command| {
                vec![Run {
                    name: "run".into(),
                    command,
                }]
            })
            .unwrap_or_default(),
        // Named run tables each provide a command.
        Some(toml::Value::Table(table)) => {
            let mut list: Vec<(bool, Run)> = table
                .into_iter()
                .filter_map(|(name, value)| {
                    let entry = value.as_table()?;
                    let command = trimmed(entry.get("command")?.as_str().map(str::to_string))?;
                    let default = entry
                        .get("default")
                        .and_then(toml::Value::as_bool)
                        .unwrap_or(false);
                    Some((default, Run { name, command }))
                })
                .collect();
            // Place the default script first and preserve the parsed order of remaining entries.
            list.sort_by_key(|(is_default, _)| !is_default);
            list.into_iter().map(|(_, run)| run).collect()
        }
        _ => Vec::new(),
    }
}

/// Resolve the files present in the clone, not only files missing from the destination, so Setup
/// does not disappear after copying. Sessions running directly in the clone copy nothing.
pub fn copies(worktree: &Path, repo: &Path, declared: Option<&[String]>) -> Vec<String> {
    if worktree == repo {
        return Vec::new();
    }
    match declared {
        Some(list) => list
            .iter()
            .map(|rel| rel.trim().to_string())
            .filter(|rel| safe(rel).is_some_and(|path| repo.join(path).exists()))
            .collect(),
        None => auto(repo),
    }
}

/// Automatically include root .env variants but exclude versioned examples already present in
/// worktrees. Copying never overwrites existing files.
fn auto(repo: &Path) -> Vec<String> {
    const SAMPLES: [&str; 4] = [".env.example", ".env.sample", ".env.template", ".env.dist"];
    let Ok(dir) = std::fs::read_dir(repo) else {
        return Vec::new();
    };
    let mut out: Vec<String> = dir
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let keep = name.starts_with(".env")
                && !SAMPLES.contains(&name.as_str())
                && entry.path().is_file();
            keep.then_some(name)
        })
        .collect();
    out.sort();
    out
}

/// Require relative paths confined to the worktree; reject absolute paths and parent traversal.
pub fn safe(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel.trim());
    let inside = path.components().all(|c| matches!(c, Component::Normal(_)));
    (inside && path.components().next().is_some()).then(|| path.to_path_buf())
}
