//! Repository configuration declares setup, run, and archive commands. Prefer
//! .prometeu/settings.toml over Conductor settings, and inherit the original clone's complete
//! configuration when a worktree has none. Copy configured secrets and ignored files before setup;
//! absent a copy declaration, include root .env variants. The app can ask an agent to write
//! configuration, but does not infer project commands itself.

use crate::i18n;
#[cfg(test)]
use prometeu_files::settings::{copies, read};
pub use prometeu_files::settings::{read_for, Scripts, FILES, TEMPLATE};
#[cfg(test)]
use std::path::Path;

pub use prometeu_files::scripts::{alloc_port, env, hydrate, usable, Copied};
#[cfg(test)]
use prometeu_files::scripts::{port_start, SLOTS};
pub fn report(notes: &[Copied]) -> Option<String> {
    prometeu_files::scripts::report(notes, i18n::pick)
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
    fn reads_all_three_keys() {
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
    fn named_run_places_the_default_first() {
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
    fn prometeu_configuration_takes_precedence_without_merging() {
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
    fn malformed_toml_does_not_panic() {
        let dir = tmp("quebrado");
        write(&dir, ".prometeu/settings.toml", "[scripts\nsetup = ");
        let s = read(&dir);
        assert!(s.setup.is_none() && s.runs.is_empty());
        assert_eq!(s.file.as_deref(), Some(".prometeu/settings.toml"));
    }

    #[test]
    fn missing_files_have_no_scripts() {
        let s = read(&tmp("vazio"));
        assert!(s.file.is_none() && s.runs.is_empty());
    }

    /// Inherit clone settings only when the worktree has none; a local file completely replaces the
    /// fallback.
    #[test]
    fn worktrees_without_configuration_inherit_from_the_clone() {
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
        let empty = tmp("herda-vazio");
        assert!(read_for(&wt, &empty).file.is_some()); // Use the worktree settings written above.
        assert!(read_for(&empty, &empty).file.is_none());
    }

    /// Verify this repository's mixed setup, named runs, and multiline command configuration so
    /// Prometeu can continue running itself.
    #[test]
    fn reads_the_repository_itself() {
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
    fn environment_includes_both_prefixes_and_the_standalone_port() {
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
    fn port_skips_reserved_values_and_is_a_multiple_of_ten() {
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
    fn automatic_copy_includes_env_files_and_excludes_examples() {
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
    fn declared_copy_filters_missing_and_escaping_paths() {
        let repo = tmp("decl-repo");
        write(&repo, ".env", "x");
        write(&repo, "config/master.key", "x");
        let wt = tmp("decl-wt");
        let declared = [
            ".env".to_string(),
            "config/master.key".to_string(),
            "does-not-exist".to_string(),
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
    fn declared_copy_does_not_shrink_after_copying() {
        let repo = tmp("estavel-repo");
        write(&repo, ".env", "PORT=3000");
        let wt = tmp("estavel-wt");
        let list = copies(&wt, &repo, None);
        hydrate(&wt, &repo, &list);
        assert_eq!(copies(&wt, &repo, None), list);
    }

    #[test]
    fn hydrate_copies_missing_files_without_overwriting() {
        let repo = tmp("hyd-repo");
        write(&repo, ".env", "do clone");
        write(&repo, "config/master.key", "key");
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
            "key"
        );

        // Rerunning hydration neither overwrites nor duplicates data.
        let again = hydrate(&wt, &repo, &list);
        assert!(again.iter().all(|n| matches!(n, Copied::Kept(_))));
        assert!(report(&again).is_some());
        assert!(report(&[]).is_none());
    }

    /// Support whole directories for credentials stored as a tree.
    #[test]
    fn hydrate_copies_directories() {
        let repo = tmp("dir-repo");
        write(&repo, "config/credentials/production.key", "key");
        let wt = tmp("dir-wt");
        hydrate(&wt, &repo, &["config/credentials".to_string()]);
        assert_eq!(
            std::fs::read_to_string(wt.join("config/credentials/production.key")).unwrap(),
            "key"
        );
    }

    /// Inherited clone settings must resolve copy declarations even when settings and secrets are
    /// ignored by Git.
    #[test]
    fn copy_configuration_follows_the_inherited_file() {
        let repo = tmp("copia-herda-repo");
        write(
            &repo,
            ".prometeu/settings.toml",
            "[scripts]\nrun = \"x\"\n\n[worktree]\ncopy = [\"secret\"]\n",
        );
        write(&repo, "secret", "s");
        write(&repo, ".env", "undeclared");
        let wt = tmp("copia-herda-wt");

        let s = read_for(&wt, &repo);
        assert!(s.inherited);
        // An explicit copy list excludes unlisted automatic .env files.
        assert_eq!(s.copy, vec!["secret".to_string()]);
    }

    /// An empty copy list disables copying rather than falling back to automatic discovery.
    #[test]
    fn empty_copy_disables_automatic_copying() {
        let repo = tmp("vazia-repo");
        write(&repo, ".prometeu/settings.toml", "[worktree]\ncopy = []\n");
        write(&repo, ".env", "x");
        let wt = tmp("vazia-wt");
        assert!(read_for(&wt, &repo).copy.is_empty());
    }

    /// Skip an entire reserved range if any port is browser-blocked, including ports reached by
    /// PORT+n.
    #[test]
    fn port_skips_values_blocked_by_browsers() {
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
    fn port_is_derived_from_the_worktree_path() {
        let a = Path::new("/Users/ana/prometeu/worktrees/app/feat-a");
        let b = Path::new("/Users/ana/prometeu/worktrees/app/feat-b");
        assert_ne!(port_start(a), port_start(b));
        for p in [a, b] {
            assert!(port_start(p) < SLOTS, "{}", port_start(p));
        }
    }

    /// The `[tools]` table parses into the layered Selections; an absent axis inherits.
    #[test]
    fn tools_table_becomes_the_project_layer() {
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
    fn inherited_clone_tools_come_only_from_the_primary_repository() {
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

    /// `[method] artifacts` is read like the other tables, inherited from the clone, and normalized;
    /// an absent or escaping path yields no path at all (ADR 0057).
    #[test]
    fn method_artifacts_are_inherited_normalized_and_never_invented() {
        let repo = tmp("method-repo");
        write(
            &repo,
            ".prometeu/settings.toml",
            "[scripts]\nrun = \"x\"\n\n[method]\nartifacts = \"./docs/specs/\"\n",
        );
        let wt = tmp("method-wt");
        let inherited = read_for(&wt, &repo);
        assert!(inherited.inherited);
        assert_eq!(inherited.artifacts.as_deref(), Some("docs/specs"));

        let plain = tmp("method-plain");
        write(
            &plain,
            ".prometeu/settings.toml",
            "[scripts]\nrun = \"x\"\n",
        );
        assert_eq!(read(&plain).artifacts, None);
        assert_eq!(read(&tmp("method-none")).artifacts, None);
        for bad in ["\"../outside\"", "\"/abs/path\"", "\"  \"", "3"] {
            let dir = tmp("method-bad");
            write(
                &dir,
                ".prometeu/settings.toml",
                &format!("[method]\nartifacts = {bad}\n"),
            );
            assert_eq!(read(&dir).artifacts, None, "{bad}");
        }
    }
}
