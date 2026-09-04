use crate::domain::Pr;
use crate::lock::lock;
use crate::state::{publish, Board, Choice, Project, ProviderId, Repo, Status, Tab, Workspace};
use crate::{chat, dock, i18n, paths, scripts, AppState};
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager, State};

pub(crate) mod diff;
pub(crate) mod files;
pub(crate) mod find;

use diff::{ahead_of, changes_in};
#[cfg(test)]
use diff::{patch_map, repo_diff};

#[tauri::command]
pub fn load_board(state: State<AppState>) -> Board {
    lock(&state.board).clone()
}

/* ---------- projetos ---------- */

/// Registrar o repositório uma vez é o que torna criar workspace rápido depois:
/// o lançador vira um seletor e uma caixa de texto.
#[tauri::command]
pub fn add_project(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<Project, String> {
    let path = PathBuf::from(expand(&path));
    if !path.join(".git").exists() {
        return Err(i18n::ta(
            "err.session.notGit",
            &[("path", path.display().to_string())],
        ));
    }
    let id = path.display().to_string();
    let project = Project {
        id: id.clone(),
        name: path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("repo")
            .to_string(),
        path: id.clone(),
    };
    {
        let mut board = lock(&state.board);
        if !board.projects.iter().any(|p| p.id == id) {
            board.projects.push(project.clone());
        }
    }
    publish(&app);
    Ok(project)
}

#[tauri::command]
pub fn remove_project(app: AppHandle, state: State<AppState>, id: String) {
    {
        let mut board = lock(&state.board);
        board.projects.retain(|p| p.id != id);
    }
    publish(&app);
}

/* ---------- workspaces ---------- */

/// A etapa é propriedade do workspace, não o lugar onde ele está: muda pelo
/// menu, pelo cabeçalho ou arrastando o card — dá no mesmo.
#[tauri::command]
pub fn set_stage(app: AppHandle, state: State<AppState>, id: String, stage: String) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.stage = stage;
        }
    }
    publish(&app);
}

/// Arquivar é sair da lista, não morrer: worktree, branch e transcript ficam, e
/// desarquivar traz tudo de volta. Os processos, esses, param — agente vivo num
/// workspace que ninguém vê é pergunta esperando resposta que ninguém lê.
#[tauri::command]
pub fn archive_workspace(app: AppHandle, state: State<AppState>, id: String, archived: bool) {
    archive(&state, &id, archived);
    publish(&app);
}

/// Concluir: a etapa vai para a última da lista e o workspace sai da frente,
/// num gesto só. São os dois que sempre andavam juntos quando o PR entrava —
/// e arquivar já derruba o agente, os docks e o que o script `archive` tiver
/// para derrubar. O worktree fica: devolver o disco é outra decisão, tomada
/// depois e com o diff ainda ao alcance.
#[tauri::command]
pub fn finish_workspace(app: AppHandle, state: State<AppState>, id: String) {
    {
        let mut board = lock(&state.board);
        let last = board.stages.last().cloned();
        if let (Some(stage), Some(ws)) = (last, board.workspace_mut(&id)) {
            ws.stage = stage;
        }
    }
    archive(&state, &id, true);
    publish(&app);
}

fn archive(state: &State<AppState>, id: &str, archived: bool) {
    let mut dead: Vec<String> = Vec::new();
    // O `archive` derruba o que o workspace deixou fora do worktree — container,
    // banco, túnel. Roda antes de arquivar, enquanto o que ele precisa apagar
    // ainda existe, e solto: é limpeza, e prender a janela nela seria pior do
    // que ela demorar. Sem pty, porque ninguém vai ler a saída.
    if archived {
        // Os docks caem primeiro: o `archive` não pode derrubar o banco com o
        // servidor de dev ainda de pé em cima dele — e servidor de workspace
        // arquivado é processo que ninguém vê.
        dock::kill_docks(state, id);
        if let Some(ws) = workspace_copy(state, id) {
            if let Some(command) = dock::scripts_of(&ws).archive {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-lc", &command])
                    .current_dir(&ws.primary().worktree);
                for (key, value) in dock::script_env(&ws) {
                    cmd.env(key, value);
                }
                let _ = cmd.spawn();
            }
        }
    }
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(id) {
            ws.archived = archived;
            if archived {
                dead = ws.tabs.iter().map(|t| t.id.clone()).collect();
                for tab in &mut ws.tabs {
                    tab.status = Status::Desligada;
                    tab.note = None;
                }
            }
        }
    }
    // Fora do lock do quadro: encerrar é sinalizar e esperar, e isso com o
    // quadro trancado pararia as outras sessões.
    stop(state, &dead);
}

/// Encerra as sessões destas abas: o processo morre. Transcript e worktree
/// ficam — a próxima fala retoma.
fn stop(state: &State<AppState>, tabs: &[String]) {
    for tab in tabs {
        chat::kill(state, tab);
    }
}

/// O nome nasce da primeira frase do prompt, que quase nunca é o nome que o
/// trabalho tem no fim. Nome vazio é desistência, não apagar o que já existe.
#[tauri::command]
pub fn rename_workspace(app: AppHandle, state: State<AppState>, id: String, title: String) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.title = title.to_string();
        }
    }
    publish(&app);
}

/// Fixar é a etiqueta de "é neste que eu volto agora" — sobe para o topo da
/// lista sem mentir sobre a etapa em que o trabalho está.
#[tauri::command]
pub fn pin_workspace(app: AppHandle, state: State<AppState>, id: String, pinned: bool) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.pinned = pinned;
        }
    }
    publish(&app);
}

/// Trocar as ferramentas do agente com o trabalho já andando: marcar o Notion
/// no meio da tarde, porque só agora deu para ver que vai precisar dele.
///
/// O MCP entra na sessão quando ela sobe, e não há como acrescentar um a um
/// processo de pé. Mas a sessão não é o processo — é o transcript no disco
/// (`chat.rs`) —, então derrubar o processo aqui não perde conversa nenhuma: a
/// próxima fala o levanta de novo com `--resume` e a lista nova. Quem está no
/// meio de um turno fica de fora: a tela não deixa marcar enquanto o agente
/// trabalha, e derrubá-lo aqui jogaria o turno fora.
#[tauri::command]
pub fn set_workspace_mcp(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    mcp: Option<Vec<String>>,
) {
    let tabs = {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(&id) else {
            return;
        };
        ws.mcp = mcp;
        ws.tabs.iter().map(|t| t.id.clone()).collect::<Vec<_>>()
    };
    for tab in tabs {
        chat::kill(&state, &tab);
    }
    publish(&app);
}

/// Trocar os plugins do workspace, pela mesma regra do MCP: eles entram quando
/// a sessão sobe, então as abas caem aqui e a próxima fala as levanta de novo
/// com a lista nova. O que se perde é o processo, não a conversa.
#[tauri::command]
pub fn set_workspace_plugins(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    plugins: Option<Vec<String>>,
) {
    let tabs = {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(&id) else {
            return;
        };
        ws.plugins = plugins;
        ws.tabs.iter().map(|t| t.id.clone()).collect::<Vec<_>>()
    };
    for tab in tabs {
        chat::kill(&state, &tab);
    }
    publish(&app);
}

/// Trocar com quem uma conversa que já começou fala: o modelo e o esforço
/// desta aba, pela mesma regra do MCP e dos plugins. As flags entram quando o
/// processo sobe, então ele cai aqui e a próxima fala o levanta com `--resume`
/// e as novas — o transcript é o mesmo, e o que muda é quem o lê daqui para a
/// frente.
///
/// Trocar de CLI no meio não: o transcript do Claude Code o Codex não retoma,
/// nem o contrário. Quem barra é a tela, que só oferece os modelos do CLI que
/// já está de pé; aqui a conta é refeita porque comando é porta de entrada.
#[tauri::command]
pub fn set_tab_choice(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    tab: String,
    choice: Choice,
) -> Result<(), String> {
    {
        let mut board = lock(&state.board);
        let ws = board
            .workspace_mut(&id)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        ws.retune(&tab, choice)?;
    }
    chat::kill(&state, &tab);
    publish(&app);
    Ok(())
}

/// Marcar como não lido à mão: dar de cara com a novidade e não poder lidar com
/// ela agora é o caso mais comum de todos.
#[tauri::command]
pub fn set_unread(app: AppHandle, state: State<AppState>, id: String, unread: bool) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.unread = unread;
        }
    }
    publish(&app);
}

/// Compartilhar com o time é uma marca no workspace: quem anuncia ao relay e
/// repassa a saída é o front, que é quem tem os bytes. Fica gravada para o
/// dono que fecha o app voltar compartilhando sozinho.
#[tauri::command]
pub fn set_shared(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    shared: bool,
    audience: Option<Vec<String>>,
) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.shared = shared;
            ws.audience = if shared { audience } else { None };
        }
    }
    publish(&app);
}

/// Qual workspace está na tela — e, por isso, deixa de ter novidade. Sem isto o
/// back marcaria como não lido o que você está vendo acontecer na sua frente.
#[tauri::command]
pub fn look_at(app: AppHandle, state: State<AppState>, id: Option<String>) {
    *lock(&state.looking) = id.clone();
    let Some(id) = id else { return };
    let had = {
        let mut board = lock(&state.board);
        match board.workspace_mut(&id) {
            Some(ws) => std::mem::replace(&mut ws.unread, false),
            None => false,
        }
    };
    if had {
        publish(&app);
    }
}

/// Tira o workspace do quadro. Não mexe no worktree nem na branch de propósito:
/// apagar trabalho é decisão sua, feita no git, não num clique de limpeza.
#[tauri::command]
pub fn remove_workspace(app: AppHandle, state: State<AppState>, id: String) {
    dock::kill_docks(&state, &id);
    let (dead, removed): (Vec<String>, bool) = {
        let mut board = lock(&state.board);
        let removed = board.workspaces.iter().any(|ws| ws.id == id);
        let dead = board
            .workspace_mut(&id)
            .map(|ws| ws.tabs.iter().map(|t| t.id.clone()).collect())
            .unwrap_or_default();
        board.workspaces.retain(|w| w.id != id);
        (dead, removed)
    };
    stop(&state, &dead);
    if removed {
        crate::plugins::forget_codex_workspace(&id);
    }
    publish(&app);
}

/* ---------- devolver o disco ---------- */

/// Um worktree que já pode sair do disco, e o que ele ocupa. `blocked` é o
/// motivo de não poder — mudança fora de commit, trabalho que não entrou no
/// alvo — e vem como código para a tela traduzir.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleanable {
    pub id: String,
    pub title: String,
    pub repo_name: String,
    pub branch: String,
    pub worktree: String,
    /// Quanto o worktree ocupa, em kilobytes. `node_modules` e `target` são a
    /// maior parte disso, e é por eles que a limpeza vale a pena.
    pub size_kb: u64,
    pub pr: Option<u64>,
    pub blocked: Option<String>,
}

