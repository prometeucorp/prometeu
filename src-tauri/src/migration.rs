//! Importa uma instalação do Prometheus sem transformar as duas identidades
//! em uma só.
//!
//! É um caminho deliberadamente estreito: a origem é a instalação release
//! (`~/.prometheus`), o destino precisa estar sem projetos e workspaces, e os
//! worktrees continuam onde já estão. O que copiamos é estado durável; tokens,
//! caches e processos temporários ficam de fora.

use crate::lock::lock;
use crate::state::{publish, Board, ProviderId};
use crate::{i18n, paths, plugins, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

const VERSION: u8 = 1;
const MANIFEST: &str = "imports/prometheus-v1.json";

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LegacyImportState {
    Ready,
    Missing,
    Imported,
    TargetNotEmpty,
    Invalid,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportCounts {
    pub projects: usize,
    pub workspaces: usize,
    pub active_workspaces: usize,
    pub archived_workspaces: usize,
    pub tabs: usize,
    pub transcripts: usize,
    pub missing_transcripts: usize,
    pub codex_files: usize,
    pub plugins: usize,
    pub settings: usize,
    pub worktrees: usize,
    pub existing_worktrees: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LegacyImportPlan {
    pub state: LegacyImportState,
    pub source: String,
    pub counts: ImportCounts,
    pub problem: Option<String>,
    pub imported_at: Option<u64>,
    pub backup: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum ManifestStatus {
    Prepared,
    Complete,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ImportManifest {
    version: u8,
    status: ManifestStatus,
    imported_at: u64,
    source: String,
    source_board_sha256: String,
    backup: String,
    #[serde(default)]
    project_ids: Vec<String>,
    workspace_ids: Vec<String>,
    counts: ImportCounts,
}

impl ImportManifest {
    fn plan(&self) -> LegacyImportPlan {
        LegacyImportPlan {
            state: LegacyImportState::Imported,
            source: self.source.clone(),
            counts: self.counts.clone(),
            problem: None,
            imported_at: Some(self.imported_at),
            backup: Some(self.backup.clone()),
        }
    }

    fn applied_to(&self, board: &Board) -> bool {
        self.status == ManifestStatus::Complete
            || ((!self.project_ids.is_empty() || !self.workspace_ids.is_empty())
                && self
                    .project_ids
                    .iter()
                    .all(|id| board.projects.iter().any(|project| &project.id == id))
                && self
                    .workspace_ids
                    .iter()
                    .all(|id| board.workspaces.iter().any(|workspace| &workspace.id == id)))
    }
}

#[derive(Clone)]
struct Roots {
    source: PathBuf,
    target: PathBuf,
    home: PathBuf,
}

impl Roots {
    fn system() -> Self {
        let home = paths::home();
        Self {
            source: home.join(".prometheus"),
            target: paths::root(),
            home,
        }
    }
}

struct LegacyBoard {
    board: Board,
    raw: Vec<u8>,
    selected: PathBuf,
    hash: String,
}

struct FileCopy {
    source: PathBuf,
    target: PathBuf,
}

struct Prepared {
    legacy: LegacyBoard,
    plan: LegacyImportPlan,
    chats: Vec<FileCopy>,
    plugins: Vec<plugins::Plugin>,
    plugin_trees: Vec<FileCopy>,
    settings: Vec<FileCopy>,
}

#[tauri::command(async)]
pub fn legacy_import_plan(state: State<AppState>) -> LegacyImportPlan {
    let board = lock(&state.board).clone();
    plan_for(&Roots::system(), &board)
}

#[tauri::command(async)]
pub fn legacy_import_run(
    app: AppHandle,
    state: State<AppState>,
) -> Result<LegacyImportPlan, String> {
    if prometheus_running() {
        return Err(i18n::t("err.import.running"));
    }
    let roots = Roots::system();
    let result = {
        // Destino vazio significa que não há agente para bloquear. Segurar o
        // quadro impede uma criação concorrente de atravessar a verificação
        // e nascer embaixo do importador.
        let mut board = lock(&state.board);
        import_for(&roots, &mut board)?
    };
    publish(&app);
    Ok(result)
}

#[cfg(target_os = "macos")]
fn prometheus_running() -> bool {
    std::process::Command::new("/usr/bin/pgrep")
        .args(["-x", "Prometheus"])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(target_os = "macos"))]
fn prometheus_running() -> bool {
    false
}

fn empty(board: &Board) -> bool {
    board.projects.is_empty() && board.workspaces.is_empty()
}

fn base_plan(roots: &Roots, state: LegacyImportState) -> LegacyImportPlan {
    LegacyImportPlan {
        state,
        source: roots.source.display().to_string(),
        counts: ImportCounts::default(),
        problem: None,
        imported_at: None,
        backup: None,
    }
}

fn read_manifest(roots: &Roots) -> Option<ImportManifest> {
    fs::read(roots.target.join(MANIFEST))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .filter(|manifest: &ImportManifest| manifest.version == VERSION)
}

fn plan_for(roots: &Roots, target: &Board) -> LegacyImportPlan {
    let manifest = read_manifest(roots);
    if let Some(done) = manifest.as_ref().filter(|done| done.applied_to(target)) {
        return done.plan();
    }

    let mut prepared = match prepare(roots) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return base_plan(roots, LegacyImportState::Missing),
        Err(problem) => {
            let mut plan = base_plan(roots, LegacyImportState::Invalid);
            plan.problem = Some(problem);
            return plan;
        }
    };

    if !empty(target) {
        prepared.plan.state = LegacyImportState::TargetNotEmpty;
        return prepared.plan;
    }
    if let Err(problem) = validate_target(roots, &prepared) {
        prepared.plan.state = LegacyImportState::Invalid;
        prepared.plan.problem = Some(problem);
        return prepared.plan;
    }
    prepared.plan
}

fn import_for(roots: &Roots, target: &mut Board) -> Result<LegacyImportPlan, String> {
    if let Some(done) = read_manifest(roots).filter(|done| done.applied_to(target)) {
        return Ok(done.plan());
    }
    if !empty(target) {
        return Err(i18n::t("err.import.targetNotEmpty"));
    }

    let prepared = prepare(roots)?.ok_or_else(|| i18n::t("err.import.missing"))?;
    validate_target(roots, &prepared)?;
    let imported_at = now();
    let backup = backup(roots, &prepared, imported_at)?;
    let mut changes = Changes::capture(roots);

    let apply = (|| -> Result<ImportManifest, String> {
        for copy in &prepared.chats {
            if copy_file_new(&copy.source, &copy.target, 0o600, true)? {
                changes.files.push(copy.target.clone());
            }
        }
        for copy in &prepared.plugin_trees {
            if copy_tree_new(&copy.source, &copy.target)? {
                changes.trees.push(copy.target.clone());
            }
        }
        for copy in &prepared.settings {
            if present(&copy.target)? {
                continue;
            }
            let raw = fs::read_to_string(&copy.source)
                .map_err(|error| read_error(&copy.source, error))?;
            // Este é só o prefixo das variáveis públicas que o script recebe;
            // nomes livres e caminhos que contenham "Prometheus" ficam como
            // estavam.
            let converted = raw.replace("PROMETHEUS_", "PROMETEU_");
            toml::from_str::<toml::Value>(&converted).map_err(|error| {
                i18n::ta(
                    "err.import.settings",
                    &[
                        ("path", copy.source.display().to_string()),
                        ("cause", error.to_string()),
                    ],
                )
            })?;
            if write_new(
                &copy.target,
                converted.as_bytes(),
                source_mode(&copy.source),
                false,
            )? {
                changes.files.push(copy.target.clone());
            }
        }

        // Relê imediatamente antes de gravar: se algum cadastro independente
        // apareceu desde a prévia, ele entra no merge em vez de ser apagado.
        let current_plugins = target_plugins(roots)?;
        let merged_plugins = merge_plugins(current_plugins.clone(), &prepared.plugins)?;
        if merged_plugins != current_plugins {
            changes.plugins = fs::read(roots.target.join("plugins.json")).ok();
            changes.plugins_touched = true;
            let raw = serde_json::to_vec_pretty(&merged_plugins)
                .map_err(|error| i18n::io(error.to_string()))?;
            write_private(&roots.target.join("plugins.json"), &raw)?;
        }

        // Se o Prometheus foi aberto depois da prévia, não ativamos um
        // snapshot cujo quadro mudou no meio da cópia.
        let current = fs::read(&prepared.legacy.selected)
            .map_err(|error| read_error(&prepared.legacy.selected, error))?;
        if digest(&current) != prepared.legacy.hash {
            return Err(i18n::t("err.import.changed"));
        }

        let manifest = ImportManifest {
            version: VERSION,
            status: ManifestStatus::Prepared,
            imported_at,
            source: roots.source.display().to_string(),
            source_board_sha256: prepared.legacy.hash.clone(),
            backup: backup.display().to_string(),
            project_ids: prepared
                .legacy
                .board
                .projects
                .iter()
                .map(|project| project.id.clone())
                .collect(),
            workspace_ids: prepared
                .legacy
                .board
                .workspaces
                .iter()
                .map(|workspace| workspace.id.clone())
                .collect(),
            counts: prepared.plan.counts.clone(),
        };
        changes.manifest_touched = true;
        write_manifest(roots, &manifest)?;
        write_board(roots, &prepared.legacy.board)?;
        Ok(manifest)
    })();

    let mut manifest = match apply {
        Ok(manifest) => manifest,
        Err(error) => {
            changes.rollback(roots);
            return Err(error);
        }
    };

    *target = prepared.legacy.board;
    manifest.status = ManifestStatus::Complete;
    // O quadro já está íntegro e ativo. Se só a troca de "prepared" por
    // "complete" falhar, o próximo plano reconhece os mesmos ids e continua
    // dizendo que a importação terminou.
    if let Err(error) = write_manifest(roots, &manifest) {
        eprintln!("não finalizei o manifesto da importação: {error}");
    }
    Ok(manifest.plan())
}

fn prepare(roots: &Roots) -> Result<Option<Prepared>, String> {
    let Some(mut legacy) = legacy_board(roots)? else {
        return Ok(None);
    };
    // Sem a credencial do time, conservar esta marca faria workspaces serem
    // anunciados sem que a pessoa tivesse escolhido compartilhar no Prometeu.
    for workspace in &mut legacy.board.workspaces {
        workspace.shared = false;
        workspace.audience = None;
    }

    let chats = chat_copies(roots)?;
    let source_plugins = legacy_plugins(roots)?;
    let (plugins, plugin_trees) = imported_plugins(roots, source_plugins)?;
    let settings = settings_copies(&legacy.board)?;

    let mut transcripts = 0;
    let mut missing_transcripts = 0;
    for workspace in &legacy.board.workspaces {
        for tab in &workspace.tabs {
            let provider = tab
                .choice
                .as_ref()
                .map(|choice| choice.agent)
                .unwrap_or(workspace.agent);
            let path = match provider {
                ProviderId::Claude => {
                    paths::transcript_at(&roots.home, &tab.id, Path::new(&workspace.worktree))
                }
                ProviderId::Codex => roots.source.join("chats").join(format!("{}.jsonl", tab.id)),
            };
            if path.is_file() {
                transcripts += 1;
            } else {
                missing_transcripts += 1;
            }
        }
    }

    let worktrees: BTreeSet<String> = legacy
        .board
        .workspaces
        .iter()
        .filter(|workspace| workspace.worktree != workspace.repo && !workspace.cleaned)
        .map(|workspace| workspace.worktree.clone())
        .collect();
    let active_workspaces = legacy
        .board
        .workspaces
        .iter()
        .filter(|workspace| !workspace.archived)
        .count();
    let tabs = legacy
        .board
        .workspaces
        .iter()
        .map(|workspace| workspace.tabs.len())
        .sum();
    let counts = ImportCounts {
        projects: legacy.board.projects.len(),
        workspaces: legacy.board.workspaces.len(),
        active_workspaces,
        archived_workspaces: legacy.board.workspaces.len() - active_workspaces,
        tabs,
        transcripts,
        missing_transcripts,
        codex_files: chats.len(),
        plugins: plugins.len(),
        settings: settings.len(),
        worktrees: worktrees.len(),
        existing_worktrees: worktrees
            .iter()
            .filter(|path| Path::new(path).exists())
            .count(),
    };
    let plan = LegacyImportPlan {
        state: LegacyImportState::Ready,
        source: roots.source.display().to_string(),
        counts,
        problem: None,
        imported_at: None,
        backup: None,
    };
    Ok(Some(Prepared {
        legacy,
        plan,
        chats,
        plugins,
        plugin_trees,
        settings,
    }))
}

fn legacy_board(roots: &Roots) -> Result<Option<LegacyBoard>, String> {
    let current = roots.source.join("board.json");
    let backup = roots.source.join("board.json.bak");
    let mut errors = Vec::new();
    for path in [&current, &backup] {
        if !path.exists() {
            continue;
        }
        let raw = match fs::read(path) {
            Ok(raw) => raw,
            Err(error) => {
                errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        match serde_json::from_slice::<Board>(&raw) {
            Ok(mut board) => {
                board.revive();
                return Ok(Some(LegacyBoard {
                    board,
                    hash: digest(&raw),
                    raw,
                    selected: path.clone(),
                }));
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    if errors.is_empty() {
        Ok(None)
    } else {
        Err(i18n::ta(
            "err.import.board",
            &[("cause", errors.join("; "))],
        ))
    }
}

fn chat_copies(roots: &Roots) -> Result<Vec<FileCopy>, String> {
    let source = roots.source.join("chats");
    let entries = match fs::read_dir(&source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(read_error(&source, error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| read_error(&source, error))?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| read_error(&path, error))?
            .is_file()
            && path.extension().and_then(|part| part.to_str()) == Some("jsonl")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|source| FileCopy {
            target: roots.target.join("chats").join(source.file_name().unwrap()),
            source,
        })
        .collect())
}

fn legacy_plugins(roots: &Roots) -> Result<Vec<plugins::Plugin>, String> {
    let path = roots.source.join("plugins.json");
    match fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw)
            .map_err(|error| i18n::ta("err.import.plugins", &[("cause", error.to_string())])),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(read_error(&path, error)),
    }
}

fn imported_plugins(
    roots: &Roots,
    source: Vec<plugins::Plugin>,
) -> Result<(Vec<plugins::Plugin>, Vec<FileCopy>), String> {
    let old_store = roots.source.join("plugins");
    let new_store = roots.target.join("plugins");
    let mut trees = BTreeMap::<PathBuf, PathBuf>::new();
    let mut imported = Vec::new();
    for mut plugin in source {
        if plugin.id.trim().is_empty() {
            return Err(i18n::t("err.import.pluginName"));
        }
        let expanded = expand(&plugin.source, &roots.home);
        if plugin.made && expanded.starts_with(&old_store) && expanded != old_store {
            let relative = expanded.strip_prefix(&old_store).unwrap();
            if relative
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(i18n::ta(
                    "err.import.conflict",
                    &[("path", expanded.display().to_string())],
                ));
            }
            let top = relative.components().next().unwrap().as_os_str();
            let source_top = old_store.join(top);
            let source_kind = fs::symlink_metadata(&source_top)
                .map_err(|error| read_error(&source_top, error))?;
            // A raiz gerenciada precisa ser a pasta real que o Prometheus
            // criou. Symlinks internos continuam sendo preservados, mas uma
            // raiz redirecionada não vira propriedade do novo aplicativo.
            if !source_kind.is_dir() {
                return Err(i18n::ta(
                    "err.import.missingPath",
                    &[("path", source_top.display().to_string())],
                ));
            }
            trees.insert(source_top, new_store.join(top));
            plugin.source = new_store.join(relative).display().to_string();
        }
        imported.push(plugin);
    }
    Ok((
        imported,
        trees
            .into_iter()
            .map(|(source, target)| FileCopy { source, target })
            .collect(),
    ))
}

fn settings_copies(board: &Board) -> Result<Vec<FileCopy>, String> {
    let mut repos = BTreeSet::new();
    for project in &board.projects {
        repos.insert(project.path.clone());
    }
    for workspace in &board.workspaces {
        repos.insert(workspace.repo.clone());
        for repo in &workspace.repos {
            repos.insert(repo.path.clone());
        }
    }
    let mut copies = Vec::new();
    for repo in repos {
        let repo = PathBuf::from(repo);
        if !repo.join(".git").exists() {
            continue;
        }
        let source = repo.join(".prometheus/settings.toml");
        let target = repo.join(".prometeu/settings.toml");
        if !source.is_file() || present(&target)? {
            continue;
        }
        let parent = target.parent().unwrap();
        if fs::symlink_metadata(parent).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(i18n::ta(
                "err.import.conflict",
                &[("path", parent.display().to_string())],
            ));
        }
        copies.push(FileCopy { source, target });
    }
    Ok(copies)
}

fn target_plugins(roots: &Roots) -> Result<Vec<plugins::Plugin>, String> {
    let path = roots.target.join("plugins.json");
    match fs::read(&path) {
        Ok(raw) => serde_json::from_slice(&raw)
            .map_err(|error| i18n::ta("err.import.plugins", &[("cause", error.to_string())])),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(read_error(&path, error)),
    }
}

fn validate_target(roots: &Roots, prepared: &Prepared) -> Result<Vec<plugins::Plugin>, String> {
    for copy in &prepared.chats {
        if present(&copy.target)? && !same_file(&copy.source, &copy.target)? {
            return Err(conflict(&copy.target));
        }
    }
    for copy in &prepared.plugin_trees {
        if present(&copy.target)? && !same_tree(&copy.source, &copy.target)? {
            return Err(conflict(&copy.target));
        }
    }

    merge_plugins(target_plugins(roots)?, &prepared.plugins)
}

fn merge_plugins(
    mut current: Vec<plugins::Plugin>,
    imported: &[plugins::Plugin],
) -> Result<Vec<plugins::Plugin>, String> {
    for plugin in imported {
        match current.iter().find(|candidate| candidate.id == plugin.id) {
            Some(candidate) if candidate == plugin => {}
            Some(_) => {
                return Err(i18n::ta(
                    "err.import.pluginConflict",
                    &[("name", plugin.id.clone())],
                ))
            }
            None => current.push(plugin.clone()),
        }
    }
    current.sort_by_key(|plugin| plugin.id.to_lowercase());
    Ok(current)
}

fn backup(roots: &Roots, prepared: &Prepared, at: u64) -> Result<PathBuf, String> {
    let short = uuid::Uuid::new_v4().simple().to_string();
    let dir = roots
        .target
        .join("imports")
        .join(format!("prometheus-{at}-{}", &short[..8]));
    paths::ensure_private_dir(&dir).map_err(i18n::io)?;
    let source = dir.join("source");
    paths::ensure_private_dir(&source).map_err(i18n::io)?;
    write_private(&source.join("board.json"), &prepared.legacy.raw)?;
    if let Ok(raw) = fs::read(roots.source.join("plugins.json")) {
        write_private(&source.join("plugins.json"), &raw)?;
    }

    let before = dir.join("before");
    for name in ["board.json", "board.json.bak", "plugins.json"] {
        let path = roots.target.join(name);
        if let Ok(raw) = fs::read(&path) {
            write_private(&before.join(name), &raw)?;
        }
    }
    Ok(dir)
}

fn write_board(roots: &Roots, board: &Board) -> Result<(), String> {
    let path = roots.target.join("board.json");
    let raw = serde_json::to_vec_pretty(board).map_err(|error| i18n::io(error.to_string()))?;
    if let Ok(previous) = fs::read(&path) {
        if serde_json::from_slice::<Board>(&previous).is_ok() {
            write_private(&path.with_extension("json.bak"), &previous)?;
        }
    }
    write_private(&path, &raw)
}

fn write_manifest(roots: &Roots, manifest: &ImportManifest) -> Result<(), String> {
    let raw = serde_json::to_vec_pretty(manifest).map_err(|error| i18n::io(error.to_string()))?;
    write_private(&roots.target.join(MANIFEST), &raw)
}

fn write_private(path: &Path, raw: &[u8]) -> Result<(), String> {
    paths::write_private_bytes(path, raw).map_err(|cause| {
        i18n::ta(
            "err.import.write",
            &[("path", path.display().to_string()), ("cause", cause)],
        )
    })
}

fn copy_file_new(source: &Path, target: &Path, mode: u32, private: bool) -> Result<bool, String> {
    if present(target)? {
        return if same_file(source, target)? {
            Ok(false)
        } else {
            Err(conflict(target))
        };
    }
    let raw = fs::read(source).map_err(|error| read_error(source, error))?;
    write_new(target, &raw, mode, private)
}

/// Cria sem jamais passar por cima de um arquivo que apareceu no meio da
/// operação. O hard link publica o temporário de forma atômica e falha se o
/// nome final já existir.
fn write_new(target: &Path, raw: &[u8], mode: u32, private: bool) -> Result<bool, String> {
    if present(target)? {
        return Err(conflict(target));
    }
    let parent = target.parent().ok_or_else(|| {
        i18n::ta(
            "err.import.conflict",
            &[("path", target.display().to_string())],
        )
    })?;
    if private {
        paths::ensure_private_dir(parent).map_err(i18n::io)?;
    } else {
        fs::create_dir_all(parent).map_err(|error| write_error(parent, error))?;
    }
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("import");
    let temporary = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| write_error(&temporary, error))?;
    if let Err(error) = file.write_all(raw).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(write_error(&temporary, error));
    }
    drop(file);
    if let Err(error) = fs::hard_link(&temporary, target) {
        let _ = fs::remove_file(&temporary);
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(conflict(target));
        }
        return Err(write_error(target, error));
    }
    let _ = fs::remove_file(&temporary);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(target, fs::Permissions::from_mode(mode)) {
            let _ = fs::remove_file(target);
            return Err(write_error(target, error));
        }
    }
    sync_parent(target);
    Ok(true)
}

