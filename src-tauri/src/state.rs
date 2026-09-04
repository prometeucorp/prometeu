//! Três camadas, como no Conductor:
//!
//!   Projeto    um repositório registrado uma vez
//!     └ Workspace   uma branch, num worktree por repositório — é o card do quadro
//!         └ Aba     uma sessão de agente (Claude Code ou Codex); várias dividem
//!                   os mesmos arquivos
//!
//! A separação existe porque perder uma conversa não pode custar o worktree, e
//! começar conversa nova sobre os arquivos que você já mexeu tem que ser ⌘T.

use crate::lock::lock;
use crate::{paths, AppState};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Identidade persistida do runtime de agente. Antes deste tipo, Claude era
/// gravado como string vazia; o `Deserialize` abaixo aceita esse legado e
/// normaliza a próxima gravação para `"claude"`. Valor desconhecido também cai
/// no default durante a migração, para uma versão nova não inutilizar o board
/// inteiro ao ser aberto por uma versão antiga do app.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    #[default]
    Claude,
    Codex,
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "codex" => Self::Codex,
            "" | "claude" => Self::Claude,
            _ => Self::default(),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Agente trabalhando.
    Rodando,
    /// Processo vivo, esperando o que você mandar.
    Pronta,
    /// Agente travado numa pergunta: não anda sem você.
    Querendo,
    /// Processo não está rodando. Não é morte: o transcript continua no disco e
    /// `claude --resume` traz a conversa de volta. Toda aba vira isso quando o
    /// app abre, porque nenhum processo sobrevive ao app.
    /// `other` cobre valores gravados por versões anteriores — e obriga esta a
    /// ser a última variante, por isso a urgência vive no `rank` e não na ordem.
    #[serde(other)]
    Desligada,
}

impl Status {
    /// Urgência. Um workspace mostra o maior entre suas abas: uma aba travada
    /// numa pergunta manda no card inteiro, senão o quadro esconderia o único
    /// número que muda o que você faz de manhã.
    pub fn rank(self) -> u8 {
        match self {
            Status::Querendo => 3,
            Status::Rodando => 2,
            Status::Pronta => 1,
            Status::Desligada => 0,
        }
    }
}

/// O que a linha de atividade da aba passa a dizer.
///
/// É um `Option<String>` com nome, e o nome é o ponto: antes o `None` que
/// chegava em `set` queria dizer "não mexe", então a aba que terminava
/// continuava mostrando a última ferramenta que rodou — o card dizia "pronta"
/// embaixo de uma linha que parecia trabalho acontecendo agora. Agora todo
/// evento diz explicitamente qual das duas coisas quer.
pub enum Note {
    /// Não há mais o que dizer: o trabalho parou.
    Clear,
    Set(String),
    /// O que estava escrito continua valendo: o agente falou no meio de uma
    /// ferramenta e outra, e a ferramenta é o que a linha conta.
    Keep,
}

