//! O Codex do outro lado da conversa.
//!
//! O `codex app-server` fala JSON-RPC pelo stdio: o app pede (`thread/start`,
//! `turn/start`, `turn/interrupt`, `thread/compact/start`), ele avisa
//! (`item/started`, `item/agentMessage/delta`, `item/completed`,
//! `turn/completed`…) e às vezes pergunta (`item/tool/requestUserInput`,
//! `item/commandExecution/requestApproval`). Nada disso chega à tela como é:
//! o `Link` traduz cada coisa diretamente para `ConversationEventV1`, que a
//! mesma timeline reduz para todos os providers.
//! No sentido contrário, uma fala vira
//! `turn/start`, uma resposta a pedido vira a resposta JSON-RPC, uma
//! interrupção vira `turn/interrupt`.
//!
//! O que não tem tradução é decidido aqui:
//!
//! - **O id da sessão é dele.** O `thread/start` devolve o id, o quadro guarda
//!   no `Tab` (`agent_session`), e é ele que volta como `thread/resume`.
//! - **A conversa no disco é a que o app gravou.** O rollout do Codex tem
//!   outra forma; o que a aba reabre amanhã é o que a tela viu hoje, linha por
//!   linha, em `paths::chat_log` (ver `chat::Pump`).
//! - **Os comandos de barra são do app.** O app-server não tem `/compact` nem
//!   `/context`: `/compact` vira `thread/compact/start`, `/context` vira um
//!   relatório montado do `thread/tokenUsage/updated`, e o resto é recusado com
//!   uma linha na tela.
//! - **Solto, como o Claude Code das abas.** `approvalPolicy: never` e sandbox
//!   aberta — o agente não para a cada comando. A pergunta ao usuário
//!   (`request_user_input`) o Codex só oferece ao modelo no modo de plano; a
//!   feature `default_mode_request_user_input` a libera no modo comum, e é
//!   ligada no spawn — sem ela o agente diz que "a ferramenta não está
//!   disponível" e segue sem perguntar.
//!
//! O `Link` não tem thread nem AppHandle: recebe uma linha e devolve linhas.
//! É o que deixa testá-lo com strings — e o que deixa o `chat.rs` não saber
//! que existe um Codex.

use crate::lock::lock;
use crate::session::Launch;
use crate::{agents, chat, conversation, i18n, paths, plugins};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Sobe o `codex app-server` numa aba. `resume` é a thread que o Codex escolheu
/// da outra vez; sem ela a conversa nasce nova.
pub fn spawn(
    app: &AppHandle,
    id: &str,
    workspace: &str,
    worktree: &Path,
    resume: Option<String>,
    launch: &Launch,
) -> Result<chat::Chat, String> {
    let selected_plugins = plugins::codex_for(workspace, launch.plugins.as_ref())?;
    let mut cmd = Command::new("codex");
    cmd.args(["app-server", "--enable", "default_mode_request_user_input"]);
    if let Some(home) = &selected_plugins.home {
        cmd.env("CODEX_HOME", home);
    }
    // Não exigir as features novas de quem abriu uma sessão sem plugin: uma
    // instalação antiga do Codex continua capaz de conversar e usar MCP.
    if !selected_plugins.ids.is_empty() {
        cmd.args(["--enable", "plugins", "--enable", "hooks"]);
    }
    cmd.current_dir(worktree);
    // As ferramentas escolhidas para este workspace. O Codex não tem
    // `--mcp-config`: a tabela inteira vai por `-c`, e os segredos vão pelo
    // ambiente deste processo — ver `mcp::codex_config`. Uma escolha que não
    // possa ser materializada precisa falhar visivelmente: subir sem as
    // ferramentas marcadas faria a sessão parecer correta até o primeiro uso.
    if let Some((servers, env)) = crate::mcp::codex_config(id, launch.mcp.as_ref())? {
        cmd.args(["-c", &format!("mcp_servers={servers}")]);
        for (key, value) in env {
            cmd.env(key, value);
        }
    }
    let log = paths::chat_log(id);
    let start = Start {
        cwd: worktree.display().to_string(),
        resume,
        model: launch.model.trim().to_string(),
        effort: agents::effort(&launch.effort).to_string(),
        plugin_ids: selected_plugins.ids,
        plugin_hook_ids: selected_plugins.hook_ids,
    };
    let io = chat::ProcessIo::new(process_stderr, move |stdin| {
        let link = Arc::new(Mutex::new(Link::new(Box::new(stdin), start)));
        let reader = link.clone();
        let translate = move |line: &str| {
            lock(&reader)
                .on_line(line)
                .iter()
                .map(|event| event.to_string())
                .collect()
        };
        (
            chat::Wire::Codex(link),
            Box::new(translate) as chat::Translate,
        )
    });
    chat::launch(app, id, cmd, &log, Some(log.clone()), "err.codex.spawn", io)
}

/// Com o que a thread abre.
pub struct Start {
    pub cwd: String,
    pub resume: Option<String>,
    /// Vazio é deixar o Codex escolher.
    pub model: String,
    /// Já no nome do Codex (`ultra`, não `ultracode`). Vazio é não passar.
    pub effort: String,
    /// IDs canônicos (`plugin@marketplace`) escolhidos explicitamente neste
    /// workspace. Só hooks destes IDs podem ganhar confiança no handshake.
    pub plugin_ids: Vec<String>,
    /// Subconjunto selecionado que declarou hooks. Todos precisam aparecer
    /// ativos antes de a thread nascer; ausência não pode virar skill-only.
    pub plugin_hook_ids: Vec<String>,
}

/// O que cada pedido nosso em voo era, para saber o que fazer com a resposta.
enum Sent {
    Init,
    /// `resumed` é `thread/resume`: se falhar, a conversa abre nova em vez de a
    /// aba morrer — o rollout pode ter sido apagado, e a aba vale mais.
    Thread {
        resumed: bool,
    },
    Turn,
    Compact,
    Interrupt,
    /// `account/rateLimits/read`, mandado uma vez no início: a notificação de
    /// cota só chega quando ela muda, e a barra de baixo não pode ficar
    /// esperando o primeiro turno para ter um número.
    Usage,
    /// Hooks dos plugins escolhidos, antes de abrir a thread.
    Hooks,
    /// A gravação dos hashes que a seleção explícita acabou de aprovar.
    HookTrust,
}

/// Um pedido do servidor esperando a tela responder.
struct Ask {
    /// O id JSON-RPC dele, que volta na resposta.
    rpc: Value,
    kind: AskKind,
}

enum AskKind {
    Command,
    Patch,
    /// As perguntas: o texto que a tela mostra e o id que o Codex espera.
    Input(Vec<(String, String)>),
}

/// Um bloco de texto (ou pensamento) chegando letra a letra.
struct Open {
    item: String,
    index: usize,
    thinking: bool,
    text: String,
}

pub struct Link {
    out: Box<dyn Write + Send>,
    start: Start,
    next: u64,
    sent: HashMap<u64, Sent>,
    thread: Option<String>,
    /// A thread não abriu, e o motivo. Toda fala daqui em diante é recusada
    /// com ele — é o que a barra mostra.
    failed: Option<String>,
    /// Falas que chegaram antes de a thread existir. Vão na ordem, assim que
    /// ela abrir.
    queue: Vec<Value>,
    model: String,
    window: Option<u64>,
    /// Quanto a conversa pesa agora, pelo último `tokenUsage`.
    ctx: Option<u64>,
    turn: Option<String>,
    asks: HashMap<String, Ask>,
    /// Os arquivos de cada `fileChange` aberto: é o que o pedido de aprovação
    /// dele não repete.
    patches: HashMap<String, Vec<String>>,
    /// O índice do próximo bloco na mensagem deste turno — `assistant.block`
    /// numera os blocos na ordem em que fecham, e o rascunho em
    /// streaming precisa nascer com o mesmo número.
    block: usize,
    message_open: bool,
    open: Option<Open>,
    /// O tamanho da conversa quando a compactação começou.
    compact_pre: Option<u64>,
}