fn copy_tree_new(source: &Path, target: &Path) -> Result<bool, String> {
    if present(target)? {
        return if same_tree(source, target)? {
            Ok(false)
        } else {
            Err(conflict(target))
        };
    }
    let parent = target.parent().ok_or_else(|| conflict(target))?;
    paths::ensure_private_dir(parent).map_err(i18n::io)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("plugin");
    let temporary = parent.join(format!(".{name}.{}.import", uuid::Uuid::new_v4()));
    if let Err(error) = copy_tree(source, &temporary) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if let Err(error) = rename_new(&temporary, target) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    sync_parent(target);
    Ok(true)
}

/// Publica a árvore inteira sem a semântica destrutiva de `rename`: no macOS,
/// `RENAME_EXCL` faz a troca falhar se qualquer entrada apareceu no destino.
#[cfg(target_os = "macos")]
fn rename_new(source: &Path, target: &Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes()).map_err(|_| conflict(source))?;
    let target_c = CString::new(target.as_os_str().as_bytes()).map_err(|_| conflict(target))?;
    // SAFETY: os dois ponteiros vêm de `CString`, permanecem vivos durante a
    // chamada e `renamex_np` não retém nenhum deles.
    let result = unsafe { libc::renamex_np(source.as_ptr(), target_c.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        Err(conflict(target))
    } else {
        Err(write_error(target, error))
    }
}

