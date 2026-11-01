//! Bundled search uses the same worktree-scoped engine in desktop and WSL sessions.
use crate::mcp::Server;
use prometeu_search::{Engine, SCHEMA_VERSION};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod map;
pub use map::{CodeMap, CodeMapRoot, CodeMapSnapshot, CodeMapStatus};

pub const MODE: &str = "--prometeu-search-mcp";

pub struct Search {
    pub executable: PathBuf,
    pub cache: PathBuf,
}

impl Search {
    pub fn servers(&self, workdir: &Path) -> Result<Vec<Server>, String> {
        let indexes = indexes(&self.cache, workdir)?;
        let multiple = indexes.len() > 1;
        indexes
            .into_iter()
            .map(|index| {
                let Index {
                    key,
                    root,
                    database,
                } = index;
                // JSON argv cannot represent non-UTF-8 paths. Refuse rather than index a different path.
                let paths = [&self.executable, &root, &database].map(|path| {
                    path.to_str().ok_or_else(|| {
                        prometeu_core::error::with_args(
                            "err.mcp.session",
                            &[("cause", "search path is not UTF-8".into())],
                        )
                    })
                });
                let [executable, root_path, database] = paths;
                let id = if multiple {
                    let name = crate::mcp::slug(
                        root.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("repo"),
                    );
                    format!(
                        "prometeu-search-{}-{}",
                        &name[..name.len().min(32)],
                        &key[..12]
                    )
                } else {
                    "prometeu-search".into()
                };
                Ok(Server {
                    id,
                    config: json!({
                        "command": executable?,
                        "args": [MODE, root_path?, database?],
                    }),
                    note: String::new(),
                })
            })
            .collect()
    }
}

struct Index {
    key: String,
    root: PathBuf,
    database: PathBuf,
}

fn indexes(cache: &Path, workdir: &Path) -> Result<Vec<Index>, String> {
    let cache = std::path::absolute(cache).map_err(error)?;
    roots(workdir)
        .map_err(error)?
        .into_iter()
        .map(|root| {
            let key = format!("{:x}", Sha256::digest(root.as_os_str().as_encoded_bytes()));
            let database = cache
                .join(format!("v{SCHEMA_VERSION}"))
                .join(&key)
                .join("index.sqlite");
            if root.to_str().is_none() || database.to_str().is_none() {
                return Err(error("search path is not UTF-8"));
            }
            Ok(Index {
                key,
                root,
                database,
            })
        })
        .collect()
}

fn error(cause: impl std::fmt::Display) -> String {
    prometeu_core::error::with_args("err.search", &[("cause", cause.to_string())])
}

fn roots(workdir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root = workdir.canonicalize()?;
    if !root.is_dir() {
        return Err(std::io::Error::other("search root must be a directory"));
    }
    let mut roots = Vec::new();
    // Multi-repository workspaces launch from a grouping directory. Each direct Git worktree
    // gets its own graph; a repository's nested repositories do not change its identity.
    if !root.join(".git").exists() {
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join(".git").exists() {
                roots.push(entry.path().canonicalize()?);
            }
        }
    }
    if roots.is_empty() {
        roots.push(root);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

/// Dispatch before either host initializes its UI, runtime, analytics or account state.
pub fn cli(args: impl IntoIterator<Item = OsString>) -> Option<Result<(), String>> {
    let mut args = args.into_iter();
    let mode = args.next()?;
    if mode != MODE {
        return None;
    }
    let result = {
        let root = args.next().map(PathBuf::from);
        let database = args.next().map(PathBuf::from);
        match (root, database, args.next()) {
            (Some(root), Some(database), None) if root.is_absolute() && database.is_absolute() => {
                serve(&root, &database)
            }
            _ => Err(format!(
                "usage: {MODE} <absolute-worktree> <absolute-database>"
            )),
        }
    };
    Some(result)
}

fn serve(root: &Path, database: &Path) -> Result<(), String> {
    let mut engine = Engine::open(root, Some(database)).map_err(|error| format!("{error:#}"))?;
    let _watcher = match engine.watch() {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            eprintln!("Background watching unavailable: {error:#}. Queries still reconcile files.");
            None
        }
    };
    prometeu_search::mcp::serve(
        &mut engine,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
    .map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worktree_identity_is_canonical_and_shared_across_workspace_shapes() {
        let temp = std::env::temp_dir().join(format!("search-roots-{}", uuid::Uuid::new_v4()));
        let group = temp.join("workspace with spaces");
        let first = group.join("first");
        let second = group.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join(".git"), "gitdir: /first").unwrap();
        std::fs::write(second.join(".git"), "gitdir: /second").unwrap();
        let search = Search {
            executable: "/app with spaces/Prometeu".into(),
            cache: temp.join("cache"),
        };
        let servers = search.servers(&group).unwrap();
        assert_eq!(servers.len(), 2);
        assert_ne!(servers[0].id, servers[1].id);
        assert_ne!(servers[0].config["args"][2], servers[1].config["args"][2]);
        let alias = temp.join("alias");
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let direct = search.servers(&alias).unwrap();
        assert_eq!(direct[0].config, servers[0].config);
        assert_eq!(direct[0].config["command"], "/app with spaces/Prometeu");
        assert!(
            !search.cache.exists(),
            "preparation must not index or create caches"
        );
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn dispatch_rejects_missing_relative_and_extra_paths() {
        for args in [
            vec![MODE],
            vec![MODE, "relative", "/cache"],
            vec![MODE, "/worktree", "relative"],
            vec![MODE, "/worktree", "/cache", "extra"],
        ] {
            assert!(cli(args.into_iter().map(OsString::from)).unwrap().is_err());
        }
        assert!(cli([OsString::from("--ordinary-host-mode")]).is_none());
    }
}