impl Link {
    pub fn new(out: Box<dyn Write + Send>, start: Start) -> Link {
        let mut link = Link {
            out,
            start,
            next: 0,
            sent: HashMap::new(),
            thread: None,
            failed: None,
            queue: vec![],
            model: String::new(),
            window: None,
            ctx: None,
            turn: None,
            asks: HashMap::new(),
            patches: HashMap::new(),
            block: 0,
            message_open: false,
            open: None,
            compact_pre: None,
        };
        let _ = link.call(
            "initialize",
            json!({
                "clientInfo": { "name": "prometeu", "title": "Prometeu", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true },
            }),
            Sent::Init,
        );
        link
    }

    /// Fecha o stdin do processo: é o sinal para ele sair.
    pub fn close(&mut self) {
        self.out = Box::new(std::io::sink());
    }

    /* ---------- da tela para o Codex ---------- */

    /// Um comando V1 da tela. Devolve eventos V1 que ele rendeu sem ir ao
    /// processo.
    pub fn write(&mut self, frame: &Value) -> Result<Vec<Value>, String> {
        if frame["v"] != 1 {
            return Err(i18n::t("err.team.bad"));
        }
        match frame["type"].as_str() {
            Some("message.send") => {
                if let Some(cause) = &self.failed {
                    return Err(i18n::ta("err.codex.thread", &[("cause", cause.clone())]));
                }
                let text = frame["text"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?;
                if self.thread.is_none() {
                    self.queue.push(frame.clone());
                    return Ok(vec![]);
                }
                self.speak(text)
            }
            Some("request.respond") => {
                let id = frame["requestId"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?
                    .to_string();
                let Some(ask) = self.asks.get(&id) else {
                    return Err(i18n::t("err.team.bad"));
                };
                let answer = &frame["response"];
                let outcome = answer["outcome"]
                    .as_str()
                    .ok_or_else(|| i18n::t("err.team.bad"))?;
                let allowed = outcome == "allow";
                let result = match &ask.kind {
                    AskKind::Command | AskKind::Patch => {
                        if !matches!(outcome, "allow" | "deny") {
                            return Err(i18n::t("err.team.bad"));
                        }
                        json!({ "decision": if allowed { "accept" } else { "decline" } })
                    }
                    AskKind::Input(questions) => {
                        if !matches!(outcome, "answer" | "deny") {
                            return Err(i18n::t("err.team.bad"));
                        }
                        let given = &answer["answers"];
                        let mut answers = serde_json::Map::new();
                        for (question, qid) in questions {
                            let text = given[&question].as_str().unwrap_or("").trim().to_string();
                            let list: Vec<String> =
                                if text.is_empty() { vec![] } else { vec![text] };
                            answers.insert(qid.clone(), json!({ "answers": list }));
                        }
                        json!({ "answers": answers })
                    }
                };
                let rpc = ask.rpc.clone();
                self.asks.remove(&id);
                self.reply(&rpc, result)?;
                Ok(vec![])
            }
            // A lista dos comandos que `slash` entende. É uma resposta local:
            // o app-server não oferece esses comandos.
            Some("commands.list") => {
                let commands: Vec<Value> = SLASH
                    .iter()
                    .map(|(name, pt, en)| json!({ "name": name, "description": i18n::pick(pt, en), "hint": "" }))
                    .collect();
                Ok(vec![canonical(
                    "commands.updated",
                    json!({ "commands": commands }),
                )])
            }
            Some("turn.interrupt") => {
                if let (Some(thread), Some(turn)) = (self.thread.clone(), self.turn.clone()) {
                    self.call(
                        "turn/interrupt",
                        json!({ "threadId": thread, "turnId": turn }),
                        Sent::Interrupt,
                    )?;
                }
                Ok(vec![])
            }
            // O Codex já nasce com approvalPolicy `never`; esta capability não
            // é oferecida para ele.
            Some("permission.mode.set") if frame["mode"] == "bypass" => Ok(vec![]),
            _ => Err(i18n::t("err.team.bad")),
        }
    }

    /// Uma fala. As que começam com um nome depois da barra são comandos do
    /// app; caminhos absolutos continuam sendo texto para o agente.
    fn speak(&mut self, text: &str) -> Result<Vec<Value>, String> {
        let text = text.trim();
        if let Some(cmd) = text.strip_prefix('/') {
            let cmd = cmd.split_whitespace().next().unwrap_or("");
            if !cmd.contains('/') {
                return self.slash(cmd);
            }
        }
        let mut params = json!({
            "threadId": self.thread.clone().unwrap_or_default(),
            "input": [{ "type": "text", "text": text, "text_elements": [] }],
            "summary": "auto",
        });
        if !self.start.effort.is_empty() {
            params["effort"] = Value::String(self.start.effort.clone());
        }
        self.call("turn/start", params, Sent::Turn)?;
        Ok(vec![])
    }

    fn slash(&mut self, cmd: &str) -> Result<Vec<Value>, String> {
        match cmd {
            "compact" => {
                let thread = self.thread.clone().unwrap_or_default();
                self.call(
                    "thread/compact/start",
                    json!({ "threadId": thread }),
                    Sent::Compact,
                )?;
                Ok(vec![canonical(
                    "context.compaction",
                    json!({ "state": "started", "detail": "" }),
                )])
            }
            "context" => Ok(vec![
                canonical(
                    "context.reported",
                    json!({ "markdown": self.context_report() }),
                ),
                turn_completed("ok", "", None),
            ]),
            other => Ok(vec![
                notice(
                    "error",
                    "command.unsupported",
                    &i18n::pick(
                        &format!("o Codex não tem o comando /{other}"),
                        &format!("Codex has no /{other} command"),
                    ),
                ),
                turn_completed("ok", "", None),
            ]),
        }
    }

    /// O `/context` do Codex: o que o `tokenUsage` conta, no mesmo markdown que
    /// o Claude Code devolve — é o que a tela sabe desenhar como painel.
    fn context_report(&self) -> String {
        let used = self.ctx.unwrap_or(0);
        let total = self.window.unwrap_or(0);
        let pct = if total > 0 {
            (used as f64 / total as f64 * 100.0).round() as u64
        } else {
            0
        };
        let free = total.saturating_sub(used);
        let conversation = i18n::pick("Conversa", "Conversation");
        format!(
            "## Context Usage\n**Model:** {}\n**Tokens:** {} / {} ({}%)\n\n### Estimated usage by category\n| Category | Tokens | Percentage |\n|---|---|---|\n| {} | {} | {}% |\n| Free space | {} | {}% |\n",
            if self.model.is_empty() { self.start.model.clone() } else { self.model.clone() },
            kilo(used),
            kilo(total),
            pct,
            conversation,
            kilo(used),
            pct,
            kilo(free),
            100 - pct.min(100),
        )
    }

    fn call(&mut self, method: &str, params: Value, sent: Sent) -> Result<(), String> {
        self.next += 1;
        let id = self.next;
        self.sent.insert(id, sent);
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn reply(&mut self, rpc: &Value, result: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "id": rpc, "result": result }))
    }