/// Com quem uma conversa fala: qual CLI sobe, com que modelo e com quanto
/// esforço. No workspace isto são três campos soltos, porque nasceram nele; na
/// aba é um só, para que "escolheu o seu" e "segue o do workspace" sejam duas
/// coisas — modelo vazio é uma escolha (o padrão do CLI), e não a falta de uma.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Choice {
    #[serde(default)]
    pub agent: ProviderId,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub effort: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Tab {
    /// É o `--session-id` do Claude Code. O transcript pendura nele.
    pub id: String,
    /// A sessão do lado do agente, quando ela não é este id. O Codex não aceita
    /// `--session-id`: escolhe o dele, conta qual foi no hook `SessionStart`, e é
    /// este número que volta como `codex resume <id>`. Vazio é aba do Claude
    /// Code (onde os dois ids são o mesmo) ou aba do Codex que ainda não subiu.
    #[serde(default)]
    pub agent_session: Option<String>,
    pub title: String,
    pub status: Status,
    pub note: Option<String>,
    pub pending_prompt: Option<String>,
    /// Tokens de contexto na última resposta — quão cheia a janela está.
    /// Atualizado quando o agente para (`Stop`), que é quando muda. Vazio é
    /// conversa que ainda não respondeu, ou quadro gravado antes disto existir.
    #[serde(default)]
    pub tokens: Option<u64>,
    /// O modelo desta conversa, quando ela fala com um diferente do que o
    /// workspace usa — escolhido ao abrir a aba, ou depois, no rodapé da caixa
    /// (`Workspace::retune`). `None` é seguir o do workspace: é o que faz ⌘T, o
    /// que toda aba gravada antes disto existir traz, e o que volta a valer
    /// quando alguém escolhe de novo o modelo do workspace. Retomar a aba
    /// respeita o que está aqui.
    #[serde(default)]
    pub choice: Option<Choice>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

/// Um repositório dentro de um workspace: de onde ele veio e onde está a cópia
/// dele nesta branch. Workspace de um repositório só tem um destes; com mais
/// de um, cada repo ganha um worktree seu, lado a lado, na mesma branch.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Repo {
    /// O clone registrado como projeto.
    pub path: String,
    /// O nome da pasta do clone — é o que a tela mostra e o nome da pasta do
    /// worktree.
    pub name: String,
    /// Onde este repositório está nesta branch: o worktree, ou o próprio clone
    /// quando o workspace roda nele.
    pub worktree: String,
    /// De onde a branch saiu neste repositório — é contra ela que a tela de
    /// mudanças conta commits e diff. Cada repo tem a sua: o principal a que
    /// o lançador escolheu, os outros o padrão de cada clone. Vazio é quadro
    /// gravado antes disto existir; o diff aprende o padrão e grava.
    #[serde(default)]
    pub base: String,
    /// O PR desta branch neste repositório, como o `gh` respondeu da última
    /// vez. É o quadro que guarda porque é o quadro que desenha: o selo de
    /// mergeado no card e os botões da barra saem daqui, e uma resposta de
    /// minutos atrás vale mais que uma consulta à rede a cada redesenho.
    /// Vazio é "não perguntei ainda" e "esta branch não tem PR aqui" — para a
    /// tela dá no mesmo. Um por repo: histórico separado, PR separado.
    #[serde(default)]
    pub pr: Option<crate::domain::Pr>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Workspace {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project: String,
    /// O repositório principal — o primeiro de `repos`. Continua aqui, e não
    /// só na lista, porque é o que todo quadro gravado até hoje tem: é dele que
    /// o diff, o PR e os scripts saem enquanto não olham para os outros.
    pub repo: String,
    pub repo_name: String,
    pub branch: String,
    /// Onde o agente trabalha. Com um repositório só, é o worktree dele (ou o
    /// clone); com mais de um, é a pasta que reúne o worktree de cada repo —
    /// e é dela que a árvore de arquivos e o shell do dock partem.
    pub worktree: String,
    /// Os repositórios deste workspace, o principal primeiro. Quadro gravado
    /// antes disto existir vem sem a lista, e o `revive` monta a de um item a
    /// partir de `repo` e `worktree` — nada muda para quem tinha um só.
    #[serde(default)]
    pub repos: Vec<Repo>,
    /// Onde o trabalho está — o que o quadro desenhava como coluna. É seu, não
    /// do processo: `Status` é o que o agente está fazendo agora, `stage` é o
    /// que você decidiu sobre o trabalho. `column` é o nome antigo.
    #[serde(alias = "column")]
    pub stage: String,
    /// Fora da lista, mas nada foi perdido: worktree, branch e transcript
    /// continuam onde estavam.
    #[serde(default)]
    pub archived: bool,
    /// No topo da lista. Não é etapa nem atividade: é "é neste que eu volto".
    #[serde(default)]
    pub pinned: bool,
    /// Aconteceu algo aqui enquanto você olhava outra coisa.
    #[serde(default)]
    pub unread: bool,
    /// O `--model` das conversas deste workspace — um alias (`opus`,
    /// `sonnet[1m]`) ou o nome inteiro. Vazio é "o que o Claude Code escolheria
    /// sozinho". É do workspace e não da aba porque ⌘T e retomar nascem com o
    /// mesmo modelo que as irmãs: trocar de modelo no meio é trocar de worktree.
    /// Quadro gravado antes disto existir vem sem o campo, e vazio é o que ele
    /// fazia então.
    #[serde(default)]
    pub model: String,
    /// O `--effort` (`low`…`max`, `ultracode`), pela mesma regra. Vazio é o
    /// padrão — só de quadro antigo: o lançador sempre escolhe um.
    #[serde(default)]
    pub effort: String,
    /// Qual CLI roda nas abas daqui. Boards antigos em que vazio significava
    /// Claude são normalizados por `ProviderId`. Sai do modelo escolhido no lançador — quem
    /// escolhe um GPT escolheu o Codex —, e é do workspace pelo mesmo motivo do
    /// modelo: ⌘T e retomar nascem com o agente das irmãs.
    #[serde(default)]
    pub agent: ProviderId,
    /// Base das dez portas reservadas a este worktree — `$PROMETEU_PORT` até
    /// `+9`. Guardada e não calculada: o script tem que achar a mesma porta na
    /// segunda vez que roda, e dois worktrees do mesmo projeto não podem
    /// disputar a mesma. Nasce vazia em quadro gravado antes disto existir, e é
    /// preenchida na primeira vez que o workspace é aberto — o painel pede os
    /// scripts, e a porta vai junto, porque é ela que ele mostra.
    #[serde(default)]
    pub port: Option<u16>,
    /// A issue do Linear de onde este trabalho saiu, se saiu de uma. O card
    /// mostra o identificador, e a aba de issues sabe que esta já tem dono.
    #[serde(default)]
    pub issue: Option<crate::linear::IssueRef>,
    /// Onde o PR morava quando o workspace só tinha um repositório. Quadro
    /// gravado por versão anterior ainda o traz aqui, e o `revive` o leva para
    /// o principal — que é de quem ele sempre foi. Nunca mais é gravado.
    #[serde(default, skip_serializing)]
    pub pr: Option<crate::domain::Pr>,
    /// O worktree foi devolvido ao disco: a pasta não existe mais e a branch
    /// local foi apagada. O card fica como histórico — transcript, o número do
    /// PR, o caminho que era —, mas nada aqui abre terminal de novo.
    #[serde(default)]
    pub cleaned: bool,
    /// Compartilhado com o time: o front anuncia este workspace ao relay e
    /// repassa a saída das conversas a quem estiver olhando. Persistido para o
    /// dono que fecha o app voltar compartilhando, sem ninguém pedir de novo.
    #[serde(default)]
    pub shared: bool,
    /// Com quem: ids de membros do time, ou `None` para o time inteiro. Só
    /// vale com `shared`. O back não sabe quem são — é o front que anuncia e
    /// o relay que faz valer.
    #[serde(default)]
    pub audience: Option<Vec<String>>,
    /// O worktree ainda está sendo montado. O card entra no quadro assim que o
    /// lançador fecha e o `git worktree add` — segundos, num repositório
    /// grande — acontece atrás. Enquanto isto for verdade não há aba nenhuma:
    /// o agente só nasce depois de existir pasta onde rodar.
    #[serde(default)]
    pub preparing: bool,
    /// Por que a montagem não deu, no formato do `i18n` — quem monta a frase é
    /// o `fromBack`. O card fica, com o erro escrito, em vez de sumir: a branch
    /// pedida pode estar viva em outro worktree, e é olhando o card que se
    /// decide o que fazer com ela.
    #[serde(default)]
    pub failed: Option<String>,
    /// Quais servidores de MCP as conversas daqui enxergam, pelo nome que eles
    /// têm no hub (`mcp.rs`). É do workspace pelo mesmo motivo do modelo: a
    /// ferramenta que o agente tem na mão é do trabalho, não da aba.
    ///
    /// `None` é workspace que nunca escolheu — todo quadro gravado antes disto
    /// existir —, e aí nada é imposto ao CLI: vale o que ele já fazia. Lista
    /// vazia é escolha de verdade, e quer dizer sessão sem MCP nenhum.
    #[serde(default)]
    pub mcp: Option<Vec<String>>,
    /// Quais plugins do Claude Code as conversas daqui carregam, pelo nome que
    /// eles têm no hub (`plugins.rs`). É do workspace pelo mesmo motivo do
    /// MCP: o hook que segura o jeito de trabalhar é do trabalho, não da aba.
    ///
    /// `None` é workspace que nunca escolheu, e aí nada é passado ao CLI —
    /// vale o que ele já carregava sozinho. Lista vazia é escolha, e quer
    /// dizer nenhum plugin a mais do que isso.
    #[serde(default)]
    pub plugins: Option<Vec<String>>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub active: Option<String>,
}

impl Workspace {
    /// O repositório principal: o primeiro da lista. Workspace construído sem
    /// a lista (quadro velho antes do `revive`, ou um teste) responde com os
    /// campos soltos, que dizem a mesma coisa.
    pub fn primary(&self) -> Repo {
        self.repos.first().cloned().unwrap_or_else(|| Repo {
            path: self.repo.clone(),
            name: self.repo_name.clone(),
            base: String::new(),
            pr: None,
            worktree: self.worktree.clone(),
        })
    }

    /// Os PRs desta branch: um por repositório que tem o seu, na ordem do
    /// workspace.
    pub fn prs(&self) -> impl Iterator<Item = (&Repo, &crate::domain::Pr)> {
        self.repos
            .iter()
            .filter_map(|r| r.pr.as_ref().map(|pr| (r, pr)))
    }

    /// O trabalho entrou: todo repositório que tem PR tem o PR mergeado, e há
    /// pelo menos um. É o que faz a barra oferecer "Concluir" e o card ganhar
    /// o selo.
    pub fn merged(&self) -> bool {
        let mut any = false;
        for (_, pr) in self.prs() {
            if !pr.merged() {
                return false;
            }
            any = true;
        }
        any
    }

    /// Mais de um repositório: `worktree` é a pasta que os reúne, e não um
    /// worktree de git.
    pub fn multi(&self) -> bool {
        self.repos.len() > 1
    }

    /// O card mostra o estado mais urgente entre as abas.
    pub fn status(&self) -> Status {
        self.tabs
            .iter()
            .map(|t| t.status)
            .max_by_key(|s| s.rank())
            .unwrap_or(Status::Desligada)
    }

    /// A linha de atividade vem da aba mais urgente, pelo mesmo motivo.
    pub fn note(&self) -> Option<String> {
        self.tabs
            .iter()
            .max_by_key(|t| t.status.rank())
            .and_then(|t| t.note.clone())
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Board {
    /// A sequência de etapas, na ordem. O ícone de cada uma sai da posição
    /// nela, então trocar a lista troca os ícones — sem tabela para manter.
    #[serde(alias = "columns")]
    pub stages: Vec<String>,
    #[serde(default)]
    pub projects: Vec<Project>,
    /// `cards` era o nome antigo, quando workspace e sessão eram a mesma coisa.
    #[serde(alias = "cards")]
    pub workspaces: Vec<Workspace>,
}

impl Default for Board {
    fn default() -> Self {
        Board {
            stages: ["Preparando", "Fazendo", "Code review", "Travado", "Feito"]
                .map(String::from)
                .to_vec(),
            projects: Vec::new(),
            workspaces: Vec::new(),
        }
    }
}

impl Board {
    pub fn load() -> Board {
        let current = path();
        let mut board = Self::load_at(&current);
        board.revive();
        board
    }

    fn load_at(current: &std::path::Path) -> Board {
        let backup = current.with_extension("json.bak");
        let read = |candidate: &std::path::Path| -> Result<Board, String> {
            let text = std::fs::read_to_string(candidate).map_err(|error| error.to_string())?;
            serde_json::from_str(&text).map_err(|error| error.to_string())
        };
        match read(current) {
            Ok(board) => board,
            Err(error) => match read(&backup) {
                Ok(board) => {
                    if current.exists() {
                        eprintln!(
                            "board.json inválido ({error}); tentando backup {}",
                            backup.display()
                        );
                    }
                    board
                }
                Err(backup_error) => {
                    if current.exists() || backup.exists() {
                        eprintln!("board.json e backup inválidos: {error}; {backup_error}");
                    }
                    Board::default()
                }
            },
        }
    }

    /// O que um quadro gravado precisa antes de virar o quadro de hoje: nada
    /// que estava vivo continua vivo, quadro velho ganha o que passou a
    /// existir, e o que ficou pela metade é dito em voz alta.
    ///
    /// Separado do `load` para poder ser testado: `load` lê de `paths::root()`,
    /// que sai de uma variável de ambiente — e ambiente é global, enquanto o
    /// cargo roda cada teste numa thread.
    pub(crate) fn revive(&mut self) {
        for ws in &mut self.workspaces {
            // O app fechou no meio da montagem. A thread que montava morreu com
            // o processo, então continuar dizendo "montando" seria esperar por
            // quem não vai voltar — e o worktree pode ter ficado pela metade.
            if ws.preparing {
                ws.preparing = false;
                ws.failed = Some(crate::i18n::t("err.session.interrupted"));
            }
            // Quadro gravado antes das abas existirem: o id do card era o id da
            // sessão, então ele vira a primeira aba e nada se perde. Workspace
            // que nunca chegou a montar não é disso: ele não tem aba porque
            // nenhuma nasceu, e inventar uma daria um "Retomar" que não retoma.
            if ws.tabs.is_empty() && ws.failed.is_none() {
                ws.tabs.push(Tab {
                    id: ws.id.clone(),
                    agent_session: None,
                    title: "conversa".into(),
                    status: Status::Desligada,
                    note: None,
                    pending_prompt: None,
                    tokens: None,
                    choice: None,
                });
            }
            // Nenhum PTY sobrevive ao fechamento do app, então qualquer status
            // gravado como vivo é mentira.
            for tab in &mut ws.tabs {
                tab.status = Status::Desligada;
            }
            if ws.active.is_none() {
                ws.active = ws.tabs.first().map(|t| t.id.clone());
            }
            if ws.project.is_empty() {
                ws.project = ws.repo.clone();
            }
            // Quadro gravado antes de um workspace poder ter mais de um
            // repositório: o único que ele tem é o principal, e o worktree é o
            // dele. É o que o app instalado encontra na primeira abertura
            // depois de atualizar — e nada além da lista muda.
            if ws.repos.is_empty() {
                ws.repos.push(Repo {
                    path: ws.repo.clone(),
                    name: ws.repo_name.clone(),
                    worktree: ws.worktree.clone(),
                    base: String::new(),
                    pr: None,
                });
            }
            // O PR morava no workspace enquanto ele só tinha um repositório:
            // passa para o principal, que é de quem ele sempre foi.
            if let Some(pr) = ws.pr.take() {
                if let Some(main) = ws.repos.first_mut() {
                    main.pr.get_or_insert(pr);
                }
            }
        }

        // Etapa gravada que não está mais na lista deixaria o workspace fora de
        // todo grupo — invisível. Volta para a primeira.
        let first = self.stages.first().cloned().unwrap_or_default();
        let stages = self.stages.clone();
        for ws in &mut self.workspaces {
            if !stages.contains(&ws.stage) {
                ws.stage = first.clone();
            }
        }

        // Repositório que já tem workspace é projeto, mesmo que nunca tenha sido
        // registrado à mão.
        for ws in self.workspaces.clone() {
            if !self.projects.iter().any(|p| p.path == ws.repo) {
                self.projects.push(Project {
                    id: ws.repo.clone(),
                    name: ws.repo_name.clone(),
                    path: ws.repo.clone(),
                });
            }
        }
    }

    /// Grava num arquivo ao lado e renomeia por cima. `rename` é atômico no
    /// mesmo sistema de arquivos, então nunca existe um `board.json` cortado no
    /// meio — e um quadro cortado no meio não volta a carregar.
    pub fn save(&self) -> Result<(), String> {
        let path = path();
        let json = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        // O backup só recebe um quadro que ainda desserializa. Se o arquivo
        // atual foi corrompido fora do app, preservá-lo por cima do último bom
        // tiraria justamente a rota de recuperação.
        if let Ok(previous) = std::fs::read_to_string(&path) {
            if serde_json::from_str::<Board>(&previous).is_ok() {
                paths::write_private(&path.with_extension("json.bak"), &previous)?;
            }
        }
        paths::write_private(&path, &json)
    }

    pub fn workspace_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    /// Procura a aba pelo id da sessão — é assim que um hook, que só conhece o
    /// `session_id`, encontra onde escrever.
    pub fn tab_mut(&mut self, session: &str) -> Option<&mut Tab> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .find(|t| t.id == session)
    }

    /// O workspace dono da sessão, para escrever nele — é assim que um hook,
    /// que só conhece o `session_id`, marca novidade no card certo.
    pub fn workspace_of_mut(&mut self, session: &str) -> Option<&mut Workspace> {
        self.workspaces
            .iter_mut()
            .find(|w| w.tabs.iter().any(|t| t.id == session))
    }

    pub fn workspace_of(&self, session: &str) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|w| w.tabs.iter().any(|t| t.id == session))
    }
}

