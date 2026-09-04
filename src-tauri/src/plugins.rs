//! O hub de plugins: quais plugins esta máquina conhece, e quais entram nas
//! conversas de cada workspace, no Claude Code e no Codex.
//!
//! Um plugin é um pacote de skill, comando, agente e — o que só ele faz —
//! *hook*: o pedaço de código que o CLI roda antes de cada fala, ao abrir a
//! sessão, depois de cada ferramenta. É o que segura um jeito de trabalhar
//! turno após turno, em vez de depender de a instrução antiga continuar
//! ganhando a atenção do modelo (o caveman é o exemplo: sem o hook de
//! `UserPromptSubmit` reinjetando a regra, ela se dissolve na conversa).
//!
//! Até aqui quem decidia isso era o CLI, e só ele: plugin de escopo `user`
//! entra em toda sessão, em todo workspace, sempre; plugin de escopo de
//! projeto nunca entra, porque o worktree que o Prometeu cria é um caminho
//! que o cadastro do CLI não conhece. Nenhum dos dois é o que se quer — o
//! plugin de revisão de front não tem o que fazer num workspace de Rails, e o
//! que o time combinou para um repositório tem que valer no worktree dele.
//!
//! O hub é a lista de plugins que o Prometeu guarda, e a escolha é do
//! workspace — como o modelo, o esforço e o MCP já são. O Claude recebe cada
//! escolhido diretamente por `--plugin-dir` ou `--plugin-url`; são flags de
//! sessão, sem alterar o cadastro do CLI. O Codex exige instalação no cache
//! próprio: o adapter monta um marketplace local a partir deste mesmo hub e
//! usa um `CODEX_HOME` derivado por workspace. Só o `config.toml` é isolado;
//! autenticação, sessões, skills e cache continuam apontando para o home real.
//! É o que mantém a escolha dentro do workspace sem reescrever a configuração
//! global da pessoa.
//!
//! `None` é workspace que nunca escolheu — todo quadro gravado antes disto
//! existir —, e aí nada é passado: vale o que cada CLI sempre fez.
//!
//! Instalar é daqui, e não de fora. `plugin_install` recebe o endereço de um
//! repositório — `github.com/JuliusBrussee/caveman`, ou só o
//! `JuliusBrussee/caveman` —, clona em `~/.prometeu/plugins/` e cadastra o
//! que veio dentro: o próprio repositório, quando ele é o plugin, ou os
//! plugins que o `marketplace.json` dele lista. Depois é `plugin_update`, que
//! é o `git pull` da mesma pasta. Ninguém precisa instalar nada no CLI antes —
//! era isso que fazia o plugin ser um assunto de fora do app.
//!
//! `plugin_make` é o outro caminho, para o plugin que ainda não existe: um
//! agente de uma pergunta só escreve o manifesto, as skills e os hooks numa
//! pasta do mesmo lugar.
//!
//! Cadastrar à mão continua existindo, para o plugin que alguém escreve num
//! repositório seu: aí a origem é a pasta dele, e quem a atualiza é quem a
//! escreve.
//!
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
use tauri::{AppHandle, Emitter};

/// Um plugin como o hub o guarda.
#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct Plugin {
    /// O nome do plugin — o mesmo que está no `plugin.json` dele, que é o que
    /// o CLI usa para deduplicar e o que a pessoa lê na lista.
    pub id: String,
    /// De onde ele sai: o caminho de uma pasta (ou de um `.zip`) nesta
    /// máquina, ou a URL de um `.zip`. É o que vira flag na linha de comando.
    pub source: String,
    /// De onde veio, ou para que serve. Livre — é a linha embaixo do nome.
    #[serde(default)]
    pub note: String,
    /// Se a pasta dele é do Prometeu — quer dizer, se o app a clonou ou a
    /// escreveu. É o que decide se remover apaga arquivo ou só tira da lista:
    /// pasta que alguém escreveu não é do app para apagar.
    #[serde(default)]
    pub made: bool,
    /// O endereço de onde ele veio, quando veio de um. É o que a lista mostra
    /// embaixo do nome e o que dá sentido ao botão de atualizar.
    #[serde(default)]
    pub from: String,
}

/// Onde o cadastro mora. Sem segredo dentro (é caminho e URL), mas fica
/// privado como o resto do `~/.prometeu`.
fn hub_path() -> PathBuf {
    paths::root().join("plugins.json")
}

pub fn load() -> Vec<Plugin> {
    std::fs::read_to_string(hub_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Vec<Plugin>>(&raw).ok())
        .unwrap_or_default()
}

fn write_hub(plugins: &[Plugin]) -> Result<(), String> {
    let body = serde_json::to_string_pretty(plugins).map_err(|e| e.to_string())?;
    paths::write_private(&hub_path(), &body)
        .map_err(|cause| i18n::ta("err.plugin.save", &[("cause", cause)]))
}

/// O cadastro inteiro, para a tela de Configurações e para os seletores.
#[tauri::command]
pub fn plugin_hub() -> Vec<Plugin> {
    load()
}