/// Os arquivados que ainda têm worktree, com o motivo de cada um poder ou não
/// sair. Uma varredura só, pedida quando a tela de limpeza abre: cada linha
/// custa um `git status` e um `du`, e isso não é coisa para o redesenho do
/// quadro fazer.
///
/// Workspace que roda no próprio clone fica de fora: não há pasta para
/// devolver, e medi-lo seria um `du` do repositório inteiro por linha — era
/// isso que fazia a lista demorar. As linhas que sobram são medidas em
/// paralelo: cada `du` anda numa árvore diferente, e o disco aguenta.
#[tauri::command(async)]
pub fn cleanup_list(state: State<AppState>) -> Vec<Cleanable> {
    let mine: Vec<Workspace> = lock(&state.board)
        .workspaces
        .iter()
        .filter(|w| has_worktree(w))
        .cloned()
        .collect();

    std::thread::scope(|scope| {
        let handles: Vec<_> = mine
            .into_iter()
            .map(|ws| {
                scope.spawn(move || {
                    let wt = PathBuf::from(&ws.worktree);
                    let pr = ws.prs().next().map(|(_, p)| p.number);
                    Cleanable {
                        size_kb: size_of(&wt),
                        blocked: check(&ws).err(),
                        pr,
                        id: ws.id,
                        title: ws.title,
                        repo_name: ws.repo_name,
                        branch: ws.branch,
                        worktree: ws.worktree,
                    }
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// Arquivado que ainda tem um worktree só dele para devolver. O que já foi
/// devolvido não conta, e o que roda no próprio clone nunca contou: a pasta
/// é o repositório.
fn has_worktree(ws: &Workspace) -> bool {
    ws.archived && !ws.cleaned && ws.worktree != ws.repo
}

/// Devolve o worktree ao disco: a pasta sai, a branch local sai, o card fica.
/// Destrutivo e sem volta.
///
/// `force` é a tela dizendo que a pessoa leu o motivo em vermelho e marcou
/// assim mesmo — mudança fora de commit e trabalho que não entrou no alvo vão
/// junto. O que `force` não desliga é o que nem a pessoa quer: arquivar antes,
/// e nunca apagar o próprio clone.
#[tauri::command(async)]
pub fn cleanup_worktree(
    app: AppHandle,
    state: State<AppState>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let ws = workspace_copy(&state, &id).ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    if ws.cleaned {
        return Ok(());
    }
    if force {
        hard(&ws)?;
    } else {
        check(&ws)?;
    }
    if ws.multi() {
        validate_multi_root(&ws)?;
    }

    // O agente e os docks caem antes de a pasta sumir debaixo deles. O script
    // `archive` do repositório não roda aqui: ele já rodou quando este
    // workspace foi arquivado, e ele sobe solto — dispará-lo agora seria soltar
    // um processo no worktree ao mesmo tempo que o git o apaga.
    dock::kill_docks(&state, &id);
    let dead: Vec<String> = lock(&state.board)
        .workspace_mut(&id)
        .map(|ws| ws.tabs.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default();
    stop(&state, &dead);

    // Um worktree por repositório, e cada um sai do seu clone.
    for r in &ws.repos {
        let repo = PathBuf::from(&r.path);
        let wt = PathBuf::from(&r.worktree);
        if wt.exists() {
            // `--force` porque o que sobrou é o que o `.gitignore` esconde:
            // `node_modules`, `target`, `.env` — e, quando a pessoa marcou o
            // vermelho, também a mudança fora de commit que ela decidiu perder.
            let out = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["worktree", "remove", "--force"])
                .arg(&wt)
                .output()
                .map_err(|e| i18n::ta("err.git.spawn", &[("cause", e.to_string())]))?;
            if !out.status.success() {
                return Err(i18n::ta(
                    "err.git",
                    &[
                        ("command", "git worktree remove".into()),
                        (
                            "cause",
                            String::from_utf8_lossy(&out.stderr).trim().to_string(),
                        ),
                    ],
                ));
            }
        }
        // `-D` e não `-d`: sem `force` a branch já não tem nada que o alvo não
        // tenha, e com `force` perdê-la é justamente o que foi marcado. Se o git
        // recusar — ela está em check-out em outro lugar —, o worktree já foi e o
        // trabalho aqui está feito: uma branch a mais no repositório não é motivo
        // para devolver erro a quem só queria o disco de volta.
        if !ws.branch.is_empty() {
            let _ = git(&repo, &["branch", "-D", &ws.branch]);
        }
        let _ = git(&repo, &["worktree", "prune"]);
    }
    // A pasta que reunia os worktrees é do Prometeu: sem eles, só sobra o
    // que o app escreveu nela, e ela vai junto.
    if ws.multi() {
        let _ = std::fs::remove_dir_all(&ws.worktree);
    }
    crate::plugins::forget_codex_workspace(&id);

    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&id) {
            ws.cleaned = true;
            for tab in &mut ws.tabs {
                tab.status = Status::Desligada;
                tab.note = None;
            }
        }
    }
    publish(&app);
    Ok(())
}

/// Este worktree pode sair? O erro é o motivo, como código para a tela dizer a
/// frase. Worktree que já sumiu do disco passa: limpar o que não existe mais é
/// só acertar o quadro.
fn check(ws: &Workspace) -> Result<(), String> {
    hard(ws)?;
    // Cada repositório responde por si, e basta um segurar para nenhum sair:
    // os worktrees são de um trabalho só, e devolver metade dele não é limpar.
    for r in &ws.repos {
        let wt = PathBuf::from(&r.worktree);
        if !wt.exists() {
            continue;
        }
        let dirty = git(&wt, &["status", "--porcelain"]).lines().count();
        if dirty > 0 {
            return Err(i18n::ta("err.cleanup.dirty", &[("n", dirty.to_string())]));
        }
        if !merged(r.pr.as_ref(), &wt) {
            return Err(i18n::ta(
                "err.cleanup.unmerged",
                &[("branch", ws.branch.clone())],
            ));
        }
    }
    Ok(())
}

/// As duas guardas que `force` não levanta: nem a pessoa mais decidida quer
/// apagar um worktree que ainda está em uso, nem o clone dela.
fn hard(ws: &Workspace) -> Result<(), String> {
    // Arquivar primeiro é o que faz o `archive` do repositório rodar com o
    // worktree ainda de pé. Devolver o disco é o passo depois dele, nunca no
    // lugar dele.
    if !ws.archived {
        return Err(i18n::t("err.cleanup.notArchived"));
    }
    if ws.worktree == ws.repo {
        return Err(i18n::t("err.cleanup.isRepo"));
    }
    Ok(())
}

/// A raiz agregadora é a única pasta que removemos diretamente; os worktrees
/// individuais são removidos pelo próprio Git. Por isso o caminho precisa ser
/// exatamente o que `create_workspace` teria produzido, e cada filho precisa
/// estar imediatamente abaixo dele. Um `board.json` editado ou corrompido não
/// pode transformar `remove_dir_all` numa remoção de pasta arbitrária.
fn validate_multi_root(ws: &Workspace) -> Result<(), String> {
    use std::path::Component;

    let normal = |name: &str| {
        let mut components = Path::new(name).components();
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
    };
    if ws.repos.len() < 2 || ws.repos.iter().any(|repo| !normal(&repo.name)) {
        return Err(i18n::t("err.cleanup.badRoot"));
    }
    let names: Vec<String> = ws.repos.iter().map(|repo| repo.name.clone()).collect();
    let expected = paths::multi_dir(&names, &ws.branch);
    // A importação preserva os worktrees no lugar para não copiar dezenas
    // de gigabytes nem alterar a origem. A raiz antiga passa pela mesma conta
    // exata; nenhum outro caminho ganha permissão para `remove_dir_all`.
    let legacy = paths::prometheus_multi_dir(&names, &ws.branch);
    let root = Path::new(&ws.worktree);
    let children_match = ws.repos.iter().all(|repo| {
        let child = Path::new(&repo.worktree);
        child.parent() == Some(root) && child.file_name() == Some(repo.name.as_ref())
    });
    if (root == expected || root == legacy) && children_match {
        Ok(())
    } else {
        Err(i18n::t("err.cleanup.badRoot"))
    }
}

/// O trabalho já está em outro lugar? Duas respostas servem: o `gh` dizendo que
/// o PR mergeou, ou o git dizendo que o que está aqui já é ancestral do alvo —
/// que é o que sobra quando o merge foi por fora do GitHub, ou o `gh` não
/// existe nesta máquina.
fn merged(pr: Option<&Pr>, wt: &Path) -> bool {
    if pr.is_some_and(|pr| pr.merged()) {
        return true;
    }
    let head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let target = if head.is_empty() {
        "origin/main".to_string()
    } else {
        head
    };
    has_commit(wt, &target) && git_ok(wt, &["merge-base", "--is-ancestor", "HEAD", &target])
}

/// Quanto a pasta ocupa, em kilobytes — o `du` do sistema, que é quem já sabe
/// andar em árvore grande. Sem resposta, zero: o número é para você decidir se
/// vale a pena, e não saber o tamanho não impede a limpeza.
fn size_of(wt: &Path) -> u64 {
    if !wt.exists() {
        return 0;
    }
    let out = Command::new("du").arg("-sk").arg(wt).output().ok();
    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
    })
    .unwrap_or(0)
}

/// O que o lançador montou. Um struct, e não doze parâmetros soltos: o front já
/// tem esse objeto inteiro, e passá-lo como um só é o que impede a lista de
/// argumentos de crescer a cada chavinha nova na tela.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    project: String,
    /// Os outros repositórios do workspace, quando a funcionalidade atravessa
    /// mais de um: cada um ganha um worktree na mesma branch, lado a lado com
    /// o do `project`. Só existe com `worktree` ligado — o agente precisa de
    /// uma pasta que contenha todos, e clones espalhados não têm uma.
    #[serde(default)]
    extras: Vec<String>,
    /// Vazia é a escolha de não criar branch nenhuma.
    branch: String,
    /// De onde a branch nova sai.
    base: String,
    /// Ligado, a branch nasce num worktree só dela; desligado, ela nasce no
    /// próprio repositório — e é o diretório de trabalho dele que troca.
    worktree: bool,
    title: String,
    stage: String,
    prompt: String,
    inject: Vec<String>,
    /// A issue do Linear que deu origem, quando o lançador saiu de uma.
    #[serde(default)]
    issue: Option<crate::linear::IssueRef>,
    /// `--model`, `--effort` e plan mode da primeira conversa. Modelo e
    /// esforço ficam no workspace; plan mode é só desta primeira fala.
    #[serde(flatten)]
    launch: Launch,
}

/// As chavinhas que viram argumento do `claude`. O que o lançador escolhe e o
/// que o workspace guarda para as próximas conversas são o mesmo conjunto.
#[derive(serde::Deserialize, Clone, Default)]
pub struct Launch {
    /// Qual CLI sobe na aba. Sai do catálogo do modelo escolhido no lançador,
    /// e não de um botão à parte.
    #[serde(default)]
    pub agent: ProviderId,
    /// Vazio é não passar `--model`: o Claude Code escolhe.
    #[serde(default)]
    pub model: String,
    /// Vazio é não passar `--effort`.
    #[serde(default)]
    pub effort: String,
    /// Nasce em plan mode: o agente lê e planeja, e aprovar o plano é o que o
    /// solta. Só vale para a conversa que o lançador abre.
    #[serde(default)]
    pub plan: bool,
    /// Os servidores de MCP que esta conversa enxerga, pelo nome no hub.
    /// `None` é não impor nada ao CLI — ver `Workspace::mcp` e `mcp.rs`.
    #[serde(default)]
    pub mcp: Option<Vec<String>>,
    /// Os plugins do Claude Code que esta conversa carrega, pelo nome no hub.
    /// `None` é não impor nada — ver `Workspace::plugins` e `plugins.rs`.
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
}

/// A escolha gravada na aba vira argumento do mesmo jeito que a do lançador —
/// e nunca em plan mode: plan é de uma fala, não de uma conversa inteira.
impl From<Choice> for Launch {
    fn from(c: Choice) -> Self {
        Launch {
            agent: c.agent,
            model: c.model,
            effort: c.effort,
            plan: false,
            mcp: None,
            plugins: None,
        }
    }
}

impl Workspace {
    /// Com o que uma conversa nasce aqui quando ninguém escolheu outra coisa: o
    /// modelo e o esforço do workspace, e nunca em plan mode — isso é escolha
    /// do lançador.
    pub fn launch(&self) -> Launch {
        Launch {
            agent: self.agent,
            model: self.model.clone(),
            effort: self.effort.clone(),
            plan: false,
            mcp: self.mcp.clone(),
            plugins: self.plugins.clone(),
        }
    }

    /// Com o que uma aba sobe: o modelo que ela escolheu, ou o do workspace.
    /// É o que separa retomar de recomeçar — a aba que nasceu no Sonnet volta
    /// no Sonnet, mesmo que as irmãs sejam de outro modelo.
    /// O MCP não entra na conta da aba: a aba escolhe com quem fala, o
    /// workspace escolhe o que o agente tem na mão. Aba do Sonnet e aba do Opus
    /// no mesmo worktree veem os mesmos servidores — e desmarcar um vale para
    /// as duas na próxima vez que subirem.
    pub fn launch_of(&self, tab: &str) -> Launch {
        self.tabs
            .iter()
            .find(|t| t.id == tab)
            .and_then(|t| t.choice.clone())
            .map_or_else(
                || self.launch(),
                |choice| Launch {
                    mcp: self.mcp.clone(),
                    plugins: self.plugins.clone(),
                    ..Launch::from(choice)
                },
            )
    }

    /// Troca o modelo e o esforço de uma conversa de pé. O que fica gravado é a
    /// escolha da aba (`Tab::choice`), que é onde já morava "esta conversa fala
    /// com outro" — e escolher de volta o do workspace apaga a escolha, para
    /// que a aba volte a acompanhar o workspace em vez de congelar o de hoje.
    ///
    /// Trocar de CLI é recusado: o `--resume` do Claude Code não abre a thread
    /// do Codex, e o `thread/resume` do Codex não abre o transcript do Claude.
    /// Falar com um GPT numa conversa do Claude é abrir aba nova.
    pub fn retune(&mut self, tab: &str, choice: Choice) -> Result<(), String> {
        if self.launch_of(tab).agent != choice.agent {
            return Err(i18n::t("err.session.otherAgent"));
        }
        let follows = choice.agent == self.agent
            && choice.model == self.model
            && choice.effort == self.effort;
        let tab = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab)
            .ok_or_else(|| i18n::t("err.session.noTab"))?;
        tab.choice = (!follows).then_some(choice);
        Ok(())
    }
}

/// Criar é otimista: o card entra no quadro agora, e o que demora acontece
/// atrás.
///
/// O que demora é o disco — `git worktree add` de um repositório com milhares
/// de arquivos passa de um segundo, e o `fetch` da base soma rede a isso. Antes
/// tudo isso ficava entre o clique e a resposta, e o lançador fechava para uma
/// tela parada. Aqui só fica o que dá para saber sem tocar em disco: se o
/// caminho é um repositório, onde o worktree vai ficar, qual é a branch, qual é
/// a porta. Com isso o `Workspace` já é inteiro o bastante para desenhar, entra
/// no quadro marcado como `preparing`, e a resposta volta em milissegundos.
///
/// A montagem de verdade é `prepare`, numa thread, e cada etapa dela chega à
/// tela pelo `publish` — que é o mesmo caminho por onde toda mudança do quadro
/// já chegava.
#[tauri::command(async)]
pub fn create_workspace(
    app: AppHandle,
    state: State<AppState>,
    draft: Draft,
    cols: u16,
    rows: u16,
) -> Result<Workspace, String> {
    let repo_path = PathBuf::from(expand(&draft.project));
    let repo_name = repo_named(&repo_path)?;

    // Os outros repositórios, conferidos do mesmo jeito. Dois clones com a
    // mesma pasta de nome cairiam no mesmo worktree, e o principal repetido
    // seria o mesmo repo duas vezes: os dois são erro, não dedução.
    let mut extras: Vec<(PathBuf, String)> = Vec::new();
    for extra in &draft.extras {
        let path = PathBuf::from(expand(extra));
        let name = repo_named(&path)?;
        if path == repo_path || extras.iter().any(|(p, n)| *p == path || *n == name) {
            return Err(i18n::ta("err.session.dupRepo", &[("name", name)]));
        }
        extras.push((path, name));
    }
    if !extras.is_empty() && !draft.worktree {
        return Err(i18n::t("err.session.extrasNeedWorktree"));
    }

    // Branch vazia é a escolha de não criar branch nenhuma: a sessão abre no
    // repositório onde ele estiver. Worktree, esse, sempre precisa de uma —
    // é a branch que dá nome e destino à pasta.
    //
    // Os dois saem de conta, não de disco: o destino é função do nome do repo e
    // da branch, e a branch ou veio digitada ou é o HEAD do clone. Dá para
    // saber os dois antes de existir pasta nenhuma, e é isso que deixa o card
    // nascer já com o nome e o caminho certos.
    //
    // Com mais de um repositório, a raiz é uma pasta que reúne o worktree de
    // cada um: é nela que o agente roda, e é ela que a árvore mostra.
    let (root, branch) = match (draft.worktree, draft.branch.trim().is_empty()) {
        (true, true) => return Err(i18n::t("err.session.worktreeNeedsBranch")),
        (true, false) if extras.is_empty() => (
            paths::worktree_dir(&repo_name, &draft.branch),
            draft.branch.clone(),
        ),
        (true, false) => {
            let names: Vec<String> = std::iter::once(repo_name.clone())
                .chain(extras.iter().map(|(_, n)| n.clone()))
                .collect();
            (
                paths::multi_dir(&names, &draft.branch),
                draft.branch.clone(),
            )
        }
        (false, false) => (repo_path.clone(), draft.branch.clone()),
        (false, true) => (
            repo_path.clone(),
            head_branch(&repo_path).unwrap_or_else(|| "HEAD".into()),
        ),
    };
    // A base escolhida no lançador é do principal — a lista de branches era
    // dele. Nos outros, a branch nova sai do que cada clone tem como principal
    // (`origin/main`, ou o que o clone gravou). Fica gravada por repo: é contra
    // ela que a tela de mudanças conta o que esta branch tem.
    let repos: Vec<Repo> = match extras.is_empty() {
        true => vec![Repo {
            path: repo_path.display().to_string(),
            name: repo_name.clone(),
            worktree: root.display().to_string(),
            base: draft.base.clone(),
            pr: None,
        }],
        false => std::iter::once((repo_path.clone(), repo_name.clone(), draft.base.clone()))
            .chain(extras.into_iter().map(|(path, name)| {
                let base = default_base(&path);
                (path, name, base)
            }))
            .map(|(path, name, base)| Repo {
                worktree: root.join(&name).display().to_string(),
                path: path.display().to_string(),
                name,
                base,
                pr: None,
            })
            .collect(),
    };

    // A branch já aberta em outra pasta é o único "não" que dá para dar antes de
    // o card nascer, e é o que separa recusar de fracassar: o `git worktree add`
    // recusaria de qualquer jeito, mas lá atrás, com o card já no quadro e a
    // pessoa olhando para um workspace que nunca vai montar. Ler
    // `git worktree list` é ler metadado — não custa o segundo que fez a
    // montagem inteira mudar de thread.
    if draft.worktree {
        for r in &repos {
            branch_free(Path::new(&r.path), &branch, Path::new(&r.worktree))?;
        }
    }

    // A porta sai antes de qualquer script, porque é ela que o `setup` e o `run`
    // recebem no ambiente — e é o que deixa dois worktrees do mesmo projeto
    // subirem o servidor ao mesmo tempo sem um matar o outro.
    //
    // O lock sai antes dos binds: `alloc_port` é syscall, e é neste mesmo lock
    // que todo `publish` de toda sessão espera.
    let taken: Vec<u16> = lock(&state.board)
        .workspaces
        .iter()
        .filter_map(|w| w.port)
        .collect();
    let port = scripts::alloc_port(&root, &taken);

    let ws = Workspace {
        id: uuid::Uuid::new_v4().to_string(),
        title: if draft.title.trim().is_empty() {
            branch.clone()
        } else {
            draft.title.clone()
        },
        issue: draft.issue.clone(),
        project: repo_path.display().to_string(),
        repo: repo_path.display().to_string(),
        repo_name,
        branch,
        worktree: root.display().to_string(),
        repos,
        stage: draft.stage.clone(),
        archived: false,
        pinned: false,
        unread: false,
        pr: None,
        cleaned: false,
        shared: false,
        audience: None,
        preparing: true,
        failed: None,
        agent: draft.launch.agent,
        model: draft.launch.model.clone(),
        effort: draft.launch.effort.clone(),
        mcp: draft.launch.mcp.clone(),
        plugins: draft.launch.plugins.clone(),
        port,
        active: None,
        tabs: Vec::new(),
    };

    // O workspace entra no quadro antes de qualquer coisa subir: é no quadro
    // que o fim do setup vai procurar as abas com fala guardada — e é ele que a
    // tela abre enquanto o resto não chega.
    lock(&state.board).workspaces.push(ws.clone());
    publish(&app);

    // O nome que veio do lançador é a primeira linha do prompt cortada; o bom
    // vem de um agente lendo o pedido inteiro, em paralelo. Ele começa aqui, e
    // não depois de montar a pasta: nomear não depende do worktree, e esperar o
    // `git worktree add` de um repositório grande só para *começar* a pensar num
    // título é somar segundos que ninguém precisava esperar.
    // Workspace que saiu de uma issue já tem o nome que a issue deu.
    if ws.issue.is_none() {
        crate::naming::rename_later(&app, &ws.id, &draft.prompt, &ws.title, &draft.launch);
    }

    let (bg, id) = (app.clone(), ws.id.clone());
    std::thread::spawn(move || prepare(&bg, &id, draft, cols, rows));

    Ok(ws)
}

/// A parte demorada de criar um workspace, fora da thread que respondeu ao
/// lançador: montar a pasta, subir o agente, subir o setup.
///
/// Falhar aqui não desfaz nada e não apaga o card. Quem falha é quase sempre o
/// `git worktree add`, e quase sempre porque a branch pedida está viva em outro
/// worktree — desfazer sozinho apagaria a única pista disso. O erro fica
/// escrito no card, que é de onde se decide o que fazer com a branch.
fn prepare(app: &AppHandle, id: &str, draft: Draft, cols: u16, rows: u16) {
    let Err(err) = build(app, id, &draft, cols, rows) else {
        return;
    };
    let state = app.state::<AppState>();
    {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(id) else {
            return;
        };
        ws.preparing = false;
        ws.failed = Some(err);
    }
    publish(app);
}

fn build(app: &AppHandle, id: &str, draft: &Draft, cols: u16, rows: u16) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (repo, root, branch, repos) = {
        let board = lock(&state.board);
        let ws = board
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        (
            PathBuf::from(&ws.repo),
            PathBuf::from(&ws.worktree),
            ws.branch.clone(),
            ws.repos.clone(),
        )
    };

    // A pasta. É o segundo que se sentia ao criar, e é por isso que ele mora
    // aqui atrás e não entre o clique e a resposta.
    //
    // Com mais de um repositório é um worktree por repo, todos na mesma
    // branch, cada um saindo da base gravada nele. Branch que já existe no
    // repo ignora a base de qualquer jeito.
    if draft.worktree {
        // Meio workspace no disco é pior que nenhum: a pessoa veria a pasta de
        // um repositório só, e o card não teria como contar que está pela
        // metade. Recusou um, desfaz os que esta montagem tinha feito.
        let mut feitos: Vec<(PathBuf, PathBuf)> = Vec::new();
        for r in &repos {
            let (clone, dest) = (PathBuf::from(&r.path), PathBuf::from(&r.worktree));
            match add_worktree(&clone, &branch, &r.base, &dest) {
                Ok(true) => feitos.push((clone, dest)),
                Ok(false) => {}
                Err(e) => {
                    undo_worktrees(&feitos);
                    if repos.len() > 1 {
                        let _ = std::fs::remove_dir(&root);
                    }
                    return Err(e);
                }
            }
        }
        if repos.len() > 1 {
            describe_root(&root, &repos, &branch);
        }
    } else if !draft.branch.trim().is_empty() {
        switch_branch(&repo, &branch, &draft.base)?;
    }

    let tab = spawn_tab(
        app,
        &state,
        id,
        "conversa",
        first_message(&draft.prompt, &draft.inject),
        &draft.launch,
        // A primeira conversa é a do lançador, e é dela que o workspace copiou
        // o modelo: nada a gravar na aba.
        None,
    )?;

    // Tirado do quadro no meio da montagem: o agente que acabou de subir não
    // tem mais card nenhum a que pertencer, e deixá-lo vivo seria um `claude`
    // rodando num worktree que ninguém vê.
    let ws = {
        let mut board = lock(&state.board);
        let Some(ws) = board.workspace_mut(id) else {
            drop(board);
            chat::kill(&state, &tab.id);
            return Ok(());
        };
        ws.preparing = false;
        ws.active = Some(tab.id.clone());
        ws.tabs.push(tab);
        ws.clone()
    };
    publish(app);

    // Worktree recém-nascido não tem nada que o `.gitignore` esconde:
    // dependências, `.env`, banco, build. O que dá para reconstruir é o setup
    // que reconstrói; o que não dá — segredo, chave — vem copiado do clone,
    // antes dele. Os dois são a aba Setup. O processo do agente sobe junto,
    // mas a primeira fala só vai quando o setup termina (ver
    // `release_prompts`): agente que roda teste antes de haver `node_modules`
    // conclui coisa errada. Falhar aqui não desfaz o worktree; o erro fica
    // escrito na aba Setup, que é onde se conserta.
    let _ = dock::start_setup(app, &state, &ws, cols, rows);
    // Com o setup de pé a fala espera por ele; sem setup, vai agora.
    if let Some(tab) = ws.tabs.last() {
        chat::ready_now(app, &tab.id);
    }

    Ok(())
}

