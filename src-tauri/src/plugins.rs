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
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub use prometeu_tools::packages::Plugin;
pub(crate) use prometeu_tools::plugins::git_url;
#[cfg(test)]
use prometeu_tools::plugins::{guessed_name, last_line, within};
use prometeu_tools::plugins::{manifest_path, plugins_in, read_json, repo_name, trim};
fn library() -> prometeu_tools::plugins::PluginLibrary {
    prometeu_tools::plugins::PluginLibrary {
        root: paths::root(),
        home: paths::home(),
        catalog: std::sync::Arc::new(prometeu_tools::packages::FilePackageCatalog {
            path: hub_path(),
        }),
        files: std::sync::Arc::new(PrivatePackageFiles),
        runner: std::sync::Arc::new(prometeu_process::command::UnixCommandRunner),
    }
}

/// Keep the path-and-URL registry private alongside other application state, even though it
/// contains no credentials.
fn hub_path() -> PathBuf {
    paths::root().join("plugins.json")
}

pub fn load() -> Vec<Plugin> {
    prometeu_tools::packages::PackageCatalog::load(&prometeu_tools::packages::FilePackageCatalog {
        path: hub_path(),
    })
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

/// Save local installation, update, and test changes without publishing them.
pub(crate) fn save_local(plugin: Plugin) -> Result<Vec<Plugin>, String> {
    library().save_local(plugin)
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
    let removed = plugins.iter().any(|p| p.id == id);
    plugins = library().remove_local(plugins, id);
    if removed && slug(id) == id && !cfg!(test) {
        codex_remove_everywhere(&format!("{}@{}", id, codex_marketplace_name()));
    }
    plugins
}

/// Reject sources the CLI cannot load before starting a session that would silently omit the
/// selected plugin.
fn check_source(source: &str) -> Result<(), String> {
    library().check_source(source)
}

/// Read a directory's manifest to prefill its name and description. For ZIP paths or URLs, suggest
/// the filename without downloading it. Apply the same source validation used during saving.
#[tauri::command]
pub fn plugin_look(source: String) -> Result<Plugin, String> {
    library().look_source(source)
}

/// Generate flags for selected IDs still present in the hub, skipping deleted entries for
/// compatibility. None leaves CLI defaults intact.
#[cfg(test)]
fn args_from(hub: &[Plugin], chosen: &[String]) -> Vec<String> {
    prometeu_tools::packages::args_from(hub, chosen, &paths::home())
}
#[cfg(test)]
fn flags(plugin: &Plugin) -> [String; 2] {
    prometeu_tools::packages::flags(plugin, &paths::home())
}
pub(crate) use prometeu_tools::packages::remote;

pub(crate) fn expand(source: &str) -> String {
    prometeu_tools::packages::expand(source, &paths::home())
}

/* Native package composition */
#[cfg(test)]
use prometeu_tools::packages::PackageBackend;
#[cfg(test)]
use prometeu_tools::CodexPlugins;

fn codex_marketplace_name() -> &'static str {
    if cfg!(debug_assertions) {
        "prometeu-dev"
    } else {
        "prometeu"
    }
}
pub(crate) struct PrivatePackageFiles;
impl prometeu_tools::packages::PackageFiles for PrivatePackageFiles {
    fn ensure_private_dir(&self, path: &Path) -> Result<(), String> {
        paths::ensure_private_dir(path)
    }
    fn write_private(&self, path: &Path, body: &str) -> Result<(), String> {
        paths::write_private(path, body)
    }
}
fn installer() -> prometeu_tools::package_installer::CodexInstaller {
    prometeu_tools::package_installer::CodexInstaller {
        executable: "codex".into(),
        marketplace: codex_marketplace_name().into(),
    }
}
pub(crate) fn native_packages() -> prometeu_tools::packages::NativePackages {
    static PREPARE: OnceLock<std::sync::Arc<Mutex<()>>> = OnceLock::new();
    prometeu_tools::packages::NativePackages::new(
        paths::root().join("codex-workspaces"),
        paths::home(),
        codex_marketplace_name().into(),
        std::sync::Arc::new(prometeu_tools::packages::FilePackageCatalog { path: hub_path() }),
        std::sync::Arc::new(PrivatePackageFiles),
        std::sync::Arc::new(installer()),
        PREPARE
            .get_or_init(|| std::sync::Arc::new(Mutex::new(())))
            .clone(),
    )
}
pub fn forget_codex_workspace(workspace: &str) {
    native_packages().forget_codex_workspace(workspace);
}
#[cfg(test)]
fn codex_for(
    workspace: &str,
    chosen: Option<&[String]>,
    profile: &crate::accounts::Profile,
) -> Result<CodexPlugins, String> {
    native_packages().codex(workspace, chosen, profile)
}
fn codex_remove_everywhere(canonical: &str) {
    native_packages().codex_remove_everywhere(canonical);
}
#[cfg(test)]
fn codex_workspace_home(workspace: &str) -> PathBuf {
    native_packages().codex_workspace_home(workspace)
}
#[cfg(test)]
fn codex_command(home: &Path) -> Command {
    installer().command(home)
}
#[cfg(test)]
fn codex_installed(
    home: &Path,
) -> Result<HashMap<String, prometeu_tools::packages::InstalledPlugin>, String> {
    prometeu_tools::packages::PackageInstaller::installed(&installer(), home)
}
#[cfg(test)]
fn codex_remove(home: &Path, canonical: &str) {
    prometeu_tools::packages::PackageInstaller::remove(&installer(), home, canonical);
}