fn path() -> std::path::PathBuf {
    paths::root().join("board.json")
}

/* ---------- publicar ---------- */

/// Manda o quadro para a tela e para o disco. Único caminho: quem mexe no
/// quadro mexe sob o lock e chama isto depois.
///
/// O lock sai antes de qualquer I/O. Antes ele ficava tomado durante o
/// `serde_json` e o `write`, e como isto roda **a cada ferramenta que o agente
/// usa**, cada tool call de cada sessão parava as outras para esperar o disco.
pub fn publish(app: &AppHandle) {
    let state = app.state::<AppState>();
    let board = Arc::new(lock(&state.board).clone());
    if state.save.later(board.clone()).is_err() {
        // A thread de persistência morreu: não transformar isso em perda
        // silenciosa. O caminho raro paga a gravação síncrona.
        if let Err(error) = board.save() {
            eprintln!("não gravei board.json depois de perder o saver: {error}");
        }
    }
    if let Err(error) = app.emit("board", &*board) {
        eprintln!("não publiquei o quadro para a webview: {error}");
    }
}

/// Pede uma gravação imediata à thread de persistência e espera a confirmação.
/// É o que fecha a janela de perda que a gravação adiada abre: o app morrendo
/// dentro do `COALESCE` levaria junto a última mudança. Chamado na saída,
/// quando não há mais depois.
pub fn save_now(app: &AppHandle) {
    let state = app.state::<AppState>();
    let board = Arc::new(lock(&state.board).clone());
    if let Err(error) = state.save.now(board.clone()) {
        eprintln!("o saver não confirmou board.json ao sair: {error}");
        if let Err(error) = board.save() {
            eprintln!("não gravei board.json ao sair: {error}");
        }
    }
}