/* ---------- abas ---------- */

/// Conversa nova nos mesmos arquivos. É o ⌘T: quando o contexto encheu, ou
/// quando o assunto virou outro, mas o worktree é o mesmo.
///
/// `choice` é o modelo escolhido na setinha ao lado do "+". Sem ele — que é o
/// ⌘T e o clique no "+" —, a conversa nasce com o do workspace, como as irmãs.
#[tauri::command]
pub fn new_tab(
    app: AppHandle,
    state: State<AppState>,
    workspace: String,
    prompt: String,
    choice: Option<Choice>,
) -> Result<Tab, String> {
    // Plan mode não vem de nenhum dos dois caminhos: é escolha de uma fala.
    let (n, launch, choice) = {
        let board = lock(&state.board);
        let ws = board
            .workspaces
            .iter()
            .find(|w| w.id == workspace)
            .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
        if ws.cleaned {
            return Err(i18n::t("err.session.cleaned"));
        }
        // Escolher o mesmo do workspace não é escolher: a aba fica sem o campo,
        // e o quadro não guarda uma cópia do que está uma linha acima.
        let choice =
            choice.filter(|c| c.agent != ws.agent || c.model != ws.model || c.effort != ws.effort);
        let launch = choice.clone().map_or_else(|| ws.launch(), Launch::from);
        (ws.tabs.len() + 1, launch, choice)
    };

    let title = if prompt.trim().is_empty() {
        format!("conversa {n}")
    } else {
        tab_title(&prompt)
    };
    let pending = (!prompt.trim().is_empty()).then(|| prompt.trim().to_string());
    let tab = spawn_tab(&app, &state, &workspace, &title, pending, &launch, choice)?;

    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.active = Some(tab.id.clone());
            ws.tabs.push(tab.clone());
        }
    }
    publish(&app);
    chat::ready_now(&app, &tab.id);
    Ok(tab)
}