    fn refuse(&mut self, rpc: &Value, message: &str) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "id": rpc, "error": { "code": -32601, "message": message } }))
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        self.out.write_all(line.as_bytes()).map_err(i18n::io)?;
        self.out.flush().map_err(i18n::io)
    }

    /* ---------- do Codex para a tela ---------- */

    /// Uma linha do processo. Devolve as linhas da tela que ela vale.
    pub fn on_line(&mut self, line: &str) -> Vec<Value> {
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        match (msg.get("method").and_then(Value::as_str), msg.get("id")) {
            (Some(method), Some(id)) => self.request(id.clone(), method, &msg["params"]),
            (Some(method), None) => self.notification(method, &msg["params"]),
            (None, Some(id)) => self.response(id.as_u64().unwrap_or(0), &msg),
            (None, None) => vec![],
        }
    }

    fn response(&mut self, id: u64, msg: &Value) -> Vec<Value> {
        let error = msg["error"]["message"].as_str().map(str::to_string);
        match self.sent.remove(&id) {
            Some(Sent::Init) => {
                let _ = self.notify("initialized", json!({}));
                let _ = self.call("account/rateLimits/read", json!({}), Sent::Usage);
                if self.start.plugin_hook_ids.is_empty() {
                    self.open_thread();
                } else {
                    let _ = self.call(
                        "hooks/list",
                        json!({ "cwds": [self.start.cwd.clone()] }),
                        Sent::Hooks,
                    );
                }
                vec![]
            }
            // Codex sem conta logada responde erro aqui, e a barra fica sem a
            // faixa dele — que é exatamente o que se quer dizer.
            Some(Sent::Usage) => match error {
                Some(_) => vec![],
                // O resultado completo contém `rateLimitsByLimitId` nas
                // versões novas. Entregar só o bucket legado apagaria as
                // cotas separadas por modelo antes de chegarem ao `usage`.
                None => vec![rate_limits(&msg["result"])],
            },
            Some(Sent::Hooks) => {
                if let Some(cause) = error {
                    return self.fail_plugin_hooks(cause);
                }
                let selected = self
                    .start
                    .plugin_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>();
                let expected = self
                    .start
                    .plugin_hook_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::HashSet<_>>();
                let mut found = std::collections::HashSet::new();
                let mut invalid = std::collections::HashSet::new();
                let mut managed_disabled = std::collections::HashSet::new();
                let mut trusts = serde_json::Map::new();
                for hook in msg["result"]["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
                {
                    let Some(plugin) = hook["pluginId"].as_str() else {
                        continue;
                    };
                    if !selected.contains(plugin) {
                        continue;
                    }
                    found.insert(plugin);
                    let enabled = hook["enabled"].as_bool() == Some(true);
                    let status = hook["trustStatus"].as_str();
                    if status == Some("managed") {
                        if !enabled {
                            managed_disabled.insert(plugin);
                        }
                        continue;
                    }
                    if status == Some("trusted") && enabled {
                        continue;
                    }
                    let (Some(key), Some(hash)) =
                        (hook["key"].as_str(), hook["currentHash"].as_str())
                    else {
                        invalid.insert(plugin);
                        continue;
                    };
                    trusts.insert(
                        key.to_string(),
                        json!({ "trusted_hash": hash, "enabled": true }),
                    );
                }
                let mut missing = expected.difference(&found).copied().collect::<Vec<_>>();
                missing.sort_unstable();
                if !missing.is_empty() {
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksMissing",
                        &[("plugins", missing.join(", "))],
                    ));
                }
                if !invalid.is_empty() {
                    let mut plugins = invalid.into_iter().collect::<Vec<_>>();
                    plugins.sort_unstable();
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksInvalid",
                        &[("plugins", plugins.join(", "))],
                    ));
                }
                if !managed_disabled.is_empty() {
                    let mut plugins = managed_disabled.into_iter().collect::<Vec<_>>();
                    plugins.sort_unstable();
                    return self.fail_plugin_hooks(i18n::ta(
                        "err.plugin.codex.hooksManaged",
                        &[("plugins", plugins.join(", "))],
                    ));
                }
                if trusts.is_empty() {
                    self.open_thread();
                } else {
                    let _ = self.call(
                        "config/batchWrite",
                        json!({
                            "edits": [{
                                "keyPath": "hooks.state",
                                "value": trusts,
                                "mergeStrategy": "upsert",
                            }],
                            "reloadUserConfig": true,
                        }),
                        Sent::HookTrust,
                    );
                }
                vec![]
            }
            Some(Sent::HookTrust) => {
                if let Some(cause) = error {
                    self.fail_plugin_hooks(cause)
                } else {
                    self.open_thread();
                    vec![]
                }
            }
            Some(Sent::Thread { resumed }) => {
                if let Some(cause) = error {
                    // Retomar falhou: a conversa de antes ficou para trás, mas a
                    // aba continua servindo — abre nova e avisa.
                    if resumed {
                        self.start.resume = None;
                        self.open_thread();
                        return vec![notice("error", "provider.resume", &i18n::pick(
                            &format!("não deu para retomar a conversa no Codex ({cause}); esta é nova"),
                            &format!("could not resume the Codex conversation ({cause}); this one is new"),
                        ))];
                    }
                    self.failed = Some(cause.clone());
                    self.queue.clear();
                    return vec![notice("error", "provider.thread", &cause)];
                }
                let thread = msg["result"]["thread"]["id"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                self.model = msg["result"]["model"].as_str().unwrap_or("").to_string();
                self.thread = Some(thread.clone());
                let mut out = vec![canonical(
                    "session.identity",
                    json!({ "providerSession": thread }),
                )];
                for frame in std::mem::take(&mut self.queue) {
                    if let Ok(more) = self.write(&frame) {
                        out.extend(more);
                    }
                }
                out
            }
            Some(Sent::Turn) => match error {
                Some(cause) => vec![
                    notice("error", "provider.turn", &cause),
                    turn_completed("error", &cause, None),
                ],
                None => {
                    if let Some(turn) = msg["result"]["turn"]["id"].as_str() {
                        self.turn = Some(turn.to_string());
                    }
                    vec![]
                }
            },
            Some(Sent::Compact) => match error {
                Some(cause) => vec![
                    canonical(
                        "context.compaction",
                        json!({ "state": "failed", "detail": cause }),
                    ),
                    turn_completed("error", &cause, None),
                ],
                None => vec![],
            },
            Some(Sent::Interrupt) | None => vec![],
        }
    }

    fn open_thread(&mut self) {
        let mut params = json!({
            "cwd": self.start.cwd,
            "approvalPolicy": "never",
            "sandbox": "danger-full-access",
        });
        if !self.start.model.is_empty() {
            params["model"] = Value::String(self.start.model.clone());
        }
        match self.start.resume.clone() {
            Some(thread) => {
                params["threadId"] = Value::String(thread);
                let _ = self.call("thread/resume", params, Sent::Thread { resumed: true });
            }
            None => {
                let _ = self.call("thread/start", params, Sent::Thread { resumed: false });
            }
        }
    }

    /// Um plugin marcado é uma expectativa de comportamento, não só de
    /// descoberta. Se seus hooks não puderem nascer ativos, a conversa não
    /// abre silenciosamente em outro modo.
    fn fail_plugin_hooks(&mut self, cause: String) -> Vec<Value> {
        let message = i18n::ta("err.plugin.codex.hooks", &[("cause", cause)]);
        self.failed = Some(message.clone());
        self.queue.clear();
        vec![notice("error", "plugin.hooks", &message)]
    }

    /// Um pedido do servidor vira um card que espera resposta, conservando o
    /// id que voltará ao app-server.
    fn request(&mut self, rpc: Value, method: &str, params: &Value) -> Vec<Value> {
        let id = match &rpc {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let item = params["itemId"].as_str().unwrap_or("").to_string();
        let (kind, tool, input) = match method {
            "item/commandExecution/requestApproval" => {
                let command = params["command"].as_str().unwrap_or("").to_string();
                (
                    AskKind::Command,
                    "Bash",
                    json!({ "command": pretty(&command, &params["commandActions"]) }),
                )
            }
            "item/fileChange/requestApproval" => {
                let paths = self.patches.get(&item).cloned().unwrap_or_default();
                (
                    AskKind::Patch,
                    "Edit",
                    json!({ "file_path": paths.join(", ") }),
                )
            }
            "item/tool/requestUserInput" => {
                let mut ids = vec![];
                let questions: Vec<Value> = params["questions"]
                    .as_array()
                    .map(|qs| {
                        qs.iter()
                            .map(|q| {
                                let question = q["question"].as_str().unwrap_or("").to_string();
                                ids.push((question.clone(), q["id"].as_str().unwrap_or("").to_string()));
                                let options: Vec<Value> = q["options"]
                                    .as_array()
                                    .map(|os| {
                                        os.iter()
                                            .map(|o| json!({ "label": o["label"], "description": o["description"] }))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                json!({ "question": question, "header": q["header"], "multiSelect": false, "options": options })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                (
                    AskKind::Input(ids),
                    "AskUserQuestion",
                    json!({ "questions": questions }),
                )
            }
            _ => {
                let _ = self.refuse(&rpc, "unsupported by prometeu");
                return vec![];
            }
        };
        let request_kind = if matches!(&kind, AskKind::Input(_)) {
            "question"
        } else {
            "approval"
        };
        self.asks.insert(id.clone(), Ask { rpc, kind });
        vec![canonical(
            "request.opened",
            json!({
                "requestId": id,
                "kind": request_kind,
                "toolId": if item.is_empty() { Value::Null } else { json!(item) },
                "tool": tool,
                "input": input,
            }),
        )]
    }

    fn notification(&mut self, method: &str, p: &Value) -> Vec<Value> {
        match method {
            "turn/started" => {
                self.turn = p["turn"]["id"].as_str().map(str::to_string);
                self.block = 0;
                self.message_open = false;
                self.open = None;
                vec![]
            }
            "item/started" => self.started(&p["item"]),
            "item/completed" => self.completed(&p["item"]),
            "item/agentMessage/delta" | "item/plan/delta" => {
                self.delta(p["itemId"].as_str(), p["delta"].as_str())
            }
            "item/reasoning/summaryTextDelta" => {
                self.delta(p["itemId"].as_str(), p["delta"].as_str())
            }
            "item/reasoning/summaryPartAdded" => match p["summaryIndex"].as_u64() {
                Some(n) if n > 0 => self.delta(p["itemId"].as_str(), Some("\n\n")),
                _ => vec![],
            },
            // Quanto da cota já foi. Não é da conversa: sai daqui pelo mesmo
            // cano das outras linhas só porque é o cano que chega ao app
            // (ver `chat::react`), e o `usage` é quem guarda.
            "account/rateLimits/updated" => vec![rate_limits(&p["rateLimits"])],
            "thread/tokenUsage/updated" => {
                let usage = &p["tokenUsage"];
                self.window = usage["modelContextWindow"].as_u64().or(self.window);
                match usage["last"]["totalTokens"].as_u64() {
                    Some(n) if n > 0 => {
                        self.ctx = Some(n);
                        vec![canonical(
                            "context.updated",
                            json!({ "used": n, "window": self.window }),
                        )]
                    }
                    _ => vec![],
                }
            }
            "turn/completed" => {
                let mut out = self.seal(None);
                self.turn = None;
                self.block = 0;
                self.message_open = false;
                let turn = &p["turn"];
                let ms = turn["durationMs"].as_u64();
                out.push(match turn["status"].as_str() {
                    Some("failed") => {
                        let cause = turn["error"]["message"].as_str().unwrap_or("").to_string();
                        turn_completed("error", &cause, ms)
                    }
                    Some("interrupted") => turn_completed("interrupted", "", ms),
                    _ => turn_completed("ok", "", ms),
                });
                out
            }
            "error" => {
                let cause = p["error"]["message"].as_str().unwrap_or("").to_string();
                let retry = p["willRetry"].as_bool() == Some(true);
                let text = match retry {
                    true => i18n::pick(
                        &format!("{cause} (tentando de novo)"),
                        &format!("{cause} (retrying)"),
                    ),
                    false => cause,
                };
                vec![notice("error", "provider.error", &text)]
            }
            "warning" => p["message"]
                .as_str()
                .map(|message| notice("warning", "provider.warning", message))
                .into_iter()
                .collect(),
            _ => vec![],
        }
    }

    fn started(&mut self, item: &Value) -> Vec<Value> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        match item["type"].as_str() {
            Some("agentMessage" | "plan") => self.open_text(&id, false),
            Some("reasoning") => self.open_text(&id, true),
            Some("commandExecution") => {
                let command = pretty(
                    item["command"].as_str().unwrap_or(""),
                    &item["commandActions"],
                );
                self.tool_use(&id, "Bash", json!({ "command": command }))
            }
            Some("fileChange") => {
                let paths: Vec<String> = item["changes"]
                    .as_array()
                    .map(|cs| {
                        cs.iter()
                            .filter_map(|c| c["path"].as_str())
                            .map(|p| self.relative(p))
                            .collect()
                    })
                    .unwrap_or_default();
                self.patches.insert(id.clone(), paths.clone());
                self.tool_use(&id, "Edit", json!({ "file_path": paths.join(", ") }))
            }
            Some("mcpToolCall") => {
                let name = format!(
                    "mcp__{}__{}",
                    item["server"].as_str().unwrap_or(""),
                    item["tool"].as_str().unwrap_or("")
                );
                self.tool_use(&id, &name, item["arguments"].clone())
            }
            Some("dynamicToolCall") => {
                let name = item["tool"].as_str().unwrap_or("tool").to_string();
                self.tool_use(&id, &name, item["arguments"].clone())
            }
            Some("webSearch") => self.tool_use(&id, "WebSearch", json!({ "query": item["query"] })),
            Some("collabAgentToolCall") => self.tool_use(
                &id,
                "Agent",
                json!({ "description": item["tool"], "prompt": item["prompt"] }),
            ),
            Some("imageView") => self.tool_use(&id, "Read", json!({ "file_path": item["path"] })),
            Some("contextCompaction") => {
                self.compact_pre = self.ctx;
                vec![canonical(
                    "context.compaction",
                    json!({ "state": "started", "detail": "" }),
                )]
            }
            _ => vec![],
        }
    }

    fn completed(&mut self, item: &Value) -> Vec<Value> {
        let id = item["id"].as_str().unwrap_or("").to_string();
        match item["type"].as_str() {
            Some("agentMessage" | "plan") => {
                let text = item["text"].as_str().unwrap_or("").to_string();
                self.close_text(&id, text, false)
            }
            Some("reasoning") => {
                let parts = |key: &str| -> Vec<String> {
                    item[key]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let mut summary = parts("summary");
                if summary.is_empty() {
                    summary = parts("content");
                }
                self.close_text(&id, summary.join("\n\n"), true)
            }
            Some("commandExecution") => {
                let ok = item["status"].as_str() == Some("completed");
                let mut text = item["aggregatedOutput"].as_str().unwrap_or("").to_string();
                if let (false, Some(code)) = (ok, item["exitCode"].as_i64()) {
                    text = format!("{text}\n(exit {code})").trim().to_string();
                }
                vec![tool_completed(&id, &text, !ok)]
            }
            Some("fileChange") => {
                self.patches.remove(&id);
                let ok = item["status"].as_str() == Some("completed");
                let diff: Vec<String> = item["changes"]
                    .as_array()
                    .map(|cs| cs.iter().map(|c| patch(c, &self.start.cwd)).collect())
                    .unwrap_or_default();
                vec![tool_completed(&id, &diff.join("\n"), !ok)]
            }
            Some("mcpToolCall") => {
                let failed = item["status"].as_str() == Some("failed") || !item["error"].is_null();
                let text = match item["error"]["message"].as_str() {
                    Some(m) => m.to_string(),
                    None => texts(&item["result"]["content"]),
                };
                vec![tool_completed(&id, &text, failed)]
            }
            Some("dynamicToolCall") => {
                let failed = item["success"].as_bool() == Some(false);
                vec![tool_completed(&id, &texts(&item["contentItems"]), failed)]
            }
            Some("webSearch" | "collabAgentToolCall" | "imageView") => {
                vec![tool_completed(&id, "", false)]
            }
            Some("contextCompaction") => {
                let post = self.ctx;
                vec![
                    canonical(
                        "context.compaction",
                        json!({ "state": "stopped", "detail": "" }),
                    ),
                    canonical(
                        "context.compacted",
                        json!({ "before": self.compact_pre, "after": post }),
                    ),
                ]
            }
            _ => vec![],
        }
    }

    /// Abre um bloco de texto em streaming. Um bloco que ainda estava aberto
    /// fecha antes com o que tinha — a tela numera os blocos na ordem, e dois
    /// abertos ao mesmo tempo é o que o Codex não faz, mas o número não pode
    /// depender disso.
    fn open_text(&mut self, id: &str, thinking: bool) -> Vec<Value> {
        let mut out = self.seal(None);
        if !self.message_open {
            self.message_open = true;
            out.push(canonical(
                "assistant.started",
                json!({ "messageId": self.msg() }),
            ));
        }
        let index = self.block;
        self.block += 1;
        out.push(canonical(
            "assistant.block.started",
            json!({
                "messageId": self.msg(),
                "index": index,
                "block": block_of("", thinking),
            }),
        ));
        self.open = Some(Open {
            item: id.to_string(),
            index,
            thinking,
            text: String::new(),
        });
        out
    }

    fn delta(&mut self, id: Option<&str>, text: Option<&str>) -> Vec<Value> {
        let (Some(id), Some(text)) = (id, text) else {
            return vec![];
        };
        let Some(open) = self.open.as_mut().filter(|o| o.item == id) else {
            return vec![];
        };
        open.text.push_str(text);
        let (index, thinking) = (open.index, open.thinking);
        vec![canonical(
            "assistant.delta",
            json!({
                "messageId": self.msg(),
                "index": index,
                "kind": if thinking { "thinking" } else { "text" },
                "delta": text,
            }),
        )]
    }

    /// Fecha o bloco deste item com o texto final, ou entrega o bloco inteiro
    /// de uma vez quando nunca houve rascunho (a linha chegou sem `started`).
    fn close_text(&mut self, id: &str, text: String, thinking: bool) -> Vec<Value> {
        if self.open.as_ref().is_some_and(|o| o.item == id) {
            return self.seal(Some(text));
        }
        let index = self.block;
        self.block += 1;
        vec![self.assistant(index, block_of(&text, thinking))]
    }

    /// Fecha o bloco aberto com o evento autoritativo que a tela guarda.
    /// `text` é o texto final; sem ele vai o que chegou.
    fn seal(&mut self, text: Option<String>) -> Vec<Value> {
        let Some(open) = self.open.take() else {
            return vec![];
        };
        let text = text.unwrap_or(open.text);
        vec![self.assistant(open.index, block_of(&text, open.thinking))]
    }

    /// Uma ferramenta começando: o card já nasce inteiro, porque o Codex conta
    /// o comando de uma vez.
    fn tool_use(&mut self, id: &str, name: &str, input: Value) -> Vec<Value> {
        let mut out = self.seal(None);
        self.message_open = true;
        let index = self.block;
        self.block += 1;
        out.push(self.assistant(
            index,
            json!({ "kind": "tool", "id": id, "name": name, "input": input }),
        ));
        out
    }

    fn assistant(&self, index: usize, block: Value) -> Value {
        canonical(
            "assistant.block",
            json!({ "messageId": self.msg(), "index": index, "block": block }),
        )
    }

    /// Todos os blocos de um turno são uma mensagem só na tela.
    fn msg(&self) -> String {
        self.turn.clone().unwrap_or_default()
    }

    /// O caminho como a pessoa o lê: dentro do worktree, sem o worktree.
    fn relative(&self, path: &str) -> String {
        relative(path, &self.start.cwd)
    }
}

fn relative(path: &str, cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    path.strip_prefix(cwd)
        .and_then(|rest| rest.strip_prefix('/'))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(path)
        .to_string()
}

/* ---------- linhas prontas ---------- */

/// A cota do Codex embrulhada como linha do app: quem a lê é `chat::react`,
/// que a entrega ao `usage`. Não vai para o transcript (ver `chat::keep`) —
/// não é conversa.
fn rate_limits(limits: &Value) -> Value {
    canonical(
        "usage.updated",
        json!({ "provider": "codex", "usage": limits }),
    )
}

/// Os comandos de barra que o tradutor entende (ver `slash`), com a descrição
/// nas duas línguas. É a resposta a `commands.list` — a lista que a caixa
/// oferece ao escrever "/".
const SLASH: [(&str, &str, &str); 2] = [
    (
        "compact",
        "Resume a conversa até aqui para liberar contexto",
        "Free up context by summarizing the conversation so far",
    ),
    (
        "context",
        "Quanto da janela de contexto está em uso",
        "How much of the context window is in use",
    ),
];

fn canonical(kind: &str, fields: Value) -> Value {
    conversation::event(kind, conversation::now(), fields)
}

fn notice(level: &str, code: &str, detail: &str) -> Value {
    canonical(
        "system.notice",
        json!({ "level": level, "code": code, "detail": detail }),
    )
}

fn turn_completed(outcome: &str, message: &str, duration_ms: Option<u64>) -> Value {
    canonical(
        "turn.completed",
        json!({
            "outcome": outcome,
            "message": message,
            "durationMs": duration_ms,
            "costUsd": Value::Null,
        }),
    )
}

fn tool_completed(id: &str, output: &str, error: bool) -> Value {
    canonical(
        "tool.completed",
        json!({ "toolId": id, "output": output, "error": error, "background": false }),
    )
}

fn block_of(text: &str, thinking: bool) -> Value {
    match thinking {
        true => json!({ "kind": "thinking", "text": text }),
        false => json!({ "kind": "text", "text": text }),
    }
}

/// O app-server escreve no stderr os mesmos erros de ferramenta que já manda
/// pelo JSON-RPC. São linhas de tracing com timestamp e nível, frequentemente
/// coloridas; repassá-las duplica o card com uma faixa vermelha ilegível. O que
/// não tem essa forma continua aparecendo, porque pode explicar um processo que
/// morreu antes de conseguir responder pelo protocolo.
fn process_stderr(line: &str) -> Option<String> {
    let plain = strip_ansi(line);
    let mut fields = plain.split_whitespace();
    let timestamp = fields.next().unwrap_or("");
    let level = fields.next().unwrap_or("");
    let tracing = timestamp.contains('T')
        && timestamp.ends_with('Z')
        && matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR");
    (!tracing).then_some(plain)
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// O comando como a pessoa o leria. O Codex embrulha tudo em
/// `/bin/zsh -lc "…"`; o `commandActions` traz o de dentro.
fn pretty(command: &str, actions: &Value) -> String {
    actions
        .as_array()
        .and_then(|a| a.first())
        .and_then(|a| a["command"].as_str())
        .filter(|c| !c.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| command.to_string())
}

/// Um arquivo mudado, como diff que a tela colore. O Codex manda só o hunk de
/// uma alteração, e o conteúdo cru de um arquivo novo (ou apagado): o
/// cabeçalho é o que diz de qual arquivo é, e o sinal é o que diz o que
/// aconteceu com cada linha.
fn patch(change: &Value, cwd: &str) -> String {
    let path = relative(change["path"].as_str().unwrap_or(""), cwd);
    let diff = change["diff"].as_str().unwrap_or("").trim_end_matches('\n');
    if diff.starts_with("diff --git") || diff.starts_with("--- ") {
        return diff.to_string();
    }
    let kind = change["kind"]["type"].as_str().unwrap_or("update");
    let signed = |sign: char| -> String {
        let n = diff.lines().count();
        let body: Vec<String> = diff.lines().map(|l| format!("{sign}{l}")).collect();
        let range = match sign {
            '+' => format!("@@ -0,0 +1,{n} @@"),
            _ => format!("@@ -1,{n} +0,0 @@"),
        };
        format!("{range}\n{}", body.join("\n"))
    };
    let (from, to, hunk) = match kind {
        "add" if !diff.starts_with("@@") => {
            ("/dev/null".to_string(), format!("b/{path}"), signed('+'))
        }
        "add" => (
            "/dev/null".to_string(),
            format!("b/{path}"),
            diff.to_string(),
        ),
        "delete" if !diff.starts_with("@@") => {
            (format!("a/{path}"), "/dev/null".to_string(), signed('-'))
        }
        "delete" => (
            format!("a/{path}"),
            "/dev/null".to_string(),
            diff.to_string(),
        ),
        _ => (format!("a/{path}"), format!("b/{path}"), diff.to_string()),
    };
    format!("diff --git a/{path} b/{path}\n--- {from}\n+++ {to}\n{hunk}")
}

/// Os textos de uma lista de blocos de conteúdo (MCP e ferramentas dinâmicas).
fn texts(content: &Value) -> String {
    content
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// `24k`, `3.1k`, `1.2m` — o mesmo desenho do `kilo` da tela.
fn kilo(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1000..=9999 => format!("{:.1}k", n as f64 / 1000.0),
        10000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{:.1}m", n as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Um stdin de mentira: o que o `Link` escreveu, para conferir.
    #[derive(Clone, Default)]
    struct Out(Arc<Mutex<Vec<Value>>>);

    impl Write for Out {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                self.0
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(line).unwrap());
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Out {
        fn take(&self) -> Vec<Value> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    fn link(resume: Option<&str>) -> (Link, Out) {
        let out = Out::default();
        let start = Start {
            cwd: "/wt".into(),
            resume: resume.map(str::to_string),
            model: "gpt-5.4".into(),
            effort: "high".into(),
            plugin_ids: vec![],
            plugin_hook_ids: vec![],
        };
        (Link::new(Box::new(out.clone()), start), out)
    }

    /// Abre a thread: responde o `initialize` e o `thread/start`. Nada vai ao
    /// processo antes disso além do próprio `initialize`.
    fn opened(link: &mut Link, out: &Out) -> Vec<Value> {
        let before = out.take();
        assert!(
            before.iter().all(|m| m["method"] == "initialize"),
            "{before:?}"
        );
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        assert_eq!(sent[0]["method"], "initialized");
        let start = call_id(&sent, "thread/start");
        let thread = &sent[start.1];
        assert_eq!(thread["params"]["approvalPolicy"], "never");
        assert_eq!(thread["params"]["model"], "gpt-5.4");
        link.on_line(&format!(
            r#"{{"id":{},"result":{{"thread":{{"id":"t-1"}},"model":"gpt-5.4"}}}}"#,
            start.0
        ))
    }

    /// O id JSON-RPC de um pedido, e onde ele saiu. Procurar pelo método em vez
    /// de contar linhas é o que deixa somar pedido novo no início da conversa
    /// (a cota, por exemplo) sem reescrever teste nenhum.
    fn call_id(sent: &[Value], method: &str) -> (u64, usize) {
        let at = sent
            .iter()
            .position(|m| m["method"] == method)
            .unwrap_or_else(|| panic!("nenhum {method} em {sent:?}"));
        (sent[at]["id"].as_u64().unwrap(), at)
    }

    fn user(text: &str) -> Value {
        json!({ "v": 1, "type": "message.send", "text": text })
    }

    #[test]
    fn a_thread_abre_e_a_fala_que_esperava_vai() {
        let (mut link, out) = link(None);
        assert_eq!(out.take()[0]["method"], "initialize");
        assert!(link.write(&user("oi")).unwrap().is_empty());
        let frames = opened(&mut link, &out);
        assert_eq!(frames[0]["v"], 1);
        assert_eq!(frames[0]["type"], "session.identity");
        assert_eq!(frames[0]["providerSession"], "t-1");
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/start");
        assert_eq!(sent[0]["params"]["threadId"], "t-1");
        assert_eq!(sent[0]["params"]["input"][0]["text"], "oi");
        assert_eq!(sent[0]["params"]["effort"], "high");
    }

    /// Marcar o plugin é a autorização que o Claude já recebe pela flag. No
    /// Codex ela também aprova o hash atual dos hooks daquele plugin — nunca
    /// hooks de usuário, projeto ou de outro pacote que apareceram na lista.
    #[test]
    fn plugins_escolhidos_aprovam_so_os_proprios_hooks_antes_da_thread() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["ponytail@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["ponytail@prometeu-dev".into()];
        out.take();

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        assert!(sent
            .iter()
            .all(|message| message["method"] != "thread/start"));
        assert_eq!(sent[hooks.1]["params"]["cwds"][0], "/wt");

        link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:0","currentHash":"sha256:novo","trustStatus":"trusted","enabled":false}},
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:1","currentHash":"sha256:novo-1","trustStatus":"untrusted","enabled":true}},
              {{"pluginId":"ponytail@prometeu-dev","key":"plugin:ponytail:2","currentHash":"sha256:pronto","trustStatus":"trusted","enabled":true}},
              {{"pluginId":"outro@prometeu-dev","key":"plugin:outro:0","currentHash":"sha256:outro","trustStatus":"untrusted","enabled":false}},
              {{"pluginId":null,"key":"/tmp/hooks.json:0","currentHash":"sha256:user","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));
        let sent = out.take();
        let trust = call_id(&sent, "config/batchWrite");
        let value = &sent[trust.1]["params"]["edits"][0]["value"];
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert_eq!(value["plugin:ponytail:0"]["trusted_hash"], "sha256:novo");
        assert_eq!(value["plugin:ponytail:0"]["enabled"], true);
        assert_eq!(value["plugin:ponytail:1"]["trusted_hash"], "sha256:novo-1");
        assert_eq!(value["plugin:ponytail:1"]["enabled"], true);
        assert!(value.get("plugin:ponytail:2").is_none());
        assert_eq!(sent[trust.1]["params"]["reloadUserConfig"], true);

        link.on_line(&format!(r#"{{"id":{},"result":{{}}}}"#, trust.0));
        assert_eq!(out.take()[0]["method"], "thread/start");
    }

    #[test]
    fn plugin_com_hook_declarado_nao_abre_sem_ser_descoberto() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["caveman@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["caveman@prometeu-dev".into()];
        out.take();
        assert!(link.write(&user("fala como caveman")).unwrap().is_empty());

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        let frames = link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"outro@prometeu-dev","key":"plugin:outro:0","currentHash":"sha256:outro","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));

        assert_eq!(frames[0]["type"], "system.notice");
        assert_eq!(frames[0]["level"], "error");
        assert_eq!(frames[0]["code"], "plugin.hooks");
        assert!(frames[0]["detail"]
            .as_str()
            .unwrap()
            .contains("caveman@prometeu-dev"));
        assert!(out
            .take()
            .iter()
            .all(|message| message["method"] != "thread/start"));
        assert!(link.write(&user("oi")).is_err());
    }

    #[test]
    fn falha_ao_ativar_hook_impede_a_thread() {
        let (mut link, out) = link(None);
        link.start.plugin_ids = vec!["caveman@prometeu-dev".into()];
        link.start.plugin_hook_ids = vec!["caveman@prometeu-dev".into()];
        out.take();

        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let hooks = call_id(&sent, "hooks/list");
        link.on_line(&format!(
            r#"{{"id":{},"result":{{"data":[{{"cwd":"/wt","hooks":[
              {{"pluginId":"caveman@prometeu-dev","key":"plugin:caveman:0","currentHash":"sha256:caveman","trustStatus":"untrusted","enabled":false}}
            ]}}]}}}}"#,
            hooks.0
        ));
        let sent = out.take();
        let trust = call_id(&sent, "config/batchWrite");
        let frames = link.on_line(&format!(
            r#"{{"id":{},"error":{{"code":-32603,"message":"config read-only"}}}}"#,
            trust.0
        ));

        assert_eq!(frames[0]["level"], "error");
        assert!(frames[0]["detail"]
            .as_str()
            .unwrap()
            .contains("config read-only"));
        assert!(out
            .take()
            .iter()
            .all(|message| message["method"] != "thread/start"));
    }

    #[test]
    fn initialize_responde_os_comandos_sem_ir_ao_processo() {
        let (mut link, out) = link(None);
        out.take();
        let req = json!({ "v": 1, "type": "commands.list" });
        let frames = link.write(&req).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["v"], 1);
        assert_eq!(frames[0]["type"], "commands.updated");
        let names: Vec<&str> = frames[0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["compact", "context"]);
        assert!(!frames[0]["commands"][0]["description"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(frames[0]["commands"][0]["hint"], "");
        assert!(out.take().is_empty());
    }

    #[test]
    fn leitura_de_cota_entrega_o_snapshot_multibucket_inteiro() {
        let (mut link, out) = link(None);
        out.take();
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let (id, _) = call_id(&sent, "account/rateLimits/read");
        let frames = link.on_line(&format!(
            r#"{{"id":{id},"result":{{"rateLimits":{{"limitId":"codex"}},"rateLimitsByLimitId":{{"codex":{{"limitId":"codex"}},"spark":{{"limitId":"spark","limitName":"Spark"}}}}}}}}"#
        ));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["type"], "usage.updated");
        assert!(frames[0]["usage"]["rateLimitsByLimitId"]["spark"].is_object());
    }

    #[test]
    fn retomar_passa_o_id_e_cai_para_nova_se_falhar() {
        let (mut link, out) = link(Some("velha"));
        out.take();
        link.on_line(r#"{"id":1,"result":{}}"#);
        let sent = out.take();
        let (id, at) = call_id(&sent, "thread/resume");
        assert_eq!(sent[at]["params"]["threadId"], "velha");
        let frames = link.on_line(&format!(
            r#"{{"id":{id},"error":{{"code":1,"message":"no such thread"}}}}"#
        ));
        assert_eq!(frames[0]["type"], "system.notice");
        assert_eq!(frames[0]["code"], "provider.resume");
        assert_eq!(out.take()[0]["method"], "thread/start");
    }

    #[test]
    fn um_turno_vira_rascunho_bloco_autoritativo_e_fim() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1","text":""}}}"#);
        assert_eq!(f[0]["type"], "assistant.started");
        assert_eq!(f[0]["messageId"], "turn-1");
        assert_eq!(f[1]["type"], "assistant.block.started");
        assert_eq!(f[1]["index"], 0);
        let f = link.on_line(
            r#"{"method":"item/agentMessage/delta","params":{"itemId":"m1","delta":"Ol"}}"#,
        );
        assert_eq!(f[0]["type"], "assistant.delta");
        assert_eq!(f[0]["kind"], "text");
        assert_eq!(f[0]["delta"], "Ol");
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"m1","text":"Olá"}}}"#);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0]["type"], "assistant.block");
        assert_eq!(f[0]["messageId"], "turn-1");
        assert_eq!(f[0]["block"]["kind"], "text");
        assert_eq!(f[0]["block"]["text"], "Olá");
        assert!(f[0]["at"].is_number());
        let f = link.on_line(r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed","durationMs":900}}}"#);
        assert_eq!(f[0]["type"], "turn.completed");
        assert_eq!(f[0]["outcome"], "ok");
        assert_eq!(f[0]["durationMs"], 900);
    }

    #[test]
    fn a_borda_do_codex_entrega_eventos_v1_sem_segunda_traducao() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(
            r#"{"method":"turn/started","params":{"threadId":"t-1","turn":{"id":"turn-1"}}}"#,
        );
        let started = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","commandActions":[]}}}"#);
        assert_eq!(started[0]["v"], 1);
        assert_eq!(started[0]["type"], "assistant.block");
        assert_eq!(started[0]["block"]["kind"], "tool");

        let completed = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","status":"completed","aggregatedOutput":"ok","exitCode":0}}}"#);
        assert_eq!(completed[0]["type"], "tool.completed");
        assert_eq!(completed[0]["toolId"], "c1");

        let completed = link.on_line(
            r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"completed"}}}"#,
        );
        assert_eq!(completed.last().unwrap()["type"], "turn.completed");
    }

    #[test]
    fn evento_externo_desconhecido_e_ignorado_na_borda() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        assert!(link
            .on_line(r#"{"method":"future/event","params":{"new":true}}"#)
            .is_empty());
    }

    #[test]
    fn comando_vira_bash_e_o_resultado_acha_o_bloco() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"/bin/zsh -lc \"ls -la\"","commandActions":[{"type":"unknown","command":"ls -la"}],"status":"inProgress"}}}"#);
        assert_eq!(f[0]["type"], "assistant.block");
        let block = &f[0]["block"];
        assert_eq!(block["kind"], "tool");
        assert_eq!(block["name"], "Bash");
        assert_eq!(block["id"], "c1");
        assert_eq!(block["input"]["command"], "ls -la");
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"commandExecution","id":"c1","status":"completed","aggregatedOutput":"a.txt\n","exitCode":0}}}"#);
        assert_eq!(f[0]["type"], "tool.completed");
        assert_eq!(f[0]["toolId"], "c1");
        assert_eq!(f[0]["output"], "a.txt\n");
        assert_eq!(f[0]["error"], false);
    }

    /// O texto que estava chegando fecha antes de a ferramenta entrar: os
    /// índices dos blocos são os que a tela vai contar.
    #[test]
    fn ferramenta_no_meio_do_texto_fecha_o_texto_antes() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"reasoning","id":"r1"}}}"#,
        );
        link.on_line(r#"{"method":"item/reasoning/summaryTextDelta","params":{"itemId":"r1","delta":"pensando"}}"#);
        let f = link.on_line(r#"{"method":"item/started","params":{"item":{"type":"commandExecution","id":"c1","command":"ls","commandActions":[]}}}"#);
        assert_eq!(f[0]["type"], "assistant.block");
        assert_eq!(f[0]["block"]["kind"], "thinking");
        assert_eq!(f[0]["block"]["text"], "pensando");
        assert_eq!(f[1]["block"]["kind"], "tool");
        // O próximo texto nasce no índice 2: pensamento (0), ferramenta (1).
        let f = link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"m1"}}}"#,
        );
        assert_eq!(f[0]["type"], "assistant.block.started");
        assert_eq!(f[0]["index"], 2);
    }

    #[test]
    fn a_pergunta_vira_card_e_a_resposta_volta_no_id_do_codex() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"id":7,"method":"item/tool/requestUserInput","params":{"itemId":"q1","questions":[{"id":"cor","header":"Cor","question":"Qual cor?","options":[{"label":"azul","description":"frio"}]}]}}"#);
        assert_eq!(f[0]["type"], "request.opened");
        assert_eq!(f[0]["requestId"], "7");
        assert_eq!(f[0]["kind"], "question");
        assert_eq!(f[0]["tool"], "AskUserQuestion");
        assert_eq!(f[0]["input"]["questions"][0]["options"][0]["label"], "azul");
        link.write(&json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "7",
            "response": { "outcome": "answer", "answers": { "Qual cor?": "azul" } },
        }))
        .unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["id"], 7);
        assert_eq!(sent[0]["result"]["answers"]["cor"]["answers"][0], "azul");
    }

    #[test]
    fn aprovacao_de_comando_responde_accept_ou_decline() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"id":"r-9","method":"item/commandExecution/requestApproval","params":{"itemId":"c1","command":"rm -rf x"}}"#);
        assert_eq!(f[0]["type"], "request.opened");
        assert_eq!(f[0]["kind"], "approval");
        assert_eq!(f[0]["tool"], "Bash");
        assert_eq!(f[0]["input"]["command"], "rm -rf x");
        link.write(&json!({
            "v": 1,
            "type": "request.respond",
            "requestId": "r-9",
            "response": { "outcome": "deny", "message": "não" },
        }))
        .unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["id"], "r-9");
        assert_eq!(sent[0]["result"]["decision"], "decline");
    }

    #[test]
    fn compact_e_a_compactacao_contam_o_antes_e_o_depois() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":20000},"modelContextWindow":258400}}}"#);
        let f = link.write(&user("/compact")).unwrap();
        assert_eq!(f[0]["type"], "context.compaction");
        assert_eq!(f[0]["state"], "started");
        assert_eq!(out.take()[0]["method"], "thread/compact/start");
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-c"}}}"#);
        link.on_line(
            r#"{"method":"item/started","params":{"item":{"type":"contextCompaction","id":"k1"}}}"#,
        );
        link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":4000},"modelContextWindow":258400}}}"#);
        let f = link.on_line(r#"{"method":"item/completed","params":{"item":{"type":"contextCompaction","id":"k1"}}}"#);
        assert_eq!(f[0]["type"], "context.compaction");
        assert_eq!(f[0]["state"], "stopped");
        assert_eq!(f[1]["type"], "context.compacted");
        assert_eq!(f[1]["before"], 20000);
        assert_eq!(f[1]["after"], 4000);
    }

    #[test]
    fn context_vira_o_relatorio_que_a_tela_desenha() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.on_line(r#"{"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"last":{"totalTokens":12300},"modelContextWindow":258400}}}"#);
        assert_eq!(f[0]["type"], "context.updated");
        assert_eq!(f[0]["used"], 12300);
        assert_eq!(f[0]["window"], 258400);
        let f = link.write(&user("/context")).unwrap();
        assert_eq!(f[0]["type"], "context.reported");
        let text = f[0]["markdown"].as_str().unwrap();
        assert!(text.starts_with("## Context Usage"));
        assert!(text.contains("**Model:** gpt-5.4"));
        assert!(text.contains("**Tokens:** 12k / 258k (5%)"));
        assert_eq!(f[1]["type"], "turn.completed");
        assert!(out.take().is_empty(), "/context não vai ao processo");
    }

    #[test]
    fn comando_que_o_codex_nao_tem_e_recusado_na_tela() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let f = link.write(&user("/cost")).unwrap();
        assert_eq!(f[0]["type"], "system.notice");
        assert_eq!(f[0]["code"], "command.unsupported");
        assert!(f[0]["detail"].as_str().unwrap().contains("/cost"));
        assert_eq!(f[1]["type"], "turn.completed");
    }

    #[test]
    fn caminho_absoluto_nao_vira_comando_de_barra() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let path = r#"/var/folders/q7/TemporaryItems/Captura\ de\ Tela.png"#;
        assert!(link.write(&user(path)).unwrap().is_empty());
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/start");
        assert_eq!(sent[0]["params"]["input"][0]["text"], path);
    }

    #[test]
    fn log_do_app_server_nao_duplica_erro_da_ferramenta() {
        let log = "\u{1b}[2m2026-08-28T16:54:16.210466Z\u{1b}[0m \u{1b}[31mERROR\u{1b}[0m \u{1b}[2mcodex_core::tools::router\u{1b}[0m: error=apply_patch verification failed";
        assert_eq!(process_stderr(log), None);
        assert_eq!(
            process_stderr("codex: not logged in"),
            Some("codex: not logged in".into())
        );
    }

    #[test]
    fn interromper_precisa_do_turno() {
        let (mut link, out) = link(None);
        opened(&mut link, &out);
        let stop = json!({ "v": 1, "type": "turn.interrupt" });
        link.write(&stop).unwrap();
        assert!(out.take().is_empty());
        link.on_line(r#"{"method":"turn/started","params":{"turn":{"id":"turn-1"}}}"#);
        link.write(&stop).unwrap();
        let sent = out.take();
        assert_eq!(sent[0]["method"], "turn/interrupt");
        assert_eq!(sent[0]["params"]["turnId"], "turn-1");
        let f = link.on_line(r#"{"method":"turn/completed","params":{"turn":{"id":"turn-1","status":"interrupted"}}}"#);
        assert_eq!(f[0]["type"], "turn.completed");
        assert_eq!(f[0]["outcome"], "interrupted");
        assert_eq!(f[0]["message"], "");
    }

    /// Como o Codex manda: caminho absoluto, hunk cru na alteração, e o
    /// conteúdo do arquivo (sem sinal) no arquivo novo.
    #[test]
    fn o_patch_vira_diff_com_cabecalho_e_sinal() {
        let change = json!({ "path": "/wt/src/a.rs", "kind": { "type": "update", "move_path": null }, "diff": "@@ -1 +1 @@\n-a\n+b\n" });
        let text = patch(&change, "/wt");
        assert_eq!(
            text,
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b"
        );
        let add =
            json!({ "path": "/wt/n.txt", "kind": { "type": "add" }, "diff": "novo\nlinha\n" });
        assert_eq!(patch(&add, "/wt"), "diff --git a/n.txt b/n.txt\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1,2 @@\n+novo\n+linha");
        let del = json!({ "path": "/outro/x.txt", "kind": { "type": "delete" }, "diff": "fim\n" });
        assert!(patch(&del, "/wt").starts_with("diff --git a//outro/x.txt b//outro/x.txt\n--- a//outro/x.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-fim"));
    }

    #[test]
    fn kilo_como_na_tela() {
        assert_eq!(kilo(368), "368");
        assert_eq!(kilo(3140), "3.1k");
        assert_eq!(kilo(24000), "24k");
        assert_eq!(kilo(1_200_000), "1.2m");
    }
}