/// Junta as gravações numa só. Uma sessão ativa dispara dezenas de eventos por
/// minuto, e o quadro inteiro cabe num write — então o que importa é gravar o
/// **último**, não todos.
const COALESCE: Duration = Duration::from_millis(250);

/// A thread que grava. Recebe o quadro por canal, espera a poeira assentar e
/// escreve uma vez só o estado mais recente que chegou.
enum Save {
    Later(Arc<Board>),
    Now(Arc<Board>, Sender<Result<(), String>>),
}

/// A fila de persistência sabe distinguir o caminho quente de um flush. O
/// flush passa pela mesma thread e espera a confirmação; assim uma gravação
/// antiga que já estava dormindo no coalesce nunca acorda depois da saída para
/// sobrescrever o estado mais novo.
#[derive(Clone)]
pub struct Saver {
    tx: Sender<Save>,
}

impl Saver {
    fn later(&self, board: Arc<Board>) -> Result<(), ()> {
        self.tx.send(Save::Later(board)).map_err(|_| ())
    }

    fn now(&self, board: Arc<Board>) -> Result<(), String> {
        let (tx, rx) = channel();
        self.tx
            .send(Save::Now(board, tx))
            .map_err(|_| "thread de persistência encerrada".to_string())?;
        // Não há timeout de propósito. Fazer uma segunda gravação enquanto a
        // primeira ainda está no disco reabriria exatamente a corrida que o
        // flush resolve: a antiga poderia terminar por último. A thread não
        // usa unwrap e sempre responde, inclusive quando o write falha.
        rx.recv().map_err(|error| error.to_string())?
    }
}