#[tauri::command]
pub fn close_tab(app: AppHandle, state: State<AppState>, workspace: String, tab: String) {
    stop(&state, std::slice::from_ref(&tab));
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.tabs.retain(|t| t.id != tab);
            if ws.active.as_deref() == Some(tab.as_str()) {
                ws.active = ws.tabs.first().map(|t| t.id.clone());
            }
        }
    }
    publish(&app);
}

#[tauri::command]
pub fn focus_tab(app: AppHandle, state: State<AppState>, workspace: String, tab: String) {
    {
        let mut board = lock(&state.board);
        if let Some(ws) = board.workspace_mut(&workspace) {
            ws.active = Some(tab);
        }
    }
    publish(&app);
}

/// O nome da conversa nasce da primeira frase do prompt, ou de um "conversa 2"
/// quando não houve prompt — e nenhum dos dois é o assunto que ela acaba tendo.
/// Nome vazio é desistência, não apagar o que já existe, como no workspace.
#[tauri::command]
pub fn rename_tab(
    app: AppHandle,
    state: State<AppState>,
    workspace: String,
    tab: String,
    title: String,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    {
        let mut board = lock(&state.board);
        if let Some(t) = board
            .workspace_mut(&workspace)
            .and_then(|ws| ws.tabs.iter_mut().find(|t| t.id == tab))
        {
            t.title = title.to_string();
        }
    }
    publish(&app);
}

/// Retoma uma aba desligada. O transcript vive em
/// `~/.claude/projects/<slug>/<id>.jsonl` e sobrevive ao app, ao worktree e ao
/// reboot — então `--resume` devolve a conversa inteira de onde parou. `true`
/// é retomou; `false` é conversa que nunca falou, reaberta nova no mesmo lugar.
#[tauri::command]
pub fn resume_tab(app: AppHandle, state: State<AppState>, tab: String) -> Result<bool, String> {
    revive(&app, &state, &tab)
}

/// Sobe de novo o processo de uma aba. É o `resume_tab`, e é o que a primeira
/// fala numa aba desligada faz por conta própria (`chat::chat_send`).
pub fn revive(app: &AppHandle, state: &State<AppState>, tab: &str) -> Result<bool, String> {
    let (workspace, worktree, launch, cleaned, agent_session) = lock(&state.board)
        .workspace_of(tab)
        .map(|w| {
            let previous = w
                .tabs
                .iter()
                .find(|t| t.id == tab)
                .and_then(|t| t.agent_session.clone());
            (
                w.id.clone(),
                PathBuf::from(&w.worktree),
                w.launch_of(tab),
                w.cleaned,
                previous,
            )
        })
        .ok_or_else(|| i18n::t("err.session.noTab"))?;
    if cleaned {
        return Err(i18n::t("err.session.cleaned"));
    }
    if !worktree.exists() {
        return Err(i18n::ta(
            "err.session.noWorktree",
            &[("path", worktree.display().to_string())],
        ));
    }

    // O que sobrou da sessão anterior sai antes: o processo já morreu, mas o
    // `Chat` continua no mapa até alguém tirar.
    chat::kill(state, tab);

    // Conversa que nunca falou não tem transcript, e retomar morre nela. Aí a
    // aba renasce com o mesmo id: não há nada perdido, e travar a tela num erro
    // por causa de uma conversa vazia seria pior. No Codex a pergunta é outra —
    // se ele já contou qual thread abriu —, porque o transcript dele não mora
    // num caminho que dê para adivinhar.
    let (resume, handle) = match launch.agent {
        ProviderId::Codex => (
            agent_session.is_some(),
            crate::codex::spawn(app, tab, &workspace, &worktree, agent_session, &launch)?,
        ),
        ProviderId::Claude => {
            let resume = paths::transcript(tab, &worktree).exists();
            (
                resume,
                crate::claude::spawn(app, tab, &worktree, claude_args(tab, resume, &launch))?,
            )
        }
    };
    lock(&state.chats).insert(tab.to_string(), handle);
    chat::ready_now(app, tab);
    {
        let mut board = lock(&state.board);
        if let Some(t) = board.tab_mut(tab) {
            t.status = Status::Pronta;
            t.note = None;
        }
    }
    publish(app);
    Ok(resume)
}

fn spawn_tab(
    app: &AppHandle,
    state: &State<AppState>,
    workspace: &str,
    title: &str,
    pending_prompt: Option<String>,
    launch: &Launch,
    choice: Option<Choice>,
) -> Result<Tab, String> {
    let worktree = lock(&state.board)
        .workspaces
        .iter()
        .find(|candidate| candidate.id == workspace)
        .map(|workspace| PathBuf::from(&workspace.worktree))
        .ok_or_else(|| i18n::t("err.session.noWorkspace"))?;
    let id = uuid::Uuid::new_v4().to_string();
    // O modelo escolhido diz qual CLI sobe (ver `agents.rs`); a aba é a mesma.
    let handle = match launch.agent {
        ProviderId::Codex => crate::codex::spawn(app, &id, workspace, &worktree, None, launch)?,
        ProviderId::Claude => {
            crate::claude::spawn(app, &id, &worktree, claude_args(&id, false, launch))?
        }
    };
    lock(&state.chats).insert(id.clone(), handle);
    // Quem chama põe a aba no quadro e só então libera a fala
    // (`chat::ready_now`): a fala guardada mora na aba, e a aba nasce aqui.
    Ok(Tab {
        id,
        agent_session: None,
        title: title.to_string(),
        status: Status::Pronta,
        note: None,
        pending_prompt,
        tokens: None,
        choice,
    })
}

/* ---------- plumbing ---------- */

/// Os argumentos do `claude`. `resume` decide se a sessão nasce nova ou
/// continua a que já existe — o id é o mesmo nos dois casos.
///
/// O modo é o headless com JSON dos dois lados: cada coisa que o agente faz
/// sai como uma linha, cada fala entra como uma linha, e o processo fica de pé
/// entre um turno e outro (`chat.rs`). `--permission-prompt-tool stdio` é o
/// que faz pergunta, plano e pedido de permissão chegarem pelo mesmo cano, em
/// vez de a sessão morrer sem ninguém para responder.
///
/// O agente roda solto: cada sessão vive no seu worktree e não para a cada
/// ferramenta — que é o motivo de existir o quadro. Em plan mode nasce
/// perguntando, e é a aprovação do plano que o solta: a tela manda um
/// `set_permission_mode` para bypass junto com o "sim" (ver `chat.ts`). Aqui
/// vai o `--allow-…`, sem o qual o `claude` recusa a troca — e sem o
/// `--dangerously-…`, que junto do `--permission-mode plan` ganha do plan.
fn claude_args(id: &str, resume: bool, launch: &Launch) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--include-partial-messages",
        "--verbose",
        "--permission-prompt-tool",
        "stdio",
        if resume { "--resume" } else { "--session-id" },
        id,
    ]
    .map(String::from)
    .to_vec();
    if launch.plan {
        args.extend(
            [
                "--permission-mode",
                "plan",
                "--allow-dangerously-skip-permissions",
            ]
            .map(String::from),
        );
    } else {
        args.push("--dangerously-skip-permissions".into());
    }
    if !launch.model.trim().is_empty() {
        args.extend(["--model".into(), launch.model.trim().into()]);
    }
    if !launch.effort.trim().is_empty() {
        args.extend(["--effort".into(), launch.effort.trim().into()]);
    }
    // Os servidores escolhidos, e nada além deles: o `--strict-mcp-config` é o
    // que faz o `~/.claude.json` do usuário parar de entrar por baixo. Sem
    // escolha (workspace de antes disto existir) nem um nem outro vão, e o CLI
    // decide como sempre decidiu. Falhar em escrever o arquivo não derruba a
    // conversa — ela sobe sem MCP, que é a mesma perda de quem não escolheu.
    match crate::mcp::config_for(id, launch.mcp.as_ref()) {
        Ok(Some(path)) => args.extend([
            "--mcp-config".into(),
            path.display().to_string(),
            "--strict-mcp-config".into(),
        ]),
        Ok(None) => {}
        Err(error) => eprintln!("mcp de {id}: {error}"),
    }
    // Os plugins escolhidos, um `--plugin-dir`/`--plugin-url` cada. São flags
    // de sessão: não mexem no cadastro do CLI, e um plugin que ele já carrega
    // sozinho não entra duas vezes — a deduplicação é por nome, e é dele. Sem
    // escolha nenhuma nada vai, e vale o que o CLI já carregava. O adapter do
    // Codex materializa a mesma seleção no home derivado do workspace, em
    // `plugins.rs`; estas flags continuam sendo só do Claude.
    args.extend(crate::plugins::args_for(launch.plugins.as_ref()));
    args
}

/// Contexto injetado vira menção `@caminho` na primeira fala — que é como o
/// próprio Claude Code já lê arquivo. Nada de mecanismo novo.
fn first_message(prompt: &str, inject: &[String]) -> Option<String> {
    let mentions = inject
        .iter()
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("@{}", p.trim()))
        .collect::<Vec<_>>()
        .join(" ");

    let parts: Vec<String> = [mentions, prompt.trim().to_string()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// Rótulo de aba a partir da primeira frase do prompt. Corta mais curto que o
/// nome do workspace (que o lançador monta): a barra de abas é estreita e
/// várias delas dividem a linha.
fn tab_title(prompt: &str) -> String {
    let line = prompt.trim().lines().next().unwrap_or("").trim();
    match line.chars().count() > 34 {
        true => line.chars().take(33).collect::<String>() + "…",
        false => line.to_string(),
    }
}

/// `base` é de onde a branch nova sai — `origin/main`, por padrão. Branch que
/// já existe ignora a base: aí o worktree só a traz de volta para o disco, e
/// mudar o ponto de partida de trabalho que já começou não é criar workspace.
///
/// Devolve se a pasta nasceu aqui: pasta adotada não é desfeita quando um
/// repositório irmão recusa.
fn add_worktree(repo: &Path, branch: &str, base: &str, dest: &Path) -> Result<bool, String> {
    // Pasta que já está lá é reaproveitada — mas só se for a branch pedida.
    // Antes qualquer pasta com o nome certo servia, então um worktree na branch
    // errada era adotado calado e o quadro passava a mentir em que branch a
    // sessão estava mexendo.
    if dest.exists() {
        return match head_branch(dest) {
            Some(head) if head == branch => Ok(false),
            Some(head) => Err(i18n::ta(
                "err.session.worktreeElsewhere",
                &[
                    ("path", dest.display().to_string()),
                    ("head", head),
                    ("branch", branch.to_string()),
                ],
            )),
            None => Err(i18n::ta(
                "err.session.worktreeDetached",
                &[("path", dest.display().to_string())],
            )),
        };
    }
    branch_free(repo, branch, dest)?;

    let parent = dest
        .parent()
        .ok_or_else(|| i18n::t("err.session.noParent"))?;
    // Quem cria a pasta do worktree é o git; daqui sai só o caminho até ela — e
    // ele volta atrás se o git recusar, senão sobra no disco uma pasta vazia que
    // não é worktree de ninguém e não aparece em `git worktree list`.
    let abertas = open_dirs(parent)?;

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("worktree").arg("add");
    if has_commit(repo, &format!("refs/heads/{branch}")) {
        cmd.arg(dest).arg(branch);
    } else {
        cmd.arg("-b").arg(branch).arg(dest);
        if !base.is_empty() {
            if let Err(e) = prepare_base(repo, base) {
                close_dirs(&abertas);
                return Err(e);
            }
            cmd.arg(base);
        }
    }

    let out = cmd.output().map_err(|e| {
        close_dirs(&abertas);
        i18n::ta("err.git.spawn", &[("cause", e.to_string())])
    })?;
    if !out.status.success() {
        close_dirs(&abertas);
        return Err(i18n::ta(
            "err.git",
            &[
                ("command", "git worktree add".into()),
                (
                    "cause",
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ),
            ],
        ));
    }
    Ok(true)
}

/// Onde esta branch já está em check-out neste repositório — o próprio clone ou
/// um worktree dele. É o que o `git worktree add` confere antes de recusar.
fn worktree_of_branch(repo: &Path, branch: &str) -> Option<PathBuf> {
    let want = format!("branch refs/heads/{branch}");
    let listed = git(repo, &["worktree", "list", "--porcelain"]);
    let mut at = None;
    for line in listed.lines() {
        match line.strip_prefix("worktree ") {
            Some(path) => at = Some(PathBuf::from(path)),
            None if line == want => return at,
            None => {}
        }
    }
    None
}

/// Uma branch só abre numa pasta — regra do git, não escolha daqui. Duas
/// sessões que saem da mesma issue do Linear pedem a mesma branch, e o destino
/// não é o mesmo: a pasta leva os nomes dos repositórios do workspace, e
/// `capim-code-rules` sozinho não mora onde `capim-code-rules+capim-autonomous`
/// mora. Quem recusava era o git, com o texto dele; aqui o erro diz de quem é a
/// pasta que está segurando a branch, que é o que decide o que fazer.
fn branch_free(repo: &Path, branch: &str, dest: &Path) -> Result<(), String> {
    match worktree_of_branch(repo, branch) {
        Some(at) if !same_path(&at, dest) => Err(i18n::ta(
            "err.session.branchBusy",
            &[
                ("name", repo_label(repo)),
                ("branch", branch.to_string()),
                ("path", at.display().to_string()),
            ],
        )),
        _ => Ok(()),
    }
}

/// O git lista worktree por caminho resolvido; o destino daqui é montado a
/// partir do `HOME`. Comparar texto puro faria `/var` e `/private/var` — o
/// mesmo lugar — passarem por pastas diferentes.
fn same_path(a: &Path, b: &Path) -> bool {
    a == b
        || match (a.canonicalize(), b.canonicalize()) {
            (Ok(x), Ok(y)) => x == y,
            _ => false,
        }
}

/// Abre o caminho até `dir` e devolve, de fora para dentro, as pastas que
/// passaram a existir agora — as que o `close_dirs` sabe desfazer.
fn open_dirs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut novas = Vec::new();
    let mut at = Some(dir);
    while let Some(p) = at.filter(|p| !p.exists()) {
        novas.push(p.to_path_buf());
        at = p.parent();
    }
    std::fs::create_dir_all(dir).map_err(i18n::io)?;
    novas.reverse();
    Ok(novas)
}

