//! Repository configuration declares setup, run, and archive commands. Prefer
//! .prometeu/settings.toml over Conductor settings, and inherit the original clone's complete
//! configuration when a worktree has none. Copy configured secrets and ignored files before setup;
//! absent a copy declaration, include root .env variants. The app can ask an agent to write
//! configuration, but does not infer project commands itself.

use crate::i18n;
use crate::selection::Tools;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

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

#[derive(Deserialize, Default)]
struct File {
    #[serde(default)]
    scripts: Table,
    #[serde(default)]
    worktree: WorktreeTable,
    /// The project layer of the tool selection (ADR 0043). Versioned, so it never activates on its
    /// own; project-declared items are gated on trust before injection.
    #[serde(default)]
    tools: Tools,
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
    /// from the clone like the scripts. Absent axes inherit the layers above (ADR 0043).
    pub tools: Tools,
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
            declared: parsed.worktree.copy,
        };
    }
    Scripts::default()
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

/* Worktree hydration */

/// Report each copy result in the Setup header so missing or failed preparation remains visible.
pub enum Copied {
    Made(String),
    /// Report existing worktree files too, explaining why local edits are preserved instead of
    /// replaced by clone copies.
    Kept(String),
    Failed(String, String),
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
fn safe(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel.trim());
    let inside = path.components().all(|c| matches!(c, Component::Normal(_)));
    (inside && path.components().next().is_some()).then(|| path.to_path_buf())
}

/// Copy only missing files before setup. Preserve committed or manually edited destination files so
/// rerunning setup is safe.
pub fn hydrate(worktree: &Path, repo: &Path, list: &[String]) -> Vec<Copied> {
    if worktree == repo {
        return Vec::new();
    }
    list.iter()
        .filter_map(|rel| {
            let path = safe(rel)?;
            let to = worktree.join(&path);
            if to.exists() {
                return Some(Copied::Kept(rel.clone()));
            }
            Some(match copy_into(&repo.join(&path), &to) {
                Ok(()) => Copied::Made(rel.clone()),
                Err(e) => Copied::Failed(rel.clone(), e),
            })
        })
        .collect()
}

/// Copy files or complete directories while retaining permissions required by private keys.
fn copy_into(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(i18n::io)?;
    }
    if !from.is_dir() {
        return std::fs::copy(from, to).map(|_| ()).map_err(i18n::io);
    }
    std::fs::create_dir_all(to).map_err(i18n::io)?;
    for entry in std::fs::read_dir(from).map_err(i18n::io)? {
        let entry = entry.map_err(i18n::io)?;
        copy_into(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

/// Omit the Setup copy header when there are no entries.
pub fn report(notes: &[Copied]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut out = String::new();
    for note in notes {
        out.push_str(&match note {
            Copied::Made(path) => {
                format!(
                    "\x1b[32m→\x1b[0m {path} {}\r\n",
                    i18n::pick("veio do clone", "copied from the clone")
                )
            }
            Copied::Kept(path) => {
                format!(
                    "\x1b[2m· {path} {}\x1b[0m\r\n",
                    i18n::pick("já estava aqui", "already here")
                )
            }
            Copied::Failed(path, why) => format!("\x1b[31m✗\x1b[0m {path}: {why}\r\n"),
        });
    }
    out.push_str("\r\n");
    Some(out)
}

/// Expose both Prometeu and Conductor environment names for compatible scripts. Also set
/// conventional PORT so common development servers started directly from the dock use the workspace
/// port.
pub fn env(worktree: &Path, repo: &Path, name: &str, port: Option<u16>) -> Vec<(String, String)> {
    let mut pairs = vec![
        ("WORKSPACE_PATH", worktree.display().to_string()),
        ("ROOT_PATH", repo.display().to_string()),
        ("WORKSPACE_NAME", name.to_string()),
    ];
    if let Some(port) = port {
        pairs.push(("PORT", port.to_string()));
    }
    let mut out: Vec<(String, String)> = pairs
        .into_iter()
        .flat_map(|(key, value)| {
            [
                (format!("PROMETEU_{key}"), value.clone()),
                (format!("CONDUCTOR_{key}"), value),
            ]
        })
        .collect();
    if let Some(port) = port {
        out.push(("PORT".into(), port.to_string()));
    }
    out
}

/// Reserve ten consecutive ports per workspace, aligned to multiples of ten. Exclude saved
/// reservations even when their servers are stopped. Begin probing at a stable hash of the worktree
/// path to reduce collisions across independent app boards. Hash collisions remain possible within
/// the 690 ranges; actual binds provide the final check.
const FIRST: u16 = 3100;
const SLOTS: u16 = (9990 - FIRST) / 10 + 1;

/// Derive the initial port range only from the worktree path so it is stable across boards and
/// machine activity.
fn port_start(worktree: &Path) -> u16 {
    (crate::paths::fnv1a(&worktree.to_string_lossy()) % u64::from(SLOTS)) as u16
}

pub fn alloc_port(worktree: &Path, taken: &[u16]) -> Option<u16> {
    let start = port_start(worktree);
    (0..SLOTS)
        .map(|i| FIRST + ((start + i) % SLOTS) * 10)
        .find(|base| !taken.contains(base) && (0..10).all(|i| usable(base + i) && free(base + i)))
}

/// Exclude browser-blocked Fetch ports within the allocator range. A server may answer curl while
/// Chromium or WebKit refuses its port.
const BAD: &[u16] = &[
    3659, 4045, 4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697, 10080,
];

/// Check whether browsers permit the localhost port.
pub fn usable(port: u16) -> bool {
    !BAD.contains(&port)
}

/// Check both IPv4 and IPv6 loopbacks. Only address-in-use errors prove a conflict; unavailable
/// IPv6 does not.
fn free(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener};
    [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]
    .into_iter()
    .all(|ip| match TcpListener::bind((ip, port)) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::AddrInUse,
    })
}