pub fn spawn_saver() -> Saver {
    spawn_saver_with(Board::save)
}

fn spawn_saver_with<F>(save: F) -> Saver
where
    F: Fn(&Board) -> Result<(), String> + Send + 'static,
{
    let (tx, rx) = channel::<Save>();
    std::thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let (mut board, mut flushes) = match first {
                Save::Later(board) => {
                    std::thread::sleep(COALESCE);
                    (board, Vec::new())
                }
                Save::Now(board, done) => (board, vec![done]),
            };
            // Tudo que chegou durante a espera: só o último quadro interessa.
            // Todos os flushes recebem o resultado dessa mesma gravação.
            while let Ok(newer) = rx.try_recv() {
                match newer {
                    Save::Later(next) => board = next,
                    Save::Now(next, done) => {
                        board = next;
                        flushes.push(done);
                    }
                }
            }
            let result = save(board.as_ref());
            if let Err(error) = &result {
                eprintln!("não gravei board.json: {error}");
            }
            if !flushes.is_empty() {
                for done in flushes {
                    let _ = done.send(result.clone());
                }
                break;
            }
        }
    });
    Saver { tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// O mínimo que um workspace precisa no `board.json`: todo o resto tem
    /// `serde(default)`, e é justamente isso que um quadro velho aproveita.
    fn board_json(extra: &str) -> Board {
        let json = format!(
            r#"{{"stages":["Fazendo"],"projects":[],"workspaces":[
                 {{"id":"w","title":"t","repo":"/r","repo_name":"r",
                   "branch":"b","worktree":"/wt","stage":"Fazendo"{extra}}}]}}"#
        );
        serde_json::from_str(&json).expect("board não desserializou")
    }

    fn temporary_board_path() -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("prometeu-board-{}", uuid::Uuid::new_v4()))
            .join("board.json")
    }

    #[test]
    fn flush_descarta_o_quadro_antigo_que_esperava_no_coalesce() {
        let written = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = written.clone();
        let saver = spawn_saver_with(move |board| {
            observed.lock().unwrap().push(board.stages[0].clone());
            Ok(())
        });
        let old = Board {
            stages: vec!["antigo".into()],
            ..Board::default()
        };
        let final_board = Board {
            stages: vec!["final".into()],
            ..Board::default()
        };

        saver.later(Arc::new(old)).unwrap();
        saver.now(Arc::new(final_board)).unwrap();

        assert_eq!(*written.lock().unwrap(), vec!["final"]);
    }

    #[test]
    fn quadro_corrompido_recupera_o_ultimo_backup_valido() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, "{cortado").unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = Board::load_at(&current);

        assert_eq!(recovered.stages, vec!["Fazendo"]);
        assert_eq!(recovered.workspaces.len(), 1);
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn backup_tambem_recupera_quadro_principal_ausente() {
        let current = temporary_board_path();
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(
            current.with_extension("json.bak"),
            serde_json::to_string(&board_json("")).unwrap(),
        )
        .unwrap();

        let recovered = Board::load_at(&current);

        assert_eq!(recovered.stages, vec!["Fazendo"]);
        assert_eq!(recovered.workspaces[0].id, "w");
        std::fs::remove_dir_all(current.parent().unwrap()).unwrap();
    }

    #[test]
    fn provider_legado_e_normalizado_sem_quebrar_o_board() {
        let vazio = board_json(r#","agent":"""#);
        let explicito = board_json(r#","agent":"codex""#);
        let futuro = board_json(r#","agent":"provider-ainda-desconhecido""#);

        assert_eq!(vazio.workspaces[0].agent, ProviderId::Claude);
        assert_eq!(explicito.workspaces[0].agent, ProviderId::Codex);
        assert_eq!(futuro.workspaces[0].agent, ProviderId::Claude);

        let normalized = serde_json::to_value(vazio).unwrap();
        assert_eq!(normalized["workspaces"][0]["agent"], "claude");
    }

    /// O app fechou no meio de montar um worktree. Voltar dizendo "montando"
    /// seria esperar por uma thread que morreu junto com o processo.
    #[test]
    fn montagem_interrompida_vira_erro_escrito_no_card() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        let ws = &board.workspaces[0];
        assert!(!ws.preparing, "não pode voltar montando");
        assert_eq!(
            ws.failed.as_deref(),
            Some(crate::i18n::t("err.session.interrupted")).as_deref()
        );
    }

    /// E não inventa aba para ele: a migração que dá uma aba a quadro antigo é
    /// para quem teve sessão, não para quem nunca chegou a ter pasta. Uma aba
    /// aqui daria um "Retomar conversa" que não retoma nada.
    #[test]
    fn montagem_interrompida_nao_ganha_aba() {
        let mut board = board_json(r#","preparing":true"#);
        board.revive();
        assert!(board.workspaces[0].tabs.is_empty());
        assert!(board.workspaces[0].active.is_none());
    }

    /// O quadro que já existia continua ganhando a aba de migração — o campo
    /// novo não pode mudar o que acontece com quem foi gravado sem ele.
    #[test]
    fn quadro_antigo_sem_aba_continua_ganhando_a_sua() {
        let mut board = board_json("");
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.tabs[0].id, "w");
        assert_eq!(ws.active.as_deref(), Some("w"));
        assert!(ws.failed.is_none());
    }

    /// Quadro gravado por uma versão em que workspace tinha um repositório só:
    /// a lista nasce com ele, e `repo`/`worktree` ficam exatamente como
    /// estavam — é isso que faz atualizar o app não quebrar workspace nenhum.
    #[test]
    fn quadro_antigo_ganha_a_lista_de_um_repositorio() {
        let mut board = board_json("");
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.repo, "/r");
        assert_eq!(ws.worktree, "/wt");
        assert_eq!(
            ws.repos,
            vec![Repo {
                path: "/r".into(),
                name: "r".into(),
                worktree: "/wt".into(),
                base: String::new(),
                pr: None
            }]
        );
        assert_eq!(ws.primary().worktree, "/wt");
        assert!(!ws.multi());
    }

    /// Quadro gravado por uma versão em que o PR era do workspace: ele passa
    /// para o repositório principal, e o campo antigo não é gravado de novo.
    #[test]
    fn quadro_antigo_leva_o_pr_para_o_principal() {
        let mut board = board_json(r#","pr":{"number":3,"title":"t","state":"MERGED"}"#);
        board.revive();
        let ws = &board.workspaces[0];
        assert!(ws.pr.is_none());
        assert_eq!(ws.repos[0].pr.as_ref().map(|p| p.number), Some(3));
        assert!(ws.merged());
        let json = serde_json::to_string(&board).unwrap();
        assert!(!json.contains(r#""pr":{"number":3"#) || json.contains(r#""repos":[{"#));
        assert!(!json.contains(r#""stage":"Fazendo","pr""#));
    }

    /// E quadro que já tem a lista não ganha item de novo — nem perde os que
    /// tem.
    #[test]
    fn quadro_com_lista_fica_como_esta() {
        let mut board = board_json(
            r#","repos":[{"path":"/r","name":"r","worktree":"/wt/r"},{"path":"/s","name":"s","worktree":"/wt/s"}]"#,
        );
        board.revive();
        let ws = &board.workspaces[0];
        assert_eq!(ws.repos.len(), 2);
        assert!(ws.multi());
        assert_eq!(ws.primary().name, "r");
        assert_eq!(ws.repos[1].worktree, "/wt/s");
    }

    /// Nenhum PTY sobrevive ao app: aba gravada rodando volta desligada.
    #[test]
    fn aba_gravada_viva_volta_desligada() {
        let mut board = board_json(
            r#","tabs":[{"id":"t1","title":"conversa","status":"rodando","note":null,"pending_prompt":null}]"#,
        );
        board.revive();
        assert!(matches!(
            board.workspaces[0].tabs[0].status,
            Status::Desligada
        ));
    }
}
