use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    dirs::home_dir().expect("sem HOME")
}

/// O que separa o app de dev do app instalado, em todo caminho que o Prometeu
/// escreve. Sem isto os dois mexem no mesmo quadro e nos mesmos worktrees.
///
/// `cfg!` resolve em tempo de compilação: `tauri dev` compila em debug, `tauri
/// build` em release. Nada para configurar.
fn suffix() -> &'static str {
    if cfg!(debug_assertions) {
        "-dev"
    } else {
        ""
    }
}

/// Raiz de tudo que o Prometeu escreve fora do repositório do usuário.
pub fn root() -> PathBuf {
    if let Ok(p) = std::env::var("PROMETEU_ROOT") {
        return PathBuf::from(p);
    }
    home().join(format!(".prometeu{}", suffix()))
}

/// O time de que este app faz parte, com o segredo — só o dono lê. Quem fala
/// com o relay é o front; o back só guarda isto fora do `localStorage`.
pub fn team_path() -> PathBuf {
    root().join("team.json")
}

/// Diretórios que guardam estado, transcript e credenciais não são parte do
/// workspace compartilhável. `create_dir_all` respeita umask e pode deixá-los
/// `0755`; reafirmar `0700` torna a regra independente da máquina.
pub fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Grava um arquivo que só o dono lê: nasce `0600`, e é reescrito inteiro —
/// nunca truncado e preenchido, para não haver um instante com ele vazio. O
/// erro é a causa crua; quem chama embrulha no código da sua tela.
pub fn write_private(target: &Path, body: &str) -> Result<(), String> {
    write_private_bytes(target, body.as_bytes())
}