/// Grava um plugin — novo, ou por cima do que tinha o mesmo nome. O nome é a
/// identidade: é por ele que o CLI deduplica, e dois plugins com o mesmo nome
/// numa sessão seriam um só de qualquer jeito.
#[tauri::command]
pub fn plugin_save(plugin: Plugin) -> Result<Vec<Plugin>, String> {
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

/// Tira do cadastro — e apaga a pasta, se ela for a que o Prometeu criou:
/// ela só existe por causa deste cadastro, e deixá-la seria guardar no escuro
/// o que a tela já não mostra. Plugin cadastrado à mão só sai da lista; a
/// pasta é de quem a escreveu.
#[tauri::command]
pub fn plugin_remove(id: String) -> Result<Vec<Plugin>, String> {
    let mut plugins = load();
    let mut removed = false;
    if let Some(gone) = plugins.iter().find(|p| p.id == id) {
        removed = true;
        let dir = PathBuf::from(expand(&gone.source));
        if gone.made && dir.starts_with(store()) && dir != store() {
            std::fs::remove_dir_all(&dir).ok();
        }
    }
    plugins.retain(|p| p.id != id);
    write_hub(&plugins)?;
    if removed && slug(&id) == id && !cfg!(test) {
        codex_remove_everywhere(&format!("{}@{}", id, codex_marketplace_name()));
    }
    Ok(plugins)
}

/// O que uma origem tem que ser para o CLI aceitá-la. Recusar aqui é o que
/// evita a conversa subir sem o plugin e ninguém saber por quê: o
/// `--plugin-dir` de uma pasta que não é plugin some num aviso do CLI que a
/// tela não mostra.
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
    // Um `.zip` o CLI abre sozinho; uma pasta tem que ser um plugin, e o que
    // diz isso é o manifesto.
    if path.is_dir() && !manifest_path(&path).exists() {
        return Err(i18n::ta(
            "err.plugin.notPlugin",
            &[("path", path.display().to_string())],
        ));
    }
    Ok(())
}

/// O que o Prometeu consegue ler de uma origem antes de gravá-la: o nome e a
/// descrição que o próprio plugin declara. É o que preenche o formulário
/// sozinho — ninguém tem que copiar à mão um nome que já está escrito no
/// disco.
///
/// Origem que não é pasta (um `.zip` daqui, uma URL) não tem manifesto para
/// ler sem baixar: o nome sai do nome do arquivo, e a pessoa corrige se
/// quiser. Origem inválida devolve erro — é o mesmo exame do `plugin_save`,
/// só que antes.
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

/// O nome de um `.zip` (daqui ou da rede) sem a extensão. Palpite, e só: quem
/// manda é o `plugin.json` de dentro, que só o CLI vai ler.
fn guessed_name(source: &str) -> String {
    source
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".zip")
        .to_string()
}

/// As flags desta sessão: um par por plugin escolhido que o hub ainda tem.
/// Plugin apagado do hub depois de escolhido some da sessão em vez de
/// derrubá-la — a mesma regra do MCP, e pelo mesmo motivo.
///
/// `None` é workspace que nunca escolheu, e aí nada é passado.
pub fn args_for(chosen: Option<&Vec<String>>) -> Vec<String> {
    match chosen {
        Some(chosen) => args_from(&load(), chosen),
        None => vec![],
    }
}

/// A tradução em si, com o hub na mão — parâmetro, e não chamada direta, para
/// o teste dela não depender de arquivo nenhum.
fn args_from(hub: &[Plugin], chosen: &[String]) -> Vec<String> {
    chosen
        .iter()
        .filter_map(|name| hub.iter().find(|p| &p.id == name))
        .flat_map(flags)
        .collect()
}

/// A flag de um plugin. `--plugin-url` para o que está na rede,
/// `--plugin-dir` para o que está no disco — e o `~` vira caminho aqui, e não
/// no cadastro: quem digitou `~/plugins/x` quis dizer a casa desta máquina.
fn flags(plugin: &Plugin) -> [String; 2] {
    let source = plugin.source.trim();
    if remote(source) {
        ["--plugin-url".to_string(), source.to_string()]
    } else {
        ["--plugin-dir".to_string(), expand(source)]
    }
}

fn remote(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.join(".claude-plugin").join("plugin.json")
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn expand(source: &str) -> String {
    match source.strip_prefix("~/") {
        Some(rest) => paths::home().join(rest).display().to_string(),
        None => source.to_string(),
    }
}

/* ---------- adaptar o mesmo hub para o Codex ---------- */

/// O que o adapter entrega ao processo do Codex. `home` isola a camada de
/// configuração do workspace; `ids` deixa o handshake confiar apenas nos
/// hooks que a pessoa acabou de escolher. `hook_ids` distingue os pacotes que
/// declararam hooks: a thread não pode nascer se o Codex não os descobrir.
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

/// Namespace próprio: o cache do Codex é global, portanto uma instalação do
/// Prometeu nunca pode colidir com um marketplace que a pessoa cadastrou.
fn codex_marketplace_name() -> &'static str {
    if cfg!(debug_assertions) {
        "prometeu-dev"
    } else {
        "prometeu"
    }
}

/// Também entra no cachebuster. Mudança na adaptação do pacote precisa
/// reinstalar plugins já preparados mesmo quando a origem não mudou.
const CODEX_PACKAGE_REVISION: &str = "2";

fn codex_workspaces_root() -> PathBuf {
    paths::root().join("codex-workspaces")
}

fn codex_marketplace_root(home: &Path) -> PathBuf {
    home.join("marketplace")
}

/// O home original continua sendo a fonte de conta e estado. Quando veio por
/// ambiente, torná-lo absoluto é importante: o app-server muda o cwd para o
/// worktree e um `CODEX_HOME` relativo passaria a significar outra pasta.
fn user_codex_home() -> PathBuf {
    let configured = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".codex"));
    if configured.is_absolute() {
        configured
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(configured)
    }
}

/// O ID persistido, e não o cwd, é a identidade correta: dois workspaces sem
/// worktree podem usar o mesmo clone com seleções diferentes. O hash não
/// depende do UUID efêmero de uma conversa e não usa dado externo como nome de
/// pasta.
fn codex_workspace_home(workspace: &str) -> PathBuf {
    let fingerprint = format!("{:x}", Sha256::digest(workspace.as_bytes()));
    codex_workspaces_root().join(&fingerprint[..24])
}