/// O produto é macOS; este caminho mantém testes e builds auxiliares de outras
/// plataformas funcionais. A checagem continua conservadora, embora só o macOS
/// ofereça aqui a publicação exclusiva em uma chamada.
#[cfg(not(target_os = "macos"))]
fn rename_new(source: &Path, target: &Path) -> Result<(), String> {
    if present(target)? {
        return Err(conflict(target));
    }
    fs::rename(source, target).map_err(|error| write_error(target, error))
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| read_error(source, error))?;
    if metadata.is_dir() {
        fs::create_dir(target).map_err(|error| write_error(target, error))?;
        for entry in fs::read_dir(source).map_err(|error| read_error(source, error))? {
            let entry = entry.map_err(|error| read_error(source, error))?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions())
            .map_err(|error| write_error(target, error))?;
        return Ok(());
    }
    if metadata.is_file() {
        fs::copy(source, target).map_err(|error| write_error(target, error))?;
        fs::File::open(target)
            .and_then(|file| file.sync_all())
            .map_err(|error| write_error(target, error))?;
        return Ok(());
    }
    if metadata.file_type().is_symlink() {
        let link = fs::read_link(source).map_err(|error| read_error(source, error))?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(link, target).map_err(|error| write_error(target, error))?;
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            let _ = link;
            return Err(i18n::ta(
                "err.import.conflict",
                &[("path", source.display().to_string())],
            ));
        }
    }
    Err(i18n::ta(
        "err.import.conflict",
        &[("path", source.display().to_string())],
    ))
}