/// A mesma gravação privada e atômica para dados que não queremos
/// transformar em `String` no caminho. A importação usa isto para preservar
/// snapshots e transcripts byte a byte.
pub fn write_private_bytes(target: &Path, body: &[u8]) -> Result<(), String> {
    use std::io::Write;
    if let Some(dir) = target.parent() {
        ensure_private_dir(dir)?;
    }
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("private");
    let tmp = target.with_file_name(format!(".{filename}.{}.tmp", uuid::Uuid::new_v4()));
    let mut opts = std::fs::OpenOptions::new();
    // `create_new` também recusa um symlink que apareça no nome temporário:
    // não há arquivo anterior que precisemos truncar, e colisão deve falhar.
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
    if let Err(error) = file.write_all(body).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    drop(file);
    if let Err(error) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        if let Some(dir) = target.parent() {
            std::fs::File::open(dir)
                .and_then(|directory| directory.sync_all())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Worktrees ficam fora de `.prometeu` porque o usuário abre esses diretórios no editor.
///
/// O sufixo também vale aqui: dois apps criando worktree para a mesma branch do
/// mesmo repo colidiriam no mesmo diretório — e o transcript, que o Claude Code
/// nomeia pelo caminho do cwd, seria o mesmo arquivo para as duas sessões.
pub fn worktree_dir(repo_name: &str, branch: &str) -> PathBuf {
    home()
        .join("prometeu")
        .join(format!("worktrees{}", suffix()))
        .join(repo_name)
        .join(dir_name(branch))
}

/// A pasta de um workspace com mais de um repositório: os nomes deles juntos
/// no lugar do nome de um só, e dentro dela um worktree por repo, cada um com
/// o nome do clone. `capim-backend+capim-portal/feat-x/capim-backend` não
/// colide com o `capim-backend/feat-x` de um workspace de um repo só, e lê-se
/// no Finder o que é.
pub fn multi_dir(names: &[String], branch: &str) -> PathBuf {
    home()
        .join("prometeu")
        .join(format!("worktrees{}", suffix()))
        .join(names.join("+"))
        .join(dir_name(branch))
}

/// A raiz exata que a versão instalada do Prometheus usava. Só entra na
/// validação de workspaces importados: o Prometeu nunca cria nada aqui.
pub(crate) fn prometheus_multi_dir(names: &[String], branch: &str) -> PathBuf {
    home()
        .join("prometheus")
        .join("worktrees")
        .join(names.join("+"))
        .join(dir_name(branch))
}

/// O nome da pasta de uma branch. Trocar `/` por `-` é o que dá nome legível,
/// mas sozinho ele colide: `feat/x` e `feat-x` viravam a mesma pasta, e a
/// segunda sessão pegava silenciosamente o worktree da primeira — na branch
/// errada, com o quadro mentindo qual era.
///
/// Quando a troca acontece, o nome ganha um sufixo tirado da branch inteira.
/// Nome sem `/` continua exatamente como era, que é o caso comum.
fn dir_name(branch: &str) -> String {
    let flat = branch.replace('/', "-");
    match flat == branch {
        true => flat,
        false => format!("{flat}-{:06x}", fnv1a(branch) & 0xff_ffff),
    }
}

/// FNV-1a. Não precisa ser criptográfico — precisa ser estável entre execuções
/// (o caminho fica gravado no quadro) e não valer uma dependência nova. A
/// porta do worktree sai da mesma conta (ver `scripts::alloc_port`).
pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in s.bytes() {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Onde o Claude Code guarda o transcript de uma sessão: ele troca no caminho do
/// cwd tudo que não é letra ou número por `-` e usa isso como nome da pasta.
///
/// O arquivo só nasce na primeira mensagem. Conversa criada e nunca usada não
/// tem transcript nenhum — e é exatamente isso que o `--resume` responde com
/// "No conversation found with session ID".
/// A conversa de uma aba do Codex, nas mesmas linhas que a tela desenha. O
/// Codex guarda o rollout dele em `~/.codex/sessions`, num formato que é dele;
/// o que o app precisa amanhã é o que mostrou hoje — então grava o que
/// traduziu (`codex.rs`), e é daqui que a aba reabre. Fica na raiz do app, e
/// não no worktree, pelo mesmo motivo do transcript do Claude Code: apagar o
/// worktree não apaga a conversa.
pub fn chat_log(id: &str) -> PathBuf {
    root().join("chats").join(format!("{id}.jsonl"))
}

pub fn transcript(id: &str, cwd: &Path) -> PathBuf {
    transcript_at(&home(), id, cwd)
}

/// Variante injetável para a prévia da importação e seus testes. O Claude
/// continua sendo dono do arquivo; apenas calculamos onde ele o guardou.
pub(crate) fn transcript_at(home: &Path, id: &str, cwd: &Path) -> PathBuf {
    let slug: String = cwd
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    home.join(".claude/projects")
        .join(slug)
        .join(format!("{id}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_troca_tudo_que_nao_e_alfanumerico() {
        let path = transcript("abc", Path::new("/Users/ana/.prometeu/wt/x_1"));
        assert!(
            path.ends_with("-Users-ana--prometeu-wt-x-1/abc.jsonl"),
            "{}",
            path.display()
        );
    }

    /// A pasta do workspace de dois repos fica ao lado das de um só, com os
    /// dois nomes — e cada repo dentro dela com o seu.
    #[test]
    fn pasta_de_varios_repos_junta_os_nomes() {
        let dir = multi_dir(&["back".into(), "front".into()], "feat/x");
        let one = worktree_dir("back", "feat/x");
        assert_eq!(dir.parent().unwrap().file_name().unwrap(), "back+front");
        assert_eq!(dir.file_name(), one.file_name());
        assert_eq!(
            dir.parent().unwrap().parent(),
            one.parent().unwrap().parent()
        );
    }

    /// Branch sem `/` mantém o nome; com `/`, o nome achatado nunca é o mesmo
    /// de uma branch que já se chamava assim.
    #[test]
    fn branch_com_barra_nao_colide_com_a_achatada() {
        assert_eq!(dir_name("feat-x"), "feat-x");
        assert_ne!(dir_name("feat/x"), dir_name("feat-x"));
        assert!(
            dir_name("feat/x").starts_with("feat-x-"),
            "{}",
            dir_name("feat/x")
        );
        // Estável: o caminho fica gravado no quadro e tem de continuar valendo.
        assert_eq!(dir_name("feat/x"), dir_name("feat/x"));
        assert_ne!(dir_name("a/b"), dir_name("a/c"));
    }

    #[cfg(unix)]
    #[test]
    fn estado_privado_nasce_com_permissoes_restritas_e_troca_atomicamente() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("prometeu-private-{}", uuid::Uuid::new_v4()));
        let file = root.join("nested/state.json");
        write_private(&file, "primeiro").unwrap();
        write_private(&file, "segundo").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "segundo");
        assert_eq!(
            std::fs::metadata(file.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::read_dir(file.parent().unwrap()).unwrap().count(),
            1
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