/// `remove_dir` e não `remove_dir_all`: só some a pasta que continua vazia. Se
/// alguma coisa chegou nela nesse meio-tempo, ela fica.
fn close_dirs(dirs: &[PathBuf]) {
    for dir in dirs.iter().rev() {
        let _ = std::fs::remove_dir(dir);
    }
}

/// O nome de um repositório para uma frase de erro: a pasta do clone.
fn repo_label(repo: &Path) -> String {
    repo.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Devolve ao disco o que esta montagem criou. A branch fica: ela não atrapalha
/// a próxima tentativa, que a reaproveita como faria com qualquer branch que já
/// existe — e apagá-la seria apagar também a que já estava lá antes.
fn undo_worktrees(feitos: &[(PathBuf, PathBuf)]) {
    for (repo, dest) in feitos.iter().rev() {
        let _ = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "remove", "--force"])
            .arg(dest)
            .output();
        let _ = git(repo, &["worktree", "prune"]);
    }
}

/// Worktree desligado: a branch nasce no próprio repositório e é o diretório de
/// trabalho dele que troca de branch. Serve para quem quer o agente mexendo no
/// clone de sempre — o preço é que o repo sai de onde estava, e mudança não
/// commitada vai junto (ou o git recusa, e o erro sobe para a tela).
fn switch_branch(repo: &Path, branch: &str, base: &str) -> Result<(), String> {
    if head_branch(repo).as_deref() == Some(branch) {
        return Ok(());
    }

    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).arg("switch");
    if has_commit(repo, &format!("refs/heads/{branch}")) {
        cmd.arg(branch);
    } else {
        cmd.arg("-c").arg(branch);
        if !base.is_empty() {
            prepare_base(repo, base)?;
            cmd.arg(base);
        }
    }

    let out = cmd
        .output()
        .map_err(|e| i18n::ta("err.git.spawn", &[("cause", e.to_string())]))?;
    if !out.status.success() {
        return Err(i18n::ta(
            "err.git",
            &[
                ("command", "git switch".into()),
                (
                    "cause",
                    String::from_utf8_lossy(&out.stderr).trim().to_string(),
                ),
            ],
        ));
    }
    Ok(())
}

fn head_branch(repo: &Path) -> Option<String> {
    let name = git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string();
    (!name.is_empty() && name != "HEAD").then_some(name)
}

/// Deixa a base pronta para virar ponto de partida: um `origin/main` velho é o
/// lugar errado, então atualiza só aquela ref — e segue mesmo se a rede não
/// deixar, porque base local desatualizada ainda é melhor que não criar nada.
fn prepare_base(repo: &Path, base: &str) -> Result<(), String> {
    if let Some((remote, rest)) = base.split_once('/') {
        if has_commit(repo, &format!("refs/remotes/{base}")) {
            let _ = fetch(repo, remote, rest);
        }
    }
    match has_commit(repo, base) {
        true => Ok(()),
        false => Err(i18n::ta(
            "err.session.noBase",
            &[
                ("base", base.to_string()),
                ("path", repo.display().to_string()),
            ],
        )),
    }
}

/// `git fetch` com coleira: rede pendurada não pode virar app pendurado, e a
/// base local velha ainda dá um worktree utilizável.
fn fetch(repo: &Path, remote: &str, branch: &str) -> Result<(), String> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["fetch", "--quiet", remote, branch])
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(i18n::io)?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Err(e) => return Err(i18n::io(e)),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                return Err(i18n::t("err.git.fetchSlow"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
}

/// Se a ref existe e aponta para um commit — `--verify` sozinho aceita coisas
/// que o `worktree add` depois recusa.
fn has_commit(repo: &Path, reference: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{reference}^{{commit}}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// O nome de um clone, conferindo antes que ele é um repositório git.
fn repo_named(path: &Path) -> Result<String, String> {
    if !path.join(".git").exists() {
        return Err(i18n::ta(
            "err.session.notGit",
            &[("path", path.display().to_string())],
        ));
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| i18n::t("err.session.badPath"))
}

/// O que um agente encontra ao abrir a pasta de um workspace com mais de um
/// repositório: qual é qual, e que todos estão na mesma branch. O Claude Code
/// lê `CLAUDE.md` de onde roda e o Codex lê `AGENTS.md`; os dois recebem o
/// mesmo texto. Nunca sobrescreve — a pessoa pode ter escrito o dela.
fn describe_root(root: &Path, repos: &[Repo], branch: &str) {
    let list: String = repos
        .iter()
        .map(|r| {
            format!(
                "- `{}/` — {} `{}`\n",
                r.name,
                i18n::pick("clone em", "clone at"),
                r.path
            )
        })
        .collect();
    let text = i18n::pick(
        &format!(
            "# Workspace com {} repositórios\n\nEsta pasta reúne um worktree por repositório, todos na branch `{branch}`:\n\n{list}\nCada um é um repositório git independente: commits, `git status` e PRs são por pasta. Leia o `CLAUDE.md` ou `AGENTS.md` de cada um antes de mexer nele.\n",
            repos.len()
        ),
        &format!(
            "# Workspace with {} repositories\n\nThis folder holds one worktree per repository, all on branch `{branch}`:\n\n{list}\nEach one is an independent git repository: commits, `git status` and PRs are per folder. Read each one's `CLAUDE.md` or `AGENTS.md` before working on it.\n",
            repos.len()
        ),
    );
    for name in ["CLAUDE.md", "AGENTS.md"] {
        let file = root.join(name);
        if !file.exists() {
            let _ = std::fs::write(&file, &text);
        }
    }
}

/// De onde uma branch nova sai neste repositório quando ninguém escolheu:
/// `origin/HEAD` como o clone gravou, senão os nomes de sempre, senão a branch
/// em que ele está. É a mesma conta que encabeça a lista do lançador.
fn default_base(repo: &Path) -> String {
    list_branches(repo.display().to_string()).default
}

/// As branches do repositório, para o lançador escolher de onde a nova sai.
/// Mais recente primeiro: a que você mexeu ontem é a que você quer hoje.
#[derive(serde::Serialize)]
pub struct Branches {
    pub all: Vec<String>,
    pub default: String,
}

#[tauri::command(async)]
pub fn list_branches(project: String) -> Branches {
    let repo = PathBuf::from(expand(&project));
    let refs = |pattern: &str| -> Vec<String> {
        git(
            &repo,
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname:short)",
                pattern,
            ],
        )
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty() && !r.ends_with("/HEAD"))
        .map(str::to_string)
        .collect()
    };
    let locals = refs("refs/heads");
    let remotes = refs("refs/remotes");

    // `origin/HEAD` é o que o clone gravou como principal do remoto. Sem ele,
    // os nomes de sempre; sem eles, a branch em que o repo está agora.
    let head = git(
        &repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .trim()
    .to_string();
    let default = [head, "origin/main".to_string(), "origin/master".to_string()]
        .into_iter()
        .find(|r| !r.is_empty() && remotes.contains(r))
        .or_else(|| {
            let head = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])
                .trim()
                .to_string();
            (!head.is_empty() && head != "HEAD").then_some(head)
        })
        .or_else(|| locals.first().cloned())
        .unwrap_or_default();

    // A base escolhida encabeça a lista; o resto vem local antes de remoto,
    // que é a ordem em que se pensa em branch.
    let mut all: Vec<String> = Vec::new();
    for name in [default.clone()].into_iter().chain(locals).chain(remotes) {
        if !name.is_empty() && !all.contains(&name) {
            all.push(name);
        }
    }
    Branches { all, default }
}

#[cfg(test)]
mod tests {
    use super::{
        claude_args, multi_pr_text, patch_map, pr_text, Choice, Launch, Pr, ProviderId, Repo,
        RepoPr, Tab, Workspace,
    };
    use crate::dock::{is_terminal, multi_setup, quoted};
    use std::path::Path;

    fn pr(number: u64, branch: &str, state: &str) -> Pr {
        Pr {
            number,
            title: format!("PR {number}"),
            is_draft: false,
            state: state.into(),
            head_ref_name: branch.into(),
        }
    }

    /// As guardas de devolver o disco, contra um git de verdade: nada sai antes
    /// de o trabalho ter entrado no alvo, nada sai com mudança fora de commit, e
    /// nada sai antes de o workspace estar arquivado — que é quando o `archive`
    /// do repositório rodou.
    #[test]
    fn check_so_deixa_sair_o_que_ja_entrou_e_esta_limpo() {
        let root = std::env::temp_dir().join(format!("prometeu-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (origin, local) = (root.join("origin"), root.join("clone"));
        std::fs::create_dir_all(&origin).unwrap();

        let run = |dir: &std::path::Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                // O commit do teste não depende da assinatura da máquina: com
                // `commit.gpgsign` global, o gpg do runner falhava em paralelo.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["config", "user.email", "t@t"]);
        run(&origin, &["config", "user.name", "t"]);
        std::fs::write(origin.join("a.txt"), "a").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "a"]);

        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&local)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let dest = root.join("wt");
        super::add_worktree(&local, "trabalho", "origin/main", &dest).unwrap();

        let mut ws = super::Workspace {
            id: "w".into(),
            title: "trabalho".into(),
            project: local.display().to_string(),
            repo: local.display().to_string(),
            repo_name: "clone".into(),
            branch: "trabalho".into(),
            worktree: dest.display().to_string(),
            repos: vec![Repo {
                path: local.display().to_string(),
                name: "clone".into(),
                worktree: dest.display().to_string(),
                base: String::new(),
                pr: None,
            }],
            stage: "Feito".into(),
            agent: ProviderId::Claude,
            archived: true,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            audience: None,
            preparing: false,
            mcp: None,
            plugins: None,
            failed: None,
            model: String::new(),
            effort: String::new(),
            port: None,
            issue: None,
            tabs: Vec::new(),
            active: None,
        };

        // Branch nova sem commit próprio já é o alvo: pode sair.
        super::check(&ws).unwrap();