fn same_file(left: &Path, right: &Path) -> Result<bool, String> {
    let left = fs::read(left).map_err(|error| read_error(left, error))?;
    let right = fs::read(right).map_err(|error| read_error(right, error))?;
    Ok(digest(&left) == digest(&right))
}

fn same_tree(left: &Path, right: &Path) -> Result<bool, String> {
    Ok(tree_digest(left)? == tree_digest(right)?)
}

/// `Path::exists` segue symlinks e chama um link quebrado de ausente. Para o
/// importador, qualquer entrada no nome final já pertence ao destino e bloqueia
/// uma escrita destrutiva.
fn present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(read_error(path, error)),
    }
}

fn tree_digest(root: &Path) -> Result<String, String> {
    fn visit(root: &Path, at: &Path, hash: &mut Sha256) -> Result<(), String> {
        let metadata = fs::symlink_metadata(at).map_err(|error| read_error(at, error))?;
        let relative = at.strip_prefix(root).unwrap_or(at).to_string_lossy();
        hash.update(relative.as_bytes());
        if metadata.is_dir() {
            hash.update(b"d\0");
            let mut entries: Vec<_> = fs::read_dir(at)
                .map_err(|error| read_error(at, error))?
                .collect::<Result<_, _>>()
                .map_err(|error| read_error(at, error))?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                visit(root, &entry.path(), hash)?;
            }
        } else if metadata.is_file() {
            hash.update(b"f\0");
            hash.update(fs::read(at).map_err(|error| read_error(at, error))?);
        } else if metadata.file_type().is_symlink() {
            hash.update(b"l\0");
            hash.update(
                fs::read_link(at)
                    .map_err(|error| read_error(at, error))?
                    .to_string_lossy()
                    .as_bytes(),
            );
        } else {
            return Err(conflict(at));
        }
        Ok(())
    }

    let mut hash = Sha256::new();
    visit(root, root, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

struct Changes {
    files: Vec<PathBuf>,
    trees: Vec<PathBuf>,
    board: Option<Vec<u8>>,
    board_backup: Option<Vec<u8>>,
    plugins: Option<Vec<u8>>,
    manifest: Option<Vec<u8>>,
    plugins_touched: bool,
    manifest_touched: bool,
}

impl Changes {
    fn capture(roots: &Roots) -> Self {
        Self {
            files: Vec::new(),
            trees: Vec::new(),
            board: fs::read(roots.target.join("board.json")).ok(),
            board_backup: fs::read(roots.target.join("board.json.bak")).ok(),
            plugins: fs::read(roots.target.join("plugins.json")).ok(),
            manifest: fs::read(roots.target.join(MANIFEST)).ok(),
            plugins_touched: false,
            manifest_touched: false,
        }
    }

    fn rollback(&self, roots: &Roots) {
        for file in self.files.iter().rev() {
            let _ = fs::remove_file(file);
        }
        for tree in self.trees.iter().rev() {
            let store = roots.target.join("plugins");
            if tree.starts_with(&store) && tree != &store {
                let _ = fs::remove_dir_all(tree);
            }
        }
        if self.plugins_touched {
            restore(&roots.target.join("plugins.json"), self.plugins.as_deref());
        }
        if self.manifest_touched {
            restore(&roots.target.join(MANIFEST), self.manifest.as_deref());
        }
        // `write_board` pode ter renomeado o arquivo e falhado ao reafirmar a
        // permissão. Restaurar sempre fecha também esse caminho raro.
        restore(&roots.target.join("board.json"), self.board.as_deref());
        restore(
            &roots.target.join("board.json.bak"),
            self.board_backup.as_deref(),
        );
    }
}

fn restore(path: &Path, raw: Option<&[u8]>) {
    match raw {
        Some(raw) => {
            if let Err(error) = paths::write_private_bytes(path, raw) {
                eprintln!("não restaurei {}: {error}", path.display());
            }
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}

fn expand(path: &str, home: &Path) -> PathBuf {
    path.strip_prefix("~/")
        .map(|rest| home.join(rest))
        .unwrap_or_else(|| PathBuf::from(path))
}

fn digest(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(unix)]
fn source_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .unwrap_or(0o600)
}

#[cfg(not(unix))]
fn source_mode(_path: &Path) -> u32 {
    0o600
}

fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        let _ = fs::File::open(parent).and_then(|directory| directory.sync_all());
    }
}