/* Installation */

/// Report the clone path and discovered plugins. Automatically register a single plugin; a
/// marketplace requires explicit selection before registration.
pub use prometeu_tools::plugins::Found;

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
    library().install_into(source, register)
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
        if !origin.success || String::from_utf8_lossy(&origin.stdout).trim() != git_url(source) {
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
    let _sync = crate::catalog::guard();
    library().scrap_directory(dir)
}

/// Update application-owned clones with git pull --ff-only, preserving manual edits on divergence.
/// Reread manifest descriptions after updates.
#[tauri::command(async)]
pub fn plugin_update(id: String) -> Result<Vec<Plugin>, String> {
    let _sync = crate::catalog::guard();
    library().update_plugin(id)
}

/// Check whether any hub plugin uses this clone directory.
fn lives_in(dir: &Path) -> bool {
    library().lives_in(dir)
}

fn git(root: &Path, args: &[&str]) -> Result<prometeu_core::command::CommandOutput, String> {
    library().git(root, args)
}

/// Find the clone root for marketplace plugins stored below it; updates must run where .git
/// belongs.
fn git_root(dir: &Path) -> Option<PathBuf> {
    library().git_root(dir)
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
    let (slug, place) = (slug, dir.clone());
    let mine = slug.clone();
    std::thread::spawn(move || {
        let end = make(&app, run, &place, &mine, &ask);
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
fn make(app: &AppHandle, run: u64, dir: &Path, slug: &str, ask: &str) -> Result<(), String> {
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
    crate::accounts::prepare_profile(&profile)?;
    crate::accounts::apply_profile(&profile, &mut cmd)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| i18n::ta("err.plugin.make", &[("cause", e.to_string())]))?;
    let out = child.stdout.take();
    lock(running()).insert(run, child);
    watch(run);
    if let Some(out) = out {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
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

use prometeu_tools::packages::slug;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_package_files_remain_private() {
        use prometeu_tools::packages::PackageFiles;
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("prometeu-package-files-{}", uuid::Uuid::new_v4()));
        let config = root.join("config.toml");
        PrivatePackageFiles.ensure_private_dir(&root).unwrap();
        PrivatePackageFiles
            .write_private(&config, "private configuration")
            .unwrap();
        assert_eq!(
            std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_to_string(config).unwrap(),
            "private configuration"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

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