/// A camada não é transcript nem trabalho do usuário. Quando o workspace sai
/// de vez, ela pode sair junto; os payloads instalados continuam no cache
/// compartilhado e qualquer workspace restante mantém sua própria config.
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

/// Traduz a seleção para um marketplace e um `CODEX_HOME` próprios do
/// workspace. O lock cobre materialização, config e cache compartilhado: duas
/// abas podem abrir juntas sem instalar a mesma versão pela metade.
pub fn codex_for(workspace: &str, chosen: Option<&Vec<String>>) -> Result<CodexPlugins, String> {
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
    let home = codex_workspace_home(workspace);
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
    let base = user_codex_home();
    prepare_codex_home(&base, &home, &marketplace, &ids)?;

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
    // `codex plugin add` escreve `enabled = true`. Refazer a camada derivada
    // depois das instalações restaura a seleção exata e preserva a confiança
    // de hooks que uma sessão anterior gravou neste mesmo workspace.
    write_codex_config(&base, &home, &marketplace, &ids)?;
    Ok(CodexPlugins {
        home: Some(home),
        ids,
        hook_ids,
    })
}

/// Monta um home que se comporta como o home real em tudo salvo a camada de
/// configuração. Links mantêm login, rollouts, skills e bancos no lugar que o
/// Codex já usa; `config.toml` e o marketplace são descartáveis e pertencem ao
/// Prometeu.
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

/// Espelha todas as entradas conhecidas e futuras do Codex, exceto os
/// arquivos que podem ser escritos pelo editor de configuração. Uma entrada
/// material que o próprio Codex já tenha criado nesse home é preservada; só um
/// link antigo para outro home pode ser trocado.
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