fn conflict(path: &Path) -> String {
    i18n::ta(
        "err.import.conflict",
        &[("path", path.display().to_string())],
    )
}

fn read_error(path: &Path, error: impl ToString) -> String {
    i18n::ta(
        "err.import.read",
        &[
            ("path", path.display().to_string()),
            ("cause", error.to_string()),
        ],
    )
}

fn write_error(path: &Path, error: impl ToString) -> String {
    i18n::ta(
        "err.import.write",
        &[
            ("path", path.display().to_string()),
            ("cause", error.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        let home = std::env::temp_dir().join(format!("prometeu-import-{}", uuid::Uuid::new_v4()));
        Roots {
            source: home.join(".prometheus"),
            target: home.join(".prometeu"),
            home,
        }
    }

    fn fixture(roots: &Roots) -> (PathBuf, PathBuf) {
        let repo = roots.home.join("repo");
        let worktree = roots.home.join("prometheus/worktrees/repo/feat-x");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        fs::create_dir_all(&roots.source).unwrap();
        let board = serde_json::json!({
            "stages": ["Fazendo", "Feito"],
            "projects": [{"id": repo.display().to_string(), "name": "repo", "path": repo.display().to_string()}],
            "workspaces": [
                {
                    "id": "w-claude", "title": "Claude", "project": repo.display().to_string(),
                    "repo": repo.display().to_string(), "repo_name": "repo", "branch": "feat/x",
                    "worktree": worktree.display().to_string(), "stage": "Fazendo", "shared": true,
                    "tabs": [{"id":"t-claude","title":"conversa","status":"rodando","note":null,"pending_prompt":null}]
                },
                {
                    "id": "w-codex", "title": "Codex", "project": repo.display().to_string(),
                    "repo": repo.display().to_string(), "repo_name": "repo", "branch": "main",
                    "worktree": repo.display().to_string(), "stage": "Feito", "archived": true, "agent":"codex",
                    "tabs": [{"id":"t-codex","agent_session":"thread-1","title":"conversa","status":"pronta","note":null,"pending_prompt":null}]
                }
            ]
        });
        paths::write_private(
            &roots.source.join("board.json"),
            &serde_json::to_string_pretty(&board).unwrap(),
        )
        .unwrap();

        let claude = paths::transcript_at(&roots.home, "t-claude", &worktree);
        paths::write_private(&claude, "{\"type\":\"user\"}\n").unwrap();
        paths::write_private(
            &roots.source.join("chats/t-codex.jsonl"),
            "{\"v\":1,\"type\":\"message.user\"}\n",
        )
        .unwrap();
        // Um log sem card também é preservado: apagar card não deve apagar
        // silenciosamente o arquivo durante uma troca de produto.
        paths::write_private(&roots.source.join("chats/orfao.jsonl"), "{}\n").unwrap();

        let plugin = roots.source.join("plugins/caveman");
        fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        fs::write(plugin.join(".claude-plugin/plugin.json"), "{}").unwrap();
        fs::write(plugin.join("README.md"), "pedra").unwrap();
        let plugins = vec![plugins::Plugin {
            id: "caveman".into(),
            source: plugin.display().to_string(),
            note: "curto".into(),
            made: true,
            from: "https://example.test/caveman".into(),
        }];
        paths::write_private(
            &roots.source.join("plugins.json"),
            &serde_json::to_string_pretty(&plugins).unwrap(),
        )
        .unwrap();

        fs::create_dir_all(repo.join(".prometheus")).unwrap();
        fs::write(
            repo.join(".prometheus/settings.toml"),
            "[scripts]\nrun = \"serve --port $PROMETHEUS_PORT\"\n",
        )
        .unwrap();
        (repo, worktree)
    }

    #[test]
    fn previa_conta_o_que_e_duravel_sem_mover_worktree() {
        let roots = roots();
        let (_, worktree) = fixture(&roots);
        let plan = plan_for(&roots, &Board::default());

        assert_eq!(plan.state, LegacyImportState::Ready);
        assert_eq!(plan.counts.projects, 1);
        assert_eq!(plan.counts.workspaces, 2);
        assert_eq!(plan.counts.active_workspaces, 1);
        assert_eq!(plan.counts.archived_workspaces, 1);
        assert_eq!(plan.counts.tabs, 2);
        assert_eq!(plan.counts.transcripts, 2);
        assert_eq!(plan.counts.missing_transcripts, 0);
        assert_eq!(plan.counts.codex_files, 2);
        assert_eq!(plan.counts.plugins, 1);
        assert_eq!(plan.counts.settings, 1);
        assert_eq!(plan.counts.worktrees, 1);
        assert_eq!(plan.counts.existing_worktrees, 1);
        assert!(worktree.exists());
        fs::remove_dir_all(&roots.home).unwrap();
    }

    #[test]
    fn importa_copia_e_normaliza_sem_tocar_a_origem() {
        let roots = roots();
        let (repo, worktree) = fixture(&roots);
        let source_before = fs::read(roots.source.join("board.json")).unwrap();
        let mut target = Board::default();

        let result = import_for(&roots, &mut target).unwrap();

        assert_eq!(result.state, LegacyImportState::Imported);
        assert_eq!(
            fs::read(roots.source.join("board.json")).unwrap(),
            source_before
        );
        assert_eq!(target.workspaces.len(), 2);
        assert_eq!(
            target.workspaces[0].worktree,
            worktree.display().to_string()
        );
        assert!(!target.workspaces[0].shared);
        assert!(target.workspaces[0].audience.is_none());
        assert!(target.workspaces[0]
            .tabs
            .iter()
            .all(|tab| matches!(tab.status, crate::state::Status::Desligada)));
        assert_eq!(
            fs::read_to_string(roots.target.join("chats/t-codex.jsonl")).unwrap(),
            "{\"v\":1,\"type\":\"message.user\"}\n"
        );
        assert!(roots.target.join("chats/orfao.jsonl").exists());
        assert_eq!(
            fs::read_to_string(roots.target.join("plugins/caveman/README.md")).unwrap(),
            "pedra"
        );
        let imported_plugins: Vec<plugins::Plugin> =
            serde_json::from_slice(&fs::read(roots.target.join("plugins.json")).unwrap()).unwrap();
        assert_eq!(
            imported_plugins[0].source,
            roots.target.join("plugins/caveman").display().to_string()
        );
        assert_eq!(
            fs::read_to_string(repo.join(".prometeu/settings.toml")).unwrap(),
            "[scripts]\nrun = \"serve --port $PROMETEU_PORT\"\n"
        );
        assert!(roots.target.join(MANIFEST).exists());
        assert!(result
            .backup
            .as_ref()
            .is_some_and(|path| Path::new(path).exists()));

        // Rodar de novo reconhece o manifesto e não duplica nada.
        let again = import_for(&roots, &mut target).unwrap();
        assert_eq!(again.state, LegacyImportState::Imported);
        assert_eq!(target.workspaces.len(), 2);
        fs::remove_dir_all(&roots.home).unwrap();
    }

    #[test]
    fn destino_com_dados_recusa_sem_escrever() {
        let roots = roots();
        fixture(&roots);
        let mut target = Board {
            projects: vec![crate::state::Project {
                id: "novo".into(),
                name: "novo".into(),
                path: "/novo".into(),
            }],
            ..Board::default()
        };
        let before = serde_json::to_vec(&target).unwrap();

        assert!(import_for(&roots, &mut target).is_err());
        assert_eq!(serde_json::to_vec(&target).unwrap(), before);
        assert!(!roots.target.join("board.json").exists());
        fs::remove_dir_all(&roots.home).unwrap();
    }

    #[test]
    fn conflito_de_transcript_bloqueia_antes_de_qualquer_importacao() {
        let roots = roots();
        let (repo, _) = fixture(&roots);
        paths::write_private(&roots.target.join("chats/t-codex.jsonl"), "diferente\n").unwrap();
        let mut target = Board::default();

        let plan = plan_for(&roots, &target);
        assert_eq!(plan.state, LegacyImportState::Invalid);
        assert!(import_for(&roots, &mut target).is_err());
        assert!(!roots.target.join("board.json").exists());
        assert!(!repo.join(".prometeu/settings.toml").exists());
        fs::remove_dir_all(&roots.home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_no_destino_do_plugin_nunca_e_substituido() {
        use std::os::unix::fs::symlink;

        let roots = roots();
        let (repo, _) = fixture(&roots);
        fs::create_dir_all(roots.target.join("plugins")).unwrap();
        let target = roots.target.join("plugins/caveman");
        symlink(roots.home.join("nao-existe"), &target).unwrap();
        let mut board = Board::default();

        let plan = plan_for(&roots, &board);
        assert_eq!(plan.state, LegacyImportState::Invalid);
        assert!(import_for(&roots, &mut board).is_err());
        assert!(fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!roots.target.join("board.json").exists());
        assert!(!repo.join(".prometeu/settings.toml").exists());
        fs::remove_dir_all(&roots.home).unwrap();
    }
}
