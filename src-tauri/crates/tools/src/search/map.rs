//! Interactive map reads the same derived index as the worktree's agent MCP server.
use super::{error, indexes};
use prometeu_core::lock::lock;
use prometeu_search::{Engine, GraphOptions, GraphResult, Overview};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Serialize)]
pub struct CodeMapRoot {
    pub id: String,
    pub label: String,
    pub path: String,
    pub relative: String,
}

#[derive(Debug, Serialize)]
pub struct CodeMapStatus {
    pub generation: u64,
    pub last_indexed: Option<u64>,
    pub files: usize,
    pub symbols: usize,
    pub references: usize,
}

#[derive(Debug, Serialize)]
pub struct CodeMapSnapshot {
    pub roots: Vec<CodeMapRoot>,
    pub root_id: String,
    pub status: CodeMapStatus,
    pub overview: Overview,
    pub result: Option<GraphResult>,
}

struct Active {
    root: PathBuf,
    engine: Engine,
}

pub struct CodeMap {
    cache: PathBuf,
    // ponytail: one active map bounds graph memory; add per-root caching only if switching is costly.
    active: Mutex<Option<Active>>,
}

impl CodeMap {
    pub fn new(cache: PathBuf) -> Self {
        Self {
            cache,
            active: Mutex::new(None),
        }
    }