/// Reconstrói a config derivada a partir da config real mais o pequeno estado
/// próprio do workspace. O estado de hooks e opções do plugin sobrevive; toda
/// entrada Prometeu começa desligada e só a seleção atual é ligada.
fn write_codex_config(
    base: &Path,
    home: &Path,
    marketplace: &Path,
    selected: &[String],
) -> Result<(), String> {
    // Esta camada é cache. Uma versão do app-server que tenha deixado TOML
    // incompleto não pode tornar o workspace impossível de abrir: nesse caso
    // ela renasce da config real e os hooks pedem confiança outra vez.
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

    // O modo padrão já é `file`. Fixá-lo no home derivado também cobre `auto`:
    // um refresh precisa atravessar o symlink de auth.json, em vez de criar
    // uma credencial de keychain separada para cada workspace. Uma escolha
    // explícita por keyring/ephemeral continua sendo respeitada.
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

/// Faz uma cópia que o Codex pode versionar sem tocar no plugin do Claude. A
/// versão ganha o hash da origem: atualizar um repositório que esqueceu de
/// subir a própria versão ainda produz outra entrada de cache.
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

/// Manifesto que nomeia hooks assume que eles fazem parte do comportamento do
/// pacote, mesmo se o caminho estiver quebrado: nesse caso o handshake precisa
/// recusar a sessão, não reinterpretar o plugin como uma coleção de skills.
/// Sem campo explícito, vale a convenção nativa `hooks/hooks.json`.
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

/// O manifesto do Codex é um overlay do manifesto compatível com Claude. Se o
/// pacote já traz os dois, campos nativos ganham; se traz só o de Claude, a
/// cópia recebe o mínimo nativo sem alterar a origem.
fn write_portable_manifest(root: &Path, id: &str, version: &str) -> Result<(), String> {
    let claude = read_json(&manifest_path(root));
    let native_path = root.join(".codex-plugin").join("plugin.json");
    let native = read_json(&native_path);
    let mut merged = serde_json::Map::new();
    if let Some(fields) = claude.as_ref().and_then(Value::as_object) {
        for (key, value) in fields {
            // No manifesto Claude, o objeto inline já é o mapa de eventos. O
            // manifesto Codex recebe um `HooksFile` completo, cujo mapa fica
            // dentro de `hooks`. Caminho em string é idêntico nos dois.
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
            // Um overlay nativo já está na forma que o Codex espera.
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

/// Remove a instalação compartilhada e apaga a referência em cada camada
/// derivada. Falhas são best effort: o item já saiu do hub, e o próximo spawn
/// reconstrói a configuração sem ele.
fn codex_remove_everywhere(canonical: &str) {
    let id = canonical.split_once('@').map_or(canonical, |(id, _)| id);
    let Ok(entries) = std::fs::read_dir(codex_workspaces_root()) else {
        return;
    };
    for entry in entries.flatten() {
        let home = entry.path();
        if !home.is_dir() {
            continue;
        }
        codex_remove(&home, canonical);
        forget_codex_config(&home, canonical);
        remove_marketplace_entry(&home, id);
    }
}

/* ---------- instalar o que já existe ---------- */

/// O que um endereço trouxe: a pasta que o clone ocupa, e os plugins que
/// vieram dentro dela. Um só é o caso comum — o repositório *é* o plugin, e ele
/// já entra no hub aqui mesmo (`saved`). Mais de um é um marketplace, e aí
/// quem escolhe é quem instalou; até escolher, nada foi cadastrado.
#[derive(serde::Serialize)]
pub struct Found {
    pub dir: String,
    pub plugins: Vec<Plugin>,
    pub saved: bool,
}

/// Instala: clona o repositório numa pasta do Prometeu e olha o que veio.
/// É `async` porque clonar leva segundos, e a janela não pode parar enquanto
/// isso acontece.
#[tauri::command(async)]
pub fn plugin_install(source: String) -> Result<Found, String> {
    let url = git_url(&source);
    if url.is_empty() {
        return Err(i18n::t("err.plugin.noSource"));
    }
    let dir = store().join(repo_name(&url));
    if dir.exists() {
        // Pasta ocupada: se algum plugin do hub mora nela, o que se quer é
        // atualizar, e não instalar de novo. Se não mora ninguém, é sobra de
        // uma escolha que ninguém terminou, e pode sair da frente.
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
    // Um plugin só não é escolha: instalar já é dizer que se quer aquele.
    let saved = plugins.len() == 1;
    if saved {
        plugin_save(plugins[0].clone())?;
    }
    Ok(Found {
        dir: dir.display().to_string(),
        plugins,
        saved,
    })
}

/// Desfaz o clone que ninguém escolheu — a folha fechada sem marcar nada. Só
/// apaga dentro da pasta do Prometeu, e só o que não está no hub.
#[tauri::command]
pub fn plugin_scrap(dir: String) {
    let dir = PathBuf::from(expand(&dir));
    if dir.starts_with(store()) && dir != store() && !lives_in(&dir) {
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Atualizar é o `git pull` da pasta que o Prometeu clonou, e só
/// `--ff-only`: se alguém mexeu no plugin à mão, o certo é dizer que não deu,
/// e não desmanchar o que a pessoa escreveu. A descrição é relida depois — é
/// dela que sai a linha embaixo do nome, e ela envelhece junto com o plugin.
#[tauri::command(async)]
pub fn plugin_update(id: String) -> Result<Vec<Plugin>, String> {
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
        return plugin_save(fresh);
    }
    Ok(load())
}

/// Algum plugin do hub mora nesta pasta?
fn lives_in(dir: &Path) -> bool {
    load()
        .iter()
        .any(|p| PathBuf::from(expand(&p.source)).starts_with(dir))
}

/// O que a pessoa cola virando endereço de clone. `owner/repo` é GitHub,
/// porque é de lá que vem quase todo plugin; o resto vai como veio, e é assim
/// que GitLab, Bitbucket e `git@…` funcionam sem o app saber deles. O
/// `/tree/branch` que o navegador põe na barra some: o que se clona é o
/// repositório.
fn git_url(source: &str) -> String {
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
    // `owner/repo`, e nada mais: qualquer outra coisa não é endereço nenhum.
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [owner, repo] => format!("https://github.com/{owner}/{repo}"),
        _ => String::new(),
    }
}

/// O nome da pasta que o clone vai ocupar: o do repositório.
fn repo_name(url: &str) -> String {
    let name = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git");
    slug(name)
}

/// O clone. `--depth 1` porque ninguém quer o histórico de um plugin, e sem
/// terminal nenhum: git que pede senha numa janela sem terminal ficaria
/// pendurado para sempre — melhor falhar e dizer que o repositório é privado.
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

/// De qual clone esta pasta faz parte. Um plugin de marketplace mora numa
/// subpasta, e quem tem `.git` é a raiz do clone — é ela que o `pull` puxa.
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

/// O erro do git tem parágrafos; o que interessa é a última linha, que é a que
/// diz o que houve.
fn last_line(text: &str) -> String {
    text.trim()
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

/// O que veio no clone. Um `plugin.json` na raiz é o caso comum: o repositório
/// é o plugin. Um `marketplace.json` é uma lista, e dela vale o que mora neste
/// mesmo clone (`"./plugins/x"`) — entrada que aponta para outro repositório é
/// outra instalação, pelo endereço dela. Sem nenhum dos dois, ainda se olha uma
/// pasta abaixo: repositório que guarda plugins em `plugins/` e não declara
/// nada é comum o bastante para não obrigar ninguém a saber disso.
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

/// Claude usa `"source": "./plugins/x"`; o formato nativo do Codex usa
/// `"source": {"source":"local","path":"./plugins/x"}`. O hub lê os
/// dois, mas só segue caminhos internos ao clone.
fn marketplace_local_source(entry: &Value) -> Option<&str> {
    match entry.get("source")? {
        Value::String(path) => Some(path),
        Value::Object(source) if source.get("source")?.as_str()? == "local" => {
            source.get("path")?.as_str()
        }
        _ => None,
    }
}

/// Uma pasta abaixo, e a de `plugins/` também: o suficiente para achar o que um
/// repositório sem manifesto na raiz guarda, sem sair varrendo o clone inteiro.
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

/// Um caminho do `marketplace.json` resolvido dentro do clone. `..` não passa:
/// o que um repositório de fora escreve não pode apontar para outro lugar do
/// disco.
fn within(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim().trim_start_matches("./");
    if rel.is_empty() {
        return Some(dir.to_path_buf());
    }
    let at = dir.join(rel);
    (!rel.starts_with('/') && !at.components().any(|c| c.as_os_str() == "..")).then_some(at)
}

/// O plugin como o hub o guarda, lido do manifesto dele. Sem `name` no
/// manifesto vale o nome da pasta — que é o que o CLI também faria.
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

/* ---------- criar um plugin aqui dentro ---------- */

/// A pasta de que o Prometeu é dono: um plugin por subpasta, com o nome
/// dele. Fora dela ficam os que alguém escreve num repositório seu e cadastra
/// à mão — e é por isso que remover só apaga arquivo quando o plugin nasceu
/// aqui.
pub fn store() -> PathBuf {
    paths::root().join("plugins")
}

/// Quem escreve o plugin não é o nomeador: aqui saem frontmatter e JSON que o
/// CLI vai ler, e manifesto torto é plugin que não carrega em lugar nenhum.
/// `sonnet` é alias, e alias não envelhece.
const MAKER_MODEL: &str = "sonnet";

/// Teto de uma criação. Passou disto algo travou — e um `claude` esquecido
/// continuaria escrevendo numa pasta que ninguém está mais olhando.
const MAKER_TIMEOUT: Duration = Duration::from_secs(600);

/// Pedido maior que isto não é o que um plugin faz — é um projeto.
const MAX_ASK: usize = 4000;

/// Linha de progresso não é parágrafo.
const MAX_STEP: usize = 140;

/// O que o agente que escreve o plugin precisa saber e não adivinha: o nome de
/// cada arquivo, o que vai no frontmatter de cada um, e como um hook aponta
/// para o script dele em qualquer máquina. O pedido da pessoa vai depois
/// disto, como prompt — este texto é o que impede que ele vire um projeto de
/// software em vez de um plugin.
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

/// Uma linha de progresso. `file` é um arquivo que ele acabou de escrever;
/// `say` é o que ele mesmo disse. O back não escreve frase: a de fora de um é
/// a tela que põe, e o outro é palavra do agente, que fica como veio.
#[derive(Clone, serde::Serialize)]
pub struct Step {
    pub kind: String,
    pub text: String,
}

/// O que a tela precisa para acompanhar uma criação: por onde os eventos vêm,
/// e o nome que a pasta levou.
#[derive(serde::Serialize)]
pub struct Make {
    pub run: u64,
    pub slug: String,
}

/// Os agentes que estão escrevendo agora, por corrida. É o que deixa cancelar
/// — e o teto de tempo matar — sem que uma corrida velha derrube a nova.
fn running() -> &'static Mutex<HashMap<u64, Child>> {
    static RUNS: OnceLock<Mutex<HashMap<u64, Child>>> = OnceLock::new();
    RUNS.get_or_init(Default::default)
}

fn next_run() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Cria um plugin e volta na hora: escrever leva minutos, e quem clicou
/// precisa ver a folha andar. O que acontece depois chega por `plugin-make`
/// (cada passo) e `plugin-made` (o fim, com o erro dentro se houve).
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
        // Pasta que não virou plugin não fica: o hub não a mostraria, e uma
        // pasta que ninguém acha é lixo que só cresce.
        if end.is_err() {
            std::fs::remove_dir_all(&place).ok();
        }
        let _ = app.emit("plugin-made", (run, end.err().unwrap_or_default()));
    });
    Ok(Make { run, slug })
}

/// Cancelar. O processo morre, o `for` das linhas acaba com o stdout fechado,
/// e o fim de `make` trata isso como qualquer outra saída ruim — inclusive
/// apagando a pasta pela metade.
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

/// A criação em si: um `claude -p` dentro da pasta nova, com as ferramentas de
/// arquivo e nada mais — sem os hooks e os plugins de quem está usando o app
/// (que aqui só atrapalhariam), sem MCP, e sem poder rodar comando. O que ele
/// escreve fora da pasta o CLI não aceita sozinho, e em `-p` não há ninguém
/// para aceitar.
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
    // Mesma razão do nomeador: um `claude` rodando dentro de outro herda
    // CLAUDE_CODE_CHILD_SESSION e companhia, e o que ele herda não é dele.
    for (k, _) in std::env::vars() {
        if k.starts_with("CLAUDE") {
            cmd.env_remove(k);
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

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

/// O teto de tempo de uma corrida, numa thread que só dorme. Corrida que
/// acabou já saiu do mapa, e aí isto não faz nada.
fn watch(run: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(MAKER_TIMEOUT);
        stop(run);
    });
}

/// O que nasceu na pasta só é plugin se o manifesto estiver lá e for legível —
/// e é o próprio manifesto que diz o nome e a descrição que vão para o hub,
/// como em qualquer plugin cadastrado à mão.
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
    plugin_save(Plugin {
        id,
        source: dir.display().to_string(),
        note: text("description"),
        made: true,
        from: String::new(),
    })?;
    Ok(())
}

/// Uma linha do `stream-json` virando o que a tela mostra: o arquivo que ele
/// acabou de escrever, ou a frase que ele disse. O resto do stream — o que ele
/// leu, o que gastou, o resultado — não é progresso para ninguém.
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

/// O caminho como ele vale dentro do plugin. O agente escreve caminho inteiro,
/// e o que interessa na tela é `skills/x/SKILL.md`.
fn inside(dir: &Path, path: &str) -> String {
    Path::new(path)
        .strip_prefix(dir)
        .unwrap_or(Path::new(path))
        .display()
        .to_string()
}

/// O nome vira pasta e vira plugin: minúsculas, sem acento e sem espaço,
/// porque é ele que vai para o disco e para a linha de comando do CLI. O
/// acento cai em cima da letra que ele acentua — "revisão" é "revisao", e não
/// "revis-o".
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

/// O acento cai em cima da letra que ele acentua. Não é normalização de
/// Unicode inteira — é o que um nome de plugin em português e espanhol traz.
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

    fn plugin(id: &str, source: &str) -> Plugin {
        Plugin {
            id: id.into(),
            source: source.into(),
            note: String::new(),
            made: false,
            from: String::new(),
        }
    }

    /// Pasta vira `--plugin-dir`, endereço vira `--plugin-url`. É a única
    /// diferença entre os dois, e é ela que decide se o CLI baixa alguma coisa.
    #[test]
    fn a_origem_decide_a_flag() {
        assert_eq!(
            flags(&plugin("caveman", "/opt/caveman")),
            ["--plugin-dir".to_string(), "/opt/caveman".to_string()]
        );
        assert_eq!(
            flags(&plugin("x", "https://exemplo.com/x.zip")),
            [
                "--plugin-url".to_string(),
                "https://exemplo.com/x.zip".to_string()
            ]
        );
    }

    /// O `~` é da casa desta máquina, e quem o resolve é o app — o `claude`
    /// recebe caminho inteiro, que é o que ele entende.
    #[test]
    fn o_til_vira_caminho() {
        let [_, path] = flags(&plugin("x", "~/plugins/x"));
        assert_eq!(path, paths::home().join("plugins/x").display().to_string());
        assert!(!path.starts_with('~'));
    }

    /// A linha de comando de uma escolha: um par por plugin marcado, na ordem
    /// em que foram marcados. Nome que já não está no hub (apagado depois de
    /// escolhido) some da linha em vez de derrubar a conversa — o que o agente
    /// perde é um plugin, e dizer isso é trabalho da tela.
    #[test]
    fn a_escolha_vira_linha_de_comando() {
        let hub = vec![
            plugin("caveman", "/opt/caveman"),
            plugin("ponytail", "https://exemplo.com/ponytail.zip"),
        ];
        let chosen = ["ponytail".to_string(), "apagado".into(), "caveman".into()];
        assert_eq!(
            args_from(&hub, &chosen),
            [
                "--plugin-url",
                "https://exemplo.com/ponytail.zip",
                "--plugin-dir",
                "/opt/caveman",
            ]
        );
        // Marcar nenhum é escolha, e não vira flag nenhuma.
        assert!(args_from(&hub, &[]).is_empty());
    }

    /// Nome vazio não grava: é a identidade do plugin, e o CLI dedupe por ele.
    #[test]
    fn sem_nome_nao_grava() {
        assert!(plugin_save(plugin("  ", "/opt/x")).is_err());
    }

    /// Pasta que não é plugin é recusada no cadastro, e não descoberta no
    /// silêncio de uma sessão que subiu sem ele.
    #[test]
    fn pasta_sem_manifesto_e_recusada() {
        let dir = std::env::temp_dir().join(format!("prometeu-plug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_source(&dir.display().to_string()).is_err());

        std::fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        std::fs::write(
            dir.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"exemplo","description":"o que ele faz"}"#,
        )
        .unwrap();
        assert!(check_source(&dir.display().to_string()).is_ok());

        // E o manifesto é quem preenche o formulário.
        let looked = plugin_look(dir.display().to_string()).unwrap();
        assert_eq!(looked.id, "exemplo");
        assert_eq!(looked.note, "o que ele faz");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Caminho que não existe também é recusado — cadastrar aponta para algo.
    #[test]
    fn caminho_que_nao_existe_e_recusado() {
        assert!(check_source("/nao/existe/plugin").is_err());
        assert!(check_source("https://exemplo.com/x.zip").is_ok());
    }

    /// `.zip` sem manifesto para ler ganha o nome do arquivo como palpite.
    #[test]
    fn zip_ganha_o_nome_do_arquivo() {
        assert_eq!(guessed_name("https://exemplo.com/caveman.zip"), "caveman");
        assert_eq!(guessed_name("/tmp/meu-plugin.zip"), "meu-plugin");
    }

    /// O que a pessoa cola na caixa vira endereço de clone: o que o navegador
    /// dá, o `owner/repo` que se diz em voz alta, e o endereço de git de
    /// qualquer outro servidor.
    #[test]
    fn o_endereco_colado_vira_clone() {
        let git = "https://github.com/JuliusBrussee/caveman";
        assert_eq!(git_url("JuliusBrussee/caveman"), git);
        assert_eq!(git_url("github.com/JuliusBrussee/caveman"), git);
        assert_eq!(git_url("https://github.com/JuliusBrussee/caveman/"), git);
        assert_eq!(
            git_url("https://github.com/JuliusBrussee/caveman/tree/main"),
            git
        );
        // O que já é endereço de git vai como veio.
        assert_eq!(
            git_url("git@github.com:dietrichgebert/ponytail.git"),
            "git@github.com:dietrichgebert/ponytail.git"
        );
        assert_eq!(
            git_url("https://gitlab.com/time/x.git"),
            "https://gitlab.com/time/x.git"
        );
        // E o que não é endereço nenhum não vira um.
        assert!(git_url("  ").is_empty());
        assert!(git_url("caveman").is_empty());
    }

    /// A pasta do clone tem o nome do repositório, com ou sem `.git`.
    #[test]
    fn a_pasta_tem_o_nome_do_repositorio() {
        assert_eq!(
            repo_name("https://github.com/JuliusBrussee/caveman"),
            "caveman"
        );
        assert_eq!(
            repo_name("git@github.com:dietrichgebert/ponytail.git"),
            "ponytail"
        );
    }

    /// O que o clone traz: o repositório que é o plugin, o marketplace que
    /// lista os do mesmo clone, e o repositório que só tem uma pasta `plugins`.
    /// Entrada que aponta para outro repositório não é deste clone, e fica de
    /// fora — instalá-la é instalar o endereço dela.
    #[test]
    fn o_clone_diz_quais_plugins_vieram() {
        let root = std::env::temp_dir().join(format!("prometeu-inst-{}", uuid::Uuid::new_v4()));
        let manifest = |at: &Path, name: &str| {
            std::fs::create_dir_all(at.join(".claude-plugin")).unwrap();
            std::fs::write(
                at.join(".claude-plugin").join("plugin.json"),
                format!(r#"{{"name":"{name}","description":"o que ele faz"}}"#),
            )
            .unwrap();
        };

        // O repositório é o plugin.
        let one = root.join("um");
        manifest(&one, "caveman");
        let found = plugins_in(&one, "https://exemplo/caveman");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "caveman");
        assert_eq!(found[0].note, "o que ele faz");
        assert!(found[0].made);
        assert_eq!(found[0].from, "https://exemplo/caveman");

        // Um marketplace, com um plugin daqui e um de outro repositório.
        let many = root.join("muitos");
        manifest(&many.join("plugins").join("a"), "a");
        manifest(&many.join("plugins").join("b"), "b");
        std::fs::create_dir_all(many.join(".claude-plugin")).unwrap();
        std::fs::write(
            many.join(".claude-plugin").join("marketplace.json"),
            r#"{"plugins":[{"name":"a","source":"./plugins/a"},{"name":"fora","source":{"source":"git-subdir","url":"https://exemplo/outro.git"}}]}"#,
        )
        .unwrap();
        std::fs::create_dir_all(many.join(".agents").join("plugins")).unwrap();
        std::fs::write(
            many.join(".agents").join("plugins").join("marketplace.json"),
            r#"{"name":"nativo","plugins":[{"name":"b","source":{"source":"local","path":"./plugins/b"}}]}"#,
        )
        .unwrap();
        let found = plugins_in(&many, "https://exemplo/muitos");
        assert_eq!(
            found.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );

        // Sem manifesto e sem marketplace, uma pasta abaixo ainda é achada.
        let loose = root.join("solto");
        manifest(&loose.join("plugins").join("c"), "c");
        assert_eq!(
            plugins_in(&loose, "")
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["c"]
        );

        // E o que não tem plugin nenhum não devolve nada.
        std::fs::create_dir_all(root.join("vazio")).unwrap();
        assert!(plugins_in(&root.join("vazio"), "").is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// O mesmo pacote vira entrada de marketplace nativa e recebe um
    /// manifesto Codex sem perder os hooks do manifesto compatível com Claude.
    /// O hash na versão é o cachebuster de atualizações sem versão upstream.
    #[test]
    fn o_marketplace_do_codex_nasce_do_mesmo_plugin() {
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

        // Um snapshot feito pela revisão anterior guardava o hash puro da
        // origem e o objeto inline sem o envelope do Codex. Mesmo sem mudar o
        // plugin, a revisão do adapter precisa refazer essa cópia.
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
    fn detecta_hooks_que_precisam_nascer_ativos() {
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
    fn versao_livre_do_claude_nao_quebra_o_cache_do_codex() {
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
    fn home_do_codex_pertence_ao_workspace_e_nao_ao_cwd() {
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
    fn apagar_home_derivado_nao_segue_links_para_o_home_real() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-remove-{}", uuid::Uuid::new_v4()));
        let homes = root.join("homes");
        let home = homes.join("workspace");
        let real = root.join("real");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("auth.json"), "conta").unwrap();
        std::os::unix::fs::symlink(real.join("auth.json"), home.join("auth.json")).unwrap();

        remove_codex_home(&homes, &home);

        assert!(!home.exists());
        assert_eq!(
            std::fs::read_to_string(real.join("auth.json")).unwrap(),
            "conta"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn home_derivado_preserva_estado_sem_mudar_a_config_global() {
        let root =
            std::env::temp_dir().join(format!("prometeu-codex-home-{}", uuid::Uuid::new_v4()));
        let base = root.join("base");
        let home = root.join("workspace");
        let marketplace = home.join("marketplace");
        std::fs::create_dir_all(base.join("plugins")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(base.join("auth.json"), "conta").unwrap();
        let global = r#"
[projects."/tmp/projeto"]
trust_level = "trusted"

[plugins."global@outro"]
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
            config["projects"]["/tmp/projeto"]["trust_level"].as_str(),
            Some("trusted")
        );
        assert_eq!(
            config["plugins"]["global@outro"]["enabled"].as_bool(),
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

    /// Prova o contrato contra o CLI instalado: marketplace local, cópia no
    /// cache, skill visível e `SessionStart` ativo no runtime isolado. É opt-in
    /// porque escreve e remove uma entrada temporária no cache real do Codex.
    #[test]
    #[ignore]
    fn codex_instala_plugin_portatil_de_verdade() {
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
        let global_config = user_codex_home().join("config.toml");
        let global_before = std::fs::read(&global_config).ok();
        std::env::set_var("PROMETEU_ROOT", &root);
        let canonical = format!("{id}@{}", codex_marketplace_name());
        let workspace = format!("workspace-{suffix}");
        let home = codex_workspace_home(&workspace);
        let result = (|| -> Result<(), String> {
            plugin_save(plugin(&id, &source.display().to_string()))?;
            let chosen = vec![id.clone()];
            let selected = codex_for(&workspace, Some(&chosen))?;
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
                .ok_or_else(|| "plugin não apareceu no cache do Codex".to_string())?
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
                .map_err(|error| format!("lista ilegível: {error}"))?;
            let active = listed["installed"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|entry| entry["pluginId"].as_str() == Some(&canonical));
            if active.and_then(|entry| entry["enabled"].as_bool()) != Some(true) {
                return Err(format!("plugin não ficou ativo no home: {active:?}"));
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
                return Err("a sessão Codex não recebeu a skill instalada".into());
            }

            // `debug prompt-input` prova a descoberta da skill, mas não abre
            // uma sessão. A prova do hook usa o app-server real e observa o
            // efeito do `SessionStart` antes de qualquer turno ou modelo.
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
                    .ok_or_else(|| "app-server sem stdin".to_string())?;
                let output = server
                    .stdout
                    .take()
                    .ok_or_else(|| "app-server sem stdout".to_string())?;
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
                        let message = receive
                            .recv_timeout(Duration::from_secs(5))
                            .map_err(|error| format!("app-server sem resposta {id}: {error}"))?;
                        if message["id"].as_u64() == Some(id) {
                            if let Some(error) = message["error"]["message"].as_str() {
                                return Err(format!("app-server recusou {id}: {error}"));
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
                    .ok_or_else(|| format!("hook não descoberto: {}", listed["result"]))?;
                let key = hook["key"]
                    .as_str()
                    .ok_or_else(|| format!("hook sem key: {hook}"))?;
                let hash = hook["currentHash"]
                    .as_str()
                    .ok_or_else(|| format!("hook sem hash: {hook}"))?;
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
                        .ok_or_else(|| "thread/start sem id".to_string())?;
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
                Err("o SessionStart do plugin não nasceu ativo".into())
            })();
            server.kill().ok();
            server.wait().ok();
            live?;
            if std::fs::read_to_string(&hook_ran).ok().as_deref()
                != Some(format!("{hook_marker}\n").as_str())
            {
                return Err("o SessionStart do plugin não nasceu ativo".into());
            }

            std::fs::write(
                source.join("skills").join(&id).join("SKILL.md"),
                format!("---\nname: {id}\ndescription: {updated_marker}\n---\n"),
            )
            .map_err(|error| error.to_string())?;
            codex_for(&workspace, Some(&chosen))?;
            let updated_version = codex_installed(&home)?
                .get(&canonical)
                .ok_or_else(|| "plugin atualizado sumiu do cache do Codex".to_string())?
                .version
                .clone();
            if updated_version == first_version {
                return Err("o hash novo não invalidou a versão instalada".into());
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
                return Err("a sessão Codex não recebeu a versão atualizada".into());
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

    /// A instalação de verdade, contra o GitHub: clona, acha o plugin na raiz
    /// e cadastra. Fica `ignore` porque depende de rede e do endereço continuar
    /// existindo — `cargo test -- --ignored instala_de_verdade` quando se mexe
    /// no clone ou na leitura do manifesto.
    #[test]
    #[ignore]
    fn instala_de_verdade() {
        let root = std::env::temp_dir().join(format!("prometeu-net-{}", uuid::Uuid::new_v4()));
        // Só este teste roda quando se pede `--ignored`; o env é do processo.
        std::env::set_var("PROMETEU_ROOT", &root);

        let found = plugin_install("JuliusBrussee/caveman".into()).unwrap();
        assert!(found.saved);
        assert_eq!(found.plugins.len(), 1);
        assert_eq!(found.plugins[0].id, "caveman");
        assert!(found.plugins[0].note.len() > 10);
        assert_eq!(
            found.plugins[0].from,
            "https://github.com/JuliusBrussee/caveman"
        );
        assert!(manifest_path(Path::new(&found.plugins[0].source)).exists());

        // E ele entrou no hub, com a linha de comando que a sessão vai receber.
        assert_eq!(
            args_from(&load(), &["caveman".to_string()]),
            ["--plugin-dir", &found.plugins[0].source]
        );

        // Instalar de novo é atualizar, e o hub diz isso em vez de clonar por
        // cima do que já está lá.
        assert!(plugin_install("https://github.com/JuliusBrussee/caveman".into()).is_err());
        plugin_update("caveman".into()).unwrap();

        // Remover leva a pasta junto, porque ela é do Prometeu.
        plugin_remove("caveman".into()).unwrap();
        assert!(!store().join("caveman").exists());
        std::env::remove_var("PROMETEU_ROOT");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Caminho de marketplace não sai do clone: o que um repositório de fora
    /// escreve não aponta para outro lugar do disco.
    #[test]
    fn caminho_de_marketplace_nao_sai_do_clone() {
        let dir = Path::new("/tmp/clone");
        assert_eq!(within(dir, "./plugins/a"), Some(dir.join("plugins/a")));
        assert_eq!(within(dir, "./"), Some(dir.to_path_buf()));
        assert!(within(dir, "../../etc").is_none());
        assert!(within(dir, "/etc").is_none());
    }

    /// O nome que a pessoa escreve vira pasta: sem acento, sem espaço e sem
    /// dois traços seguidos, porque é ele que o CLI vai ler.
    #[test]
    fn o_nome_vira_pasta() {
        assert_eq!(slug("Revisão de front"), "revisao-de-front");
        assert_eq!(slug("  Caveman!!  "), "caveman");
        assert_eq!(slug("padrões — do time"), "padroes-do-time");
        assert_eq!(slug("!!!"), "");
    }

    /// O que a tela mostra enquanto ele escreve: o arquivo que saiu, ou a
    /// primeira frase dele. Ler não é progresso, e o caminho aparece como ele
    /// vale dentro do plugin.
    #[test]
    fn o_stream_vira_progresso() {
        let dir = Path::new("/tmp/plug");
        let wrote = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/tmp/plug/skills/x/SKILL.md"}}]}}"#;
        let wrote = step(dir, wrote).unwrap();
        assert_eq!(
            (wrote.kind.as_str(), wrote.text.as_str()),
            ("file", "skills/x/SKILL.md")
        );

        let read = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/tmp/plug/x"}}]}}"#;
        assert!(step(dir, read).is_none());

        let said = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"\n  Vou começar pelo manifesto.\nDepois as skills."}]}}"#;
        let said = step(dir, said).unwrap();
        assert_eq!(
            (said.kind.as_str(), said.text.as_str()),
            ("say", "Vou começar pelo manifesto.")
        );

        // O resto do stream não é progresso de ninguém, e linha que não é JSON
        // não pode derrubar a leitura.
        assert!(step(dir, r#"{"type":"result","subtype":"success"}"#).is_none());
        assert!(step(dir, "não é json").is_none());
    }

    /// Sem manifesto não nasceu plugin nenhum, e o que não nasceu não entra no
    /// hub — é o que separa "o agente escreveu" de "o agente respondeu".
    #[test]
    fn pasta_sem_manifesto_nao_vira_plugin() {
        let dir = std::env::temp_dir().join(format!("prometeu-made-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(born(&dir, "exemplo").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