/// Ask the agent to derive configuration from repository documentation and manifests rather than
/// guessing project commands here.
pub fn ask_prompt(file: &str) -> String {
    format!(
        r#"Descubra como preparar e como rodar este projeto, e escreva isso em `{file}`.

O Prometeu roda cada trabalho num worktree git separado. Worktree novo vem sem
nada que o `.gitignore` esconde: dependências, `.env`, banco, build. O `setup` é
o que transforma o worktree num lugar onde dá para trabalhar; o `run` é o que
sobe o projeto para eu ver a mudança funcionando. O que nenhum comando
reconstrói — segredo, chave — vai em `[worktree] copy`, e o Prometeu copia do
clone de origem antes do setup.

Leia o README, os manifestos de pacote e os scripts do repositório antes de
responder. Não chute.

Formato:

```toml
[scripts]
setup = "..."
run = "..."

[worktree]
copy = [".env"]
```

Regras:

- Rodam com `/bin/sh -lc`, com o worktree como diretório atual.
- `setup` precisa ser idempotente: roda inteiro em cada worktree novo.
- `copy` são caminhos relativos à raiz do repositório, e só o que o `.gitignore`
  esconde e nenhum comando refaz. Não escreva `cp` no `setup` para isso: a cópia
  acontece antes dele, nunca sobrescreve, e aparece na aba Setup. Omita a seção
  inteira se o `.env` da raiz é o único caso — esse já vai sozinho.
- `run` precisa ficar em primeiro plano — sem `&`, sem `--daemon`. O Prometeu
  mostra a saída num terminal e mata o processo quando eu peço.
- Se o projeto abre porta, use `$PROMETEU_PORT`. Ela é reservada só para este
  worktree; porta fixa faz dois worktrees brigarem. Há mais nove, de
  `$PROMETEU_PORT`+1 a +9. `$PORT` vale o mesmo, para o que já lê a convenção.
- Outras variáveis: `$PROMETEU_WORKSPACE_PATH`, `$PROMETEU_ROOT_PATH`,
  `$PROMETEU_WORKSPACE_NAME`.
- Nada destrutivo fora do worktree, e nada de rede além do que instalar
  dependência exige.

Escreva o arquivo e rode o `setup` uma vez para confirmar que ele passa."#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(std::path::PathBuf);

    impl std::ops::Deref for Tmp {
        type Target = Path;

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn tmp(name: &str) -> Tmp {
        let dir =
            std::env::temp_dir().join(format!("prometeu-scripts-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    #[test]
    fn le_as_tres_chaves() {
        let dir = tmp("tres");
        write(
            &dir,
            ".prometeu/settings.toml",
            "[scripts]\nsetup = \"npm i\"\nrun = \"npm dev\"\narchive = \"rm -rf tmp\"\n",
        );
        let s = read(&dir);
        assert_eq!(s.setup.as_deref(), Some("npm i"));
        assert_eq!(s.run(None).unwrap().command, "npm dev");
        assert_eq!(s.archive.as_deref(), Some("rm -rf tmp"));
        assert_eq!(s.file.as_deref(), Some(".prometeu/settings.toml"));
    }

    /// Support Conductor's named run tables and place the configured default first.
    #[test]
    fn run_nomeado_com_padrao_na_frente() {
        let dir = tmp("nomeado");
        write(
            &dir,
            ".conductor/settings.toml",
            r#"[scripts]
setup = "pnpm i"

[scripts.run.api]
command = "bin/api"

[scripts.run.web]
command = "pnpm dev"
default = true
"#,
        );
        let s = read(&dir);
        assert_eq!(s.runs.len(), 2);
        assert_eq!(s.run(None).unwrap().name, "web");
        assert_eq!(s.run(Some("api")).unwrap().command, "bin/api");
    }

    /// Prometeu settings replace the entire Conductor fallback rather than merging partial files.
    #[test]
    fn prometeu_tem_prioridade_e_nao_mistura() {
        let dir = tmp("prioridade");
        write(
            &dir,
            ".conductor/settings.toml",
            "[scripts]\nsetup = \"velho\"\nrun = \"velho\"\n",
        );
        write(
            &dir,
            ".prometeu/settings.toml",
            "[scripts]\nrun = \"novo\"\n",
        );
        let s = read(&dir);
        assert_eq!(s.run(None).unwrap().command, "novo");
        assert!(s.setup.is_none());
    }

    /// Malformed settings remain on disk for repair while script discovery returns empty.
    #[test]
    fn toml_quebrado_nao_explode() {
        let dir = tmp("quebrado");
        write(&dir, ".prometeu/settings.toml", "[scripts\nsetup = ");
        let s = read(&dir);
        assert!(s.setup.is_none() && s.runs.is_empty());
        assert_eq!(s.file.as_deref(), Some(".prometeu/settings.toml"));
    }

    #[test]
    fn sem_arquivo_nao_tem_script() {
        let s = read(&tmp("vazio"));
        assert!(s.file.is_none() && s.runs.is_empty());
    }

    /// Inherit clone settings only when the worktree has none; a local file completely replaces the
    /// fallback.
    #[test]
    fn worktree_sem_arquivo_herda_o_do_clone() {
        let repo = tmp("herda-repo");
        let wt = tmp("herda-wt");
        write(
            &repo,
            ".prometeu/settings.toml",
            "[scripts]\nsetup = \"npm i\"\nrun = \"npm dev\"\n",
        );

        let s = read_for(&wt, &repo);
        assert!(s.inherited);
        assert_eq!(s.run(None).unwrap().command, "npm dev");
        assert_eq!(s.file.as_deref(), Some(".prometeu/settings.toml"));

        write(&wt, ".prometeu/settings.toml", "[scripts]\nrun = \"meu\"\n");
        let s = read_for(&wt, &repo);
        assert!(!s.inherited);
        assert_eq!(s.run(None).unwrap().command, "meu");
        assert!(s.setup.is_none());

        // A session in the original clone does not inherit from another directory.
        assert!(!read_for(&repo, &repo).inherited);
        // Do not invent settings when neither location has a file.
        let vazio = tmp("herda-vazio");
        assert!(read_for(&wt, &vazio).file.is_some()); // Use the worktree settings written above.
        assert!(read_for(&vazio, &vazio).file.is_none());
    }

    /// Verify this repository's mixed setup, named runs, and multiline command configuration so
    /// Prometeu can continue running itself.
    #[test]
    fn o_proprio_repositorio_e_lido() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let s = read(root);
        assert_eq!(s.file.as_deref(), Some(".prometeu/settings.toml"));
        assert_eq!(s.setup.as_deref(), Some("npm install"));
        assert_eq!(s.run(None).unwrap().name, "app");
        assert!(s
            .run(Some("browser"))
            .unwrap()
            .command
            .contains("Google Chrome"));
    }

    #[test]
    fn env_traz_os_dois_prefixos_e_o_port_solto() {
        let pairs = env(Path::new("/wt"), Path::new("/repo"), "x", Some(3100));
        let get = |k: &str| pairs.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone());
        assert_eq!(get("PROMETEU_PORT").as_deref(), Some("3100"));
        assert_eq!(get("CONDUCTOR_PORT").as_deref(), Some("3100"));
        assert_eq!(get("PORT").as_deref(), Some("3100"));
        assert_eq!(get("CONDUCTOR_WORKSPACE_PATH").as_deref(), Some("/wt"));
        // Omit PORT when unavailable instead of passing an empty value to scripts.
        assert!(env(Path::new("/wt"), Path::new("/repo"), "x", None)
            .iter()
            .all(|(k, _)| k != "PORT"));
    }

    /// Align reservations to ten ports and never reuse a range already saved by another workspace.
    #[test]
    fn porta_pula_a_ja_guardada_e_e_multipla_de_dez() {
        let wt = Path::new("/wt/a");
        let first = alloc_port(wt, &[]).unwrap();
        assert_eq!(first % 10, 0);
        let second = alloc_port(wt, &[first]).unwrap();
        assert_ne!(second, first);
        assert_eq!(second % 10, 0);
        assert!((3100..=9990).contains(&second));
    }

    /// Automatic copying includes root .env variants but excludes committed examples.
    #[test]
    fn copia_automatica_pega_os_env_e_deixa_o_exemplo() {
        let repo = tmp("auto-repo");
        for name in [".env", ".env.local", ".env.example", "package.json"] {
            write(&repo, name, "x");
        }
        std::fs::create_dir_all(repo.join(".env.d")).unwrap();
        let wt = tmp("auto-wt");
        assert_eq!(
            copies(&wt, &repo, None),
            vec![".env".to_string(), ".env.local".to_string()]
        );
    }

    /// An explicit declaration includes only existing clone paths that remain inside the
    /// repository.
    #[test]
    fn copia_declarada_filtra_o_que_nao_existe_e_o_que_escapa() {
        let repo = tmp("decl-repo");
        write(&repo, ".env", "x");
        write(&repo, "config/master.key", "x");
        let wt = tmp("decl-wt");
        let declared = [
            ".env".to_string(),
            "config/master.key".to_string(),
            "nao-existe".to_string(),
            "../fora".to_string(),
            "/etc/passwd".to_string(),
        ];
        assert_eq!(
            copies(&wt, &repo, Some(&declared)),
            vec![".env".to_string(), "config/master.key".to_string()]
        );
        // Sessions in the original clone copy nothing.
        assert!(copies(&repo, &repo, Some(&declared)).is_empty());
    }

    /// Keep resolved copy entries after copying so the Setup tab retains its content.
    #[test]
    fn copia_declarada_nao_encolhe_depois_de_copiar() {
        let repo = tmp("estavel-repo");
        write(&repo, ".env", "PORT=3000");
        let wt = tmp("estavel-wt");
        let list = copies(&wt, &repo, None);
        hydrate(&wt, &repo, &list);
        assert_eq!(copies(&wt, &repo, None), list);
    }

    #[test]
    fn hydrate_copia_o_que_falta_e_nao_sobrescreve() {
        let repo = tmp("hyd-repo");
        write(&repo, ".env", "do clone");
        write(&repo, "config/master.key", "chave");
        let wt = tmp("hyd-wt");
        write(&wt, ".env", "meu");

        let list = vec![".env".to_string(), "config/master.key".to_string()];
        let notes = hydrate(&wt, &repo, &list);
        assert!(matches!(notes[0], Copied::Kept(_)));
        assert!(matches!(notes[1], Copied::Made(_)));
        // Preserve existing worktree content.
        assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "meu");
        assert_eq!(
            std::fs::read_to_string(wt.join("config/master.key")).unwrap(),
            "chave"
        );

        // Rerunning hydration neither overwrites nor duplicates data.
        let de_novo = hydrate(&wt, &repo, &list);
        assert!(de_novo.iter().all(|n| matches!(n, Copied::Kept(_))));
        assert!(report(&de_novo).is_some());
        assert!(report(&[]).is_none());
    }

    /// Support whole directories for credentials stored as a tree.
    #[test]
    fn hydrate_copia_diretorio() {
        let repo = tmp("dir-repo");
        write(&repo, "config/credentials/production.key", "chave");
        let wt = tmp("dir-wt");
        hydrate(&wt, &repo, &["config/credentials".to_string()]);
        assert_eq!(
            std::fs::read_to_string(wt.join("config/credentials/production.key")).unwrap(),
            "chave"
        );
    }

    /// Inherited clone settings must resolve copy declarations even when settings and secrets are
    /// ignored by Git.
    #[test]
    fn copia_vem_junto_com_o_arquivo_herdado() {
        let repo = tmp("copia-herda-repo");
        write(
            &repo,
            ".prometeu/settings.toml",
            "[scripts]\nrun = \"x\"\n\n[worktree]\ncopy = [\"segredo\"]\n",
        );
        write(&repo, "segredo", "s");
        write(&repo, ".env", "nao-declarado");
        let wt = tmp("copia-herda-wt");

        let s = read_for(&wt, &repo);
        assert!(s.inherited);
        // An explicit copy list excludes unlisted automatic .env files.
        assert_eq!(s.copy, vec!["segredo".to_string()]);
    }

    /// An empty copy list disables copying rather than falling back to automatic discovery.
    #[test]
    fn copia_vazia_desliga_o_automatico() {
        let repo = tmp("vazia-repo");
        write(&repo, ".prometeu/settings.toml", "[worktree]\ncopy = []\n");
        write(&repo, ".env", "x");
        let wt = tmp("vazia-wt");
        assert!(read_for(&wt, &repo).copy.is_empty());
    }

    /// Skip an entire reserved range if any port is browser-blocked, including ports reached by
    /// PORT+n.
    #[test]
    fn porta_pula_as_que_o_navegador_recusa() {
        const SLOTS: u64 = (9990 - 3100) / 10 + 1;
        let slot = (5060 - 3100) / 10;
        let path = (0..)
            .map(|i| format!("/wt/{i}"))
            .find(|p| crate::paths::fnv1a(p) % SLOTS == slot)
            .unwrap();
        let base = alloc_port(Path::new(&path), &[]).unwrap();
        assert_ne!(base, 5060);
        assert!((base..base + 10).all(usable), "{base}");
    }

    /// Different paths produce different starting ranges, while a given path remains stable across
    /// boards. Verify the starting point because final allocation also depends on live sockets.
    #[test]
    fn porta_sai_do_caminho_do_worktree() {
        let a = Path::new("/Users/ana/prometeu/worktrees/app/feat-a");
        let b = Path::new("/Users/ana/prometeu/worktrees/app/feat-b");
        assert_ne!(port_start(a), port_start(b));
        for p in [a, b] {
            assert!(port_start(p) < SLOTS, "{}", port_start(p));
        }
    }

    /// The `[tools]` table parses into the layered Selections; an absent axis inherits.
    #[test]
    fn tools_da_tabela_vira_camada_do_projeto() {
        use crate::selection::{Base, Selection};
        let dir = tmp("tools");
        write(
            &dir,
            ".prometeu/settings.toml",
            "[scripts]\nsetup = \"npm i\"\n\n[tools]\nmcp = { base = \"inherit\", add = [\"notion\"] }\nplugins = { base = \"none\", add = [\"revisor\"] }\n",
        );
        let s = read(&dir);
        assert_eq!(
            s.tools.mcp,
            Some(Selection {
                base: Base::Inherit,
                add: vec!["notion".into()],
                remove: vec![],
            })
        );
        assert_eq!(
            s.tools.plugins,
            Some(Selection::only(vec!["revisor".into()]))
        );
        assert_eq!(s.tools.skills, None);
    }

    /// A worktree without settings inherits the clone's `[tools]`, and only the repository passed to
    /// read_for governs: for a multi-repository workspace the caller selects the primary one.
    #[test]
    fn tools_herdados_do_clone_e_so_do_repositorio_primario() {
        use crate::selection::Selection;
        let primary = tmp("tools-primario");
        let secondary = tmp("tools-secundario");
        let wt = tmp("tools-wt");
        write(
            &primary,
            ".prometeu/settings.toml",
            "[tools]\nplugins = { base = \"none\", add = [\"do-primario\"] }\n",
        );
        write(
            &secondary,
            ".prometeu/settings.toml",
            "[tools]\nplugins = { base = \"none\", add = [\"do-secundario\"] }\n",
        );

        let from_primary = read_for(&wt, &primary);
        assert!(from_primary.inherited);
        assert_eq!(
            from_primary.tools.plugins,
            Some(Selection::only(vec!["do-primario".into()]))
        );

        // Reading against the secondary repository yields its own layer, never the primary's.
        let from_secondary = read_for(&wt, &secondary);
        assert_eq!(
            from_secondary.tools.plugins,
            Some(Selection::only(vec!["do-secundario".into()]))
        );
    }
}