        // Um commit que não está no alvo segura o worktree.
        std::fs::write(dest.join("b.txt"), "b").unwrap();
        run(&dest, &["config", "user.email", "t@t"]);
        run(&dest, &["config", "user.name", "t"]);
        run(&dest, &["add", "-A"]);
        run(&dest, &["commit", "-qm", "b"]);
        assert!(super::check(&ws).unwrap_err().contains("unmerged"));

        // Mas o `gh` dizendo que o PR entrou é a outra resposta que serve — o
        // merge por squash não deixa a branch ancestral de nada.
        ws.repos[0].pr = Some(pr(3, "trabalho", "MERGED"));
        super::check(&ws).unwrap();

        // Mudança fora de commit segura de qualquer jeito.
        std::fs::write(dest.join("c.txt"), "c").unwrap();
        assert!(super::check(&ws).unwrap_err().contains("dirty"));
        // Mas é justamente o que `force` atravessa: quem marcou o vermelho na
        // tela sabe que essa mudança vai junto.
        super::hard(&ws).unwrap();
        std::fs::remove_file(dest.join("c.txt")).unwrap();
        super::check(&ws).unwrap();

        // Ainda na frente de todo mundo: arquivar é o passo de antes, e nem
        // `force` pula ele.
        ws.archived = false;
        assert!(super::check(&ws).unwrap_err().contains("notArchived"));
        assert!(super::hard(&ws).unwrap_err().contains("notArchived"));
        ws.archived = true;

        // O próprio clone também não sai por `force` nenhum.
        ws.worktree = ws.repo.clone();
        assert!(super::hard(&ws).unwrap_err().contains("isRepo"));

        // Com um segundo repositório, ele responde pelas mesmas guardas: um
        // commit fora do alvo *nele* segura o workspace inteiro.
        ws.worktree = dest.display().to_string();
        let dest2 = root.join("wt2");
        super::add_worktree(&local, "trabalho-2", "origin/main", &dest2).unwrap();
        ws.repos.push(Repo {
            path: local.display().to_string(),
            name: "clone-2".into(),
            worktree: dest2.display().to_string(),
            base: String::new(),
            pr: None,
        });
        super::check(&ws).unwrap();
        std::fs::write(dest2.join("d.txt"), "d").unwrap();
        assert!(super::check(&ws).unwrap_err().contains("dirty"));

        // A pasta agregadora, que o app apaga com `remove_dir_all`, só vale no
        // formato exato criado pelo Prometeu e com os worktrees como filhos.
        let names: Vec<String> = ws.repos.iter().map(|repo| repo.name.clone()).collect();
        let multi = super::paths::multi_dir(&names, &ws.branch);
        ws.worktree = multi.display().to_string();
        for repo in &mut ws.repos {
            repo.worktree = multi.join(&repo.name).display().to_string();
        }
        super::validate_multi_root(&ws).unwrap();

        // O importador não move os worktrees. A raiz antiga calculada pela
        // mesma branch também é segura; um caminho apenas parecido, não.
        let legacy = super::paths::prometheus_multi_dir(&names, &ws.branch);
        ws.worktree = legacy.display().to_string();
        for repo in &mut ws.repos {
            repo.worktree = legacy.join(&repo.name).display().to_string();
        }
        super::validate_multi_root(&ws).unwrap();
        ws.worktree = legacy.join("vizinho").display().to_string();
        assert!(super::validate_multi_root(&ws)
            .unwrap_err()
            .contains("badRoot"));