    /// Hosts run this blocking operation outside their UI/request loop. No watcher is started.
    #[allow(clippy::too_many_arguments)]
    pub fn snapshot(
        &self,
        workdir: &Path,
        root: Option<&str>,
        query: &str,
        path: Option<&str>,
        line: Option<usize>,
        force: bool,
    ) -> Result<CodeMapSnapshot, String> {
        let options = GraphOptions {
            path: path.map(str::to_owned),
            line,
            limit: 50,
            max_bytes: 65_536,
            ..GraphOptions::default()
        };
        let query = if query.trim().is_empty() {
            if query.len() > 1024 || path.is_some() || line.is_some() {
                return Err(error("an overview cannot include a source selection"));
            }
            None
        } else {
            options.validate(query).map_err(error)?;
            Some(query)
        };
        let workdir = workdir.canonicalize().map_err(error)?;
        let indexes = indexes(&self.cache, &workdir)?;
        let selected = match root {
            Some(id) => indexes.iter().find(|index| index.key == id),
            None => indexes.first(),
        }
        .ok_or_else(|| error("search root is not part of this workspace"))?;
        let mut active = lock(&self.active);
        if active
            .as_ref()
            .is_none_or(|active| active.root != selected.root)
        {
            *active = Some(Active {
                root: selected.root.clone(),
                engine: Engine::open(&selected.root, Some(&selected.database))
                    .map_err(|cause| error(format!("{cause:#}")))?,
            });
        }
        let engine = &mut active.as_mut().unwrap().engine;
        if force {
            engine.sync()
        } else {
            engine.refresh()
        }
        .map_err(|cause| error(format!("{cause:#}")))?;
        let overview = engine.overview().map_err(error)?;
        let result = query
            .map(|query| engine.explore(query, &options))
            .transpose()
            .map_err(error)?;
        let status = engine.status().map_err(error)?;
        Ok(CodeMapSnapshot {
            roots: indexes
                .iter()
                .map(|index| {
                    Ok(CodeMapRoot {
                        id: index.key.clone(),
                        label: index
                            .root
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        path: index.root.to_string_lossy().into_owned(),
                        relative: index
                            .root
                            .strip_prefix(&workdir)
                            .map_err(error)?
                            .components()
                            .map(|part| part.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/"),
                    })
                })
                .collect::<Result<_, String>>()?,
            root_id: selected.key.clone(),
            status: CodeMapStatus {
                generation: status.generation,
                last_indexed: status.last_indexed,
                files: status.files,
                symbols: status.symbols,
                references: status.references,
            },
            overview,
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Search;
    use std::fs;

    #[test]
    fn map_reuses_agent_indexes_and_bounds_selection_to_workspace_roots() {
        fn thread_safe<T: Send + Sync>() {}
        thread_safe::<CodeMap>();
        let temp = std::env::temp_dir().join(format!("code-map-{}", uuid::Uuid::new_v4()));
        let group = temp.join("workspace");
        let first = group.join("first");
        let second = group.join("second");
        for root in [&first, &second] {
            fs::create_dir_all(root).unwrap();
            fs::write(root.join(".git"), "gitdir: /fixture").unwrap();
        }
        fs::write(first.join("leaf.ts"), "export function orchid() {}").unwrap();
        fs::write(
            first.join("main.ts"),
            "import { orchid } from './leaf'; function run() { orchid(); }",
        )
        .unwrap();
        fs::write(second.join("leaf.ts"), "export function cactus() {}").unwrap();
        let cache = temp.join("cache");
        let map = CodeMap::new(cache.clone());
        let search = Search {
            executable: "/app".into(),
            cache,
        };
        let servers = search.servers(&group).unwrap();
        let snapshot = map.snapshot(&group, None, "", None, None, false).unwrap();
        assert_eq!(snapshot.roots.len(), 2);
        assert_eq!(snapshot.roots[0].relative, "first");
        assert_eq!(snapshot.roots[1].relative, "second");
        assert_eq!(snapshot.status.files, 2);
        assert!(snapshot.status.last_indexed.is_some());
        assert!(snapshot.result.is_none());
        assert_eq!(snapshot.overview.edges.len(), 1);
        assert!(!snapshot.overview.truncated);
        let first_id = snapshot.root_id;
        let second_id = snapshot.roots[1].id.clone();
        let database = Path::new(servers[0].config["args"][2].as_str().unwrap());
        let writer = Engine::open(&first, Some(database)).unwrap();
        assert_eq!(
            writer.status().unwrap().generation,
            snapshot.status.generation
        );
        assert_eq!(writer.status().unwrap().symbols, snapshot.status.symbols);

        let selected = map
            .snapshot(&group, Some(&first_id), "main.ts", None, None, false)
            .unwrap();
        assert!(selected
            .result
            .unwrap()
            .files
            .iter()
            .any(|file| file.path == "main.ts"));
        fs::write(first.join("main.ts"), "function changed() {}").unwrap();
        let updated = map
            .snapshot(
                &group,
                Some(&first_id),
                "changed",
                Some("main.ts"),
                Some(1),
                false,
            )
            .unwrap();
        assert!(updated.status.generation > snapshot.status.generation);
        assert!(updated.overview.edges.is_empty());
        assert!(updated.result.unwrap().files[0]
            .symbols
            .iter()
            .any(|symbol| symbol.snippet.contains("changed")));
        assert_eq!(
            writer.status().unwrap().generation,
            updated.status.generation
        );
        fs::rename(first.join("leaf.ts"), first.join("renamed.ts")).unwrap();
        let forced = map
            .snapshot(&group, Some(&first_id), "", None, None, true)
            .unwrap();
        assert!(forced
            .overview
            .files
            .iter()
            .any(|file| file.path == "renamed.ts"));
        assert!(!forced
            .overview
            .files
            .iter()
            .any(|file| file.path == "leaf.ts"));

        let other = map
            .snapshot(&group, Some(&second_id), "cactus", None, None, false)
            .unwrap();
        assert_eq!(other.status.files, 1);
        assert!(other
            .result
            .unwrap()
            .files
            .iter()
            .flat_map(|file| &file.symbols)
            .any(|symbol| symbol.name == "cactus"));
        assert_eq!(writer.status().unwrap().files, 2);
        let alias = temp.join("alias");
        std::os::unix::fs::symlink(&first, &alias).unwrap();
        let aliased = map
            .snapshot(&alias, Some(&first_id), "", None, None, false)
            .unwrap();
        assert_eq!(aliased.root_id, first_id);
        assert_eq!(aliased.roots[0].relative, "");
        let group_alias = temp.join("group-alias");
        std::os::unix::fs::symlink(&group, &group_alias).unwrap();
        let grouped_alias = map
            .snapshot(&group_alias, Some(&first_id), "", None, None, false)
            .unwrap();
        assert_eq!(grouped_alias.roots[0].relative, "first");
        assert_eq!(grouped_alias.roots[1].relative, "second");

        for (root, query, path, line) in [
            (Some(second_id.as_str()), "", None, None),
            (Some("/outside"), "", None, None),
            (None, "changed", Some("../main.ts"), None),
            (None, "changed", Some("/main.ts"), None),
            (None, "changed", None, Some(1)),
            (None, "changed", Some("main.ts"), Some(0)),
            (None, "", Some("main.ts"), None),
        ] {
            assert!(map
                .snapshot(&first, root, query, path, line, false)
                .is_err());
        }
        assert!(map
            .snapshot(&first, None, &"x".repeat(1025), None, None, false)
            .is_err());
        drop(writer);
        drop(map);
        fs::remove_dir_all(temp).unwrap();
    }
}