        ws.worktree = super::paths::home().display().to_string();
        assert!(super::validate_multi_root(&ws)
            .unwrap_err()
            .contains("badRoot"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn repo_pr(
        name: &str,
        branch: Option<&str>,
        dirty: usize,
        ahead: u32,
        target: &str,
        upstream: bool,
        open: Option<u64>,
    ) -> RepoPr {
        RepoPr {
            name: name.into(),
            branch: branch.map(str::to_string),
            dirty,
            ahead,
            target: target.into(),
            upstream,
            open,
        }
    }

    /// O prompt de PR diz o estado e os passos com os nomes certos: a branch
    /// no push, o alvo sem o remoto no `--base`, e a sujeira contada.
    #[test]
    fn pr_text_diz_o_estado_e_os_passos() {
        let t = pr_text(&repo_pr(
            "app",
            Some("meu/ajuste"),
            3,
            2,
            "origin/main",
            false,
            None,
        ));
        assert!(t.contains("Há 3 arquivos"));
        assert!(t.contains("git push -u origin HEAD:meu/ajuste"));
        assert!(t.contains("gh pr create --base main"));
        assert!(t.contains("Ainda não há branch upstream."));

        let limpo = pr_text(&repo_pr("app", None, 0, 0, "origin/master", true, None));
        assert!(limpo.contains("limpo"));
        assert!(limpo.contains("HEAD solto"));
        assert!(limpo.contains("--base master"));
        assert!(limpo.contains("A branch já tem upstream."));
    }

    /// Com PR aberto o pedido é outro: atualizar o #42, e não criar um segundo.
    #[test]
    fn pr_text_com_pr_aberto_pede_atualizacao() {
        let t = pr_text(&repo_pr(
            "app",
            Some("meu/ajuste"),
            1,
            1,
            "origin/main",
            true,
            Some(42),
        ));
        assert!(t.contains("Quero atualizar o PR #42"));
        assert!(t.contains("gh pr view 42"));
        assert!(t.contains("gh pr edit 42"));
        assert!(!t.contains("gh pr create"));
        // O caminho até lá é o mesmo: commitar e empurrar continua sendo o miolo.
        assert!(t.contains("git push -u origin HEAD:meu/ajuste"));
    }

    /// Com mais de um repositório o pedido lista cada um com o que ele tem: o
    /// que já tem PR pede atualização, o que não mudou fica sem PR, e o resto
    /// ganha o seu — todos linkando os outros.
    #[test]
    fn multi_pr_text_lista_cada_repositorio() {
        let t = multi_pr_text(&[
            repo_pr("backend", Some("feat/x"), 2, 4, "origin/main", true, None),
            repo_pr(
                "portal",
                Some("feat/x"),
                0,
                1,
                "origin/develop",
                true,
                Some(17),
            ),
            repo_pr("dash", Some("feat/x"), 0, 0, "origin/main", false, None),
        ]);
        assert!(t.contains("reúne 3 repositórios"));
        assert!(t.contains("`backend/` — branch `feat/x`, 4 commits além de `origin/main`, 2 arquivos fora de commit. sem PR ainda."));
        assert!(t.contains("`portal/` — branch `feat/x`, 1 commit além de `origin/develop`, nada fora de commit. PR #17 aberto"));
        assert!(t.contains("`dash/` — branch `feat/x`, nenhum commit além de `origin/main`, nada fora de commit, sem upstream. nada a entregar: fica sem PR."));
        assert!(t.contains("linkar os outros PRs"));
    }
    use std::process::Command;

    fn launch(model: &str, effort: &str, plan: bool) -> Launch {
        Launch {
            mcp: None,
            plugins: None,
            agent: ProviderId::Claude,
            model: model.into(),
            effort: effort.into(),
            plan,
        }
    }

    /// Quem nunca escolheu MCP não recebe `--strict-mcp-config`: o CLI segue
    /// decidindo sozinho, como fazia antes do hub existir. Quem escolheu recebe
    /// o arquivo e o `--strict-…`, que é o que fecha a sessão no que foi
    /// marcado.
    #[test]
    fn mcp_so_entra_quando_alguem_escolheu() {
        let sem = claude_args("id", false, &launch("", "", false));
        assert!(!sem.contains(&"--mcp-config".to_string()));
        assert!(!sem.contains(&"--strict-mcp-config".to_string()));

        let root = std::env::temp_dir().join(format!("prometeu-mcp-{}", uuid::Uuid::new_v4()));
        std::env::set_var("PROMETEU_ROOT", &root);
        let escolheu = Launch {
            mcp: Some(vec!["notion".into()]),
            ..launch("", "", false)
        };
        let args = claude_args("id", false, &escolheu);
        std::env::remove_var("PROMETEU_ROOT");
        let at = args
            .iter()
            .position(|a| a == "--mcp-config")
            .expect("o arquivo");
        assert!(std::path::Path::new(&args[at + 1]).exists());
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Quem nunca escolheu plugin não recebe flag nenhuma: o CLI carrega o que
    /// sempre carregou. A tradução de escolha em flag é do `plugins.rs`, e o
    /// teste dela mora lá — aqui só o caso de não haver escolha, que é o de
    /// todo quadro gravado antes disto existir.
    #[test]
    fn sem_escolha_nao_ha_flag_de_plugin() {
        let args = claude_args("id", false, &launch("", "", false));
        assert!(!args.contains(&"--plugin-dir".to_string()));
        assert!(!args.contains(&"--plugin-url".to_string()));
    }

    /// Bypass e plan não convivem na mesma linha: `--dangerously-skip-permissions`
    /// engole o plan. Plan mode é `--allow-…` mais `--permission-mode plan`.
    #[test]
    fn plan_mode_nao_leva_o_bypass_junto() {
        let solto = claude_args("id", false, &launch("", "", false));
        assert!(solto.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!solto.contains(&"--permission-mode".to_string()));

        let plano = claude_args("id", false, &launch("", "", true));
        assert!(!plano.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(plano.contains(&"--allow-dangerously-skip-permissions".to_string()));
        let at = plano.iter().position(|a| a == "--permission-mode").unwrap();
        assert_eq!(plano[at + 1], "plan");
    }

    /// Vazio é não passar a flag — o Claude Code escolhe. Cheio vai como veio.
    #[test]
    fn modelo_e_esforco_so_quando_escolhidos() {
        let padrao = claude_args("id", true, &launch("", " ", false));
        assert!(!padrao.contains(&"--model".to_string()));
        assert!(!padrao.contains(&"--effort".to_string()));
        let at = padrao.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(padrao[at + 1], "id");

        let escolhido = claude_args("id", false, &launch("opus[1m]", "max", false));
        assert_eq!(
            escolhido[escolhido.len() - 4..],
            ["--model", "opus[1m]", "--effort", "max"]
        );
        let at = escolhido.iter().position(|a| a == "--session-id").unwrap();
        assert_eq!(escolhido[at + 1], "id");
    }

    /// Um workspace vazio, para o que não depende de disco.
    fn bare() -> Workspace {
        Workspace {
            id: "w".into(),
            title: "w".into(),
            project: String::new(),
            repo: String::new(),
            repo_name: String::new(),
            branch: String::new(),
            worktree: String::new(),
            repos: Vec::new(),
            stage: String::new(),
            archived: false,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            audience: None,
            preparing: false,
            mcp: None,
            plugins: None,
            failed: None,
            agent: ProviderId::Claude,
            model: String::new(),
            effort: String::new(),
            port: None,
            issue: None,
            tabs: Vec::new(),
            active: None,
        }
    }

    /// Uma aba com o que basta para dizer com quem ela fala.
    fn tab(id: &str, choice: Option<Choice>) -> Tab {
        Tab {
            id: id.into(),
            agent_session: None,
            title: id.into(),
            status: crate::state::Status::Desligada,
            note: None,
            pending_prompt: None,
            tokens: None,
            choice,
        }
    }

    /// A aba que nasceu com outro modelo volta com ele, e não com o das irmãs
    /// — é o que separa retomar de recomeçar. Aba sem escolha segue o
    /// workspace, que é o quadro gravado antes disto existir e o ⌘T de sempre.
    #[test]
    fn retomar_uma_aba_respeita_o_modelo_com_que_ela_nasceu() {
        let mut ws = bare();
        ws.agent = ProviderId::Claude;
        ws.model = "opus[1m]".into();
        ws.effort = "high".into();
        ws.tabs = vec![
            tab("herda", None),
            tab(
                "propria",
                Some(Choice {
                    agent: ProviderId::Codex,
                    model: "gpt-5.6-sol".into(),
                    effort: "ultracode".into(),
                }),
            ),
        ];

        let herda = ws.launch_of("herda");
        assert_eq!(
            (herda.model.as_str(), herda.effort.as_str()),
            ("opus[1m]", "high")
        );
        assert_eq!(herda.agent, ProviderId::Claude);

        let propria = ws.launch_of("propria");
        assert_eq!(propria.agent, ProviderId::Codex);
        assert_eq!(
            (propria.model.as_str(), propria.effort.as_str()),
            ("gpt-5.6-sol", "ultracode")
        );
        // Plan mode é de uma fala, não da conversa: retomar nunca volta nele.
        assert!(!propria.plan);

        // Aba que não está no quadro — fechada entre o pedido e a resposta —
        // cai no do workspace, e não num modelo inventado.
        assert_eq!(ws.launch_of("sumiu").model, "opus[1m]");
    }

    /// Trocar o modelo de uma conversa de pé grava a escolha na aba, e voltar
    /// ao do workspace apaga a escolha em vez de congelar o de hoje. Trocar de
    /// CLI é recusado: o transcript de um o outro não retoma.
    #[test]
    fn trocar_o_modelo_de_uma_conversa_grava_na_aba() {
        let mut ws = bare();
        ws.model = "opus[1m]".into();
        ws.effort = "high".into();
        ws.tabs = vec![tab("aberta", None)];

        let choice = |model: &str, effort: &str| Choice {
            agent: ProviderId::Claude,
            model: model.into(),
            effort: effort.into(),
        };

        ws.retune("aberta", choice("sonnet", "medium")).unwrap();
        let launch = ws.launch_of("aberta");
        assert_eq!(
            (launch.model.as_str(), launch.effort.as_str()),
            ("sonnet", "medium")
        );
        // As irmãs que seguem o workspace não foram junto.
        assert_eq!(ws.model, "opus[1m]");

        // De volta ao do workspace: a aba volta a segui-lo, e não guarda uma
        // cópia do que ele é hoje.
        ws.retune("aberta", choice("opus[1m]", "high")).unwrap();
        assert!(ws.tabs[0].choice.is_none());

        // O CLI não troca no meio da conversa.
        let gpt = Choice {
            agent: ProviderId::Codex,
            model: "gpt-5.6-sol".into(),
            effort: "high".into(),
        };
        assert!(ws.retune("aberta", gpt).is_err());
        assert!(ws.retune("sumiu", choice("sonnet", "high")).is_err());
    }

    /// O que faz a conversa ser JSON dos dois lados, e o pedido de permissão
    /// chegar pelo mesmo cano em vez de matar a sessão.
    #[test]
    fn a_conversa_e_stream_json_com_permissao_por_stdio() {
        let args = claude_args("id", false, &launch("", "", false));
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1]);
        assert_eq!(args[0], "-p");
        assert!(has(["--input-format", "stream-json"]));
        assert!(has(["--output-format", "stream-json"]));
        assert!(has(["--permission-prompt-tool", "stdio"]));
        assert!(args.contains(&"--include-partial-messages".to_string()));
    }

    /// Saída de `git diff HEAD` com três arquivos: um mexido, um apagado e um
    /// com espaço no nome. O caminho tem de sair certo nos três.
    const DIFF: &str = "\
diff --git a/src/main.ts b/src/main.ts
index 1c1c1c1..2d2d2d2 100644
--- a/src/main.ts
+++ b/src/main.ts
@@ -12,3 +12,4 @@ const $ = (id: string) => document.getElementById(id)!;
 let state: Board;
-let openWs = null;
+let openWs: string | null = null;
+let sidePane = \"files\";
diff --git a/src/old.ts b/src/old.ts
deleted file mode 100644
index 3e3e3e3..0000000
--- a/src/old.ts
+++ /dev/null
@@ -1,2 +0,0 @@
-const gone = true;
-export default gone;
diff --git a/docs/com espaco.md b/docs/com espaco.md
--- a/docs/com espaco.md
+++ b/docs/com espaco.md
@@ -1 +1 @@
-antes
+depois
";

    /// Contra o git de verdade, no worktree onde este teste está rodando: todo
    /// arquivo que a lista mostra com linhas contadas tem de vir com trecho para
    /// desenhar. Binário conta 0/0 e não tem patch — esse é o caso de fora.
    #[test]
    fn a_lista_e_o_patch_falam_do_mesmo_arquivo() {
        let wt = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        for change in super::changes_in(wt) {
            if change.added + change.removed == 0 {
                continue;
            }
            assert!(
                change.patch.contains("@@"),
                "{} tem {}+/{}- e nenhum trecho",
                change.path,
                change.added,
                change.removed
            );
        }
    }

    /// O diff contra a base, num repositório de verdade: o que está no commit
    /// desta branch e o que está fora de commit vêm juntos, cada um marcado, por
    /// caminho — e o número de commits além da base é o que se contou. Sem base
    /// que exista, sobra o que está fora de commit.
    #[test]
    fn o_diff_contra_a_base_junta_commit_e_fora_de_commit() {
        let root = std::env::temp_dir().join(format!("prometeu-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&root)
                // O commit do teste não depende da assinatura da máquina: com
                // `commit.gpgsign` global, o gpg do runner falhava em paralelo.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        std::fs::write(root.join("gone.txt"), "x\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "base"]);
        run(&["checkout", "-qb", "feat"]);
        std::fs::write(root.join("a.txt"), "a\nb\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        std::fs::write(root.join("z.txt"), "z\n").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-qm", "feat"]);
        // Fora de commit: um mexido depois do commit, e um que nem foi adicionado.
        std::fs::write(root.join("a.txt"), "a\nb\nc\n").unwrap();
        std::fs::write(root.join("novo.txt"), "n\n").unwrap();

        let d = super::repo_diff("r", &root, "main");
        assert_eq!(d.ahead, 1);
        // Sem upstream, nada desta branch está publicado: todo commit dela conta
        // como não empurrado.
        assert_eq!(d.unpushed, 1);
        assert_eq!(d.dirty, 2);
        let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "gone.txt", "novo.txt", "z.txt"]);
        let by = |p: &str| d.files.iter().find(|f| f.path == p).unwrap();
        assert!(by("a.txt").dirty && by("a.txt").added == 2 && !by("a.txt").new_file);
        assert!(by("gone.txt").deleted && !by("gone.txt").dirty);
        assert!(by("z.txt").new_file && !by("z.txt").dirty && by("z.txt").patch.contains("+z"));
        assert!(by("novo.txt").new_file && by("novo.txt").dirty);

        let sem_base = super::repo_diff("r", &root, "nao-existe");
        assert_eq!((sem_base.ahead, sem_base.files.len()), (0, 2));

        // Com a branch empurrada, o mesmo commit deixa de contar: é o que a tela
        // usa para dizer "tudo empurrado" em vez de oferecer atualizar o PR.
        let remoto =
            std::env::temp_dir().join(format!("prometeu-diff-remoto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&remoto);
        let out = Command::new("git")
            .args(["init", "-q", "--bare"])
            .arg(&remoto)
            .output()
            .unwrap();
        assert!(out.status.success());
        run(&["remote", "add", "origin", &remoto.display().to_string()]);
        run(&["push", "-q", "-u", "origin", "feat"]);
        assert_eq!(super::repo_diff("r", &root, "main").unpushed, 0);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&remoto);
    }

    /// Um repo de mentira com remoto de verdade (o "origin" é uma pasta ao
    /// lado): é o único jeito de provar que a base escolhida no lançador é de
    /// onde a branch nasce, e que `origin/main` é o padrão que o clone gravou.
    #[test]
    fn a_branch_nova_sai_da_base_escolhida() {
        let root = std::env::temp_dir().join(format!("prometeu-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (origin, local) = (root.join("origin"), root.join("clone"));
        std::fs::create_dir_all(&origin).unwrap();

        let run = |dir: &std::path::Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                // O commit do teste não depende da assinatura da máquina: com
                // `commit.gpgsign` global, o gpg do runner falhava em paralelo.
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        run(&origin, &["init", "-q", "-b", "main"]);
        run(&origin, &["config", "user.email", "t@t"]);
        run(&origin, &["config", "user.name", "t"]);
        std::fs::write(origin.join("a.txt"), "a").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "a"]);
        run(&origin, &["checkout", "-qb", "velha"]);
        std::fs::write(origin.join("b.txt"), "b").unwrap();
        run(&origin, &["add", "-A"]);
        run(&origin, &["commit", "-qm", "b"]);
        let velha = run(&origin, &["rev-parse", "HEAD"]);
        run(&origin, &["checkout", "-q", "main"]);

        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&local)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let branches = super::list_branches(local.display().to_string());
        assert_eq!(branches.default, "origin/main");
        assert_eq!(branches.all.first().unwrap(), "origin/main");
        assert!(
            branches.all.contains(&"origin/velha".to_string()),
            "{:?}",
            branches.all
        );

        let dest = root.join("wt");
        super::add_worktree(&local, "nova", "origin/velha", &dest).unwrap();
        assert_eq!(run(&dest, &["rev-parse", "HEAD"]), velha);
        assert_eq!(run(&dest, &["rev-parse", "--abbrev-ref", "HEAD"]), "nova");

        // Base que não existe não vira worktree de lugar nenhum: dá erro.
        let erro = super::add_worktree(&local, "outra", "origin/fantasma", &root.join("wt2"));
        assert!(erro.unwrap_err().contains("fantasma"));

        // Pasta que já existe na branch pedida é reaproveitada — é o que faz
        // criar duas vezes o mesmo workspace não estourar.
        super::add_worktree(&local, "nova", "origin/velha", &dest).unwrap();
        // Mas na branch errada, não: adotar calado era o quadro passar a mentir
        // em que branch a sessão estava mexendo.
        let erro = super::add_worktree(&local, "outra-branch", "origin/main", &dest).unwrap_err();
        assert!(erro.contains("nova"), "{erro}");

        // Worktree desligado: a branch nasce no próprio clone, e é o HEAD dele
        // que anda. Nenhuma pasta nova, mesmo commit da base.
        super::switch_branch(&local, "aqui", "origin/velha").unwrap();
        assert_eq!(run(&local, &["rev-parse", "--abbrev-ref", "HEAD"]), "aqui");
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), velha);
        // Já estar na branch pedida é um no-op, não um erro.
        super::switch_branch(&local, "aqui", "origin/main").unwrap();
        assert_eq!(run(&local, &["rev-parse", "HEAD"]), velha);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A mesma branch em duas pastas é o que o git recusa — e é o que dois
    /// workspaces saídos da mesma issue do Linear pedem, um com um repositório
    /// e outro com dois: a branch é a mesma, a pasta não, porque ela leva os
    /// nomes dos repositórios do workspace. O erro tem de dizer onde a branch
    /// está, e a tentativa não pode deixar pasta vazia para trás.
    #[test]
    fn branch_aberta_em_outra_pasta_recusa_sem_deixar_pasta() {
        let root = std::env::temp_dir().join(format!("prometeu-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("code-rules");
        std::fs::create_dir_all(&repo).unwrap();

        let run = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&repo, &["init", "-q", "-b", "main"]);
        run(&repo, &["config", "user.email", "t@t"]);
        run(&repo, &["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        run(&repo, &["add", "-A"]);
        run(&repo, &["commit", "-qm", "a"]);

        // O workspace de um repositório só.
        let um = root.join("code-rules").join("aut-49");
        assert!(super::add_worktree(&repo, "aut-49", "", &um).unwrap());

        // O de dois: mesma branch, outra pasta.
        let dois = root.join("code-rules+autonomous").join("aut-49");
        let erro = super::add_worktree(&repo, "aut-49", "", &dois.join("code-rules")).unwrap_err();
        let onde = um.canonicalize().unwrap().display().to_string();
        assert!(erro.contains("aut-49") && erro.contains(&onde), "{erro}");
        assert!(
            !dois.exists(),
            "sobrou a pasta da tentativa: {}",
            dois.display()
        );
        assert!(!root.join("code-rules+autonomous").exists());

        // Pasta que já está na branch pedida continua sendo adotada — e adotar
        // não é criar: quem adota não é desfeito quando um irmão recusa.
        assert!(!super::add_worktree(&repo, "aut-49", "", &um).unwrap());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// O `setup` de cada repo entra num subshell com o caminho entre aspas:
    /// pasta com espaço ou apóstrofo não pode virar dois argumentos.
    #[test]
    fn quoted_aguenta_espaco_e_apostrofo() {
        assert_eq!(quoted("/a b"), "'/a b'");
        assert_eq!(quoted("/d'x"), "'/d'\\''x'");
    }

    /// Dois repos, cada um com o seu `setup`: o comando composto roda um depois
    /// do outro, cada um no seu worktree e vendo as suas variáveis — provado
    /// rodando o comando de verdade num `sh` e lendo o que cada um escreveu.
    #[test]
    fn setup_de_varios_repos_roda_cada_um_na_sua_pasta() {
        let root = std::env::temp_dir().join(format!("prometeu-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mk = |name: &str, setup: &str| {
            let repo = root.join("clones").join(name);
            let wt = root.join("ws").join(name);
            std::fs::create_dir_all(repo.join(".prometeu")).unwrap();
            std::fs::create_dir_all(&wt).unwrap();
            // String literal do TOML: o comando tem aspas duplas dentro.
            std::fs::write(
                repo.join(".prometeu/settings.toml"),
                format!("[scripts]\nsetup = '{setup}'\n"),
            )
            .unwrap();
            Repo {
                path: repo.display().to_string(),
                name: name.into(),
                worktree: wt.display().to_string(),
                base: String::new(),
                pr: None,
            }
        };
        let repos = vec![
            mk("back end", "echo \"$PROMETEU_WORKSPACE_PATH\" > saida.txt"),
            mk("front", "echo \"$PROMETEU_ROOT_PATH:$PORT\" > saida.txt"),
        ];
        let ws = super::Workspace {
            id: "w".into(),
            title: "t".into(),
            project: repos[0].path.clone(),
            repo: repos[0].path.clone(),
            repo_name: repos[0].name.clone(),
            branch: "b".into(),
            worktree: root.join("ws").display().to_string(),
            repos: repos.clone(),
            stage: "Fazendo".into(),
            agent: ProviderId::Claude,
            archived: false,
            pinned: false,
            unread: false,
            pr: None,
            cleaned: false,
            shared: false,
            audience: None,
            preparing: false,
            mcp: None,
            plugins: None,
            failed: None,
            model: String::new(),
            effort: String::new(),
            port: Some(3100),
            issue: None,
            tabs: Vec::new(),
            active: None,
        };

        let (header, command) = multi_setup(&ws).unwrap();
        // Nenhum dos dois declara cópia: não há cabeçalho.
        assert!(header.is_none());
        let out = Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let read =
            |r: &Repo| std::fs::read_to_string(Path::new(&r.worktree).join("saida.txt")).unwrap();
        assert_eq!(read(&repos[0]).trim(), repos[0].worktree);
        assert_eq!(read(&repos[1]).trim(), format!("{}:3100", repos[1].path));

        // Só um com setup ainda é uma aba; nenhum, não.
        std::fs::remove_file(Path::new(&repos[1].path).join(".prometeu/settings.toml")).unwrap();
        assert!(multi_setup(&ws).unwrap().1.contains("back end"));
        std::fs::remove_file(Path::new(&repos[0].path).join(".prometeu/settings.toml")).unwrap();
        assert!(multi_setup(&ws).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn separa_um_patch_por_arquivo() {
        let map = patch_map(DIFF);
        assert_eq!(map.len(), 3);

        let main = &map["src/main.ts"].body;
        assert!(main.starts_with("@@ -12,3 +12,4 @@ const $"), "{main}");
        assert!(main.contains("+let openWs: string | null = null;"));
        // Cabeçalho `index`/`---`/`+++` não entra: a tela não mostra.
        assert!(!main.contains("index 1c1c1c1"));
        assert!(!main.contains("--- a/src/main.ts"));
        assert!(!map["src/main.ts"].new && !map["src/main.ts"].deleted);

        // Apagado: o destino é /dev/null, então o caminho vem do `--- a/`, e o
        // `deleted file mode` do cabeçalho é a marca.
        assert!(map["src/old.ts"].body.contains("-export default gone;"));
        assert!(map["src/old.ts"].deleted);
        assert_eq!(map["docs/com espaco.md"].body.lines().count(), 3);
    }

    /// O front numera as abas de terminal, mas quem abre pty é o back: chave
    /// que não seja `terminal` ou `terminal-<número>` não pode virar shell,
    /// senão qualquer string entra no mapa de ptys com nome próprio.
    #[test]
    fn so_terminal_numerado_vira_shell() {
        assert!(is_terminal("terminal"));
        assert!(is_terminal("terminal-2"));
        assert!(is_terminal("terminal-10"));
        assert!(!is_terminal("terminal-"));
        assert!(!is_terminal("terminal-2x"));
        assert!(!is_terminal("terminalzinho"));
        assert!(!is_terminal("setup"));
        assert!(!is_terminal("run"));
    }
}

/// O git só pelo sim ou não da saída — `merge-base --is-ancestor` e afins, que
/// não escrevem nada e respondem no código de saída.
fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn expand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => paths::home().join(rest).display().to_string(),
        None => p.to_string(),
    }
}

/// O texto que o botão "Open PR" injeta na conversa ativa: o estado do git e
/// os passos até o PR. Sai daqui e não do front porque quem sabe a branch, o
/// alvo e o que falta commitar é quem tem o worktree. Quem commita, empurra e
/// cria o PR é o agente — e uma skill de PR do repositório, quando existe,
/// manda mais que este texto.
#[tauri::command(async)]
pub fn pr_prompt(state: State<AppState>, id: String) -> Result<String, String> {
    let repos = repos_of(&state, &id);
    if repos.is_empty() {
        return Err(i18n::t("err.session.noWorkspace"));
    }
    let states: Vec<RepoPr> = repos.iter().map(pr_state).collect();
    Ok(match states.as_slice() {
        [one] => pr_text(one),
        many => multi_pr_text(many),
    })
}

/// O que o pedido de PR precisa saber de um repositório: onde a branch está,
/// o que falta commitar, quantos commits ela tem além da base, e se já há um
/// PR aberto para ela.
struct RepoPr {
    name: String,
    branch: Option<String>,
    dirty: usize,
    ahead: u32,
    target: String,
    upstream: bool,
    open: Option<u64>,
}

fn pr_state(r: &Repo) -> RepoPr {
    let wt = Path::new(&r.worktree);
    let branch = head_branch(wt);
    let dirty = changes_in(wt).len();
    // O alvo é o principal do remoto; sem `origin/HEAD` gravado, o de sempre.
    let head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .trim()
        .to_string();
    let target = if head.is_empty() {
        "origin/main".to_string()
    } else {
        head
    };
    let upstream = !git(wt, &["rev-parse", "--abbrev-ref", "@{upstream}"])
        .trim()
        .is_empty();
    // Com PR já aberto o pedido é outro: não criar de novo, e sim empurrar o
    // que falta e conferir se o que está escrito lá ainda cobre a branch.
    let open = branch
        .as_deref()
        .and_then(|b| crate::github::pr_for_branch(wt, b))
        .filter(|pr| pr.open())
        .map(|pr| pr.number);
    let (_, ahead) = ahead_of(wt, &r.base);
    RepoPr {
        name: r.name.clone(),
        branch,
        dirty,
        ahead,
        target,
        upstream,
        open,
    }
}

fn pr_text(s: &RepoPr) -> String {
    let estado = match s.dirty {
        0 => "O worktree está limpo — nada fora de commit.".to_string(),
        1 => "Há 1 arquivo com mudanças fora de commit.".to_string(),
        n => format!("Há {n} arquivos com mudanças fora de commit."),
    };
    let onde = match &s.branch {
        Some(b) => format!("A branch atual é `{b}`"),
        None => "O worktree está em HEAD solto — crie uma branch antes de commitar".to_string(),
    };
    let target = &s.target;
    let up = if s.upstream {
        "A branch já tem upstream."
    } else {
        "Ainda não há branch upstream."
    };
    let base = target.split_once('/').map_or(target.as_str(), |(_, b)| b);
    let push = match &s.branch {
        Some(b) => format!("git push -u origin HEAD:{b}"),
        None => "git push -u origin HEAD:<nome-da-branch>".to_string(),
    };
    let (abertura, fim) = match s.open {
        Some(n) => (
            format!("Quero atualizar o PR #{n} desta branch."),
            format!(
                "5. Revise o diff inteiro da branch contra `{target}` e confira com `gh pr view {n}` se o título e a \
                 descrição ainda cobrem tudo.\n6. Se não cobrirem mais, atualize com `gh pr edit {n}`. Título com \
                 menos de 80 caracteres; descrição com até cinco frases, cobrindo todas as mudanças da branch — não \
                 só as desta conversa."
            ),
        ),
        None => (
            "Quero abrir um PR deste worktree.".to_string(),
            format!(
                "5. Revise o diff inteiro da branch contra `{target}` antes de escrever o PR.\n6. Crie o PR com \
                 `gh pr create --base {base}`. Título com menos de 80 caracteres; descrição com até cinco frases, \
                 cobrindo todas as mudanças da branch — não só as desta conversa."
            ),
        ),
    };
    format!(
        r#"{abertura}

{estado} {onde}; o alvo é `{target}`. {up}

Siga estes passos:

1. Se este repositório tiver uma skill ou comando de abrir PR (ex.: /open-pr), invoque-a agora — as instruções dela têm precedência sobre as daqui.
2. Revise o que está fora de commit com `git status` e `git diff`.
3. Commite seguindo as convenções de commit do repositório.
4. Empurre com `{push}`.
{fim}

Se algum passo falhar, pare e me pergunte."#
    )
}

/// Workspace com mais de um repositório: um PR por repo que tem o que
/// entregar, na mesma branch, e cada descrição linkando os outros — é assim
/// que uma funcionalidade que atravessa repositórios se revisa no GitHub.
fn multi_pr_text(states: &[RepoPr]) -> String {
    let lista: String = states
        .iter()
        .map(|s| {
            let branch = match &s.branch {
                Some(b) => format!("branch `{b}`"),
                None => "HEAD solto (crie a branch antes de commitar)".to_string(),
            };
            let commits = match s.ahead {
                0 => format!("nenhum commit além de `{}`", s.target),
                1 => format!("1 commit além de `{}`", s.target),
                n => format!("{n} commits além de `{}`", s.target),
            };
            let dirty = match s.dirty {
                0 => "nada fora de commit".to_string(),
                1 => "1 arquivo fora de commit".to_string(),
                n => format!("{n} arquivos fora de commit"),
            };
            let pr = match s.open {
                Some(n) => format!("PR #{n} aberto — atualize, não crie outro"),
                None if s.ahead == 0 && s.dirty == 0 => "nada a entregar: fica sem PR".to_string(),
                None => "sem PR ainda".to_string(),
            };
            let up = if s.upstream { "" } else { ", sem upstream" };
            format!(
                "- `{}/` — {branch}, {commits}, {dirty}{up}. {pr}.\n",
                s.name
            )
        })
        .collect();
    format!(
        r#"Quero abrir os PRs deste workspace. Ele reúne {n} repositórios na mesma branch, cada um com histórico próprio — e cada um leva o seu PR:

{lista}
Siga estes passos em cada repositório que tem o que entregar, entrando na pasta dele antes de cada comando de git ou gh:

1. Se o repositório tiver uma skill ou comando de abrir PR (ex.: /open-pr), invoque-a nele — as instruções dela têm precedência sobre as daqui.
2. Revise o que está fora de commit com `git status` e `git diff`, e commite seguindo as convenções daquele repositório.
3. Empurre com `git push -u origin HEAD:<branch>`.
4. Revise o diff inteiro da branch contra o alvo daquele repositório antes de escrever.
5. Sem PR: crie com `gh pr create --base <alvo sem o origin/>`. Com PR aberto: confira com `gh pr view <n>` se o título e a descrição ainda cobrem tudo, e atualize com `gh pr edit <n>` se não cobrirem. Título com menos de 80 caracteres; descrição com até cinco frases, cobrindo todas as mudanças da branch naquele repositório — não só as desta conversa.
6. Com todos abertos, edite a descrição de cada um para linkar os outros PRs deste workspace ("Parte de: <url>"): quem revisa um precisa achar o resto.

Repositório sem commit além da base e sem mudança fora de commit não ganha PR.
Se algum passo falhar, pare e me pergunte."#,
        n = states.len()
    )
}

/// O worktree do repositório principal, se ainda houver um — é dele que saem
/// diff, branch e PR. Devolvido ao disco é o mesmo que não existir: quem lê
/// daqui recebe o vazio, e não um caminho que já não é de ninguém.
fn workspace_copy(state: &State<AppState>, id: &str) -> Option<Workspace> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id)
        .cloned()
}

fn worktree_of(state: &State<AppState>, id: &str) -> Option<PathBuf> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| PathBuf::from(w.primary().worktree))
}

/// Os repositórios de um workspace que ainda tem worktree, na ordem dele — o
/// principal primeiro. Devolvido ao disco é lista vazia, pela mesma razão de
/// `worktree_of`.
fn repos_of(state: &State<AppState>, id: &str) -> Vec<Repo> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| w.repos.clone())
        .unwrap_or_default()
}

/// Onde o agente trabalha: o worktree, ou a pasta que reúne os worktrees
/// quando há mais de um repositório. É daqui que a árvore de arquivos e o
/// Finder partem — a pessoa quer ver todos, não só o principal.
fn cwd_of(state: &State<AppState>, id: &str) -> Option<PathBuf> {
    lock(&state.board)
        .workspaces
        .iter()
        .find(|w| w.id == id && !w.cleaned)
        .map(|w| PathBuf::from(&w.worktree))
}
